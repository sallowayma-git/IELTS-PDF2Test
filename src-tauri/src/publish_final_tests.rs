//! 一键发布（显式放行记录）与「题库保存」（发布后冻结最终版 + 清理原文件）的
//! 命令处理层测试。
//!
//! 全部走真实的 command core：`get_workspace_item_core` 播种权威稿、
//! `publish_items_core` 发布、`apply_editor_commands_core` 编辑，
//! 断言落在 NAS 清单、SQLite 与 job 目录这些**磁盘事实**上。

use crate::authoring_v2_commands::AUTHORING_V2_SHADOW_FILE;
use crate::job_store::save_job;
use crate::nas_package_v2::{publish_items_core, ForceOverride, PublishItemsInput};
use crate::pdf_facts_shadow::SHADOW_ARTIFACT_FILE as DOCUMENT_V2_SHADOW_FILE;
use crate::product_chain::{
    build_e2e_png, chain_job, first_text_node, physical_shadow_for, temp_root, workspace_path,
    READY_AUTHORING_FIXTURE,
};
use crate::util::{ensure_app_dirs, ensure_job_dirs, job_dir, write_json};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

fn png_sha(png: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(png);
    format!("{:x}", hasher.finalize())
}

/// 播种一道 ready 题：授权稿 + 带题号声明行的物理 shadow + 一个伪原文件；
/// `with_image` 时再加一张被 passage figure 引用的图片资产。
fn seed_item(root: &Path, exam_id: &str, with_image: bool, mutate: impl FnOnce(&mut Value)) -> String {
    let job = chain_job(&format!("Publish final {exam_id}"));
    save_job(root, &job).unwrap();
    let dir = job_dir(root, &job.job_id);
    ensure_job_dirs(&dir).unwrap();
    let mut authoring: Value =
        serde_json::from_slice(&fs::read(workspace_path(READY_AUTHORING_FIXTURE)).unwrap()).unwrap();
    authoring["jobId"] = json!(job.job_id);
    authoring["exam"]["examId"] = json!(exam_id);
    let png = build_e2e_png();
    let descriptor = json!({
        "assetId": "img-map",
        "kind": "raster_image",
        "mime": "image/png",
        "relativePath": "assets/blobs/img-map.png",
        "sha256": png_sha(&png),
        "byteLength": png.len() as u64,
        "widthPx": 1,
        "heightPx": 1,
        "extractionMode": "embedded",
        "altText": "Map"
    });
    if with_image {
        let anchor = authoring
            .pointer("/passage/content/0/sourceAnchors/0")
            .cloned()
            .unwrap();
        authoring["assets"] = json!([descriptor.clone()]);
        authoring["passage"]["content"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "id": "passage-figure-map",
                "type": "figure",
                "provenanceStatus": "source",
                "sourceAnchors": [anchor],
                "assetId": "img-map",
                "display": {"widthPercent": 60, "align": "center"},
                "caption": []
            }));
    }
    mutate(&mut authoring);
    write_json(&dir.join(AUTHORING_V2_SHADOW_FILE), &authoring).unwrap();
    let mut physical = physical_shadow_for(&authoring);
    // 原文声明的题号域：独立于题组识别，用于 source question coverage。
    physical["pages"][0]["lines"] = json!([{
        "id": "line-declaration",
        "text": "Questions 14-15",
        "spanIds": []
    }]);
    physical["pages"][0]["regions"][0]["childLineIds"]
        .as_array_mut()
        .unwrap()
        .push(json!("line-declaration"));
    if with_image {
        physical["assets"] = json!([descriptor]);
        fs::create_dir_all(dir.join("assets").join("blobs")).unwrap();
        fs::write(dir.join("assets").join("blobs").join("img-map.png"), &png).unwrap();
    }
    write_json(&dir.join(DOCUMENT_V2_SHADOW_FILE), &physical).unwrap();
    fs::create_dir_all(dir.join("uploads")).unwrap();
    fs::write(dir.join("uploads").join("abcd1234-source.pdf"), b"%PDF-1.4 fake source").unwrap();
    fs::write(dir.join("pipeline-report.json"), b"{}").unwrap();
    crate::library::commands::get_workspace_item_core(root, &job.job_id)
        .expect("on-demand migration must seed the canonical draft");
    job.job_id
}

fn destination(root: &Path) -> PathBuf {
    root.join("nas").join("publish")
}

fn reading_root(root: &Path) -> PathBuf {
    crate::export_nas_library::nas_reading_exams_dir(
        &crate::export_nas_library::normalize_nas_library_root(&destination(root)),
    )
}

fn force_now() -> Option<ForceOverride> {
    Some(ForceOverride {
        confirmed_at: chrono::Utc::now().to_rfc3339(),
        acknowledged_reasons: Vec::new(),
    })
}

