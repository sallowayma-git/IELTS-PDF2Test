//! 受限编辑工具的真实执行器。
//!
//! 这一层是**唯一**允许云端修复改动权威稿的入口。它存在的意义不是"多一层封装"，
//! 而是把三件只有在后端才知道的事集中在一处：
//!
//! 1. **身份**：来源是 `CloudRepair` 而不是模型在 JSON 里自称的什么；请求 id 由
//!    `run/round/toolCall` 派生，同一个调用重试复用同一个 id（重复写入不会发生），
//!    内容不同则不复用（不会被旧结果顶替）。
//! 2. **授权**：命令先过白名单，再按 [`EditFootprint`] 与**人工保护目标**求交集；
//!    命中就整批拒绝，并把**具体**目标回报给模型，让它缩小修复范围。
//! 3. **校验分层**：工作副本上试算 → schema 解析 → 质量与引用闭合 → 只拒绝**本次
//!    引入的**新机械错误。原稿其他题组已有的缺答问题不应阻止当前题组的有效修复
//!    （"有一个问题就整卷修不动"是这条要避免的失败模式）。
//!
//! 模型能做的只有"提交一批领域命令 + 说明依据"。它不能执行代码、不能改源码、
//! 不能直接写导出 JS、不能碰质量/审计/来源路径字段、不能标记问题已解决。

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::authoring_v2_commands::{apply_patch, refresh_quality_report, validate_authoring};
use crate::library::repository::{
    apply_editor_commands_tx_with, get_canonical_ds, human_protected_targets,
    open_library_connection, undo_cloud_repair_run, ApplyEditorCommandsInput, EditFootprint,
    EditOrigin,
};
use crate::CommandResult;

/// 模型可以调用的编辑命令（**只有这些**）。
///
/// 有意不开放：
/// - `resolveIssue`：允许它等于允许模型靠"标记已解决"代替真正修复内容；
/// - `deleteAnswerSlot`：槽位删除必须走 `upsertTaskGroupBundle` 的整组替换，那里才会
///   连带处理旧槽位的去向与跨组引用；
/// - `bindSource` / `cropAsset` / `setHotspot` / `removeHotspot`：属于来源绑定与图像
///   裁切，不是本轮"识别修复"的范围，开放只会扩大越权面。
pub(crate) const MODEL_ALLOWED_OPS: [&str; 14] = [
    "replaceText",
    "replaceContent",
    "insertNode",
    "moveNode",
    "deleteNode",
    "setAnswer",
    "setTaskType",
    "setQuestionExpression",
    "setResponseCardinality",
    "setResponseGroup",
    "setOptionBank",
    "insertAnswerSlot",
    "setNodeAttrs",
    "upsertTaskGroupBundle",
];

/// `setNodeAttrs` 对模型开放的属性：**只有展示与交互**。
///
/// 现有的 `is_safe_node_attribute` 是给编辑器的宽白名单（含 `provenanceStatus`、
/// `slotIds`、`assetId`）。原样交给模型等于让它改来源标记与结构引用——那不是"修复
/// 内容"，而是绕过保护。
const MODEL_ALLOWED_NODE_ATTRS: [&str; 9] = [
    "align",
    "indentLevel",
    "level",
    "altText",
    "placeholder",
    "displayLabel",
    "inline",
    "label",
    "display",
];

/// 模型**不得**提交的字段。命中的一律剥离并在结果里如实回报，不报错。
///
/// 为什么剥离而不是报错：这些字段对结果没有任何作用，报错只会白白消耗一个模型回合，
/// 而模型也无法从错误里学到"我本来就不该写它"。剥离 + 回报既安全又不浪费预算。
const FORBIDDEN_COMMAND_KEYS: [&str; 11] = [
    "provenanceStatus",
    "preserveProvenance",
    "restoreProvenanceStatus",
    "quality",
    "audit",
    "reviewState",
    "humanVerified",
    "sourcePath",
    "sourceFilePath",
    "publishPassed",
    "exportedAt",
];

