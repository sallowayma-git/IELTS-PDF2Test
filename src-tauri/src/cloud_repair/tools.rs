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

use crate::authoring_v2_commands::{
    apply_patch, refresh_quality_report, refresh_quality_report_for_targets, validate_authoring,
};
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
    ///
    /// **重要**：这里每一项是一条**事实指纹**（`错误码@目标类型:目标id[来源锚点]`），
    /// 而**不是**裸的 `quality.hardFailures` 错误码。原因：`/quality/hardFailures` 是按
    /// code 去重后的列表（`push_issue` 只在 `severity == "blocking"` 时压入 code），它把
    /// 「问题落在哪个目标上」丢掉了。于是「q15 缺答案」与「q16 缺答案」在 code 集合里是
    /// 同一个 `ANSWER_KEY_MISSING_SLOT`：云端若把 q16 的答案也删掉，code 集合前后不变、差集
    /// 为空，会被判成「没引入新问题」而放行，用户的内容就此静默丢失。反向同理：code 集合少
    /// 一个元素，并不等于那个问题真的被修好。所以这里比较的是 `/quality/issues[]` 里的
    /// **具体诊断指纹**——模型也能从指纹里直接读出「哪道题」出了问题。
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

/// 阻断性诊断的**事实指纹**：错误码 + 稳定目标 + 必要引用。
///
/// 为什么不能只比较错误码集合：`/quality/hardFailures` 是**去重后的 code 列表**
/// （见 `ielts_grammar/quality.rs::push_issue`），它丢掉了「问题落在哪个目标上」。
/// 于是「q11 缺答案」与「q12 缺答案」在集合里是同一个 `ANSWER_KEY_MISSING_SLOT`：
/// 云端把 q12 的答案删掉后，集合前后相同、差集为空 ⇒ 被判为「没引入新问题」而放行，
/// 用户的内容就此静默丢失。反向同样立不住：集合里少一个元素并不等于那个问题真被修好。
/// 所以比较的必须是**具体诊断**（`/quality/issues[]`），而不是它的 code 投影。
///
/// 指纹形状：`{code}@{targetType}:{targetId}`，当 `sourceAnchors` 非空时追加
/// `[sourceFileId#pageIndex#nodeIds,...]` 后缀（锚点按字符串排序后拼接，每个锚点的
/// nodeIds 用 `+` 连接）。整体确定性、可读，模型能从中直接读出「哪道题」坏。
fn blocking_diagnostic_fingerprints(ds: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Some(issues) = ds.pointer("/quality/issues").and_then(Value::as_array) else {
        return out;
    };
    for issue in issues {
        let Some(obj) = issue.as_object() else {
            continue;
        };
        // 与 `push_issue` 填充 `hardFailures` 用同一个判据，二者保持一致。
        if obj.get("severity").and_then(Value::as_str) != Some("blocking") {
            continue;
        }
        let code = obj.get("code").and_then(Value::as_str).unwrap_or("");
        let target_type = obj.get("targetType").and_then(Value::as_str).unwrap_or("");
        let target_id = obj.get("targetId").and_then(Value::as_str).unwrap_or("");
        let mut fingerprint = format!("{code}@{target_type}:{target_id}");
        if let Some(anchors) = obj.get("sourceAnchors").and_then(Value::as_array) {
            if !anchors.is_empty() {
                let mut rendered: Vec<String> = anchors
                    .iter()
                    .filter_map(|anchor| {
                        let file = anchor
                            .get("sourceFileId")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let page = anchor.get("pageIndex").and_then(Value::as_i64).unwrap_or(0);
                        let node_ids = anchor
                            .get("nodeIds")
                            .and_then(Value::as_array)
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join("+")
                            })
                            .unwrap_or_default();
                        Some(format!("{file}#{page}#{node_ids}"))
                    })
                    .collect();
                rendered.sort();
                fingerprint.push('[');
                fingerprint.push_str(&rendered.join(","));
                fingerprint.push(']');
            }
        }
        out.insert(fingerprint);
    }
    out
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
    // 库里那一份 `quality.hardFailures` / `quality.issues`。理由：库里那份可能是播种 / 迁移
    // 时写下的、与当前内容不同步的旧结果，于是「本次是否引入新机械错误」会退化成
    // 「库里那份质量块新不新」，一批与本次修复无关的旧差异就能把一次有效修复顶掉——正是
    // 任务书要避免的「有一个问题就整卷修不动」。用 clone 重算，基线才有唯一、可复现的含义。
    // 注意：比较的是**阻断性诊断指纹**（`blocking_diagnostic_fingerprints`，即
    // `/quality/issues[]` 里 severity == "blocking" 的 `[code@targetType:targetId[锚点]]`），
    // 而不是去重后的 code 集合——否则「q15 缺答案」与「q16 缺答案」会被当成同一个
    // `ANSWER_KEY_MISSING_SLOT` 而漏判。
    let mut baseline_ds = current_ds.clone();
    refresh_quality_report(root, &request.item_id, &mut baseline_ds)?;
    let before_diagnostics = blocking_diagnostic_fingerprints(&baseline_ds);

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
            //
            // 质量重算带上本次编辑的影响范围（`footprint.targets`）：这些目标上的旧
            // resolution 是人对**改动前**内容作出的判断，内容变了就不能继续算数，
            // 必须重置后重新评价（详见 `refresh_quality_report_for_targets`）。
            refresh_quality_report_for_targets(root, &request.item_id, ds, &footprint.targets)?;
            validate_authoring(ds)?;
            let after = blocking_diagnostic_fingerprints(ds);
            let new_ones: Vec<String> = after.difference(&before_diagnostics).cloned().collect();
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

    #[test]
    fn blocking_diagnostics_distinguish_same_code_on_different_targets() {
        // 两个质量块：hardFailures 的 code 数组**完全相同**，但 issue 落在不同目标上。
        let on_q11 = json!({
            "quality": {
                "hardFailures": ["ANSWER_KEY_MISSING_SLOT"],
                "issues": [{
                    "issueId": "i-q11",
                    "code": "ANSWER_KEY_MISSING_SLOT",
                    "severity": "blocking",
                    "message": "slot 缺答案",
                    "targetType": "slot",
                    "targetId": "q11",
                    "sourceAnchors": [],
                    "suggestedActions": []
                }]
            }
        });
        let on_q12 = json!({
            "quality": {
                "hardFailures": ["ANSWER_KEY_MISSING_SLOT"],
                "issues": [{
                    "issueId": "i-q12",
                    "code": "ANSWER_KEY_MISSING_SLOT",
                    "severity": "blocking",
                    "message": "slot 缺答案",
                    "targetType": "slot",
                    "targetId": "q12",
                    "sourceAnchors": [],
                    "suggestedActions": []
                }]
            }
        });

        // 前提（显式断言）：旧逻辑只比较 hardFailures 的 code 集合，会认为两者无差异。
        let codes_q11: Vec<String> = on_q11
            .pointer("/quality/hardFailures")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        let codes_q12: Vec<String> = on_q12
            .pointer("/quality/hardFailures")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        assert_eq!(
            codes_q11, codes_q12,
            "前提：两个质量块的 hardFailures code 数组必须字节相等，否则证明不了旧逻辑的盲区"
        );

        let fp_q11 = blocking_diagnostic_fingerprints(&on_q11);
        let fp_q12 = blocking_diagnostic_fingerprints(&on_q12);
        // 关键断言：同样的 code、不同目标 ⇒ 不同指纹（旧逻辑则看不到差异）。
        assert_ne!(
            fp_q11, fp_q12,
            "不同目标的同 code 必须产生不同指纹：{:?} vs {:?}",
            fp_q11, fp_q12
        );
        // 确定性：同一输入跑两次结果一致。
        assert_eq!(fp_q11, blocking_diagnostic_fingerprints(&on_q11));
        assert_eq!(fp_q12, blocking_diagnostic_fingerprints(&on_q12));
        // 指纹必须包含目标，模型才能读出「哪道题」坏。
        assert!(fp_q11.iter().any(|f| f.contains("q11")), "指纹应含目标 q11: {:?}", fp_q11);
        assert!(fp_q12.iter().any(|f| f.contains("q12")), "指纹应含目标 q12: {:?}", fp_q12);

        // 非阻断（warning）诊断必须被排除。
        let with_warning = json!({
            "quality": {
                "hardFailures": [],
                "issues": [{
                    "issueId": "w",
                    "code": "SOME_WARNING",
                    "severity": "warning",
                    "message": "无害",
                    "targetType": "document",
                    "targetId": "document",
                    "sourceAnchors": [],
                    "suggestedActions": []
                }]
            }
        });
        assert!(
            blocking_diagnostic_fingerprints(&with_warning).is_empty(),
            "warning 不应进入阻断指纹"
        );
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

    /// 生成一个 `insertAnswerSlot` 命令：在 golden fixture 的共享题组里插入一个**答案未解**
    /// （`unresolved`）的新槽。这会令质量管线在**新目标**上产生一条 `ANSWER_KEY_MISSING_SLOT`
    /// 阻断诊断——用于验证「同 code、不同目标」必须被识别为新引入的硬失败。
    fn insert_unresolved_slot_command(slot_id: &str, question_number: u64, index: usize) -> Value {
        let hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        json!({
            "op": "insertAnswerSlot",
            "taskId": "early-approaches-q14-15",
            "responseGroupId": "early-approaches-shared-response",
            "target": {"kind": "responsePrompt", "responseGroupId": "early-approaches-shared-response"},
            "parentId": "early-approaches-shared-prompt",
            "index": index,
            "slotIndex": index,
            "node": {
                "type": "answer_slot",
                "id": format!("slot-node-{slot_id}"),
                "slotId": slot_id,
                "displayLabel": question_number.to_string(),
                "inline": true,
                "sourceAnchors": [],
                "provenanceStatus": "manual"
            },
            "slot": {
                "slotId": slot_id,
                "questionNumber": question_number,
                "displayLabel": question_number.to_string(),
                "hostNodeId": "early-approaches-shared-prompt",
                "hostType": "prompt",
                "interaction": "checkbox",
                "participation": "scoring",
                "constraints": {"acceptedOptionLabels": ["A", "B", "C", "D", "E"]},
                "sourceAnchors": [{
                    "sourceFileId": "early-approaches-pdf",
                    "pageIndex": 1,
                    "nodeIds": [format!("slot-{slot_id}"), "line-shared-prompt"],
                    "extractionMode": "pdf_native",
                    "sourceHash": hash
                }],
                "confidence": 1.0
            },
            "value": {"kind": "unresolved"},
            "expression": {"kind": "set", "values": [14, 15, question_number]}
        })
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

    // 7. 同 code、不同目标：稿件已存在 q15 缺答案（陈旧阻断 ANSWER_KEY_MISSING_SLOT）的前提下，
    //    又对**新**目标 q16 引入同样的 ANSWER_KEY_MISSING_SLOT，必须被拒。这是任务书点名的
    //    静默丢失回归——旧逻辑只比较 hardFailures 的 code 集合，q15 与 q16 都是同一个 code，
    //    差集为空 ⇒ 误判「没引入新问题」而放行。新指纹逻辑按具体目标区分，q16 被识别为新引入。
    #[test]
    fn cloud_edit_rejects_new_same_code_failure_on_a_different_target() {
        let root = temp_root();
        let mut ds = load_fixture();
        // 预置一个陈旧的真实阻断：删掉 q15 的答案 ⇒ q15 缺答案（ANSWER_KEY_MISSING_SLOT）。
        ds["answerKey"].as_object_mut().unwrap().remove("q15");
        let item_id = seed_item(&root, &ds);

        // 编辑：插入一个**新**槽 q16，答案为 unresolved ⇒ 在 q16 上引入新的
        // ANSWER_KEY_MISSING_SLOT，而 q15 的老问题依旧存在。两者 code 相同、目标不同。
        let command = insert_unresolved_slot_command("q16", 16, 1);
        let request = base_request(&item_id, "run-same-code-diff-target", 1, command);
        let outcome = apply_cloud_edits(&root, &request).expect("apply_cloud_edits");

        assert_eq!(
            outcome.status,
            CloudEditStatus::Rejected,
            "同 code 不同目标的新硬失败必须被拒 errors={:?}",
            outcome.errors
        );
        // 必须是一条**指纹**（含 code 与具体目标），而不只是裸 code——否则模型读不出哪道题坏。
        let hit = outcome
            .introduced_hard_failures
            .iter()
            .find(|f| f.contains("ANSWER_KEY_MISSING_SLOT") && f.contains("q16"));
        assert!(
            hit.is_some(),
            "应报出 q16 的 ANSWER_KEY_MISSING_SLOT 指纹，实际 introduced_hard_failures={:?}",
            outcome.introduced_hard_failures
        );
        // 错误前缀必须保持可识别（任务书要求保留 CLOUD_EDIT_INTRODUCED_HARD_FAILURES:）。
        assert!(
            outcome.errors.iter().any(|e| e.contains("CLOUD_EDIT_INTRODUCED_HARD_FAILURES")),
            "错误码前缀必须保持可识别: {:?}",
            outcome.errors
        );
        // canonical 不变：q14 答案仍是原始 B，且不应出现新槽 q16。
        assert_eq!(
            read_answer(&root, &item_id, "q14").pointer("/labels"),
            Some(&json!(["B"])),
            "被拒后 q14 答案不得改变"
        );
        assert!(
            read_answer(&root, &item_id, "q16").is_null(),
            "被拒后不应出现新槽 q16 的 answerKey"
        );
        let conn = open_library_connection(&root).expect("打开库连接");
        let (stored, _) = get_canonical_ds(&conn, &item_id)
            .expect("读 canonical")
            .expect("稿件已播");
        assert!(
            stored["answerSlots"].get("q16").is_none(),
            "被拒后 answerSlots 不应含 q16"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // 8. 静默放行回归（用户真正要防的场景）：稿件已存在 q15 缺答案（陈旧 ANSWER_KEY_MISSING_SLOT）
    //    的前提下，云端用 setAnswer 把 **q14 原本正确的答案**替换成 unresolved，从而「销毁」q14 的
    //    答案。这次编辑前后 hardFailures 的 code 集合完全相同（都只有 ANSWER_KEY_MISSING_SLOT），
    //    旧逻辑的 `after.difference(&before)` 因此为空、会把它判成「没引入新问题」而 **Applied**——
    //    q14 的答案就此静默丢失。新指纹逻辑按具体目标区分 q14 与 q15，正确识别这是一条新引入的
    //    阻断诊断并拒绝。
    #[test]
    fn cloud_edit_rejects_destroying_a_second_answer_when_a_same_code_problem_already_exists() {
        let root = temp_root();
        let mut ds = load_fixture();
        // 预置一个陈旧的真实阻断：删掉 q15 的答案 ⇒ q15 缺答案（ANSWER_KEY_MISSING_SLOT）。
        ds["answerKey"].as_object_mut().unwrap().remove("q15");
        let item_id = seed_item(&root, &ds);

        // 编辑：把 q14 原本正确的选项答案替换成 unresolved —— 销毁 q14 的答案，
        // 在 q14 上引入一条**同 code** 的新阻断诊断（q15 的老问题依旧）。
        let command = json!({"op": "setAnswer", "slotId": "q14", "value": {"kind": "unresolved"}});
        let request = base_request(&item_id, "run-silent-loss", 1, command.clone());
        let outcome = apply_cloud_edits(&root, &request).expect("apply_cloud_edits");

        assert_eq!(
            outcome.status,
            CloudEditStatus::Rejected,
            "销毁 q14 答案（同 code、不同目标）必须被拒 errors={:?}",
            outcome.errors
        );
        // 必须报出 q14 的 ANSWER_KEY_MISSING_SLOT 指纹（含具体目标），而非仅裸 code。
        let hit = outcome
            .introduced_hard_failures
            .iter()
            .find(|f| f.contains("ANSWER_KEY_MISSING_SLOT") && f.contains("q14"));
        assert!(
            hit.is_some(),
            "应报出 q14 的 ANSWER_KEY_MISSING_SLOT 指纹，实际 introduced_hard_failures={:?}",
            outcome.introduced_hard_failures
        );

        // ---- 盲点证明：编辑前后 hardFailures 的 code 集合完全相同 ----
        // 这正是旧逻辑 `after.difference(&before)` 为空、从而把这次销毁当成「没引入新问题」而
        // Applied 放行、让 q14 答案静默丢失的充要条件。
        let collect_codes = |doc: &Value| -> Vec<String> {
            let mut v: Vec<String> = doc
                .pointer("/quality/hardFailures")
                .and_then(Value::as_array)
                .unwrap_or(&Vec::new())
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            v.sort();
            v
        };
        let mut pre = ds.clone();
        refresh_quality_report(&root, &item_id, &mut pre).expect("pre recompute");
        let mut post = ds.clone();
        apply_patch(&mut post, &command).expect("apply patch");
        refresh_quality_report(&root, &item_id, &mut post).expect("post recompute");
        let pre_codes = collect_codes(&pre);
        let post_codes = collect_codes(&post);
        assert_eq!(
            pre_codes, post_codes,
            "盲点前提：编辑前后 hardFailures 的 code 集合必须完全相同（都仅含 ANSWER_KEY_MISSING_SLOT）——\
             正是这个相等让旧的 code 集合差集为空、从而把 q14 答案的销毁判成「没引入新问题」而静默放行"
        );
        // 而指纹集合必须不同（q14 与 q15 是不同目标）。
        assert_ne!(
            blocking_diagnostic_fingerprints(&post),
            blocking_diagnostic_fingerprints(&pre),
            "指纹集合必须不同：q14 与 q15 是不同目标"
        );

        // canonical 不变：q14 答案仍是 golden 原始值 ["B"]，且 q15 仍缺失。
        assert_eq!(
            read_answer(&root, &item_id, "q14").pointer("/labels"),
            Some(&json!(["B"])),
            "被拒后 q14 答案不得被销毁"
        );
        assert!(
            read_answer(&root, &item_id, "q15").is_null(),
            "被拒后 q15 答案仍应缺失（与种子状态一致）"
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
    // 8. 复核任务书里的一条怀疑：基线用 `refresh_quality_report`（全量），事务内用
    //    `refresh_quality_report_for_targets`（只刷受影响目标）——两边口径不同，会不会
    //    让一次合法修复被 `CLOUD_EDIT_INTRODUCED_HARD_FAILURES` 误拒？
    //
    // 结论：**不会**，这条怀疑不成立。两个函数是同一个实现——`refresh_quality_report`
    // 就是 `refresh_quality_report_for_targets(..., &BTreeSet::new())`，两边都调
    // `evaluate_quality` 做**全量重算**，差别只在「哪些目标上的人工 resolution 被继承」。
    // 而比较用的 `blocking_diagnostic_fingerprints` 只看 severity / code / 目标 / 锚点，
    // **不读 resolution**，所以继承与否根本不改变被比较的集合。
    //
    // 这条用例把等价关系钉住：以后若有人给 `_for_targets` 加上「只重算受影响目标」的
    // 优化（那才会真的引入误拒），这里会立刻变红。
    #[test]
    fn the_baseline_and_the_in_transaction_quality_pass_agree_on_blocking_diagnostics() {
        let root = temp_root();
        let mut ds = load_fixture();
        // 造一个「真实但陈旧」的阻断，落在与本次编辑**无关**的目标上（q15 缺答案）。
        ds["answerKey"].as_object_mut().unwrap().remove("q15");
        // 再给一条人工 resolution，确保「继承 resolution」这条唯一差异真的被触发。
        if let Some(issues) = ds.pointer_mut("/quality/issues").and_then(Value::as_array_mut) {
            for issue in issues.iter_mut() {
                if let Some(details) = issue.get_mut("details").and_then(Value::as_object_mut) {
                    details.insert("resolution".to_string(), json!("resolved"));
                }
            }
        }
        let item_id = seed_item(&root, &ds);

        // 本次只改 q14：合法修复，不该被 q15 的老问题挡住。
        let request = base_request(&item_id, "run-quality-parity", 1, set_answer_command("q14", &["A"]));
        let outcome = apply_cloud_edits(&root, &request).expect("apply_cloud_edits");
        assert_eq!(
            outcome.status,
            CloudEditStatus::Applied,
            "无关目标上的老阻断不得顶掉本次合法修复 errors={:?}",
            outcome.errors
        );
        assert!(outcome.introduced_hard_failures.is_empty(), "本次没有引入新硬失败");

        // 直接对比两种口径：同一份稿子上必须给出**同一组**阻断指纹。
        let conn = open_library_connection(&root).expect("打开库连接");
        let (current, _) = get_canonical_ds(&conn, &item_id).expect("读 canonical").expect("稿件已播");
        drop(conn);

        let mut full = current.clone();
        crate::authoring_v2_commands::refresh_quality_report(&root, &item_id, &mut full)
            .expect("全量刷新");
        let mut none = current.clone();
        crate::authoring_v2_commands::refresh_quality_report_for_targets(
            &root,
            &item_id,
            &mut none,
            &std::collections::BTreeSet::new(),
        )
        .expect("空受影响集刷新");

        assert_eq!(
            blocking_diagnostic_fingerprints(&full),
            blocking_diagnostic_fingerprints(&none),
            "两种口径必须给出同一组阻断指纹；不同就说明基线比对的前提不成立"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

}
