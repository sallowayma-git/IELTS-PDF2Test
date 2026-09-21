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
            "cloud-q15": {"kind": "option", "labels": ["D"], "assignment": "unordered_set"}
        },
        "assets": []
    })
}

/// 把候选规范化并落盘到独立 artifact（供 `build_repair_context` 读取）。
fn store_candidate(root: &Path, q14_label: &str) {
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
    let raw = json!({"authoring": cloud_draft(q14_label)});
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
    let report = run_repair_loop(&request, |context: &Value, _observations: &[Value]| {
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
    let report = run_repair_loop(&request, |_context: &Value, _observations: &[Value]| {
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
    let report = run_repair_loop(&request, |_context: &Value, _observations: &[Value]| {
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
    let report = run_repair_loop(&request, |_context: &Value, _observations: &[Value]| {
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
    let report = run_repair_loop(&request, |_context: &Value, _observations: &[Value]| {
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
    let report = run_repair_loop(&request, |_context: &Value, _observations: &[Value]| {
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
    let second = run_repair_loop(&request, |_context: &Value, _observations: &[Value]| {
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
    let report = run_repair_loop(&request, |context: &Value, _observations: &[Value]| {
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
    let report = run_repair_loop(&request, |_context: &Value, _observations: &[Value]| {
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
    let first = run_repair_loop(&request, |_context: &Value, _observations: &[Value]| {
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
    let second = run_repair_loop(&request, |context: &Value, _observations: &[Value]| {
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
    run_repair_loop(&request, |context: &Value, _observations: &[Value]| {
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
                "evidence": [{
                    "sourceFileId": "early-approaches-pdf",
                    "pageIndex": 1,
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
    write_json(
        &dir.join("document-ir.json"),
        &json!({"pages":[{"pageIndex":0,"lines":[{"text":"Early approaches to organisational design."}]}]}),
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
    let repair = run_repair_loop(&request, |context, observations| {
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
    assert!(seen[0].contains("sourceText"), "DOCX repair must send source text evidence");
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
    let report = run_repair_loop(&request, |context: &Value, observations: &[Value]| {
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
                    "quote": "14 C"
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
                    "quote": "15 E"
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
                    "quote": "factor B (revised)"
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
                    "quote": "16-17 new factors"
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
    let report = run_repair_loop(&request, |context, observations| {
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
    let report = run_repair_loop(&request, |context, observations| {
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
    let report = run_repair_loop(&request, |context, observations| {
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
    let report = run_repair_loop(&request, |context, observations| {
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
    let report = run_repair_loop(&request, |context, observations| {
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
    let report = run_repair_loop(&request, |_context: &Value, _observations: &[Value]| {
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
    let report = run_repair_loop(&request, |_context: &Value, _observations: &[Value]| {
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
    let report = run_repair_loop(&request, |_context: &Value, _observations: &[Value]| {
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
    let report = run_repair_loop(&request, |_context: &Value, _observations: &[Value]| {
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