/// 一次云端编辑请求（由工具层构造，**不是**直接反序列化的模型输出）。
#[derive(Debug, Clone)]
pub(crate) struct CloudEditRequest {
    pub item_id: String,
    pub repair_run_id: String,
    pub base_version: i64,
    /// 同一轮内的调用序号，参与 requestId 派生。
    pub round: i64,
    pub tool_call_id: String,
    pub commands: Vec<Value>,
    /// 证据（原文页索引与引文）：只做留痕与后端核对，不参与内容正确性判断。
    pub evidence: Vec<Value>,
}

/// 一次云端编辑的执行结果。**这是回给模型的事实**，不是模型的自我描述。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloudEditOutcome {
    pub status: CloudEditStatus,
    /// 提交后的编辑版本（成功时；失败时为读取到的当前版本，便于模型据此重读重试）。
    pub edit_version: i64,
    pub applied_count: usize,
    pub applied_targets: Vec<String>,
    /// 具体错误码与原因（模型据此调整命令，而不是收到一句"失败"）。
    pub errors: Vec<String>,
    /// 被剥离的越权 / 无意义字段，如实回报。
    pub stripped_keys: Vec<String>,
    /// 本次引入的新机械错误（硬失败）。空 = 没有引入新的结构破坏。
    pub introduced_hard_failures: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CloudEditStatus {
    Applied,
    Rejected,
}

/// 命令净化：白名单 + 剥离越权字段 + 收敛 `setNodeAttrs` 的属性面。
///
/// 返回净化后的命令与"被剥离了什么"，后者要如实回报给模型——静默剥离会让模型以为
/// 自己写的字段生效了，下一轮继续写。
pub(crate) fn sanitize_commands(raw: &[Value]) -> Result<(Vec<Value>, Vec<String>), String> {
    if raw.is_empty() {
        return Err("CLOUD_EDIT_NO_COMMANDS".to_string());
    }
    let mut cleaned = Vec::with_capacity(raw.len());
    let mut stripped = BTreeSet::new();
    for command in raw {
        let object = command
            .as_object()
            .ok_or_else(|| "CLOUD_EDIT_COMMAND_NOT_OBJECT".to_string())?;
        let op = object
            .get("op")
            .and_then(Value::as_str)
            .ok_or_else(|| "CLOUD_EDIT_COMMAND_MISSING_OP".to_string())?;
        if !MODEL_ALLOWED_OPS.contains(&op) {
            return Err(format!("CLOUD_EDIT_OP_NOT_ALLOWED:{op}"));
        }
        let mut next = Map::new();
        for (key, value) in object {
            if FORBIDDEN_COMMAND_KEYS.contains(&key.as_str()) {
                stripped.insert(key.clone());
                continue;
            }
            if op == "setNodeAttrs" && key == "attrs" {
                let mut attrs = Map::new();
                if let Some(given) = value.as_object() {
                    for (attr, attr_value) in given {
                        if MODEL_ALLOWED_NODE_ATTRS.contains(&attr.as_str()) {
                            attrs.insert(attr.clone(), attr_value.clone());
                        } else {
                            stripped.insert(format!("attrs.{attr}"));
                        }
                    }
                }
                next.insert(key.clone(), Value::Object(attrs));
                continue;
            }
            next.insert(key.clone(), value.clone());
        }
        cleaned.push(Value::Object(next));
    }
    Ok((cleaned, stripped.into_iter().collect()))
}

/// 后端派生 requestId：同一 run / 同一轮 / 同一调用重试 => 同一 id（幂等命中）。
///
/// **不让模型提供 requestId**：否则它可以用同一个 id 提交不同内容触发
/// `EDIT_REQUEST_ID_REUSED`，也可以换 id 把同一个补丁重复写进去。
fn request_id_for(request: &CloudEditRequest) -> String {
    format!(
        "cloud-repair:{}:{}:{}",
        request.repair_run_id, request.round, request.tool_call_id
    )
}

fn hard_failures_of(ds: &Value) -> BTreeSet<String> {
    ds.pointer("/quality/hardFailures")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .map(str::to_string)
        .collect()
}

