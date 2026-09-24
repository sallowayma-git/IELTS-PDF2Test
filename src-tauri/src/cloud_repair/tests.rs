//! 修复循环的测试。
//!
//! 断言的都是**产品语义**，不是实现细节：
//! - 模型提交非法编辑会被拒，且收到**具体**错误（能据此改对）；
//! - 模型提交合法编辑会真的落库（版本推进、canonical 真变）；
//! - 未知工具不执行；
//! - 取消 / 预算耗尽不会谎报完成；
//! - 剩余问题按**当前 canonical** 重算，已修好的项不会因冻结本地稿仍有差异而复活。

use super::*;
use crate::library::repository::{
    get_canonical_ds, open_library_connection, seed_canonical_ds, upsert_item_shell, UpsertItemInput,
};
use crate::reconcile::candidate::{
    cloud_authoring_candidate_from_normalized, normalize_cloud_authoring, CloudAuthoringIdentity,
};
use crate::auto_pipeline::repair_authoring_step_through_gateway;
use crate::util::ensure_app_dirs;

const ITEM_ID: &str = "early-approaches-architecture-proof";
const BATCH_ID: &str = "batch-1";

fn temp_root() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("cloud-repair-loop-{}", uuid::Uuid::new_v4().simple()))
}

fn golden_authoring() -> Value {
    let manifest = env!("CARGO_MANIFEST_DIR").trim_end_matches(['\\', '/']);
    let path = std::path::Path::new(manifest)
        .parent()
        .expect("src-tauri 必须有父目录")
        .join("fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取 golden 稿失败 path={path:?} err={error}"));
    serde_json::from_str(&text).expect("golden 稿必须是合法 JSON")
}

fn seed_item(root: &Path, ds: &Value) -> String {
    ensure_app_dirs(root).expect("ensure_app_dirs");
    let conn = open_library_connection(root).expect("打开库连接");
    upsert_item_shell(
        &conn,
        &UpsertItemInput {
            id: ITEM_ID,
            modality: "reading",
            title: "Early Approaches to Organisational Design",
            status: "action_required",
            source_asset_id: None,
        },
    )
    .expect("upsert_item_shell");
    seed_canonical_ds(
        &conn,
        ITEM_ID,
        &serde_json::to_string(ds).expect("序列化稿件"),
        "action_required",
    )
    .expect("seed_canonical_ds");
    ITEM_ID.to_string()
}

fn read_answer(root: &Path, slot: &str) -> Value {
    let conn = open_library_connection(root).expect("打开库连接");
    let (ds, _) = get_canonical_ds(&conn, ITEM_ID)
        .expect("读 canonical")
        .expect("稿件已播");
    ds.pointer(&format!("/answerKey/{slot}"))
        .cloned()
        .unwrap_or(Value::Null)
}

/// 造一份「云端完整候选」草稿：结构与 golden 对齐，但 q14 的答案由 B 改成 A。
/// 这样候选与当前稿之间**恰好**只有一处实质差异，便于断言「修好之后不再复活」。
fn cloud_draft(q14_label: &str) -> Value {
    cloud_draft_with(q14_label, "D")
}

/// 同上，但 q15 也能改：包模式要验证「改完一处差异后重新切包」，那需要**两个**差异。
fn cloud_draft_with(q14_label: &str, q15_label: &str) -> Value {
    let node = |id: &str, child: &str, text: &str| {
        json!({
            "type": "paragraph",
            "id": id,
            "sourceAnchors": [],
            "provenanceStatus": "source",
            "children": [{
                "type": "text",
                "id": child,
                "sourceAnchors": [],
                "provenanceStatus": "source",
                "text": text
            }]
        })
    };
    let options: Vec<Value> = ["A", "B", "C", "D", "E"]
        .iter()
        .map(|label| {
            json!({
                "optionId": format!("cloud-opt-{label}"),
                "label": label,
                "content": [{
                    "type": "text",
                    "id": format!("cloud-opt-{label}-text"),
                    "sourceAnchors": [],
                    "provenanceStatus": "source",
                    "text": format!("factor {label}")
                }],
                "sourceAnchors": []
            })
        })
        .collect();
    json!({
        "taskGroups": [{
            "taskId": "cloud-tg-1",
            "displayRange": {"kind": "set", "values": [14, 15]},
            "taskType": "multiple_choice",
            "instructions": [node("cloud-ins", "cloud-ins-text", "Choose TWO letters, A-E.")],
            "optionBank": {
                "optionBankId": "cloud-ob-1",
                "scope": "task_group",
                "options": options,
                "allowReuse": false,
                "sourceAnchors": []
            },
            "responseGroups": [{
                "responseGroupId": "cloud-rg-1",
                "kind": "choice",
                "prompt": [node(
                    "cloud-prompt",
                    "cloud-prompt-text",
                    "Which TWO factors influenced early organisational design?"
                )],
                "slotIds": ["cloud-q14", "cloud-q15"],
                "optionBankRef": "cloud-ob-1",
                "cardinality": {"min": 2, "max": 2, "exact": 2},
                "assignment": "unordered_set",
                "scoringPolicy": "per_slot_ielts_normalized",
                "duplicatePolicy": "reject_submission",
                "allowOptionReuse": false,
                "sourceAnchors": []
            }],
            "sourceAnchors": []
        }],
        "answerSlots": {
            "cloud-q14": {"slotId": "cloud-q14", "questionNumber": 14, "displayLabel": "14",
                "hostNodeId": "cloud-prompt", "hostType": "prompt", "interaction": "checkbox",
                "participation": "scoring", "sourceAnchors": [], "confidence": 0.9},
            "cloud-q15": {"slotId": "cloud-q15", "questionNumber": 15, "displayLabel": "15",
                "hostNodeId": "cloud-prompt", "hostType": "prompt", "interaction": "checkbox",
                "participation": "scoring", "sourceAnchors": [], "confidence": 0.9}
        },
        "answerKey": {
            "cloud-q14": {"kind": "option", "labels": [q14_label], "assignment": "unordered_set"},
            "cloud-q15": {"kind": "option", "labels": [q15_label], "assignment": "unordered_set"}
        },
        "assets": []
    })
}

/// 在 [`cloud_draft_with`] 之上再加一个**本地完全没有的**云端题组（题号 21-22）。
///
/// 用途：`packets::owner_of` 对「云端有、本地没有的题号」返回 `Owner::Document`，于是切包
/// 时会多出一个**文档包**。这是不引入第二份夹具就能造出「两个包」的最短路径——而
/// 「一个包收工后、它的差异被**别的包**改掉」正需要两个包。
fn cloud_draft_with_extra_group(q14_label: &str, q15_label: &str) -> Value {
    let mut draft = cloud_draft_with(q14_label, q15_label);
    let node = |id: &str, child: &str, text: &str| {
        json!({
            "type": "paragraph",
            "id": id,
            "sourceAnchors": [],
            "provenanceStatus": "source",
            "children": [{
                "type": "text",
                "id": child,
                "sourceAnchors": [],
                "provenanceStatus": "source",
                "text": text
            }]
        })
    };
    let options: Vec<Value> = ["A", "B", "C"]
        .iter()
        .map(|label| {
            json!({
                "optionId": format!("cloud-ob2-{label}"),
                "label": label,
                "content": [{
                    "type": "text",
                    "id": format!("cloud-ob2-{label}-text"),
                    "sourceAnchors": [],
                    "provenanceStatus": "source",
                    "text": format!("extra factor {label}")
                }],
                "sourceAnchors": []
            })
        })
        .collect();
    let extra = json!({
        "taskId": "cloud-tg-2",
        "displayRange": {"kind": "set", "values": [21, 22]},
        "taskType": "multiple_choice",
        "instructions": [node("cloud-ins-2", "cloud-ins-2-text", "Choose TWO letters, A-C.")],
        "optionBank": {
            "optionBankId": "cloud-ob-2",
            "scope": "task_group",
            "options": options,
            "allowReuse": false,
            "sourceAnchors": []
        },
        "responseGroups": [{
            "responseGroupId": "cloud-rg-2",
            "kind": "choice",
            "prompt": [node("cloud-prompt-2", "cloud-prompt-2-text", "Which TWO extra factors were identified?")],
            "slotIds": ["cloud-q21", "cloud-q22"],
            "optionBankRef": "cloud-ob-2",
            "cardinality": {"min": 2, "max": 2, "exact": 2},
            "assignment": "unordered_set",
            "scoringPolicy": "per_slot_ielts_normalized",
            "duplicatePolicy": "reject_submission",
            "allowOptionReuse": false,
            "sourceAnchors": []
        }],
        "sourceAnchors": []
    });
    draft["taskGroups"]
        .as_array_mut()
        .expect("候选必须有 taskGroups")
        .push(extra);
    for (slot, number) in [("cloud-q21", 21), ("cloud-q22", 22)] {
        draft["answerSlots"][slot] = json!({
            "slotId": slot, "questionNumber": number, "displayLabel": number.to_string(),
            "hostNodeId": "cloud-prompt-2", "hostType": "prompt", "interaction": "checkbox",
            "participation": "scoring", "sourceAnchors": [], "confidence": 0.9
        });
    }
    draft["answerKey"]["cloud-q21"] = json!({"kind": "option", "labels": ["A"], "assignment": "unordered_set"});
    draft["answerKey"]["cloud-q22"] = json!({"kind": "option", "labels": ["C"], "assignment": "unordered_set"});
    draft
}

/// 把候选规范化并落盘到独立 artifact（供 `build_repair_context` 读取）。
fn store_candidate(root: &Path, q14_label: &str) {
    store_candidate_with(root, q14_label, "D")
}

fn store_candidate_with(root: &Path, q14_label: &str, q15_label: &str) {
    store_candidate_draft(root, cloud_draft_with(q14_label, q15_label));
}

fn store_candidate_draft(root: &Path, draft: Value) {
    let canonical = golden_authoring();
    let source_sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let identity = CloudAuthoringIdentity {
        job_id: ITEM_ID,
        item_id: ITEM_ID,
        batch_id: BATCH_ID,
        source_file_id: "early-approaches-pdf",
        source_sha256,
        base_edit_version: 1,
        generated_at: "2026-09-18T00:00:00Z",
        exam: canonical.get("exam").cloned().unwrap_or(Value::Null),
        modality: "reading",
        source_document_id: "early-approaches-document",
        extraction_mode: "pdf_native",
    };
    let raw = json!({"authoring": draft});
    let normalized =
        normalize_cloud_authoring(&identity, Some(&canonical), &raw).expect("标准化必须成功");
    let candidate =
        cloud_authoring_candidate_from_normalized(&identity, normalized).expect("必须可装配");
    store::write_cloud_authoring_candidate(root, BATCH_ID, &candidate).expect("落盘候选");
}

fn set_answer(slot: &str, label: &str) -> Value {
    json!({
        "op": "setAnswer",
        "slotId": slot,
        "value": {"kind": "option", "labels": [label], "assignment": "unordered_set"}
    })
}

/// 构造一次 `record_ruling` 工具调用。
///
/// 裁定**不是**编辑：它只记录结论，不改内容。所以这个 helper 里没有 commands。
fn ruling_call(
    call_id: &str,
    target_type: &str,
    target_id: &str,
    field: &str,
    ruling: &str,
    reason: &str,
) -> Value {
    json!({
        "callId": call_id,
        "tool": "record_ruling",
        "arguments": {"rulings": [{
            "targetType": target_type,
            "targetId": target_id,
            "field": field,
            "ruling": ruling,
            "reason": reason,
            "evidence": [{
                "sourceFileId": "early-approaches-pdf",
                "pageIndex": 1,
                "quote": "14 B"
            }],
        }]},
    })
}

/// 剩余任务里是否存在某个 `userTaskId`。
fn has_task(tasks: &[Value], user_task_id: &str) -> bool {
    tasks
        .iter()
        .any(|task| task["userTaskId"].as_str() == Some(user_task_id))
}

fn first_text_in_value(value: &Value) -> Option<(String, String)> {
    if value.get("type").and_then(Value::as_str) == Some("text") {
        let id = value.get("id").and_then(Value::as_str)?.to_string();
        let text = value.get("text").and_then(Value::as_str)?.to_string();
        return Some((id, text));
    }
    match value {
        Value::Array(items) => items.iter().find_map(first_text_in_value),
        Value::Object(object) => object.values().find_map(first_text_in_value),
        _ => None,
    }
}

fn request<'a>(root: &'a Path, cancelled: &'a dyn Fn() -> bool, max_rounds: u32) -> RepairRunRequest<'a> {
    request_for_batch(root, BATCH_ID, "run-1", cancelled, max_rounds)
}

/// 跑 **legacy** 模式的循环。
///
/// 本文件里绝大多数用例断言的是**循环纪律**（取消 / 超时 / 无进展 / 终态 / 裁定落盘 /
/// 剩余任务重算），它们必须在「改造前的行为一字未变」这条基线上继续绿。默认模式已经
/// 切到 `packets`（见 `REPAIR_CONTEXT_MODE`），所以这些用例显式指定 legacy——**不是**
/// 把测试改绿，而是把模式显式化：模式一换，上下文形状就换了，不显式指定的话断言会
/// 在两种语义之间漂移。
///
/// 包模式有自己的一组用例（见文件末尾「包模式」一节）与真实 HTTP 集成用例。
fn run_legacy<F>(request: &RepairRunRequest<'_>, step: F) -> CommandResult<RepairRunReport>
where
    F: FnMut(&Value, &[Value]) -> CommandResult<Value>,
{
    run_repair_loop_in_mode(request, RepairContextMode::Legacy, step)
}

/// 跑 **packets** 模式的循环（默认模式）。
fn run_packets<F>(request: &RepairRunRequest<'_>, step: F) -> CommandResult<RepairRunReport>
where
    F: FnMut(&Value, &[Value]) -> CommandResult<Value>,
{
    run_repair_loop_in_mode(request, RepairContextMode::Packets, step)
}

fn request_for_batch<'a>(
    root: &'a Path,
    batch_id: &'a str,
    repair_run_id: &'a str,
    cancelled: &'a dyn Fn() -> bool,
    max_rounds: u32,
) -> RepairRunRequest<'a> {
    RepairRunRequest {
        root,
        item_id: ITEM_ID,
        job_id: ITEM_ID,
        batch_id,
        repair_run_id,
        max_rounds,
        deadline: Instant::now() + std::time::Duration::from_secs(30),
        cancelled,
        progress: None,
    }
}

/// 上下文必须给出整卷范围索引与真实差异（只读，不写任何东西）。
#[test]
fn repair_context_indexes_the_whole_document_and_reports_real_differences() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");

    let context = build_repair_context(&root, ITEM_ID, ITEM_ID, BATCH_ID).expect("构建上下文");

    let index = context["documentIndex"].as_array().expect("必须有范围索引");
    assert_eq!(index.len(), 1, "整卷索引必须列出全部题组");
    assert_eq!(index[0]["taskId"], "early-approaches-q14-15");
    assert_eq!(index[0]["questionNumbers"], json!([14, 15]));

    let differences = context["differences"].as_array().expect("必须有差异列表");
    let answer_difference = differences
        .iter()
        .find(|entry| entry["field"] == "answer" && entry["targetId"] == "q14")
        .expect("q14 的答案差异必须被发现");
    assert_eq!(answer_difference["canonical"]["labels"], json!(["B"]));
    assert_eq!(answer_difference["candidate"]["labels"], json!(["A"]));

    // 上下文只读：构建之后稿件与版本都没有变化。
    let conn = open_library_connection(&root).expect("打开库连接");
    let (_, version) = get_canonical_ds(&conn, ITEM_ID).expect("读 canonical").expect("已播");
    assert_eq!(version, 1, "构建上下文不得改动版本");
    let _ = std::fs::remove_dir_all(&root);
}

/// `read_draft` 按题组选择，且只回该题组覆盖的答案槽。
#[test]
fn read_draft_section_filters_by_task_group() {
    let canonical = golden_authoring();
    let section = read_draft_section(&canonical, 7, &json!({"taskGroupIds": ["early-approaches-q14-15"]}));
    assert_eq!(section["editVersion"], 7);
    assert_eq!(section["taskGroups"].as_array().unwrap().len(), 1);
    assert!(section["answerSlots"]["q14"].is_object());
    assert!(section["answerKey"]["q15"].is_object());

    let empty = read_draft_section(&canonical, 7, &json!({"taskGroupIds": ["does-not-exist"]}));
    assert_eq!(empty["taskGroups"].as_array().unwrap().len(), 0);
    assert!(empty["answerSlots"].as_object().unwrap().is_empty());
}

/// 主闭环：非法编辑被拒（收到具体错误）→ 合法编辑真的落库 → finish 不等于产品完成。
#[test]
fn repair_loop_rejects_bad_edit_then_applies_the_real_fix() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);

    let mut calls = 0u32;
    let report = run_legacy(&request, |context: &Value, _observations: &[Value]| {
        calls += 1;
        let version = context.get("editVersion").and_then(Value::as_i64).unwrap_or(0);
        Ok(match calls {
            // 第一次：漏了 baseVersion —— 必须被拒，且错误要具体到「先 read_draft」。
            1 => json!({"callId": "c1", "tool": "apply_edits",
                        "arguments": {"commands": [set_answer("q14", "A")]}}),
            2 => json!({"callId": "c2", "tool": "read_draft",
                        "arguments": {"taskGroupIds": ["early-approaches-q14-15"]}}),
            // 第三次：拿到真实 editVersion 后提交合法编辑。
            3 => json!({"callId": "c3", "tool": "apply_edits",
                        "arguments": {"baseVersion": version, "commands": [set_answer("q14", "A")]}}),
            _ => json!({"callId": "c4", "tool": "finish", "arguments": {"note": "q14 fixed"}}),
        })
    })
    .expect("修复循环必须返回结果");

    assert_eq!(report.rounds, 4, "四个回合都要被计入预算");
    assert_eq!(report.applied_count, 1, "只应有一批真实写入");
    assert_eq!(report.finish_note.as_deref(), Some("q14 fixed"));

    // 第一次调用被拒，且错误具体。
    let first = &report.observations[0];
    assert_eq!(first["status"], "rejected");
    assert!(
        first["errors"][0]
            .as_str()
            .unwrap_or("")
            .contains("CLOUD_EDIT_BASE_VERSION_MISSING"),
        "必须给出具体错误码：{:?}",
        first
    );
    // 第三次调用真的写入。
    let applied = &report.observations[2];
    assert_eq!(applied["status"], "ok");
    assert_eq!(applied["result"]["status"], "applied");

    // 落库是真的：答案变了，版本推进了。
    assert_eq!(read_answer(&root, "q14")["labels"], json!(["A"]));
    assert!(report.edit_version > 1, "真实写入必须推进版本");

    // 已修好的差异**不再**出现在剩余任务里（不能因为冻结本地稿仍有差异而复活）。
    assert!(
        !report
            .remaining_tasks
            .iter()
            .any(|task| task["userTaskId"]
                .as_str()
                .unwrap_or("")
                .contains("q14")),
        "已修好的项不得复活：{:?}",
        report.remaining_tasks
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 未知工具不执行，模型收到具体拒绝原因后可改对。
#[test]
fn repair_loop_never_executes_unknown_tools() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 4);
    let mut calls = 0u32;
    let report = run_legacy(&request, |_context: &Value, _observations: &[Value]| {
        calls += 1;
        Ok(match calls {
            1 => json!({"callId": "x1", "tool": "resolveIssue",
                        "arguments": {"issueId": "anything"}}),
            _ => json!({"callId": "x2", "tool": "finish", "arguments": {}}),
        })
    })
    .expect("修复循环必须返回结果");

    let rejected = &report.observations[0];
    assert_eq!(rejected["status"], "rejected");
    assert!(
        rejected["errors"][0]
            .as_str()
            .unwrap_or("")
            .contains("CLOUD_REPAIR_UNKNOWN_TOOL"),
        "{rejected:?}"
    );
    assert_eq!(report.applied_count, 0);
    // canonical 一字未改。
    assert_eq!(read_answer(&root, "q14")["labels"], json!(["B"]));
    let _ = std::fs::remove_dir_all(&root);
}

/// 取消：即使模型还有回合，也必须立刻停下，且状态如实。
#[test]
fn repair_loop_stops_on_cancel_and_never_claims_completion() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");

    let cancelled = || true;
    let request = request(&root, &cancelled, 6);
    let mut calls = 0u32;
    let report = run_legacy(&request, |_context: &Value, _observations: &[Value]| {
        calls += 1;
        Ok(json!({"callId": "c1", "tool": "finish", "arguments": {}}))
    })
    .expect("修复循环必须返回结果");

    assert_eq!(report.status, REPAIR_STATUS_CANCELLED);
    assert_eq!(calls, 0, "取消后不得再调用模型");
    assert_eq!(report.applied_count, 0);
    let _ = std::fs::remove_dir_all(&root);
}

/// 网关不可用：如实标 unavailable，题稿保持原样，不谎报完成。
#[test]
fn repair_loop_reports_unavailable_without_touching_the_draft() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    let report = run_legacy(&request, |_context: &Value, _observations: &[Value]| {
        Err("llm_http_500:upstream".to_string())
    })
    .expect("修复循环必须返回结果");

    assert_eq!(report.status, REPAIR_STATUS_UNAVAILABLE);
    assert_eq!(report.last_error.as_deref(), Some("llm_http_500:upstream"));
    assert_eq!(read_answer(&root, "q14")["labels"], json!(["B"]));
    let _ = std::fs::remove_dir_all(&root);
}

/// 重复的无效调用不会无限烧预算。
#[test]
fn repair_loop_stops_on_repeated_identical_calls() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 20);
    let mut calls = 0u32;
    let report = run_legacy(&request, |_context: &Value, _observations: &[Value]| {
        calls += 1;
        // 每次都是同一个「读同一段、什么都不改」的调用。
        Ok(json!({"callId": format!("c{calls}"), "tool": "read_draft",
                  "arguments": {"taskGroupIds": ["early-approaches-q14-15"]}}))
    })
    .expect("修复循环必须返回结果");

    assert!(
        calls <= REPEAT_LIMIT + 1,
        "重复且无进展的调用必须在有限回合内停下，实际 {calls} 次"
    );
    assert_ne!(report.status, REPAIR_STATUS_COMPLETED);
    let _ = std::fs::remove_dir_all(&root);
}

/// 没有 job 时 `read_source` 如实失败，绝不编造原文证据。
#[test]
fn read_source_evidence_fails_instead_of_fabricating() {
    let root = temp_root();
    ensure_app_dirs(&root).expect("ensure_app_dirs");
    let result = read_source_evidence(&root, "no-such-job", &json!({"pageIndex": 1}));
    assert!(result.is_err(), "没有原文件时必须失败，而不是返回空证据");
    let _ = std::fs::remove_dir_all(&root);
}

// ── 裁定：云端可以了结争议，不必把每条差异都变成用户任务 ──────────────────────
//
// 首遍云端候选**也只是输入**。它同样会错，而校核回合看过原文件之后是有资格推翻它的。
// 这一组用例守的就是这件事：模型说「候选错、当前稿对」之后，那条差异不能再回来找用户；
// 模型说「两边都不对」并写入第三种内容之后，用户也不该被要求回到候选；模型留下的疑问
// 即使没有表现为结构错误、也没有表现为差异，也必须留在清单里。
//
// 反过来也要守住边界：裁定**不是**编辑（内容一字不动），也**不能**消除程序发现的
// 结构错误，而且一旦内容再变，旧裁定作废重评。

