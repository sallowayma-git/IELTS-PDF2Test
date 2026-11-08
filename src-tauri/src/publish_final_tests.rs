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
            paper: None,
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
/// P2 门禁收敛：质量未就绪（review_required）但**编译通过**的题稿，
/// 严格模式直接发布——不再需要放行，`forced` 恒为 false；质量事实作为
/// 警告随 verdict 带走，审计里分不清「用户确认过什么」的问题随之消失。
fn quality_blocked_but_compilable_item_publishes_without_override_and_reports_warnings() {
    let root = temp_root("publish-quality-warned");
    ensure_app_dirs(&root).unwrap();
    let item = seed_item(&root, "final-blocked", false, |_| {});
    // 非清理条目缺 physical shadow ⇒ 质量如实判为 review_required（P2 起是警告不是阻断）。
    fs::remove_file(job_dir(&root, &item).join(DOCUMENT_V2_SHADOW_FILE)).unwrap();

    let result = publish(&root, &[&item], None).expect("编译通过的题稿必须能严格发布");
    let outcome = outcome_for(&result, &item);
    assert_eq!(outcome["forced"], json!(false), "{outcome}");
    assert_eq!(outcome["studentLoadable"], json!(true));

    let manifest = manifest(&root);
    let entry = &manifest["final-blocked"];
    assert!(
        entry.is_object(),
        "编译通过的题必须进学生清单：{manifest}"
    );
    assert!(
        entry.get("publishOverride").is_none(),
        "没有任何东西被放行：清单条目不得带 publishOverride"
    );
    let forced_items = manifest["_meta"]["forcedItems"]
        .as_array()
        .expect("_meta.forcedItems");
    assert!(forced_items.is_empty(), "{forced_items:?}");

    let records = publish_records(&root, &item);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].forced, 0);
    assert_eq!(records[0].student_loadable, 1);
    assert_eq!(records[0].verdict["status"], json!("ready"));
    // 质量事实不丢失：作为警告随 verdict 记录。
    assert!(records[0]
        .verdict["warnings"]
        .as_array()
        .is_some_and(|warnings| warnings
            .iter()
            .any(|warning| warning["code"] == json!("QUALITY_NOT_READY"))));
    assert_eq!(records[0].status, "published");
    assert_eq!(item_status(&root, &item), "published");

    let receipt = snapshot_receipts(&root)
        .into_iter()
        .find(|receipt| receipt["examId"] == json!("final-blocked"))
        .expect("snapshot receipt");
    assert!(receipt.get("publishOverride").is_none());
    assert_eq!(
        receipt["reviewRequired"],
        json!(true),
        "reviewRequired 必须如实：稿子确实还需要人工过目"
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
    // P2 门禁收敛：review_required（缺 shadow 的如实结论）不再是发布阻断——
    // 编译通过的题稿严格模式即可发布，质量事实作为警告记录。
    let result = publish(&root, &[&item], None).expect("编译通过的题稿必须能严格发布");
    let outcome = outcome_for(&result, &item);
    assert_eq!(outcome["forced"], json!(false), "{outcome}");
    let records = publish_records(&root, &item);
    assert_eq!(records[0].verdict["status"], json!("ready"));
    assert!(records[0].verdict["warnings"]
        .as_array()
        .is_some_and(|warnings| warnings
            .iter()
            .any(|warning| warning["code"] == json!("QUALITY_NOT_READY"))));
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

    // V1.1：资源位于清单条目声明的批次目录（releases/<batchId>/resources/<examId>）。
    let resources = reading_root(&root).join(
        entry["resourcesBase"]
            .as_str()
            .expect("entry declares resourcesBase")
            .trim_start_matches("./"),
    );
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

// ───────────────────────── 组卷：当前试卷指针 ─────────────────────────

fn set_part(root: &Path, item_id: &str, label: &str) {
    crate::library::commands::set_library_item_part_core(root, item_id, Some(label))
        .expect("manual part must be set");
}

fn paper_spec(p1: &str, p2: &str, p3: &str, listening: &[(&str, &str)]) -> crate::nas_package_v2::PaperSpec {
    serde_json::from_value(json!({
        "reading": {"P1": p1, "P2": p2, "P3": p3},
        "listening": listening
            .iter()
            .map(|(part, item)| json!({"part": part, "itemId": item}))
            .collect::<Vec<_>>(),
    }))
    .expect("paper spec must deserialize with the wire field names")
}

fn publish_with_paper(
    root: &Path,
    item_ids: &[&str],
    paper: Option<crate::nas_package_v2::PaperSpec>,
) -> Result<Value, String> {
    publish_items_core(
        root,
        PublishItemsInput {
            item_ids: item_ids.iter().map(|id| id.to_string()).collect(),
            destination: destination(root).to_string_lossy().into_owned(),
            fault: None,
            force: None,
            paper,
        },
    )
}

fn current_paper_path(root: &Path) -> PathBuf {
    reading_root(root).join("current-paper.json")
}

#[test]
fn current_paper_is_null_before_any_paper_is_published() {
    let root = temp_root("paper-none");
    ensure_app_dirs(&root).unwrap();
    let value = crate::nas_package_v2::get_current_paper_core(&destination(&root).to_string_lossy())
        .expect("reading a missing pointer must not fail");
    assert!(value.is_null(), "{value}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn paper_publish_points_current_paper_at_the_committed_manifest_entries() {
    let root = temp_root("paper-ok");
    ensure_app_dirs(&root).unwrap();
    let p1 = seed_item(&root, "paper-p1", false, |_| {});
    let p2 = seed_item(&root, "paper-p2", false, |_| {});
    let p3 = seed_item(&root, "paper-p3", false, |_| {});
    set_part(&root, &p1, "P1");
    set_part(&root, &p2, "P2");
    set_part(&root, &p3, "P3");

    let result = publish_with_paper(
        &root,
        &[&p1, &p2, &p3],
        Some(paper_spec(&p1, &p2, &p3, &[])),
    )
    .expect("three ready reading items must publish as a paper");
    assert_eq!(result["paper"]["ok"], json!(true), "{result}");

    let current: Value = serde_json::from_slice(&fs::read(current_paper_path(&root)).unwrap()).unwrap();
    assert_eq!(current["schemaVersion"], json!("CurrentPaperV1"));
    assert_eq!(current["paperId"], result["paper"]["paperId"]);
    assert_eq!(current["batchId"], result["paper"]["batchId"]);
    assert_eq!(current["listening"], json!([]));

    let manifest = manifest(&root);
    let reading = current["reading"].as_array().unwrap();
    assert_eq!(reading.len(), 3, "{current}");
    for (entry, (part, item)) in reading.iter().zip([("P1", &p1), ("P2", &p2), ("P3", &p3)]) {
        assert_eq!(entry["part"], json!(part));
        let exam_id = outcome_for(&result, item)["examId"].clone();
        assert_eq!(entry["examId"], exam_id, "指针里的 examId 必须与清单条目一致");
        let committed = &manifest[exam_id.as_str().unwrap()];
        assert_eq!(entry["script"], committed["script"], "script 取自同一次提交后的清单");
        assert_eq!(entry["scriptSha256"], committed["checksums"]["scriptSha256"]);
        assert_eq!(entry["runtimeSha256"], committed["checksums"]["runtimeSha256"]);
        assert!(
            entry["script"].as_str().unwrap().starts_with(&format!(
                "./releases/{}/",
                current["batchId"].as_str().unwrap()
            )),
            "{entry}"
        );
    }

    // 历史留档：同一 paperId，学生端不读但必须可查。
    let history = crate::export_nas_library::normalize_nas_library_root(&destination(&root))
        .join(".publish-control")
        .join("papers")
        .join(format!("{}.json", current["paperId"].as_str().unwrap()));
    assert!(history.is_file(), "历史留档缺失：{}", history.display());
    let history_value: Value = serde_json::from_slice(&fs::read(&history).unwrap()).unwrap();
    assert_eq!(history_value, current);

    // 临时文件不得残留在学生端目录里。
    let leftovers: Vec<_> = fs::read_dir(reading_root(&root))
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(".current-paper-"))
        .collect();
    assert!(leftovers.is_empty(), "临时指针文件残留");

    // 读取命令与磁盘文件一致。
    let read_back = crate::nas_package_v2::get_current_paper_core(&destination(&root).to_string_lossy())
        .unwrap();
    assert_eq!(read_back, current);

    // 列表行的 examId 与指针一致（界面用它判断「是否在当前试卷」）。
    let rows = crate::library::commands::list_library_items_core(&root, false).unwrap();
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == json!(p1))
        .unwrap();
    assert_eq!(row["examId"], json!("paper-p1"));
    assert_eq!(row["questionTypes"], json!(["multiple_choice"]), "{row}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn paper_publish_rejects_a_part_mismatch_before_writing_anything() {
    let root = temp_root("paper-part-mismatch");
    ensure_app_dirs(&root).unwrap();
    let p1 = seed_item(&root, "mismatch-p1", false, |_| {});
    let p2 = seed_item(&root, "mismatch-p2", false, |_| {});
    let p3 = seed_item(&root, "mismatch-p3", false, |_| {});
    // p1 被标成 P2：三篇位置与 Part 不符，必须整次拒绝。
    set_part(&root, &p1, "P2");
    set_part(&root, &p2, "P2");
    set_part(&root, &p3, "P3");

    let error = publish_with_paper(&root, &[&p1, &p2, &p3], Some(paper_spec(&p1, &p2, &p3, &[])))
        .expect_err("Part 不符必须报错");
    assert_eq!(error, format!("PAPER_PART_MISMATCH:{p1}"));
    assert!(!current_paper_path(&root).exists(), "拒绝时不得写指针");
    assert!(
        !reading_root(&root).join("manifest.js").exists(),
        "拒绝发生在任何写入之前：清单也不得出现"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn paper_publish_requires_every_paper_item_to_be_in_the_batch() {
    let root = temp_root("paper-not-selected");
    ensure_app_dirs(&root).unwrap();
    let p1 = seed_item(&root, "unselected-p1", false, |_| {});
    let p2 = seed_item(&root, "unselected-p2", false, |_| {});
    let p3 = seed_item(&root, "unselected-p3", false, |_| {});
    set_part(&root, &p1, "P1");
    set_part(&root, &p2, "P2");
    set_part(&root, &p3, "P3");

    let error = publish_with_paper(&root, &[&p1, &p2], Some(paper_spec(&p1, &p2, &p3, &[])))
        .expect_err("试卷题必须同时出现在 item_ids 里");
    assert_eq!(error, format!("PAPER_ITEM_NOT_SELECTED:{p3}"));
    assert!(!current_paper_path(&root).exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn paper_publish_writes_no_pointer_when_a_paper_item_does_not_publish() {
    let root = temp_root("paper-incomplete");
    ensure_app_dirs(&root).unwrap();
    let p1 = seed_item(&root, "incomplete-p1", false, |_| {});
    let p2 = seed_item(&root, "incomplete-p2", false, |_| {});
    // p3 的答案未闭合：严格发布下它不进学生端，试卷必须整体不更新。
    let p3 = seed_item(&root, "incomplete-p3", false, |authoring| {
        authoring["answerKey"]["q15"] = json!({"kind": "unresolved"});
    });
    set_part(&root, &p1, "P1");
    set_part(&root, &p2, "P2");
    set_part(&root, &p3, "P3");

    let result = publish_with_paper(&root, &[&p1, &p2, &p3], Some(paper_spec(&p1, &p2, &p3, &[])))
        .expect("批次本身照常提交，缺题只体现在 paper 结果里");
    assert_eq!(result["paper"]["ok"], json!(false), "{result}");
    assert_eq!(result["paper"]["code"], json!("PAPER_INCOMPLETE"), "{result}");
    assert_eq!(result["paper"]["missing"], json!([p3.clone()]), "{result}");
    assert!(!current_paper_path(&root).exists(), "缺题时不得写指针");
    assert!(
        !reading_root(&root).join("manifest.js").exists(),
        "替换语义：缺题时整次不提交清单，否则旧试卷的题会被撤下"
    );
    assert_eq!(result["succeeded"], json!([]), "{result}");
    let _ = fs::remove_dir_all(root);
}

fn seed_part(root: &Path, exam_id: &str, part: &str) -> String {
    let item = seed_item(root, exam_id, false, |_| {});
    set_part(root, &item, part);
    item
}

fn manifest_exam_ids(root: &Path) -> Vec<String> {
    let mut ids: Vec<String> = manifest(root)
        .as_object()
        .unwrap()
        .keys()
        .filter(|key| key.as_str() != "_meta")
        .cloned()
        .collect();
    ids.sort();
    ids
}

/// 发布两套试卷：第一套 `rep-a*`，第二套 `rep-b*`。返回 (第一批 batchId, 第二批结果)。
fn publish_two_papers(root: &Path) -> (String, Value) {
    let (a1, a2, a3) = (
        seed_part(root, "rep-a1", "P1"),
        seed_part(root, "rep-a2", "P2"),
        seed_part(root, "rep-a3", "P3"),
    );
    let first = publish_with_paper(root, &[&a1, &a2, &a3], Some(paper_spec(&a1, &a2, &a3, &[])))
        .expect("first paper must publish");
    assert_eq!(first["paper"]["ok"], json!(true), "{first}");
    let (b1, b2, b3) = (
        seed_part(root, "rep-b1", "P1"),
        seed_part(root, "rep-b2", "P2"),
        seed_part(root, "rep-b3", "P3"),
    );
    let second = publish_with_paper(root, &[&b1, &b2, &b3], Some(paper_spec(&b1, &b2, &b3, &[])))
        .expect("second paper must publish");
    assert_eq!(second["paper"]["ok"], json!(true), "{second}");
    (first["paper"]["batchId"].as_str().unwrap().to_string(), second)
}

#[test]
fn second_paper_publish_replaces_the_manifest_instead_of_accumulating() {
    let root = temp_root("paper-replace");
    ensure_app_dirs(&root).unwrap();
    let (first_batch, second) = publish_two_papers(&root);

    assert_eq!(
        manifest_exam_ids(&root),
        vec!["rep-b1", "rep-b2", "rep-b3"],
        "清单只能含本套试卷，旧题必须撤下"
    );
    let current: Value = serde_json::from_slice(&fs::read(current_paper_path(&root)).unwrap()).unwrap();
    assert_eq!(current["paperId"], second["paper"]["paperId"]);
    // 撤下的旧批次在 24 小时内保留（正在考试的学生仍锁定旧脚本）。
    assert!(reading_root(&root).join("releases").join(&first_batch).is_dir());
    assert!(reading_root(&root)
        .join("releases")
        .join(&first_batch)
        .join("resources")
        .join("rep-a1")
        .is_dir());
    let kept = second["cleanup"]["kept"].as_array().unwrap();
    assert!(kept.contains(&json!(format!("releases/{first_batch}"))), "{second}");
    assert_eq!(second["cleanup"]["removed"], json!([]), "{second}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn superseded_files_are_removed_only_after_the_grace_period_and_never_the_current_paper() {
    let root = temp_root("paper-cleanup-grace");
    ensure_app_dirs(&root).unwrap();
    let (first_batch, second) = publish_two_papers(&root);
    let second_batch = second["paper"]["batchId"].as_str().unwrap().to_string();
    let library_root = crate::export_nas_library::normalize_nas_library_root(&destination(&root));
    let reading = reading_root(&root);
    let manifest_value = manifest(&root);
    let current = manifest_value.as_object().unwrap();

    // 未满 24 小时：什么都不删。
    let soon = crate::nas_package_v2::cleanup_superseded_releases(
        &library_root, &reading, current, &second_batch,
        chrono::Utc::now() + chrono::Duration::hours(1),
    );
    assert_eq!(soon["removed"], json!([]), "{soon}");
    assert!(reading.join("releases").join(&first_batch).is_dir());

    // 超过 24 小时：旧批次与旧题资源删除；当前试卷的一切从不删除。
    let later = crate::nas_package_v2::cleanup_superseded_releases(
        &library_root, &reading, current, &second_batch,
        chrono::Utc::now() + chrono::Duration::hours(25),
    );
    assert!(later["failed"].as_array().unwrap().is_empty(), "{later}");
    assert!(!reading.join("releases").join(&first_batch).exists(), "{later}");
    for old in ["rep-a1", "rep-a2", "rep-a3"] {
        assert!(!reading.join("resources").join(old).exists(), "{later}");
    }
    assert!(reading.join("releases").join(&second_batch).is_dir(), "当前批次绝不删除");
    for current_exam in ["rep-b1", "rep-b2", "rep-b3"] {
        assert!(
            reading
                .join("releases")
                .join(&second_batch)
                .join("resources")
                .join(current_exam)
                .is_dir(),
            "当前试卷资源绝不删除"
        );
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn an_undeletable_superseded_directory_is_reported_as_failed_not_raised() {
    let root = temp_root("paper-cleanup-failure");
    ensure_app_dirs(&root).unwrap();
    let (_first_batch, second) = publish_two_papers(&root);
    let second_batch = second["paper"]["batchId"].as_str().unwrap().to_string();
    let library_root = crate::export_nas_library::normalize_nas_library_root(&destination(&root));
    let reading = reading_root(&root);
    // 一个无归属记录的旧目录；删除动作注入为失败，模拟文件被占用。
    let stuck = reading.join("releases").join("orphan-batch");
    fs::create_dir_all(&stuck).unwrap();
    backdate_superseded(&root, "orphan-batch", chrono::Utc::now() - chrono::Duration::hours(48));
    let manifest_value = manifest(&root);
    let outcome = crate::nas_package_v2::cleanup_superseded_releases_with(
        &library_root, &reading, manifest_value.as_object().unwrap(), &second_batch,
        chrono::Utc::now() + chrono::Duration::hours(48),
        &|path| Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, format!("locked: {}", path.display()))),
    );
    // 清理只报告，不抛错：失败项带路径与原因；当前试卷不受影响。
    let failed = outcome["failed"].as_array().unwrap();
    assert!(
        failed.iter().any(|item| item["path"] == json!("releases/orphan-batch")),
        "删除失败必须进入 failed：{outcome}"
    );
    assert!(stuck.is_dir(), "删除失败时目录仍在，且不误报为 removed");
    assert!(outcome["removed"].as_array().unwrap().iter().all(|item| item != "releases/orphan-batch"));
    assert!(reading.join("releases").join(&second_batch).is_dir());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn an_incomplete_replacement_leaves_the_previous_paper_untouched() {
    let root = temp_root("paper-incomplete-keeps-old");
    ensure_app_dirs(&root).unwrap();
    let (_first_batch, _second) = publish_two_papers(&root);
    let before_manifest = manifest_exam_ids(&root);
    let before_pointer: Value = serde_json::from_slice(&fs::read(current_paper_path(&root)).unwrap()).unwrap();

    let (c1, c2) = (seed_part(&root, "rep-c1", "P1"), seed_part(&root, "rep-c2", "P2"));
    let c3 = seed_item(&root, "rep-c3", false, |authoring| {
        authoring["answerKey"]["q15"] = json!({"kind": "unresolved"});
    });
    set_part(&root, &c3, "P3");
    let result = publish_with_paper(&root, &[&c1, &c2, &c3], Some(paper_spec(&c1, &c2, &c3, &[])))
        .expect("缺题是正常结果，不是错误");
    assert_eq!(result["paper"]["code"], json!("PAPER_INCOMPLETE"), "{result}");
    assert_eq!(manifest_exam_ids(&root), before_manifest, "旧试卷的清单不得改变");
    let after_pointer: Value = serde_json::from_slice(&fs::read(current_paper_path(&root)).unwrap()).unwrap();
    assert_eq!(after_pointer, before_pointer);
    let _ = fs::remove_dir_all(root);
}

fn item_version(root: &Path, item_id: &str) -> i64 {
    let conn = crate::library::repository::open_library_connection(root).unwrap();
    conn.query_row(
        "SELECT current_edit_version FROM library_items_v2 WHERE id = ?1",
        [item_id],
        |row| row.get(0),
    )
    .unwrap()
}

fn batch_file(root: &Path, relative: &str) -> PathBuf {
    reading_root(root).join(relative.trim_start_matches("./"))
}

#[test]
fn rename_library_item_trims_validates_and_returns_the_row() {
    use crate::library::commands::rename_library_item_core as rename;
    let root = temp_root("rename-item");
    ensure_app_dirs(&root).unwrap();
    let item = seed_item(&root, "rename-me", false, |_| {});

    let version = item_version(&root, &item);
    let row = rename(&root, &item, "  新的题名  ", version).expect("trimmed non-empty title must save");
    assert_eq!(row["title"], json!("新的题名"), "{row}");
    assert_eq!(row["id"], json!(item));
    let version = item_version(&root, &item);
    assert_eq!(row["currentEditVersion"], json!(version), "改名走版本 CAS，版本必须前进");

    assert_eq!(rename(&root, &item, "   ", version).unwrap_err(), "LIBRARY_TITLE_EMPTY");
    let too_long = "字".repeat(121);
    assert!(rename(&root, &item, &too_long, version)
        .unwrap_err()
        .starts_with("LIBRARY_TITLE_TOO_LONG"));
    let at_limit = "字".repeat(120);
    assert!(rename(&root, &item, &at_limit, version).is_ok());
    let _ = fs::remove_dir_all(root);
}

/// 审计 A4 回归：已发布条目只改名，状态仍是 published（版本照常前进、照常 CAS），
/// 题库列表里仍有这一行且状态未变（前端的题库行与组卷栏都依据这个状态）。
#[test]
fn renaming_a_published_item_keeps_its_published_status() {
    use crate::library::commands::{list_library_items_core, rename_library_item_core as rename};
    let root = temp_root("rename-published");
    ensure_app_dirs(&root).unwrap();
    let item = seed_item(&root, "rename-published", false, |_| {});
    {
        let conn = crate::library::repository::open_library_connection(&root).unwrap();
        crate::library::repository::set_item_status(&conn, &item, "published").unwrap();
    }
    let before = item_version(&root, &item);
    rename(&root, &item, "改名后的已发布题", before).expect("rename must save");
    assert_eq!(item_version(&root, &item), before + 1, "改名仍递增版本");

    let conn = crate::library::repository::open_library_connection(&root).unwrap();
    let status: String = conn
        .query_row("SELECT status FROM library_items_v2 WHERE id = ?1", [&item], |row| row.get(0))
        .unwrap();
    assert_eq!(status, "published", "只改标题不得改变发布状态");
    drop(conn);
    let listing = list_library_items_core(&root, false).unwrap();
    let row = listing
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["id"] == json!(item)))
        .unwrap_or_else(|| panic!("改名后题库必须仍列出该条目：{listing}"));
    assert_eq!(row["status"], json!("published"), "{row}");
    assert_eq!(row["title"], json!("改名后的已发布题"), "{row}");
    let _ = fs::remove_dir_all(root);
}

/// 改名与编辑共用版本 CAS：基线版本过期时明确拒绝，且不改任何东西。
#[test]
fn rename_with_a_stale_base_version_is_refused_and_changes_nothing() {
    use crate::library::commands::rename_library_item_core as rename;
    let root = temp_root("rename-stale");
    ensure_app_dirs(&root).unwrap();
    let item = seed_item(&root, "rename-stale", false, |_| {});
    let stale = item_version(&root, &item);
    rename(&root, &item, "第一次改名", stale).expect("first rename must save");

    let conflict = rename(&root, &item, "过期的改名", stale).unwrap_err();
    assert!(conflict.starts_with("EDIT_VERSION_CONFLICT"), "{conflict}");
    let conn = crate::library::repository::open_library_connection(&root).unwrap();
    let title: String = conn
        .query_row("SELECT title FROM library_items_v2 WHERE id = ?1", [&item], |row| row.get(0))
        .unwrap();
    assert_eq!(title, "第一次改名", "冲突时题库标题不得被改");
    let _ = fs::remove_dir_all(root);
}

/// P1-4 回归：改名后重新组卷发布，清单条目与当前试卷指针都必须是新名。
#[test]
fn a_renamed_paper_item_publishes_its_new_title_into_manifest_and_pointer() {
    use crate::library::commands::rename_library_item_core as rename;
    let root = temp_root("rename-then-publish");
    ensure_app_dirs(&root).unwrap();
    let (p1, p2, p3) = (
        seed_part(&root, "rn-p1", "P1"),
        seed_part(&root, "rn-p2", "P2"),
        seed_part(&root, "rn-p3", "P3"),
    );
    publish_with_paper(&root, &[&p1, &p2, &p3], Some(paper_spec(&p1, &p2, &p3, &[])))
        .expect("first publish");

    rename(&root, &p1, "改名后的题", item_version(&root, &p1)).expect("rename");
    let result = publish_with_paper(&root, &[&p1, &p2, &p3], Some(paper_spec(&p1, &p2, &p3, &[])))
        .expect("republish after rename");
    assert_eq!(result["paper"]["ok"], json!(true), "{result}");

    let exam_id = outcome_for(&result, &p1)["examId"].as_str().unwrap().to_string();
    assert_eq!(manifest(&root)[&exam_id]["title"], json!("改名后的题"));
    let current: Value = serde_json::from_slice(&fs::read(current_paper_path(&root)).unwrap()).unwrap();
    assert_eq!(current["reading"][0]["title"], json!("改名后的题"), "{current}");
    assert_eq!(current["reading"][0]["entry"]["title"], json!("改名后的题"), "{current}");
    let _ = fs::remove_dir_all(root);
}

/// V1.1 第1条：同一内容重复发布时，两批次的 asset-manifest 字节相同；
/// 旧批次的文件（旧 pin 路径）在新发布之后仍在且字节未变。
#[test]
fn republishing_the_same_paper_keeps_old_batch_files_and_writes_identical_asset_manifests() {
    let root = temp_root("paper-immutable-batches");
    ensure_app_dirs(&root).unwrap();
    let (p1, p2, p3) = (
        seed_part(&root, "im-p1", "P1"),
        seed_part(&root, "im-p2", "P2"),
        seed_part(&root, "im-p3", "P3"),
    );
    let first = publish_with_paper(&root, &[&p1, &p2, &p3], Some(paper_spec(&p1, &p2, &p3, &[])))
        .expect("first publish");
    let first_batch = first["paper"]["batchId"].as_str().unwrap().to_string();
    let first_manifest = manifest(&root);
    let exams: Vec<String> = [&p1, &p2, &p3]
        .iter()
        .map(|item| outcome_for(&first, item)["examId"].as_str().unwrap().to_string())
        .collect();
    let mut pinned: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    for exam in &exams {
        let entry = &first_manifest[exam];
        for key in ["script", "assetManifest"] {
            let path = batch_file(&root, entry[key].as_str().unwrap());
            pinned.push((path.clone(), fs::read(&path).unwrap()));
        }
        let expected_base = format!("./releases/{first_batch}/resources/{exam}");
        assert_eq!(entry["resourcesBase"], json!(expected_base), "{entry}");
    }

    let second = publish_with_paper(&root, &[&p1, &p2, &p3], Some(paper_spec(&p1, &p2, &p3, &[])))
        .expect("second publish");
    assert_eq!(second["status"], json!("published"), "{second}");
    let second_batch = second["paper"]["batchId"].as_str().unwrap().to_string();
    assert_ne!(first_batch, second_batch);
    assert!(
        reading_root(&root).join("releases").join(&first_batch).is_dir(),
        "24 小时内旧批次必须保留"
    );

    for exam in &exams {
        let first_asset = fs::read(batch_file(
            &root,
            &format!("./releases/{first_batch}/resources/{exam}/asset-manifest.json"),
        ))
        .unwrap();
        let second_asset = fs::read(batch_file(
            &root,
            &format!("./releases/{second_batch}/resources/{exam}/asset-manifest.json"),
        ))
        .unwrap();
        assert_eq!(first_asset, second_asset, "同内容两次发布 asset-manifest 字节必须相同（{exam}）");
        let text = String::from_utf8(first_asset).unwrap();
        assert!(!text.contains("generatedAt"), "asset-manifest 不得含随时间变化的字段");
    }
    for (path, bytes) in pinned {
        assert_eq!(fs::read(&path).unwrap(), bytes, "旧 pin 路径文件必须字节不变：{}", path.display());
    }
    let _ = fs::remove_dir_all(root);
}

/// V1.1 第2条：指针的 entry 与清单原样一致；顶层 script/sha 与 entry 一致。
#[test]
fn pointer_entries_are_self_contained_copies_of_the_manifest_entries() {
    let root = temp_root("paper-pointer-entry");
    ensure_app_dirs(&root).unwrap();
    let (p1, p2, p3) = (
        seed_part(&root, "pe-p1", "P1"),
        seed_part(&root, "pe-p2", "P2"),
        seed_part(&root, "pe-p3", "P3"),
    );
    publish_with_paper(&root, &[&p1, &p2, &p3], Some(paper_spec(&p1, &p2, &p3, &[])))
        .expect("publish");
    let current: Value = serde_json::from_slice(&fs::read(current_paper_path(&root)).unwrap()).unwrap();
    let manifest = manifest(&root);
    for item in current["reading"].as_array().unwrap() {
        let exam = item["examId"].as_str().unwrap();
        assert_eq!(item["entry"], manifest[exam], "entry 必须是清单条目的原样拷贝");
        assert_eq!(item["script"], item["entry"]["script"]);
        assert_eq!(item["scriptSha256"], item["entry"]["checksums"]["scriptSha256"]);
        assert_eq!(item["runtimeSha256"], item["entry"]["checksums"]["runtimeSha256"]);
        assert!(item["entry"]["assetManifest"].as_str().unwrap().contains("/resources/"));
    }
    let _ = fs::remove_dir_all(root);
}

/// P1-1：指针写失败 → 顶层为未完成（`paper_pointer_stale`），不得清理旧批次。
/// 旧批次被回拨到超过 24 小时宽限期，若清理误执行它会被删掉——据此断言清理确实被跳过。
/// 指针恢复后再次发布，清理才执行，且旧批次（不再被指针引用、已过宽限期）被删除。
#[test]
fn a_failed_pointer_write_is_reported_as_stale_and_never_cleans_up() {
    let root = temp_root("paper-pointer-fail");
    ensure_app_dirs(&root).unwrap();
    let (p1, p2, p3) = (
        seed_part(&root, "pf-p1", "P1"),
        seed_part(&root, "pf-p2", "P2"),
        seed_part(&root, "pf-p3", "P3"),
    );
    let first = publish_with_paper(&root, &[&p1, &p2, &p3], Some(paper_spec(&p1, &p2, &p3, &[])))
        .expect("first publish");
    let batch_a = first["paper"]["batchId"].as_str().unwrap().to_string();
    // 批次 A 在 48 小时前已被撤下（撤下时间回拨；宽限期从撤下起算，不看发布时间）。
    backdate_superseded(&root, &batch_a, chrono::Utc::now() - chrono::Duration::hours(48));

    // 让指针无法替换：指针路径变成目录，原子替换必然失败。
    let pointer = current_paper_path(&root);
    fs::remove_file(&pointer).unwrap();
    fs::create_dir_all(&pointer).unwrap();
    let second = publish_with_paper(&root, &[&p1, &p2, &p3], Some(paper_spec(&p1, &p2, &p3, &[])))
        .expect("清单已提交，发布调用本身不得报错");
    assert_eq!(second["status"], json!("paper_pointer_stale"), "{second}");
    assert_eq!(second["paper"]["ok"], json!(false), "{second}");
    assert!(second.get("cleanup").is_none(), "指针失败时不得执行清理：{second}");
    assert!(
        reading_root(&root).join("releases").join(&batch_a).is_dir(),
        "旧批次必须保留"
    );
    assert_eq!(manifest(&root).as_object().unwrap().len() - 1, 3, "清单已是新一套（清单提交不因指针失败回滚）");

    // 恢复：指针可写后再次发布，清理执行，旧批次（已不被指针引用、已过宽限期）被删除。
    fs::remove_dir_all(&pointer).unwrap();
    let third = publish_with_paper(&root, &[&p1, &p2, &p3], Some(paper_spec(&p1, &p2, &p3, &[])))
        .expect("third publish");
    assert_eq!(third["status"], json!("published"), "{third}");
    assert_eq!(third["paper"]["ok"], json!(true), "{third}");
    let current: Value = serde_json::from_slice(&fs::read(&pointer).unwrap()).unwrap();
    let third_batch = third["paper"]["batchId"].as_str().unwrap();
    assert_eq!(current["batchId"], json!(third_batch));
    assert!(!reading_root(&root).join("releases").join(&batch_a).exists(), "{third}");
    assert!(reading_root(&root).join("releases").join(third_batch).is_dir(), "当前指针批次绝不删除");
    let _ = fs::remove_dir_all(root);
}

/// 撤下时间只由 `.publish-control/superseded/<key>.json` 决定：发布历史的 publishedAt 再早也不影响宽限期。
/// 历史记录的文件顺序与内容都不改变结果（替代原 P1-2 的「取最近发布时间」语义）。
#[test]
fn superseded_grace_ignores_publish_history_and_file_order() {
    for names in [["a-recent.json", "z-old.json"], ["z-old.json", "a-recent.json"]] {
        let root = temp_root("cleanup-order");
        let library_root = root.join("lib");
        let reading = crate::export_nas_library::nas_reading_exams_dir(&library_root);
        let history = library_root.join(".publish-control").join("papers");
        fs::create_dir_all(&history).unwrap();
        let now = chrono::Utc::now();
        let recent = (now - chrono::Duration::hours(2)).to_rfc3339();
        let old = (now - chrono::Duration::hours(300)).to_rfc3339();
        for name in names {
            let published = if name.starts_with("a-") { &recent } else { &old };
            let record = json!({
                "schemaVersion": "CurrentPaperV1",
                "paperId": name,
                "publishedAt": published,
                "batchId": "batch-shared",
                "reading": [{"examId": "exam-shared"}],
                "listening": []
            });
            fs::write(history.join(name), serde_json::to_vec(&record).unwrap()).unwrap();
        }
        fs::create_dir_all(reading.join("releases").join("batch-shared")).unwrap();
        fs::create_dir_all(reading.join("resources").join("exam-shared")).unwrap();
        // 撤下 2 小时前：未满宽限期，无论发布历史如何都保留。
        backdate_superseded(&root, "batch-shared", now - chrono::Duration::hours(2));

        let outcome = crate::nas_package_v2::cleanup_superseded_releases(
            &library_root,
            &reading,
            &serde_json::Map::new(),
            "batch-current",
            now,
        );
        assert_eq!(outcome["removed"], json!([]), "文件顺序 {names:?}：{outcome}");
        assert!(reading.join("releases").join("batch-shared").is_dir(), "{names:?}");
        let _ = fs::remove_dir_all(root);
    }
}

/// 把批次撤下时间写到 `<destination>/.publish-control/superseded/<key>.json`（测试用，等价于指针切换时的记录）。
fn backdate_superseded(root: &Path, key: &str, at: chrono::DateTime<chrono::Utc>) {
    let dir = crate::export_nas_library::normalize_nas_library_root(&destination(root))
        .join(".publish-control")
        .join("superseded");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(format!("{key}.json")),
        serde_json::to_vec(&json!({"schemaVersion": "SupersededV1", "key": key, "supersededAt": at.to_rfc3339()}))
            .unwrap(),
    )
    .unwrap();
}

/// 审计 A2 回归：旧卷 30 天前发布，此刻才被替换 → 撤下记录从此刻起算，批次不得立即删除；
/// 满 24 小时后才删除。
#[test]
fn a_long_published_paper_is_kept_for_24_hours_after_it_is_superseded() {
    let root = temp_root("paper-superseded-grace");
    ensure_app_dirs(&root).unwrap();
    let (a1, a2, a3) = (
        seed_part(&root, "sg-a1", "P1"),
        seed_part(&root, "sg-a2", "P2"),
        seed_part(&root, "sg-a3", "P3"),
    );
    let first = publish_with_paper(&root, &[&a1, &a2, &a3], Some(paper_spec(&a1, &a2, &a3, &[])))
        .expect("first paper must publish");
    let batch_a = first["paper"]["batchId"].as_str().unwrap().to_string();
    let paper_a = first["paper"]["paperId"].as_str().unwrap().to_string();
    // 旧卷发布于 30 天前（只回拨发布历史的 publishedAt，撤下记录尚不存在）。
    let history_path = crate::export_nas_library::normalize_nas_library_root(&destination(&root))
        .join(".publish-control")
        .join("papers")
        .join(format!("{paper_a}.json"));
    let mut record: Value = serde_json::from_slice(&fs::read(&history_path).unwrap()).unwrap();
    record["publishedAt"] = json!((chrono::Utc::now() - chrono::Duration::days(30)).to_rfc3339());
    fs::write(&history_path, serde_json::to_vec(&record).unwrap()).unwrap();

    let (b1, b2, b3) = (
        seed_part(&root, "sg-b1", "P1"),
        seed_part(&root, "sg-b2", "P2"),
        seed_part(&root, "sg-b3", "P3"),
    );
    let second = publish_with_paper(&root, &[&b1, &b2, &b3], Some(paper_spec(&b1, &b2, &b3, &[])))
        .expect("second paper must publish");
    assert_eq!(second["paper"]["ok"], json!(true), "{second}");
    let reading = reading_root(&root);
    assert!(
        reading.join("releases").join(&batch_a).is_dir(),
        "刚被替换的旧批次不得因发布于 30 天前而被删除：{second}"
    );
    assert!(second["cleanup"]["kept"]
        .as_array()
        .unwrap()
        .contains(&json!(format!("releases/{batch_a}"))), "{second}");

    let library_root = crate::export_nas_library::normalize_nas_library_root(&destination(&root));
    let manifest_value = manifest(&root);
    let current = manifest_value.as_object().unwrap();
    let before_grace = crate::nas_package_v2::cleanup_superseded_releases(
        &library_root, &reading, current, second["paper"]["batchId"].as_str().unwrap(),
        chrono::Utc::now() + chrono::Duration::hours(23),
    );
    assert!(reading.join("releases").join(&batch_a).is_dir(), "未满 24 小时保留：{before_grace}");
    let after_grace = crate::nas_package_v2::cleanup_superseded_releases(
        &library_root, &reading, current, second["paper"]["batchId"].as_str().unwrap(),
        chrono::Utc::now() + chrono::Duration::hours(25),
    );
    assert!(!reading.join("releases").join(&batch_a).exists(), "满 24 小时后删除：{after_grace}");
    let _ = fs::remove_dir_all(root);
}

/// P2-1：试卷题集合之外的 item_id 不得被当成试卷的一部分发出去，且整次不写任何东西。
#[test]
fn paper_publish_refuses_selected_items_that_are_not_part_of_the_paper() {
    let root = temp_root("paper-extra-item");
    ensure_app_dirs(&root).unwrap();
    let (p1, p2, p3) = (
        seed_part(&root, "ex-p1", "P1"),
        seed_part(&root, "ex-p2", "P2"),
        seed_part(&root, "ex-p3", "P3"),
    );
    let extra = seed_item(&root, "ex-extra", false, |_| {});
    let error = publish_with_paper(
        &root,
        &[&p1, &p2, &p3, &extra],
        Some(paper_spec(&p1, &p2, &p3, &[])),
    )
    .unwrap_err();
    assert!(error.starts_with("PAPER_ITEM_NOT_IN_PAPER"), "{error}");
    assert!(!reading_root(&root).join("manifest.js").exists());
    assert!(!current_paper_path(&root).exists());
    let _ = fs::remove_dir_all(root);
}

/// 生产命令层（`publish_items`）拒绝单题发布：契约 V1.1 第5条。
#[test]
fn the_publish_command_refuses_a_single_item_publish_without_a_paper() {
    let input: PublishItemsInput = serde_json::from_value(json!({
        "itemIds": ["any-item"],
        "destination": "/tmp/unused"
    }))
    .unwrap();
    assert_eq!(
        crate::nas_package_v2::require_paper_for_publish(&input).unwrap_err(),
        "PUBLISH_REQUIRES_PAPER"
    );
}