/// 证据条目的结构校验。
///
/// 只校验**结构**（来源归属字段、页范围、引文非空），不校验内容正确性——后者是模型
/// 结合原文的语义判断，程序无法替代。但"结构有效"不等于"内容正确"，所以这里通过
/// 也不代表后端认可了模型的主张。
///
/// `pageIndex >= 1`：页索引 0 在本产品里被判为无效来源定位（见
/// `cloud_outline_group_quote_invalid`），早在这里拦下比事后返工便宜。
fn validate_evidence(evidence: &[Value]) -> Vec<String> {
    let mut problems = Vec::new();
    for (index, entry) in evidence.iter().enumerate() {
        let Some(object) = entry.as_object() else {
            problems.push(format!("CLOUD_EDIT_EVIDENCE_NOT_OBJECT:{index}"));
            continue;
        };
        if object
            .get("sourceFileId")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
        {
            problems.push(format!("CLOUD_EDIT_EVIDENCE_SOURCE_MISSING:{index}"));
        }
        match object.get("pageIndex").and_then(Value::as_i64) {
            Some(page) if page >= 1 => {}
            Some(page) => problems.push(format!("CLOUD_EDIT_EVIDENCE_PAGE_INVALID:{index}:{page}")),
            None => problems.push(format!("CLOUD_EDIT_EVIDENCE_PAGE_MISSING:{index}")),
        }
        if object
            .get("quote")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("")
            .is_empty()
        {
            problems.push(format!("CLOUD_EDIT_EVIDENCE_QUOTE_MISSING:{index}"));
        }
    }
    problems
}