fn publish(root: &Path, item_ids: &[&str], force: Option<ForceOverride>) -> Result<Value, String> {
    publish_items_core(
        root,
        PublishItemsInput {
            item_ids: item_ids.iter().map(|id| id.to_string()).collect(),
            destination: destination(root).to_string_lossy().into_owned(),
            fault: None,
            force,
        },
    )
}

fn manifest(root: &Path) -> Value {
    let text = fs::read_to_string(reading_root(root).join("manifest.js")).unwrap();
    let json_text = text
        .trim()
        .trim_start_matches("window.__READING_EXAM_MANIFEST__ = ")
        .trim_end_matches(';');
    serde_json::from_str(json_text).unwrap()
}

fn outcome_for<'a>(result: &'a Value, item_id: &str) -> &'a Value {
    result["succeeded"]
        .as_array()
        .unwrap()
        .iter()
        .find(|outcome| outcome["itemId"] == json!(item_id))
        .unwrap_or_else(|| panic!("no outcome for {item_id}: {result}"))
}

struct PublishRecordRow {
    forced: i64,
    verdict: Value,
    reasons: Value,
    student_loadable: i64,
    status: String,
    edit_version: i64,
}

fn publish_records(root: &Path, item_id: &str) -> Vec<PublishRecordRow> {
    let conn = crate::library::repository::open_library_connection(root).unwrap();
    let mut statement = conn
        .prepare(
            "SELECT forced, verdict_json, reasons_json, student_loadable, status, edit_version
             FROM publish_records_v2 WHERE library_item_id = ?1 ORDER BY created_at, rowid",
        )
        .unwrap();
    let rows = statement
        .query_map([item_id], |row| {
            Ok(PublishRecordRow {
                forced: row.get(0)?,
                verdict: serde_json::from_str(&row.get::<_, String>(1)?).unwrap(),
                reasons: serde_json::from_str(&row.get::<_, String>(2)?).unwrap(),
                student_loadable: row.get(3)?,
                status: row.get(4)?,
                edit_version: row.get(5)?,
            })
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    rows
}

fn item_status(root: &Path, item_id: &str) -> String {
    let conn = crate::library::repository::open_library_connection(root).unwrap();
    crate::library::repository::get_item(&conn, item_id)
        .unwrap()
        .unwrap()
        .status
}

fn snapshot_receipts(root: &Path) -> Vec<Value> {
    let mut receipts = Vec::new();
    let releases = reading_root(root).join("releases");
    for batch in fs::read_dir(&releases).into_iter().flatten().flatten() {
        for snapshot in fs::read_dir(batch.path().join("snapshots")).into_iter().flatten().flatten() {
            let path = snapshot.path().join("manifest-v2.json");
            if path.is_file() {
                let mut receipt: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                receipt["__dir"] = json!(snapshot.path());
                receipts.push(receipt);
            }
        }
    }
    receipts
}

// ───────────────────────── Feature 1：一键发布 ─────────────────────────

#[test]
fn ready_item_publishes_normally_even_when_the_override_is_sent() {
    let root = temp_root("publish-ready-with-override");
    ensure_app_dirs(&root).unwrap();
    let item = seed_item(&root, "final-ready", false, |_| {});

    let result = publish(&root, &[&item], force_now()).expect("ready item must publish");
    let outcome = outcome_for(&result, &item);
    assert_eq!(outcome["forced"], json!(false), "{outcome}");
    assert_eq!(outcome["studentLoadable"], json!(true));

    let manifest = manifest(&root);
    assert!(manifest["final-ready"].is_object());
    assert!(
        manifest["final-ready"].get("publishOverride").is_none(),
        "只有真正绕过门禁时 override 才算被使用：{manifest}"
    );
    let records = publish_records(&root, &item);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].forced, 0);
    assert_eq!(records[0].student_loadable, 1);
    assert_eq!(records[0].verdict["status"], json!("ready"));
    assert_eq!(item_status(&root, &item), "published");
    let receipts = snapshot_receipts(&root);
    assert!(receipts.iter().all(|receipt| receipt.get("publishOverride").is_none()));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn forced_publish_of_a_quality_blocked_item_records_the_override_and_keeps_the_verdict() {
    let root = temp_root("publish-forced-blocked");
    ensure_app_dirs(&root).unwrap();
    let item = seed_item(&root, "final-blocked", false, |_| {});
    // 非清理条目缺 physical shadow ⇒ 与今天一样判为 review_required（门禁 Blocked）。
    fs::remove_file(job_dir(&root, &item).join(DOCUMENT_V2_SHADOW_FILE)).unwrap();

    // 不带 override 的严格发布仍按原样被门禁拒绝。
    let strict = publish(&root, &[&item], None).unwrap_err();
    assert!(strict.contains("authoring_v2_export_blocked"), "{strict}");
    assert!(publish_records(&root, &item).is_empty(), "被拒的严格发布不得留下发布记录");

    let result = publish(&root, &[&item], force_now()).expect("forced publish must succeed");
    let outcome = outcome_for(&result, &item);
    assert_eq!(outcome["forced"], json!(true), "{outcome}");
    assert_eq!(outcome["studentLoadable"], json!(true));

    let manifest = manifest(&root);
    let entry = &manifest["final-blocked"];
    assert!(entry.is_object(), "可编译的强制发布题必须进学生清单：{manifest}");
    let override_ = &entry["publishOverride"];
    assert_eq!(override_["forced"], json!(true));
    assert_eq!(override_["verdict"]["status"], json!("blocked"), "门禁结论必须原样保留");
    assert_eq!(override_["verdict"]["ready"], json!(false));
    assert!(override_["confirmedAt"].as_str().is_some_and(|value| !value.is_empty()));
    assert!(override_["reasons"].as_array().is_some_and(|reasons| !reasons.is_empty()));
    let forced_items = manifest["_meta"]["forcedItems"].as_array().expect("_meta.forcedItems");
    assert!(forced_items.iter().any(|item_meta| item_meta["examId"] == json!("final-blocked")));

    let records = publish_records(&root, &item);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].forced, 1);
    assert_eq!(records[0].student_loadable, 1);
    assert_eq!(records[0].verdict["status"], json!("blocked"));
    assert!(records[0].reasons.as_array().is_some_and(|reasons| !reasons.is_empty()));
    assert_eq!(records[0].status, "published_forced");
    assert_eq!(item_status(&root, &item), "published_forced");

    let receipt = snapshot_receipts(&root)
        .into_iter()
        .find(|receipt| receipt["examId"] == json!("final-blocked"))
        .expect("snapshot receipt");
    assert_eq!(receipt["publishOverride"]["forced"], json!(true));
    assert_eq!(receipt["publishOverride"]["verdict"]["status"], json!("blocked"));
    assert_eq!(receipt["reviewRequired"], json!(true), "reviewRequired 必须如实");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn forced_publish_with_an_unresolved_answer_is_authoring_only_and_invents_nothing() {
    let root = temp_root("publish-forced-unresolved");
    ensure_app_dirs(&root).unwrap();
    let item = seed_item(&root, "final-unresolved", false, |authoring| {
        authoring["answerKey"]["q15"] = json!({"kind": "unresolved"});
    });

    let result = publish(&root, &[&item], force_now()).expect("forced publish must succeed");
    let outcome = outcome_for(&result, &item);
    assert_eq!(outcome["forced"], json!(true));
    assert_eq!(outcome["studentLoadable"], json!(false), "{outcome}");

    let manifest_path = reading_root(&root).join("manifest.js");
    if manifest_path.is_file() {
        let manifest = manifest(&root);
        assert!(
            manifest.get("final-unresolved").is_none(),
            "学生端加载不了的题不得进学生清单：{manifest}"
        );
    }
    let receipt = snapshot_receipts(&root)
        .into_iter()
        .find(|receipt| receipt["examId"] == json!("final-unresolved"))
        .expect("authoring-only snapshot must be written");
    assert_eq!(receipt["studentLoadable"], json!(false));
    assert_eq!(receipt["publishOverride"]["forced"], json!(true));
    let dir = PathBuf::from(receipt["__dir"].as_str().unwrap());
    assert!(!dir.join("reading-source-v2.json").exists(), "不得产出学生端运行时");
    let authoring: Value =
        serde_json::from_slice(&fs::read(dir.join("authoring-ir-v2.json")).unwrap()).unwrap();
    assert_eq!(
        authoring["answerKey"]["q15"]["kind"],
        json!("unresolved"),
        "强制发布绝不编造答案"
    );
    let records = publish_records(&root, &item);
    assert_eq!(records[0].forced, 1);
    assert_eq!(records[0].student_loadable, 0);
    assert_eq!(item_status(&root, &item), "published_forced");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn forced_publish_with_a_compile_failure_writes_only_the_authoring_snapshot() {
    let root = temp_root("publish-forced-compile");
    ensure_app_dirs(&root).unwrap();
    let item = seed_item(&root, "final-compile", false, |authoring| {
        authoring["answerKey"].as_object_mut().unwrap().remove("q15");
    });

    let result = publish(&root, &[&item], force_now()).expect("forced publish must succeed");
    let outcome = outcome_for(&result, &item);
    assert_eq!(outcome["studentLoadable"], json!(false), "{outcome}");
    let receipt = snapshot_receipts(&root)
        .into_iter()
        .find(|receipt| receipt["examId"] == json!("final-compile"))
        .expect("snapshot");
    let dir = PathBuf::from(receipt["__dir"].as_str().unwrap());
    assert!(dir.join("authoring-ir-v2.json").is_file());
    assert!(!dir.join("reading-source-v2.json").exists());
    let records = publish_records(&root, &item);
    assert_eq!(records[0].student_loadable, 0);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_forced_item_does_not_abort_a_batch_with_a_ready_item() {
    let root = temp_root("publish-forced-batch");
    ensure_app_dirs(&root).unwrap();
    let ready = seed_item(&root, "final-batch-ready", false, |_| {});
    let blocked = seed_item(&root, "final-batch-unresolved", false, |authoring| {
        authoring["answerKey"]["q15"] = json!({"kind": "unresolved"});
    });
    let result = publish(&root, &[&ready, &blocked], force_now()).expect("batch must publish");
    assert_eq!(result["succeeded"].as_array().unwrap().len(), 2, "{result}");
    let manifest = manifest(&root);
    assert!(manifest["final-batch-ready"].is_object());
    assert!(manifest.get("final-batch-unresolved").is_none());
    assert_eq!(item_status(&root, &ready), "published");
    assert_eq!(item_status(&root, &blocked), "published_forced");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn malformed_or_unknown_publish_input_is_rejected_explicitly() {
    let base = json!({"itemIds": ["a"], "destination": "C:/nas"});
    // 未知字段（例如旧的 validationPolicy）必须报错，而不是被忽略成严格发布。
    let mut unknown = base.clone();
    unknown["validationPolicy"] = json!("force");
    assert!(serde_json::from_value::<PublishItemsInput>(unknown).is_err());
    for bad_force in [
        json!({}),
        json!({"confirmedAt": 5, "acknowledgedReasons": []}),
        json!({"confirmedAt": "2026-09-21T00:00:00Z", "acknowledgedReasons": [], "extra": 1}),
        json!(true),
    ] {
        let mut input = base.clone();
        input["force"] = bad_force.clone();
        assert!(
            serde_json::from_value::<PublishItemsInput>(input).is_err(),
            "malformed override must be rejected: {bad_force}"
        );
    }
    let mut ok = base.clone();
    ok["force"] = json!({"confirmedAt": "2026-09-21T00:00:00Z", "acknowledgedReasons": ["x"]});
    assert!(serde_json::from_value::<PublishItemsInput>(ok).is_ok());

    // 结构合法但时间戳非法：在任何写入之前明确报错，绝不静默降级成严格发布。
    let root = temp_root("publish-force-invalid-time");
    ensure_app_dirs(&root).unwrap();
    let item = seed_item(&root, "final-bad-time", false, |_| {});
    let error = publish(
        &root,
        &[&item],
        Some(ForceOverride {
            confirmed_at: "not-a-time".to_string(),
            acknowledged_reasons: Vec::new(),
        }),
    )
    .unwrap_err();
    assert!(error.starts_with("PUBLISH_FORCE_OVERRIDE_INVALID"), "{error}");
    assert!(!reading_root(&root).join("manifest.js").exists());
    assert!(publish_records(&root, &item).is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn force_never_bypasses_unsafe_exam_ids_asset_io_or_duplicate_exam_ids() {
    let root = temp_root("publish-force-hard-guards");
    ensure_app_dirs(&root).unwrap();

    let unsafe_item = seed_item(&root, "final-unsafe", false, |authoring| {
        authoring["exam"]["examId"] = json!("../evil");
    });
    let error = publish(&root, &[&unsafe_item], force_now()).unwrap_err();
    assert!(error.contains("exam_id"), "{error}");
    assert!(publish_records(&root, &unsafe_item).is_empty());

    let missing_asset = seed_item(&root, "final-missing-asset", true, |_| {});
    fs::remove_file(job_dir(&root, &missing_asset).join("assets").join("blobs").join("img-map.png")).unwrap();
    let error = publish(&root, &[&missing_asset], force_now()).unwrap_err();
    assert!(error.contains("asset"), "{error}");
    assert!(publish_records(&root, &missing_asset).is_empty());

    let first = seed_item(&root, "final-dup", false, |_| {});
    let second = seed_item(&root, "final-dup", false, |authoring| {
        authoring["answerKey"]["q15"] = json!({"kind": "unresolved"});
    });
    let error = publish(&root, &[&first, &second], force_now()).unwrap_err();
    assert!(error.contains("PUBLISH_DUPLICATE_EXAM_ID"), "{error}");
    assert!(!reading_root(&root).join("manifest.js").exists());
    assert!(publish_records(&root, &first).is_empty());
    let _ = fs::remove_dir_all(root);
}
