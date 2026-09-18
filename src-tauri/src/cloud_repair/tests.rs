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

fn request<'a>(root: &'a Path, cancelled: &'a dyn Fn() -> bool, max_rounds: u32) -> RepairRunRequest<'a> {
    RepairRunRequest {
        root,
        item_id: ITEM_ID,
        job_id: ITEM_ID,
        batch_id: BATCH_ID,
        repair_run_id: "run-1",
        max_rounds,
        deadline: Instant::now() + std::time::Duration::from_secs(30),
        cancelled,
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