/// 云端修复的写入入口。
///
/// 全流程落在**一个**事务里：版本 CAS、授权检查、试算校验、写入、journal、保护目标
/// 更新彼此同生共死。任何一步失败整批回滚，权威稿一字不改。
pub(crate) fn apply_cloud_edits(
    root: &Path,
    request: &CloudEditRequest,
) -> CommandResult<CloudEditOutcome> {
    let (commands, stripped_keys) = sanitize_commands(&request.commands)?;
    let evidence_problems = validate_evidence(&request.evidence);

    let mut conn = open_library_connection(root)?;
    // 读一次当前稿：既用于**预检**（给模型更快的具体反馈），也用于"本次引入了哪些
    // 新硬失败"的基线。注意它**不**作为写入前提——真正的版本前提由事务内的 CAS 复核，
    // 因为预检与提交之间可能有人工保存落地。
    let (current_ds, current_version) = get_canonical_ds(&conn, &request.item_id)?
        .ok_or_else(|| format!("ITEM_DS_NOT_SEEDED:{}", request.item_id))?;

    // 基线必须用**同一套质量管线**在"未施加本次编辑的同一份稿件"上重算，而不是直接读
    // 库里那一份 `quality.hardFailures`。理由：库里那份可能是播种 / 迁移时写下的、与当前
    // 内容不同步的旧结果，于是「本次是否引入新机械错误」会退化成「库里那份质量块新不新」，
    // 一批与本次修复无关的旧差异就能把一次有效修复顶掉——正是任务书要避免的
    // 「有一个问题就整卷修不动」。用 clone 重算，基线才有唯一、可复现的含义。
    let mut baseline_ds = current_ds.clone();
    refresh_quality_report(root, &request.item_id, &mut baseline_ds)?;
    let before_hard_failures = hard_failures_of(&baseline_ds);

    if !evidence_problems.is_empty() {
        return Ok(CloudEditOutcome {
            status: CloudEditStatus::Rejected,
            edit_version: current_version,
            applied_count: 0,
            applied_targets: Vec::new(),
            errors: evidence_problems,
            stripped_keys,
            introduced_hard_failures: Vec::new(),
        });
    }

    // 授权预检：命中人工保护目标时**直接回报具体目标**，而不是让模型等到提交才吃
    // 一个笼统的拒绝。事务内还会再查一次（那时才是真正生效的一份）。
    let footprint = EditFootprint::merge(&current_ds, &commands);
    let protected = human_protected_targets(&conn, &request.item_id, &current_ds)?;
    if let Some(conflict) = footprint.first_conflict(&protected) {
        return Ok(CloudEditOutcome {
            status: CloudEditStatus::Rejected,
            edit_version: current_version,
            applied_count: 0,
            applied_targets: Vec::new(),
            errors: vec![format!("EDIT_PROTECTED_TARGET:{conflict}")],
            stripped_keys,
            introduced_hard_failures: Vec::new(),
        });
    }

    let request_id = request_id_for(request);
    // 事务回调的签名是 `&dyn Fn`（见 `apply_editor_commands_tx_with`），**不能**在里面
    // 改外部 `Vec`——那会让它变成 `FnMut`，编译不过，而为了绕开它去改回调类型会破坏
    // 调用方「回调只读」的既有约定。用 `RefCell` 记录，回调结束后再取出来。
    let introduced: std::cell::RefCell<Vec<String>> = std::cell::RefCell::new(Vec::new());
    let attempt = apply_editor_commands_tx_with(
        &mut conn,
        &ApplyEditorCommandsInput {
            item_id: request.item_id.clone(),
            base_version: request.base_version,
            request_id: Some(request_id.clone()),
            commands,
            title: None,
        },
        EditOrigin::CloudRepair,
        Some(&request.repair_run_id),
        &apply_patch,
        &|ds| {
            // 校验分层：schema 解析 → 质量重算（内部含 ID / 引用闭合）→ 只拒绝
            // **本次新增**的硬失败。原稿本来就有的问题不阻止本次有效修复。
            refresh_quality_report(root, &request.item_id, ds)?;
            validate_authoring(ds)?;
            let after = hard_failures_of(ds);
            let new_ones: Vec<String> = after.difference(&before_hard_failures).cloned().collect();
            if !new_ones.is_empty() {
                let message = format!(
                    "CLOUD_EDIT_INTRODUCED_HARD_FAILURES:{}",
                    new_ones.join(",")
                );
                *introduced.borrow_mut() = new_ones;
                return Err(message);
            }
            Ok(())
        },
        &|_, _| Ok(()),
    );

    match attempt {
        Ok(result) => Ok(CloudEditOutcome {
            status: CloudEditStatus::Applied,
            edit_version: result.edit_version,
            applied_count: result.applied_count,
            applied_targets: result.applied_targets,
            errors: Vec::new(),
            stripped_keys,
            introduced_hard_failures: Vec::new(),
        }),
        Err(error) => {
            // 失败也要如实给出**当前**版本：模型据此重新 read_draft 再修，而不是
            // 盲目重放一份基于旧版本的补丁。
            let current = get_canonical_ds(&conn, &request.item_id)?
                .map(|(_, version)| version)
                .unwrap_or(current_version);
            Ok(CloudEditOutcome {
                status: CloudEditStatus::Rejected,
                edit_version: current,
                applied_count: 0,
                applied_targets: Vec::new(),
                errors: vec![error],
                stripped_keys,
                introduced_hard_failures: introduced.into_inner(),
            })
        }
    }
}

/// 撤销整轮自动修复（走 Rust 批次撤销，前端不得挪用自己的本地 undoStack）。
pub(crate) fn undo_repair(
    root: &Path,
    item_id: &str,
    repair_run_id: &str,
    base_version: i64,
) -> CommandResult<Value> {
    let mut conn = open_library_connection(root)?;
    let outcome = undo_cloud_repair_run(&mut conn, item_id, repair_run_id, base_version, &|ds| {
        refresh_quality_report(root, item_id, ds)?;
        validate_authoring(ds)
    })?;
    Ok(json!({
        "status": "undone",
        "itemId": outcome.item_id,
        "editVersion": outcome.edit_version,
        "restored": outcome.restored,
        "skipped": outcome.skipped,
    }))
}

