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
/// 返回 `(baseUrl, 收到的请求体)`。请求体留痕是为了断言「发出去的确实是修复请求，
/// 且带着原文件证据面」，而不是只看最终结果猜中间发生了什么。
fn spawn_scripted_repair_service() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
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

            let content = scripted_repair_reply(&body, round);
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