/// 场景 1：候选错误，云端依据原文保留当前稿 → 该差异不再产生用户任务。
#[test]
fn a_ruling_that_the_candidate_is_wrong_retires_the_difference_for_good() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");

    // 对照组：**没有**裁定时这条差异必须出现在清单里。否则下面的断言可能是「本来就
    // 没有差异」导致的空过——那种绿灯比红灯更危险。
    let without_ruling =
        remaining_tasks(&root, ITEM_ID, ITEM_ID, BATCH_ID, &[], &[]).expect("重算剩余任务");
    assert!(
        has_task(&without_ruling, "cloud-diff:slot:q14:answer"),
        "夹具必须先真的制造出 q14 的答案差异：{without_ruling:?}"
    );

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 4);
    let mut calls = 0u32;
    let report = run_legacy(&request, |_context: &Value, _observations: &[Value]| {
        calls += 1;
        Ok(match calls {
            1 => ruling_call(
                "r1",
                "slot",
                "q14",
                "answer",
                crate::schema::cloud_repair_v1::CLOUD_RULING_CURRENT_IS_CORRECT,
                "原文件第 1 页答案为 B，当前稿正确；候选 A 是识别错误",
            ),
            _ => json!({"callId": "r2", "tool": "finish",
                        "arguments": {"note": "候选错误，保留当前稿"}}),
        })
    })
    .expect("修复循环必须返回结果");

    assert_eq!(report.adjudicated_count, 1, "必须留下一条可核对的裁定");
    assert!(
        !has_task(&report.remaining_tasks, "cloud-diff:slot:q14:answer"),
        "已裁定「当前稿对、候选错」的差异不得再问用户：{:?}",
        report.remaining_tasks
    );
    // 裁定不是编辑：内容必须一字未动。
    assert_eq!(read_answer(&root, "q14")["labels"], json!(["B"]));

    // 裁定必须落盘：重开一次不该让用户第二次回答同一个问题。
    let stored = store::read_repair_rulings(&root, ITEM_ID, BATCH_ID)
        .expect("读裁定")
        .expect("裁定是产品状态，必须落盘");
    assert_eq!(stored["rulings"].as_array().expect("rulings").len(), 1);

    // 再跑一轮（模型这次什么都不做）：差异仍然不回来。
    let mut second_calls = 0u32;
    let second = run_legacy(&request, |_context: &Value, _observations: &[Value]| {
        second_calls += 1;
        Ok(json!({"callId": "s1", "tool": "finish", "arguments": {"note": "再核一遍"}}))
    })
    .expect("第二次修复循环必须返回结果");
    assert!(
        !has_task(&second.remaining_tasks, "cloud-diff:slot:q14:answer"),
        "重新运行不得让已了结的差异复活：{:?}",
        second.remaining_tasks
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 场景 2：本地与候选均错误，云端写入第三种内容 → 不要求用户回到候选。
#[test]
fn when_both_sides_are_wrong_the_third_content_is_written_and_the_candidate_is_not_forced_back() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    let mut calls = 0u32;
    let report = run_legacy(&request, |context: &Value, _observations: &[Value]| {
        calls += 1;
        let version = context.get("editVersion").and_then(Value::as_i64).unwrap_or(0);
        Ok(match calls {
            1 => json!({"callId": "c1", "tool": "read_draft",
                        "arguments": {"taskGroupIds": ["early-approaches-q14-15"]}}),
            // 当前稿 B、候选 A 都错，原文件是 C：直接写成第三种内容。
            2 => json!({"callId": "c2", "tool": "apply_edits",
                        "arguments": {"baseVersion": version, "commands": [set_answer("q14", "C")]}}),
            // 写完之后再裁定：本地与候选都不对，差异已了结，**不必**回到候选 A。
            3 => ruling_call(
                "c3",
                "slot",
                "q14",
                "answer",
                crate::schema::cloud_repair_v1::CLOUD_RULING_CURRENT_IS_CORRECT,
                "当前稿 B 与候选 A 均与原文不符；已按原文改为 C",
            ),
            _ => json!({"callId": "c4", "tool": "finish",
                        "arguments": {"note": "已写入第三种内容 C"}}),
        })
    })
    .expect("修复循环必须返回结果");

    assert_eq!(report.applied_count, 1, "第三种内容必须真的落库");
    assert_eq!(
        read_answer(&root, "q14")["labels"],
        json!(["C"]),
        "落库的必须是原文的第三种内容，而不是候选的 A"
    );
    assert_eq!(report.adjudicated_count, 1);
    assert!(
        !has_task(&report.remaining_tasks, "cloud-diff:slot:q14:answer"),
        "两边都错、已按原文改正之后，不能再要求用户回到候选：{:?}",
        report.remaining_tasks
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 场景 3：模型报告疑问，但没有结构错误、也没有候选差异 → 疑问必须保留。
#[test]
fn a_reported_doubt_survives_even_when_nothing_else_is_wrong() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    // 候选与当前稿完全一致：既没有内容差异，也没有 blocking 质量问题。
    store_candidate(&root, "B");

    let context = build_repair_context(&root, ITEM_ID, ITEM_ID, BATCH_ID).expect("构建上下文");
    assert!(
        context["differences"].as_array().expect("差异列表").is_empty(),
        "这条用例的前提是「没有候选差异」：{:?}",
        context["differences"]
    );
    assert!(
        context["qualityIssues"]
            .as_array()
            .expect("质量问题")
            .iter()
            .all(|issue| issue["severity"] != "blocking"),
        "这条用例的前提是「没有结构错误」：{:?}",
        context["qualityIssues"]
    );

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 2);
    let report = run_legacy(&request, |_context: &Value, _observations: &[Value]| {
        Ok(json!({"callId": "q1", "tool": "finish", "arguments": {
            "note": "整卷核完",
            "unresolved": [{
                "targetId": "q14",
                "message": "第 1 页第 14 题的答案栏字形模糊，B 与 8 无法区分",
                "evidence": [{"sourceFileId": "early-approaches-pdf", "pageIndex": 1, "quote": "14 B"}]
            }]
        }}))
    })
    .expect("修复循环必须返回结果");

    let question = report
        .remaining_tasks
        .iter()
        .find(|task| {
            task["userTaskId"]
                .as_str()
                .unwrap_or("")
                .starts_with("cloud-question:")
        })
        .unwrap_or_else(|| {
            panic!(
                "程序校验通过不能消掉模型的未解疑问，实际清单：{:?}",
                report.remaining_tasks
            )
        });
    assert!(
        question["message"]
            .as_str()
            .unwrap_or("")
            .contains("字形模糊"),
        "疑问必须带着模型的原话给用户：{question:?}"
    );
    assert_eq!(
        question["action"], "review_difference",
        "有具体目标时剩余任务必须能定位过去，而不是一句空话"
    );
    assert_eq!(report.status, REPAIR_STATUS_NEEDS_ATTENTION, "有未解疑问就不能说「可以导出」");
    assert!(
        report
            .remaining_tasks
            .iter()
            .all(|task| task["blocking"] != json!(true)),
        "疑问不是结构错误，不该标成阻断：{:?}",
        report.remaining_tasks
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 裁定绑定的是**当时看到的那一对内容**：内容再变，旧裁定作废重评。
#[test]
fn a_ruling_is_re_evaluated_once_the_content_changes_again() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 4);
    // 第一轮：裁定「当前稿 B 是对的」，差异了结。
    let mut calls = 0u32;
    let first = run_legacy(&request, |_context: &Value, _observations: &[Value]| {
        calls += 1;
        Ok(match calls {
            1 => ruling_call(
                "r1",
                "slot",
                "q14",
                "answer",
                crate::schema::cloud_repair_v1::CLOUD_RULING_CURRENT_IS_CORRECT,
                "原文是 B",
            ),
            _ => json!({"callId": "r2", "tool": "finish", "arguments": {}}),
        })
    })
    .expect("第一轮必须跑完");
    assert!(
        !has_task(&first.remaining_tasks, "cloud-diff:slot:q14:answer"),
        "第一轮裁定之后差异就该了结：{:?}",
        first.remaining_tasks
    );

    // 内容又变了（这里走真实写入路径改 q14 → C）：旧裁定当时的前提不存在了。
    let mut second_calls = 0u32;
    let second = run_legacy(&request, |context: &Value, _observations: &[Value]| {
        second_calls += 1;
        let version = context.get("editVersion").and_then(Value::as_i64).unwrap_or(0);
        Ok(match second_calls {
            1 => json!({"callId": "c1", "tool": "apply_edits",
                        "arguments": {"baseVersion": version, "commands": [set_answer("q14", "C")]}}),
            _ => json!({"callId": "c2", "tool": "finish", "arguments": {}}),
        })
    })
    .expect("第二轮必须跑完");

    assert_eq!(read_answer(&root, "q14")["labels"], json!(["C"]), "第二轮必须真的改了稿");
    assert!(
        has_task(&second.remaining_tasks, "cloud-diff:slot:q14:answer"),
        "内容变了之后，基于旧内容的裁定必须作废、差异重新回到用户面前：{:?}",
        second.remaining_tasks
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ── P9：evidence.quote 必须能在完整原文文本层里找到（A-2 的落地） ────────────────

/// 编造引文的编辑整批拒绝（错误码点名是哪一条）；换成原文里真实存在的引文后落库。
///
/// 这是 A-2 在**链路层**的可执行版本：以前 `apply_edits` 只查证据结构，一句凭空编造的
/// `quote` 也会被判 `Applied`。剧本第二轮故意编一句文本层里没有的话，第三轮才引用
/// `seed_job_with_source` 文本层里真实存在的那一行。
#[test]
fn an_edit_with_a_fabricated_quote_is_rejected_and_a_real_quote_lands() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");
    seed_job_with_source(&root);

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    let mut calls = 0u32;
    let report = run_legacy(&request, |context: &Value, _observations: &[Value]| {
        calls += 1;
        let version = context.get("editVersion").and_then(Value::as_i64).unwrap_or(0);
        Ok(match calls {
            1 => json!({"callId": "c1", "tool": "read_draft",
                        "arguments": {"taskGroupIds": ["early-approaches-q14-15"]}}),
            2 => json!({"callId": "c2", "tool": "apply_edits",
                        "arguments": {"baseVersion": version, "commands": [set_answer("q14", "C")],
                                      "evidence": [{"sourceFileId": "early-approaches-pdf",
                                                    "pageIndex": 1,
                                                    "quote": "A sentence that appears nowhere in the file"}]}}),
            3 => json!({"callId": "c3", "tool": "apply_edits",
                        "arguments": {"baseVersion": version, "commands": [set_answer("q14", "C")],
                                      "evidence": [{"sourceFileId": "early-approaches-pdf",
                                                    "pageIndex": 1,
                                                    "quote": "Early approaches to organisational design."}]}}),
            _ => json!({"callId": "c4", "tool": "finish",
                        "arguments": {"note": "编造的引文被拒后，改用真实原文行"}}),
        })
    })
    .expect("修复循环必须返回结果");

    // 第一次提交被拒：错误码点名是 evidence 数组的第 0 条。
    let first_apply = report
        .observations
        .iter()
        .find(|observation| observation["callId"] == json!("c2"))
        .expect("第一次 apply_edits 的观察必须在报告里");
    assert_eq!(first_apply["status"], json!("rejected"), "{first_apply:#?}");
    assert!(
        first_apply["errors"]
            .as_array()
            .is_some_and(|errors| errors.iter().any(|error| error
                == "CLOUD_EDIT_EVIDENCE_QUOTE_NOT_IN_SOURCE:0")),
        "编造引文必须以 CLOUD_EDIT_EVIDENCE_QUOTE_NOT_IN_SOURCE:<index> 拒绝：{first_apply:#?}"
    );
    // 第二次提交（真实引文）落地；最终答案真的是 C。
    assert_eq!(report.applied_count, 1, "只有真实引文的那一批落地");
    assert_eq!(read_answer(&root, "q14")["labels"], json!(["C"]));
    let _ = std::fs::remove_dir_all(&root);
}

/// 裁定证据走同一套引文核验：编造引文的裁定**不得记录**；真实引文的裁定照常记录。
#[test]
fn a_ruling_with_a_fabricated_quote_is_rejected_and_a_grounded_one_is_recorded() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");
    // seed_packet_job：文本层第 3 页有一行真实的「14 A」。
    seed_packet_job(&root);

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    let mut calls = 0u32;
    let report = run_packets(&request, |_context: &Value, _observations: &[Value]| {
        calls += 1;
        Ok(match calls {
            1 => ruling_call(
                "r1",
                "slot",
                "q14",
                "answer",
                crate::schema::cloud_repair_v1::CLOUD_RULING_CURRENT_IS_CORRECT,
                "编造引文的一轮：这句原文不存在",
            ),
            2 => json!({
                "callId": "r2",
                "tool": "record_ruling",
                "arguments": {"rulings": [{
                    "targetType": "slot",
                    "targetId": "q14",
                    "field": "answer",
                    "ruling": crate::schema::cloud_repair_v1::CLOUD_RULING_CURRENT_IS_CORRECT,
                    "reason": "真实引文的一轮：引第 3 页文本层里的那一行",
                    "evidence": [{"sourceFileId": "early-approaches-pdf",
                                  "pageIndex": 3,
                                  "quote": "14 A"}]
                }]}
            }),
            _ => json!({"callId": "r3", "tool": "finish_packet", "arguments": {}}),
        })
    })
    .expect("包模式循环必须返回结果");

    let first_ruling = report
        .observations
        .iter()
        .find(|observation| observation["callId"] == json!("r1"))
        .expect("第一次 record_ruling 的观察必须在报告里");
    assert_eq!(first_ruling["status"], json!("rejected"), "{first_ruling:#?}");
    assert!(
        first_ruling["errors"].as_array().is_some_and(|errors| errors.iter().any(|error| error
            == "CLOUD_EDIT_EVIDENCE_QUOTE_NOT_IN_SOURCE:0:0")),
        "编造引文的裁定必须被拒（<裁定下标>:<证据下标> 都要点名）：{first_ruling:#?}"
    );

    // 第二次（真实引文）记录成功，且只有那一条进了裁定。
    assert_eq!(report.adjudicated_count, 1, "只记录了真实引文的那条裁定");
    // 落盘的裁定（rulings journal）里，证据必须带着核验标记：这里文本层存在 → verified。
    let stored = crate::reconcile::store::read_repair_rulings(&root, ITEM_ID, BATCH_ID)
        .expect("读裁定记录")
        .expect("裁定必须落盘");
    let evidence = &stored["rulings"][0]["evidence"][0];
    assert_eq!(evidence["quote"], json!("14 A"), "落盘的是剧本给出的真实引文");
    assert_eq!(
        evidence["verification"],
        json!("verified"),
        "有文本层且引文核对成功：标记必须是 verified：{stored:#?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 原文没有文本层（本用例不给 `document-ir.json`）时，裁定**照常记录**，但证据标
/// `unverifiable`——不拒绝、也不冒充「已核验」。这是扫描件的路径。
#[test]
fn rulings_recorded_without_a_text_layer_carry_the_unverifiable_mark() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");
    // 注意：不 seed 任何 job / document-ir —— 原文索引读不到，等价于扫描件没有文本层。

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 4);
    let report = run_legacy(&request, |_context: &Value, _observations: &[Value]| {
        Ok(ruling_call(
            "r1",
            "slot",
            "q14",
            "answer",
            crate::schema::cloud_repair_v1::CLOUD_RULING_CURRENT_IS_CORRECT,
            "原文是 B",
        ))
    })
    .expect("循环必须返回结果");

    assert_eq!(report.adjudicated_count, 1, "没有文本层不构成拒绝的理由");
    let stored = crate::reconcile::store::read_repair_rulings(&root, ITEM_ID, BATCH_ID)
        .expect("读裁定记录")
        .expect("裁定必须落盘");
    let evidence = &stored["rulings"][0]["evidence"][0];
    assert_eq!(
        evidence["verification"],
        json!("unverifiable"),
        "没有文本层：证据必须标 unverifiable，不能算已核验：{stored:#?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 进度上报：开工一次 `running`，**每批有效写入之后立刻再来一次**。
///
/// 这条锁的是「不要等十分钟循环结束」。修复循环的真实预算十分钟，只在结尾上报一次
/// 的话，用户在这十分钟里看不到任何变化——稿子已经改好了，画布还是旧的。缺陷的表现
/// 只是「好像有点慢」，所以必须在进度这一层钉住。
#[test]
fn progress_is_reported_at_start_and_after_every_effective_commit() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");

    let seen: std::cell::RefCell<Vec<RepairProgress>> = std::cell::RefCell::new(Vec::new());
    let sink = |progress: RepairProgress| seen.borrow_mut().push(progress);
    let not_cancelled = || false;
    let mut request = request(&root, &not_cancelled, 6);
    request.progress = Some(&sink);

    let mut calls = 0u32;
    run_legacy(&request, |context: &Value, _observations: &[Value]| {
        calls += 1;
        let version = context.get("editVersion").and_then(Value::as_i64).unwrap_or(0);
        Ok(match calls {
            1 => json!({"callId": "c1", "tool": "read_draft",
                        "arguments": {"taskGroupIds": ["early-approaches-q14-15"]}}),
            2 => json!({"callId": "c2", "tool": "apply_edits",
                        "arguments": {"baseVersion": version, "commands": [set_answer("q14", "A")]}}),
            _ => json!({"callId": "c3", "tool": "finish", "arguments": {}}),
        })
    })
    .expect("修复循环必须返回结果");

    let progress = seen.borrow();
    assert_eq!(progress.len(), 2, "开工一次 + 一批写入一次：{progress:?}");
    assert_eq!(progress[0].status, REPAIR_STATUS_RUNNING);
    assert_eq!(progress[0].applied_count, 0, "开工时还没改到东西");
    assert_eq!(progress[0].round, 0);
    // 第二次必须在**写入之后**，且带着真实的版本与已修数量。
    assert_eq!(progress[1].applied_count, 1);
    assert!(
        progress[1].edit_version > progress[0].edit_version,
        "写入后的进度必须带着推进过的版本：{progress:?}"
    );
    // 进行中的摘要不得把中间差异当成用户待办。
    assert_eq!(progress[1].to_json()["remainingTasks"], json!([]));
    assert_eq!(progress[1].to_json()["undoAvailable"], json!(false));
    let _ = std::fs::remove_dir_all(&root);
}

// ── 受控模型服务 → 真实网关 → 真实工具执行 → 真实权威稿 ─────────────────────
//
// 上面所有用例都把模型输出**直接塞进** `run_repair_loop` 的 `step`，验证的是循环与
// 工具分发。它们证明不了「模型服务那一端真的接上了」——网关的 prompt 构造、HTTP
// 往返、`validate_repair_step_output`、`repair_authoring_step_through_gateway` 里的
// profile / 主源文件 / 证据面解析，全在那条缝里。交接文档点名「云端完整识别和自主
// 编辑循环仍未接通」，指的正是这条缝，所以这里必须真起一个 HTTP 服务、真发请求。
//
// 覆盖层次（AGENTS.md 的分类）：**服务/命令处理器层**，不是 UI。
// 未覆盖：`run_job_inner` 的编排与前端界面（需要 `AppHandle`），报告里明说。

/// 起一个**有剧本**的受控修复服务。
///
/// 与 `reconcile::commands` 里那个固定应答的服务不同：修复回合是多轮的，第二轮必须
/// 用**第一轮真实读到的** `editVersion` 作 `baseVersion`，否则 CAS 会拒绝——静态样本
/// 无法预知版本号，所以这里从请求体里把 `Input JSON:` 之后的那份输入解析出来现取。
///
/// `script` 决定每一轮回什么。**按请求体现算**（而不是预置一份样本），是为了让剧本能
/// 引用真实读到的版本、真实看到的差异——预置样本做不到这两件事，于是「读稿→改稿」这条
/// 闭环就只能在测试里假装成立。
///
/// 返回 `(baseUrl, 收到的请求体)`。请求体留痕是为了断言「发出去的确实是修复请求，
/// 且带着原文件证据面」，而不是只看最终结果猜中间发生了什么。
fn spawn_scripted_repair_service_with(
    script: fn(&str, usize) -> String,
) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind controlled service");
    let addr = listener.local_addr().expect("local addr");
    let seen: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorder = seen.clone();

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            // 必须把请求体读完再回写：否则客户端还在发 body 时会收到 RST，
            // 得到一个与「受控服务」无关的传输错误，把要验证的东西掩盖掉。
            let mut request = Vec::<u8>::new();
            let mut chunk = [0u8; 4096];
            let mut header_end: Option<usize> = None;
            let mut content_length = 0usize;
            loop {
                if let Some(end) = header_end {
                    if request.len() >= end + content_length {
                        break;
                    }
                }
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(read) => {
                        request.extend_from_slice(&chunk[..read]);
                        if header_end.is_none() {
                            if let Some(position) =
                                request.windows(4).position(|window| window == b"\r\n\r\n")
                            {
                                header_end = Some(position + 4);
                                let headers =
                                    String::from_utf8_lossy(&request[..position]).to_lowercase();
                                content_length = headers
                                    .lines()
                                    .find_map(|line| line.strip_prefix("content-length:"))
                                    .and_then(|value| value.trim().parse::<usize>().ok())
                                    .unwrap_or(0);
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
            let body = String::from_utf8_lossy(&request).to_string();
            let round = {
                let mut guard = recorder.lock().expect("recorder");
                guard.push(body.clone());
                guard.len()
            };

            let content = script(&body, round);
            let envelope = json!({
                "id": "controlled-repair-0001",
                "object": "chat.completion",
                "model": "controlled-repair-v1",
                "choices": [{
                    "index": 0,
                    "finish_reason": "stop",
                    "message": {"role": "assistant", "content": content}
                }]
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                envelope.as_bytes().len(),
                envelope
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });

    (format!("http://127.0.0.1:{}/v1", addr.port()), seen)
}

fn spawn_scripted_repair_service() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    spawn_scripted_repair_service_with(scripted_repair_reply)
}

/// 从请求体里取出网关嵌进 prompt 的那份输入 JSON（`Input JSON: {...}` 之后的全部内容）。
///
/// 必须**先按 JSON 解析信封**再取文本：prompt 是 `messages[1].content` 里的一个 text
/// part，直接从原始字节里找 `Input JSON: ` 会拿到一层 `\"` 转义，解析必然失败——失败
/// 的表现是版本号取成兜底值、`apply_edits` 被 CAS 拒，于是这条用例会「跑完了但什么都没改」，
/// 看起来像产品没接通，其实是夹具没读懂请求。
fn repair_request_input(body: &str) -> Option<Value> {
    let envelope: Value = serde_json::from_str(body.get(body.find('{')?..)?).ok()?;
    let text = envelope
        .get("messages")?
        .as_array()?
        .iter()
        .flat_map(|message| {
            message
                .get("content")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[])
        })
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    let marker = "Input JSON: ";
    let at = text.rfind(marker)?;
    serde_json::from_str(text[at + marker.len()..].trim()).ok()
}

/// 剧本：先读稿 → 再按**真实读到的版本**提交一批合法编辑 → 收尾。
fn scripted_repair_reply(body: &str, round: usize) -> String {
    let input = repair_request_input(body);
    let version = input
        .as_ref()
        .and_then(|value| value.pointer("/context/editVersion"))
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    match round {
        1 => json!({
            "callId": "c1",
            "tool": "read_draft",
            "arguments": {"taskGroupIds": ["early-approaches-q14-15"]}
        }),
        2 => json!({
            "callId": "c2",
            "tool": "apply_edits",
            "arguments": {
                "baseVersion": version,
                "commands": [set_answer("q14", "A")],
                // P9 起引文要对照完整原文核验：`14 A` 只在答案页（第 3 页）的文本层里，
                // 页号必须如实声明（旧值 1 会因「找到的页与声明页差 2」被拒）。
                "evidence": [{
                    "sourceFileId": "early-approaches-pdf",
                    "pageIndex": 3,
                    "quote": "14 A"
                }]
            }
        }),
        _ => json!({
            "callId": "c3",
            "tool": "finish",
            "arguments": {"note": "受控服务：q14 已按原文件改为 A"}
        }),
    }
    .to_string()
}

/// DOCX 云端链的最小收尾剧本：不伪造编辑，只要求真实网关收到一次带原文证据的
/// 修复请求并正常结束。题面是否真实来自 DOCX 已由导入阶段断言，这里验证同一份稿件
/// 没有在「导入成功、云端却因类型被拒」的缝里断掉。
fn scripted_docx_finish_reply(_body: &str, _round: usize) -> String {
    json!({
        "callId": "docx-finish-1",
        "tool": "finish",
        "arguments": {"note": "受控服务：DOCX 原文已进入云端修复回合"}
    })
    .to_string()
}

/// 造一份带主源文件的作业（网关要读 `uploads/<storedName>` 才能附上原文件证据）。
fn seed_job_with_source(root: &Path) {
    use crate::job_store::{make_job, save_job};
    use crate::util::{ensure_job_dirs, job_dir, write_json};
    use crate::{CreateJobInput, SourceFile, WorkflowStep};

    let mut job = make_job(CreateJobInput {
        title: Some("Early Approaches".to_string()),
        category: Some("P1".to_string()),
        frequency: Some("medium".to_string()),
        tags: Some(vec!["controlled".to_string()]),
        llm_profile_id: None,
    });
    // 作业 id 必须与 item id 一致：请求里 `job_id` 就是它，网关按它 `load_job`。
    job.job_id = ITEM_ID.to_string();
    job.current_step = WorkflowStep::Authoring;
    job.active_llm_profile_id = Some("controlled-repair".to_string());
    job.source_files = vec![SourceFile {
        file_id: "early-approaches-pdf".to_string(),
        original_name: "early-approaches.pdf".to_string(),
        stored_name: "early-approaches.pdf".to_string(),
        file_type: "pdf".to_string(),
        sha256: "a".repeat(64),
        size_bytes: 8,
        role: "MainQuestion".to_string(),
        imported_at: chrono::Utc::now(),
    }];
    save_job(root, &job).expect("save job");
    let dir = job_dir(root, ITEM_ID);
    ensure_job_dirs(&dir).expect("job dirs");
    // 主源文件必须真实存在：`main_source_for_cloud` 找不到就报错，而不是静默降级成
    // 「没有证据面」——那样这条用例会退化成「只发了一句 prompt 也能过」。
    std::fs::create_dir_all(dir.join("uploads")).expect("uploads dir");
    std::fs::write(dir.join("uploads").join("early-approaches.pdf"), b"%PDF-1.4\n")
        .expect("write source");
    // 文本层带一个真实的答案页（第 3 页有一行 `14 A`）：修复剧本提交的证据引文必须
    // 能在**完整原文**文本层里核验（P9）。注意答案页里没有 `14 C` —— 「云端推翻本地、
    // 写入第三种内容 C」的那条用例靠这一点证明答案值来自模型的判断，不是文本抽取。
    write_json(
        &dir.join("document-ir.json"),
        &json!({"pages":[
            {"pageIndex":0,"lines":[{"text":"Early approaches to organisational design."}]},
            {"pageIndex":1,"lines":[{"text":"Section 2"}]},
            {"pageIndex":2,"lines":[{"text":"Answer key"},{"text":"14 A"}]}
        ]}),
    )
    .expect("document-ir");
}

/// 造一份真正由 `complex-reading.docx` 驱动的作业。与 PDF 夹具不同，这里不手写
/// `document-ir.json`：导入测试必须先走真实 DOCX 解析，云端修复随后再从原始上传件
/// 独立抽取 `sourceText`。
fn seed_docx_job_with_source(root: &Path) {
    use crate::job_store::{make_job, save_job};
    use crate::util::{ensure_job_dirs, hash_file_or_path, job_dir};
    use crate::{CreateJobInput, SourceFile, WorkflowStep};

    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/parser/complex-reading.docx");
    let (sha256, size_bytes, _) = hash_file_or_path(&fixture).expect("DOCX fixture");
    let mut job = make_job(CreateJobInput {
        title: Some("Complex reading DOCX".to_string()),
        category: Some("P1".to_string()),
        frequency: Some("medium".to_string()),
        tags: Some(vec!["controlled".to_string()]),
        llm_profile_id: None,
    });
    job.job_id = ITEM_ID.to_string();
    job.current_step = WorkflowStep::Authoring;
    job.active_llm_profile_id = Some("controlled-repair".to_string());
    job.source_files = vec![SourceFile {
        file_id: "complex-reading-docx".to_string(),
        original_name: "complex-reading.docx".to_string(),
        stored_name: "complex-reading.docx".to_string(),
        file_type: "docx".to_string(),
        sha256,
        size_bytes,
        role: "MainQuestion".to_string(),
        imported_at: chrono::Utc::now(),
    }];
    save_job(root, &job).expect("save DOCX job");
    let dir = job_dir(root, ITEM_ID);
    ensure_job_dirs(&dir).expect("DOCX job dirs");
    std::fs::copy(
        &fixture,
        dir.join("uploads").join("complex-reading.docx"),
    )
    .expect("copy DOCX source");
}

/// 同一份真实 DOCX 必须从导入一路走到云端修复网关：不是只证明本地能生成稿件，
/// 也不是只调用 `main_source_for_cloud` 的类型分支。这个测试把产品边界钉在：
/// 导入的物理稿 / V2 会话 → 首次 canonical → 本地批次 → 云端候选 → 真实 HTTP 网关。
#[test]
fn real_docx_import_reaches_cloud_repair_with_original_source_evidence() {
    use crate::auto_pipeline::{
        finalize_cloud_authoring_candidate, run_auto_pipeline_core,
    };
    use crate::authoring_v2_commands::get_authoring_v2_core;
    use crate::library::migration::ensure_initial_canonical;
    use crate::processing::scheduler::run_local_only_recognition_cycle;

    let root = temp_root();
    crate::util::ensure_app_dirs(&root).expect("app dirs");
    seed_docx_job_with_source(&root);

    let report = run_auto_pipeline_core(
        &root,
        ITEM_ID,
        Some(crate::AutoPipelineInput {
            execution_mode: Some("localOnly".to_string()),
            target: Some("editableDraft".to_string()),
            allow_overwrite: Some(true),
            ..Default::default()
        }),
    )
    .expect("真实 DOCX 导入必须完成");
    assert!(report.get("status").and_then(Value::as_str).is_some());

    let session = get_authoring_v2_core(&root, ITEM_ID).expect("DOCX V2 session");
    let imported_authoring = session
        .get("authoring")
        .cloned()
        .expect("DOCX session authoring");
    let responses = imported_authoring
        .get("taskGroups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|group| {
            group
                .get("responseGroups")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 2);
    for response in &responses {
        let (_, text) = first_text_in_value(response.get("prompt").unwrap())
            .expect("DOCX prompt must contain real text before cloud repair");
        assert!(!text.trim().is_empty());
        assert!(!text.contains("pending review"));
    }

    assert!(ensure_initial_canonical(&root, ITEM_ID).expect("seed imported DOCX canonical"));
    let base_edit_version = canonical_version(&root);
    run_local_only_recognition_cycle(&root, ITEM_ID, base_edit_version)
        .expect("DOCX local recognition batch must complete");
    let batch_id = batch_id_for(&root, base_edit_version);

    // Candidate 由刚刚导入的同一份 V2 稿件组成；云端修复仍要经过身份绑定、质量重算和
    // 独立 candidate artifact，不能把 canonical 直接当作修复循环输入。
    finalize_cloud_authoring_candidate(
        &root,
        ITEM_ID,
        &batch_id,
        base_edit_version,
        &json!({"authoring": imported_authoring}),
    )
    .expect("DOCX candidate must enter the repair batch");

    let requests = start_repair_service(&root, scripted_docx_finish_reply);
    let not_cancelled = || false;
    let request = request_for_batch(&root, &batch_id, "run-docx-repair", &not_cancelled, 2);
    let repair = run_legacy(&request, |context, observations| {
        repair_authoring_step_through_gateway(
            &root,
            ITEM_ID,
            Some("controlled-repair"),
            context,
            observations,
        )
    })
    .expect("DOCX repair must return a terminal report");
    assert!(
        repair.status == REPAIR_STATUS_COMPLETED
            || repair.status == REPAIR_STATUS_NEEDS_ATTENTION,
        "DOCX must reach the repair loop, not fail as unavailable: {repair:?}"
    );
    assert_eq!(repair.rounds, 1, "controlled DOCX service should finish in one round");
    assert!(repair.last_error.is_none(), "DOCX repair transport must succeed: {repair:?}");

    let seen = requests.lock().expect("requests");
    assert_eq!(seen.len(), 1, "one real DOCX repair request expected");
    // The evidence is the dedicated source-text block; the prompt's Input JSON no
    // longer repeats the same text a second time.
    assert!(
        seen[0].contains("SOURCE TEXT BEGIN"),
        "DOCX repair must send source text evidence"
    );
    assert_eq!(
        seen[0].matches("SOURCE TEXT BEGIN").count(),
        1,
        "the source text must be attached exactly once"
    );
    assert!(
        seen[0].contains("complex-reading-docx") || seen[0].contains("complex-reading.docx"),
        "DOCX repair request must identify the original source: {}",
        seen[0]
    );
    assert!(!seen[0].contains("main_source_is_not_pdf"));

    let _ = std::fs::remove_dir_all(&root);
}

/// 受控服务 → 真实网关 → 真实工具执行 → 真实权威稿。
#[test]
fn controlled_model_service_drives_a_real_repair_round_through_the_real_gateway() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");
    seed_job_with_source(&root);

    let (base_url, requests) = spawn_scripted_repair_service();
    crate::llm_profiles::save_profiles(
        &root,
        &[json!({
            "profileId": "controlled-repair",
            "name": "Controlled Repair Service",
            "provider": "OpenAiCompatible",
            "baseUrl": base_url,
            "model": "controlled-repair-v1",
            "temperature": 0,
            "timeoutMs": 60000,
            "forceJson": true,
            "enabled": true
        })],
    )
    .expect("profile 必须能落盘，否则网关取不到 baseUrl");

    let before = read_answer(&root, "q14");
    let version_before = {
        let conn = open_library_connection(&root).expect("打开库连接");
        let (_, version) = get_canonical_ds(&conn, ITEM_ID).expect("读 canonical").expect("已播");
        version
    };
    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    let report = run_legacy(&request, |context: &Value, observations: &[Value]| {
        repair_authoring_step_through_gateway(
            &root,
            ITEM_ID,
            Some("controlled-repair"),
            context,
            observations,
        )
    })
    .expect("修复循环必须跑完");

    // 1) 请求真的到了受控服务（而不是「循环自己以为调用了模型」）。
    let seen = requests.lock().expect("requests");
    assert!(
        seen.len() >= 2,
        "至少要有 read_draft 与 apply_edits 两轮真实 HTTP 请求，实际 {}",
        seen.len()
    );
    assert!(
        seen[0].contains("repair_authoring_step") || seen[0].contains("You are repairing"),
        "发出去的必须是修复回合的 prompt，实际首轮请求：{}",
        &seen[0][..seen[0].len().min(400)]
    );
    // 证据面：PDF 必须以附件形式带上原文件，而不是只发文字。
    assert!(
        seen[0].contains("application/pdf"),
        "修复回合必须把原文件作为证据附上；只发 prompt 就等于让模型凭空猜"
    );
    // 第二轮必须带上第一轮的真实观察结果（`read_draft` 的返回），否则「读稿→改稿」
    // 这条闭环是假的。
    assert!(
        seen[1].contains("read_draft") || seen[1].contains("CloudRepairToolResultV1"),
        "第二轮必须把上一轮工具的真实结果回传给模型"
    );
    drop(seen);

    // 2) 真实工具执行：权威稿真的被改了（不是只产生了一份「建议」）。
    let after = read_answer(&root, "q14");
    assert_ne!(after, before, "受控服务提交的编辑必须真的落到权威稿上");
    assert_eq!(after["labels"], json!(["A"]), "落库的必须是模型提交的那个值");

    // 3) 报告如实反映「改了、但未必改完」：finish 不是产品完成。
    assert!(
        report.applied_count > 0,
        "至少有一批编辑被真实应用，实际 {}",
        report.applied_count
    );
    assert_eq!(
        report.repair_run_id, "run-1",
        "修复运行归属必须原样带出，前端撤销依赖它"
    );
    assert!(
        report.edit_version > version_before,
        "编辑版本必须推进（{version_before} -> {}），否则「改了稿」这句话没有证据",
        report.edit_version
    );

    let _ = std::fs::remove_dir_all(&root);
}

// ── 真实调度分支：本地周期 → 完整候选 → 云端修复 → 真实事务写入 ─────────────
//
// 上面那条受控服务用例证明的是「网关那一端真的接上了」。这一组再往前一步：按**生产的
// 调用顺序**把 `processing/scheduler.rs` 主链上的四个真实函数串起来跑，证明
// 「云端可以纠正本地的错误结论」——不是「修复函数自己能跑」。
//
// 为什么不能直接调用修复函数代替这条链：`finalize_cloud_authoring_candidate` 负责给候选
// 接身份、`run_local_only_recognition_cycle` 负责建批次行与本地候选。跳过它们，测的就
// 只剩一个孤立的循环；而缺陷恰恰长在这些接缝上。
//
// 覆盖层次（AGENTS.md 的分类）：**服务/命令处理器层**，不是 UI。`run_job_inner` 需要
// `AppHandle`，因此「事件真的发到前端」只能由 CDP 那条真机脚本证明，报告里明说。

/// 剧本：读稿 → 按原文件把 q14 改成 C → 收尾。
///
/// C 是**本地与候选之外的第三种内容**也无所谓，这里的关键是「本地是 B、原文件是 C」：
/// 模型必须能推翻本地结论，而不是只能在候选与当前稿之间二选一。
/// 证据引文取自夹具文本层里**真实存在**的那一行（P9 起引文要对照原文核验；
/// 编造的引文整批拒绝），而答案值 C 只能来自模型的判断——文本层里没有答案行。
fn scripted_third_answer_reply(body: &str, round: usize) -> String {
    let input = repair_request_input(body);
    let version = input
        .as_ref()
        .and_then(|value| value.pointer("/context/editVersion"))
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    match round {
        1 => json!({
            "callId": "c1",
            "tool": "read_draft",
            "arguments": {"taskGroupIds": ["early-approaches-q14-15"]}
        }),
        2 => json!({
            "callId": "c2",
            "tool": "apply_edits",
            "arguments": {
                "baseVersion": version,
                "commands": [set_answer("q14", "C")],
                "evidence": [{
                    "sourceFileId": "early-approaches-pdf",
                    "pageIndex": 1,
                    "quote": "Early approaches to organisational design."
                }]
            }
        }),
        _ => json!({
            "callId": "c3",
            "tool": "finish",
            "arguments": {"note": "按原文件把 q14 改成 C"}
        }),
    }
    .to_string()
}

/// 剧本：读稿 → 试图改 q14 → 收尾（改不动也要如实收尾，不能卡死）。
fn scripted_edit_attempt_reply(body: &str, round: usize) -> String {
    let input = repair_request_input(body);
    let version = input
        .as_ref()
        .and_then(|value| value.pointer("/context/editVersion"))
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    match round {
        1 => json!({
            "callId": "c1",
            "tool": "read_draft",
            "arguments": {"taskGroupIds": ["early-approaches-q14-15"]}
        }),
        2 => json!({
            "callId": "c2",
            "tool": "apply_edits",
            "arguments": {"baseVersion": version, "commands": [set_answer("q14", "C")]}
        }),
        _ => json!({"callId": "c3", "tool": "finish", "arguments": {"note": "改不动，交给用户"}}),
    }
    .to_string()
}

/// 重试场景：先尝试改动用户刚编辑过的 q14（必须被保护拒绝），再只提交新识别带来的
/// q15 改进。这样测试既证明人工编辑不会被覆盖，也证明同一轮里其它可安全落地的改进
/// 不会因为一个受保护目标而一起丢失。
fn scripted_retry_edit_reply(body: &str, round: usize) -> String {
    let input = repair_request_input(body);
    let version = input
        .as_ref()
        .and_then(|value| value.pointer("/context/editVersion"))
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    match round {
        1 => json!({
            "callId": "retry-c1",
            "tool": "read_draft",
            "arguments": {"taskGroupIds": ["early-approaches-q14-15"]}
        }),
        2 => json!({
            "callId": "retry-c2",
            "tool": "apply_edits",
            "arguments": {"baseVersion": version, "commands": [set_answer("q14", "C")]}
        }),
        3 => json!({
            "callId": "retry-c3",
            "tool": "apply_edits",
            "arguments": {
                "baseVersion": version,
                "commands": [set_answer("q15", "E")],
                "evidence": [{
                    "sourceFileId": "early-approaches-pdf",
                    "pageIndex": 1,
                    "quote": "Early approaches to organisational design."
                }]
            }
        }),
        _ => json!({
            "callId": "retry-c4",
            "tool": "finish",
            "arguments": {"note": "q14 保留人工编辑，q15 采用新识别结果"}
        }),
    }
    .to_string()
}

/// 剧本：读稿 → 修正**选项库**与**作答结构**（都是已有题组内的结构写入）→ 收尾。
///
/// 为什么必须验结构而不只是答案：云端独立识别最常见的偏差不是「答案选错一个字母」，
/// 而是「选项文字读错」「作答区提示读错」。只验 `setAnswer` 的话，
/// `setOptionBank` / `setResponseGroup` 这两条真正承载结构的路径一次都没被真实执行过。
///
/// 剧本有意复刻真实情形：**改写结构时必须把原有的来源依据一并带回来**。`setOptionBank`
/// 是整块替换，漏掉 `sourceAnchors` 就等于把「这段文字出自原文件哪一页」抹掉，质量门禁
/// 会逐个点名拒绝。所以第二轮先漏、第三轮从 `read_draft` 的**真实返回**里把依据取回来。
fn scripted_structure_fix_reply(body: &str, round: usize) -> String {
    let input = repair_request_input(body);
    let version = input
        .as_ref()
        .and_then(|value| value.pointer("/context/editVersion"))
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    // 从上一轮的真实观察里取回某个对象的来源依据——这正是模型手里能拿到的东西。
    let anchors = |pointer: &str| -> Value {
        input
            .as_ref()
            .and_then(|value| value.pointer(&format!("/observations/0/result{pointer}")))
            .cloned()
            .unwrap_or_else(|| json!([]))
    };
    let complete = round >= 3;
    let text_node = |id: &str, text: &str| {
        json!({
            "type": "text",
            "id": id,
            "sourceAnchors": [],
            "provenanceStatus": "source",
            "text": text
        })
    };
    let option = |index: usize, label: &str, text: &str| {
        json!({
            "optionId": format!("option-{}", label.to_lowercase()),
            "label": label,
            "content": [text_node(&format!("option-{}-text", label.to_lowercase()), text)],
            // 整块替换：依据必须带回来，否则「这段文字出自哪一页」就没了。
            "sourceAnchors": if complete {
                anchors(&format!("/taskGroups/0/optionBank/options/{index}/sourceAnchors"))
            } else {
                json!([])
            }
        })
    };
    let options: Vec<Value> = vec![
        option(0, "A", "factor A"),
        // 原文件里 B 的措辞是 "factor B (revised)"：本地读漏了括号部分。
        option(1, "B", "factor B (revised)"),
        option(2, "C", "factor C"),
        option(3, "D", "factor D"),
        option(4, "E", "factor E"),
    ];
    match round {
        1 => json!({
            "callId": "c1",
            "tool": "read_draft",
            "arguments": {"taskGroupIds": ["early-approaches-q14-15"]}
        }),
        2 | 3 => json!({
            "callId": format!("c{round}"),
            "tool": "apply_edits",
            "arguments": {
                "baseVersion": version,
                "commands": [
                    {
                        "op": "setOptionBank",
                        "taskId": "early-approaches-q14-15",
                        "optionBank": {
                            "optionBankId": "early-approaches-options",
                            "scope": "task_group",
                            "options": options,
                            "allowReuse": false,
                            "sourceAnchors": if complete {
                                anchors("/taskGroups/0/optionBank/sourceAnchors")
                            } else {
                                json!([])
                            }
                        }
                    },
                    {
                        "op": "setResponseGroup",
                        "taskId": "early-approaches-q14-15",
                        "responseGroup": {
                            "responseGroupId": "early-approaches-shared-response",
                            "kind": "choice",
                            "prompt": [{
                                "type": "paragraph",
                                "id": "early-approaches-shared-prompt",
                                "sourceAnchors": if complete {
                                    anchors("/taskGroups/0/responseGroups/0/prompt/0/sourceAnchors")
                                } else {
                                    json!([])
                                },
                                "provenanceStatus": "source",
                                "children": [text_node(
                                    "early-approaches-shared-prompt-text",
                                    "Which TWO factors shaped early organisational design?"
                                )]
                            }],
                            "slotIds": ["q14", "q15"],
                            "optionBankRef": "early-approaches-options",
                            "cardinality": {"min": 2, "max": 2, "exact": 2},
                            "assignment": "unordered_set",
                            "scoringPolicy": "per_slot_ielts_normalized",
                            "duplicatePolicy": "reject_submission",
                            "allowOptionReuse": false,
                            "sourceAnchors": if complete {
                                anchors("/taskGroups/0/responseGroups/0/sourceAnchors")
                            } else {
                                json!([])
                            }
                        }
                    }
                ],
                "evidence": [{
                    "sourceFileId": "early-approaches-pdf",
                    "pageIndex": 1,
                    "quote": "Early approaches to organisational design."
                }]
            }
        }),
        _ => json!({"callId": "c4", "tool": "finish", "arguments": {"note": "选项与作答结构已按原文件修正"}}),
    }
    .to_string()
}

/// 剧本：读稿 → 试图**凭空新增一整组题**（两遍，第二遍补上 sourceAnchors）→ 收尾。
///
/// 这个剧本存在的意义是记录一条**权限边界**，而不是记录一次失败：
/// `upsertTaskGroupBundle` 在实现里把 `sourceAnchors` / `evidenceAnchors` 强制写成空数组，
/// 而模型又**没有** `bindSource`（`MODEL_ALLOWED_OPS` 有意排除）。因此模型**结构上不可能**
/// 造出一个带来源依据的新题组，质量门禁会如实拒绝它。
///
/// 这决定了一件产品事实：**「云端补上本地漏掉的整组题」目前做不到**，正确的行为是
/// 模型把这件事作为未解疑问留给用户，而不是硬写进去制造一份无法发布、看起来却已完成的稿。
fn scripted_new_task_group_reply(body: &str, round: usize) -> String {
    let input = repair_request_input(body);
    let version = input
        .as_ref()
        .and_then(|value| value.pointer("/context/editVersion"))
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    // 第一遍缺 `sourceAnchors`（schema 直接拒），第二遍补上（过 schema，但过不了质量门禁）。
    let complete = round >= 3;
    match round {
        1 => json!({
            "callId": "c1",
            "tool": "read_draft",
            "arguments": {"questionNumbers": [14, 15]}
        }),
        2 | 3 => json!({
            "callId": format!("c{round}"),
            "tool": "apply_edits",
            "arguments": {
                "baseVersion": version,
                "commands": [new_task_group_bundle(complete)],
                "evidence": [{
                    "sourceFileId": "early-approaches-pdf",
                    "pageIndex": 1,
                    "quote": "Early approaches to organisational design."
                }]
            }
        }),
        _ => json!({
            "callId": "c4",
            "tool": "finish",
            "arguments": {
                "note": "新增题组被质量门禁拒绝，留给用户",
                "unresolved": [{
                    "message": "原文件里 16-17 题似乎是一整组新题，但云端没有来源绑定权限，无法新增"
                }]
            }
        }),
    }
    .to_string()
}

/// `upsertTaskGroupBundle` 的载荷。
///
/// `complete = false` 时**省略** `sourceAnchors`——这正是模型第一次提交时的真实样子。
/// 结构类补丁的必填字段必须逐个补齐（任务组 / 指令 / 选项库 / 响应组 / 选项各自都要），
/// 后端会指名缺哪一个，模型据此改对。
fn new_task_group_bundle(complete: bool) -> Value {
    let anchors = || json!([]);
    let node = |id: &str, text: &str| {
        let mut object = serde_json::Map::new();
        object.insert("type".to_string(), json!("text"));
        object.insert("id".to_string(), json!(id));
        object.insert("text".to_string(), json!(text));
        if complete {
            object.insert("sourceAnchors".to_string(), anchors());
            object.insert("provenanceStatus".to_string(), json!("source"));
        }
        Value::Object(object)
    };
    let paragraph = |id: &str, child: &str, text: &str| {
        let mut object = serde_json::Map::new();
        object.insert("type".to_string(), json!("paragraph"));
        object.insert("id".to_string(), json!(id));
        object.insert("children".to_string(), json!([node(child, text)]));
        if complete {
            object.insert("sourceAnchors".to_string(), anchors());
            object.insert("provenanceStatus".to_string(), json!("source"));
        }
        Value::Object(object)
    };
    let options: Vec<Value> = ["A", "B", "C"]
        .iter()
        .map(|label| {
            let mut object = serde_json::Map::new();
            object.insert("optionId".to_string(), json!(format!("cloud-new-option-{label}")));
            object.insert("label".to_string(), json!(label));
            object.insert(
                "content".to_string(),
                json!([node(&format!("cloud-new-option-{label}-text"), &format!("new factor {label}"))]),
            );
            if complete {
                object.insert("sourceAnchors".to_string(), anchors());
            }
            Value::Object(object)
        })
        .collect();

    let mut task_group = serde_json::Map::new();
    task_group.insert("taskType".to_string(), json!("multiple_choice"));
    task_group.insert(
        "instructions".to_string(),
        json!([paragraph("cloud-new-instructions", "cloud-new-instructions-text", "Choose TWO letters, A-C.")]),
    );
    let mut option_bank = serde_json::Map::new();
    option_bank.insert("optionBankId".to_string(), json!("cloud-new-options"));
    option_bank.insert("scope".to_string(), json!("task_group"));
    option_bank.insert("options".to_string(), json!(options));
    option_bank.insert("allowReuse".to_string(), json!(false));
    if complete {
        option_bank.insert("sourceAnchors".to_string(), anchors());
    }
    task_group.insert("optionBank".to_string(), Value::Object(option_bank));
    let mut response_group = serde_json::Map::new();
    response_group.insert("kind".to_string(), json!("choice"));
    response_group.insert(
        "prompt".to_string(),
        json!([paragraph("cloud-new-prompt", "cloud-new-prompt-text", "Which TWO new factors were identified?")]),
    );
    response_group.insert("slotIds".to_string(), json!(["q16", "q17"]));
    response_group.insert("optionBankRef".to_string(), json!("cloud-new-options"));
    response_group.insert("cardinality".to_string(), json!({"min": 2, "max": 2, "exact": 2}));
    response_group.insert("assignment".to_string(), json!("unordered_set"));
    if complete {
        response_group.insert("sourceAnchors".to_string(), anchors());
    }
    task_group.insert("responseGroups".to_string(), json!([Value::Object(response_group)]));
    if complete {
        task_group.insert("sourceAnchors".to_string(), anchors());
    }

    json!({
        "op": "upsertTaskGroupBundle",
        "taskGroup": Value::Object(task_group),
        "answerSlots": [
            {"slotId": "q16", "questionNumber": 16, "interaction": "checkbox"},
            {"slotId": "q17", "questionNumber": 17, "interaction": "checkbox"}
        ],
        "answerKey": {
            "q16": {"kind": "option", "labels": ["A"], "assignment": "unordered_set"},
            "q17": {"kind": "option", "labels": ["C"], "assignment": "unordered_set"}
        }
    })
}

/// 起服务 + 落 profile（真实网关按 profile 取 baseUrl）。
fn start_repair_service(
    root: &Path,
    script: fn(&str, usize) -> String,
) -> std::sync::Arc<std::sync::Mutex<Vec<String>>> {
    let (base_url, requests) = spawn_scripted_repair_service_with(script);
    crate::llm_profiles::save_profiles(
        root,
        &[json!({
            "profileId": "controlled-repair",
            "name": "Controlled Repair Service",
            "provider": "OpenAiCompatible",
            "baseUrl": base_url,
            "model": "controlled-repair-v1",
            "temperature": 0,
            "timeoutMs": 60000,
            "forceJson": true,
            "enabled": true
        })],
    )
    .expect("profile 必须能落盘，否则网关取不到 baseUrl");
    requests
}

fn batch_id_for(root: &Path, base_edit_version: i64) -> String {
    let source_sha256 = crate::reconcile::commands::source_sha256_for_job(root, ITEM_ID);
    crate::reconcile::commands::recognition_batch_id(ITEM_ID, &source_sha256, base_edit_version)
}

fn canonical_version(root: &Path) -> i64 {
    let conn = open_library_connection(root).expect("打开库连接");
    let (_, version) = get_canonical_ds(&conn, ITEM_ID).expect("读 canonical").expect("已播");
    version
}

/// 场景：**本地规则识别错、云端识别对**。要求云端自动改对并落库，全程没有用户点过接受。
#[test]
fn cloud_repair_overrules_a_wrong_local_answer_through_the_real_chain() {
    use crate::auto_pipeline::finalize_cloud_authoring_candidate;
    use crate::processing::scheduler::run_local_only_recognition_cycle;

    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    seed_job_with_source(&root);
    let _requests = start_repair_service(&root, scripted_third_answer_reply);

    // 前提：本地结论是 B（错），原文件的文本层里**没有**任何可读的答案行。
    // 没有答案行这件事很重要：它保证 C 只可能来自模型的判断，而不是某条确定性抽取。
    let base_edit_version = canonical_version(&root);
    assert_eq!(read_answer(&root, "q14")["labels"], json!(["B"]));
    let document_ir = std::fs::read_to_string(
        crate::util::job_dir(&root, ITEM_ID).join("document-ir.json"),
    )
    .expect("document-ir");
    assert!(
        !document_ir.contains("14 C"),
        "夹具必须先保证原文件的文本层里没有这条答案，否则「云端推翻本地」无从谈起"
    );

    // ① 真实本地周期：建批次行 + 落本地候选 / 决策证据。
    let cycle = run_local_only_recognition_cycle(&root, ITEM_ID, base_edit_version)
        .expect("本地周期必须跑完");
    assert_eq!(cycle.reconcile_status, "succeeded", "本地周期本身必须成功");
    assert_eq!(
        read_answer(&root, "q14")["labels"],
        json!(["B"]),
        "本地周期不得改动一个**非空**答案（旧策略只补空）"
    );
    let batch_id = batch_id_for(&root, base_edit_version);
    let local = store::read_candidate(&root, ITEM_ID, &batch_id, store::LOCAL_CANDIDATE_FILE)
        .expect("本地候选必须真正落盘");
    let local_q14 = local
        .slots
        .iter()
        .find(|slot| slot.slot_id == "q14")
        .and_then(|slot| slot.answer.as_ref())
        .unwrap_or_else(|| panic!("本地候选必须带 q14 的答案：{:?}", local.slots));
    assert_eq!(
        local_q14.get("labels"),
        Some(&json!(["B"])),
        "本地候选必须带着那个错误结论，否则这条用例什么都没证明"
    );

    // ② 完整候选接身份 + 独立落盘：候选**不写**权威稿。
    let raw = json!({"authoring": cloud_draft("C")});
    finalize_cloud_authoring_candidate(&root, ITEM_ID, &batch_id, base_edit_version, &raw)
        .expect("候选必须能接身份并落盘");
    assert_eq!(
        read_answer(&root, "q14")["labels"],
        json!(["B"]),
        "云端候选绝不能写权威稿——它只是输入"
    );

    // ③ 真实修复循环：真实网关 → 受控服务 → 真实工具执行 → 真实事务写入。
    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    let report = run_legacy(&request, |context, observations| {
        repair_authoring_step_through_gateway(
            &root,
            ITEM_ID,
            Some("controlled-repair"),
            context,
            observations,
        )
    })
    .expect("修复循环必须跑完");

    assert_eq!(report.applied_count, 1, "必须有一批真实写入");
    assert_eq!(
        read_answer(&root, "q14")["labels"],
        json!(["C"]),
        "云端必须把本地的错误结论改成原文件的 C——没有任何用户点击参与"
    );
    assert!(
        report.edit_version > base_edit_version,
        "写入必须推进版本（{base_edit_version} -> {}）",
        report.edit_version
    );
    // 差异已被云端自己了结：不再有需要用户处理的 q14 任务。
    assert!(
        !has_task(&report.remaining_tasks, "cloud-diff:slot:q14:answer"),
        "云端已经改对，就不该再把这条差异丢回给用户：{:?}",
        report.remaining_tasks
    );

    // ④ 再跑一次本地周期：本地候选仍是 B、权威稿是 C，守卫必须拒绝把 C 改回 B。
    // 这一条在产品上真实存在——用户点「重新识别」或任务重试都会再跑一遍这条链。
    let second = run_local_only_recognition_cycle(&root, ITEM_ID, base_edit_version)
        .expect("本地周期必须可重复跑");
    assert_eq!(second.reconcile_status, "succeeded");
    assert_eq!(
        read_answer(&root, "q14")["labels"],
        json!(["C"]),
        "修复后的内容不得被本地周期改回去"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 人工编辑保护在这条链上仍然有效：云端**改不动**人改过的地方，且必须收到具体原因。
#[test]
fn cloud_repair_cannot_overwrite_a_human_edited_target_on_the_real_chain() {
    use crate::auto_pipeline::finalize_cloud_authoring_candidate;
    use crate::processing::scheduler::run_local_only_recognition_cycle;

    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    seed_job_with_source(&root);
    let _requests = start_repair_service(&root, scripted_edit_attempt_reply);

    let base_edit_version = canonical_version(&root);
    let _ = run_local_only_recognition_cycle(&root, ITEM_ID, base_edit_version).expect("本地周期");
    let batch_id = batch_id_for(&root, base_edit_version);
    let raw = json!({"authoring": cloud_draft("C")});
    finalize_cloud_authoring_candidate(&root, ITEM_ID, &batch_id, base_edit_version, &raw)
        .expect("候选落盘");

    // 用户改过 q14：写入人工保护目标（v5 起由人工编辑事务同事务维护）。
    {
        let conn = open_library_connection(&root).expect("打开库连接");
        conn.execute(
            "UPDATE library_items_v2 SET protected_edits_json = ?1 WHERE id = ?2",
            rusqlite::params![json!({"targets": ["q14"]}).to_string(), ITEM_ID],
        )
        .expect("写入 protected_edits_json");
    }

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    let report = run_legacy(&request, |context, observations| {
        repair_authoring_step_through_gateway(
            &root,
            ITEM_ID,
            Some("controlled-repair"),
            context,
            observations,
        )
    })
    .expect("修复循环必须跑完");

    assert_eq!(report.applied_count, 0, "受保护目标不得有任何写入");
    assert_eq!(
        read_answer(&root, "q14")["labels"],
        json!(["B"]),
        "人工编辑过的答案必须原样保留"
    );
    // 拒绝理由必须**具体到目标**：模型要据此缩小修复范围，而不是空转重试。
    let rejected = report
        .observations
        .iter()
        .find(|observation| observation["status"] == "rejected")
        .unwrap_or_else(|| panic!("必须有被拒的观察：{:?}", report.observations));
    assert!(
        rejected["errors"]
            .as_array()
            .map(|errors| errors.iter().any(|error| error
                .as_str()
                .unwrap_or("")
                .contains("EDIT_PROTECTED_TARGET")))
            .unwrap_or(false),
        "拒绝理由必须具体：{rejected:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 真实「重新识别」收口：新的 attempt 产生新的本地候选与修复批次，人工改过的 q14
/// 仍然受保护，而同一候选里可安全落地的 q15 改进经唯一云端写入入口落库。
#[test]
fn retry_recognition_preserves_human_edit_and_applies_a_new_improvement() {
    use crate::auto_pipeline::finalize_cloud_authoring_candidate;
    use crate::authoring_v2_commands::{apply_patch, refresh_quality_report, validate_authoring};
    use crate::library::repository::{
        apply_editor_commands_tx_with, ApplyEditorCommandsInput, EditOrigin,
    };
    use crate::processing::scheduler::{
        freeze_local_candidate_snapshot_for_attempt, run_local_only_recognition_cycle,
        run_local_only_recognition_cycle_for_batch,
    };
    use crate::reconcile::commands::recognition_batch_id_for_attempt;
    use crate::util::{job_dir, write_json};

    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    seed_job_with_source(&root);

    // First attempt: establish the old batch identity before the author edits q14.
    let first_base_version = canonical_version(&root);
    run_local_only_recognition_cycle(&root, ITEM_ID, first_base_version)
        .expect("初次本地识别周期必须完成");
    let first_batch = batch_id_for(&root, first_base_version);

    // Use the real editor transaction, not a direct protected_edits_json mutation.  This is the
    // same origin the UI save path supplies, so the target is protected as a product side effect.
    {
        let mut conn = open_library_connection(&root).expect("打开库连接");
        apply_editor_commands_tx_with(
            &mut conn,
            &ApplyEditorCommandsInput {
                item_id: ITEM_ID.to_string(),
                base_version: first_base_version,
                request_id: Some("human-retry-edit".to_string()),
                commands: vec![set_answer("q14", "A")],
                title: None,
            },
            EditOrigin::Human,
            None,
            &apply_patch,
            &|ds| {
                refresh_quality_report(&root, ITEM_ID, ds)?;
                validate_authoring(ds)
            },
            &|_, _| Ok(()),
        )
        .expect("人工编辑必须经正式事务落库");
    }
    assert_eq!(read_answer(&root, "q14")["labels"], json!(["A"]));

    // Controlled fresh recognition output: q14 conflicts with the human edit, q15 contains a
    // new candidate improvement.  The scheduler's real freeze function reads this V2 shadow and
    // writes it as the retry-local candidate without replacing canonical.
    let mut fresh_shadow = golden_authoring();
    fresh_shadow["answerKey"]["q14"]["labels"] = json!(["C"]);
    fresh_shadow["answerKey"]["q15"]["labels"] = json!(["E"]);
    write_json(
        &job_dir(&root, ITEM_ID).join(crate::authoring_v2_commands::AUTHORING_V2_SHADOW_FILE),
        &fresh_shadow,
    )
    .expect("fresh recognition shadow");

    let retry_base_version = canonical_version(&root);
    let frozen_version = freeze_local_candidate_snapshot_for_attempt(&root, ITEM_ID, 1)
        .expect("重试候选冻结必须成功");
    assert_eq!(frozen_version, retry_base_version);
    let source_sha256 = crate::reconcile::commands::source_sha256_for_job(&root, ITEM_ID);
    let retry_batch = recognition_batch_id_for_attempt(
        ITEM_ID,
        &source_sha256,
        retry_base_version,
        1,
    );
    assert_ne!(retry_batch, first_batch, "重试必须使用新的 batch 身份");
    let local_candidate = store::read_candidate(
        &root,
        ITEM_ID,
        &retry_batch,
        store::LOCAL_CANDIDATE_FILE,
    )
    .expect("重试必须落下新的本地候选");
    assert_eq!(
        local_candidate
            .slots
            .iter()
            .find(|slot| slot.slot_id == "q14")
            .and_then(|slot| slot.answer.as_ref())
            .and_then(|answer| answer.get("labels")),
        Some(&json!(["C"]))
    );
    assert_eq!(read_answer(&root, "q14")["labels"], json!(["A"]));

    // The production local stage consumes the newly frozen batch before the repair writer runs.
    run_local_only_recognition_cycle_for_batch(
        &root,
        ITEM_ID,
        retry_base_version,
        &retry_batch,
    )
    .expect("重试本地识别周期必须完成");

    let _requests = start_repair_service(&root, scripted_retry_edit_reply);
    let mut cloud_candidate = cloud_draft("C");
    cloud_candidate["answerKey"]["cloud-q15"]["labels"] = json!(["E"]);
    finalize_cloud_authoring_candidate(
        &root,
        ITEM_ID,
        &retry_batch,
        retry_base_version,
        &json!({"authoring": cloud_candidate}),
    )
    .expect("重试云端候选必须能接入同一批次");

    let not_cancelled = || false;
    let request = request_for_batch(
        &root,
        &retry_batch,
        "run-retry-1",
        &not_cancelled,
        8,
    );
    let report = run_legacy(&request, |context, observations| {
        repair_authoring_step_through_gateway(
            &root,
            ITEM_ID,
            Some("controlled-repair"),
            context,
            observations,
        )
    })
    .expect("重试修复循环必须完成");

    assert_eq!(
        report.applied_count,
        1,
        "未受保护的新改进必须真实写入: {:?}",
        report.observations
    );
    assert_eq!(read_answer(&root, "q14")["labels"], json!(["A"]));
    assert_eq!(
        read_answer(&root, "q15")["labels"],
        json!(["E"]),
        "新识别带来的 q15 改进必须落到 canonical"
    );
    assert!(
        report
            .observations
            .iter()
            .any(|observation| observation["errors"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|error| error.as_str().unwrap_or_default().contains("EDIT_PROTECTED_TARGET"))),
        "q14 的人工编辑必须收到具体保护拒绝"
    );
    assert!(
        has_task(&report.remaining_tasks, "cloud-diff:slot:q14:answer"),
        "改不动的人工差异必须进入剩余任务而不是消失: {:?}",
        report.remaining_tasks
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// 云端能写入**结构**，不只是答案：选项库与作答结构都能按原文件修正并落库。
#[test]
fn cloud_repair_writes_option_bank_and_response_structure_through_the_real_chain() {
    use crate::auto_pipeline::finalize_cloud_authoring_candidate;
    use crate::processing::scheduler::run_local_only_recognition_cycle;

    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    seed_job_with_source(&root);
    let _requests = start_repair_service(&root, scripted_structure_fix_reply);

    let base_edit_version = canonical_version(&root);
    let _ = run_local_only_recognition_cycle(&root, ITEM_ID, base_edit_version).expect("本地周期");
    let batch_id = batch_id_for(&root, base_edit_version);
    // 候选与当前稿的**答案**没有差异：这条用例验的是结构写入，不是答案差异。
    let raw = json!({"authoring": cloud_draft("B")});
    finalize_cloud_authoring_candidate(&root, ITEM_ID, &batch_id, base_edit_version, &raw)
        .expect("候选落盘");

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    let report = run_legacy(&request, |context, observations| {
        repair_authoring_step_through_gateway(
            &root,
            ITEM_ID,
            Some("controlled-repair"),
            context,
            observations,
        )
    })
    .expect("修复循环必须跑完");

    // 第一轮漏带来源依据 → 被拒，且逐个点名缺的是谁（模型据此知道要把依据带回来）。
    let first_attempt = &report.observations[1];
    assert_eq!(first_attempt["status"], "rejected", "漏带依据必须被拒：{first_attempt:?}");
    let first_errors = first_attempt["errors"]
        .as_array()
        .map(|errors| errors.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    assert!(
        first_errors.contains("PROVENANCE_MISSING@response_group:early-approaches-shared-response"),
        "拒绝理由必须点名是哪个对象丢了依据：{first_errors}"
    );
    // 第二轮把依据带回来 → 两条命令都落库。
    assert_eq!(
        report.applied_count,
        2,
        "选项库与作答结构两条命令都要落库：{:?}",
        report.observations
    );

    let conn = open_library_connection(&root).expect("打开库连接");
    let (ds, _) = get_canonical_ds(&conn, ITEM_ID).expect("读 canonical").expect("已播");
    // 选项库：B 的措辞必须被改成原文件里的那份。
    let option_b_text = ds
        .pointer("/taskGroups/0/optionBank/options/1/content/0/text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    assert_eq!(
        option_b_text, "factor B (revised)",
        "选项库必须被真实写入，而不是只回一句「已修正」"
    );
    // 作答结构：提示语必须被改写。
    let prompt_text = ds
        .pointer("/taskGroups/0/responseGroups/0/prompt/0/children/0/text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    assert_eq!(
        prompt_text, "Which TWO factors shaped early organisational design?",
        "作答结构必须被真实写入"
    );
    // 结构写入不得把答案或槽位搞丢。
    assert_eq!(ds.pointer("/answerKey/q14/labels"), Some(&json!(["B"])));
    assert_eq!(
        ds.pointer("/taskGroups/0/responseGroups/0/slotIds"),
        Some(&json!(["q14", "q15"])),
        "改写作答结构不得丢掉槽位"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 权限边界：云端**不能**凭空新增一整组题——门禁会拒绝，且模型必须把它留成未解疑问。
///
/// 这条记录的是产品事实而不是缺陷：`upsertTaskGroupBundle` 把 `sourceAnchors` /
/// `evidenceAnchors` 强制写成空数组，模型又没有 `bindSource`（`MODEL_ALLOWED_OPS` 有意
/// 排除，见 `tools.rs` 的说明）。于是「云端补上本地漏掉的整组题」在**当前权限模型下做不到**。
///
/// 关键的不是「它做不到」，而是**做不到时产品怎么表现**：
///  - 门禁必须拒绝（绝不落一份无法发布、却看起来已完成的稿）；
///  - 拒绝理由必须具体到哪个对象缺什么，模型才有机会缩小范围或如实上报；
///  - 模型把它留成未解疑问之后，它必须出现在用户的剩余任务里。
#[test]
fn cloud_repair_cannot_invent_an_ungrounded_task_group_and_says_so() {
    use crate::auto_pipeline::finalize_cloud_authoring_candidate;
    use crate::processing::scheduler::run_local_only_recognition_cycle;

    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    seed_job_with_source(&root);
    let _requests = start_repair_service(&root, scripted_new_task_group_reply);

    let base_edit_version = canonical_version(&root);
    let _ = run_local_only_recognition_cycle(&root, ITEM_ID, base_edit_version).expect("本地周期");
    let batch_id = batch_id_for(&root, base_edit_version);
    let raw = json!({"authoring": cloud_draft("B")});
    finalize_cloud_authoring_candidate(&root, ITEM_ID, &batch_id, base_edit_version, &raw)
        .expect("候选落盘");

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    let report = run_legacy(&request, |context, observations| {
        repair_authoring_step_through_gateway(
            &root,
            ITEM_ID,
            Some("controlled-repair"),
            context,
            observations,
        )
    })
    .expect("修复循环必须跑完");

    // 两次尝试都被拒：第一次缺必填字段，第二次过 schema 但过不了质量门禁。
    assert_eq!(report.applied_count, 0, "无来源依据的新题组绝不能落库");
    let rejections: Vec<&Value> = report
        .observations
        .iter()
        .filter(|observation| observation["status"] == "rejected")
        .collect();
    assert_eq!(rejections.len(), 2, "两次尝试都应被拒：{:?}", report.observations);
    let second_errors = rejections[1]["errors"]
        .as_array()
        .map(|errors| {
            errors
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    // 拒绝理由必须点名**具体对象与具体缺口**，模型才可能据此缩小范围。
    assert!(
        second_errors.contains("PROVENANCE_MISSING")
            || second_errors.contains("INSTRUCTION_PROVENANCE_MISSING"),
        "拒绝理由必须具体到「哪个对象缺来源依据」：{second_errors}"
    );
    assert!(
        second_errors.contains("OPTION_BANK_REFERENCE_MISSING"),
        "拒绝理由必须点名选项库引用缺失：{second_errors}"
    );

    // 权威稿一字未改：仍然只有原来那一组题。
    let conn = open_library_connection(&root).expect("打开库连接");
    let (ds, _) = get_canonical_ds(&conn, ITEM_ID).expect("读 canonical").expect("已播");
    assert_eq!(
        ds.pointer("/taskGroups").and_then(Value::as_array).map(Vec::len),
        Some(1),
        "被拒的新题组不得留在稿里"
    );
    assert_eq!(ds.pointer("/answerKey/q16"), None, "新槽位也不得残留");

    // 模型如实把它留成了未解疑问 → 必须出现在用户的剩余任务里，而不是无声消失。
    let question = report
        .remaining_tasks
        .iter()
        .find(|task| {
            task["userTaskId"]
                .as_str()
                .unwrap_or("")
                .starts_with("cloud-question:")
        })
        .unwrap_or_else(|| {
            panic!(
                "做不到的事必须留给用户，实际剩余任务：{:?}",
                report.remaining_tasks
            )
        });
    assert!(
        question["message"]
            .as_str()
            .unwrap_or("")
            .contains("16-17"),
        "疑问必须带着模型的原话：{question:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ── 本轮修复的回归：终态保证、状态不误报、指纹覆盖依据、跨源去重 ────────────────
//
// 这一组每一条都对应一个**具体的用户可见后果**，并且都先写清楚「怎么复现」。
// 断言的是产品语义（批次行会不会停在 running、用户会不会被告知假的完成、
// 一条差异会不会被旧裁定永久压住、同一件事会不会被说三遍）。

/// 循环内部任何失败都必须**转成终态报告**返回，绝不能把 `running` 留在批次行里。
///
/// 复现：`run_repair_loop` 一开工就把批次行写成 `running`（见 `report_progress`），
/// 而循环体里有多处可以提前返回。返回 Err 时调用方拿不到报告，也就没人把 `running`
/// 改掉 —— 批次行永久停在 running，而同一时刻 job 行已经是 failed。前端读的是批次行
/// （`repair_json` 是读取权威），于是用户永远看到「云端正在自动修复」，两个界面互相矛盾。
#[test]
fn every_exit_path_returns_a_terminal_report_and_never_leaves_running() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");

    let seen: std::cell::RefCell<Vec<RepairProgress>> = std::cell::RefCell::new(Vec::new());
    let sink = |progress: RepairProgress| seen.borrow_mut().push(progress);
    let not_cancelled = || false;
    let mut request = request(&root, &not_cancelled, 3);
    request.progress = Some(&sink);

    // 注入一个「第一回合就返回 Err」的模型调用：以前这条路径会让循环直接返回 Err。
    let report = run_legacy(&request, |_context: &Value, _observations: &[Value]| {
        Err("llm_http_500:upstream".to_string())
    })
    .expect("循环内部失败必须转成报告，不能返回 Err");

    assert_ne!(report.status, REPAIR_STATUS_RUNNING, "报告绝不能是 running");
    assert_eq!(report.status, REPAIR_STATUS_UNAVAILABLE);
    // 开工确实写了 running —— 这正是「必须有人写终态」的原因。
    assert_eq!(seen.borrow()[0].status, REPAIR_STATUS_RUNNING);
    // 循环之外的失败（join 失败 / 开工前丢 lease）走兜底摘要，同样必须是终态。
    let fallback = unavailable_summary(&root, ITEM_ID, BATCH_ID, "join failed");
    assert_eq!(fallback["status"], REPAIR_STATUS_UNAVAILABLE);
    assert_ne!(fallback["status"], REPAIR_STATUS_RUNNING);
    let _ = std::fs::remove_dir_all(&root);
}

/// `budget_exhausted` 的判据必须是「模型**真的**调用过 finish」，不是「finish 里没写 note」。
///
/// 复现：`note` 是可选字段。旧判据把「模型正常收工但没写 note」误报成预算耗尽，
/// 用户于是看到「达到本轮上限，剩余问题需要你处理」，而云端其实已经把该做的做完了。
#[test]
fn a_model_that_finishes_without_a_note_is_not_reported_as_budget_exhausted() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    // 候选与当前稿一致：没有差异、没有阻断问题 → 正常收工就是真的完成。
    store_candidate(&root, "B");

    let not_cancelled = || false;
    // 预算恰好 1 回合，模型在第 1 回合就 finish，且**不写 note**。
    let request = request(&root, &not_cancelled, 1);
    let report = run_legacy(&request, |_context: &Value, _observations: &[Value]| {
        Ok(json!({"callId": "f1", "tool": "finish", "arguments": {}}))
    })
    .expect("修复循环必须返回结果");

    assert!(report.finish_note.is_none(), "这条用例的前提是「没写 note」");
    assert_ne!(
        report.status, REPAIR_STATUS_BUDGET_EXHAUSTED,
        "模型正常收工不得被误报成预算耗尽"
    );
    assert_eq!(report.status, REPAIR_STATUS_COMPLETED);
    let _ = std::fs::remove_dir_all(&root);
}

/// 中途一次可恢复的往返（工具名不认识 → 错误回给模型 → 模型改对）不该在**成功**的运行上
/// 留下一条 `lastError`。
///
/// 复现：旧代码把解析失败写进 `last_error` 且从不清理。于是一次「中途被拒一次、随后正常
/// 收工」的修复会带着非空 lastError 返回，界面把它当失败显示，用户以为云端出错了。
#[test]
fn a_recovered_tool_error_does_not_leave_a_last_error_on_a_successful_run() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "B");

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 4);
    let mut calls = 0u32;
    let report = run_legacy(&request, |_context: &Value, _observations: &[Value]| {
        calls += 1;
        Ok(match calls {
            // 第 1 回合交了个不认识的工具：错误回给模型（observations 里有），
            // 那是可恢复的往返，不是终态失败。
            1 => json!({"callId": "bad", "tool": "not_a_tool", "arguments": {}}),
            _ => json!({"callId": "f1", "tool": "finish", "arguments": {"note": "收工"}}),
        })
    })
    .expect("修复循环必须返回结果");

    assert_eq!(calls, 2, "第一次被拒之后必须有机会改对");
    assert!(
        report.last_error.is_none(),
        "中途一次可恢复的往返不该留下终态错误：{:?}",
        report.last_error
    );
    assert_ne!(report.status, REPAIR_STATUS_UNAVAILABLE);
    let _ = std::fs::remove_dir_all(&root);
}

/// 「云端替用户了结了多少争议」只数**此刻仍然有效**的裁定：重复的算一条，
/// 历史 / 已失效的一条都不算。
///
/// 复现：旧代码 `adjudicated_count = rulings.len()`。那份列表是 append-only 的累积记录，
/// 于是同一个差异被反复裁定会重复计数，上一批留下的旧裁定也算进去，重跑一次数字还会变大。
#[test]
fn adjudicated_count_only_counts_rulings_that_still_hold() {
    let canonical = golden_authoring();
    // 「候选整组缺失」的极简候选：必然产生 task_group 差异。
    let candidate = json!({
        "taskGroups": [], "answerSlots": {}, "answerKey": {}, "assets": []
    });
    let differences = candidate_differences(&canonical, &candidate);
    assert!(!differences.is_empty(), "夹具必须先真的产生差异");
    let first = differences[0].clone();
    let target_type = first["targetType"].as_str().unwrap_or_default().to_string();
    let target_id = first["targetId"].as_str().unwrap_or_default().to_string();
    let field = first["field"].as_str().unwrap_or_default().to_string();
    let (canonical_digest, candidate_digest, context_digest) = difference_digests(&first);
    let ruling = |canonical_digest: &str, candidate_digest: &str, context_digest: &str| {
        json!({
            "targetType": target_type,
            "targetId": target_id,
            "field": field,
            "ruling": crate::schema::cloud_repair_v1::CLOUD_RULING_CURRENT_IS_CORRECT,
            "canonicalDigest": canonical_digest,
            "candidateDigest": candidate_digest,
            "contextDigest": context_digest,
        })
    };

    // 没有任何裁定 → 0。
    assert_eq!(effective_adjudicated_count(&canonical, &candidate, &[]), 0);
    // 同一条差异被裁定两次（模型反复裁定）→ 只算一条。
    let duplicated = vec![
        ruling(&canonical_digest, &candidate_digest, &context_digest),
        ruling(&canonical_digest, &candidate_digest, &context_digest),
    ];
    assert_eq!(
        effective_adjudicated_count(&canonical, &candidate, &duplicated),
        1,
        "同一条差异重复裁定只能算一条"
    );
    // 指纹对不上的历史 / 已失效裁定 → 一条都不算。
    let stale = vec![ruling("stale", "stale", "stale")];
    assert_eq!(
        effective_adjudicated_count(&canonical, &candidate, &stale),
        0,
        "已失效的裁定不能计入「了结了多少争议」"
    );

    // 「查过但定不了论」（`cannot_resolve`，理由不是 `CONTEXT_INSUFFICIENT`）仍然算一条：
    // 云端确实看过材料并给出了结论，这条争议有了着落。
    let mut cannot_resolve = ruling(&canonical_digest, &candidate_digest, &context_digest);
    cannot_resolve["ruling"] = json!(crate::schema::cloud_repair_v1::CLOUD_RULING_CANNOT_RESOLVE);
    cannot_resolve["reason"] = json!("原文件这一段本身自相矛盾，无法定论");
    assert_eq!(
        effective_adjudicated_count(&canonical, &candidate, &[cannot_resolve]),
        1,
        "「查过但定不了论」是云端给出的结论，要算进了结"
    );

    // 「上下文不足」不算：云端根本没拿到材料，这条差异**没有**被核对过。
    // 三态不坍缩（not_executed / insufficient_context / passed）靠的就是这一条。
    let mut insufficient = ruling(&canonical_digest, &candidate_digest, &context_digest);
    insufficient["ruling"] = json!(crate::schema::cloud_repair_v1::CLOUD_RULING_CANNOT_RESOLVE);
    insufficient["reason"] =
        json!(crate::schema::cloud_repair_v1::CLOUD_RULING_REASON_CONTEXT_INSUFFICIENT);
    assert_eq!(
        effective_adjudicated_count(&canonical, &candidate, &[insufficient]),
        0,
        "上下文不足不是裁定：算成已了结就是假完成"
    );
}

/// 裁定必须绑定**它依赖的内容**，而不只是差异两侧的字面值。
///
/// 复现（旧裁定压制变质差异）：模型对一条 `task_group` 差异裁定「当前稿对」。这条差异的
/// 两侧指纹取自 `group_index_entry` —— 一个只含 taskId / 题型 / 题号 / slotIds / 说明 /
/// 选项库 id 的**摘要**。随后该组的 stimulus / 题面 / 选项文本 / 答案被改写，摘要一个字
/// 没变、差异两侧也没变，旧代码于是继续 `continue`，这条已经变质的内容永远不进用户清单。
#[test]
fn a_ruling_stops_holding_once_the_content_it_depended_on_changes() {
    let canonical = golden_authoring();
    let task_id = canonical["taskGroups"][0]["taskId"]
        .as_str()
        .expect("golden 必须有一个题组")
        .to_string();
    // 候选完全没有这个题组 → 一条 (task_group, taskId, task_group) 差异。
    let candidate = json!({
        "taskGroups": [], "answerSlots": {}, "answerKey": {}, "assets": []
    });
    let before_diff = candidate_differences(&canonical, &candidate)
        .into_iter()
        .find(|difference| difference["field"] == "task_group")
        .expect("必须有一条整组差异");
    let (canonical_digest, candidate_digest, context_digest) = difference_digests(&before_diff);
    let rulings = vec![json!({
        "targetType": "task_group",
        "targetId": task_id,
        "field": "task_group",
        "ruling": crate::schema::cloud_repair_v1::CLOUD_RULING_CURRENT_IS_CORRECT,
        "canonicalDigest": canonical_digest,
        "candidateDigest": candidate_digest,
        "contextDigest": context_digest,
    })];
    assert!(
        fresh_ruling_for_difference(&rulings, &before_diff).is_some(),
        "对照组：内容没变时裁定必须有效"
    );

    // 改写该组内容：加一段 stimulus。差异两侧一个字都没变（差异还是「候选缺这个组」）。
    let mut changed = canonical.clone();
    changed["taskGroups"][0]["stimulus"] = json!([{
        "type": "paragraph",
        "id": "rewritten-stimulus",
        "sourceAnchors": [],
        "provenanceStatus": "source",
        "children": [{
            "type": "text",
            "id": "rewritten-stimulus-text",
            "sourceAnchors": [],
            "provenanceStatus": "source",
            "text": "REWRITTEN BY THE USER"
        }]
    }]);
    let after_diff = candidate_differences(&changed, &candidate)
        .into_iter()
        .find(|difference| difference["field"] == "task_group")
        .expect("整组差异必须还在");

    // 旧判据（`group_index_entry` 摘要）逐字未变 —— 这正是旧代码会继续压住差异的原因。
    assert_eq!(
        group_index_entry(&canonical["taskGroups"][0]),
        group_index_entry(&changed["taskGroups"][0]),
        "摘要里本来就不含 stimulus，所以它不可能发现这次改动"
    );
    // 但裁定依赖的内容变了 → 必须失效重评。
    assert!(
        fresh_ruling_for_difference(&rulings, &after_diff).is_none(),
        "依据被改写之后，旧裁定不得继续压制这条差异"
    );
}

/// 裁定只绑差异两侧、不绑依据的另一种形状：答案的**依据**是选项库。
///
/// 复现：模型基于「选项库把 B 映射到某个词」裁定答案 B 对。随后选项库被改，答案值一个字
/// 没变，差异两侧也就没变 —— 旧代码继续认这条裁定有效，而它的依据已经没了。
#[test]
fn a_ruling_grounded_in_the_option_bank_dies_when_the_option_bank_changes() {
    let canonical = golden_authoring();
    // 候选把 q14 的答案改成 A → 一条 (slot, q14, answer) 差异。
    let mut candidate = canonical.clone();
    candidate["answerKey"]["q14"] =
        json!({"kind": "option", "labels": ["A"], "assignment": "unordered_set"});

    let before_diff = candidate_differences(&canonical, &candidate)
        .into_iter()
        .find(|difference| difference["targetType"] == "slot" && difference["field"] == "answer")
        .expect("必须有一条答案差异");
    let (canonical_digest, candidate_digest, context_digest) = difference_digests(&before_diff);
    let rulings = vec![json!({
        "targetType": "slot",
        "targetId": "q14",
        "field": "answer",
        "ruling": crate::schema::cloud_repair_v1::CLOUD_RULING_CURRENT_IS_CORRECT,
        "canonicalDigest": canonical_digest,
        "candidateDigest": candidate_digest,
        "contextDigest": context_digest,
    })];
    assert!(
        fresh_ruling_for_difference(&rulings, &before_diff).is_some(),
        "对照组：裁定此刻有效"
    );

    // 只改选项库文本；答案与差异两侧都不动。
    let mut changed = canonical.clone();
    changed["taskGroups"][0]["optionBank"]["options"][0]["content"][0]["text"] =
        json!("a completely different factor");
    let after_diff = candidate_differences(&changed, &candidate)
        .into_iter()
        .find(|difference| difference["targetType"] == "slot" && difference["field"] == "answer")
        .expect("答案差异必须还在");

    assert_eq!(
        after_diff["canonical"], before_diff["canonical"],
        "答案本身没变（这正是旧代码认为裁定仍有效的原因）"
    );
    assert!(
        fresh_ruling_for_difference(&rulings, &after_diff).is_none(),
        "选项库（裁定的依据）变了，基于它的裁定必须失效"
    );
}

/// 阻断任务必须带一条**真能做完**的动作，不允许出现「阻断但零动作」的任务。
///
/// 复现：`targetId` 缺失时旧代码落成空串，前端把它过滤掉 → 一条 blocking 却没有任何
/// 按钮的任务：用户看到「不处理不能导出」，却无处可去。
#[test]
fn a_blocking_task_always_carries_a_real_action() {
    let root = temp_root();
    let mut canonical = golden_authoring();
    // 一条**文档级**阻断问题：没有 targetId（真实形状：这类问题常常只带锚点），
    // 题面上没有可改的地方。
    canonical["quality"]["issues"] = json!([{
        "issueId": "doc-level-blocker",
        "code": "SLOT_ID_MISMATCH",
        "severity": "blocking",
        "message": "题面里的空位编号与答案槽对不上",
        "targetType": "document",
        "targetId": null
    }]);
    seed_item(&root, &canonical);
    store_candidate(&root, "B");

    let tasks = remaining_tasks(&root, ITEM_ID, ITEM_ID, BATCH_ID, &[], &[]).expect("重算剩余任务");
    let blocking: Vec<&Value> = tasks
        .iter()
        .filter(|task| task["blocking"] == json!(true))
        .collect();
    assert!(
        !blocking.is_empty(),
        "夹具必须先真的产生一条阻断任务：{tasks:?}"
    );
    for task in blocking {
        let target_ids = task["targetIds"].as_array().cloned().unwrap_or_default();
        let action = task["action"].as_str().unwrap_or("");
        assert!(
            !target_ids.is_empty() || action == "review_source",
            "阻断任务必须能定位到目标、或有一条真能推进它的兜底动作：{task:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// 同一目标、同一动作的四源并集必须只留一条，且保留最严重的级别。
///
/// 复现：去重键以前各带来源前缀（`quality:` / `cloud-diff:` / …），跨源去重从不发生。
/// 「q14 缺答案」会同时产出 `quality:ANSWER_KEY_MISSING_SLOT:q14`（blocking）与
/// `cloud-diff:slot:q14:answer`（非 blocking），用户看到两件其实是同一件的事。
#[test]
fn one_missing_answer_is_reported_once_at_the_most_severe_level() {
    let root = temp_root();
    let mut canonical = golden_authoring();
    canonical["quality"]["issues"] = json!([{
        "issueId": "missing-q14-answer",
        "code": "ANSWER_KEY_MISSING_SLOT",
        "severity": "blocking",
        "message": "第 14 题没有答案",
        "targetType": "slot",
        "targetId": "q14"
    }]);
    seed_item(&root, &canonical);
    // 候选把 q14 改成 A → 另有一条 `cloud-diff:slot:q14:answer`（非阻断）。
    store_candidate(&root, "A");

    let tasks = remaining_tasks(&root, ITEM_ID, ITEM_ID, BATCH_ID, &[], &[]).expect("重算剩余任务");
    let for_q14: Vec<&Value> = tasks
        .iter()
        .filter(|task| {
            task["targetIds"]
                .as_array()
                .map(|ids| ids.iter().any(|id| id == "q14"))
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(for_q14.len(), 1, "同一目标同一动作只能留一条：{tasks:?}");
    assert_eq!(for_q14[0]["blocking"], json!(true), "必须保留最严重的那条");
    assert_eq!(
        for_q14[0]["userTaskId"], "quality:ANSWER_KEY_MISSING_SLOT:q14",
        "保留的应当是阻断的那条"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 不同的修复动作**不许**合并：同一个目标上的「缺答案」与「结构不完整」是两件事，
/// 用户照做其中一件修不好另一件。
#[test]
fn different_repairs_on_the_same_target_are_not_merged() {
    let root = temp_root();
    let mut canonical = golden_authoring();
    canonical["quality"]["issues"] = json!([
        {"issueId": "missing-q14-answer", "code": "ANSWER_KEY_MISSING_SLOT", "severity": "blocking",
         "message": "第 14 题没有答案", "targetType": "slot", "targetId": "q14"},
        {"issueId": "broken-q14-structure", "code": "SLOT_STRUCTURE_INCOMPLETE", "severity": "blocking",
         "message": "第 14 题所在的作答区结构不完整", "targetType": "slot", "targetId": "q14"}
    ]);
    seed_item(&root, &canonical);
    // 候选与当前稿一致：只留两条质量问题，不掺差异。
    store_candidate(&root, "B");

    let tasks = remaining_tasks(&root, ITEM_ID, ITEM_ID, BATCH_ID, &[], &[]).expect("重算剩余任务");
    let for_q14 = tasks
        .iter()
        .filter(|task| {
            task["targetIds"]
                .as_array()
                .map(|ids| ids.iter().any(|id| id == "q14"))
                .unwrap_or(false)
        })
        .count();
    assert_eq!(for_q14, 2, "两种不同的修理必须各留一条：{tasks:?}");
    let _ = std::fs::remove_dir_all(&root);
}

/// 两条**不同**的文档级问题不能因为「都没有 targetId」就折叠成一条。
///
/// 复现：`quality:{code}:{targetId}` 在 targetId 为空时把两条不同的文档级问题折叠成一条，
/// 第二条的 message 被静默丢掉，用户以为只有一条。
#[test]
fn two_different_document_level_issues_are_not_collapsed_into_one() {
    let root = temp_root();
    let mut canonical = golden_authoring();
    canonical["quality"]["issues"] = json!([
        {"issueId": "doc-issue-a", "code": "SIGNIFICANT_REGION_UNASSIGNED", "severity": "blocking",
         "message": "原文件第 2 页有一段没被任何题组接住", "targetId": null},
        {"issueId": "doc-issue-b", "code": "SIGNIFICANT_REGION_UNASSIGNED", "severity": "blocking",
         "message": "原文件第 3 页有一段没被任何题组接住", "targetId": null}
    ]);
    seed_item(&root, &canonical);
    store_candidate(&root, "B");

    let tasks = remaining_tasks(&root, ITEM_ID, ITEM_ID, BATCH_ID, &[], &[]).expect("重算剩余任务");
    assert_eq!(tasks.len(), 2, "两条不同的文档级问题必须各留一条：{tasks:?}");
    let messages: Vec<String> = tasks
        .iter()
        .filter_map(|task| task["message"].as_str().map(str::to_string))
        .collect();
    assert!(
        messages.iter().any(|message| message.contains("第 2 页")),
        "第一条的 message 不能被丢掉：{messages:?}"
    );
    assert!(
        messages.iter().any(|message| message.contains("第 3 页")),
        "第二条的 message 不能被丢掉：{messages:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ── read_source 必须真的给出原文文本（否则「照原文件改」不可证伪）────────────────
//
// 这一组守的是「云端凭什么说自己改对了」：如果 `read_source` 只回页图，模型就无法
// 逐字引用原文，它给出的引文也就无从核对——「照着原文件改的」退化成一句自述。

/// `read_source` 要给 PDF 页附上**原文件抽取出来的文本**，且不碰页上其它字段。
///
/// 复现：以前 PDF 分支只回 `{pageIndex, images, width, height}`，note 却写着
/// 「下面的页文本就是抽取出来的文本层」——承诺与内容不符。模型只能凭图片印象转述，
/// 引文随便编也无人能证伪。
#[test]
fn read_source_pages_carry_the_text_layer_extracted_from_the_original_file() {
    let root = temp_root();
    ensure_app_dirs(&root).expect("ensure_app_dirs");
    let dir = crate::util::job_dir(&root, ITEM_ID);
    std::fs::create_dir_all(&dir).expect("job dir");
    crate::util::write_json(
        &dir.join("document-ir.json"),
        &json!({"pages":[
            {"pageIndex":0,"lines":[
                {"text":"Early approaches to organisational design."},
                {"text":"Q1 Choose the correct letter."}
            ]},
            {"pageIndex":1,"lines":[{"text":"BLANK PAGE"}]}
        ]}),
    )
    .expect("document-ir");

    let texts = source_page_texts(&root, ITEM_ID);
    // DocumentIR 的 pageIndex 是 0-based，返回的 key 统一成 1-based（见函数文档）。
    assert_eq!(
        texts.get(&1).map(String::as_str),
        Some("Early approaches to organisational design.\nQ1 Choose the correct letter."),
        "逐页文本必须按页号归位"
    );
    assert_eq!(texts.get(&2).map(String::as_str), Some("BLANK PAGE"));

    // `read_source` 返回的页对象来自 `pdf-images.json`，页码是 1-based。
    //
    // 这里**走真实分支函数**（`pdf_read_source_response`），而不是直接调
    // `attach_page_texts`。旧写法只证明助手能拼文本，证明不了 PDF 分支**真的把文本层
    // 接上了**：把那一行调用删掉（退回「PDF 只回页图」），旧写法照样全绿——变异检查
    // 就是这么抓出来的。走分支函数后，同一处删除会立刻让下面的断言变红。
    let evidence = json!({
        "kind": "pdf",
        "sourceFileId": "early-approaches-pdf",
        "pages": [
            {"pageIndex":1,"images":[{"fileName":"page-001-rendered.png"}],"width":2000},
            {"pageIndex":2,"images":[{"fileName":"page-002-rendered.png"}],"width":2000},
            {"pageIndex":3,"images":[],"width":2000}
        ]
    });
    let response = pdf_read_source_response(&evidence, None, None, &texts);
    assert_eq!(response["kind"], "pdf");
    assert_eq!(response["sourceFileId"], "early-approaches-pdf");
    assert_eq!(
        response["pagesWithText"], 2,
        "两页抽到了文本层，第三页没有——计数必须如实"
    );
    let attached = response["pages"].as_array().expect("pages");
    assert_eq!(
        attached[0]["text"],
        "Early approaches to organisational design.\nQ1 Choose the correct letter."
    );
    assert_eq!(attached[1]["text"], "BLANK PAGE");
    // 抽不出文本的页**不编造**：干脆不出现 text 字段，而不是补一个空串
    // （空串会让模型以为「这一页真的没有字」）。
    assert!(
        attached[2].get("text").is_none(),
        "抽不出文本的页不能凭空补一个空文本"
    );
    // 原有字段必须保留：页图是模型打开原文件的入口。
    assert_eq!(attached[0]["images"][0]["fileName"], "page-001-rendered.png");
    assert_eq!(attached[0]["width"], 2000);
    // note 必须跟着事实走：抽到文本才敢说「下面就是文本层」。
    let note = response["note"].as_str().unwrap_or_default();
    assert!(
        note.contains("text layer extracted"),
        "抽到文本层时 note 必须说明可以逐字引用，实际：{note}"
    );

    // 反向对照：一页都抽不出文本时，note 必须**改口**，不能继续承诺有文本层——
    // 否则模型会以为自己读到了原文，从而编造引文。
    let no_text = pdf_read_source_response(&evidence, None, None, &BTreeMap::new());
    assert_eq!(no_text["pagesWithText"], 0);
    let note = no_text["note"].as_str().unwrap_or_default();
    assert!(
        note.contains("No text layer could be extracted"),
        "抽不出文本时 note 必须如实说明，实际：{note}"
    );

    // 页范围过滤仍然生效：模型可以只要某一页。
    let only_second = pdf_read_source_response(&evidence, Some(2), Some(2), &texts);
    let filtered = only_second["pages"].as_array().expect("pages");
    assert_eq!(filtered.len(), 1, "pageIndex=2 只应返回第 2 页");
    assert_eq!(filtered[0]["pageIndex"], 2);
    assert_eq!(only_second["pagesWithText"], 1);
    let _ = std::fs::remove_dir_all(&root);
}

/// 没有规范抽取产物时退回 `DocumentIRV2` 比对报告里的 V1 文本；两者都没有就如实为空。
#[test]
fn source_page_texts_falls_back_to_the_v1_extraction_and_never_invents_text() {
    let root = temp_root();
    ensure_app_dirs(&root).expect("ensure_app_dirs");
    let dir = crate::util::job_dir(&root, ITEM_ID);
    std::fs::create_dir_all(&dir).expect("job dir");

    // 对照组：既没有 `document-ir.json`、也没有比对报告 → 空，而不是编造一段文本。
    assert!(
        source_page_texts(&root, ITEM_ID).is_empty(),
        "没有抽取产物时必须如实为空"
    );

    crate::util::write_json(
        &dir.join("document-ir-v2.shadow.compare.json"),
        &json!({"schemaVersion":"DocumentIRV2CompareReportV1","pages":[
            {"pageIndex":3,"v1Text":"40 The writer recommends that to be effective, social history must"},
            {"pageIndex":4,"v1Text":"BLANK PAGE"}
        ]}),
    )
    .expect("compare report");

    let texts = source_page_texts(&root, ITEM_ID);
    // 比对报告的 pageIndex 也是 0-based（3、4）→ 统一成 1-based（4、5）。
    assert_eq!(
        texts.get(&4).map(String::as_str),
        Some("40 The writer recommends that to be effective, social history must")
    );
    assert_eq!(texts.get(&5).map(String::as_str), Some("BLANK PAGE"));
    let _ = std::fs::remove_dir_all(&root);
}

// ── S4：修复回合被校验器拒绝时的受约束重试 ─────────────────────────────

/// 复现：网关对修复回合的**校验拒绝**（未知工具、坏 JSON、截断）以前直接让整个循环
/// 以 `unavailable` 结束——`repairNote` 分支在生产里是死的。现在同一回合内给模型**一次**
/// 带原因的重试。
#[test]
fn a_rejected_repair_reply_gets_one_constrained_retry_carrying_the_reason() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "B");

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 4);
    let mut calls = 0u32;
    let mut note_seen_on_retry: Option<String> = None;
    let report = run_legacy(&request, |_context: &Value, observations: &[Value]| {
        calls += 1;
        if calls == 1 {
            return Err("cloud_repair_step_tool_unknown:edit_everything".to_string());
        }
        note_seen_on_retry = observations
            .last()
            .and_then(|observation| observation.get("repairNote"))
            .and_then(Value::as_str)
            .map(str::to_string);
        Ok(json!({"callId": "f1", "tool": "finish", "arguments": {"note": "done"}}))
    })
    .expect("修复循环必须返回结果");

    assert_eq!(calls, 2, "被拒之后必须在同一回合重试一次");
    assert_eq!(report.status, REPAIR_STATUS_COMPLETED, "{report:?}");
    assert_eq!(report.rounds, 1, "重试不另算一个回合：{report:?}");
    assert!(report.last_error.is_none(), "{report:?}");
    assert!(
        note_seen_on_retry
            .as_deref()
            .unwrap_or("")
            .contains("cloud_repair_step_tool_unknown"),
        "重试必须带上被拒的具体原因：{note_seen_on_retry:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 只重试**一次**，且只对校验拒绝重试：连续两次被拒如实 unavailable（两个原因都留下），
/// 传输错误不重试。
#[test]
fn a_second_rejection_or_a_transport_error_still_ends_the_loop_unavailable() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "B");
    let not_cancelled = || false;

    let request_twice = request(&root, &not_cancelled, 4);
    let mut calls = 0u32;
    let report = run_legacy(&request_twice, |_context: &Value, _observations: &[Value]| {
        calls += 1;
        Err(format!("llm_json_parse_failed:attempt-{calls}"))
    })
    .expect("修复循环必须返回结果");
    assert_eq!(calls, 2, "每回合最多一次受约束重试");
    assert_eq!(report.status, REPAIR_STATUS_UNAVAILABLE);
    let last_error = report.last_error.clone().unwrap_or_default();
    assert!(
        last_error.contains("attempt-2") && last_error.contains("attempt-1"),
        "两次被拒的原因都要留下：{last_error}"
    );

    let request_transport = request(&root, &not_cancelled, 4);
    let mut transport_calls = 0u32;
    let report = run_legacy(&request_transport, |_context: &Value, _observations: &[Value]| {
        transport_calls += 1;
        Err("llm_timeout_budget_exhausted:llm_http_timeout:stalled".to_string())
    })
    .expect("修复循环必须返回结果");
    assert_eq!(transport_calls, 1, "超时重问同一句话没有意义，不得重试");
    assert_eq!(report.status, REPAIR_STATUS_UNAVAILABLE);
    let _ = std::fs::remove_dir_all(&root);
}

/// 剧本：第一轮回一段解析不了的文字，第二轮（重试）正常收工。
fn scripted_garbage_then_finish_reply(_body: &str, round: usize) -> String {
    if round == 1 {
        "I think the draft looks fine overall.".to_string()
    } else {
        json!({"callId": "f1", "tool": "finish", "arguments": {"note": "retry succeeded"}})
            .to_string()
    }
}

/// 同一件事走**真实网关**：受控服务第一次回坏 JSON，网关拒绝；循环带着原因重问，
/// 第二个 HTTP 请求的 prompt 里必须真的写着被拒原因，循环以 completed 结束（以前是 unavailable）。
#[test]
fn a_rejected_reply_is_retried_through_the_real_gateway_with_the_rejection_in_the_prompt() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "B");
    seed_job_with_source(&root);
    let requests = start_repair_service(&root, scripted_garbage_then_finish_reply);

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 4);
    let report = run_legacy(&request, |context: &Value, observations: &[Value]| {
        repair_authoring_step_through_gateway(
            &root,
            ITEM_ID,
            Some("controlled-repair"),
            context,
            observations,
        )
    })
    .expect("修复循环必须返回结果");

    assert_eq!(report.status, REPAIR_STATUS_COMPLETED, "{report:?}");
    let seen = requests.lock().expect("requests");
    assert_eq!(seen.len(), 2, "一次被拒 + 一次受约束重试");
    assert!(
        seen[1].contains("REJECTED") && seen[1].contains("llm_json_parse_failed"),
        "重试请求必须把被拒原因写进 prompt"
    );
    assert!(!seen[0].contains("REJECTED"), "首轮请求不该带被拒说明");
    drop(seen);
    let _ = std::fs::remove_dir_all(&root);
}

// ── 读路径：陈旧判定与剩余任务重算（产品读命令 `get_recognition_decision_core`）──────

/// 建一行真实批次（与本地周期写入同一张表），基线为 `base_edit_version`。
fn seed_batch_row(root: &Path, base_edit_version: i64) {
    use crate::schema::recognition_v1::{
        ChainStatusSummaryV1, ChainStatusV1, DecisionSummaryV1, RecognitionDecisionV1,
        RECOGNITION_DECISION_V1_SCHEMA_VERSION,
    };
    let decision = RecognitionDecisionV1 {
        schema_version: RECOGNITION_DECISION_V1_SCHEMA_VERSION.to_string(),
        batch_id: BATCH_ID.to_string(),
        item_id: ITEM_ID.to_string(),
        job_id: ITEM_ID.to_string(),
        base_edit_version,
        generated_at: "2026-09-20T00:00:00Z".to_string(),
        chain_status: ChainStatusSummaryV1 {
            local: ChainStatusV1::Succeeded,
            cloud: ChainStatusV1::Succeeded,
            source: ChainStatusV1::Succeeded,
            cloud_reason_code: None,
            source_reason_code: None,
        },
        items: vec![],
        summary: DecisionSummaryV1 { agreed: 0, auto_fixed: 0, needs_review: 0, unverifiable: 0 },
    };
    let conn = open_library_connection(root).expect("打开库连接");
    store::upsert_batch(&conn, &decision).expect("写批次行");
}

/// 走真实编辑事务写一笔（`origin` 决定是人还是机器）。
fn commit_edit(root: &Path, origin: crate::library::repository::EditOrigin, run_id: Option<&str>, command: Value) {
    use crate::authoring_v2_commands::{apply_patch, refresh_quality_report, validate_authoring};
    use crate::library::repository::{apply_editor_commands_tx_with, ApplyEditorCommandsInput};
    let base_version = canonical_version(root);
    let mut conn = open_library_connection(root).expect("打开库连接");
    apply_editor_commands_tx_with(
        &mut conn,
        &ApplyEditorCommandsInput {
            item_id: ITEM_ID.to_string(),
            base_version,
            request_id: Some(format!("edit-{}", uuid::Uuid::new_v4().simple())),
            commands: vec![command],
            title: None,
        },
        origin,
        run_id,
        &apply_patch,
        &|ds| {
            refresh_quality_report(root, ITEM_ID, ds)?;
            validate_authoring(ds)
        },
        &|_, _| Ok(()),
    )
    .expect("编辑必须经正式事务落库");
}

/// 「这批建议是针对你修改之前的内容做的」只能在**人**改过之后出现。
///
/// 复现：`stale = base_edit_version < current`，而云端修复、答案页识别每写一笔都推进
/// 版本，于是用户一下都没改，面板就警告「你修改之前」。
#[test]
fn stale_only_turns_on_after_a_human_edit_never_after_machine_writes() {
    use crate::library::repository::EditOrigin;
    let root = temp_root();
    seed_item(&root, &golden_authoring());
    seed_batch_row(&root, canonical_version(&root));

    // 机器写入（云端修复）推进了版本。
    commit_edit(&root, EditOrigin::CloudRepair, Some("run-machine"), set_answer("q15", "E"));
    let view = crate::reconcile::commands::get_recognition_decision_core(&root, ITEM_ID).expect("读视图");
    assert!(view["editVersion"].as_i64().unwrap() > view["baseEditVersion"].as_i64().unwrap());
    assert_eq!(view["stale"], json!(false), "只有机器写入时不能说「你修改之前」：{view}");

    // 人工编辑之后才算过期。
    commit_edit(&root, EditOrigin::Human, None, set_answer("q14", "A"));
    let view = crate::reconcile::commands::get_recognition_decision_core(&root, ITEM_ID).expect("读视图");
    assert_eq!(view["stale"], json!(true), "人改过之后必须标记过期：{view}");
    let _ = std::fs::remove_dir_all(&root);
}

/// 剩余任务在**读取时**按当前稿重算：用户补上答案之后，那条任务不用刷新就消失。
///
/// 复现：剩余任务是修复循环结束时写进 `repair_json` 的一次性快照，用户修好了问题，
/// 任务还挂着，直到下一次重新识别。
#[test]
fn remaining_tasks_are_recomputed_on_read_so_a_fixed_problem_disappears() {
    use crate::authoring_v2_commands::refresh_quality_report;
    use crate::library::repository::EditOrigin;
    let root = temp_root();
    ensure_app_dirs(&root).expect("ensure_app_dirs");
    let mut canonical = golden_authoring();
    // q15 没有答案：质量报告按真实规则重算出这条阻断。
    canonical["answerKey"]["q15"] = json!({"kind": "unresolved"});
    refresh_quality_report(&root, ITEM_ID, &mut canonical).expect("质量报告");
    seed_item(&root, &canonical);
    seed_batch_row(&root, canonical_version(&root));

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 2);
    let report = run_legacy(&request, |_context: &Value, _observations: &[Value]| {
        Ok(json!({"callId": "f1", "tool": "finish", "arguments": {"note": "q15 原文没有答案"}}))
    })
    .expect("修复循环必须返回结果");
    let targets_q15 = |tasks: &Value| {
        tasks
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .any(|task| task["targetIds"].as_array().map(|ids| ids.iter().any(|id| id == "q15")).unwrap_or(false))
    };
    assert!(
        targets_q15(&json!(report.remaining_tasks)),
        "前提：修复结束时 q15 缺答案必须是一条剩余任务：{:?}",
        report.remaining_tasks
    );
    {
        let conn = open_library_connection(&root).expect("打开库连接");
        store::write_batch_repair(&conn, BATCH_ID, &report.to_json(false)).expect("写修复摘要");
    }
    let view = crate::reconcile::commands::get_recognition_decision_core(&root, ITEM_ID).expect("读视图");
    assert!(targets_q15(&view["repair"]["remainingTasks"]), "读路径也必须看到这条任务：{view}");

    // 用户在题面上补上 q15。
    commit_edit(&root, EditOrigin::Human, None, set_answer("q15", "D"));
    let view = crate::reconcile::commands::get_recognition_decision_core(&root, ITEM_ID).expect("读视图");
    assert!(
        !targets_q15(&view["repair"]["remainingTasks"]),
        "q15 已经填上，这条任务必须在读取时消失：{}",
        view["repair"]
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 工作区读取要带上最近的编辑来源：前端保存撞上冲突时，据此判断「只是被云端修复
/// 自己的写入挤掉了」，自动重放，而不是逼用户在「重试保存 / 放弃本地修改」之间选。
#[test]
fn workspace_item_reports_recent_edit_origins_for_conflict_rebase() {
    use crate::library::repository::EditOrigin;
    let root = temp_root();
    seed_item(&root, &golden_authoring());
    let base = canonical_version(&root);
    commit_edit(&root, EditOrigin::CloudRepair, Some("run-machine"), set_answer("q15", "E"));
    commit_edit(&root, EditOrigin::Human, None, set_answer("q14", "A"));

    let workspace = crate::library::commands::get_workspace_item_core(&root, ITEM_ID).expect("读工作区");
    let edits = workspace["recentEdits"].as_array().cloned().unwrap_or_default();
    let origin_at = |version: i64| {
        edits
            .iter()
            .find(|edit| edit["baseVersion"].as_i64() == Some(version))
            .and_then(|edit| edit["origin"].as_str().map(str::to_string))
    };
    assert_eq!(origin_at(base).as_deref(), Some("cloud_repair"), "{workspace}");
    assert_eq!(origin_at(base + 1).as_deref(), Some("human"), "{workspace}");
    let _ = std::fs::remove_dir_all(&root);
}

/// 差异任务必须带上「现在是什么、云端读到的是什么」，否则用户只看到「不一致」却无从判断。
#[test]
fn a_difference_task_carries_the_current_and_the_cloud_value() {
    let root = temp_root();
    seed_item(&root, &golden_authoring());
    store_candidate(&root, "A");
    let tasks = remaining_tasks(&root, ITEM_ID, ITEM_ID, BATCH_ID, &[], &[]).expect("重算剩余任务");
    let diff = tasks
        .iter()
        .find(|task| task["userTaskId"] == "cloud-diff:slot:q14:answer")
        .unwrap_or_else(|| panic!("夹具必须产生 q14 的答案差异：{tasks:?}"));
    assert!(!diff["currentValue"].is_null(), "{diff}");
    assert!(!diff["cloudValue"].is_null(), "{diff}");
    assert_ne!(diff["currentValue"], diff["cloudValue"]);
    assert_eq!(diff["field"], "answer");
    let _ = std::fs::remove_dir_all(&root);
}

// ── 听力 Part 边界 ────────────────────────────────────────────────────────────
//
// 音频是**每段一条**的（Section 1..4 各一个文件）。模型只被问结构（标签、题号、
// 题组归属），但它可以改分界：把两段并成一段、或把一段拆开。那种改动会换掉
// 考生听到的音频切分，属于**必须让用户看见**的差异——悄悄应用等于替用户重排了
// 一份已经绑定音频的卷子。

fn listening_numbers(range: std::ops::RangeInclusive<u32>) -> Vec<u32> {
    range.collect()
}

/// 把一份听力稿件的 Part 结构换成模型给的那份（其余字段原样），返回原始候选 JSON。
///
/// 走的是**真实**的 `normalize_cloud_authoring`：模型输出里的 `listeningParts` 是外层
/// 键，与 `authoring` 并列——正是网关真实转发的形状。
fn listening_cloud_raw(canonical: &Value, model_parts: Value) -> Value {
    let mut draft = canonical.clone();
    draft["listening"]["parts"] = json!([]);
    json!({"authoring": draft, "listeningParts": model_parts})
}

fn listening_identity<'a>(canonical: &'a Value) -> CloudAuthoringIdentity<'a> {
    CloudAuthoringIdentity {
        job_id: ITEM_ID,
        item_id: ITEM_ID,
        batch_id: BATCH_ID,
        source_file_id: "listening-audio-source",
        source_sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        base_edit_version: 1,
        generated_at: "2026-09-22T00:00:00Z",
        exam: canonical.get("exam").cloned().unwrap_or(Value::Null),
        modality: "listening",
        source_document_id: "listening-document",
        extraction_mode: "pdf_native",
    }
}

/// 模型把 Section 3 与 Section 4 并成一段：用户的任务清单里必须出现分段范围差异，
/// 且**既有音频不会被抹掉**（part-1 / part-2 原样保留、并出来的新段不带音频）。
#[test]
fn a_listening_part_boundary_change_reaches_the_users_task_list() {
    let root = temp_root();
    ensure_app_dirs(&root).expect("ensure_app_dirs");
    let canonical_value =
        serde_json::to_value(crate::test_support::complete_listening_exam()).expect("听力夹具");
    {
        let conn = open_library_connection(&root).expect("打开库连接");
        upsert_item_shell(
            &conn,
            &UpsertItemInput {
                id: ITEM_ID,
                modality: "listening",
                title: "Listening Paper",
                status: "action_required",
                source_asset_id: None,
            },
        )
        .expect("upsert_item_shell");
        seed_canonical_ds(
            &conn,
            ITEM_ID,
            &serde_json::to_string(&canonical_value).expect("序列化稿件"),
            "action_required",
        )
        .expect("seed_canonical_ds");
    }

    let model_parts = json!([
        {"displayLabel": "SECTION 1", "expectedQuestionNumbers": listening_numbers(1..=10), "taskIds": ["task-1"]},
        {"displayLabel": "SECTION 2", "expectedQuestionNumbers": listening_numbers(11..=20), "taskIds": ["task-2"]},
        {"displayLabel": "SECTION 3", "expectedQuestionNumbers": listening_numbers(21..=40), "taskIds": ["task-3", "task-4"]},
    ]);
    let raw = listening_cloud_raw(&canonical_value, model_parts);
    let identity = listening_identity(&canonical_value);
    let normalized = normalize_cloud_authoring(&identity, Some(&canonical_value), &raw)
        .expect("听力候选必须能标准化");
    let candidate =
        cloud_authoring_candidate_from_normalized(&identity, normalized).expect("必须可装配");
    let candidate_value = serde_json::to_value(&candidate.authoring).expect("候选可序列化");

    // 归一化后的候选必须真的带上了听力结构（这正是 T4 要修的那件事）。
    let candidate_parts = candidate_value
        .pointer("/listening/parts")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("候选必须带 listening.parts：{candidate_value}"));
    assert_eq!(candidate_parts.len(), 3, "模型给了三段，候选就必须是三段");

    // 既有分段：身份与音频原样保留。
    for ordinal in 1..=2 {
        let part = candidate_parts
            .iter()
            .find(|part| part["partId"] == format!("part-{ordinal}"))
            .unwrap_or_else(|| panic!("part-{ordinal} 必须被复用：{candidate_parts:?}"));
        assert!(
            !part["media"].is_null(),
            "复用的 Part 必须带着用户已经绑好的音频：{part}"
        );
        assert_eq!(part["media"]["sha256"], canonical_value["listening"]["parts"][ordinal - 1]["media"]["sha256"]);
    }
    // 并出来的新段：后端分配身份，且**不带**音频（模型无权给音频事实）。
    let merged = candidate_parts
        .iter()
        .find(|part| part["expectedQuestionNumbers"] == json!(listening_numbers(21..=40)))
        .unwrap_or_else(|| panic!("必须有一段覆盖 21-40：{candidate_parts:?}"));
    assert!(
        merged["media"].is_null() || merged.get("media").is_none(),
        "新分段不能凭空带上音频：{merged}"
    );
    let merged_id = merged["partId"].as_str().expect("新分段必须有身份").to_string();
    assert!(
        !canonical_value["listening"]["parts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|part| part["partId"] == json!(merged_id.clone())),
        "新分段必须拿到一个未被占用的 id，不能借用既有分段：{merged_id}"
    );

    // 存盘，然后走用户真正看到的「剩余问题」重算。
    store::write_cloud_authoring_candidate(&root, BATCH_ID, &candidate).expect("落盘候选");
    let tasks =
        remaining_tasks(&root, ITEM_ID, ITEM_ID, BATCH_ID, &[], &[]).expect("重算剩余任务");

    let boundary_ids: Vec<String> = tasks
        .iter()
        .filter(|task| task["field"] == "part_boundary")
        .filter_map(|task| task["userTaskId"].as_str().map(str::to_string))
        .collect();
    assert!(
        boundary_ids.contains(&format!("cloud-diff:part:{merged_id}:part_boundary")),
        "模型新加的分段必须出现在用户清单里：{boundary_ids:?}"
    );
    for removed in ["part-3", "part-4"] {
        assert!(
            boundary_ids.contains(&format!("cloud-diff:part:{removed}:part_boundary")),
            "被并掉的 {removed} 必须出现在用户清单里：{boundary_ids:?}"
        );
    }
    // 没变的分段不该冒出来打扰用户。
    for unchanged in ["part-1", "part-2"] {
        assert!(
            !boundary_ids.contains(&format!("cloud-diff:part:{unchanged}:part_boundary")),
            "{unchanged} 没变，不该产生分段差异：{boundary_ids:?}"
        );
    }

    // 人话说明必须能落地：任务要能让用户看懂「哪一段的分段范围对不上」。
    let task = tasks
        .iter()
        .find(|task| task["userTaskId"] == format!("cloud-diff:part:{merged_id}:part_boundary"))
        .expect("新分段的差异任务");
    let message = task["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("听力 Part") && message.contains("分段范围"),
        "任务说明必须指出是哪一段、差在什么上：{message}"
    );
    assert!(!task["cloudValue"].is_null(), "{task}");
    assert_eq!(task["action"], "review_difference");

    // 给模型看的上下文里**不能**出现音频指纹：模型无权也无法核对它。
    let serialized = serde_json::to_string(&tasks).expect("任务可序列化");
    assert!(
        !serialized.contains(&canonical_value["listening"]["parts"][0]["media"]["sha256"]
            .as_str()
            .unwrap()
            .to_string()),
        "音频哈希不得进入用户任务 / 模型上下文"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 分段裁定必须绑定**这一段的身份与范围**，而不只是差异两侧的字面值。
///
/// 复现：模型对 `part-3` 的标签差异裁定「当前稿对」。随后该段的 `cue`（这一段音频的
/// 起止边界）被改写——分段没变、标签差异两侧也没变，旧代码于是继续压住这条差异。
#[test]
fn a_part_ruling_dies_when_the_boundary_it_depended_on_changes() {
    let mut canonical_value =
        serde_json::to_value(crate::test_support::complete_listening_exam()).expect("听力夹具");
    let candidate_value = {
        let mut value = canonical_value.clone();
        value["listening"]["parts"][2]["displayLabel"] = json!("SECTION THREE");
        value
    };

    let differences = candidate_differences(&canonical_value, &candidate_value);
    let label_diff = differences
        .iter()
        .find(|difference| {
            difference["targetType"] == "part"
                && difference["targetId"] == "part-3"
                && difference["field"] == "part_label"
        })
        .unwrap_or_else(|| panic!("夹具必须先产生 part-3 的标签差异：{differences:?}"));
    let (canonical_digest, candidate_digest, context_digest) = difference_digests(label_diff);
    let ruling = json!({
        "targetType": "part",
        "targetId": "part-3",
        "field": "part_label",
        "ruling": crate::schema::cloud_repair_v1::CLOUD_RULING_CURRENT_IS_CORRECT,
        "canonicalDigest": canonical_digest,
        "candidateDigest": candidate_digest,
        "contextDigest": context_digest,
    });
    let rulings = vec![ruling];
    assert_eq!(
        effective_adjudicated_count(&canonical_value, &candidate_value, &rulings),
        1,
        "裁定在前提未变时必须生效"
    );

    // 只改这一段音频的起止边界：标签差异两侧一个字没变。
    canonical_value["listening"]["parts"][2]["cue"] = json!({
        "startMs": 0, "endMs": 240000, "confidence": 1.0, "confirmed": true
    });
    assert_eq!(
        effective_adjudicated_count(&canonical_value, &candidate_value, &rulings),
        0,
        "分段边界变了，基于旧边界的裁定必须失效重评"
    );
}

// ── 包模式（默认）：本地预切 → 不够就说 → 自己去取 → 最差如实交给用户 ──────────
//
// 这一节守的是任务书 §5 的 1-8 条。与上一节的分工：上一节跑 **legacy**（改造前的行为
// 一字未变），这一节跑 **packets**（生产默认）。两节的断言都必须在，缺一节就等于
// 「新模式上线、老行为没人看」或「老行为还在、新模式没人看」。

/// 包模式要用的原文夹具：三页，**答案页故意不在文本层模式里**（只有一行 `14 A`，
/// 少于「两条以上带题号的行」这条判据），于是切包时定位不到答案页 → `answerPagesUnknown`。
///
/// 这正是要验的场景：包**不知道**答案在哪，模型必须自己报「不够」并要回那一页。
fn seed_packet_job(root: &Path) {
    use crate::job_store::{make_job, save_job};
    use crate::util::{ensure_job_dirs, job_dir, write_json};
    use crate::{CreateJobInput, SourceFile, WorkflowStep};

    let mut job = make_job(CreateJobInput {
        title: Some("Early Approaches".to_string()),
        category: Some("P1".to_string()),
        frequency: Some("medium".to_string()),
        tags: Some(vec!["packets".to_string()]),
        llm_profile_id: None,
    });
    job.job_id = ITEM_ID.to_string();
    job.current_step = WorkflowStep::Authoring;
    job.active_llm_profile_id = Some("controlled-repair".to_string());
    job.source_files = vec![SourceFile {
        file_id: "early-approaches-pdf".to_string(),
        original_name: "early-approaches.pdf".to_string(),
        stored_name: "early-approaches.pdf".to_string(),
        file_type: "pdf".to_string(),
        sha256: "a".repeat(64),
        size_bytes: 8,
        role: "MainQuestion".to_string(),
        imported_at: chrono::Utc::now(),
    }];
    save_job(root, &job).expect("save job");
    let dir = job_dir(root, ITEM_ID);
    ensure_job_dirs(&dir).expect("job dirs");
    std::fs::create_dir_all(dir.join("uploads")).expect("uploads dir");
    std::fs::write(dir.join("uploads").join("early-approaches.pdf"), b"%PDF-1.4\n")
        .expect("write source");
    write_json(
        &dir.join("document-ir.json"),
        &json!({"pages": [
            // 0-based：pageIndex 0 → 1-based 第 1 页。
            {"pageIndex": 0, "lines": [
                {"text": "Questions 14-15"},
                {"text": "Which TWO factors influenced early organisational design?"}
            ]},
            {"pageIndex": 1, "lines": [
                {"text": "Section 2"},
                {"text": "Notes on the reading passage"}
            ]},
            // 答案页：只有**一行**带题号，因此文本层判据认不出它是答案区。
            {"pageIndex": 2, "lines": [
                {"text": "Answer key"},
                {"text": "14 A"}
            ]}
        ]}),
    )
    .expect("document-ir");
}

/// 默认模式必须是 packets，且 legacy 只能靠诊断开关进入。
#[test]
fn the_default_context_mode_is_packets_and_legacy_is_diagnostic_only() {
    assert_eq!(REPAIR_CONTEXT_MODE, RepairContextMode::Packets);
    assert_eq!(configured_repair_context_mode(), RepairContextMode::Packets);
}

/// 包模式下每轮的上下文是**一个校核包**：范围自足、带差异、带稿件切片与行 id；
/// 范围外的页一律不在包里（那正是输入量下降的来源）。
#[test]
fn a_packet_carries_only_its_own_scope_and_the_differences_inside_it() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");
    seed_packet_job(&root);

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    let mut seen: Vec<Value> = Vec::new();
    let report = run_packets(&request, |context: &Value, _observations: &[Value]| {
        seen.push(context.clone());
        Ok(json!({"callId": "p1", "tool": "finish_packet", "arguments": {}}))
    })
    .expect("包模式循环必须返回结果");

    let packet = seen.first().expect("必须至少有一轮").clone();
    assert_eq!(packet["contextMode"], json!("packets"));
    assert_eq!(packet["schemaVersion"], json!("RepairPacketV1"));
    assert_eq!(
        packet["taskIds"],
        json!(["early-approaches-q14-15"]),
        "差异必须归属到本地题组：{packet:#?}"
    );
    assert!(
        packet["differences"]
            .as_array()
            .is_some_and(|differences| !differences.is_empty()),
        "包里必须带着要核的差异：{packet:#?}"
    );
    assert!(
        packet["draftSlice"]["editVersion"].as_i64().unwrap_or(0) > 0,
        "稿件切片必须带当前 editVersion（apply_edits 要用它）：{packet:#?}"
    );

    // 范围自足：包里出现的页**只能**是 scope.pages 里的页。
    let scope: Vec<u64> = packet["scope"]["pages"]
        .as_array()
        .map(|items| items.iter().filter_map(Value::as_u64).collect())
        .unwrap_or_default();
    let included: Vec<u64> = packet["sourceEvidence"]["pages"]
        .as_array()
        .map(|items| items.iter().filter_map(|page| page.get("pageIndex").and_then(Value::as_u64)).collect())
        .unwrap_or_default();
    assert!(
        !included.is_empty(),
        "包里一页证据都没有时 `all(...)` 是空真，证明不了「范围自足」：{packet:#?}"
    );
    assert!(
        included.iter().all(|page| scope.contains(page)),
        "包里有范围外的页：scope={scope:?} included={included:?}"
    );

    // 答案页定位不到时必须**如实说明**，不能拿空数组冒充「这份卷子没有答案页」。
    assert_eq!(packet["scope"]["answerPages"], json!([]));
    assert_eq!(packet["scope"]["answerPagesUnknown"], json!(true));

    // 逐包诊断：级别、轮数、范围都记下来了。
    assert_eq!(report.packets.len(), 1, "{:#?}", report.packets);
    assert_eq!(report.packets[0]["status"], json!("finished"));
    assert_eq!(report.packets[0]["escalationLevel"], json!(0));

    let _ = std::fs::remove_dir_all(&root);
}

/// 包的「行 id」必须与页号一致：模型引用 `p3:l2` 时，后端能对到真实那一行。
///
/// 页号 0-based / 1-based 混淆在这里最要命——模型会引到隔壁页的句子，看起来有出处，
/// 出处却是错的。
#[test]
fn packet_source_lines_carry_one_based_page_ids_matching_the_text_layer() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");
    seed_packet_job(&root);

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    let mut first: Option<Value> = None;
    run_packets(&request, |context: &Value, _observations: &[Value]| {
        if first.is_none() {
            first = Some(context.clone());
        }
        Ok(json!({"callId": "p1", "tool": "finish_packet", "arguments": {}}))
    })
    .expect("包模式循环必须返回结果");

    let packet = first.expect("必须至少有一轮");
    let pages = packet["sourceEvidence"]["pages"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(!pages.is_empty(), "包里必须有原文页：{packet:#?}");

    // 文本层（`document-ir.json`）是 0-based，包里对外一律 1-based。断言不能只对着
    // 常量喊话，要**对着文本层**核：包里的第 N 页必须逐字等于文本层 pageIndex=N-1。
    let dir = crate::util::job_dir(&root, ITEM_ID);
    let text_layer = crate::util::read_json_opt(&dir.join("document-ir.json"))
        .expect("读 document-ir")
        .expect("document-ir 必须存在")["pages"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(!text_layer.is_empty(), "夹具的文本层不能是空的");

    // 题组锚点是 0-based `pageIndex: 1` ⇒ 包里必须出现第 **2** 页。若换算漏做一次，
    // 这里会看到第 1 页——那正是「模型引到隔壁页的句子」这个坑的入口。
    let anchor_page = pages
        .iter()
        .find(|page| page["pageIndex"] == json!(2))
        .unwrap_or_else(|| panic!("锚点页（0-based 1 → 1-based 2）必须在包里：{packet:#?}"));
    assert_eq!(anchor_page["lines"][0]["id"], json!("p2:l1"));
    assert_eq!(anchor_page["lines"][0]["text"], json!("Section 2"));

    for page in &pages {
        let page_index = page["pageIndex"].as_u64().expect("页号必须是数字");
        let expected = text_layer
            .iter()
            .find(|entry| entry["pageIndex"] == json!(page_index - 1))
            .unwrap_or_else(|| panic!("第 {page_index} 页在文本层里不存在：{text_layer:#?}"));
        let lines = page["lines"].as_array().expect("逐行文本");
        assert_eq!(
            lines.len(),
            expected["lines"].as_array().map(Vec::len).unwrap_or(0),
            "第 {page_index} 页的行数必须与文本层一致"
        );
        for (position, line) in lines.iter().enumerate() {
            assert_eq!(
                line["id"],
                json!(format!("p{page_index}:l{}", position + 1)),
                "行 id 的页号必须与所在页对象一致（1-based）"
            );
            assert_eq!(
                line["text"], expected["lines"][position]["text"],
                "行文本必须逐字来自该页文本层，不能是隔壁页的句子"
            );
        }
    }

    let _ = std::fs::remove_dir_all(&root);
}

/// 包**不知道**答案页在哪时，模型必须能说「不够」并要回那一页；下一轮请求里必须
/// 真的出现那一页的原文——否则「回退真的在传内容」这句话就没有证据。
///
/// 这一条同时钉住 §6 的防自证要求：假模型在拿到那一页**之前**绝不可能说出 `14 A`。
#[test]
fn a_packet_that_lacks_the_answer_page_says_so_and_gets_it_next_round() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    // 候选说 q14=A；原文件（第 3 页）也说 A ⇒ 当前稿的 B 是错的，模型应当改稿。
    store_candidate(&root, "A");
    seed_packet_job(&root);

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    let mut rounds: Vec<Value> = Vec::new();
    let mut saw_answer_before_fetching = false;
    let mut version_after_fetch = -1i64;
    let report = run_packets(&request, |context: &Value, observations: &[Value]| {
        rounds.push(context.clone());
        let answer_visible = context.to_string().contains("14 A");
        match rounds.len() {
            1 => {
                saw_answer_before_fetching = answer_visible;
                Ok(json!({
                    "callId": "c1",
                    "tool": "report_insufficient_context",
                    "arguments": {
                        "packetId": context["packetId"],
                        "reason": "the answer page is not in scope",
                        "needs": [{"kind": "pages", "from": 3, "to": 3}]
                    }
                }))
            }
            2 => {
                // 拿到那一页之后才可能知道答案；引文必须逐字来自返回的行。
                assert!(answer_visible, "第 3 页必须已经并入本包：{context:#?}");
                // 上一轮的「不够」必须收到**结构化回应**：取到了什么、什么没取到，都在里面。
                // 只回一句「已处理」是不够的——模型得知道自己下一轮手里有什么。
                let answer = observations.last().cloned().unwrap_or(Value::Null);
                assert_eq!(answer["status"], json!("ok"), "报「不够」必须收到结果：{answer:#?}");
                assert_eq!(
                    answer["result"]["status"],
                    json!("needs_answered"),
                    "{answer:#?}"
                );
                assert!(
                    answer["result"]["satisfied"]
                        .as_array()
                        .is_some_and(|items| !items.is_empty()),
                    "取回的内容必须回给模型：{answer:#?}"
                );
                assert_eq!(
                    answer["result"]["unsatisfied"],
                    json!([]),
                    "这一条需求是能满足的：{answer:#?}"
                );
                version_after_fetch = context["draftSlice"]["editVersion"].as_i64().unwrap_or(-1);
                Ok(json!({
                    "callId": "c2",
                    "tool": "apply_edits",
                    "arguments": {
                        "baseVersion": version_after_fetch,
                        "commands": [set_answer("q14", "A")],
                        "evidence": [{
                            "sourceFileId": "early-approaches-pdf",
                            "pageIndex": 3,
                            "quote": "14 A"
                        }]
                    }
                }))
            }
            // 编辑落地 ⇒ 差异归零 ⇒ 重切出一个**只带索引**的收尾包（任务书 §4.1：一条
            // 差异都没有时也给模型一次机会）。它的观察是空的，所以这一轮不碰 observations。
            _ => Ok(json!({"callId": "c3", "tool": "finish_packet", "arguments": {}})),
        }
    })
    .expect("包模式循环必须返回结果");

    assert!(
        !saw_answer_before_fetching,
        "答案页不在请求里时，模型不可能知道答案——夹具必须证明这一点"
    );
    assert!(version_after_fetch > 0, "必须读到真实的 editVersion");
    assert_eq!(read_answer(&root, "q14")["labels"], json!(["A"]), "编辑必须真的落库");
    assert_eq!(report.applied_count, 1);
    assert_eq!(report.packets[0]["insufficientContext"], json!(1));
    assert_eq!(
        report.packets[0]["escalationLevel"],
        json!(1),
        "模型主动报「不够」就是 L1：{:#?}",
        report.packets[0]
    );
    assert_eq!(report.packets[0]["status"], json!("edited"));
    assert_eq!(
        report.packets[0]["edits"],
        json!(1),
        "逐包诊断必须记下这一包落了几个编辑：{:#?}",
        report.packets[0]
    );
    assert_eq!(
        report.packets.len(),
        2,
        "差异修完之后的收尾包也要出现在诊断里：{:#?}",
        report.packets
    );
    assert_eq!(report.packets[1]["status"], json!("finished"));
    assert_eq!(
        report.status,
        REPAIR_STATUS_COMPLETED,
        "每包都收工、队列自然跑空 ⇒ 这是一次**完成**，不是预算耗尽"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn insufficient_context_report_must_name_the_active_packet() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");
    seed_packet_job(&root);

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    let mut calls = 0usize;
    let report = run_packets(&request, |context: &Value, observations: &[Value]| {
        calls += 1;
        match calls {
            1 => Ok(json!({
                "callId": "wrong-packet",
                "tool": "report_insufficient_context",
                "arguments": {
                    "packetId": format!("{}-stale", context["packetId"].as_str().unwrap()),
                    "reason": "the answer page is missing",
                    "needs": [{"kind": "pages", "from": 3, "to": 3}]
                }
            })),
            2 => {
                assert_eq!(
                    observations[0]["status"],
                    json!("rejected"),
                    "旧包的上下文不足请求不能作用于当前包：{observations:#?}"
                );
                assert!(
                    observations[0]["errors"].to_string().contains("PACKET_MISMATCH"),
                    "拒绝原因必须点明 packetId 不匹配：{observations:#?}"
                );
                Ok(json!({"callId": "finish", "tool": "finish_packet", "arguments": {}}))
            }
            _ => Ok(json!({"callId": "finish-again", "tool": "finish_packet", "arguments": {}})),
        }
    })
    .expect("包模式循环必须返回结果");

    assert_eq!(calls, 2);
    assert_eq!(report.packets[0]["status"], json!("finished"));
    assert_eq!(
        report.packets[0]["insufficientContext"],
        json!(0),
        "被拒绝的旧包报告不应计作一次真实的上下文不足报告"
    );
    assert_eq!(
        report.packets[0]["escalationLevel"],
        json!(0),
        "被拒绝的旧包报告不应推进 L1"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 始终拿不到材料 ⇒ 后端**代记** `cannot_resolve`（理由码 `CONTEXT_INSUFFICIENT`），
/// 差异进用户清单且文案明说「云端没能拿到足够的原文」。
///
/// 三态不坍缩就落在这里：这条差异**没有**被核对过，绝不能被算成已核对、也不能变成
/// 「云端猜了一个」。
#[test]
fn a_packet_that_never_gets_enough_context_hands_the_difference_to_the_user_honestly() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");
    seed_packet_job(&root);

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    let report = run_packets(&request, |context: &Value, _observations: &[Value]| {
        // 要一个**不存在**的页：需求永远满足不了 ⇒ 每轮升级一档，直到 L4。
        Ok(json!({
            "callId": "c1",
            "tool": "report_insufficient_context",
            "arguments": {
                "packetId": context["packetId"],
                "reason": "the page I need is missing",
                "needs": [{"kind": "pages", "from": 99, "to": 99}]
            }
        }))
    })
    .expect("包模式循环必须返回结果");

    assert_ne!(report.status, REPAIR_STATUS_COMPLETED, "上下文不足**不得**报成完成");
    assert_eq!(report.status, REPAIR_STATUS_NEEDS_ATTENTION);
    assert_eq!(
        report.packets[0]["escalationLevel"],
        json!(4),
        "必须走完 L1→L4：{:#?}",
        report.packets[0]
    );

    let task = report
        .remaining_tasks
        .iter()
        .find(|task| task["contextInsufficient"] == json!(true))
        .unwrap_or_else(|| panic!("必须有一条「上下文不足」的用户任务：{:#?}", report.remaining_tasks));
    assert_eq!(
        task["message"],
        json!("云端没能拿到足够的原文来判断第 14-15 题，请对照原文确认")
    );

    // 原稿一字未改：没有材料就不许猜。
    assert_eq!(read_answer(&root, "q14")["labels"], json!(["B"]));
    // 「上下文不足」不是裁定：云端根本没拿到材料，争议还在用户手上。把它算进
    // 「已了结」就是假完成 —— TASK §2 明写「上下文不足不得算作已核对」。
    assert_eq!(
        report.adjudicated_count, 0,
        "上下文不足不得被算成「已了结」：report={:#?}",
        report.packets
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 抓取边界（§5.7）：无页范围被拒、超页数被拒、越界页被拒、`search_source` 返回行 id。
/// 每一条的**原因必须具体**，模型才能据此改对而不是反复瞎试。
#[test]
fn grab_tools_reject_out_of_bounds_requests_with_specific_reasons() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");
    seed_packet_job(&root);

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    let mut round = 0usize;
    let report = run_packets(&request, |_context: &Value, observations: &[Value]| {
        round += 1;
        Ok(match round {
            1 => json!({"callId": "g1", "tool": "read_source", "arguments": {}}),
            2 => json!({"callId": "g2", "tool": "read_source",
                        "arguments": {"pageIndex": 1, "pageTo": 9}}),
            3 => json!({"callId": "g3", "tool": "read_source",
                        "arguments": {"pageIndex": 99}}),
            4 => json!({"callId": "g4", "tool": "search_source",
                        "arguments": {"query": "organisational design"}}),
            5 => {
                // 前四轮的拒绝/命中必须都在本包观察里（不跨包、不丢）。
                let errors: Vec<String> = observations
                    .iter()
                    .filter_map(|observation| observation.get("errors").and_then(Value::as_array))
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                assert!(
                    errors.iter().any(|error| error.starts_with("CLOUD_GRAB_PAGE_RANGE_REQUIRED")),
                    "无选择器的 read_source 必须被拒：{errors:?}"
                );
                assert!(
                    errors.iter().any(|error| error.starts_with("CLOUD_GRAB_PAGE_LIMIT_EXCEEDED")),
                    "超过单次页数上限必须被拒：{errors:?}"
                );
                assert!(
                    errors.iter().any(|error| error.starts_with("CLOUD_GRAB_PAGE_OUT_OF_RANGE")),
                    "越界页必须被拒：{errors:?}"
                );
                let hits = observations
                    .iter()
                    .find_map(|observation| observation.pointer("/result/hits"))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                assert!(!hits.is_empty(), "search_source 必须真的搜到行：{observations:#?}");
                assert_eq!(hits[0]["lineId"], json!("p1:l2"));
                assert_eq!(hits[0]["pageIndex"], json!(1));
                json!({"callId": "g5", "tool": "finish_packet", "arguments": {}})
            }
            _ => json!({"callId": "g6", "tool": "finish_packet", "arguments": {}}),
        })
    })
    .expect("包模式循环必须返回结果");
    assert_eq!(report.applied_count, 0);
    let _ = std::fs::remove_dir_all(&root);
}

/// `read_draft` 在包里**默认范围限定本包**：范围外的题组要被明确拒绝（而不是偷偷把
/// 整卷返回回来——那等于包白切了）。
#[test]
fn read_draft_outside_the_packet_is_rejected_instead_of_returning_the_whole_paper() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");
    seed_packet_job(&root);

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    let mut round = 0usize;
    let report = run_packets(&request, |_context: &Value, observations: &[Value]| {
        round += 1;
        match round {
            1 => Ok(json!({"callId": "d1", "tool": "read_draft", "arguments": {}})),
            2 => Ok(json!({"callId": "d2", "tool": "read_draft",
                           "arguments": {"taskGroupIds": ["not-in-this-packet"]}})),
            3 => {
                let errors: Vec<&str> = observations
                    .iter()
                    .filter_map(|observation| observation.get("errors").and_then(Value::as_array))
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect();
                assert!(
                    errors.iter().any(|error| error.starts_with("CLOUD_DRAFT_SCOPE_REQUIRED")),
                    "不给选择器必须被拒：{errors:?}"
                );
                assert!(
                    errors.iter().any(|error| error.starts_with("CLOUD_DRAFT_OUTSIDE_PACKET")),
                    "范围外的题组必须被拒：{errors:?}"
                );
                Ok(json!({"callId": "d3", "tool": "read_draft",
                          "arguments": {"taskGroupIds": ["early-approaches-q14-15"]}}))
            }
            4 => {
                assert_eq!(
                    observations.last().map(|value| value["status"].clone()),
                    Some(json!("ok")),
                    "包内题组必须能读：{observations:#?}"
                );
                Ok(json!({"callId": "d4", "tool": "finish_packet", "arguments": {}}))
            }
            _ => Ok(json!({"callId": "d5", "tool": "finish_packet", "arguments": {}})),
        }
    })
    .expect("包模式循环必须返回结果");
    assert_eq!(report.packets.len(), 1);
    let _ = std::fs::remove_dir_all(&root);
}

/// 编辑之后必须**重切受影响的包**：下一轮的 `editVersion` 是新的，已经修掉的差异
/// 不再出现在包里，剩下的差异仍然在（否则就是「改完就不管了」）。
#[test]
fn an_applied_edit_reslices_the_packet_with_a_fresh_version_and_fewer_differences() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    // 两处差异：q14（B→A）与 q15（D→C）。修掉 q14 之后包里只剩 q15。
    store_candidate_with(&root, "A", "C");
    seed_packet_job(&root);

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    let mut rounds: Vec<Value> = Vec::new();
    let report = run_packets(&request, |context: &Value, _observations: &[Value]| {
        rounds.push(context.clone());
        match rounds.len() {
            1 => {
                let version = context["draftSlice"]["editVersion"].as_i64().unwrap_or(-1);
                assert_eq!(
                    context["differences"].as_array().map(Vec::len),
                    Some(2),
                    "两处差异必须同包：{context:#?}"
                );
                Ok(json!({"callId": "e1", "tool": "apply_edits",
                          "arguments": {"baseVersion": version, "commands": [set_answer("q14", "A")]}}))
            }
            2 => {
                let version = context["draftSlice"]["editVersion"].as_i64().unwrap_or(-1);
                let first = rounds[0]["draftSlice"]["editVersion"].as_i64().unwrap_or(-1);
                assert!(
                    version > first,
                    "重切之后必须带**新**版本（{first} -> {version}）"
                );
                let differences = context["differences"].as_array().cloned().unwrap_or_default();
                assert_eq!(differences.len(), 1, "已修掉的差异不得再出现：{context:#?}");
                // 差异的目标是**本地**答案槽 id（候选的 `cloud-q15` 在标准化时已经映射回
                // 本地槽），否则用户清单会指向一个稿件里根本不存在的 id。
                assert_eq!(differences[0]["targetId"], json!("q15"));
                assert!(
                    context["draftSlice"]["answerKey"]["q14"]["labels"] == json!(["A"]),
                    "新切片必须反映刚落库的编辑：{context:#?}"
                );
                Ok(json!({"callId": "e2", "tool": "finish_packet", "arguments": {}}))
            }
            _ => Ok(json!({"callId": "e3", "tool": "finish_packet", "arguments": {}})),
        }
    })
    .expect("包模式循环必须返回结果");

    assert_eq!(report.applied_count, 1);
    assert_eq!(read_answer(&root, "q14")["labels"], json!(["A"]));
    assert!(
        report.packets.len() >= 2,
        "重切之后应当多出一个包（剩下的差异）：{:#?}",
        report.packets
    );
    assert_eq!(
        report.packets[0]["status"],
        json!("edited"),
        "{:#?}",
        report.packets[0]
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 已完成包不能只按差异键跳过：另一个包的成功编辑可能让相同差异键带上了新值。
/// 重切后内容已变化的包必须重新排队，避免 `done_packets` 把仍存在的差异误当作已核。
#[test]
fn a_replanned_packet_with_changed_difference_values_is_not_skipped_as_done() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    // 本地 q14 差异先收工；候选额外的 21–22 题组会形成第二个文档包。
    store_candidate_draft(&root, cloud_draft_with_extra_group("A", "D"));
    seed_packet_job(&root);

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    let mut packets_seen: Vec<Value> = Vec::new();
    let report = run_packets(&request, |packet: &Value, _observations: &[Value]| {
        packets_seen.push(packet.clone());
        match packets_seen.len() {
            1 => {
                assert_eq!(packet["documentOnly"], json!(false), "本地差异包应先处理");
                Ok(json!({"callId": "stale-1", "tool": "finish_packet", "arguments": {}}))
            }
            2 => {
                assert_eq!(packet["documentOnly"], json!(true), "候选独有题组应形成文档包");
                // 真实工具执行仍校验 CAS 与证据；这里刻意编辑一个其他包仍有差异的答案，
                // 让它保持同一差异键、但 canonical 值发生变化。
                let version = packet["draftSlice"]["editVersion"].as_i64().unwrap_or(-1);
                Ok(json!({"callId": "stale-2", "tool": "apply_edits", "arguments": {
                    "baseVersion": version,
                    "commands": [set_answer("q14", "E")]
                }}))
            }
            3 => {
                if packet["documentOnly"] == json!(false) {
                    assert_eq!(packet["differences"][0]["targetId"], json!("q14"));
                    assert_eq!(packet["differences"][0]["canonical"]["labels"], json!(["E"]));
                }
                // 旧实现会把 q14 包当成已做完而跳过，只剩文档包；修复后先重跑 q14，
                // 再收工文档包。两种情况下本回合都可以结束当前包。
                Ok(json!({"callId": "stale-3", "tool": "finish_packet", "arguments": {}}))
            }
            4 => {
                assert_eq!(packet["documentOnly"], json!(true));
                Ok(json!({"callId": "stale-4", "tool": "finish_packet", "arguments": {}}))
            }
            _ => panic!("循环不应重复或额外处理包: {packet:#?}"),
        }
    })
    .expect("包模式循环必须返回结果");

    assert_eq!(read_answer(&root, "q14")["labels"], json!(["E"]));
    assert_eq!(
        packets_seen.len(),
        4,
        "重切后 q14 的差异仍存在且值已变，必须重新排队：{:#?}",
        report.packets
    );
    assert!(
        packets_seen.iter().any(|packet| {
            packet["documentOnly"] == json!(false)
                && packet["differences"][0]["targetId"] == json!("q14")
                && packet["differences"][0]["canonical"]["labels"] == json!(["E"])
        }),
        "q14 新内容的差异包必须重新出现：{packets_seen:#?}"
    );
    assert_eq!(report.packets.len(), 4);
    assert_ne!(
        report.status,
        REPAIR_STATUS_COMPLETED,
        "候选差异仍在时不得误报整次校核 completed"
    );
    assert!(
        report.remaining_tasks.iter().any(|task| {
            task["targetIds"]
                .as_array()
                .is_some_and(|targets| targets.iter().any(|target| target == "q14"))
        }),
        "仍未解决的 q14 差异必须留给用户：{:#?}",
        report.remaining_tasks
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 循环纪律在包模式下同样成立：取消立刻停、网关不可用如实降级、终态不留 running。
#[test]
fn the_packet_loop_keeps_the_loop_discipline_of_cancel_unavailable_and_terminal_state() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");
    seed_packet_job(&root);

    // 取消：一次模型调用都不许发生。
    let cancelled = || true;
    let cancelled_request = request(&root, &cancelled, 6);
    let mut calls = 0u32;
    let report = run_packets(&cancelled_request, |_context: &Value, _observations: &[Value]| {
        calls += 1;
        Ok(json!({"callId": "x", "tool": "finish_packet", "arguments": {}}))
    })
    .expect("必须返回结果");
    assert_eq!(report.status, REPAIR_STATUS_CANCELLED);
    assert_eq!(calls, 0);

    // 网关不可用：如实 unavailable，稿子不动。
    let not_cancelled = || false;
    let live_request = request(&root, &not_cancelled, 6);
    let report = run_packets(&live_request, |_context: &Value, _observations: &[Value]| {
        Err("llm_http_500:upstream".to_string())
    })
    .expect("必须返回结果");
    assert_eq!(report.status, REPAIR_STATUS_UNAVAILABLE);
    assert_eq!(report.last_error.as_deref(), Some("llm_http_500:upstream"));
    assert_eq!(read_answer(&root, "q14")["labels"], json!(["B"]));

    let _ = std::fs::remove_dir_all(&root);
}

/// 剧本（包模式）：第一包**故意**不含答案页 → `report_insufficient_context` →
/// 拿到那一页后 `apply_edits` → `finish_packet`。
///
/// 引文 `14 A` **只能**从第 3 页的真实返回里读出来：第一轮的请求体里没有它。
fn scripted_packet_reply(body: &str, round: usize) -> String {
    let input = repair_request_input(body);
    let version = input
        .as_ref()
        .and_then(|value| value.pointer("/context/draftSlice/editVersion"))
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    let answer_page_in_scope = input
        .as_ref()
        .and_then(|value| value.pointer("/context/scope/pages"))
        .and_then(Value::as_array)
        .map(|pages| pages.iter().any(|page| page == &json!(3)))
        .unwrap_or(false);
    match round {
        1 => json!({
            "callId": "p1",
            "tool": "report_insufficient_context",
            "arguments": {
                "packetId": input.as_ref().and_then(|value| value.pointer("/context/packetId")).cloned().unwrap_or(Value::Null),
                "reason": "the answer page is not in scope",
                "needs": [{"kind": "pages", "from": 3, "to": 3}]
            }
        }),
        2 if answer_page_in_scope => json!({
            "callId": "p2",
            "tool": "apply_edits",
            "arguments": {
                "baseVersion": version,
                "commands": [set_answer("q14", "A")],
                "evidence": [{
                    "sourceFileId": "early-approaches-pdf",
                    "pageIndex": 3,
                    "quote": "14 A"
                }]
            }
        }),
        _ => json!({
            "callId": "p3",
            "tool": "finish_packet",
            "arguments": {"note": "受控服务：q14 已按原文件改为 A"}
        }),
    }
    .to_string()
}

/// 包模式 + **真实 HTTP 网关**：请求体里**没有**整份 PDF 附件，只有范围内页文本与
/// 区域图；`report_insufficient_context` 取回的那一页在下一轮请求里真实出现。
///
/// 这一条是 §5.5 的可执行版本（只在网关层看得到「附了什么」），也是 macOS 上能做到的
/// 最强证据：产品端到端那条 CDP 链只跑在 Windows（见报告）。
#[test]
fn packets_mode_requests_carry_no_whole_pdf_and_the_fetched_page_reaches_the_model() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");
    seed_packet_job(&root);

    let (base_url, requests) = spawn_scripted_repair_service_with(scripted_packet_reply);
    crate::llm_profiles::save_profiles(
        &root,
        &[json!({
            "profileId": "controlled-repair",
            "name": "Controlled Repair Service",
            "provider": "OpenAiCompatible",
            "baseUrl": base_url,
            "model": "controlled-repair-v1",
            "temperature": 0,
            "timeoutMs": 60000,
            "forceJson": true,
            "enabled": true
        })],
    )
    .expect("profile 必须能落盘");

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    let report = run_packets(&request, |context: &Value, observations: &[Value]| {
        repair_authoring_step_through_gateway(
            &root,
            ITEM_ID,
            Some("controlled-repair"),
            context,
            observations,
        )
    })
    .expect("包模式循环必须跑完");

    let seen = requests.lock().expect("requests");
    assert!(seen.len() >= 2, "至少要有两轮真实 HTTP 请求，实际 {}", seen.len());
    // ① 包模式下**不附**整份原文件：那正是这一轮要消掉的输入量。
    assert!(
        !seen[0].contains("application/pdf"),
        "包模式的请求不得携带整份 PDF 附件：{}",
        &seen[0][..seen[0].len().min(600)]
    );
    // ② 模型确实看到了「这是一个包」与包里要核的差异。
    assert!(seen[0].contains("RepairPacketV1"), "请求必须带上包本身");
    // 只断言工具名是**恒真**的：工具清单无条件拼在 prompt 里，模型即使没被告知这条
    // 出口也能通过。要断言的是「不够就说」这条出口的**说明**确实在，且写明不许猜。
    assert!(
        seen[0].contains("call `report_insufficient_context` with the exact pages"),
        "prompt 必须告诉模型「不够就说」这条出口怎么用：{}",
        &seen[0][..seen[0].len().min(600)]
    );
    assert!(
        seen[0].contains("do NOT guess"),
        "prompt 必须写明「上下文不够时不许猜」"
    );
    // ③ 第一轮里没有答案页那一行；第二轮里必须有（回退真的在传内容）。
    assert!(!seen[0].contains("14 A"), "第一轮不该凭空出现答案页内容");
    assert!(
        seen[1].contains("14 A"),
        "第二轮必须带上模型要回来的那一页：{}",
        &seen[1][..seen[1].len().min(600)]
    );
    drop(seen);

    assert_eq!(read_answer(&root, "q14")["labels"], json!(["A"]), "编辑必须真的落库");
    assert_eq!(report.applied_count, 1);
    assert_eq!(report.packets[0]["insufficientContext"], json!(1));

    // ④ 调用记录必须能**对账**：逐包记下包 id、升级级别、带了哪些页、估了多少 token、
    //    请求体多少字节。没有这几个字段，「输入量下降」就只是一句感觉。
    let records: Vec<Value> = std::fs::read_to_string(
        crate::util::job_dir(&root, ITEM_ID).join("llm-calls.jsonl"),
    )
    .expect("网关必须留下 llm-calls.jsonl")
    .lines()
    .filter_map(|line| serde_json::from_str(line).ok())
    .collect();
    let packet_records: Vec<&Value> = records
        .iter()
        .filter(|record| record["commandName"] == json!("repair_authoring_step"))
        .collect();
    assert!(
        packet_records.len() >= 2,
        "至少两轮修复调用要落记录：{records:#?}"
    );
    for record in &packet_records {
        assert!(
            record["packetId"].as_str().is_some_and(|id| id.starts_with("pkt-")),
            "每条修复记录都要写明是哪个包：{record:#?}"
        );
        assert!(record["escalationLevel"].as_u64().is_some(), "{record:#?}");
        assert!(
            record["estimatedInputTokens"].as_u64().unwrap_or(0) > 0,
            "包自己估的 token 必须记下来：{record:#?}"
        );
        assert!(
            record["requestBytes"].as_u64().unwrap_or(0) > 0,
            "请求体字节数必须记下来：{record:#?}"
        );
        assert!(record["imageCount"].as_u64().is_some(), "{record:#?}");
    }
    // 第一轮里没有答案页，第二轮里有——这正是「回退真的在传内容」的可对账版本。
    let first_pages = packet_records[0]["pagesIncluded"].as_array().cloned().unwrap_or_default();
    let second_pages = packet_records[1]["pagesIncluded"].as_array().cloned().unwrap_or_default();
    assert!(!first_pages.contains(&json!(3)), "第一轮不该包含答案页：{first_pages:?}");
    assert!(second_pages.contains(&json!(3)), "第二轮必须包含取回的答案页：{second_pages:?}");
    // 记录的是**那一轮请求**的级别：第一轮模型还没报「不够」，所以是 L0；报过之后
    // 第二轮就是 L1。升级在记录里看得见，正是「这一次输入量下降是不是靠模型自己补的」
    // 这个问题的答案。
    assert_eq!(packet_records[0]["escalationLevel"], json!(0), "{:#?}", packet_records[0]);
    assert_eq!(packet_records[1]["escalationLevel"], json!(1), "{:#?}", packet_records[1]);

    let _ = std::fs::remove_dir_all(&root);
}

/// PATH 里找一个 `node` 可执行文件；找不到返回 `None`。
///
/// **调用方必须把 `None` 当成失败，不能当成跳过。** 这些用例证明的是「仓库里那个受控
/// 服务在包模式下能自己把缺的页要回来 / 能裁定」——没跑成 node 就等于没证。以前这里
/// `eprintln!` 一句就 `return`，而 `cargo test` 默认捕获通过用例的 stderr，于是在没有
/// node 的机器上这是一条**恒绿但什么都没证**的用例（P7 审计 #2 的 P2）。
///
/// 与仓库里 pdfium 用例的差别是有意的：pdfium 是**可选**渲染器，node 是这个仓库的
/// **必需**工具链（vitest、全部 `scripts/e2e/*.mjs`、`npm run check` 都靠它）。
/// 缺 node 意味着工具链坏了，静默跳过会把这件事藏起来。
fn node_binary() -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        for name in ["node", "node.exe"] {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// 往受控服务发一个 JSON POST 并读回响应体（只够这条用例用，不引入额外依赖）。
///
/// 走 `Connection: close`：服务端自己带 `content-length`，读到 EOF 即完整响应体。
fn post_json(port: u16, path: &str, body: &Value) -> Value {
    use std::io::{Read, Write};
    let payload = serde_json::to_string(body).expect("请求体必须可序列化");
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("连接受控服务");
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(20)))
        .expect("设置读超时");
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    stream.write_all(request.as_bytes()).expect("写请求");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).expect("读响应");
    let text = String::from_utf8_lossy(&raw);
    let at = text.find("\r\n\r\n").expect("HTTP 响应必须有头体分隔");
    serde_json::from_str(text[at + 4..].trim()).expect("响应体必须是 JSON")
}

/// 一个当前空闲的本地端口。
///
/// 先绑 0 让内核挑，再立刻放开给受控服务去绑。中间有一个很短的窗口；单机测试里
/// 可以接受，因为失败的表现是「服务起不来 → 健康检查超时 → 用例报错」，不会静默。
fn free_local_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind 临时端口");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    port
}

/// 受控服务子进程的看门狗：无论用例怎么退出（包括断言失败 panic）都要收掉它，
/// 否则一个还占着端口的 node 进程会留在机器上。
struct ChildGuard(std::process::Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// §7.2：**真实网关代码 + 真实 HTTP + 仓库里那个受控服务**，驱动包模式循环走完
/// L0 → L1 → 编辑 → 收工。
///
/// 与 [`packets_mode_requests_carry_no_whole_pdf_and_the_fetched_page_reaches_the_model`]
/// 的区别只在 HTTP 对端：那一条是测试内的 TCP stub（够用来量输入量与断言请求体形状），
/// 这一条起的是仓库里真正交付、CDP 链在 Windows 上用的
/// `scripts/controlled-llm-service.mjs`。任务书 §7 点名的就是后者 —— 因为「受控服务在
/// 包模式下能自己把缺的页要回来」这件事，只有让**它**真的跑一遍才算证过：A-4 补的
/// 题面类 / 答案类分支此前只有一份未提交的临时脚本验证过，CDP 链又只跑在 Windows。
///
/// 剧本里**不给正确答案**，只给「改哪个槽、答案在哪一页」：答案与引文都只能由受控
/// 服务从包里真实出现的行读出来。
#[test]
fn the_real_controlled_service_drives_the_packet_loop_through_l0_l1_and_finish() {
    let Some(node) = node_binary() else {
        // 不许静默跳过：这条用例是 §7.2 的验收证据，跳过却报绿等于假绿。
        panic!("本机 PATH 里没有 node：§7.2 的真实受控服务用例无法执行——这不是通过（见 node_binary 的说明）");
    };
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri 必须有父目录")
        .join("scripts/controlled-llm-service.mjs");
    assert!(script.is_file(), "受控服务脚本必须在仓库里：{script:?}");

    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");
    seed_packet_job(&root);

    let plan_path = root.join("repair-plan.json");
    let request_log_path = root.join("controlled-llm-requests.jsonl");
    crate::util::write_json(
        &plan_path,
        &json!({
            "fixSlotIds": ["q14"],
            "questionNumber": 14,
            "sourcePageOneBased": 3,
            "rulings": [],
            "unresolved": [],
            "finishNote": "受控服务：q14 已按原文件改为 A"
        }),
    )
    .expect("写剧本");

    let port = free_local_port();
    let child = std::process::Command::new(&node)
        .arg(&script)
        .arg("--port")
        .arg(port.to_string())
        .arg("--plan")
        .arg(&plan_path)
        .arg("--request-log")
        .arg(&request_log_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap_or_else(|error| panic!("起受控服务失败 node={node:?}: {error}"));
    let _guard = ChildGuard(child);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut ready = false;
    while std::time::Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(ready, "受控服务 15 秒内没有起来（端口 {port}）");

    crate::llm_profiles::save_profiles(
        &root,
        &[json!({
            "profileId": "controlled-repair",
            "name": "Controlled Repair Service",
            "provider": "OpenAiCompatible",
            "baseUrl": format!("http://127.0.0.1:{port}/v1"),
            "model": "controlled-repair-v1",
            "temperature": 0,
            "timeoutMs": 60000,
            "forceJson": true,
            "enabled": true
        })],
    )
    .expect("profile 必须能落盘");

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    let report = run_packets(&request, |context: &Value, observations: &[Value]| {
        repair_authoring_step_through_gateway(
            &root,
            ITEM_ID,
            Some("controlled-repair"),
            context,
            observations,
        )
    })
    .expect("包模式循环必须跑完（受控服务真的被驱动过）");

    let captured_requests = std::fs::read_to_string(&request_log_path)
        .expect("真实受控服务必须记录它实际收到的 HTTP 请求体");
    let request_bodies: Vec<&str> = captured_requests.lines().collect();
    assert!(
        request_bodies.len() >= 3,
        "应记录 L0 / L1 / finish_packet 请求：{request_bodies:#?}"
    );
    assert!(
        request_bodies
            .iter()
            .all(|body| !body.contains("data:application/pdf;base64,")),
        "包模式的真实受控服务请求不得携带整份 PDF"
    );

    // ① 编辑真的落库，且值是**从包里那一行**读出来的（剧本里没有 "A"）。
    assert_eq!(
        read_answer(&root, "q14")["labels"],
        json!(["A"]),
        "编辑必须真的落库"
    );
    assert_eq!(report.applied_count, 1);
    assert!(
        report
            .packets
            .iter()
            .any(|packet| packet["status"] == json!("finished")),
        "真实受控服务必须在编辑后调用 finish_packet 并结束一个包：{:#?}",
        report.packets
    );
    let edited_packet = report
        .packets
        .iter()
        .position(|packet| packet["edits"].as_u64().unwrap_or(0) > 0)
        .expect("逐包诊断必须记录实际编辑的包");
    let finished_packet = report
        .packets
        .iter()
        .position(|packet| packet["status"] == json!("finished"))
        .expect("逐包诊断必须记录 finish_packet");
    assert!(
        finished_packet > edited_packet,
        "finish_packet 必须发生在编辑包之后：{:#?}",
        report.packets
    );
    // ② 「不够就说」这条出口真的被走过：L1 在逐包诊断里看得见。
    assert_eq!(
        report.packets[0]["insufficientContext"],
        json!(1),
        "受控服务必须真的报过一次「不够」：{:#?}",
        report.packets
    );
    assert!(
        report.packets[0]["escalationLevel"].as_u64().unwrap_or(0) >= 1,
        "走过 report_insufficient_context 之后级别必须抬到 L1：{:#?}",
        report.packets[0]
    );

    // ③ 逐包记录对账：第一轮 L0 且没有答案页；第二轮 L1 且带着取回的答案页。
    let records: Vec<Value> = std::fs::read_to_string(
        crate::util::job_dir(&root, ITEM_ID).join("llm-calls.jsonl"),
    )
    .expect("网关必须留下 llm-calls.jsonl")
    .lines()
    .filter_map(|line| serde_json::from_str(line).ok())
    .filter(|record: &Value| record["commandName"] == json!("repair_authoring_step"))
    .collect();
    assert!(records.len() >= 2, "至少两轮修复调用要落记录：{records:#?}");
    assert_eq!(records[0]["escalationLevel"], json!(0), "{:#?}", records[0]);
    assert_eq!(records[1]["escalationLevel"], json!(1), "{:#?}", records[1]);
    let first_pages = records[0]["pagesIncluded"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let second_pages = records[1]["pagesIncluded"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(!first_pages.contains(&json!(3)), "第一轮不该包含答案页：{first_pages:?}");
    assert!(second_pages.contains(&json!(3)), "第二轮必须包含取回的答案页：{second_pages:?}");

    let _ = std::fs::remove_dir_all(&root);
}

/// 包模式裁定用例共用的原文定义句：`evidenceKeyword`（`NOT GIVEN`）落在这一行里。
const RULING_DEFINITION: &str = "Do the following statements agree with the claims of the writer? \
     Write YES if the statement agrees with the claims of the writer, \
     NO if the statement contradicts the claims of the writer, \
     NOT GIVEN if it is impossible to say what the writer thinks about this.";

/// 造一个「承载裁定型差异的包」的输入信封（`context.contextMode == "packets"`）。
///
/// 包里只有一条差异，类型是 `task_group` + `instructions`：这正是
/// `cloud-repair-scenario.mjs` 里那条「当前稿对、候选错」的差异形状——**不能**靠
/// `apply_edits` 消掉，只能靠 `record_ruling` 了结。
fn ruling_packet_input(observations: Value) -> Value {
    json!({
        "mode": "repair_authoring_step",
        "context": {
            "contextMode": "packets",
            "packetId": "pkt-rule-1",
            "scope": {"pages": [4]},
            "differences": [{
                "targetType": "task_group",
                "targetId": "demanding-q27-40",
                "field": "instructions",
                "canonical": RULING_DEFINITION,
                "candidate": "Do the following statements agree with the claims of the writer?"
            }],
            "draftSlice": {"editVersion": 7, "taskGroups": []},
            "sourceEvidence": {
                "sourceFileId": "demanding-reading-pdf",
                "pages": [{
                    "pageIndex": 4,
                    "lines": [
                        {"id": "p4:l1", "text": "Questions 27-40"},
                        {"id": "p4:l2", "text": RULING_DEFINITION}
                    ]
                }]
            }
        },
        "observations": observations
    })
}

/// 一条真实的 `record_ruling` observation（形状与 `CloudRepairToolResultV1::ok` 一致）。
fn recorded_ruling_observation(target_type: &str, target_id: &str, field: &str) -> Value {
    json!({
        "schemaVersion": "CloudRepairToolResultV1",
        "callId": "p1",
        "status": "ok",
        "result": {
            "status": "recorded",
            "recorded": [{
                "targetType": target_type,
                "targetId": target_id,
                "field": field,
                "ruling": "current_is_correct",
                "reason": "原文件里这段说明包含完整的 YES / NO / NOT GIVEN 定义。",
                "evidence": [{"sourceFileId": "demanding-reading-pdf", "pageIndex": 4, "quote": "NOT GIVEN"}]
            }],
            "errors": []
        },
        "errors": []
    })
}

/// 把输入信封发成受控服务认得的请求体，并取回它给的**工具调用**。
///
/// 请求体形状与网关一致：`repair_step_prompt` 的首句用于 `detectTask` 分流，
/// `Input JSON: ` 之后是整份输入信封（`repairInput` 按最后一个标记切）。
fn ask_controlled_service(port: u16, input: &Value) -> Value {
    let body = json!({
        "model": "controlled-repair-v1",
        "messages": [{
            "role": "user",
            "content": [{
                "type": "text",
                "text": format!(
                    "You are repairing an IELTS Reading authoring draft so it matches the ORIGINAL FILE.\n\
                     Input JSON: {input}"
                )
            }]
        }]
    });
    let reply = post_json(port, "/v1/chat/completions", &body);
    let content = reply["choices"][0]["message"]["content"]
        .as_str()
        .expect("受控服务必须回 content 字符串");
    serde_json::from_str(content).expect("content 必须是一段工具调用 JSON")
}

/// 起一个受控服务子进程并等它就绪（返回看门狗，用例结束自动收掉）。
fn start_controlled_service(node: &std::path::Path, plan_path: &std::path::Path) -> (u16, ChildGuard) {
    let port = free_local_port();
    let child = std::process::Command::new(node)
        .arg(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("src-tauri 必须有父目录")
                .join("scripts/controlled-llm-service.mjs"),
        )
        .arg("--port")
        .arg(port.to_string())
        .arg("--plan")
        .arg(plan_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap_or_else(|error| panic!("起受控服务失败 node={node:?}: {error}"));
    let guard = ChildGuard(child);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return (port, guard);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("受控服务 15 秒内没有起来（端口 {port}）");
}

/// 包模式剧本必须能裁定「当前稿对、候选错」的差异（P7 审计 #2 的 P1）。
///
/// CDP 链默认就跑在包模式下（`REPAIR_CONTEXT_MODE` 默认 `Packets`，而全仓库没有一处设
/// `IELTS_REPAIR_CONTEXT_MODE`），它的 `cloud-fixed-content-on-its-own` 要求
/// `adjudicatedCount >= 1`。这个数字只数**裁定**（`effective_adjudicated_count`）——
/// 被编辑改掉的差异进的是 `appliedCount`，两者刻意不重叠。包模式剧本此前只会
/// `report_insufficient_context` / `apply_edits` / `finish_packet`，**从不**
/// `record_ruling`，于是那条断言在包模式下恒红。
///
/// 这里把一个「承载裁定型差异的包」直接喂给仓库里那个真实脚本（真 HTTP），断言：
///   ① 它回的是 `record_ruling`，不是直接 `finish_packet` 收工；
///   ② 裁定指向的正是包里列出的那条差异；
///   ③ 引文逐字来自**包内原文行**，不是脚本里的常量。
///
/// 注意裁定**不会**让差异从 `context.differences` 里消失（`build_repair_context` 给的是
/// 原始 `candidate_differences`），所以剧本必须靠 `observations` 里的 `record_ruling`
/// 结果去重，否则会原地打转到轮数用尽。这条用例只覆盖「第一次该裁定」这一半；
/// 去重那一半由 [`the_controlled_service_stops_ruling_once_it_already_has`] 覆盖。
#[test]
fn the_real_controlled_service_rules_on_a_ruling_type_difference_in_packet_mode() {
    let Some(node) = node_binary() else {
        panic!("本机 PATH 里没有 node：包模式裁定用例无法执行——这不是通过（见 node_binary 的说明）");
    };
    let root = temp_root();
    std::fs::create_dir_all(&root).expect("临时目录");
    let plan_path = root.join("repair-plan.json");
    crate::util::write_json(
        &plan_path,
        &json!({
            "fixSlotIds": [],
            "questionNumber": 40,
            "sourcePageOneBased": 4,
            "rulings": [{
                "targetType": "task_group",
                "targetId": "demanding-q27-40",
                "field": "instructions",
                "ruling": "current_is_correct",
                "reason": "原文件里这段说明包含完整的 YES / NO / NOT GIVEN 定义，当前稿与之一致。",
                "evidenceKeyword": "NOT GIVEN"
            }],
            "unresolved": [],
            "finishNote": "受控服务：裁定完成"
        }),
    )
    .expect("写剧本");

    let (port, _guard) = start_controlled_service(&node, &plan_path);
    let call = ask_controlled_service(port, &ruling_packet_input(json!([])));

    assert_eq!(
        call["tool"], json!("record_ruling"),
        "包模式剧本必须能裁定「当前稿对、候选错」的差异，否则 CDP 链的 adjudicatedCount 恒为 0：{call:#?}"
    );
    let ruling = &call["arguments"]["rulings"][0];
    assert_eq!(ruling["targetType"], json!("task_group"));
    assert_eq!(ruling["targetId"], json!("demanding-q27-40"));
    assert_eq!(ruling["field"], json!("instructions"));
    assert_eq!(ruling["ruling"], json!("current_is_correct"));
    let quote = ruling["evidence"][0]["quote"].as_str().unwrap_or_default();
    assert!(
        RULING_DEFINITION.contains(quote) && quote.contains("NOT GIVEN"),
        "引文必须逐字取自包内原文行（不是脚本里的常量）：{quote:?}"
    );
    assert_eq!(
        ruling["evidence"][0]["pageIndex"],
        json!(4),
        "引文要落在它真实所在的那一页：{ruling:#?}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// 已经裁定过之后，剧本必须收工——否则每个包会一直裁到轮数用尽。
///
/// 裁定**不会**把差异从 `context.differences` 里拿掉：`build_repair_context` 给的是原始
/// `candidate_differences`，而 `effective_adjudicated_count` 的设计恰恰依赖「差异还在、
/// 裁定也还在」才算数。所以剧本只能靠 `observations` 里 `record_ruling` 的真实返回去重。
/// 少了这一步，一个只承载裁定型差异的包会每轮重复同一次调用，被 `REPEAT_LIMIT` 判成
/// `no_progress`，整个 run 报成 `budget_exhausted`。
#[test]
fn the_controlled_service_stops_ruling_once_it_already_has() {
    let Some(node) = node_binary() else {
        panic!("本机 PATH 里没有 node：包模式裁定用例无法执行——这不是通过（见 node_binary 的说明）");
    };
    let root = temp_root();
    std::fs::create_dir_all(&root).expect("临时目录");
    let plan_path = root.join("repair-plan.json");
    crate::util::write_json(
        &plan_path,
        &json!({
            "fixSlotIds": [],
            "questionNumber": 40,
            "sourcePageOneBased": 4,
            "rulings": [{
                "targetType": "task_group",
                "targetId": "demanding-q27-40",
                "field": "instructions",
                "ruling": "current_is_correct",
                "reason": "原文件里这段说明包含完整的 YES / NO / NOT GIVEN 定义，当前稿与之一致。",
                "evidenceKeyword": "NOT GIVEN"
            }],
            "unresolved": [],
            "finishNote": "受控服务：裁定完成"
        }),
    )
    .expect("写剧本");

    let (port, _guard) = start_controlled_service(&node, &plan_path);
    let observations = json!([recorded_ruling_observation(
        "task_group",
        "demanding-q27-40",
        "instructions"
    )]);
    let call = ask_controlled_service(port, &ruling_packet_input(observations));

    assert_ne!(
        call["tool"], json!("record_ruling"),
        "这条差异已经裁定过了，不许再裁一次（会原地打转到轮数用尽）：{call:#?}"
    );
    assert_eq!(
        call["tool"], json!("finish_packet"),
        "裁定过的包应当收工：{call:#?}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// 给 `seed_packet_job` 的作业补一份**视觉缓存**（页图 + `pdf-images.json`）。
///
/// `grab::load_source_index` 只认这一个产物；没有它 `source.page_images` 是空的，
/// 「区域图到底附没附上」就无从断言 —— A-15 / A-16 的覆盖缺口正在这里。
fn seed_page_images(root: &Path, pages: &[u32]) {
    let directory = crate::util::job_dir(root, ITEM_ID)
        .join("cache")
        .join("vision");
    std::fs::create_dir_all(&directory).expect("vision 目录");
    let mut entries = Vec::new();
    for page in pages {
        let path = directory.join(format!("page-{page}.png"));
        let file = std::fs::File::create(&path).expect("页图文件");
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), 595, 842);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("PNG 头");
        writer
            .write_image_data(&vec![255u8; 595 * 842 * 3])
            .expect("PNG 数据");
        writer.finish().expect("PNG 收尾");
        entries.push(json!({
            "pageIndex": page,
            "width": 595.0,
            "height": 842.0,
            "images": [{"path": path.to_string_lossy(), "mimeType": "image/png"}]
        }));
    }
    crate::util::write_json(&directory.join("pdf-images.json"), &json!({"pages": entries}))
        .expect("视觉缓存");
}

/// 从捕获到的原始 HTTP 请求里取出「模型真正看到的输入 JSON」。
///
/// 对原始文本直接 `contains("\"regions\":")` 是**转义盲**的：prompt 里的 JSON 在请求体里
/// 是 `\"regions\":`，而 prompt 的说明文字里又原样出现了 `regions[]` 这个词。两者都会
/// 让「包里到底有没有区域图」这类断言变成恒真——A-14 那条就是同一个坑。
fn request_input_json(raw: &str) -> Value {
    let start = raw.find("{\"max_tokens").expect("请求体必须是 JSON");
    let body: Value = serde_json::from_str(&raw[start..]).expect("请求体必须是合法 JSON");
    let text = body["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|message| {
            message["content"].as_array().and_then(|parts| {
                parts.iter().find_map(|part| {
                    part["text"]
                        .as_str()
                        .filter(|text| text.contains("Input JSON: "))
                })
            })
        })
        .expect("prompt 文本块必须存在");
    let marker = "Input JSON: ";
    let index = text.rfind(marker).expect("prompt 必须带输入 JSON") + marker.len();
    serde_json::from_str(&text[index..]).expect("输入 JSON 必须合法")
}

/// 包模式真的会把区域图附上，并且**本机绝对路径绝不进 prompt**
/// （审计发现 A-15 / A-16）。
///
/// 两件事都只在网关层看得见：① `sourceEvidence.regions` 非空且带图（§5.5 要求
/// 「只有范围内页文本与**区域图**」）；② `strip_packet_image_paths` 把路径换成
/// `imageAttached` —— 路径进 prompt 既无用又泄露目录结构。这条实现点原来零测试引用。
#[test]
fn packets_mode_attaches_the_region_image_and_keeps_local_paths_out_of_the_prompt() {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");
    seed_packet_job(&root);
    seed_page_images(&root, &[1, 2, 3]);

    let (base_url, requests) = spawn_scripted_repair_service_with(scripted_packet_reply);
    crate::llm_profiles::save_profiles(
        &root,
        &[json!({
            "profileId": "controlled-repair",
            "name": "Controlled Repair Service",
            "provider": "OpenAiCompatible",
            "baseUrl": base_url,
            "model": "controlled-repair-v1",
            "temperature": 0,
            "timeoutMs": 60000,
            "forceJson": true,
            "enabled": true
        })],
    )
    .expect("profile 必须能落盘");

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    run_packets(&request, |context: &Value, observations: &[Value]| {
        repair_authoring_step_through_gateway(
            &root,
            ITEM_ID,
            Some("controlled-repair"),
            context,
            observations,
        )
    })
    .expect("包模式循环必须跑完");

    let seen = requests.lock().expect("requests");
    let input = request_input_json(&seen[0]);
    let regions = input
        .pointer("/context/sourceEvidence/regions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        !regions.is_empty(),
        "§5.5 要求包里有范围内的区域图，实际一条都没有：{input:#?}"
    );
    assert!(
        regions
            .iter()
            .any(|region| region["imageAttached"] == json!(true)),
        "区域图必须真的附上（`imageAttached`）：{regions:#?}"
    );
    // 附图本身必须在请求里（图片部分），不能只写一句「已附」。
    assert!(
        seen[0].contains("image_url"),
        "区域图必须作为图片部分附上，而不是只在文本里声称"
    );
    let job_dir = crate::util::job_dir(&root, ITEM_ID)
        .to_string_lossy()
        .to_string();
    assert!(
        !seen[0].contains(&job_dir),
        "本机绝对路径不得进 prompt（既无用又泄露目录结构）：{}",
        &seen[0][..seen[0].len().min(600)]
    );
    drop(seen);

    let records: Vec<Value> = std::fs::read_to_string(
        crate::util::job_dir(&root, ITEM_ID).join("llm-calls.jsonl"),
    )
    .expect("网关必须留下 llm-calls.jsonl")
    .lines()
    .filter_map(|line| serde_json::from_str(line).ok())
    .collect();
    let first = records
        .iter()
        .find(|record| record["commandName"] == json!("repair_authoring_step"))
        .expect("至少一条修复调用记录");
    assert!(
        first["imageCount"].as_u64().unwrap_or(0) >= 1,
        "调用记录必须数得出附图张数：{first:#?}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// 一个题组在权威稿里覆盖的页（`sourceAnchors[].pageIndex` 是 0-based，转成 1-based）。
fn anchor_pages_of_group(canonical: &Value, task_id: &str) -> Vec<u64> {
    fn walk(value: &Value, out: &mut Vec<u64>) {
        match value {
            Value::Array(items) => items.iter().for_each(|item| walk(item, out)),
            Value::Object(map) => {
                if let Some(anchors) = map.get("sourceAnchors").and_then(Value::as_array) {
                    for anchor in anchors {
                        if let Some(page) = anchor.get("pageIndex").and_then(Value::as_i64) {
                            if page >= 0 {
                                out.push(page as u64 + 1);
                            }
                        }
                    }
                }
                map.values().for_each(|child| walk(child, out));
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for group in canonical["taskGroups"].as_array().into_iter().flatten() {
        if group["taskId"].as_str() != Some(task_id) {
            continue;
        }
        walk(group, &mut out);
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// §5 第 1 条：**包模式**下用真实 `complex-reading` 的 canonical + 构造候选切包 ——
/// 包数与每包题组符合规则 2，`scope.pages` 与锚点一致，包内没有范围外的页。
///
/// 为什么必须补这一条：本分支原有的包模式用例全部走 `seed_packet_job` 手写的 3 页
/// `document-ir.json`，而唯一用 `complex-reading` 的用例是 **legacy** 用例。
/// 「真实多题组长文下的切分规则」在包模式里一次都没被验过。
#[test]
fn complex_reading_splits_into_packets_that_obey_the_grouping_rules() {
    use crate::library::migration::ensure_initial_canonical;

    let root = temp_root();
    crate::util::ensure_app_dirs(&root).expect("app dirs");
    seed_docx_job_with_source(&root);
    crate::auto_pipeline::run_auto_pipeline_core(
        &root,
        ITEM_ID,
        Some(crate::AutoPipelineInput {
            execution_mode: Some("localOnly".to_string()),
            target: Some("editableDraft".to_string()),
            allow_overwrite: Some(true),
            ..Default::default()
        }),
    )
    .expect("真实 DOCX 导入必须完成");
    ensure_initial_canonical(&root, ITEM_ID).expect("seed canonical");
    let conn = open_library_connection(&root).expect("库连接");
    let (canonical, version) = get_canonical_ds(&conn, ITEM_ID)
        .expect("读 canonical")
        .expect("已播");
    drop(conn);

    let group_ids: Vec<String> = canonical["taskGroups"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|group| group["taskId"].as_str().map(str::to_string))
        .collect();
    assert!(
        group_ids.len() >= 2,
        "complex-reading 必须有多题组，否则这条用例测不到切分：{group_ids:?}"
    );

    // 构造候选：每个题组的**第一条答案**改一个值 ⇒ 每个题组各一条答案差异。
    // 刻意改答案而不是改正文：slot → 题组的归属是切分规则 1 的正路，也让规则 2 的
    // 「不共享选项库/刺激 ⇒ 各自成包」可判定。
    let mut candidate = canonical.clone();
    let mut expected: Vec<String> = Vec::new();
    for group in canonical["taskGroups"].as_array().into_iter().flatten() {
        let Some(slot) = group["responseGroups"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|response| response["slotIds"].as_array().into_iter().flatten())
            .filter_map(Value::as_str)
            .next()
        else {
            continue;
        };
        let Some(values) = candidate["answerKey"][slot]["values"].as_array_mut() else {
            continue;
        };
        let first = values.first_mut().expect("答案必须有值");
        let text = first.as_str().unwrap_or_default().to_string();
        *first = json!(format!("{text} CHANGED"));
        expected.push(slot.to_string());
    }
    assert_eq!(
        expected.len(),
        group_ids.len(),
        "每个题组都应被构造出一条差异：{expected:?}"
    );

    let differences = super::candidate_differences(&canonical, &candidate);
    assert_eq!(
        differences.len(),
        group_ids.len(),
        "每个题组各应产出一条差异：{differences:#?}"
    );

    let source = super::grab::load_source_index(&root, ITEM_ID, "complex-reading-docx", "docx");
    let planned = super::packets::plan_packets(&super::packets::PacketPlanInput {
        canonical: &canonical,
        candidate: &candidate,
        differences: &differences,
        blocking_issues: &[],
        protected: &BTreeSet::new(),
        source: &source,
        edit_version: version,
    });

    // 规则 2（负向）：两个题组既不共享选项库也不共享 stimulus ⇒ 各自成包，不许并。
    assert_eq!(
        planned.len(),
        group_ids.len(),
        "不共享选项库/刺激的题组必须各自成包：{planned:#?}"
    );
    let mut seen_groups: Vec<String> = Vec::new();
    for packet in &planned {
        let task_ids: Vec<String> = packet["taskIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        assert_eq!(task_ids.len(), 1, "每包只该带一个题组：{packet:#?}");
        seen_groups.extend(task_ids.iter().cloned());

        // `scope.pages` 与锚点一致：题组的每一个锚点页都必须在范围内。
        let anchors = anchor_pages_of_group(&canonical, &task_ids[0]);
        let scope: Vec<u64> = packet["scope"]["pages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .collect();
        assert!(!scope.is_empty(), "scope.pages 不能为空：{packet:#?}");
        for page in &anchors {
            assert!(
                scope.contains(page),
                "题组 {task_ids:?} 的锚点页 {page} 不在 scope.pages({scope:?}) 里：{packet:#?}"
            );
        }
        // 包内没有范围外的页。
        let included: Vec<u64> = packet["sourceEvidence"]["pages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|page| page["pageIndex"].as_u64())
            .collect();
        assert!(!included.is_empty(), "包里必须有范围内页文本：{packet:#?}");
        for page in &included {
            assert!(
                scope.contains(page),
                "包里有范围外的页 {page}：scope={scope:?} included={included:?}"
            );
        }
    }
    seen_groups.sort();
    let mut wanted = group_ids.clone();
    wanted.sort();
    assert_eq!(seen_groups, wanted, "所有题组都必须被切进某个包");

    let _ = std::fs::remove_dir_all(&root);
}

/// 与 [`seed_packet_job`] 同一份作业，但 `uploads/` 里放的是**真实多页 PDF**。
///
/// 两模式对比必须有真实附件才成立：`seed_packet_job` 写的是 8 字节的 `%PDF-1.4\n`，
/// 拿它比「附整份 PDF 贵多少」等于什么都没比。这里换用仓库里真实存在的 212 KB 样本。
fn seed_packet_job_with_real_pdf(root: &Path) {
    seed_packet_job(root);
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/parser/demanding-reading-passage-3.pdf");
    let bytes = std::fs::read(&source).expect("真实 PDF 样本必须在仓库里");
    std::fs::write(
        crate::util::job_dir(root, ITEM_ID)
            .join("uploads")
            .join("early-approaches.pdf"),
        bytes,
    )
    .expect("把真实 PDF 放进 uploads");
}

/// 同一份 fixture、同一条**真实 HTTP 网关**，分别跑 legacy 与 packets，把每一次修复
/// 请求的 `requestBytes` 与包自己估的 `estimatedInputTokens` 加起来。
///
/// 这是任务书 §6 要求的「两模式对比」，也是「单次校核总输入量显著下降」这句话唯一
/// 可对账的版本：两个数字都取自 `llm-calls.jsonl` 的真实记录，不是脚本自己算的。
fn repair_input_totals(mode: RepairContextMode, script: fn(&str, usize) -> String) -> (u64, u64) {
    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");
    seed_packet_job_with_real_pdf(&root);

    let (base_url, _requests) = spawn_scripted_repair_service_with(script);
    crate::llm_profiles::save_profiles(
        &root,
        &[json!({
            "profileId": "controlled-repair",
            "name": "Controlled Repair Service",
            "provider": "OpenAiCompatible",
            "baseUrl": base_url,
            "model": "controlled-repair-v1",
            "temperature": 0,
            "timeoutMs": 60000,
            "forceJson": true,
            "enabled": true
        })],
    )
    .expect("profile 必须能落盘");

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    let step = |context: &Value, observations: &[Value]| {
        repair_authoring_step_through_gateway(
            &root,
            ITEM_ID,
            Some("controlled-repair"),
            context,
            observations,
        )
    };
    match mode {
        RepairContextMode::Legacy => {
            run_legacy(&request, step).expect("legacy 循环必须跑完");
        }
        RepairContextMode::Packets => {
            run_packets(&request, step).expect("包模式循环必须跑完");
        }
    }

    let records: Vec<Value> = std::fs::read_to_string(
        crate::util::job_dir(&root, ITEM_ID).join("llm-calls.jsonl"),
    )
    .expect("网关必须留下 llm-calls.jsonl")
    .lines()
    .filter_map(|line| serde_json::from_str(line).ok())
    .filter(|record: &Value| record["commandName"] == json!("repair_authoring_step"))
    .collect();
    assert!(!records.is_empty(), "两种模式都必须真的发出修复请求");
    let bytes: u64 = records
        .iter()
        .filter_map(|record| record["requestBytes"].as_u64())
        .sum();
    let tokens: u64 = records
        .iter()
        .filter_map(|record| record["estimatedInputTokens"].as_u64())
        .sum();
    let _ = std::fs::remove_dir_all(&root);
    (bytes, tokens)
}

/// §6 两模式对比：**同一份卷子**，包模式的请求体总量必须显著小于 legacy。
///
/// 判据刻意取「不到一半」而不是「小一点点」：如果只是小一点点，那说明真正的大头
/// （整份 PDF 附件）还在路上，改造就没落地。
#[test]
fn packets_mode_sends_much_less_input_than_legacy_for_the_same_paper() {
    let (legacy_bytes, legacy_tokens) =
        repair_input_totals(RepairContextMode::Legacy, scripted_repair_reply);
    let (packet_bytes, packet_tokens) =
        repair_input_totals(RepairContextMode::Packets, scripted_packet_reply);

    assert!(legacy_bytes > 0, "legacy 侧的请求体字节数必须真实记录");
    assert!(packet_bytes > 0, "包模式侧的请求体字节数必须真实记录");
    // 数字打进测试输出（默认被捕获，`--nocapture` 可见）：报告里的对比值必须是**量出来的**。
    eprintln!(
        "[repair-input] legacy requestBytes={legacy_bytes} packets requestBytes={packet_bytes} \
         packets estimatedInputTokens={packet_tokens}"
    );
    assert!(
        packet_bytes * 2 < legacy_bytes,
        "同一份卷子，包模式的总请求体必须不到 legacy 的一半：\
         legacy={legacy_bytes} packets={packet_bytes}"
    );
    // 逐包估算 token 是包模式才有的对账口径（legacy 每轮附整份 PDF，没有「包」这个概念）。
    assert!(
        packet_tokens > 0,
        "包模式必须记下每包的估算 token，否则「输入量下降」无从对账"
    );
    assert_eq!(
        legacy_tokens, 0,
        "legacy 不产生逐包估算：这个字段的有无本身就是两种模式的分界"
    );
}

/// 包超预算时：按 §4.3 的「整页图 → 区域图」丢**最少够用**的图，并且**不许**在
/// `escalationNote` 里继续声称「整页图已附」——那句 note 是 L2 写的，而预算可能
/// 紧接着就把图清掉了（审计发现 A-3：包里会同时躺着两句互相矛盾的话，模型看到的
/// 却是一张图都没有）。
#[test]
fn an_over_budget_packet_drops_the_fewest_images_and_corrects_the_escalation_note() {
    // 每张图按 1200 token 计费、字符按 4 折算：8.2 万字符 ≈ 20500 token，加 3 张图
    // 3600 ⇒ 约 24100，只超一点点 ⇒ 丢 1 张就够。
    let mut packet = json!({
        "contextMode": "packets",
        "packetId": "pkt-budget",
        "filler": "x".repeat(82_000),
        "scopeManifest": {
            "escalationNote": "L2: the backend attached whole-page images for every page in scope."
        },
        "sourceEvidence": {
            "regions": [
                {"pageIndex": 1, "bbox": Value::Null, "image": "whole-1.png"},
                {"pageIndex": 2, "bbox": {"x": 1.0}, "image": "crop-2.png"},
                {"pageIndex": 3, "bbox": {"x": 2.0}, "image": "crop-3.png"},
            ]
        }
    });
    let estimate = packet_token_estimate(&packet);
    assert!(
        estimate > super::packets::PACKET_TOKEN_BUDGET,
        "夹具必须先真的超预算，否则这条用例什么都没测：{estimate}"
    );

    apply_budget_after_escalation(&mut packet);

    let regions = packet["sourceEvidence"]["regions"]
        .as_array()
        .expect("regions 必须还在")
        .clone();
    assert_eq!(
        regions.len(),
        2,
        "只该丢最少够用的那几张，不该一超预算就全清：{regions:#?}"
    );
    assert!(
        regions.iter().all(|region| !region["bbox"].is_null()),
        "退让顺序是「整页图 → 区域图」，整页图必须先丢：{regions:#?}"
    );
    assert!(
        packet["budgetNote"].as_str().is_some_and(|note| note.contains("dropped")),
        "丢了什么必须写下来：{:#?}",
        packet["budgetNote"]
    );
    let note = packet["scopeManifest"]["escalationNote"]
        .as_str()
        .unwrap_or_default();
    assert!(
        note.contains("dropped"),
        "预算把图清掉之后，L2 的 note 必须改口，不能继续说「已附整页图」：{note}"
    );
}

/// 图全丢完了还是超预算时，`budgetNote` 必须**如实说没得再退让**（审计发现 A-9）。
///
/// 单题组的正文既不裁也不拆（`packets.rs::plan_packets` 的拆分只按题组），所以这种情况
/// 真的存在。静默超限会让「单包上限 24k」这句话变成一句没人核对的口号。
#[test]
fn a_packet_that_is_still_over_budget_says_it_cannot_concede_any_further() {
    // 12 万字符 ≈ 30000 token，加 1 张图 1200 ⇒ 31200。图丢光后仍有 30000 > 24000。
    let mut packet = json!({
        "contextMode": "packets",
        "packetId": "pkt-over-budget",
        "filler": "x".repeat(120_000),
        "scopeManifest": {},
        "sourceEvidence": {
            "regions": [
                {"pageIndex": 1, "bbox": Value::Null, "image": "whole-1.png"},
            ]
        }
    });
    let estimate = packet_token_estimate(&packet);
    assert!(
        estimate > super::packets::PACKET_TOKEN_BUDGET,
        "夹具必须先真的超预算：{estimate}"
    );

    enforce_packet_budget(&mut packet);

    assert_eq!(
        packet["sourceEvidence"]["regions"].as_array().map(Vec::len),
        Some(0),
        "图该丢光：{:#?}",
        packet["sourceEvidence"]["regions"]
    );
    let note = packet["budgetNote"].as_str().unwrap_or_default();
    assert!(
        note.contains("still not enough"),
        "丢光之后仍超预算时必须明说没得再退让，不能只写「丢了 N 张」：{note}"
    );
    assert!(
        packet_token_estimate(&packet) > super::packets::PACKET_TOKEN_BUDGET,
        "这条用例的前提就是「丢光也还超」，夹具本身不能自相矛盾"
    );
}

/// 文档包（`task_ids` 为空）里的 `read_draft` **永远**被拒（审计发现 A-11）。
///
/// 这是**有意**的：文档包没有属于它的稿件切片，`draftSlice` 本身也是空的。把它固定成
/// 测试，是为了让「文档包读不到稿」是一个决定，而不是一个没人注意的副作用。
#[test]
fn a_document_packet_never_hands_out_a_draft_slice() {
    // 走**真实切包路径**拿到文档包，而不是手工拼一个空 `taskIds` 的 `PacketTools`：
    // 手工拼的那一版只证明了「`scope_error` 在空集合上会拒绝」，而那条分支这条改动
    // 根本没碰过 —— 等于没测到新东西（P7 审计 #2）。真正要固定的是两件事**连在一起**
    // 成立：① 文档包的 `taskIds` 是空的、`draftSlice` 里没有题组；② 因此 `read_draft`
    // 在文档包里必被拒。①和②分别由不同的函数负责，只有一起断言才能防住「切包那边
    // 开始给文档包塞题组，而拒绝逻辑没跟着变」。
    let canonical = golden_authoring();
    let source = super::packets::SourcePageIndex::default();
    // `passage` 不是任何题组能认领的 target ⇒ 归到文档级差异。
    let differences = vec![json!({
        "targetType": "passage",
        "targetId": "passage-1",
        "field": "text",
        "canonical": "本地稿里的文章正文",
        "candidate": "候选里的文章正文",
        "contextDigest": "digest",
    })];
    let planned = super::packets::plan_packets(&super::packets::PacketPlanInput {
        canonical: &canonical,
        candidate: &Value::Null,
        differences: &differences,
        blocking_issues: &[],
        protected: &std::collections::BTreeSet::new(),
        source: &source,
        edit_version: 7,
    });
    assert_eq!(planned.len(), 1, "只有文档级差异时只该有一个包：{planned:#?}");
    let packet = &planned[0];
    assert_eq!(packet["documentOnly"], json!(true), "这条差异归文档级：{packet:#?}");
    assert_eq!(packet["taskIds"], json!([]), "文档包不带题组：{packet:#?}");
    assert_eq!(
        packet["draftSlice"]["taskGroups"],
        json!([]),
        "文档包没有可读的稿件切片：{packet:#?}"
    );

    let mut budget = super::grab::GrabBudget::new();
    let tools = super::PacketTools {
        source: &source,
        budget: &mut budget,
        task_ids: std::collections::BTreeSet::new(),
        question_numbers: Vec::new(),
    };
    let error = tools.scope_error(&[], &[]).expect("空选择器必须被拒");
    assert!(
        error.starts_with("CLOUD_DRAFT_SCOPE_REQUIRED"),
        "文档包里没有可读的稿件切片，必须明说而不是返回整卷：{error}"
    );
    let error = tools
        .scope_error(&["early-approaches-q14-15".to_string()], &[14])
        .expect("不在包内的题组必须被拒");
    assert!(
        error.starts_with("CLOUD_DRAFT_OUTSIDE_PACKET"),
        "文档包不含任何题组，带选择器也只能拿到「不在包内」：{error}"
    );
}

/// 升级阶梯只许写**实际发生过**的事（审计发现 A-3 / A-12）。
///
/// 两处「假陈述」都在这里：
/// ① L2 拿不到任何页图（例如这一卷没有渲染产物）时，note 仍照抄「已附整页图」；
/// ② 第二个包要 L3 时整份原文的额度已用完，代码静默跳过却把级别抬成 3，
///    读者只看到一个 `escalationLevel: 3`，以为附过了。
#[test]
fn the_escalation_ladder_only_claims_what_it_actually_attached() {
    let root = temp_root();
    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    // 没有任何页图的来源索引：`materialize_regions` 一张也裁不出来。
    let source = super::packets::SourcePageIndex {
        source_file_id: "early-approaches-pdf".to_string(),
        kind: "pdf".to_string(),
        lines: std::collections::BTreeMap::new(),
        page_images: std::collections::BTreeMap::new(),
        answer_pages: Vec::new(),
        answer_pages_known: false,
        paragraphs: Vec::new(),
    };
    let mut packet = json!({
        "packetId": "pkt-ladder",
        "scope": {"pages": [1, 2]},
        "sourceEvidence": {"regions": []},
        "scopeManifest": {}
    });
    let mut used_full_source = false;

    // L1 → L2：一张图都没附上，note 就必须说「没附上」。
    let level = escalate_packet(&request, &source, &mut packet, 1, &mut used_full_source);
    assert_eq!(level, 2);
    assert_eq!(packet["escalationLevel"], json!(2));
    let note = packet["scopeManifest"]["escalationNote"]
        .as_str()
        .unwrap_or_default();
    assert!(
        note.contains("no page image was available"),
        "L2 一张图都没附上，却仍声称「已附整页图」：{note}"
    );

    // L2 → L3：整份原文这一次用掉。
    let level = escalate_packet(&request, &source, &mut packet, 2, &mut used_full_source);
    assert_eq!(level, 3);
    assert_eq!(packet["attachFullSource"], json!(true));
    assert!(used_full_source);

    // 第二个包再要 L3：拿不到（每次运行只允许一次），但必须留下说明。
    let mut second = json!({
        "packetId": "pkt-ladder-2",
        "scope": {"pages": [3]},
        "sourceEvidence": {"regions": []},
        "scopeManifest": {}
    });
    let level = escalate_packet(&request, &source, &mut second, 2, &mut used_full_source);
    assert_eq!(level, 3, "级别仍要抬，否则阶梯不前进、到不了 L4");
    assert!(
        second.get("attachFullSource").is_none(),
        "第二个包不该拿到整份原文：{second:#?}"
    );
    let note = second["scopeManifest"]["escalationNote"]
        .as_str()
        .unwrap_or_default();
    assert!(
        note.contains("already attached"),
        "L3 被额度挡下时必须说清楚，不能只留一个「级别 3」让读者以为附过了：{note}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// L2 的 note 只许数**它这一次**附上的整页图（P7 审计 #2 的残留项）。
///
/// `attached` 原来是「包内所有带图的 region」——把**计划期就裁好的区域图**、以及模型
/// 自己取回的页图一并数了进去。于是当包里已经有一条带图的区域图、而 L2 一张整页图都
/// 附不上时（这一卷没有渲染产物），note 会写「已附 1 张整页图」：那张图既不是整页图，
/// 也不是 L2 附的。这正是 A-12 修掉的那类「把没发生的事记成发生了」，只是换了条路径
/// ——A-12 的用例只覆盖 `regions` 本来就空的那一种。
#[test]
fn the_l2_note_counts_only_the_images_it_actually_attached() {
    let root = temp_root();
    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    // 这一卷**没有**页图产物：`materialize_regions` 一张也裁不出来。
    let source = super::packets::SourcePageIndex {
        source_file_id: "early-approaches-pdf".to_string(),
        kind: "pdf".to_string(),
        lines: std::collections::BTreeMap::new(),
        page_images: std::collections::BTreeMap::new(),
        answer_pages: Vec::new(),
        answer_pages_known: false,
        paragraphs: Vec::new(),
    };
    // 计划期已经裁好的一张区域图（带图）在第 1 页；第 2 页什么都没有。
    // L2 只需要补第 2 页，而它补不上。
    let mut packet = json!({
        "packetId": "pkt-l2-count",
        "scope": {"pages": [1, 2]},
        "sourceEvidence": {"regions": [{
            "pageIndex": 1,
            "bbox": {"x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0, "origin": "top-left"},
            "taskId": "tg",
            "image": {"path": "/tmp/crop-1.png", "mimeType": "image/png"}
        }]},
        "scopeManifest": {}
    });
    let mut used_full_source = false;
    let level = escalate_packet(&request, &source, &mut packet, 1, &mut used_full_source);
    assert_eq!(level, 2);
    let note = packet["scopeManifest"]["escalationNote"]
        .as_str()
        .unwrap_or_default();
    assert!(
        note.contains("no page image was available"),
        "L2 一张整页图都没附上，note 不许把计划期那张区域图算成「已附整页图」：{note}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// L2 的 note 也不许把「有 region 条目、但那条目没有图」的页说成「已经有过图」。
///
/// 这是 P7 审计 #2 找到的 A-19 残留（也是 A-19 修复自己引入的一处**倒退**）：
/// `existing` 只按 `pageIndex` 收集，不看 `image` 是不是 `null`；而
/// `grab::materialize_regions` 在这一卷没有页图产物时会给每条 region 写
/// `"image": null`（`(None, None) => Value::Null`），`enforce_packet_budget` 又只删
/// **带图**的条目 —— null 条目原样留着。于是 `scope.pages ⊆ region 页` 时
/// `wanted == 0`，note 写的是「every page in scope already had an image in this packet」，
/// 而实际上**一张图都没有**。A-19 之前走的是诚实的「no page image was available」。
///
/// 这条路径在整个仓库里原本零用例覆盖：`grep "every page in scope already had an image"`
/// 只命中生产代码。
#[test]
fn the_l2_note_does_not_call_a_page_without_an_image_covered() {
    let root = temp_root();
    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    // 同样：这一卷没有页图产物，`materialize_regions` 补不出任何图。
    let source = super::packets::SourcePageIndex {
        source_file_id: "early-approaches-pdf".to_string(),
        kind: "pdf".to_string(),
        lines: std::collections::BTreeMap::new(),
        page_images: std::collections::BTreeMap::new(),
        answer_pages: Vec::new(),
        answer_pages_known: false,
        paragraphs: Vec::new(),
    };
    // 第 1 页**已经有一条 region 条目**（计划期生成的），但它没有图。
    // scope 只有这一页 ⇒ 旧实现的 `wanted` 会是 0。
    let mut packet = json!({
        "packetId": "pkt-l2-null-image",
        "scope": {"pages": [1]},
        "sourceEvidence": {"regions": [{
            "pageIndex": 1,
            "bbox": {"x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0, "origin": "top-left"},
            "taskId": "tg",
            "image": Value::Null,
            "note": "no page image available for this page"
        }]},
        "scopeManifest": {}
    });
    let mut used_full_source = false;
    let level = escalate_packet(&request, &source, &mut packet, 1, &mut used_full_source);
    assert_eq!(level, 2);
    let note = packet["scopeManifest"]["escalationNote"]
        .as_str()
        .unwrap_or_default();
    assert!(
        !note.contains("every page in scope already had an image"),
        "第 1 页的 region 条目没有图，就不算「已经有过图」：{note}"
    );
    assert!(
        note.contains("no page image was available"),
        "补不上图时必须如实说补不上：{note}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ── P10：答案类场景 —— 包模式真的会「自己去取」（L1 的抓取工具路径） ─────────────
//
// 与上面的 `the_real_controlled_service_drives_the_packet_loop_through_l0_l1_and_finish`
// 互补：那条证明的是 `report_insufficient_context` 路径（模型说「不够」，后端替它取）。
// 这条证明的是**抓取工具**路径：模型用 `read_source` 主动把答案页取回来，然后才改。
// 两者合起来才覆盖「必须经过 report_insufficient_context **或** 抓取工具取到之后才能改对」。
//
// 场景前提（与 CDP 答案类场景同一形状）：承载 q14 的题组锚点页是第 1 页，正确答案
// 印在第 3 页的答案区（「14 A」）——**不在锚点页上**，所以第一轮的包里没有它；
// 本地稿的 B 是真实存在的错误值（candidate 是 A，原文件是 A）。
//
// 反自证与既有纪律一致：剧本（plan）里没有答案值；受控服务在第一轮请求里**看不到**
// 答案行，它只能先抓页——第一轮请求体里没有「14 A」是这条用例的硬前提。
#[test]
fn an_answer_difference_fetches_the_answer_page_through_read_source_before_fixing() {
    let Some(node) = node_binary() else {
        panic!("本机 PATH 里没有 node：P10 的真实受控服务用例无法执行——这不是通过（见 node_binary 的说明）");
    };
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri 必须有父目录")
        .join("scripts/controlled-llm-service.mjs");
    assert!(script.is_file(), "受控服务脚本必须在仓库里：{script:?}");

    let root = temp_root();
    let canonical = golden_authoring();
    seed_item(&root, &canonical);
    store_candidate(&root, "A");
    seed_packet_job(&root);

    // 前提自检：本地稿的 q14 是 B（错误值），答案页（第 3 页）在文本层里确实印着「14 A」，
    // 而且这一页不在题组锚点页上（否则包第一轮就带着它，L1 无从发生）。
    assert_eq!(read_answer(&root, "q14")["labels"], json!(["B"]), "前提：本地答案必须是错误的 B");
    let text_layer = crate::util::read_json_opt(
        &crate::util::job_dir(&root, ITEM_ID).join("document-ir.json"),
    )
    .expect("读 document-ir")
    .expect("document-ir 必须存在");
    let answer_page_lines: Vec<String> = text_layer["pages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|page| page["pageIndex"] == json!(2))
        .flat_map(|page| page["lines"].as_array().unwrap().iter())
        .filter_map(|line| line["text"].as_str().map(str::to_string))
        .collect();
    assert!(
        answer_page_lines.iter().any(|line| line.trim() == "14 A"),
        "前提：答案页的文本层里必须有「14 A」这一行：{answer_page_lines:?}"
    );

    let plan_path = root.join("repair-plan-answer-fetch.json");
    let request_log_path = root.join("controlled-llm-requests.jsonl");
    crate::util::write_json(
        &plan_path,
        &json!({
            // 剧本里**没有**答案值：只有「改哪个槽、答案印在哪一页」和「用抓取工具去取」。
            "fixSlotIds": ["q14"],
            "questionNumber": 14,
            "sourcePageOneBased": 3,
            "answerFetch": "read_source",
            "rulings": [],
            "unresolved": [],
            "finishNote": "受控服务：q14 已按抓取到的答案页改为 A"
        }),
    )
    .expect("写剧本");

    // 与 L0→L1 用例相同的显式启动：多带 `--request-log`，受控服务会把它收到的
    // **每一轮真实 HTTP 请求体**落盘——「第一轮请求里没有答案行」靠它对账。
    let port = free_local_port();
    let child = std::process::Command::new(&node)
        .arg(&script)
        .arg("--port")
        .arg(port.to_string())
        .arg("--plan")
        .arg(&plan_path)
        .arg("--request-log")
        .arg(&request_log_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap_or_else(|error| panic!("起受控服务失败 node={node:?}: {error}"));
    let _guard = ChildGuard(child);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut ready = false;
    while std::time::Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(ready, "受控服务 15 秒内没有起来（端口 {port}）");

    crate::llm_profiles::save_profiles(
        &root,
        &[json!({
            "profileId": "controlled-repair",
            "name": "Controlled Repair Service",
            "provider": "OpenAiCompatible",
            "baseUrl": format!("http://127.0.0.1:{port}/v1"),
            "model": "controlled-repair-v1",
            "temperature": 0,
            "timeoutMs": 60000,
            "forceJson": true,
            "enabled": true
        })],
    )
    .expect("profile 必须能落盘");

    let not_cancelled = || false;
    let request = request(&root, &not_cancelled, 6);
    let report = run_packets(&request, |context: &Value, observations: &[Value]| {
        repair_authoring_step_through_gateway(
            &root,
            ITEM_ID,
            Some("controlled-repair"),
            context,
            observations,
        )
    })
    .expect("包模式循环必须跑完（受控服务真的被驱动过）");

    // ① 最终答案正确，且是从抓取到的页里读出来的。
    assert_eq!(
        read_answer(&root, "q14")["labels"],
        json!(["A"]),
        "抓取到答案页之后必须把答案改对"
    );

    // ② 真实请求体对账（受控服务的 --request-log 落盘了它收到的每一轮请求）：
    //    第一轮请求里**没有**答案行（模型此时不可能知道答案），也没有整份 PDF；
    //    第二轮请求带着上一轮抓取结果（答案行在观察里真实出现）。
    let captured = std::fs::read_to_string(&request_log_path)
        .expect("真实受控服务必须记录它实际收到的 HTTP 请求体");
    let bodies: Vec<&str> = captured.lines().collect();
    assert!(bodies.len() >= 2, "至少要有「抓取」与「修复」两轮请求：{bodies:#?}");
    assert!(
        !bodies[0].contains("14 A"),
        "第一轮请求里不得出现答案行「14 A」——出现即自证：{}",
        &bodies[0][..bodies[0].len().min(600)]
    );
    assert!(
        bodies.iter().all(|body| !body.contains("data:application/pdf;base64,")),
        "整个过程不得附整份 PDF"
    );
    assert!(
        bodies[1].contains("read_source") && bodies[1].contains("14 A"),
        "第二轮请求必须带着上一轮 read_source 抓回的答案页（观察里真实出现「14 A」）"
    );

    // ③ 抓取动作真的发生了：观察里有 read_source 的 ok 结果，且带着第 3 页的行文本。
    assert!(
        report.observations.iter().any(|observation| {
            observation["status"] == json!("ok")
                && observation["result"]["pages"]
                    .as_array()
                    .is_some_and(|pages| pages
                        .iter()
                        .any(|page| page["pageIndex"] == json!(3)
                            && page["lines"]
                                .as_array()
                                .is_some_and(|lines| lines.iter().any(|line| line["text"] == json!("14 A")))))
        }),
        "必须真的有一轮 read_source 把第 3 页的行文本带了回来：{:#?}",
        report.observations
    );

    // ④ L1 走的是**抓取工具**而不是「报告不够」：级别 ≥ 1，但 insufficientContext == 0。
    assert_eq!(
        report.packets[0]["insufficientContext"],
        json!(0),
        "这条路径没有调用 report_insufficient_context：{:#?}",
        report.packets[0]
    );
    assert!(
        report.packets[0]["escalationLevel"].as_u64().unwrap_or(0) >= 1,
        "用抓取工具取页也是 L1：{:#?}",
        report.packets[0]
    );

    // ⑤ 逐包请求记录对账：第一轮是 L0、包内页不含答案页；第二轮级别抬到 L1
    //（抓取工具也是 L1）。注意 `pagesIncluded` 记的是**包 scope**——抓取结果并不到包里
    //（那是 report_insufficient_context 的待遇），「取回的页到了模型手上」由 ②③ 证明。
    let records: Vec<Value> = std::fs::read_to_string(
        crate::util::job_dir(&root, ITEM_ID).join("llm-calls.jsonl"),
    )
    .expect("网关必须留下 llm-calls.jsonl")
    .lines()
    .filter_map(|line| serde_json::from_str(line).ok())
    .filter(|record: &Value| record["commandName"] == json!("repair_authoring_step"))
    .collect();
    assert!(records.len() >= 2, "至少两轮修复调用要落记录：{records:#?}");
    assert_eq!(records[0]["escalationLevel"], json!(0), "{:#?}", records[0]);
    assert_eq!(
        records[1]["escalationLevel"], json!(1),
        "read_source 抓页的那一轮必须是 L1：{:#?}",
        records[1]
    );
    let first_pages = records[0]["pagesIncluded"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !first_pages.contains(&json!(3)),
        "第一轮请求里没有答案页（它不在题组锚点页上）：{first_pages:?}"
    );

    let _ = std::fs::remove_dir_all(&root);
}