/// 工具协议解析结果（阶段四的循环消费它；此处提供纯粹的解析，便于单测）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ToolMessage {
    pub call_id: String,
    pub tool: String,
    #[serde(default)]
    pub arguments: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizer_rejects_ops_outside_the_model_allow_list() {
        // `resolveIssue` 是重点：它能让模型靠"标记已解决"冒充修复完成。
        let error = sanitize_commands(&[json!({"op": "resolveIssue", "issueId": "i1"})])
            .expect_err("resolveIssue 绝不能被模型调用");
        assert!(error.contains("CLOUD_EDIT_OP_NOT_ALLOWED"), "{error}");
        let error = sanitize_commands(&[json!({"op": "bindSource", "nodeId": "n1"})])
            .expect_err("bindSource 不在本轮开放范围");
        assert!(error.contains("CLOUD_EDIT_OP_NOT_ALLOWED"), "{error}");
    }

    #[test]
    fn sanitizer_strips_authority_fields_and_narrows_node_attrs() {
        let (cleaned, stripped) = sanitize_commands(&[json!({
            "op": "setNodeAttrs",
            "nodeId": "n1",
            "attrs": {
                "displayLabel": "(a)",
                "provenanceStatus": "user_edited",
                "slotIds": ["slot-1"],
                "assetId": "asset-9"
            },
            "provenanceStatus": "user_edited",
            "preserveProvenance": true,
            "quality": {"state": "ready"}
        })])
        .expect("允许的命令必须能被净化而不是被判死");
        assert_eq!(cleaned.len(), 1);
        let command = cleaned[0].as_object().unwrap();
        assert!(!command.contains_key("preserveProvenance"), "越权字段必须被剥离");
        assert!(!command.contains_key("quality"), "模型不得提交 quality");
        assert!(
            !command.contains_key("provenanceStatus"),
            "顶层来源标记必须被剥离"
        );
        let attrs = command["attrs"].as_object().unwrap();
        assert_eq!(attrs.get("displayLabel"), Some(&json!("(a)")));
        assert!(!attrs.contains_key("provenanceStatus"), "来源标记必须被剥离");
        assert!(!attrs.contains_key("slotIds"), "结构引用必须被剥离");
        assert!(!attrs.contains_key("assetId"), "资源引用必须被剥离");
        // 两层都要如实回报：顶层越权字段与 attrs 内被收敛掉的属性。精确匹配而不是
        // `ends_with`——后者会把"根本没剥掉顶层字段"也判成通过。
        assert!(
            stripped.iter().any(|key| key == "provenanceStatus"),
            "顶层 provenanceStatus 必须被剥离并回报：{stripped:?}"
        );
        assert!(
            stripped.iter().any(|key| key == "attrs.provenanceStatus"),
            "attrs 内的 provenanceStatus 必须被收敛并回报：{stripped:?}"
        );
    }

    #[test]
    fn request_id_is_derived_from_run_round_and_call_not_from_the_model() {
        let request = CloudEditRequest {
            item_id: "item".into(),
            repair_run_id: "run-1".into(),
            base_version: 3,
            round: 2,
            tool_call_id: "call-7".into(),
            commands: Vec::new(),
            evidence: Vec::new(),
        };
        assert_eq!(request_id_for(&request), "cloud-repair:run-1:2:call-7");
        // 重试同一调用 ⇒ 同一 id（幂等命中，不重复写入）。
        let mut retry = request.clone();
        retry.base_version = 99;
        assert_eq!(request_id_for(&retry), request_id_for(&request));
        // 换一轮 ⇒ 换 id（新内容不会被当成旧调用的重放）。
        let mut next_round = request.clone();
        next_round.round = 3;
        assert_ne!(request_id_for(&next_round), request_id_for(&request));
    }

    #[test]
    fn empty_command_batches_are_rejected_instead_of_silently_succeeding() {
        let error = sanitize_commands(&[]).expect_err("空批次必须被拒，不能静默成功");
        assert_eq!(error, "CLOUD_EDIT_NO_COMMANDS");
    }
}

/// 云端修复「写入入口」的端到端测试。
///
/// 这些函数依赖**文件型 SQLite**（`open_library_connection`），不是内存库，所以每个测试
/// 都在临时目录里造一个真实库根，并播入一份合规稿。稿件用仓库里的 golden fixture
/// `fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json`——它是真实通过
/// `validate_authoring` 的 `IeltsAuthoringIRV2` 稿，避免手写精简稿被
/// `AUTHORING_SCHEMA_INVALID` 拒掉而掩盖真正要验证的逻辑。
///
/// 每条测试都断言「若被测行为被删除则应失败」：成功路径断言版本推进 + canonical 实际变化
/// + journal 落库；各拒绝路径断言 `status == Rejected`、具体错误码、且 canonical 不变。
#[cfg(test)]
mod cloud_repair_write_entry_tests {
    use super::*;
    use std::path::Path;

