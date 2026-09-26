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
fn seed_item(
    root: &Path,
    exam_id: &str,
    with_image: bool,
    mutate: impl FnOnce(&mut Value),
) -> String {
    let job = chain_job(&format!("Publish final {exam_id}"));
    save_job(root, &job).unwrap();
    let dir = job_dir(root, &job.job_id);
    ensure_job_dirs(&dir).unwrap();
    let mut authoring: Value =
        serde_json::from_slice(&fs::read(workspace_path(READY_AUTHORING_FIXTURE)).unwrap())
            .unwrap();
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
    fs::write(
        dir.join("uploads").join("abcd1234-source.pdf"),
        b"%PDF-1.4 fake source",
    )
    .unwrap();
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
        for snapshot in fs::read_dir(batch.path().join("snapshots"))
            .into_iter()
            .flatten()
            .flatten()
        {
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
    assert!(receipts
        .iter()
        .all(|receipt| receipt.get("publishOverride").is_none()));
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
    assert!(
        publish_records(&root, &item).is_empty(),
        "被拒的严格发布不得留下发布记录"
    );

    let result = publish(&root, &[&item], force_now()).expect("forced publish must succeed");
    let outcome = outcome_for(&result, &item);
    assert_eq!(outcome["forced"], json!(true), "{outcome}");
    assert_eq!(outcome["studentLoadable"], json!(true));

    let manifest = manifest(&root);
    let entry = &manifest["final-blocked"];
    assert!(
        entry.is_object(),
        "可编译的强制发布题必须进学生清单：{manifest}"
    );
    let override_ = &entry["publishOverride"];
    assert_eq!(override_["forced"], json!(true));
    assert_eq!(
        override_["verdict"]["status"],
        json!("blocked"),
        "门禁结论必须原样保留"
    );
    assert_eq!(override_["verdict"]["ready"], json!(false));
    assert!(override_["confirmedAt"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
    assert!(override_["reasons"]
        .as_array()
        .is_some_and(|reasons| !reasons.is_empty()));
    let forced_items = manifest["_meta"]["forcedItems"]
        .as_array()
        .expect("_meta.forcedItems");
    assert!(forced_items
        .iter()
        .any(|item_meta| item_meta["examId"] == json!("final-blocked")));

    let records = publish_records(&root, &item);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].forced, 1);
    assert_eq!(records[0].student_loadable, 1);
    assert_eq!(records[0].verdict["status"], json!("blocked"));
    assert!(records[0]
        .reasons
        .as_array()
        .is_some_and(|reasons| !reasons.is_empty()));
    assert_eq!(records[0].status, "published_forced");
    assert_eq!(item_status(&root, &item), "published_forced");

    let receipt = snapshot_receipts(&root)
        .into_iter()
        .find(|receipt| receipt["examId"] == json!("final-blocked"))
        .expect("snapshot receipt");
    assert_eq!(receipt["publishOverride"]["forced"], json!(true));
    assert_eq!(
        receipt["publishOverride"]["verdict"]["status"],
        json!("blocked")
    );
    assert_eq!(
        receipt["reviewRequired"],
        json!(true),
        "reviewRequired 必须如实"
    );
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
    assert!(
        !dir.join("reading-source-v2.json").exists(),
        "不得产出学生端运行时"
    );
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
    assert_eq!(
        item_status(&root, &item),
        "published_forced_not_loadable",
        "放行了、但学生端打不开：状态必须同时说出这两件事，不能只说「已发布」"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn forced_publish_with_a_compile_failure_writes_only_the_authoring_snapshot() {
    let root = temp_root("publish-forced-compile");
    ensure_app_dirs(&root).unwrap();
    let item = seed_item(&root, "final-compile", false, |authoring| {
        authoring["answerKey"]
            .as_object_mut()
            .unwrap()
            .remove("q15");
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
    assert_eq!(
        item_status(&root, &blocked),
        "published_forced_not_loadable",
        "答案没闭合的那条放行后仍打不开，状态要如实说"
    );
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
    assert!(
        error.starts_with("PUBLISH_FORCE_OVERRIDE_INVALID"),
        "{error}"
    );
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
    fs::remove_file(
        job_dir(&root, &missing_asset)
            .join("assets")
            .join("blobs")
            .join("img-map.png"),
    )
    .unwrap();
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

// ───────────────────────── Feature 2：题库保存 ─────────────────────────

fn final_version_row(
    root: &Path,
    item_id: &str,
) -> Option<(i64, Value, Option<String>, Option<Value>)> {
    let conn = crate::library::repository::open_library_connection(root).unwrap();
    conn.query_row(
        "SELECT edit_version, evidence_json, source_purged_at, purge_report_json
         FROM library_final_versions_v2 WHERE library_item_id = ?1",
        [item_id],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                serde_json::from_str::<Value>(&row.get::<_, String>(1)?).unwrap(),
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?
                    .map(|text| serde_json::from_str::<Value>(&text).unwrap()),
            ))
        },
    )
    .ok()
}

fn list_relative_files(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, files: &mut Vec<String>) {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, files);
            } else {
                files.push(
                    path.strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let mut files = Vec::new();
    walk(dir, dir, &mut files);
    files.sort();
    files
}

fn edit_first_text(root: &Path, item_id: &str, suffix: &str) -> i64 {
    let workspace = crate::library::commands::get_workspace_item_core(root, item_id).unwrap();
    let version = workspace["editVersion"].as_i64().unwrap();
    let (node_id, text) = first_text_node(&workspace["ds"]).expect("text node");
    let length = text.chars().count();
    let result = crate::library::commands::apply_editor_commands_core(
        root,
        crate::library::repository::ApplyEditorCommandsInput {
            item_id: item_id.to_string(),
            base_version: version,
            request_id: Some(format!("edit-{item_id}-{version}")),
            commands: vec![json!({
                "op": "replaceText",
                "nodeId": node_id,
                "from": length,
                "to": length,
                "text": suffix
            })],
            title: None,
        },
    )
    .expect("editing a purged item must save");
    result["editVersion"].as_i64().unwrap()
}

#[test]
fn publish_freezes_evidence_and_purges_only_this_items_sources() {
    let root = temp_root("final-purge");
    ensure_app_dirs(&root).unwrap();
    let item = seed_item(&root, "final-purge", true, |_| {});
    let untouched = seed_item(&root, "final-untouched", false, |_| {});
    // 另一位 agent 的听力托管音频：属于可编辑版本，绝不可被清理。
    let audio = root.join("audio").join(&item).join("part1.mp3");
    fs::create_dir_all(audio.parent().unwrap()).unwrap();
    fs::write(&audio, b"ID3 fake audio").unwrap();
    let untouched_before = list_relative_files(&job_dir(&root, &untouched));

    let result = publish(&root, &[&item], force_now()).expect("publish must succeed");
    let outcome = outcome_for(&result, &item);
    assert_eq!(outcome["finalVersion"]["frozen"], json!(true), "{outcome}");
    assert_eq!(
        outcome["finalVersion"]["sourcePurged"],
        json!(true),
        "{outcome}"
    );

    let remaining = list_relative_files(&job_dir(&root, &item));
    assert_eq!(
        remaining,
        vec![
            "assets/blobs/img-map.png".to_string(),
            "job.json".to_string()
        ],
        "只保留权威稿引用的资产与作业元数据"
    );
    assert!(
        audio.is_file(),
        "<appData>/audio/<itemId>/ 下的文件必须保留"
    );
    assert_eq!(
        list_relative_files(&job_dir(&root, &untouched)),
        untouched_before,
        "不得清理其他条目的文件"
    );

    let (edit_version, evidence, purged_at, purge_report) =
        final_version_row(&root, &item).expect("final version row");
    assert_eq!(edit_version, 1);
    assert!(purged_at.is_some());
    assert_eq!(evidence["declaredQuestionNumbers"], json!([14, 15]));
    assert_eq!(evidence["questionCoverage"]["status"], json!("complete"));
    assert_eq!(evidence["nodeCoverage"]["complete"], json!(true));
    assert_eq!(evidence["publishedEditVersion"], json!(1));
    assert!(evidence["publishedAt"].as_str().is_some());
    assert_eq!(purge_report.unwrap()["failed"], json!([]));

    // 重新打开：预览资产仍可解析；工作区知道原文件已删除。
    let preview =
        crate::authoring_v2_commands::resolve_authoring_asset_preview_core(&root, &item, "img-map")
            .expect("a reopened preview must still render the kept image");
    assert_eq!(preview["assetId"], json!("img-map"));
    let workspace = crate::library::commands::get_workspace_item_core(&root, &item).unwrap();
    assert_eq!(workspace["item"]["sourcePurged"], json!(true));
    assert!(workspace["ds"].is_object(), "最终版必须仍可编辑");
    let _ = fs::remove_dir_all(root);
}

/// F3：发布后**解析缓存**（`<appData>/cache/parser/`）也要随源产物一起清掉。
///
/// 修前 `purge_source_artifacts` 只扫 job 目录，从不碰 `cache/parser/`：发布后
/// 这道题的原文抽取结果仍留在磁盘上，与「发布后只保留可编辑最终版」相违。
/// 归属按**精确身份**匹配（T5-b 改好的那条规则），所以 id 相近的另一条不受牵连。
#[test]
fn publishing_purges_this_items_parser_cache_and_leaves_a_similar_id_alone() {
    let root = temp_root("final-parser-cache");
    ensure_app_dirs(&root).unwrap();
    let item = seed_item(&root, "final-cache", false, |_| {});

    let cache = root.join("cache").join("parser");
    fs::create_dir_all(&cache).unwrap();
    // 本条目：按 job id 命名，以及按 `<jobId>-` 前缀命名的答案页产物。
    let mine = [
        cache.join(format!("{item}-document-ir.json")),
        cache.join(format!("{item}-answer-src-2-document-ir.json")),
    ];
    for path in &mine {
        fs::write(path, b"{}").unwrap();
    }
    // 另一道题的 id 只比本条目多一个字符——正是 T5-b 那个 `job-1` / `job-10` 陷阱：
    // 按前缀匹配会把它的缓存一起删掉，按精确身份匹配则不会。
    let similar = cache.join(format!("{item}0-document-ir.json"));
    fs::write(&similar, b"{}").unwrap();
    // 共享资产目录不属于任何单个条目，必须留下。
    let shared = cache.join("image-assets");
    fs::create_dir_all(&shared).unwrap();

    let result = publish(&root, &[&item], force_now()).expect("publish must succeed");
    let outcome = outcome_for(&result, &item);
    assert_eq!(
        outcome["finalVersion"]["purge"]["parserCache"]["cleaned"],
        json!(true),
        "发布报告要如实说解析缓存被清了：{outcome}"
    );

    for path in &mine {
        assert!(
            !path.exists(),
            "发布后本条目的解析缓存必须消失：{}",
            path.display()
        );
    }
    assert!(similar.exists(), "id 相近的另一道题的缓存不得被牵连");
    assert!(shared.is_dir(), "共享资产目录不属于任何单个条目，不得删除");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn purged_item_reopens_edits_saves_and_republishes_as_a_normal_publish() {
    let root = temp_root("final-republish");
    ensure_app_dirs(&root).unwrap();
    let item = seed_item(&root, "final-republish", true, |_| {});
    publish(&root, &[&item], force_now()).expect("first publish");
    assert!(!job_dir(&root, &item).join(DOCUMENT_V2_SHADOW_FILE).exists());

    let version = edit_first_text(&root, &item, " (revised)");
    assert_eq!(version, 2);
    let workspace = crate::library::commands::get_workspace_item_core(&root, &item).unwrap();
    assert_eq!(
        workspace["ds"]["quality"]["coverageStatus"]["physicalShadow"],
        json!("verified_at_publish_source_purged")
    );
    assert_eq!(
        workspace["ds"]["quality"]["state"],
        json!("ready"),
        "{:#}",
        workspace["ds"]["quality"]
    );

    let result = publish(&root, &[&item], force_now()).expect("republish must succeed");
    let outcome = outcome_for(&result, &item);
    assert_eq!(
        outcome["forced"],
        json!(false),
        "清理后的正常稿必须是正常发布：{outcome}"
    );
    assert_eq!(outcome["studentLoadable"], json!(true));
    let records = publish_records(&root, &item);
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].forced, 0);
    assert_eq!(records[1].edit_version, 2);
    assert_eq!(records[1].verdict["status"], json!("ready"));
    assert_eq!(item_status(&root, &item), "published");
    let (edit_version, evidence, _, _) = final_version_row(&root, &item).unwrap();
    assert_eq!(edit_version, 2, "最终版只有一份，指向最新发布的版本");
    assert_eq!(
        evidence["declaredQuestionNumbers"],
        json!([14, 15]),
        "冻结声明被沿用"
    );
    let manifest = manifest(&root);
    assert!(manifest["final-republish"].get("publishOverride").is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn purged_item_with_a_deleted_question_republishes_as_forced_and_blocked_by_coverage() {
    let root = temp_root("final-delete-question");
    ensure_app_dirs(&root).unwrap();
    let item = seed_item(&root, "final-delete-question", false, |_| {});
    publish(&root, &[&item], force_now()).expect("first publish");

    // 删除第 15 题。编辑器没有针对该共享多选题组的单命令删除路径，这里直接按
    // 编辑事务的写法改权威稿并推进版本（质量块由导出时统一重算）。
    let conn = crate::library::repository::open_library_connection(&root).unwrap();
    let (mut ds, version) = crate::library::repository::get_canonical_ds(&conn, &item)
        .unwrap()
        .unwrap();
    ds["answerSlots"].as_object_mut().unwrap().remove("q15");
    ds["answerKey"].as_object_mut().unwrap().remove("q15");
    for group in ds["taskGroups"][0]["responseGroups"]
        .as_array_mut()
        .unwrap()
    {
        if let Some(slots) = group.get_mut("slotIds").and_then(Value::as_array_mut) {
            slots.retain(|slot| slot != "q15");
        }
    }
    conn.execute(
        "UPDATE library_items_v2 SET canonical_ds_json = ?2, current_edit_version = ?3 WHERE id = ?1",
        rusqlite::params![item, ds.to_string(), version + 1],
    )
    .unwrap();
    drop(conn);

    let mut refreshed = ds.clone();
    crate::authoring_v2_commands::refresh_quality_report(&root, &item, &mut refreshed).unwrap();
    assert_eq!(
        refreshed["quality"]["questionCoverage"]["missingQuestionNumbers"],
        json!([15])
    );
    assert!(refreshed["quality"]["hardFailures"]
        .as_array()
        .unwrap()
        .iter()
        .any(|code| code == "SOURCE_QUESTION_COVERAGE_MISSING"));

    let result = publish(&root, &[&item], force_now()).expect("republish must still go out");
    let outcome = outcome_for(&result, &item);
    assert_eq!(outcome["forced"], json!(true), "{outcome}");
    let records = publish_records(&root, &item);
    let last = records.last().unwrap();
    assert_eq!(last.forced, 1);
    assert_eq!(last.verdict["status"], json!("blocked"));
    assert!(
        last.verdict.to_string().contains("原文声明的题号集合"),
        "门禁原因必须是题号覆盖：{}",
        last.verdict
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn non_purged_item_with_a_missing_shadow_behaves_exactly_as_today() {
    let root = temp_root("final-non-purged");
    ensure_app_dirs(&root).unwrap();
    let item = seed_item(&root, "final-non-purged", false, |_| {});
    fs::remove_file(job_dir(&root, &item).join(DOCUMENT_V2_SHADOW_FILE)).unwrap();
    let conn = crate::library::repository::open_library_connection(&root).unwrap();
    let (mut ds, _) = crate::library::repository::get_canonical_ds(&conn, &item)
        .unwrap()
        .unwrap();
    drop(conn);
    crate::authoring_v2_commands::refresh_quality_report(&root, &item, &mut ds).unwrap();
    assert_eq!(
        ds["quality"]["coverageStatus"]["physicalShadow"],
        json!("missing")
    );
    assert_eq!(ds["quality"]["sourceCoverage"], json!(0.0));
    assert_eq!(ds["quality"]["state"], json!("review_required"));
    assert!(ds["quality"]["issues"]
        .as_array()
        .unwrap()
        .iter()
        .any(|issue| issue["code"] == "PHYSICAL_SHADOW_MISSING"));
    assert!(crate::library::final_version::ensure_source_available(&root, &item).is_ok());
    let error = publish(&root, &[&item], None).unwrap_err();
    assert!(error.contains("authoring_v2_export_blocked"), "{error}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn purged_items_reject_source_dependent_operations_explicitly() {
    let root = temp_root("final-guard");
    ensure_app_dirs(&root).unwrap();
    let item = seed_item(&root, "final-guard", false, |_| {});
    assert!(crate::library::final_version::ensure_source_available(&root, &item).is_ok());
    publish(&root, &[&item], force_now()).unwrap();
    let error = crate::library::final_version::ensure_source_available(&root, &item).unwrap_err();
    assert!(
        error.starts_with(crate::library::final_version::SOURCE_PURGED_ERROR),
        "{error}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_purge_failure_is_reported_but_never_fails_the_publish() {
    let root = temp_root("final-purge-failure");
    ensure_app_dirs(&root).unwrap();
    let item = seed_item(&root, "final-purge-failure", false, |_| {});
    let locked = job_dir(&root, &item)
        .join("uploads")
        .join("abcd1234-source.pdf");
    // 让删除必然失败：Windows 上以「不共享删除」打开句柄；其他平台把父目录设为只读。
    #[cfg(windows)]
    let _guard = {
        use std::os::windows::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&locked)
            .unwrap()
    };
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(locked.parent().unwrap(), fs::Permissions::from_mode(0o555)).unwrap();
    }

    let result =
        publish(&root, &[&item], force_now()).expect("purge failure must not fail publish");
    let outcome = outcome_for(&result, &item);
    let failed = outcome["finalVersion"]["purge"]["failed"]
        .as_array()
        .unwrap()
        .clone();
    assert!(!failed.is_empty(), "清理失败必须如实报告：{outcome}");
    assert_eq!(item_status(&root, &item), "published");
    assert!(manifest(&root)["final-purge-failure"].is_object());

    #[cfg(windows)]
    drop(_guard);
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(locked.parent().unwrap(), fs::Permissions::from_mode(0o755));
    }
    let _ = fs::remove_dir_all(root);
}

/// 播种一道**听力**题：识别阶段产出四个 Section、没有音频的草稿，音频由用户后传。
///
/// `bound_parts` 列出已上传音频的 Section 序号，其余 Section 保持未绑定（用户还没传完）。
/// 音频走真实的受管入口 `bind_audio`（受管表 + `<appData>/audio/<itemId>/<sha>.<ext>`），
/// 而不是往草稿里塞一个假 `media`——权威稿里的 `media` 只能由那条路产生。
///
/// 题库行在导入时就带上了用户确认的 `modality`，播种按行里的模态决定稿件形状：这正是 T1
/// 修掉的那个「听力卷被播成阅读形状」的缺陷。
fn seed_listening_item(root: &Path, exam_id: &str, bound_parts: &[i64]) -> String {
    let job = chain_job(&format!("Publish final {exam_id}"));
    save_job(root, &job).unwrap();
    let dir = job_dir(root, &job.job_id);
    ensure_job_dirs(&dir).unwrap();
    let mut authoring =
        serde_json::to_value(crate::test_support::complete_listening_exam()).unwrap();
    authoring["jobId"] = json!(job.job_id);
    authoring["exam"]["examId"] = json!(exam_id);
    authoring["assets"] = json!([]);
    for part in authoring["listening"]["parts"].as_array_mut().unwrap() {
        part.as_object_mut().unwrap().remove("media");
    }

    let conn = crate::library::repository::open_library_connection(root).unwrap();
    crate::library::repository::upsert_item_shell(
        &conn,
        &crate::library::repository::UpsertItemInput {
            id: &job.job_id,
            modality: "listening",
            title: "Listening Paper",
            status: "processing",
            source_asset_id: None,
        },
    )
    .unwrap();
    drop(conn);
    write_json(&dir.join(AUTHORING_V2_SHADOW_FILE), &authoring).unwrap();
    let physical = physical_shadow_for(&authoring);
    write_json(&dir.join(DOCUMENT_V2_SHADOW_FILE), &physical).unwrap();
    fs::create_dir_all(dir.join("uploads")).unwrap();
    fs::write(
        dir.join("uploads").join("abcd1234-source.pdf"),
        b"%PDF-1.4 fake source",
    )
    .unwrap();
    fs::write(dir.join("pipeline-report.json"), b"{}").unwrap();

    for ordinal in bound_parts {
        let upload = root.join(format!("upload-section-{ordinal}.wav"));
        crate::test_support::write_audio_fixture(&upload, 220.0 * (*ordinal as f64));
        let bound = crate::listening_audio::store::bind_audio(root, &job.job_id, *ordinal, &upload)
            .expect("binding a section upload must succeed");
        assert!(
            bound.playable,
            "section {ordinal} must probe clean: {:?}",
            bound.issue_codes
        );
    }

    crate::library::commands::get_workspace_item_core(root, &job.job_id)
        .expect("on-demand migration must seed the canonical draft");
    job.job_id
}

/// 缺一个 Section 音频的听力卷，放行后只能停在授权快照里。
///
/// 这一条在单一编译入口之前是**假通过**：导出走的是阅读编译器，一份没有 passage 的听力稿
/// 会被编成一份「空文章的阅读稿」并判定可加载，于是学生端拿到一份没有声音的卷子。
#[test]
fn forced_publish_of_a_listening_paper_missing_section_audio_is_authoring_only() {
    let root = temp_root("publish-forced-listening-audio");
    ensure_app_dirs(&root).unwrap();
    let item = seed_listening_item(&root, "final-listening-audio", &[1, 2, 4]);

    let result = publish(&root, &[&item], force_now()).expect("forced publish must succeed");
    let outcome = outcome_for(&result, &item);
    assert_eq!(outcome["forced"], json!(true));
    assert_eq!(outcome["studentLoadable"], json!(false), "{outcome}");

    let receipt = snapshot_receipts(&root)
        .into_iter()
        .find(|receipt| receipt["examId"] == json!("final-listening-audio"))
        .expect("authoring-only snapshot must be written");
    assert_eq!(receipt["studentLoadable"], json!(false));
    assert_eq!(
        receipt["reviewRequired"],
        json!(true),
        "reviewRequired 必须如实"
    );
    let dir = PathBuf::from(receipt["__dir"].as_str().unwrap());
    assert!(
        !dir.join("listening-source-v1.json").exists(),
        "缺音频的听力卷不得产出学生端运行时"
    );
    assert!(
        !dir.join("reading-source-v2.json").exists(),
        "听力卷绝不能被编成阅读稿"
    );
    // 已上传的三段音频原样留在授权快照里：放行不清理、不编造。
    let authoring: Value =
        serde_json::from_slice(&fs::read(dir.join("authoring-ir-v2.json")).unwrap()).unwrap();
    assert_eq!(authoring["modality"], json!("listening"));
    let parts = authoring["listening"]["parts"].as_array().unwrap();
    assert!(
        parts[0]["media"]["sha256"].is_string(),
        "已上传的 Section 音频不得被放行清掉"
    );
    assert!(parts[1]["media"]["sha256"].is_string());
    assert!(parts[3]["media"]["sha256"].is_string());
    assert!(
        parts[2].get("media").is_none(),
        "没上传的 Section 不得被伪造出音频"
    );
    assert_eq!(authoring["assets"].as_array().unwrap().len(), 3);
    if reading_root(&root).join("manifest.js").is_file() {
        assert!(
            manifest(&root).get("final-listening-audio").is_none(),
            "学生端加载不了的题不得进学生清单"
        );
    }
    let records = publish_records(&root, &item);
    assert_eq!(records[0].forced, 1);
    assert_eq!(records[0].student_loadable, 0);
    assert_eq!(
        item_status(&root, &item),
        "published_forced_not_loadable",
        "缺音频的听力卷放行后学生端打不开，状态要如实说"
    );
    let _ = fs::remove_dir_all(root);
}

/// 四个 Section 音频齐全的听力卷，放行后**要**进学生清单，并且以听力的身份进。
#[test]
fn forced_publish_of_a_complete_listening_paper_reaches_the_student_manifest() {
    let root = temp_root("publish-forced-listening-complete");
    ensure_app_dirs(&root).unwrap();
    let item = seed_listening_item(&root, "final-listening-full", &[1, 2, 3, 4]);

    let result = publish(&root, &[&item], force_now()).expect("publish must succeed");
    let outcome = outcome_for(&result, &item);
    assert_eq!(outcome["studentLoadable"], json!(true), "{outcome}");

    let manifest = manifest(&root);
    let entry = manifest
        .get("final-listening-full")
        .unwrap_or_else(|| panic!("the exam is in the manifest: {manifest}"));
    assert_eq!(entry["schemaVersion"], json!("ListeningExamSourceV1"));
    assert_eq!(entry["modality"], json!("listening"));

    let resources = reading_root(&root)
        .join("resources")
        .join("final-listening-full");
    let asset_manifest: Value =
        serde_json::from_slice(&fs::read(resources.join("asset-manifest.json")).unwrap()).unwrap();
    let assets = asset_manifest["assets"].as_object().unwrap();
    assert_eq!(
        assets.len(),
        4,
        "四段 Section 音频都要进资源清单: {asset_manifest}"
    );
    for (asset_id, descriptor) in assets {
        let relative = descriptor["relativePath"].as_str().unwrap();
        assert!(
            resources.join(relative).is_file(),
            "{asset_id} must reach the student resources directory ({relative})"
        );
    }
    let records = publish_records(&root, &item);
    assert_eq!(records[0].student_loadable, 1);
    let _ = fs::remove_dir_all(root);
}

// ───────────────── 放行发布：某一条打包失败不该拖垮整批 ─────────────────

/// 让一道题**通过门禁与导出、但在包检查处失败**：把资产声明成一个不在学生端离线
/// 运行时 allowlist 里的 MIME（`allowed_asset_mime` 只收 `image/*`、`audio/*`、
/// `application/octet-stream`）。
///
/// 刻意不用「删掉资源文件」制造失败：那是 IO 类硬错误，任务书要求它继续中断整批
/// （见 `force_never_bypasses_unsafe_exam_ids_asset_io_or_duplicate_exam_ids`）。
/// 这里要的是「这条题自己组装出来的包，学生端读不了」——门禁、导出、资源复制都照常
/// 通过，是学生加载器探针在包组装**之后**判定它不可加载。
fn seed_item_with_an_unloadable_asset_mime(root: &Path, exam_id: &str) -> String {
    seed_item(root, exam_id, true, |authoring| {
        authoring["assets"][0]["mime"] = json!("text/plain");
    })
}

/// 放行发布（用户已明确点击「就这样发」）时，某一条的**包检查**失败必须只降级这一条，
/// 整批继续。
///
/// 旧行为：`stage_package_files` 的 `?` 直接把错误冒到批次外层闭包，于是整批回滚、
/// staging/release 被删、manifest 不替换——用户点了「放行」，结果**一条都没发出去**，
/// 而且失败原因与他刚才确认的问题毫无关系。
///
/// 降级后的语义与「授权快照不进学生清单」完全一致：授权快照照发（用户能继续编辑/导出），
/// 只是这一条不进学生端清单。IO / 安全类硬错误**不**降级，仍中断整批。
#[test]
fn forced_publish_degrades_one_unpackageable_item_and_publishes_the_rest() {
    let root = temp_root("publish-forced-partial-package");
    ensure_app_dirs(&root).unwrap();
    let good = seed_item(&root, "final-good", false, |_| {});
    let bad = seed_item_with_an_unloadable_asset_mime(&root, "final-bad");

    let result =
        publish(&root, &[&good, &bad], force_now()).expect("放行发布时一条打不了包不该让整批失败");

    let good_outcome = outcome_for(&result, &good);
    assert_eq!(good_outcome["ok"], json!(true));
    assert_eq!(
        good_outcome["studentLoadable"],
        json!(true),
        "好的一条必须照常发布：{good_outcome}"
    );
    let bad_outcome = outcome_for(&result, &bad);
    assert_eq!(bad_outcome["ok"], json!(true), "{bad_outcome}");
    assert_eq!(
        bad_outcome["studentLoadable"],
        json!(false),
        "打不了包的那条必须降级成 authoring-only：{bad_outcome}"
    );
    assert!(
        bad_outcome["packageError"]
            .as_str()
            .is_some_and(|error| error.starts_with("nas_package_v2_probe_failed")),
        "降级必须如实带上原因码：{bad_outcome}"
    );

    // 磁盘事实：学生清单里只有好的那条；坏的那条不能留下半成品条目。
    let manifest = manifest(&root);
    assert!(
        manifest["final-good"].is_object(),
        "整批没有被回滚，好的一条真的进了清单：{manifest}"
    );
    assert!(
        manifest["final-bad"].is_null(),
        "打不了包的那条绝不能进学生清单：{manifest}"
    );

    // 授权快照仍然为两条都产出（用户能继续编辑/导出），只是坏的那条标为不可加载。
    // 这也是「降级发生在包检查、而不是导出」的判据：导出让了快照，包检查才失败。
    let receipts = snapshot_receipts(&root);
    assert_eq!(receipts.len(), 2, "两条都该有授权快照：{receipts:?}");

    // 打了一半的包不留：坏的那条的资源目录与脚本不能出现在 release 里。
    for batch in fs::read_dir(reading_root(&root).join("releases"))
        .unwrap()
        .flatten()
    {
        assert!(
            !batch.path().join("resources").join("final-bad").exists(),
            "降级条目不该留下资源目录：{}",
            batch.path().display()
        );
        assert!(
            !batch.path().join("final-bad.js").exists(),
            "降级条目不该留下脚本：{}",
            batch.path().display()
        );
    }

    // 发布记录如实区分两条。
    let records = publish_records(&root, &bad);
    assert_eq!(records.len(), 1, "{}", records.len());
    assert_eq!(records[0].student_loadable, 0);
    assert_eq!(
        records[0].verdict["ready"],
        json!(true),
        "门禁结论是**真实**的那份，没被这次打包失败改写：{:?}",
        records[0].verdict
    );
    assert_eq!(item_status(&root, &good), "published");
    // 坏的那条：门禁是 Ready、没有任何东西被放行（`forced == false`），但它装不进学生包。
    // 状态必须**如实说出学生端打不开**——旧断言写成 `published || published_forced`
    // 两边都能过，于是「门禁 Ready + 包检查失败 → 状态 published」这个错误永远测不出来。
    let bad_status = item_status(&root, &bad);
    assert_eq!(
        bad_status, "published_not_loadable",
        "门禁 Ready 但装不进学生包的条目，状态不能是 published：{bad_status}"
    );
    assert_ne!(
        bad_status, "published",
        "`published` 是「学生能打开」的同义词，学生打不开就不能用它"
    );
    assert_eq!(
        records[0].status, "published_not_loadable",
        "发布记录里的状态必须和题库行一致"
    );

    let _ = fs::remove_dir_all(root);
}

/// 严格发布（没有放行）时，包检查失败仍然如实让整批失败——用户没有说「就这样发」，
/// 后端不该替他决定「发一半也行」。
#[test]
fn strict_publish_still_fails_the_batch_when_one_item_cannot_be_packaged() {
    let root = temp_root("publish-strict-partial-package");
    ensure_app_dirs(&root).unwrap();
    let good = seed_item(&root, "strict-good", false, |_| {});
    let bad = seed_item_with_an_unloadable_asset_mime(&root, "strict-bad");

    let error = publish(&root, &[&good, &bad], None).expect_err("严格发布必须如实失败");
    assert!(
        error.contains("nas_package_v2_probe_failed"),
        "失败原因必须指向真正的问题：{error}"
    );
    assert!(
        !reading_root(&root).join("manifest.js").exists(),
        "严格发布失败时不得留下清单"
    );
    let _ = fs::remove_dir_all(root);
}