    use crate::library::repository::{get_canonical_ds, open_library_connection, seed_canonical_ds, upsert_item_shell, UpsertItemInput};
    use crate::util::ensure_app_dirs;
    use rusqlite::{OptionalExtension, params};

    fn temp_root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("cloud-repair-{}", uuid::Uuid::new_v4().simple()))
    }

    fn fixture_path() -> std::path::PathBuf {
        // `CARGO_MANIFEST_DIR` 指向 `src-tauri`（Windows 上可能带尾部分隔符），而 golden
        // fixture 在仓库根 `fixtures/` 下。先去掉尾部分隔符再取父目录，避免 parent() 在
        // 尾部分隔符时返回目录自身。
        let manifest = env!("CARGO_MANIFEST_DIR").trim_end_matches(['\\', '/']);
        std::path::Path::new(manifest)
            .parent()
            .unwrap()
            .join("fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json")
    }

    fn load_fixture() -> Value {
        let path = fixture_path();
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("读取 golden fixture 失败 path={:?} err={}", path, error));
        serde_json::from_str(&text).expect("解析 golden fixture 为合法 IeltsAuthoringIRV2")
    }

    /// 在临时库根里造一个真实库行并播入合规稿，返回 item id。
    fn seed_item(root: &Path, ds: &Value) -> String {
        ensure_app_dirs(root).expect("ensure_app_dirs");
        let conn = open_library_connection(root).expect("打开库连接");
        let item_id = "early-approaches-architecture-proof";
        upsert_item_shell(
            &conn,
            &UpsertItemInput {
                id: item_id,
                modality: "reading",
                title: "Early Approaches to Organisational Design",
                status: "action_required",
                source_asset_id: None,
            },
        )
        .expect("upsert_item_shell");
        seed_canonical_ds(
            &conn,
            item_id,
            &serde_json::to_string(ds).expect("序列化稿件"),
            "action_required",
        )
        .expect("seed_canonical_ds");
        item_id.to_string()
    }

    /// 读回某个槽位的答案条目（用于确认 canonical 真的变了 / 没变）。
    fn read_answer(root: &Path, item_id: &str, slot: &str) -> Value {
        let conn = open_library_connection(root).expect("打开库连接");
        let (ds, _) = get_canonical_ds(&conn, item_id).expect("读 canonical").expect("稿件已播");
        ds.pointer(&format!("/answerKey/{slot}")).cloned().unwrap_or(Value::Null)
    }

    fn read_quality_hard_failures(root: &Path, item_id: &str) -> Vec<String> {
        let conn = open_library_connection(root).expect("打开库连接");
        let (ds, _) = get_canonical_ds(&conn, item_id).expect("读 canonical").expect("稿件已播");
        ds.pointer("/quality/hardFailures")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default()
    }

    fn set_answer_command(slot: &str, labels: &[&str]) -> Value {
        json!({
            "op": "setAnswer",
            "slotId": slot,
            "value": {"kind": "option", "labels": labels, "assignment": "unordered_set"}
        })
    }

    fn base_request(item_id: &str, repair_run_id: &str, base_version: i64, command: Value) -> CloudEditRequest {
        CloudEditRequest {
            item_id: item_id.to_string(),
            repair_run_id: repair_run_id.to_string(),
            base_version,
            round: 1,
            tool_call_id: "call-1".to_string(),
            commands: vec![command],
            evidence: Vec::new(),
        }
    }

    // 1. 成功路径：setAnswer 改一个真实存在的槽位 → Applied、版本推进、canonical 真变、journal 落库。
    #[test]
    fn cloud_edit_success_path_applies_and_records_journal() {
        let root = temp_root();
        let item_id = seed_item(&root, &load_fixture());

        let request = base_request(&item_id, "run-success", 1, set_answer_command("q14", &["A"]));
        let outcome = apply_cloud_edits(&root, &request).expect("apply_cloud_edits");

        assert_eq!(outcome.status, CloudEditStatus::Applied, "errors={:?}", outcome.errors);
        assert_eq!(outcome.edit_version, 2, "版本应推进到 base + 1");
        // 重新读 canonical：答案真的变了（不是只回报成功）。
        let answer = read_answer(&root, &item_id, "q14");
        assert_eq!(answer.pointer("/labels"), Some(&json!(["A"])), "答案未实际写入 canonical");
        // journal 必须存在一行 edit_origin = 'cloud_repair' 且 repair_run_id 等于传的 run id。
        let conn = open_library_connection(&root).expect("打开库连接");
        let found: Option<(String, String)> = conn
            .query_row(
                "SELECT edit_origin, repair_run_id FROM editor_journal_v1 \
                 WHERE library_item_id = ?1 AND edit_origin = 'cloud_repair'",
                params![item_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .expect("查询 journal");
        let (origin, run) = found.expect("应存在 cloud_repair journal 行");
        assert_eq!(origin, "cloud_repair");
        assert_eq!(run, "run-success");
        let _ = std::fs::remove_dir_all(&root);
    }

    // 2. 证据门：pageIndex:0 → Rejected、错误码、canonical 不变。
    #[test]
    fn cloud_edit_rejects_invalid_evidence_page_index() {
        let root = temp_root();
        let item_id = seed_item(&root, &load_fixture());

        let mut request = base_request(&item_id, "run-evidence", 1, set_answer_command("q14", &["A"]));
        request.evidence = vec![json!({
            "sourceFileId": "early-approaches-pdf",
            "pageIndex": 0,
            "quote": "some quoted span"
        })];
        let outcome = apply_cloud_edits(&root, &request).expect("apply_cloud_edits");

        assert_eq!(outcome.status, CloudEditStatus::Rejected);
        assert!(
            outcome.errors.iter().any(|e| e == "CLOUD_EDIT_EVIDENCE_PAGE_INVALID:0:0"),
            "errors={:?}", outcome.errors
        );
        // canonical 的版本与内容都没变。
        assert_eq!(outcome.edit_version, 1);
        let answer = read_answer(&root, &item_id, "q14");
        assert_eq!(answer.pointer("/labels"), Some(&json!(["B"])), "证据被拒时答案不得改变");
        let _ = std::fs::remove_dir_all(&root);
    }

    // 3. 人工保护：目标在 protected_edits_json 里 → Rejected、EDIT_PROTECTED_TARGET、canonical 不变。
    #[test]
    fn cloud_edit_rejects_protected_human_target() {
        let root = temp_root();
        let item_id = seed_item(&root, &load_fixture());
        // 模拟「人工改过 q14」：直接写入人工保护目标（v5 起由人工编辑事务同事务维护）。
        {
            let conn = open_library_connection(&root).expect("打开库连接");
            conn.execute(
                "UPDATE library_items_v2 SET protected_edits_json = ?1 WHERE id = ?2",
                params![json!({"targets": ["q14"]}).to_string(), item_id],
            )
            .expect("写入 protected_edits_json");
        }

        let request = base_request(&item_id, "run-protected", 1, set_answer_command("q14", &["A"]));
        let outcome = apply_cloud_edits(&root, &request).expect("apply_cloud_edits");

        assert_eq!(outcome.status, CloudEditStatus::Rejected);
        assert!(
            outcome.errors.iter().any(|e| e.contains("EDIT_PROTECTED_TARGET:q14")),
            "errors={:?}", outcome.errors
        );
        // canonical 不变。
        assert_eq!(outcome.edit_version, 1);
        let answer = read_answer(&root, &item_id, "q14");
        assert_eq!(answer.pointer("/labels"), Some(&json!(["B"])), "受保护目标不得被改动");
        let _ = std::fs::remove_dir_all(&root);
    }

    // 4. 撤销往返：先成功写入，再 undo_repair → 答案恢复为修复前的原值。
    #[test]
    fn cloud_edit_undo_restores_pre_repair_answer() {
        let root = temp_root();
        let item_id = seed_item(&root, &load_fixture());
        let run = "run-undo";

        let outcome = apply_cloud_edits(&root, &base_request(&item_id, run, 1, set_answer_command("q14", &["A"])))
            .expect("apply_cloud_edits");
        assert_eq!(outcome.status, CloudEditStatus::Applied);
        assert_eq!(outcome.edit_version, 2, "修复后版本应推进到 2");
        assert_eq!(
            read_answer(&root, &item_id, "q14").pointer("/labels"),
            Some(&json!(["A"])),
            "修复应已写入"
        );

        // 在版本 2 上撤销整轮。
        let undo = undo_repair(&root, &item_id, run, 2).expect("undo_repair");
        assert_eq!(undo["status"], json!("undone"), "撤销应返回 undone");
        // 答案恢复成修复前的原值 B。
        let answer = read_answer(&root, &item_id, "q14");
        assert_eq!(
            answer.pointer("/labels"),
            Some(&json!(["B"])),
            "撤销后答案应恢复为修复前的原值"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // 5. 陈旧基线回归：稿件存在一个真实但陈旧的硬失败（recompute 得 ANSWER_KEY_MISSING_SLOT，
    //    但库里存的 quality.hardFailures 为空、与重算不一致），合法 setAnswer 仍必须 Applied，
    //    不得被 CLOUD_EDIT_INTRODUCED_HARD_FAILURES 拒掉。锁定「只拒本次引入的新机械错误」。
    #[test]
    fn cloud_edit_legal_fix_survives_stale_hard_failure_baseline() {
        let root = temp_root();
        let mut ds = load_fixture();
        // 故意删掉 q15 的答案条目：recompute 会产出真实硬失败 ANSWER_KEY_MISSING_SLOT，
        // 而库里存的 golden quality.hardFailures 仍为空（陈旧）。本次只改 q14，不影响 q15，
        // 所以该硬失败在修复前后都存在——正是「修一处不该被另一处的老问题挡住」的场景。
        ds["answerKey"].as_object_mut().unwrap().remove("q15");
        let item_id = seed_item(&root, &ds);

        let request = base_request(&item_id, "run-stale", 1, set_answer_command("q14", &["A"]));
        let outcome = apply_cloud_edits(&root, &request).expect("apply_cloud_edits");

        assert_eq!(
            outcome.status,
            CloudEditStatus::Applied,
            "陈旧基线不应阻断合法修复 errors={:?}", outcome.errors
        );
        assert!(outcome.introduced_hard_failures.is_empty(), "未引入新硬失败");
        // 证明：稿件真的存在旧硬失败（重算后质量块含 ANSWER_KEY_MISSING_SLOT），且答案确实改了。
        let hard = read_quality_hard_failures(&root, &item_id);
        assert!(
            hard.iter().any(|code| code == "ANSWER_KEY_MISSING_SLOT"),
            "陈旧硬失败应仍存在: {:?}", hard
        );
        assert_eq!(
            read_answer(&root, &item_id, "q14").pointer("/labels"),
            Some(&json!(["A"])),
            "合法修复仍应写入"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // 6. 版本冲突：错误的 base_version → Rejected、EDIT_VERSION_CONFLICT、canonical 不变。
    #[test]
    fn cloud_edit_rejects_stale_base_version() {
        let root = temp_root();
        let item_id = seed_item(&root, &load_fixture());

        // 实际当前版本是 1，但传 99。
        let request = base_request(&item_id, "run-conflict", 99, set_answer_command("q14", &["A"]));
        let outcome = apply_cloud_edits(&root, &request).expect("apply_cloud_edits");

        assert_eq!(outcome.status, CloudEditStatus::Rejected);
        assert!(
            outcome.errors.iter().any(|e| e.contains("EDIT_VERSION_CONFLICT")),
            "errors={:?}", outcome.errors
        );
        // canonical 不变。
        assert_eq!(outcome.edit_version, 1);
        let answer = read_answer(&root, &item_id, "q14");
        assert_eq!(answer.pointer("/labels"), Some(&json!(["B"])), "版本冲突时答案不得改变");
        let _ = std::fs::remove_dir_all(&root);
    }
}
