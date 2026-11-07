//! Phase 6 NAS V2 package builder and two-phase publisher.
//!
//! The V1 exporter remains untouched.  This module is an opt-in publisher for
//! the runtime contract the paper actually uses (reading or listening) and
//! deliberately commits the discovery
//! manifest last, after the staged package has passed the same probe used by
//! the runtime validator.

use crate::artifact_store::JobArtifactPaths;
use crate::authoring_v2_commands::{
    validate_authoring_v2_publish_readiness, PublishMode, AUTHORING_V2_SHADOW_FILE,
};
use crate::export_artifacts::{build_wrapper, safe_exam_id, student_package_source};
use crate::export_nas_library::{nas_reading_exams_dir, normalize_nas_library_root};
use crate::listening_source_v1::{compile_exam_source_v2, CompiledExamSourceV2};
use crate::reading_runtime_v2::{
    run_student_loader_probe_with_files, safe_join_asset_path, ExamAssetManifestV2,
    ProbePackageFiles, StudentProbeReportV2,
};
use crate::schema::common::{canonical_json_bytes, canonical_json_bytes_js};
use crate::schema::IeltsAuthoringIRV2;
use crate::CommandResult;
use chrono::Utc;
use fs2::FileExt;
use rusqlite::OptionalExtension;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const ASSET_MANIFEST_FILE_NAME: &str = "asset-manifest.json";
const CONTROL_DIR_NAME: &str = ".publish-control";
const REPORT_DIR_NAME: &str = "publish/reports";
const BACKUP_DIR_NAME: &str = "backups";
const CURRENT_STUDENT_RUNTIME_VERSION: [u64; 3] = [0, 2, 0];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NasPackagePublishInput {
    #[serde(alias = "exportDir")]
    pub library_root: String,
    #[serde(alias = "runtimePath", alias = "sourceFile")]
    pub source_path: String,
    pub asset_root: Option<String>,
    pub exam_id: Option<String>,
    pub minimum_runtime_version: Option<String>,
    pub expected_manifest_sha256: Option<String>,
    pub fault: Option<String>,
    pub job_id: Option<String>,
    pub revision: Option<u64>,
}

fn validate_v2_export_binding(
    root: &Path,
    input: &NasPackagePublishInput,
    source_path: &Path,
    source_bytes: &[u8],
) -> CommandResult<()> {
    // A receipt names the runtime file after the contract it holds: reading keeps
    // `reading-source-v2.json`, a listening paper ships `listening-source-v1.json`.
    // Anything else is not an export we produced.
    let runtime_file_name = source_path.file_name().and_then(|name| name.to_str());
    let expected_runtime_file = match runtime_file_name {
        Some("reading-source-v2.json") => "reading-source-v2.json",
        Some("listening-source-v1.json") => "listening-source-v1.json",
        _ => {
            return Err("nas_package_v2_export_receipt_required:source_filename".to_string());
        }
    };
    let export_dir = source_path
        .parent()
        .ok_or_else(|| "nas_package_v2_export_receipt_required:source_parent".to_string())?;
    let manifest_path = export_dir.join("manifest-v2.json");
    let authoring_path = export_dir.join("authoring-ir-v2.json");
    let manifest_bytes = fs::read(&manifest_path).map_err(|error| {
        format!(
            "nas_package_v2_export_receipt_missing:{}:{error}",
            manifest_path.display()
        )
    })?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes).map_err(|error| {
        format!(
            "nas_package_v2_export_receipt_invalid:{}:{error}",
            manifest_path.display()
        )
    })?;
    if manifest.get("schemaVersion").and_then(Value::as_str) != Some("AuthoringV2ExportReceiptV1") {
        return Err("nas_package_v2_export_receipt_invalid:schema".to_string());
    }
    let files = manifest
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| "nas_package_v2_export_receipt_invalid:files".to_string())?;
    let expected_files = [
        "authoring-ir-v2.json",
        expected_runtime_file,
        "manifest-v2.json",
    ];
    if files.len() != expected_files.len()
        || expected_files
            .iter()
            .any(|expected| !files.iter().any(|file| file.as_str() == Some(*expected)))
    {
        return Err("nas_package_v2_export_receipt_invalid:files".to_string());
    }
    let authoring_bytes = fs::read(&authoring_path).map_err(|error| {
        format!(
            "nas_package_v2_export_receipt_missing:{}:{error}",
            authoring_path.display()
        )
    })?;
    let authoring_value: Value = serde_json::from_slice(&authoring_bytes).map_err(|error| {
        format!(
            "nas_package_v2_export_authoring_invalid:{}:{error}",
            authoring_path.display()
        )
    })?;
    let job_id = manifest
        .get("jobId")
        .and_then(Value::as_str)
        .ok_or_else(|| "nas_package_v2_export_receipt_invalid:jobId".to_string())?;
    let revision = manifest
        .get("revision")
        .and_then(Value::as_u64)
        .ok_or_else(|| "nas_package_v2_export_receipt_invalid:revision".to_string())?;
    if input.job_id.as_deref().is_some_and(|value| value != job_id)
        || input.revision.is_some_and(|value| value != revision)
    {
        return Err("nas_package_v2_export_receipt_input_mismatch".to_string());
    }
    if manifest.get("examId").and_then(Value::as_str)
        != authoring_value
            .pointer("/exam/examId")
            .and_then(Value::as_str)
        || manifest.get("sourceDocumentId").and_then(Value::as_str)
            != authoring_value
                .get("sourceDocumentId")
                .and_then(Value::as_str)
    {
        return Err("nas_package_v2_export_receipt_invalid:authoring_binding".to_string());
    }
    if manifest.get("reviewRequired") != Some(&Value::Bool(false)) {
        return Err("nas_package_v2_export_receipt_invalid:reviewRequired".to_string());
    }
    let expected_authoring_hash = manifest
        .get("authoringSha256")
        .and_then(Value::as_str)
        .ok_or_else(|| "nas_package_v2_export_receipt_invalid:authoringSha256".to_string())?;
    let expected_runtime_hash = manifest
        .get("runtimeSha256")
        .and_then(Value::as_str)
        .ok_or_else(|| "nas_package_v2_export_receipt_invalid:runtimeSha256".to_string())?;
    if sha256_hex(&authoring_bytes) != expected_authoring_hash
        || sha256_hex(source_bytes) != expected_runtime_hash
    {
        return Err("nas_package_v2_export_receipt_hash_mismatch".to_string());
    }

    if manifest.get("authoringSource").and_then(Value::as_str) != Some("canonical_ds") {
        let paths = JobArtifactPaths::for_job(root, job_id)?;
        let persisted_path = if revision == 0 {
            paths.job_dir.join(AUTHORING_V2_SHADOW_FILE)
        } else {
            paths.revision_path(revision)
        };
        let persisted_bytes = fs::read(&persisted_path).map_err(|error| {
            format!(
                "nas_package_v2_export_binding_missing:{}:{error}",
                persisted_path.display()
            )
        })?;
        let persisted_value: Value = serde_json::from_slice(&persisted_bytes).map_err(|error| {
            format!(
                "nas_package_v2_export_binding_invalid:{}:{error}",
                persisted_path.display()
            )
        })?;
        // `quality` is a derived report and is refreshed during every export, so
        // it is deliberately excluded from the persisted-content binding. The
        // receipt still authenticates the exact exported authoring bytes via
        // authoringSha256 above; this comparison only prevents exporting an old
        // or unrelated authoring revision from the same job.
        let persisted_binding = authoring_binding_value(&persisted_value);
        let exported_binding = authoring_binding_value(&authoring_value);
        let persisted_canonical =
            canonical_json_bytes(&persisted_binding).map_err(|error| error.to_string())?;
        let exported_canonical =
            canonical_json_bytes(&exported_binding).map_err(|error| error.to_string())?;
        if sha256_hex(&persisted_canonical) != sha256_hex(&exported_canonical)
            || persisted_binding != exported_binding
        {
            return Err("nas_package_v2_export_binding_detached".to_string());
        }
    }
    let bound_authoring: IeltsAuthoringIRV2 = serde_json::from_value(authoring_value.clone())
        .map_err(|error| format!("nas_package_v2_export_binding_invalid:authoring:{error}"))?;
    let bound_source = compile_exam_source_v2(&bound_authoring).map_err(|issues| {
        format!(
            "nas_package_v2_export_binding_invalid:authoring_compile:{}",
            serde_json::to_string(&issues).unwrap_or_default()
        )
    })?;
    let exported_source = CompiledExamSourceV2::parse(
        &serde_json::from_slice::<Value>(source_bytes)
            .map_err(|error| format!("nas_package_v2_export_binding_invalid:runtime:{error}"))?,
    )
    .map_err(|error| format!("nas_package_v2_export_binding_invalid:runtime:{error}"))?;
    if exported_source != bound_source {
        return Err("nas_package_v2_export_binding_detached:runtime".to_string());
    }
    // M1：DB 直通发布（manifest.authoringSource == "canonical_ds"）以 typed preflight
    // 复核证明（只查当前稿）；legacy 路径维持原门禁。证明绑定逐字节一致的语义不变。
    let proof = if manifest.get("authoringSource").and_then(Value::as_str) == Some("canonical_ds") {
        crate::authoring_v2_commands::check_publish_preflight(
            root,
            job_id,
            revision,
            &authoring_value,
        )
    } else {
        validate_authoring_v2_publish_readiness(root, job_id, revision, &authoring_value)?
    };
    if manifest.get("publishProof") != Some(&proof) {
        return Err("nas_package_v2_export_receipt_proof_mismatch".to_string());
    }
    Ok(())
}

fn authoring_binding_value(value: &Value) -> Value {
    let mut binding = value.clone();
    if let Some(object) = binding.as_object_mut() {
        object.remove("quality");
    }
    binding
}

#[derive(Debug, Clone)]
struct PackagePaths {
    library_root: PathBuf,
    reading_root: PathBuf,
    manifest_path: PathBuf,
    exam_path: PathBuf,
    resource_path: PathBuf,
    staging_root: PathBuf,
    staging_exam_path: PathBuf,
    staging_resource_path: PathBuf,
    staging_manifest_path: PathBuf,
    backup_root: PathBuf,
    journal_path: PathBuf,
    report_path: PathBuf,
    lock_path: PathBuf,
    lock_metadata_path: PathBuf,
    base_manifest_sha256: String,
}

#[derive(Debug, Clone)]
struct PackageReceipt {
    exam_id: String,
    runtime_sha256: String,
    asset_manifest_sha256: String,
    asset_count: usize,
    probe: StudentProbeReportV2,
    manifest_sha256: String,
}

/// 发布请求。`deny_unknown_fields`：旧的 `validationPolicy` 之类字段必须报错，
/// 不能被静默忽略成一次严格发布。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PublishItemsInput {
    pub item_ids: Vec<String>,
    pub destination: String,
    pub fault: Option<String>,
    /// 用户点击「发布」即为放行确认。前端总是带上它（`confirmedAt` = 点击时间）；
    /// 只有门禁结论确实不是 Ready 时才**被使用**并记录为显式放行。
    #[serde(default)]
    pub force: Option<ForceOverride>,
    /// 组卷：有它时，三篇阅读（P1/P2/P3）与听力题必须都在 `item_ids` 里、Part 相符；
    /// 全部学生端可加载时在清单提交之后写 `current-paper.json`（契约 CurrentPaperV1）。
    #[serde(default)]
    pub paper: Option<PaperSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ForceOverride {
    pub confirmed_at: String,
    pub acknowledged_reasons: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PaperSpec {
    pub reading: PaperReadingSlots,
    #[serde(default)]
    pub listening: Vec<PaperListeningSlot>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PaperReadingSlots {
    #[serde(rename = "P1")]
    pub p1: String,
    #[serde(rename = "P2")]
    pub p2: String,
    #[serde(rename = "P3")]
    pub p3: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PaperListeningSlot {
    pub part: String,
    pub item_id: String,
}

/// 当前试卷指针文件名（学生端只读这一个位置，与 manifest.js 同目录）。
pub(crate) const CURRENT_PAPER_FILE_NAME: &str = "current-paper.json";
const CURRENT_PAPER_SCHEMA: &str = "CurrentPaperV1";
const PAPER_HISTORY_DIR_NAME: &str = "papers";

/// 组卷校验（清单写入之前）：三篇阅读必须是 reading 且 Part 分别为 P1/P2/P3，
/// 听力题必须是 listening 且 Part 相符；所有试卷题都必须在本次 `item_ids` 里、不得重复。
fn validate_paper_spec(conn: &rusqlite::Connection, input: &PublishItemsInput) -> CommandResult<()> {
    use crate::library::repository::get_item;
    let Some(paper) = input.paper.as_ref() else {
        return Ok(());
    };
    let mut selected: BTreeSet<&str> = BTreeSet::new();
    for item_id in &input.item_ids {
        if !selected.insert(item_id.as_str()) {
            return Err(format!("PAPER_DUPLICATE_ITEM:{item_id}"));
        }
    }
    let mut slots: Vec<(&str, &str, &str)> = vec![
        ("reading", "P1", paper.reading.p1.as_str()),
        ("reading", "P2", paper.reading.p2.as_str()),
        ("reading", "P3", paper.reading.p3.as_str()),
    ];
    for slot in &paper.listening {
        slots.push(("listening", slot.part.as_str(), slot.item_id.as_str()));
    }
    let mut seen = BTreeSet::new();
    let mut seen_listening_parts = BTreeSet::new();
    for (modality, part, item_id) in &slots {
        if !seen.insert(*item_id) {
            return Err(format!("PAPER_DUPLICATE_ITEM:{item_id}"));
        }
        if *modality == "listening" && !seen_listening_parts.insert(*part) {
            return Err(format!("PAPER_DUPLICATE_PART:{part}"));
        }
    }
    // 试卷题集合与 item_ids 必须完全相等：多出来的题会被当成试卷的一部分发出去，
    // 缺的题则会让 current-paper 指向一套不完整的试卷。
    if let Some(extra) = selected.iter().find(|id| !seen.contains(*id)) {
        return Err(format!("PAPER_ITEM_NOT_IN_PAPER:{extra}"));
    }
    for (modality, part, item_id) in slots {
        if !selected.contains(item_id) {
            return Err(format!("PAPER_ITEM_NOT_SELECTED:{item_id}"));
        }
        let row = get_item(conn, item_id)?.ok_or_else(|| format!("ITEM_NOT_FOUND:{item_id}"))?;
        if row.modality != modality || row.part_label.as_deref() != Some(part) {
            return Err(format!("PAPER_PART_MISMATCH:{item_id}"));
        }
    }
    Ok(())
}

/// 清单提交之后写当前试卷。全部试卷题在本批以「学生端可加载」成功、且清单里有对应条目
/// 时才写；否则返回 `PAPER_INCOMPLETE` 与缺失清单，且**不写任何文件**。调用方保证在
/// `export.lock` 之内、清单原子替换之后调用；本函数的失败不回滚已提交的清单。
fn publish_current_paper(
    library_root: &Path,
    reading_root: &Path,
    manifest_path: &Path,
    batch_id: &str,
    paper: &PaperSpec,
    outcomes: &[Value],
) -> CommandResult<Value> {
    let committed = load_existing_manifest(manifest_path)?;
    let mut slots: Vec<(&str, &str, &str)> = vec![
        ("reading", "P1", paper.reading.p1.as_str()),
        ("reading", "P2", paper.reading.p2.as_str()),
        ("reading", "P3", paper.reading.p3.as_str()),
    ];
    for slot in &paper.listening {
        slots.push(("listening", slot.part.as_str(), slot.item_id.as_str()));
    }
    let script_prefix = format!("./releases/{batch_id}/");
    let mut missing = Vec::new();
    let mut reading = Vec::new();
    let mut listening = Vec::new();
    for (modality, part, item_id) in slots {
        let loadable = outcomes.iter().find(|outcome| {
            outcome.get("itemId").and_then(Value::as_str) == Some(item_id)
                && outcome.get("ok").and_then(Value::as_bool) == Some(true)
                && outcome.get("studentLoadable").and_then(Value::as_bool) == Some(true)
        });
        // V1.1 第2条：指针自包含。`entry` 原样拷贝清单里那条完整条目；顶层
        // script/scriptSha256/runtimeSha256 必须与 entry 中对应值一致（由这里统一生成）。
        let entry = loadable
            .and_then(|outcome| outcome.get("examId").and_then(Value::as_str))
            .and_then(|exam_id| committed.get(exam_id).map(|entry| (exam_id, entry)))
            .and_then(|(exam_id, entry)| {
                let script = entry.get("script").and_then(Value::as_str)?;
                let script_sha = entry.pointer("/checksums/scriptSha256").and_then(Value::as_str)?;
                let runtime_sha = entry.pointer("/checksums/runtimeSha256").and_then(Value::as_str)?;
                if !script.starts_with(&script_prefix) {
                    return None;
                }
                let title = entry.get("title").and_then(Value::as_str).unwrap_or_default();
                Some(json!({
                    "part": part,
                    "examId": exam_id,
                    "title": title,
                    "script": script,
                    "scriptSha256": script_sha,
                    "runtimeSha256": runtime_sha,
                    "entry": entry,
                }))
            });
        match entry {
            Some(entry) if modality == "reading" => reading.push(entry),
            Some(entry) => listening.push(entry),
            None => missing.push(item_id.to_string()),
        }
    }
    if !missing.is_empty() {
        return Ok(json!({"ok": false, "code": "PAPER_INCOMPLETE", "missing": missing}));
    }
    let paper_id = Uuid::new_v4().simple().to_string();
    let published_at = Utc::now().to_rfc3339();
    let document = json!({
        "schemaVersion": CURRENT_PAPER_SCHEMA,
        "paperId": paper_id,
        "publishedAt": published_at,
        "batchId": batch_id,
        "reading": reading,
        "listening": listening,
    });
    let bytes = canonical_json_bytes(&document).map_err(|error| error.to_string())?;
    // 历史留档先写：学生端不读它，但指针指向的 paperId 必须有可查的记录。
    let history = library_root
        .join(CONTROL_DIR_NAME)
        .join(PAPER_HISTORY_DIR_NAME)
        .join(format!("{paper_id}.json"));
    write_synced_file(&history, &bytes)?;
    // 指针：临时文件 + 原子替换。失败则清掉临时文件，旧指针保持不变。
    let temporary = reading_root.join(format!(".current-paper-{paper_id}.tmp"));
    write_synced_file(&temporary, &bytes)?;
    if let Err(error) = atomic_replace_file(&temporary, &reading_root.join(CURRENT_PAPER_FILE_NAME)) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(json!({"ok": true, "paperId": paper_id, "publishedAt": published_at, "batchId": batch_id,
        "reading": document["reading"], "listening": document["listening"]}))
}

/// 读取 NAS 上的当前试卷指针；不存在返回 `null`。
pub(crate) fn get_current_paper_core(destination: &str) -> CommandResult<Value> {
    let library_root = normalize_nas_library_root(&absolute_path("library_root", destination)?);
    let path = nas_reading_exams_dir(&library_root).join(CURRENT_PAPER_FILE_NAME);
    if !path.is_file() {
        return Ok(Value::Null);
    }
    let text = fs::read_to_string(&path).map_err(|error| error.to_string())?;
    serde_json::from_str::<Value>(&text).map_err(|error| format!("CURRENT_PAPER_INVALID:{error}"))
}


/// 门禁类失败（编译未通过 / 结构非法）：P2 起 strict 发布里这是**条目级**可降级失败，
/// 不是批次级环境故障。IO / 资产 / 清单 CAS 等错误不在此列，照旧中断整批。
fn is_publish_gate_failure(error: &str) -> bool {
    error.starts_with("authoring_v2_export_blocked:")
        || error.starts_with("authoring_v2_export_compile_blocked:")
        || error.starts_with("AUTHORING_SCHEMA_INVALID")
}

/// 生产命令层的门槛（契约 V1.1 第5条）：`publish_items` 只接受组卷发布，单题发布已取消。
pub(crate) fn require_paper_for_publish(input: &PublishItemsInput) -> CommandResult<()> {
    if input.paper.is_none() {
        return Err("PUBLISH_REQUIRES_PAPER".to_string());
    }
    Ok(())
}

pub(crate) fn publish_items_core(root: &Path, input: PublishItemsInput) -> CommandResult<Value> {
    use crate::library::repository::{get_canonical_ds, open_library_connection};
    if input.item_ids.is_empty() {
        return Err("PUBLISH_ITEMS_REQUIRED".to_string());
    }
    // 放行确认在任何写入之前校验：结构不合法已由 serde 拒绝；时间戳不合法同样明确
    // 报错，绝不静默降级成严格发布（那会让用户以为自己的点击生效了）。
    let mode = match input.force.as_ref() {
        None => PublishMode::Strict,
        Some(force) => {
            chrono::DateTime::parse_from_rfc3339(force.confirmed_at.trim())
                .map_err(|error| format!("PUBLISH_FORCE_OVERRIDE_INVALID:confirmedAt:{error}"))?;
            PublishMode::Forced {
                confirmed_at: force.confirmed_at.trim().to_string(),
                acknowledged_reasons: force.acknowledged_reasons.clone(),
            }
        }
    };
    let ids: BTreeSet<_> = input.item_ids.iter().collect();
    for id in &ids {
        crate::library::migration::migrate_single_item(root, id)?;
    }
    let mut conn = open_library_connection(root)?;
    let transaction = conn.transaction().map_err(|error| error.to_string())?;
    // P2-3：Part 标签与试卷约定在快照事务内重新核对，与读取权威稿处于同一一致视图。
    validate_paper_spec(&transaction, &input)?;
    let snapshots = ids
        .iter()
        .map(|id| {
            let (ds, version) = get_canonical_ds(&transaction, id)?
                .ok_or_else(|| format!("ITEM_DS_NOT_SEEDED:{id}"))?;
            Ok(((*id).clone(), ds, version))
        })
        .collect::<CommandResult<Vec<_>>>()?;
    transaction.commit().map_err(|error| error.to_string())?;

    let library_root =
        normalize_nas_library_root(&absolute_path("library_root", &input.destination)?);
    let reading_root = nas_reading_exams_dir(&library_root);
    let paths = make_paths(&library_root, &reading_root, "batch")?;
    fs::create_dir_all(paths.lock_path.parent().unwrap()).map_err(|error| error.to_string())?;
    fs::create_dir_all(&reading_root).map_err(|error| error.to_string())?;
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&paths.lock_path)
        .map_err(|error| error.to_string())?;
    lock.try_lock_exclusive()
        .map_err(|error| format!("nas_package_v2_lock_busy:{error}"))?;
    recover_incomplete_transactions(&paths)?;
    let base_hash = manifest_sha256(&paths.manifest_path)?;
    // 组卷发布是「替换」语义：新清单只从空开始构建，旧题整体撤下。
    // 无 paper 的老路径保持累积（别处仍在使用），CAS 基线两者都取当前清单哈希。
    let mut manifest = if input.paper.is_some() {
        Map::new()
    } else {
        load_existing_manifest(&paths.manifest_path)?
    };
    let batch_id = Uuid::new_v4().simple().to_string();
    write_lock_metadata(&paths, &batch_id)?;
    let staging = reading_root.join(format!(".batch-staging-{batch_id}"));
    let release = reading_root.join("releases").join(&batch_id);
    let backup_dir = library_root
        .join(CONTROL_DIR_NAME)
        .join(BACKUP_DIR_NAME)
        .join(format!("batch-{batch_id}"));
    // 提交点标志：清单替换成功即为「包已对学生可见」。此后任何失败都不得回滚资源。
    let mut manifest_committed = false;
    // 组卷缺题：未提交任何东西时，这里保存要返回的结果（见函数末尾）。
    let mut paper_incomplete: Option<Value> = None;
    let mut publications: Vec<ItemPublication> = Vec::new();
    let mut result = (|| -> CommandResult<Value> {
        fs::create_dir_all(&staging).map_err(|error| error.to_string())?;
        let mut outcomes = Vec::new();
        // 全部 examId（重复检查是硬性不变量，放行模式同样适用）。
        // 资源与脚本都随 staging 一起整体重命名进 `releases/<batchId>/`，不再写根级资源目录。
        let mut exam_ids = BTreeSet::new();
        let mut forced_items = Vec::new();
        // 被门禁挡下的条目（编译未通过）：本条失败，不影响批次其余条目。
        let mut failed_items: Vec<Value> = Vec::new();
        for (index, (item_id, ds, version)) in snapshots.iter().enumerate() {
            // ── 导出 ──────────────────────────────────────────────────────
            // 导出自己的失败**一律中断整批**，放行也不例外。门禁不是 Ready 的稿在这里
            // 已经由 `export_authoring_snapshot_with_mode` 自己降级成 `studentLoadable=false`
            // （编译不过、答案没闭合同理），所以走到 `?` 的只剩 IO 与安全类硬错误：
            // 资产缺失/越界、examId 路径不安全、目录不可写、清单 CAS、锁。把这类错误吞成
            // 「这条没发出去」，等于替用户接受环境故障——而他要的是「发布」。
            let materialized = match crate::authoring_v2_commands::export_authoring_snapshot_with_mode(
                root,
                crate::authoring_v2_commands::ExportAuthoringV2Input {
                    job_id: item_id.clone(),
                    export_dir: staging.join("snapshots").to_string_lossy().into_owned(),
                    revision: None,
                    authoring: Some(ds.clone()),
                    edit_version: Some(*version as u64),
                },
                &mode,
            ) {
                Ok(materialized) => materialized,
                Err(error) if is_publish_gate_failure(&error) => {
                    // P2 门禁收敛：strict 模式（前端不再带 force）下，编译不过的条目
                    // 是**这条题自己的内容问题**——与「包检查失败」同级的可降级失败：
                    // 这一条如实失败，整批照常发布，绝不静默吞掉也不中断批次。
                    eprintln!("[publish] gate blocked {item_id}: {error}");
                    failed_items.push(json!({
                        "itemId": item_id,
                        "ok": false,
                        "message": "这道题还没达到发布条件：编译未通过。请在编辑器里处理列出的问题后重试。"
                    }));
                    continue;
                }
                Err(error) => return Err(error),
            };
            let receipt = materialized.get("receipt").cloned().unwrap_or(Value::Null);
            let publish_override = receipt.get("publishOverride").cloned();
            let student_loadable = receipt
                .get("studentLoadable")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let verdict = receipt
                .get("publishVerdict")
                .cloned()
                .ok_or("PUBLISH_VERDICT_MISSING")?;
            let reasons = publish_override
                .as_ref()
                .and_then(|value| value.get("reasons").cloned())
                .unwrap_or_else(|| json!([]));
            // 这条题的 examId：取授权快照 receipt 里那份。它与编译出来的 runtime、
            // 清单条目共用同一个身份，所以降级与正常发布用同一个值，不做二次推导。
            let exam_id = safe_exam_id(&json!({
                "examId": materialized.get("examId").cloned().unwrap_or(Value::Null)
            }))?;
            // ── 包检查（组装 + 学生加载器探针）──────────────────────────────
            // 这是唯一在放行模式下允许「只降级这一条」的失败面：包组装出来学生端读不了，
            // 是**这条题自己的内容问题**，不是环境故障。判据是白名单（见
            // `is_degradable_package_check_failure`），未知错误码默认中断整批。
            // 严格发布不降级——用户没说「发一半也行」，后端不该替他决定。
            let package_check: CommandResult<Option<(StagedPackage, usize)>> = if student_loadable {
                (|| {
                    let source_path = PathBuf::from(
                        receipt
                            .get("runtimePath")
                            .and_then(Value::as_str)
                            .ok_or("PUBLISH_RUNTIME_MISSING")?,
                    );
                    let source_value: Value = crate::util::read_json(&source_path)?;
                    let source = CompiledExamSourceV2::parse(&source_value)
                        .map_err(|error| error.to_string())?;
                    let mut staged_paths = paths.clone();
                    staged_paths.staging_root = staging.clone();
                    staged_paths.staging_exam_path = staging.join(format!("{exam_id}.js"));
                    staged_paths.staging_resource_path = staging.join("resources").join(&exam_id);
                    let package_input = NasPackagePublishInput {
                        library_root: input.destination.clone(),
                        source_path: source_path.to_string_lossy().into_owned(),
                        asset_root: None,
                        exam_id: Some(exam_id.clone()),
                        minimum_runtime_version: None,
                        expected_manifest_sha256: None,
                        fault: None,
                        job_id: None,
                        revision: None,
                    };
                    let staged = stage_package_files(
                        &package_input,
                        &source,
                        &source_value,
                        &source_path,
                        &staged_paths,
                    )?;
                    Ok(Some((staged, source.assets().len())))
                })()
            } else {
                // 门禁（或编译、答案闭合）已经判定这条进不了学生端：不碰包，
                // 直接走 authoring-only。这不是失败，没有 packageError。
                Ok(None)
            };
            let record_id = Uuid::new_v4().simple().to_string();
            let attempt = match package_check {
                Ok(None) => ItemAttempt::AuthoringOnly {
                    publish_override,
                    verdict,
                    reasons,
                    package_error: None,
                },
                Ok(Some((staged, asset_count))) => ItemAttempt::Packaged {
                    staged,
                    exam_id: exam_id.clone(),
                    asset_count,
                    publish_override,
                    verdict,
                    reasons,
                },
                Err(error)
                    if matches!(mode, PublishMode::Forced { .. })
                        && is_degradable_package_check_failure(&error) =>
                {
                    // 包检查没过：这条降级为 authoring-only。授权快照**已经**写进
                    // staging/snapshots（随 release 一起提交），用户能继续编辑/导出；
                    // 只是不进学生清单——绝不把一个学生端会拒绝的 runtime 发出去。
                    // 门禁结论用 receipt 里那份真实的，不重算也不改写。
                    // 打了一半的包不留：它不属于任何清单条目。
                    let _ = fs::remove_dir_all(staging.join("resources").join(&exam_id));
                    let _ = fs::remove_file(staging.join(format!("{exam_id}.js")));
                    ItemAttempt::AuthoringOnly {
                        publish_override,
                        verdict,
                        reasons,
                        package_error: Some(error),
                    }
                }
                Err(error) => return Err(error),
            };
            match attempt {
                ItemAttempt::AuthoringOnly {
                    publish_override,
                    verdict,
                    reasons,
                    package_error,
                } => {
                    let forced = publish_override.is_some();
                    // 授权快照已写在 staging/snapshots（随 release 一起提交）；不进学生清单。
                    if !exam_ids.insert(exam_id.clone()) {
                        return Err(format!("PUBLISH_DUPLICATE_EXAM_ID:{exam_id}"));
                    }
                    let mut entry = json!({"itemId": item_id, "examId": exam_id, "studentLoadable": false,
                        "verdictStatus": verdict.get("status").cloned().unwrap_or(Value::Null),
                        "confirmedAt": publish_override.as_ref().and_then(|value| value.get("confirmedAt").cloned())});
                    let mut outcome = json!({"itemId": item_id, "ok": true, "examId": exam_id, "editVersion": version,
                        "forced": forced, "studentLoadable": false, "publishRecordId": record_id,
                        "verdictStatus": verdict.get("status").cloned().unwrap_or(Value::Null)});
                    if let Some(package_error) = package_error.as_ref() {
                        // 审计要能分清「门禁放行」（用户确认过的问题）与「包检查没过」
                        // （这条题根本装不进学生包）。原因码原样带上，不做人话化。
                        entry["packageError"] = json!(package_error);
                        outcome["packageError"] = json!(package_error);
                    }
                    forced_items.push(entry);
                    outcomes.push(outcome);
                    publications.push(ItemPublication {
                        item_id: item_id.clone(),
                        edit_version: *version,
                        record_id,
                        forced,
                        student_loadable: false,
                        verdict,
                        reasons,
                    });
                }
                ItemAttempt::Packaged {
                    mut staged,
                    exam_id,
                    asset_count,
                    publish_override,
                    verdict,
                    reasons,
                } => {
                    let forced = publish_override.is_some();
                    if !exam_ids.insert(exam_id.clone()) {
                        return Err(format!("PUBLISH_DUPLICATE_EXAM_ID:{exam_id}"));
                    }
                    // V1.1 第1条：脚本与资源都在 `releases/<batchId>/` 下，批次内不可变。
                    let script_relative = staged.entry["script"]
                        .as_str()
                        .ok_or("PUBLISH_PATH_MISSING")?
                        .trim_start_matches("./")
                        .to_string();
                    staged.entry["script"] =
                        json!(format!("./releases/{batch_id}/{script_relative}"));
                    let batch_resources = format!("./releases/{batch_id}/resources/{exam_id}");
                    staged.entry["resourcesBase"] = json!(batch_resources);
                    staged.entry["assetManifest"] =
                        json!(format!("{batch_resources}/{ASSET_MANIFEST_FILE_NAME}"));
                    // 学生端忽略未知的清单条目字段；放行标记留在条目上供审计。
                    if let Some(publish_override) = publish_override.as_ref() {
                        staged.entry["publishOverride"] = publish_override.clone();
                        forced_items.push(
                            json!({"itemId": item_id, "examId": exam_id, "studentLoadable": true,
                            "verdictStatus": verdict.get("status").cloned().unwrap_or(Value::Null),
                            "confirmedAt": publish_override.get("confirmedAt").cloned()}),
                        );
                    }
                    manifest.insert(exam_id.clone(), staged.entry);
                    outcomes.push(json!({"itemId": item_id, "ok": true, "examId": exam_id,
                        "editVersion": version, "manifestPath": paths.manifest_path, "assetCount": asset_count,
                        "forced": forced, "studentLoadable": true, "publishRecordId": record_id,
                        "verdictStatus": verdict.get("status").cloned().unwrap_or(Value::Null)}));
                    publications.push(ItemPublication {
                        item_id: item_id.clone(),
                        edit_version: *version,
                        record_id,
                        forced,
                        student_loadable: true,
                        verdict,
                        reasons,
                    });
                }
            }
            if input.fault.as_deref() == Some(&format!("after_item_{}", index + 1)) {
                return Err("PUBLISH_BATCH_INTERRUPTED".to_string());
            }
        }
        // 组卷替换：试卷有任何一题不能被学生端加载，就整次不提交清单。否则新清单会撤下
        // 旧试卷的题，而旧指针（仍是学生端当前试卷）随之失效。本批什么都不写，旧试卷保持不变。
        if let Some(paper) = input.paper.as_ref() {
            let missing = paper_missing_items(paper, &outcomes);
            if !missing.is_empty() {
                paper_incomplete = Some(json!({
                    "status": "paper_incomplete",
                    "destination": input.destination,
                    "succeeded": [],
                    "failed": failed_items.clone(),
                    "paper": {"ok": false, "code": "PAPER_INCOMPLETE", "missing": missing},
                }));
                return Err("PAPER_INCOMPLETE".to_string());
            }
        }
        manifest.remove("_meta");
        manifest.insert("_meta".to_string(), json!({"schemaVersion": "ReadingExamManifestV2",
            "assetCount": manifest.len(), "generatedAt": Utc::now().to_rfc3339(), "batchId": batch_id,
            "forcedItems": forced_items}));
        let candidate = staging.join("manifest.js");
        let candidate_bytes = format!(
            "window.__READING_EXAM_MANIFEST__ = {};\n",
            serde_json::to_string_pretty(&manifest).map_err(|error| error.to_string())?
        )
        .into_bytes();
        write_synced_file(&candidate, &candidate_bytes)?;
        // 提交点判定只能靠磁盘事实。提交用的是 `atomic_replace_file`（移动），
        // 提交后 release 里已无候选清单，所以必须先把候选清单的内容哈希记进状态，
        // 崩溃恢复才能凭「线上清单哈希 == 待提交哈希」认出包已经生效。
        let candidate_manifest_sha256 = sha256_hex(&candidate_bytes);
        // 状态文件只是诊断与恢复的辅助事实：V1.1 起资源不再移动到根级，因此 `exams` 恒为空；
        // 旧版本遗留的进行中事务仍由 recover_interrupted_batches 的兼容分支处理。
        let write_batch_state = |committed: bool| -> CommandResult<()> {
            let state = json!({
                "schemaVersion": "NasBatchBackupStateV1",
                "manifestCommitted": committed,
                "hadManifest": paths.manifest_path.is_file(),
                "pendingManifestSha256": candidate_manifest_sha256,
                "exams": [],
            });
            fs::create_dir_all(&backup_dir).map_err(|error| error.to_string())?;
            write_synced_file(
                &backup_dir.join("state.json"),
                &canonical_json_bytes(&state).map_err(|error| error.to_string())?,
            )
        };
        // 保留清单基线。`atomic_replace_file` 本身是原子替换，但提交点判定与
        // 「未提交却已替换」的还原都不应依赖状态文件的写入时机，因此留一份字节备份。
        fs::create_dir_all(&backup_dir).map_err(|error| error.to_string())?;
        if paths.manifest_path.is_file() {
            copy_file_verified(&paths.manifest_path, &backup_dir.join("manifest.js"))
                .map_err(|error| format!("PUBLISH_BATCH_MANIFEST_BACKUP:{error}"))?;
        }
        write_batch_state(false)?;
        verify_manifest_compare_and_swap(&paths.manifest_path, &base_hash)?;
        fs::create_dir_all(release.parent().unwrap()).map_err(|error| error.to_string())?;
        // 资源与脚本已经在 staging 内完整就位（`releases/<batchId>/resources/<examId>/`），
        // 整目录一次重命名进 release；之后任何失败都只删除本批新建的 release，不触碰线上文件。
        fs::rename(&staging, &release).map_err(|error| error.to_string())?;
        if input.fault.as_deref() == Some("before_manifest") {
            return Err("PUBLISH_BATCH_INTERRUPTED".to_string());
        }
        // 第一个提交点：清单原子替换。
        atomic_replace_file(&release.join("manifest.js"), &paths.manifest_path)?;
        // 提交点已过：新清单引用的 scripts 与 resources 都已在批次目录就位。状态文件此刻
        // 只是诊断信息，写失败不得升级为回滚——回滚会删掉刚生效的包。
        manifest_committed = true;
        if let Err(error) = write_batch_state(true) {
            eprintln!("[publish] batch committed state write failed: {error}");
        }
        // 第二个提交点（学生端考试路径的唯一读取点）：当前试卷指针。清单已提交之后才写；
        // 失败不回滚清单，但结果顶层必须是「未完成」，且不做清理（旧指针仍引用旧批次）。
        let paper_result = input.paper.as_ref().map(|paper| {
            publish_current_paper(
                &library_root,
                &reading_root,
                &paths.manifest_path,
                &batch_id,
                paper,
                &outcomes,
            )
            .unwrap_or_else(|error| {
                eprintln!("[publish] current paper write failed: {error}");
                json!({"ok": false, "code": "PAPER_WRITE_FAILED", "message": error})
            })
        });
        let paper_written = paper_result
            .as_ref()
            .and_then(|value| value.get("ok"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // 清理旧批次：只在指针成功之后执行；它永不删除当前磁盘指针引用的批次。
        let cleanup = paper_written.then(|| {
            cleanup_superseded_releases(&library_root, &reading_root, &manifest, &batch_id, Utc::now())
        });
        let succeeded = outcomes;
        let mut value =
            json!({"destination": input.destination, "succeeded": succeeded, "failed": failed_items});
        let pointer_failed = input.paper.is_some() && !paper_written;
        if let Some(paper_result) = paper_result {
            value["paper"] = paper_result;
        }
        if let Some(cleanup) = cleanup {
            value["cleanup"] = cleanup;
        }
        // 顶层结论：清单已提交但指针未更新时，学生端仍是旧试卷，必须明确标为未完成，
        // 前端不得把 succeeded 当成功展示。
        value["status"] = json!(if pointer_failed {
            "paper_pointer_stale"
        } else {
            "published"
        });
        value["batchId"] = json!(batch_id);
        Ok(value)
    })();
    if result.is_err() {
        if manifest_committed {
            eprintln!("[publish] batch failed after manifest commit; leaving the committed package in place");
        } else {
            // 清单未替换：线上文件从未被触碰，只删除本批新建的 staging / release / 备份。
            let _ = fs::remove_dir_all(&staging);
            let _ = fs::remove_dir_all(&release);
            let _ = fs::remove_dir_all(&backup_dir);
        }
    } else {
        let _ = fs::remove_dir_all(&backup_dir);
        // ── 提交后的记录与状态：清单（与指针）已提交，这里的任何失败都**不得返回 Err**。
        // 返回 Err 会让前端显示"发布失败"，而包其实已经对学生生效。失败一律作为 `warnings`
        // 随成功结果返回，前端据此如实提示。
        //
        //   * CAS 未匹配（快照之后条目又被编辑）：确凿的版本漂移，记为 status_drift。
        //   * 状态提交的真数据库错误：记为 warning。
        //   * 发布记录写失败：记为 warning。
        let mut post_commit_warnings: Vec<Value> = Vec::new();
        let status_drift = match commit_published_status(&conn, &publications) {
            Ok(drift) => drift,
            Err(error) => {
                post_commit_warnings.push(json!({"reason": "status_commit_failed", "error": error}));
                Vec::new()
            }
        };
        if let Err(error) = write_publish_records(
            &conn,
            &publications,
            &status_drift,
            &batch_id,
            &input.destination,
        ) {
            post_commit_warnings.push(json!({"reason": "publish_record_write_failed", "error": error}));
        }
        if !status_drift.is_empty() {
            post_commit_warnings.push(json!({"reason": "status_drift", "items": status_drift}));
        }
        let _ = fs::remove_file(&paths.lock_metadata_path);
        // ── 题库保存：发布**已提交且状态已确认**之后，冻结最终版证据并删除原文件与
        // 过程文件。两步都不得让发布失败或回滚（包已对学生可见）；结果逐题如实报告。
        // 冻结失败时**不**删除：没有冻结证据，删除 shadow 会让这道题再也无法正常发布。
        let mut final_versions = BTreeMap::new();
        for publication in &publications {
            let Some((_, ds, _)) = snapshots
                .iter()
                .find(|(id, _, _)| id == &publication.item_id)
            else {
                continue;
            };
            let report = match crate::library::final_version::freeze_final_version(
                &conn,
                root,
                &publication.item_id,
                publication.edit_version,
                &publication.record_id,
                ds,
            ) {
                Ok(_) => {
                    let purge = crate::library::final_version::purge_source_artifacts(
                        root,
                        &publication.item_id,
                        ds,
                    );
                    let marked = crate::library::final_version::mark_source_purged(
                        &conn,
                        &publication.item_id,
                        &purge,
                    );
                    let failed = purge
                        .get("failed")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len);
                    if failed > 0 {
                        eprintln!(
                            "[publish] source purge for {} left {failed} entries",
                            publication.item_id
                        );
                    }
                    json!({"frozen": true, "sourcePurged": marked.is_ok(), "purge": purge,
                        "error": marked.err()})
                }
                Err(error) => {
                    eprintln!(
                        "[publish] final version freeze failed for {}: {error}",
                        publication.item_id
                    );
                    json!({"frozen": false, "sourcePurged": false, "error": error})
                }
            };
            final_versions.insert(publication.item_id.clone(), report);
        }
        if let Ok(value) = result.as_mut() {
            for outcome in value
                .get_mut("succeeded")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                let item_id = outcome
                    .get("itemId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if let Some(report) = final_versions.remove(&item_id) {
                    outcome["finalVersion"] = report;
                }
            }
            if !post_commit_warnings.is_empty() {
                for warning in &post_commit_warnings {
                    eprintln!("[publish] post-commit warning: {warning}");
                }
                value["warnings"] = json!(post_commit_warnings);
                if value.get("status").and_then(Value::as_str) == Some("published") {
                    value["status"] = json!("published_with_warnings");
                }
            }
        }
    }
    let _ = fs::remove_file(&paths.lock_metadata_path);
    match (result, paper_incomplete) {
        // 组卷缺题：本批没有提交任何东西，作为正常结果返回（旧试卷保持不变）。
        (Err(_), Some(incomplete)) => Ok(incomplete),
        (result, _) => result,
    }
}

/// 组卷的试卷题里，本批没有以「学生端可加载」成功的 examId（按 item id 列出）。
fn paper_missing_items(paper: &PaperSpec, outcomes: &[Value]) -> Vec<String> {
    let mut slots: Vec<&str> = vec![paper.reading.p1.as_str(), paper.reading.p2.as_str(), paper.reading.p3.as_str()];
    slots.extend(paper.listening.iter().map(|slot| slot.item_id.as_str()));
    slots
        .into_iter()
        .filter(|item_id| {
            !outcomes.iter().any(|outcome| {
                outcome.get("itemId").and_then(Value::as_str) == Some(item_id)
                    && outcome.get("ok").and_then(Value::as_bool) == Some(true)
                    && outcome.get("studentLoadable").and_then(Value::as_bool) == Some(true)
            })
        })
        .map(str::to_string)
        .collect()
}

/// 撤下的旧批次与旧题资源的宽限期：发布时间距今不足此时长的保留（正在考试的学生仍锁定旧脚本）。
const SUPERSEDED_GRACE_HOURS: i64 = 24;

/// 清理被替换掉的 `releases/<batchId>/` 与 `resources/<examId>/`。
///
/// 规则（与契约 V1 一致）：
/// - 被当前清单（即当前试卷）引用的 examId 资源、当前批次目录：永不删除。
/// - 其余对象按归属时间判定：归属 = `.publish-control/papers/*.json` 中含它的记录的 `publishedAt`
///   （批次目录按 `batchId`，资源目录按 examId 所在的试卷记录）。归属时间距 `now` ≥ 24 小时删除，否则保留。
/// - 找不到归属记录时退回目录 mtime：≥ 24 小时删除，否则保留。
/// 任何单项失败只记入 `failed`，不影响调用方的发布结果。
pub(crate) fn cleanup_superseded_releases(
    library_root: &Path,
    reading_root: &Path,
    manifest: &Map<String, Value>,
    current_batch_id: &str,
    now: chrono::DateTime<Utc>,
) -> Value {
    cleanup_superseded_releases_with(library_root, reading_root, manifest, current_batch_id, now, &|path| {
        fs::remove_dir_all(path)
    })
}

/// 把 `key` 的发布时间更新为「已知的最晚时间」。
fn newest_published(
    map: &mut BTreeMap<String, chrono::DateTime<Utc>>,
    key: &str,
    published: chrono::DateTime<Utc>,
) {
    map.entry(key.to_string())
        .and_modify(|current| {
            if published > *current {
                *current = published;
            }
        })
        .or_insert(published);
}

/// 磁盘上当前指针引用的批次号集合：顶层 `batchId` 与每条的 `releases/<batchId>/` 前缀。
/// 指针缺失或损坏时返回空集合（此时清理仍只删除超过宽限期的批次，且指针写入路径不会走到这里）。
fn current_pointer_batch_ids(reading_root: &Path) -> BTreeSet<String> {
    let mut batches = BTreeSet::new();
    let Ok(bytes) = fs::read(reading_root.join(CURRENT_PAPER_FILE_NAME)) else {
        return batches;
    };
    let Ok(pointer) = serde_json::from_slice::<Value>(&bytes) else {
        return batches;
    };
    if let Some(batch) = pointer.get("batchId").and_then(Value::as_str) {
        batches.insert(batch.to_string());
    }
    for group in ["reading", "listening"] {
        for item in pointer.get(group).and_then(Value::as_array).into_iter().flatten() {
            let script = item
                .get("script")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if let Some(rest) = script.strip_prefix("./releases/") {
                if let Some((batch, _)) = rest.split_once('/') {
                    batches.insert(batch.to_string());
                }
            }
        }
    }
    batches
}

/// 同 `cleanup_superseded_releases`，删除动作可注入（测试用它模拟删除失败）。
pub(crate) fn cleanup_superseded_releases_with(
    library_root: &Path,
    reading_root: &Path,
    manifest: &Map<String, Value>,
    current_batch_id: &str,
    now: chrono::DateTime<Utc>,
    remove_dir: &dyn Fn(&Path) -> std::io::Result<()>,
) -> Value {
    let mut batch_published: BTreeMap<String, chrono::DateTime<Utc>> = BTreeMap::new();
    let mut exam_published: BTreeMap<String, chrono::DateTime<Utc>> = BTreeMap::new();
    let history_dir = library_root.join(CONTROL_DIR_NAME).join(PAPER_HISTORY_DIR_NAME);
    let mut failed = Vec::new();
    for entry in fs::read_dir(&history_dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let record = fs::read(&path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).map_err(|error| error.to_string()));
        let record = match record {
            Ok(record) => record,
            Err(error) => {
                failed.push(json!({"path": path.to_string_lossy(), "error": error}));
                continue;
            }
        };
        let Some(published) = record
            .get("publishedAt")
            .and_then(Value::as_str)
            .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
            .map(|time| time.with_timezone(&Utc))
        else {
            continue;
        };
        // P1-2：归属取「最近」的发布时间。同一批次/题目可能出现在多份历史记录里，
        // 目录遍历顺序不稳定，因此只在时间更晚时覆盖，结果与文件名顺序无关。
        if let Some(batch) = record.get("batchId").and_then(Value::as_str) {
            newest_published(&mut batch_published, batch, published);
        }
        for group in ["reading", "listening"] {
            for item in record.get(group).and_then(Value::as_array).into_iter().flatten() {
                if let Some(exam_id) = item.get("examId").and_then(Value::as_str) {
                    newest_published(&mut exam_published, exam_id, published);
                }
            }
        }
    }
    // 当前磁盘指针引用的批次（无论是刚写入的新指针，还是仍然有效的旧指针）永不删除。
    let pointer_batches = current_pointer_batch_ids(reading_root);

    let mut removed = Vec::new();
    let mut kept = Vec::new();
    let grace = chrono::Duration::hours(SUPERSEDED_GRACE_HOURS);
    let mut sweep = |dir_root: PathBuf, label: &str, owner: &dyn Fn(&str) -> Option<chrono::DateTime<Utc>>, referenced: &dyn Fn(&str) -> bool| {
        for entry in fs::read_dir(&dir_root).into_iter().flatten().flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let display = format!("{label}/{name}");
            if referenced(&name) {
                kept.push(display);
                continue;
            }
            let age_ok = match owner(&name) {
                Some(published) => now - published >= grace,
                None => match fs::metadata(&path).and_then(|meta| meta.modified()) {
                    Ok(modified) => now - chrono::DateTime::<Utc>::from(modified) >= grace,
                    Err(error) => {
                        failed.push(json!({"path": display, "error": error.to_string()}));
                        continue;
                    }
                },
            };
            if !age_ok {
                kept.push(display);
                continue;
            }
            match remove_dir(&path) {
                Ok(()) => removed.push(display),
                Err(error) => failed.push(json!({"path": display, "error": error.to_string()})),
            }
        }
    };
    sweep(
        reading_root.join("releases"),
        "releases",
        &|name| batch_published.get(name).copied(),
        &|name| name == current_batch_id || pointer_batches.contains(name),
    );
    sweep(
        reading_root.join("resources"),
        "resources",
        &|name| exam_published.get(name).copied(),
        &|name| manifest.contains_key(name),
    );
    json!({"removed": removed, "kept": kept, "failed": failed})
}

/// 提交后的状态 CAS：把已发布的条目标成 `status = 'published'`，且**版本必须仍是
/// 发布时冻结的那一版**。返回**未能确认**的条目（空表示全部确认）。
///
/// 三种结果必须分开处理，修前这里只处理了第一种：
///
/// - `Err`（真数据库错误）：不升级为回滚 —— 清单已替换、资源已就位，回滚会留下
///   「新清单 + 无资源」的坏包（既有注释记录的决策）；但同样**不得静默**。
/// - **CAS 未匹配**：快照之后该条目又被编辑过（`current_edit_version` 前进），
///   即发布出去的**不是**当前编辑版本。这是确凿的版本漂移，修前被完全忽略，
///   于是 `publish` 对调用方返回 `Ok`（表现为"发布成功"），而库里那条目的
///   `status` 根本没被改 —— 同一个事实在返回值和数据库里是两个答案。
/// - 正常。
///
/// 判据用**读回校验**而不是 `execute` 的受影响行数：行数是"匹配到"还是"真的改了"
/// 属于驱动层语义，不该成为发布判据的一部分。直接查 `status` + `current_edit_version`，
/// 后置条件成立与否一目了然。
/// 一道题在本批中的**打包尝试**结果。
///
/// 分成两支是因为「能不能进学生清单」在放行模式下不再是硬失败：
/// - `Packaged`：脚本 + 资源都已就位，清单里会有这条。
/// - `AuthoringOnly`：授权快照照发（用户仍可编辑/导出/继续修），但不进学生清单。
///   两种来源都会走到这里：门禁结论本身不允许进（`studentLoadable == false`），
///   以及放行模式下包检查失败后的降级（`package_error` 记下原因码）。
enum ItemAttempt {
    AuthoringOnly {
        publish_override: Option<Value>,
        /// 门禁原样结论，从不被改写成 Ready。
        verdict: Value,
        reasons: Value,
        /// 包检查（组装 + 学生加载器探针）的失败原因码。门禁降级时为 `None`。
        package_error: Option<String>,
    },
    Packaged {
        staged: StagedPackage,
        exam_id: String,
        asset_count: usize,
        publish_override: Option<Value>,
        verdict: Value,
        reasons: Value,
    },
}

/// 放行模式下「只降级这一条、继续整批」的失败面：**包检查**没过
/// （`stage_package_files` 组装出包之后，学生加载器探针判定它读不了）。
///
/// 判据刻意写成**白名单**：新增的错误码默认中断整批。理由——放行发布是「用户接受
/// 这条题的内容质量缺陷」，不是「用户接受后端替他吞掉环境故障」。IO 类
/// （资产缺失/复制失败、staging 目录不可写）、安全类（资产相对路径越界、examId
/// 非法）、以及清单 CAS / 锁的失败都必须继续中断：把它们降级成一条静默的
/// authoring-only 记录，用户会以为「发布成功了」，而磁盘上少了一整个资源目录。
fn is_degradable_package_check_failure(error: &str) -> bool {
    error.starts_with("nas_package_v2_probe_failed:")
}

/// 一道题在本批中的发布结果（清单提交后写状态与发布记录用）。
struct ItemPublication {
    item_id: String,
    edit_version: i64,
    record_id: String,
    /// 门禁结论不是 Ready、由用户点击发布显式放行。
    forced: bool,
    student_loadable: bool,
    /// 门禁原样结论（`PublishVerdict::to_value`），从不被改写成 Ready。
    verdict: Value,
    reasons: Value,
}

impl ItemPublication {
    /// 发布后写进 `library_items_v2.status` / `publish_records_v2.status` 的结论。
    ///
    /// **`published` 是「学生能打开」的同义词**：`student_loadable == false` 的条目
    /// 永远不能叫 `published`，无论门禁是否被放行。旧实现只看 `forced`，于是
    /// 「门禁 Ready、但包检查失败降级成 authoring-only」这一条（`forced == false`、
    /// `student_loadable == false`）被记成 `published`——题库显示「已发布」，
    /// 学生端点开却打不开，而按 `status === "published"` 判干净通过的验收链会假绿。
    ///
    /// 两个维度都要有确切名字，别让「放行」掩盖「打不开」：
    /// - `(forced=false, loadable=true)`  → `published`
    /// - `(forced=true,  loadable=true)`  → `published_forced`
    /// - `(forced=false, loadable=false)` → `published_not_loadable`
    /// - `(forced=true,  loadable=false)` → `published_forced_not_loadable`
    fn item_status(&self) -> &'static str {
        match (self.forced, self.student_loadable) {
            (false, true) => "published",
            (true, true) => "published_forced",
            (false, false) => "published_not_loadable",
            (true, false) => "published_forced_not_loadable",
        }
    }
}

fn write_publish_records(
    conn: &rusqlite::Connection,
    publications: &[ItemPublication],
    status_drift: &[Value],
    batch_id: &str,
    destination: &str,
) -> CommandResult<()> {
    let now = Utc::now().to_rfc3339();
    for publication in publications {
        let drifted = status_drift.iter().any(|drift| {
            drift.get("itemId").and_then(Value::as_str) == Some(publication.item_id.as_str())
        });
        let status = if drifted {
            "status_drift"
        } else {
            publication.item_status()
        };
        conn.execute(
            "INSERT INTO publish_records_v2
             (id, library_item_id, edit_version, batch_id, destination, forced, verdict_json,
              reasons_json, student_loadable, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                publication.record_id,
                publication.item_id,
                publication.edit_version,
                batch_id,
                destination,
                publication.forced as i64,
                publication.verdict.to_string(),
                publication.reasons.to_string(),
                publication.student_loadable as i64,
                status,
                now
            ],
        )
        .map_err(|error| format!("{}:{error}", publication.item_id))?;
    }
    Ok(())
}

fn commit_published_status(
    conn: &rusqlite::Connection,
    publications: &[ItemPublication],
) -> CommandResult<Vec<Value>> {
    let mut status_drift: Vec<Value> = Vec::new();
    for publication in publications {
        let id = &publication.item_id;
        let version = &publication.edit_version;
        let target_status = publication.item_status();
        if let Err(error) = conn.execute(
            "UPDATE library_items_v2 SET status = ?3 WHERE id = ?1 AND current_edit_version = ?2",
            rusqlite::params![id, version, target_status],
        ) {
            status_drift.push(json!({
                "itemId": id,
                "publishedEditVersion": version,
                "reason": "status_update_failed",
                "error": error.to_string(),
            }));
            continue;
        }
        let observed: Option<(String, i64)> = conn
            .query_row(
                "SELECT status, current_edit_version FROM library_items_v2 WHERE id = ?1",
                rusqlite::params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| format!("PUBLISH_BATCH_STATUS_READ:{id}:{error}"))?;
        match observed {
            Some((status, current)) if status == target_status && current == *version => {}
            Some((status, current)) => status_drift.push(json!({
                "itemId": id,
                "publishedEditVersion": version,
                "currentEditVersion": current,
                "currentStatus": status,
                "reason": "post_publish_state_not_confirmed",
            })),
            None => status_drift.push(json!({
                "itemId": id,
                "publishedEditVersion": version,
                "reason": "item_missing_after_manifest_commit",
            })),
        }
    }
    Ok(status_drift)
}

pub(crate) fn publish_nas_package_v2_core(root: &Path, input: Value) -> CommandResult<Value> {
    let input: NasPackagePublishInput = serde_json::from_value(input)
        .map_err(|error| format!("nas_package_v2_invalid_request:{error}"))?;
    // The public command accepts either the NAS parent directory or its
    // `publish/` child (the legacy exporter accepts both).  Normalize before
    // deriving *any* transaction path so both spellings share one lock,
    // manifest/CAS target, journal and backup namespace.
    let library_root =
        normalize_nas_library_root(&absolute_path("library_root", &input.library_root)?);
    let source_path = absolute_path("source_path", &input.source_path)?;
    let source_bytes = fs::read(&source_path).map_err(|error| {
        format!(
            "nas_package_v2_source_read:{}:{error}",
            source_path.display()
        )
    })?;
    validate_v2_export_binding(&root, &input, &source_path, &source_bytes)?;
    let source_value: Value = serde_json::from_slice(&source_bytes).map_err(|error| {
        format!(
            "nas_package_v2_source_json:{}:{error}",
            source_path.display()
        )
    })?;
    let source = CompiledExamSourceV2::parse(&source_value)
        .map_err(|error| format!("nas_package_v2_source_contract:{error}"))?;
    let issues = source.validate();
    if !issues.is_empty() {
        return Err(format!(
            "nas_package_v2_source_invalid:{}",
            serde_json::to_string(&issues).unwrap_or_default()
        ));
    }
    let safe_source_exam_id = safe_exam_id(&source_value)?;
    if input
        .exam_id
        .as_deref()
        .is_some_and(|exam_id| exam_id != safe_source_exam_id)
    {
        return Err("nas_package_v2_exam_id_mismatch".to_string());
    }

    let reading_root = nas_reading_exams_dir(&library_root);
    fs::create_dir_all(&reading_root).map_err(|error| error.to_string())?;
    let mut paths = make_paths(&library_root, &reading_root, &safe_source_exam_id)?;
    fs::create_dir_all(&paths.library_root).map_err(|error| error.to_string())?;
    fs::create_dir_all(&paths.reading_root).map_err(|error| error.to_string())?;
    fs::create_dir_all(paths.lock_path.parent().unwrap_or(&paths.reading_root))
        .map_err(|error| error.to_string())?;

    let export_id = Uuid::new_v4().simple().to_string();
    paths.staging_root = paths
        .reading_root
        .join(format!(".phase6-staging-{export_id}"));
    paths.staging_exam_path = paths.staging_root.join(format!("{safe_source_exam_id}.js"));
    paths.staging_resource_path = paths
        .staging_root
        .join("resources")
        .join(&safe_source_exam_id);
    paths.staging_manifest_path = paths.staging_root.join("manifest.js");
    paths.backup_root = paths
        .library_root
        .join(CONTROL_DIR_NAME)
        .join(BACKUP_DIR_NAME)
        .join(format!("{safe_source_exam_id}-{export_id}"));
    paths.journal_path = paths
        .library_root
        .join(CONTROL_DIR_NAME)
        .join(format!("{safe_source_exam_id}-{export_id}.journal.json"));
    paths.report_path = paths
        .library_root
        .join(REPORT_DIR_NAME)
        .join(format!("{safe_source_exam_id}-{export_id}.json"));

    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&paths.lock_path)
        .map_err(|error| format!("nas_package_v2_lock_open:{error}"))?;
    lock.try_lock_exclusive()
        .map_err(|error| format!("nas_package_v2_lock_busy:{error}"))?;

    write_lock_metadata(&paths, export_id.as_str())?;
    recover_incomplete_transactions(&paths)?;
    paths.base_manifest_sha256 = manifest_sha256(&paths.manifest_path)?;
    if let Some(expected) = input.expected_manifest_sha256.as_deref() {
        verify_manifest_compare_and_swap(&paths.manifest_path, expected)?;
    }
    write_journal(&paths, &source, "staging", None, export_id.as_str())?;
    let result = stage_and_commit(
        root,
        &input,
        &source,
        &source_value,
        &source_path,
        &paths,
        export_id.as_str(),
    );
    if let Err(error) = &result {
        let _ = write_journal(&paths, &source, "failed", Some(error), export_id.as_str());
        let _ = fs::remove_dir_all(&paths.staging_root);
    }
    let _ = fs::remove_file(&paths.lock_metadata_path);
    lock.unlock().ok();
    result
}

struct StagedPackage {
    entry: Value,
    probe: StudentProbeReportV2,
    runtime_sha256: String,
    minimum_runtime_version: String,
}

fn stage_package_files(
    input: &NasPackagePublishInput,
    source: &CompiledExamSourceV2,
    source_value: &Value,
    source_path: &Path,
    paths: &PackagePaths,
) -> CommandResult<StagedPackage> {
    fs::create_dir_all(&paths.staging_resource_path)
        .map_err(|error| format!("nas_package_v2_staging_create:{error}"))?;
    let asset_root = input
        .asset_root
        .as_deref()
        .map(|value| absolute_path("asset_root", value))
        .transpose()?;
    let asset_root = asset_root.unwrap_or_else(|| {
        source_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    });

    let mut manifest_assets = BTreeMap::new();
    let mut seen_destinations = BTreeSet::new();
    for descriptor in source.assets() {
        validate_package_relative_path(&descriptor.relative_path)?;
        let source_asset =
            safe_join_asset_path(&asset_root, &descriptor.relative_path).map_err(|error| {
                format!(
                    "nas_package_v2_asset_source:{}:{error}",
                    descriptor.asset_id
                )
            })?;
        let destination_relative = format!(
            "resources/{}/{}",
            source.exam_id(),
            descriptor.relative_path
        );
        let destination = paths.staging_root.join(&destination_relative);
        let collision_key = destination_relative.to_ascii_lowercase();
        if !seen_destinations.insert(collision_key) {
            return Err(format!(
                "nas_package_v2_asset_path_collision:{}",
                descriptor.relative_path
            ));
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::copy(&source_asset, &destination).map_err(|error| {
            format!("nas_package_v2_asset_copy:{}:{error}", descriptor.asset_id)
        })?;
        manifest_assets.insert(descriptor.asset_id.clone(), descriptor.clone());
    }

    // 确定性：只含 schema、examId 与按 assetId 排序的资产描述，不含时间戳。
    let asset_manifest = ExamAssetManifestV2 {
        schema_version: "ExamAssetManifestV2".to_string(),
        exam_id: source.exam_id().to_string(),
        assets: manifest_assets,
    };
    let asset_manifest_value =
        serde_json::to_value(&asset_manifest).map_err(|error| error.to_string())?;
    let asset_manifest_bytes =
        canonical_json_bytes(&asset_manifest_value).map_err(|error| error.to_string())?;
    let asset_manifest_path = paths.staging_resource_path.join(ASSET_MANIFEST_FILE_NAME);
    write_synced_file(&asset_manifest_path, &asset_manifest_bytes)?;

    // 学生端用 JSON.stringify 重算 runtimeSha256，manifest 里的值必须与之一致，
    // 不能是 serde_json 的数字写法（1.0 vs 1、1e21 vs 1e+21）。哈希操作数必须是
    // wrapper 真正嵌入的那份 payload（student_package_source 投影 + canonical 编码），
    // 而不是对磁盘 source JSON 原样重编码：任何 `skip_serializing_if` 字段显式为
    // null 时，两者字节不同，学生端会以 reading_source_integrity_failed 拒绝整个包。
    let runtime_bytes = canonical_json_bytes_js(&student_package_source(source_value));
    let wrapper = build_wrapper(source_value)?;
    write_synced_file(&paths.staging_exam_path, wrapper.as_bytes())?;
    if input.fault.as_deref() == Some("after_assets") {
        return Err("nas_package_v2_fault_after_assets".to_string());
    }
    if input.fault.as_deref() == Some("after_source") {
        return Err("nas_package_v2_fault_after_source".to_string());
    }

    let runtime_sha256 = sha256_hex(&runtime_bytes);
    let asset_manifest_sha256 = sha256_hex(&asset_manifest_bytes);
    let script_sha256 = sha256_hex(wrapper.as_bytes());

    update_lock_heartbeat(paths)?;
    // 探针必须复算学生端会读到的真实字节，否则发布门禁会对一个学生加载不了的
    // 包报 passed——正是今天这个缺陷类型。
    let probe = run_student_loader_probe_with_files(
        source,
        &asset_manifest,
        &paths.staging_resource_path,
        Some(&ProbePackageFiles {
            exam_script_path: &paths.staging_exam_path,
            asset_manifest_path: &asset_manifest_path,
            expected_script_sha256: &script_sha256,
            expected_runtime_sha256: &runtime_sha256,
            expected_asset_manifest_sha256: &asset_manifest_sha256,
        }),
    );
    if !probe.passed {
        return Err(format!(
            "nas_package_v2_probe_failed:{}",
            serde_json::to_string(&probe).unwrap_or_default()
        ));
    }

    let minimum_runtime_version = input
        .minimum_runtime_version
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("0.2.0");
    validate_minimum_runtime_version(minimum_runtime_version)?;
    let entry = json!({
        "examId": source.exam_id(),
        "dataKey": source.exam_id(),
        "script": format!("./{}.js", source.exam_id()),
        "title": source.title(),
        "category": source.category_value(),
        "schemaVersion": source.schema_version(),
        "modality": source.modality(),
        "minimumRuntimeVersion": minimum_runtime_version,
        "resourcesBase": format!("./resources/{}/", source.exam_id()),
        "assetManifest": format!("./resources/{}/{}", source.exam_id(), ASSET_MANIFEST_FILE_NAME),
        "checksums": {
            "scriptSha256": script_sha256,
            "assetManifestSha256": asset_manifest_sha256,
            "runtimeSha256": runtime_sha256
        }
    });
    Ok(StagedPackage {
        entry,
        probe,
        runtime_sha256,
        minimum_runtime_version: minimum_runtime_version.to_string(),
    })
}

fn stage_and_commit(
    _app_root: &Path,
    input: &NasPackagePublishInput,
    source: &CompiledExamSourceV2,
    source_value: &Value,
    source_path: &Path,
    paths: &PackagePaths,
    export_id: &str,
) -> CommandResult<Value> {
    let StagedPackage {
        entry,
        probe,
        runtime_sha256,
        minimum_runtime_version,
    } = stage_package_files(input, source, source_value, source_path, paths)?;
    verify_manifest_compare_and_swap(&paths.manifest_path, &paths.base_manifest_sha256)?;
    let mut manifest = load_existing_manifest(&paths.manifest_path)?;
    manifest.insert(source.exam_id().to_string(), entry);
    let mut metadata = manifest
        .remove("_meta")
        .unwrap_or_else(|| json!({"schemaVersion": "ReadingExamManifestV1"}));
    if let Some(meta) = metadata.as_object_mut() {
        meta.insert("schemaVersion".to_string(), json!("ReadingExamManifestV2"));
        meta.insert("assetCount".to_string(), json!(manifest.len()));
        meta.insert("generatedAt".to_string(), json!(Utc::now().to_rfc3339()));
    }
    manifest.insert("_meta".to_string(), metadata);
    let manifest_value = Value::Object(manifest);
    let candidate_manifest = format!(
        "window.__READING_EXAM_MANIFEST__ = {};\n",
        serde_json::to_string_pretty(&manifest_value).map_err(|error| error.to_string())?
    );
    write_synced_file(&paths.staging_manifest_path, candidate_manifest.as_bytes())?;
    if input.fault.as_deref() == Some("before_manifest") {
        return Err("nas_package_v2_fault_before_manifest".to_string());
    }

    write_journal(paths, source, "committing", None, export_id)?;
    let receipt = commit_package(
        paths,
        source,
        &probe,
        &runtime_sha256,
        export_id,
        input.fault.as_deref(),
    )?;
    let report = json!({
        "schemaVersion": "NasPackagePublishReportV2",
        "status": "committed",
        "examId": receipt.exam_id,
        "runtimeVersion": minimum_runtime_version,
        "runtimeSha256": receipt.runtime_sha256,
        "assetManifestSha256": receipt.asset_manifest_sha256,
        "manifestSha256": receipt.manifest_sha256,
        "assetCount": receipt.asset_count,
        "checkedAssetIds": receipt.probe.checked_asset_ids,
        "referencedAssetIds": receipt.probe.referenced_asset_ids,
        "probe": receipt.probe,
        "rollback": {"performed": false, "reason": null},
        "exportId": export_id,
        "writtenAt": Utc::now().to_rfc3339()
    });
    if input.fault.as_deref() == Some("report_write") {
        return Err(recover_post_commit_metadata_failure(
            paths,
            "nas_package_v2_fault_report_write",
        ));
    }
    if let Some(parent) = paths.report_path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return Err(recover_post_commit_metadata_failure(
                paths,
                &format!("nas_package_v2_report_dir_create:{error}"),
            ));
        }
    }
    let report_bytes = match canonical_json_bytes(&report) {
        Ok(bytes) => bytes,
        Err(error) => {
            return Err(recover_post_commit_metadata_failure(
                paths,
                &format!("nas_package_v2_report_encode:{error}"),
            ));
        }
    };
    if let Err(error) = write_synced_file(&paths.report_path, &report_bytes) {
        return Err(recover_post_commit_metadata_failure(
            paths,
            &format!("nas_package_v2_report_write:{error}"),
        ));
    }
    if input.fault.as_deref() == Some("committed_journal") {
        return Err(recover_post_commit_metadata_failure(
            paths,
            "nas_package_v2_fault_committed_journal",
        ));
    }
    if let Err(error) = write_journal(paths, source, "committed", None, export_id) {
        return Err(recover_post_commit_metadata_failure(
            paths,
            &format!("nas_package_v2_committed_journal:{error}"),
        ));
    }
    cleanup_transaction_artifacts(paths);
    Ok(json!({
        "schemaVersion": "NasPackagePublishReportV2",
        "status": "committed",
        "examId": source.exam_id(),
        "manifestPath": paths.manifest_path,
        "reportPath": paths.report_path,
        "probe": receipt.probe,
        "assetCount": receipt.asset_count,
        "exportId": export_id
    }))
}

/// A package commit has already moved the runtime files into their final
/// locations by the time report/journal metadata is written.  Never surface a
/// plain error here: immediately replay the durable journal/backup recovery so
/// the caller receives either a fully rolled-back failure or an explicit
/// rollback-incomplete error that will be retried on the next publish/startup.
fn recover_post_commit_metadata_failure(paths: &PackagePaths, error: &str) -> String {
    match recover_incomplete_transactions(paths) {
        Ok(()) => format!("{error};rollback=complete"),
        Err(recovery_error) => {
            format!("{error};rollback=incomplete:{recovery_error}")
        }
    }
}

fn commit_package(
    paths: &PackagePaths,
    source: &CompiledExamSourceV2,
    probe: &StudentProbeReportV2,
    runtime_sha256: &str,
    export_id: &str,
    fault: Option<&str>,
) -> CommandResult<PackageReceipt> {
    if paths.exam_path.exists() && !paths.exam_path.is_file() {
        return Err("nas_package_v2_existing_exam_not_file".to_string());
    }
    if paths.resource_path.exists() && !paths.resource_path.is_dir() {
        return Err("nas_package_v2_existing_resources_not_directory".to_string());
    }
    if paths.manifest_path.exists() && !paths.manifest_path.is_file() {
        return Err("nas_package_v2_existing_manifest_not_file".to_string());
    }
    let backup_root = paths.backup_root.clone();
    fs::create_dir_all(&backup_root).map_err(|error| error.to_string())?;
    let backup_exam = backup_root.join("exam.js");
    let backup_resources = backup_root.join("resources");
    let backup_manifest = backup_root.join("manifest.js");
    let backup_report = backup_root.join("report.json");
    let had_exam = paths.exam_path.is_file();
    let had_resources = paths.resource_path.exists();
    let had_manifest = paths.manifest_path.is_file();
    if paths.report_path.exists() && !paths.report_path.is_file() {
        return Err("nas_package_v2_existing_report_not_file".to_string());
    }
    let had_report = paths.report_path.is_file();
    let state = json!({
        "schemaVersion": "NasBackupStateV1",
        "hadExam": had_exam,
        "hadResources": had_resources,
        "hadManifest": had_manifest,
        "hadReport": had_report
    });
    write_synced_file(
        &backup_root.join("state.json"),
        &canonical_json_bytes(&state).map_err(|error| error.to_string())?,
    )?;

    // Keep the discovery manifest at its live path until the new staged
    // manifest is ready to replace it.  Moving the old manifest to the
    // backup directory here creates a window in which a student loader sees
    // no manifest at all.  A verified copy gives recovery a durable snapshot
    // without changing the old package's visible surface.
    if had_manifest {
        if let Err(error) = copy_file_verified(&paths.manifest_path, &backup_manifest) {
            cleanup_transaction_artifacts(paths);
            return Err(format!(
                "nas_package_v2_backup_manifest:{error};rollback=complete"
            ));
        }
    }
    if had_exam {
        if let Err(error) = fs::rename(&paths.exam_path, &backup_exam) {
            cleanup_transaction_artifacts(paths);
            return Err(format!(
                "nas_package_v2_backup_exam:{error};rollback=complete"
            ));
        }
    }
    // A failure while moving the old resource directory can happen after the
    // old exam has already been moved.  Restore that missing live entry before
    // returning so the still-live manifest never points at a half-backed-up
    // package.  The durable journal/backup remains available only when this
    // best-effort immediate restore itself fails.
    if fault == Some("backup_resources") {
        let rollback_errors = restore_partial_backup_before_commit(
            paths,
            &backup_exam,
            &backup_resources,
            source.exam_id(),
            had_exam,
            had_resources,
        );
        return Err(if rollback_errors.is_empty() {
            cleanup_transaction_artifacts(paths);
            "nas_package_v2_fault_backup_resources;rollback=complete".to_string()
        } else {
            format!(
                "nas_package_v2_fault_backup_resources;rollback={}",
                rollback_errors.join("|")
            )
        });
    }
    if had_resources {
        if let Err(error) = fs::create_dir_all(&backup_resources) {
            let rollback_errors = restore_partial_backup_before_commit(
                paths,
                &backup_exam,
                &backup_resources,
                source.exam_id(),
                had_exam,
                had_resources,
            );
            return Err(if rollback_errors.is_empty() {
                cleanup_transaction_artifacts(paths);
                format!("nas_package_v2_backup_resources:{error};rollback=complete")
            } else {
                format!(
                    "nas_package_v2_backup_resources:{error};rollback={}",
                    rollback_errors.join("|")
                )
            });
        }
        if let Err(error) = fs::rename(
            &paths.resource_path,
            backup_resources.join(source.exam_id()),
        ) {
            let rollback_errors = restore_partial_backup_before_commit(
                paths,
                &backup_exam,
                &backup_resources,
                source.exam_id(),
                had_exam,
                had_resources,
            );
            return Err(if rollback_errors.is_empty() {
                cleanup_transaction_artifacts(paths);
                format!("nas_package_v2_backup_resources:{error};rollback=complete")
            } else {
                format!(
                    "nas_package_v2_backup_resources:{error};rollback={}",
                    rollback_errors.join("|")
                )
            });
        }
    }
    let rollback = |manifest_committed: bool| -> Vec<String> {
        let mut errors = Vec::new();
        // Do not remove any currently visible package file until every old
        // item that was declared present has a durable backup.  A process or
        // NAS interruption can occur between the individual backup renames;
        // deleting the remaining visible file in that state would turn a
        // recoverable interruption into permanent data loss.
        if let Err(error) = validate_backup_contents(
            &backup_exam,
            &backup_resources,
            &backup_manifest,
            &backup_report,
            source.exam_id(),
            had_exam,
            had_resources,
            had_manifest,
            had_report && backup_report.exists(),
        ) {
            // If an earlier backup rename already removed a path, restoring
            // that missing path is safe and does not overwrite any visible
            // data.  Leave all other visible paths untouched and retain the
            // transaction artifacts for the next recovery attempt.
            if had_exam && !paths.exam_path.exists() && backup_exam.is_file() {
                if let Err(restore_error) = fs::rename(&backup_exam, &paths.exam_path) {
                    errors.push(format!("restore_exam_before_incomplete:{restore_error}"));
                }
            }
            if had_resources
                && !paths.resource_path.exists()
                && backup_resources.join(source.exam_id()).is_dir()
            {
                if let Err(restore_error) = fs::rename(
                    backup_resources.join(source.exam_id()),
                    &paths.resource_path,
                ) {
                    errors.push(format!(
                        "restore_resources_before_incomplete:{restore_error}"
                    ));
                }
            }
            errors.push(format!("backup_incomplete:{error}"));
            return errors;
        }
        if manifest_committed && paths.manifest_path.exists() {
            if let Err(error) = fs::remove_file(&paths.manifest_path) {
                errors.push(format!("remove_manifest:{error}"));
            }
        }
        if paths.exam_path.exists() {
            if let Err(error) = fs::remove_file(&paths.exam_path) {
                errors.push(format!("remove_exam:{error}"));
            }
        }
        if paths.resource_path.exists() {
            if let Err(error) = fs::remove_dir_all(&paths.resource_path) {
                errors.push(format!("remove_resources:{error}"));
            }
        }
        if had_exam {
            if let Err(error) = fs::rename(&backup_exam, &paths.exam_path) {
                errors.push(format!("restore_exam:{error}"));
            }
        }
        if had_resources {
            let backup_path = backup_resources.join(source.exam_id());
            if let Err(error) = fs::rename(backup_path, &paths.resource_path) {
                errors.push(format!("restore_resources:{error}"));
            }
        }
        if had_manifest && backup_manifest.exists() {
            // Before the final manifest replacement the old manifest is
            // still live, so there is nothing to restore.  Once replacement
            // has happened, use the same atomic replacement primitive used
            // for commit; this works on Unix and on Windows where a plain
            // rename cannot replace an existing file.
            if manifest_committed || !paths.manifest_path.exists() {
                if let Err(error) = atomic_replace_file(&backup_manifest, &paths.manifest_path) {
                    errors.push(format!("restore_manifest:{error}"));
                }
            }
        }
        if had_report && backup_report.exists() {
            if let Err(error) = fs::rename(&backup_report, &paths.report_path) {
                errors.push(format!("restore_report:{error}"));
            }
        }
        errors
    };

    if had_report {
        if let Err(error) = fs::rename(&paths.report_path, &backup_report) {
            let rollback_errors = rollback(false);
            return Err(if rollback_errors.is_empty() {
                cleanup_transaction_artifacts(paths);
                format!("nas_package_v2_backup_report:{error};rollback=complete")
            } else {
                format!(
                    "nas_package_v2_backup_report:{error};rollback={}",
                    rollback_errors.join("|")
                )
            });
        }
    }
    let mut manifest_committed = false;
    let move_result: CommandResult<()> = (|| {
        fs::rename(&paths.staging_exam_path, &paths.exam_path)
            .map_err(|error| format!("nas_package_v2_commit_exam:{error}"))?;
        if let Some(parent) = paths.resource_path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::rename(&paths.staging_resource_path, &paths.resource_path)
            .map_err(|error| format!("nas_package_v2_commit_resources:{error}"))?;
        if fault == Some("manifest_rename") {
            return Err("nas_package_v2_fault_manifest_rename".to_string());
        }
        atomic_replace_file(&paths.staging_manifest_path, &paths.manifest_path)
            .map_err(|error| format!("nas_package_v2_commit_manifest:{error}"))?;
        manifest_committed = true;
        write_journal(paths, source, "manifest_committed", None, export_id)?;
        Ok(())
    })();
    if let Err(error) = move_result {
        let rollback_errors = rollback(manifest_committed);
        return Err(if rollback_errors.is_empty() {
            cleanup_transaction_artifacts(paths);
            format!("{error};rollback=complete")
        } else {
            format!("{error};rollback={}", rollback_errors.join("|"))
        });
    }

    let manifest_bytes = fs::read(&paths.manifest_path).map_err(|error| error.to_string())?;
    Ok(PackageReceipt {
        exam_id: source.exam_id().to_string(),
        runtime_sha256: runtime_sha256.to_string(),
        asset_manifest_sha256: sha256_hex(
            &fs::read(paths.resource_path.join(ASSET_MANIFEST_FILE_NAME))
                .map_err(|error| error.to_string())?,
        ),
        asset_count: source.assets().len(),
        probe: probe.clone(),
        manifest_sha256: sha256_hex(&manifest_bytes),
    })
}

fn make_paths(
    library_root: &Path,
    reading_root: &Path,
    exam_id: &str,
) -> CommandResult<PackagePaths> {
    if exam_id.is_empty() || exam_id == "." || exam_id == ".." {
        return Err("nas_package_v2_exam_id_invalid".to_string());
    }
    Ok(PackagePaths {
        library_root: library_root.to_path_buf(),
        reading_root: reading_root.to_path_buf(),
        manifest_path: reading_root.join("manifest.js"),
        exam_path: reading_root.join(format!("{exam_id}.js")),
        resource_path: reading_root.join("resources").join(exam_id),
        staging_root: PathBuf::new(),
        staging_exam_path: PathBuf::new(),
        staging_resource_path: PathBuf::new(),
        staging_manifest_path: PathBuf::new(),
        backup_root: PathBuf::new(),
        journal_path: PathBuf::new(),
        report_path: PathBuf::new(),
        lock_path: library_root.join(CONTROL_DIR_NAME).join("export.lock"),
        lock_metadata_path: library_root
            .join(CONTROL_DIR_NAME)
            .join("export.lock.owner.json"),
        base_manifest_sha256: String::new(),
    })
}

fn absolute_path(label: &str, value: &str) -> CommandResult<PathBuf> {
    let path = PathBuf::from(value.trim());
    if !path.is_absolute() {
        return Err(format!("nas_package_v2_{label}_must_be_absolute"));
    }
    Ok(path)
}

fn validate_package_relative_path(value: &str) -> CommandResult<()> {
    if value.is_empty()
        || value.contains('\0')
        || value.contains('\\')
        || value.contains("://")
        || value.contains(':')
        || value.starts_with('/')
        || value.starts_with("//")
        || value
            .chars()
            .any(|character| matches!(character, '⁄' | '∕' | '╱' | '⧸'))
    {
        return Err(format!("nas_package_v2_asset_path_unsafe:{value}"));
    }
    if value
        .split('/')
        .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(format!("nas_package_v2_asset_path_unsafe:{value}"));
    }
    Ok(())
}

fn parse_runtime_version(value: &str) -> Option<[u64; 3]> {
    let core = value
        .trim()
        .strip_prefix('v')
        .or_else(|| value.trim().strip_prefix('V'))
        .unwrap_or(value.trim())
        .split(['-', '+'])
        .next()?;
    let parts = core.split('.').collect::<Vec<_>>();
    if parts.is_empty() || parts.len() > 3 || parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    let mut parsed = [0_u64; 3];
    for (index, part) in parts.iter().enumerate() {
        parsed[index] = part.parse().ok()?;
    }
    Some(parsed)
}

fn validate_minimum_runtime_version(value: &str) -> CommandResult<()> {
    let parsed = parse_runtime_version(value)
        .ok_or_else(|| format!("nas_package_v2_runtime_version_invalid:{value}"))?;
    if parsed > CURRENT_STUDENT_RUNTIME_VERSION {
        return Err(format!(
            "nas_package_v2_runtime_incompatible:minimum={value}:current=0.2.0"
        ));
    }
    Ok(())
}

fn load_existing_manifest(path: &Path) -> CommandResult<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let source = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let payload = extract_manifest_json_object(&source)?;
    serde_json::from_str::<Value>(&payload)
        .map_err(|error| format!("nas_package_v2_existing_manifest_invalid_json:{error}"))?
        .as_object()
        .cloned()
        .ok_or_else(|| "nas_package_v2_existing_manifest_not_object".to_string())
}

/// Extract the JSON object assigned to the manifest global from either the
/// direct V2 wrapper or the legacy closure-style V1 wrapper, e.g.
/// `global.__READING_EXAM_MANIFEST__ = { ... };`. The scanner only accepts a
/// balanced JSON object and respects quoted strings/escapes, so surrounding JS
/// statements are never passed to serde_json.
fn extract_manifest_json_object(source: &str) -> CommandResult<String> {
    let marker = "__READING_EXAM_MANIFEST__";
    let marker_index = source
        .find(marker)
        .ok_or_else(|| "nas_package_v2_existing_manifest_invalid_assignment".to_string())?;
    let assignment = &source[marker_index + marker.len()..];
    let object_start = assignment
        .find('{')
        .ok_or_else(|| "nas_package_v2_existing_manifest_invalid_assignment".to_string())?;
    let bytes = assignment.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for index in object_start..bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Ok(assignment[object_start..=index].to_string());
                }
            }
            _ => {}
        }
    }
    Err("nas_package_v2_existing_manifest_invalid_assignment".to_string())
}

fn verify_manifest_compare_and_swap(path: &Path, expected: &str) -> CommandResult<()> {
    let actual = manifest_sha256(path)?;
    if !actual.eq_ignore_ascii_case(expected.trim()) {
        return Err(format!(
            "nas_package_v2_manifest_conflict:expected={}:actual={actual}",
            expected.trim()
        ));
    }
    Ok(())
}

fn manifest_sha256(path: &Path) -> CommandResult<String> {
    if !path.exists() {
        return Ok(String::new());
    }
    if !path.is_file() {
        return Err("nas_package_v2_manifest_not_file".to_string());
    }
    Ok(sha256_hex(
        &fs::read(path).map_err(|error| error.to_string())?,
    ))
}

fn safe_transaction_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
        && !value.starts_with('.')
}

fn write_lock_metadata(paths: &PackagePaths, export_id: &str) -> CommandResult<()> {
    let now = Utc::now().to_rfc3339();
    let metadata = json!({
        "schemaVersion": "NasPublishLockV1",
        "exportId": export_id,
        "pid": std::process::id(),
        "createdAt": now,
        "heartbeatAt": now,
        "lockPath": paths.lock_path,
        "manifestPath": paths.manifest_path
    });
    write_synced_file(
        &paths.lock_metadata_path,
        &canonical_json_bytes(&metadata).map_err(|error| error.to_string())?,
    )
}

fn update_lock_heartbeat(paths: &PackagePaths) -> CommandResult<()> {
    let raw = fs::read_to_string(&paths.lock_metadata_path)
        .map_err(|error| format!("nas_package_v2_lock_metadata_read:{error}"))?;
    let mut metadata: Value = serde_json::from_str(&raw)
        .map_err(|error| format!("nas_package_v2_lock_metadata_invalid:{error}"))?;
    let object = metadata
        .as_object_mut()
        .ok_or_else(|| "nas_package_v2_lock_metadata_not_object".to_string())?;
    object.insert("heartbeatAt".to_string(), json!(Utc::now().to_rfc3339()));
    write_synced_file(
        &paths.lock_metadata_path,
        &canonical_json_bytes(&metadata).map_err(|error| error.to_string())?,
    )
}

fn cleanup_transaction_artifacts(paths: &PackagePaths) {
    let _ = fs::remove_dir_all(&paths.staging_root);
    let _ = fs::remove_dir_all(&paths.backup_root);
}

fn remove_path_if_present(path: &Path) -> CommandResult<()> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        fs::remove_dir_all(path).map_err(|error| error.to_string())
    } else {
        fs::remove_file(path).map_err(|error| error.to_string())
    }
}

/// Copy a file into the durable transaction backup and verify that the copy
/// contains exactly the same bytes before the source is allowed to remain in
/// service.  The manifest uses copy (rather than rename) so the previous
/// discovery point remains visible while the package is being committed.
fn copy_file_verified(source: &Path, destination: &Path) -> CommandResult<()> {
    if !source.is_file() {
        return Err(format!("backup_source_not_file:{}", source.display()));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let temporary = destination.with_extension(format!("copy-{}", Uuid::new_v4().simple()));
    fs::copy(source, &temporary).map_err(|error| error.to_string())?;
    let source_bytes = fs::read(source).map_err(|error| error.to_string())?;
    let destination_bytes = fs::read(&temporary).map_err(|error| error.to_string())?;
    if source_bytes != destination_bytes {
        let _ = fs::remove_file(&temporary);
        return Err("backup_copy_verification_failed".to_string());
    }
    fs::rename(&temporary, destination).map_err(|error| error.to_string())?;
    Ok(())
}

/// Replace a destination file with a staged file in the same directory.
/// Keep the old discovery manifest visible until its replacement is ready.
fn atomic_replace_file(source: &Path, destination: &Path) -> CommandResult<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
        }
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        // Both buffers are NUL-terminated and live for the duration of the call.
        if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0x1 | 0x8) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        fs::rename(source, destination).map_err(|error| error.to_string())
    }
}

fn validate_backup_contents(
    backup_exam: &Path,
    backup_resources: &Path,
    backup_manifest: &Path,
    backup_report: &Path,
    exam_id: &str,
    had_exam: bool,
    had_resources: bool,
    had_manifest: bool,
    require_report_backup: bool,
) -> CommandResult<()> {
    let mut missing = Vec::new();
    if had_exam && !backup_exam.is_file() {
        missing.push("exam.js");
    }
    if had_resources && !backup_resources.join(exam_id).is_dir() {
        missing.push("resources");
    }
    if had_manifest && !backup_manifest.is_file() {
        missing.push("manifest.js");
    }
    if require_report_backup && !backup_report.is_file() {
        missing.push("report.json");
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("missing={}", missing.join(",")))
    }
}

/// Restore only entries that were already moved out of the live package while
/// building the durable backup.  This helper is intentionally conservative:
/// it never removes an existing live path, so a partial backup failure cannot
/// turn into a manifest-visible data loss.  The full rollback closure below is
/// still responsible for failures after the new package starts committing.
fn restore_partial_backup_before_commit(
    paths: &PackagePaths,
    backup_exam: &Path,
    backup_resources: &Path,
    exam_id: &str,
    had_exam: bool,
    had_resources: bool,
) -> Vec<String> {
    let mut errors = Vec::new();
    if had_exam && !paths.exam_path.exists() && backup_exam.is_file() {
        if let Err(error) = fs::rename(backup_exam, &paths.exam_path) {
            errors.push(format!("restore_exam_before_backup_failure:{error}"));
        }
    }
    let backup_resource_path = backup_resources.join(exam_id);
    if had_resources && !paths.resource_path.exists() && backup_resource_path.is_dir() {
        if let Err(error) = fs::rename(&backup_resource_path, &paths.resource_path) {
            errors.push(format!("restore_resources_before_backup_failure:{error}"));
        }
    }
    errors
}

fn copy_dir_recursive(source: &Path, destination: &Path) -> CommandResult<()> {
    if !source.is_dir() {
        return Err(format!("backup_source_not_directory:{}", source.display()));
    }
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir_recursive(&source_path, &destination_path)?;
        } else if source_path.is_file() {
            copy_file_verified(&source_path, &destination_path)?;
        } else {
            return Err(format!(
                "backup_source_not_regular:{}",
                source_path.display()
            ));
        }
    }
    Ok(())
}

fn restore_backup_file_or_keep(
    backup: &Path,
    live: &Path,
    required: bool,
    label: &str,
    atomic_replace: bool,
) -> CommandResult<()> {
    if backup.exists() {
        if atomic_replace {
            atomic_replace_file(backup, live)
                .map_err(|error| format!("nas_package_v2_recovery_{label}:{error}"))?;
        } else {
            remove_path_if_present(live)?;
            if let Some(parent) = live.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            fs::rename(backup, live)
                .map_err(|error| format!("nas_package_v2_recovery_{label}:{error}"))?;
        }
    } else if required && !live.exists() {
        return Err(format!(
            "nas_package_v2_recovery_backup_incomplete:{label}_missing"
        ));
    }
    Ok(())
}

fn restore_backup_dir_or_keep(
    backup: &Path,
    live: &Path,
    required: bool,
    label: &str,
) -> CommandResult<()> {
    if backup.exists() {
        remove_path_if_present(live)?;
        if let Some(parent) = live.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::rename(backup, live)
            .map_err(|error| format!("nas_package_v2_recovery_{label}:{error}"))?;
    } else if required && !live.exists() {
        return Err(format!(
            "nas_package_v2_recovery_backup_incomplete:{label}_missing"
        ));
    }
    Ok(())
}

/// 重放被中断的批量发布：清单未替换 → 还原备份资源、清理 staging/release；
/// 清单已替换 → 新资源已生效，仅需清理备份。
fn recover_interrupted_batches(paths: &PackagePaths, control_root: &Path) -> CommandResult<()> {
    let backups_root = control_root.join(BACKUP_DIR_NAME);
    if !backups_root.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(&backups_root).map_err(|error| error.to_string())? {
        let dir = entry.map_err(|error| error.to_string())?.path();
        let Some(name) = dir.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !name.starts_with("batch-") {
            continue;
        }
        let state_path = dir.join("state.json");
        let Ok(raw) = fs::read_to_string(&state_path) else {
            continue;
        };
        let state: Value = serde_json::from_str(&raw)
            .map_err(|error| format!("nas_package_v2_recovery_state_invalid:{error}"))?;
        if state.get("schemaVersion").and_then(Value::as_str) != Some("NasBatchBackupStateV1") {
            continue;
        }
        let batch_id = name.trim_start_matches("batch-");
        if !safe_transaction_component(batch_id) {
            return Err("nas_package_v2_recovery_journal_identity_invalid".to_string());
        }
        // 提交点判定必须基于磁盘事实，而不是状态文件。清单替换成功后进程可能
        // 在写入 `manifestCommitted` 之前崩溃；此时包已经生效，按状态文件回滚
        // 会把它弄坏。三个正向信号取或：状态自称已提交 / 线上清单已被本批次替换 /
        // 线上清单哈希等于状态里记录的候选哈希。
        let had_manifest = state
            .get("hadManifest")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let committed = state
            .get("manifestCommitted")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || manifest_replaced_since_baseline(paths, &dir, had_manifest)
            || manifest_matches_state(paths, &state);
        if committed {
            let _ = fs::remove_dir_all(&dir);
            continue;
        }
        for exam in state
            .get("exams")
            .and_then(Value::as_array)
            .unwrap_or(&Vec::new())
        {
            let Some(exam_id) = exam.get("examId").and_then(Value::as_str) else {
                continue;
            };
            if !safe_transaction_component(exam_id) {
                return Err("nas_package_v2_recovery_journal_identity_invalid".to_string());
            }
            let root_resource = paths.reading_root.join("resources").join(exam_id);
            let backup_resource = dir.join("resources").join(exam_id);
            // 只有备份确实存在时才动线上目录。状态先于 rename 写入，所以
            // 「有该题记录但无备份」意味着旧资源从未被移走，必须原样保留。
            if backup_resource.exists() {
                if let Err(error) = fs::remove_dir_all(&root_resource) {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        return Err(error.to_string());
                    }
                }
                fs::rename(&backup_resource, &root_resource).map_err(|error| error.to_string())?;
            }
        }
        restore_manifest_baseline(paths, &dir)?;
        let _ = fs::remove_dir_all(
            paths
                .reading_root
                .join(format!(".batch-staging-{batch_id}")),
        );
        let _ = fs::remove_dir_all(paths.reading_root.join("releases").join(batch_id));
        let _ = fs::remove_dir_all(&dir);
    }
    Ok(())
}

/// 线上清单是否已被本批次替换。
///
/// 只看磁盘：批次开始前有清单基线时，「线上清单与基线不同」即已被替换；
/// 批次开始前没有清单（新库）时，「线上清单存在」即由本批次创建。
///
/// 这个判据不依赖状态文件的写入时机，也**不依赖新加的 `pendingManifestSha256`**，
/// 因此旧版本写下的、没有该字段的状态文件同样能判对——否则那些批次仍会被误判为
/// 未提交，进而把已经生效的包回滚掉。
fn manifest_replaced_since_baseline(
    paths: &PackagePaths,
    backup_dir: &Path,
    had_manifest: bool,
) -> bool {
    let baseline = backup_dir.join("manifest.js");
    if baseline.is_file() {
        let Ok(base) = fs::read(&baseline) else {
            return false;
        };
        return match fs::read(&paths.manifest_path) {
            Ok(live) => live != base,
            Err(_) => false,
        };
    }
    if had_manifest {
        // 状态自称有基线，但基线文件不在：无法判定，保守按「未替换」处理，
        // 走备份感知的还原路径（它只在备份确实存在时才动线上目录）。
        return false;
    }
    paths.manifest_path.is_file()
}

/// 批量清单是否已经替换：线上清单的内容哈希等于状态里记录的「待提交清单」哈希。
///
/// 注意**不能**拿 `releases/<batchId>/manifest.js` 来比对：提交走的是
/// `atomic_replace_file`（移动语义），提交成功后候选清单已被移走，release 目录里
/// 不再有该文件，比对会恒为 false——于是崩溃恢复会把一个已经生效的包当成未提交，
/// 删掉刚上线的资源并把清单退回旧基线，正好毁掉提交结果。
fn manifest_matches_state(paths: &PackagePaths, state: &Value) -> bool {
    let Some(expected) = state.get("pendingManifestSha256").and_then(Value::as_str) else {
        return false;
    };
    match fs::read(&paths.manifest_path) {
        Ok(live) => sha256_hex(&live) == expected,
        Err(_) => false,
    }
}

/// 未提交但清单已被替换时还原基线清单。`atomic_replace_file` 本身原子，
/// 正常路径下线上清单不会损坏，因此只在基线存在且内容不同时才动作。
fn restore_manifest_baseline(paths: &PackagePaths, backup_dir: &Path) -> CommandResult<()> {
    let baseline = backup_dir.join("manifest.js");
    if !baseline.is_file() {
        return Ok(());
    }
    let bytes = fs::read(&baseline).map_err(|error| error.to_string())?;
    if fs::read(&paths.manifest_path)
        .map(|live| live == bytes)
        .unwrap_or(false)
    {
        return Ok(());
    }
    atomic_replace_file(&baseline, &paths.manifest_path)
        .map_err(|error| format!("nas_package_v2_recovery_manifest:{error}"))
}

fn recover_incomplete_transactions(paths: &PackagePaths) -> CommandResult<()> {
    let control_root = paths.library_root.join(CONTROL_DIR_NAME);
    if !control_root.is_dir() {
        return Ok(());
    }
    recover_interrupted_batches(paths, &control_root)?;
    for entry in fs::read_dir(&control_root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let journal_path = entry.path();
        if journal_path.extension().and_then(|value| value.to_str()) != Some("json")
            || !journal_path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.ends_with(".journal.json"))
        {
            continue;
        }
        let raw = fs::read_to_string(&journal_path).map_err(|error| error.to_string())?;
        let mut journal: Value = serde_json::from_str(&raw)
            .map_err(|error| format!("nas_package_v2_recovery_journal_invalid:{error}"))?;
        let status = journal
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if matches!(status, "committed" | "recovered") {
            continue;
        }
        let exam_id = journal
            .get("examId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let export_id = journal
            .get("exportId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !safe_transaction_component(exam_id) || !safe_transaction_component(export_id) {
            return Err("nas_package_v2_recovery_journal_identity_invalid".to_string());
        }
        let staging_root = paths
            .reading_root
            .join(format!(".phase6-staging-{export_id}"));
        let backup_root = control_root
            .join(BACKUP_DIR_NAME)
            .join(format!("{exam_id}-{export_id}"));
        let exam_path = paths.reading_root.join(format!("{exam_id}.js"));
        let resource_path = paths.reading_root.join("resources").join(exam_id);
        let manifest_path = paths.reading_root.join("manifest.js");
        let report_path = paths
            .library_root
            .join(REPORT_DIR_NAME)
            .join(format!("{exam_id}-{export_id}.json"));
        let commit_started = matches!(status, "committing" | "manifest_committed" | "failed")
            || backup_root.exists();
        if commit_started && backup_root.exists() {
            let state_path = backup_root.join("state.json");
            let state: Value = serde_json::from_str(
                &fs::read_to_string(&state_path)
                    .map_err(|error| format!("nas_package_v2_recovery_state_missing:{error}"))?,
            )
            .map_err(|error| format!("nas_package_v2_recovery_state_invalid:{error}"))?;
            let had_exam = state
                .get("hadExam")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let had_resources = state
                .get("hadResources")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let had_manifest = state
                .get("hadManifest")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let had_report = state
                .get("hadReport")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            let backup_exam = backup_root.join("exam.js");
            let backup_resources = backup_root.join("resources");
            let backup_manifest = backup_root.join("manifest.js");
            let backup_report = backup_root.join("report.json");
            // A crash may occur between individual backup renames. Restore
            // each available old item independently; if its backup is absent,
            // preserve the live item and only fail when that required item is
            // absent too. This is idempotent and keeps the old package
            // loadable across repeated recovery attempts.
            restore_backup_file_or_keep(&backup_exam, &exam_path, had_exam, "exam", false)?;
            restore_backup_dir_or_keep(
                &backup_resources.join(exam_id),
                &resource_path,
                had_resources,
                "resources",
            )?;
            restore_backup_file_or_keep(
                &backup_manifest,
                &manifest_path,
                had_manifest,
                "manifest",
                true,
            )?;
            restore_backup_file_or_keep(&backup_report, &report_path, had_report, "report", false)?;
        }
        remove_path_if_present(&staging_root)?;
        remove_path_if_present(&backup_root)?;
        if let Some(object) = journal.as_object_mut() {
            object.insert("status".to_string(), json!("recovered"));
            object.insert("recoveredAt".to_string(), json!(Utc::now().to_rfc3339()));
        }
        write_synced_file(
            &journal_path,
            &canonical_json_bytes(&journal).map_err(|error| error.to_string())?,
        )?;
    }
    Ok(())
}

fn write_synced_file(path: &Path, bytes: &[u8]) -> CommandResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let tmp = path.with_extension(format!("tmp-{}", Uuid::new_v4().simple()));
    let mut file = File::create(&tmp).map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);
    fs::rename(&tmp, path).map_err(|error| error.to_string())?;
    Ok(())
}

fn write_journal(
    paths: &PackagePaths,
    source: &CompiledExamSourceV2,
    status: &str,
    error: Option<&String>,
    export_id: &str,
) -> CommandResult<()> {
    let journal = json!({
        "schemaVersion": "NasCommitJournalV1",
        "exportId": export_id,
        "examId": source.exam_id(),
        "status": status,
        "manifestPath": paths.manifest_path,
        "stagingRoot": paths.staging_root,
        "backupRoot": paths.backup_root,
        "manifestCommitted": status == "manifest_committed" || status == "committed",
        "baseManifestSha256": paths.base_manifest_sha256,
        "error": error,
        "updatedAt": Utc::now().to_rfc3339()
    });
    if let Some(parent) = paths.journal_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    write_synced_file(
        &paths.journal_path,
        &canonical_json_bytes(&journal).map_err(|error| error.to_string())?,
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reading_source_v2::compile_reading_source_v2;
    use crate::schema::IeltsAuthoringIRV2;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("ielts-phase6-package-{suffix}"));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn strict_publication(item_id: &str, edit_version: i64) -> ItemPublication {
        ItemPublication {
            item_id: item_id.to_string(),
            edit_version,
            record_id: "record-test".to_string(),
            forced: false,
            student_loadable: true,
            verdict: json!({"status": "ready"}),
            reasons: json!([]),
        }
    }

    /// 状态名必须同时说出「门禁是否被放行」与「学生端能不能打开」这两件事。
    ///
    /// 旧实现只看 `forced`，于是 `(forced=false, student_loadable=false)`——门禁 Ready、
    /// 但包检查失败被降级成 authoring-only——被记成 `published`：题库显示「已发布」，
    /// 学生端点开却打不开，而按 `status === "published"` 判干净通过的验收链会假绿。
    #[test]
    fn published_status_never_hides_that_students_cannot_open_the_item() {
        let make = |forced: bool, student_loadable: bool| ItemPublication {
            item_id: "item-status".to_string(),
            edit_version: 1,
            record_id: "record-test".to_string(),
            forced,
            student_loadable,
            verdict: json!({"status": "ready"}),
            reasons: json!([]),
        };
        assert_eq!(make(false, true).item_status(), "published");
        assert_eq!(make(true, true).item_status(), "published_forced");
        assert_eq!(make(false, false).item_status(), "published_not_loadable");
        assert_eq!(
            make(true, false).item_status(),
            "published_forced_not_loadable"
        );
        // 不变式：学生端打不开的条目永远不是 `published`，放行与否都一样。
        for forced in [false, true] {
            assert_ne!(
                make(forced, false).item_status(),
                "published",
                "student_loadable=false 永远不能叫 published（forced={forced}）"
            );
        }
    }

    /// 反例（#13）：快照冻结的是版本 1，提交后库里已经是版本 2。
    ///
    /// 修前 `publish_items_core` 里那段是
    /// `if let Err(error) = conn.execute("UPDATE … WHERE id=?1 AND current_edit_version=?2") { eprintln!(…) }`。
    /// `execute` 返回 `Ok(0)`（CAS 未匹配）**不是** `Err`，于是这一格被静默放过，
    /// 函数继续返回 `Ok`（表现为"发布成功"），而库里那条目的 `status` 从来没被改成
    /// `published` —— 同一个事实在返回值和数据库里是两个答案。
    #[test]
    fn post_publish_status_cas_reports_edit_version_drift_instead_of_swallowing_it() {
        let root = temp_root();
        let conn = crate::library::repository::open_library_connection(&root).unwrap();
        conn.execute(
            "INSERT INTO library_items_v2 (id, modality, title, status, current_edit_version, canonical_ds_json, source_asset_id, created_at, updated_at, deleted_at)
             VALUES (?1, 'reading', ?2, 'ready', ?3, NULL, NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', NULL)",
            rusqlite::params!["item-drifted", "Drifted item", 2i64],
        )
        .unwrap();

        // 冻结版本 = 1（快照时看到的），库里现在是 2（期间被编辑过）。
        let drift =
            commit_published_status(&conn, &[strict_publication("item-drifted", 1)]).unwrap();

        assert_eq!(
            drift.len(),
            1,
            "CAS 未匹配必须被报出来，不能当作成功：{drift:?}"
        );
        assert_eq!(
            drift[0].get("itemId").and_then(Value::as_str),
            Some("item-drifted")
        );
        assert_eq!(
            drift[0].get("publishedEditVersion").and_then(Value::as_i64),
            Some(1)
        );
        assert_eq!(
            drift[0].get("currentEditVersion").and_then(Value::as_i64),
            Some(2)
        );
        assert_eq!(
            drift[0].get("reason").and_then(Value::as_str),
            Some("post_publish_state_not_confirmed")
        );
        let status: String = conn
            .query_row(
                "SELECT status FROM library_items_v2 WHERE id = ?1",
                rusqlite::params!["item-drifted"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "ready", "CAS 未匹配时不得把条目标成已发布");

        // 反向：版本一致时不得误报，否则每次正常发布都会被判"漂移"。
        let confirmed =
            commit_published_status(&conn, &[strict_publication("item-drifted", 2)]).unwrap();
        assert!(
            confirmed.is_empty(),
            "版本一致时必须确认成功：{confirmed:?}"
        );
        let status: String = conn
            .query_row(
                "SELECT status FROM library_items_v2 WHERE id = ?1",
                rusqlite::params!["item-drifted"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "published");

        let _ = fs::remove_dir_all(root);
    }

    fn test_export_bundle(root: &Path) -> (PathBuf, crate::reading_source_v2::ReadingExamSourceV2) {
        let mut authoring_value: Value = serde_json::from_str(include_str!(
            "../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
        ))
        .unwrap();
        authoring_value["quality"]["state"] = json!("ready");
        authoring_value["quality"]["issues"] = json!([]);
        let authoring: IeltsAuthoringIRV2 =
            serde_json::from_value(authoring_value.clone()).unwrap();
        let source = compile_reading_source_v2(&authoring).unwrap();
        let source_value = serde_json::to_value(&source).unwrap();
        let export_dir = root.join("export");
        fs::create_dir_all(&export_dir).unwrap();
        fs::create_dir_all(root.join("jobs").join(&authoring.job_id)).unwrap();
        fs::write(
            export_dir.join("authoring-ir-v2.json"),
            canonical_json_bytes(&authoring_value).unwrap(),
        )
        .unwrap();
        let source_path = export_dir.join("reading-source-v2.json");
        let source_bytes = canonical_json_bytes(&source_value).unwrap();
        fs::write(&source_path, &source_bytes).unwrap();
        fs::write(
            root.join("jobs")
                .join(&authoring.job_id)
                .join(AUTHORING_V2_SHADOW_FILE),
            canonical_json_bytes(&authoring_value).unwrap(),
        )
        .unwrap();
        let proof =
            validate_authoring_v2_publish_readiness(root, &authoring.job_id, 0, &authoring_value)
                .unwrap();
        let manifest = json!({
            "schemaVersion": "AuthoringV2ExportReceiptV1",
            "jobId": authoring.job_id,
            "examId": authoring.exam.exam_id,
            "revision": 0,
            "sourceDocumentId": authoring.source_document_id,
            "files": ["authoring-ir-v2.json", "reading-source-v2.json", "manifest-v2.json"],
            "authoringSha256": sha256_hex(&canonical_json_bytes(&authoring_value).unwrap()),
            "runtimeSha256": sha256_hex(&source_bytes),
            "assetCount": authoring.assets.len(),
            "assets": authoring.assets,
            "v1FilesRemainReadable": true,
            "pdfPerQuestionLlmRepair": false,
            "reviewRequired": false,
            "publishProof": proof
        });
        fs::write(
            export_dir.join("manifest-v2.json"),
            canonical_json_bytes(&manifest).unwrap(),
        )
        .unwrap();
        (source_path, source)
    }

    /// A listening export bundle: the canonical listening draft, its compiled
    /// `ListeningExamSourceV1`, the four section audio files and the export receipt the
    /// publisher demands before it will touch the NAS root.
    fn test_listening_export_bundle(root: &Path) -> PathBuf {
        let authoring = crate::test_support::complete_listening_exam();
        let mut authoring_value = serde_json::to_value(&authoring).unwrap();
        authoring_value["quality"]["state"] = json!("ready");
        authoring_value["quality"]["issues"] = json!([]);
        let authoring: IeltsAuthoringIRV2 =
            serde_json::from_value(authoring_value.clone()).unwrap();

        let compiled = crate::listening_source_v1::compile_exam_source_v2(&authoring)
            .expect("the listening paper compiles");
        assert_eq!(compiled.modality(), "listening");
        assert_eq!(compiled.file_name(), "listening-source-v1.json");
        let source_value = compiled.document().unwrap();
        let source_bytes = canonical_json_bytes(&source_value).unwrap();

        let export_dir = root.join("export");
        fs::create_dir_all(&export_dir).unwrap();
        fs::create_dir_all(root.join("jobs").join(&authoring.job_id)).unwrap();
        // The packager resolves every asset relative to the runtime source's own directory.
        let staged_audio = crate::test_support::stage_listening_audio(&export_dir, &[1, 2, 3, 4]);
        assert_eq!(staged_audio.len(), 4);

        fs::write(
            export_dir.join("authoring-ir-v2.json"),
            canonical_json_bytes(&authoring_value).unwrap(),
        )
        .unwrap();
        let source_path = export_dir.join("listening-source-v1.json");
        fs::write(&source_path, &source_bytes).unwrap();
        fs::write(
            root.join("jobs")
                .join(&authoring.job_id)
                .join(AUTHORING_V2_SHADOW_FILE),
            canonical_json_bytes(&authoring_value).unwrap(),
        )
        .unwrap();

        let proof =
            validate_authoring_v2_publish_readiness(root, &authoring.job_id, 0, &authoring_value)
                .unwrap();
        let manifest = json!({
            "schemaVersion": "AuthoringV2ExportReceiptV1",
            "jobId": authoring.job_id,
            "examId": authoring.exam.exam_id,
            "revision": 0,
            "sourceDocumentId": authoring.source_document_id,
            "files": ["authoring-ir-v2.json", "listening-source-v1.json", "manifest-v2.json"],
            "authoringSha256": sha256_hex(&canonical_json_bytes(&authoring_value).unwrap()),
            "runtimeSha256": sha256_hex(&source_bytes),
            "assetCount": authoring.assets.len(),
            "assets": authoring.assets,
            "v1FilesRemainReadable": true,
            "pdfPerQuestionLlmRepair": false,
            "reviewRequired": false,
            "publishProof": proof
        });
        fs::write(
            export_dir.join("manifest-v2.json"),
            canonical_json_bytes(&manifest).unwrap(),
        )
        .unwrap();
        source_path
    }

    /// The whole point of T2's packaging half: a listening paper must reach the NAS root as a
    /// listening paper, with every section's own audio present and hash-checked. Before the
    /// single compile entry point existed there was no way for a listening paper to get here at
    /// all — the package builder only knew how to publish a `ReadingExamSourceV2`.
    #[test]
    fn a_listening_paper_packages_every_section_audio_into_the_nas_root() {
        let root = temp_root();
        let source_path = test_listening_export_bundle(&root);
        let nas_parent = root.join("nas");
        let result = publish_nas_package_v2_core(
            &root,
            json!({
                "libraryRoot": nas_parent,
                "sourcePath": source_path
            }),
        )
        .unwrap();
        assert_eq!(result["status"], "committed");

        // The manifest entry has to say what this paper actually is. Writing a hard-coded
        // "ReadingExamSourceV2" here is exactly the drift this task removes.
        let manifest = load_existing_manifest(&nas_parent.join("manifest.js")).unwrap();
        let entry = manifest
            .get("early-approaches")
            .unwrap_or_else(|| panic!("the exam is in the manifest: {manifest:?}"));
        assert_eq!(entry["schemaVersion"], "ListeningExamSourceV1", "{entry}");
        assert_eq!(entry["modality"], "listening", "{entry}");
        assert_eq!(
            entry["category"],
            Value::Null,
            "a listening paper has no passage category"
        );
        assert_eq!(entry["resourcesBase"], "./resources/early-approaches/");

        // Four sections, four files, each byte-identical to what the descriptor promised.
        let resource_dir = nas_parent.join("resources").join("early-approaches");
        for ordinal in 1..=4 {
            let descriptor = crate::test_support::listening_audio_asset(ordinal);
            let staged = resource_dir.join(&descriptor.relative_path);
            let bytes =
                fs::read(&staged).unwrap_or_else(|error| panic!("{}: {error}", staged.display()));
            assert_eq!(sha256_hex(&bytes), descriptor.sha256, "section {ordinal}");
            assert_eq!(
                bytes.len() as u64,
                descriptor.byte_length,
                "section {ordinal}"
            );
        }

        let asset_manifest: Value =
            serde_json::from_slice(&fs::read(resource_dir.join(ASSET_MANIFEST_FILE_NAME)).unwrap())
                .unwrap();
        assert_eq!(asset_manifest["schemaVersion"], "ExamAssetManifestV2");
        assert_eq!(asset_manifest["examId"], "early-approaches");
        assert_eq!(
            asset_manifest["assets"].as_object().unwrap().len(),
            4,
            "every section's audio must be listed, not just the first"
        );

        // The wrapper is modality-agnostic on purpose: the student registry is the same one,
        // only the payload inside it differs (parts instead of a passage).
        let script = fs::read_to_string(nas_parent.join("early-approaches.js"))
            .unwrap_or_else(|error| panic!("the exam script must be readable: {error}"));
        assert!(
            script.contains("__READING_EXAM_DATA__.register("),
            "{script}"
        );
        assert!(script.contains("\"ListeningExamSourceV1\""), "{script}");
        assert!(
            !script.contains("\"passage\""),
            "a listening payload has no passage: {script}"
        );

        let _ = fs::remove_dir_all(root);
    }

    /// Deleting or never uploading one section's audio must stop the package, not silently
    /// publish three sections plus a hole.
    #[test]
    fn a_listening_paper_whose_audio_is_absent_is_refused_rather_than_published_silent() {
        let root = temp_root();
        let source_path = test_listening_export_bundle(&root);
        let missing = crate::test_support::listening_audio_asset(3);
        fs::remove_file(root.join("export").join(&missing.relative_path)).unwrap();

        let nas_parent = root.join("nas");
        let error = publish_nas_package_v2_core(
            &root,
            json!({
                "libraryRoot": nas_parent,
                "sourcePath": source_path
            }),
        )
        .unwrap_err();
        assert!(
            error.starts_with("nas_package_v2_asset_copy")
                || error.starts_with("nas_package_v2_asset_source"),
            "{error}"
        );
        assert!(
            !nas_parent.join("manifest.js").exists(),
            "no discovery manifest may be committed for a paper we cannot ship"
        );
        assert!(!nas_parent
            .join("resources")
            .join("early-approaches")
            .exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn package_path_policy_rejects_unsafe_relative_paths() {
        for path in [
            "../x.png",
            "/x.png",
            "C:/x.png",
            "images/⁄x.png",
            "images//x.png",
        ] {
            assert!(validate_package_relative_path(path).is_err(), "{path}");
        }
        assert!(validate_package_relative_path("images/x.png").is_ok());
    }

    #[test]
    fn manifest_cas_rejects_stale_writer() {
        let root = temp_root();
        let manifest = root.join("manifest.js");
        fs::write(&manifest, "window.__READING_EXAM_MANIFEST__ = {};\n").unwrap();
        assert!(verify_manifest_compare_and_swap(&manifest, &"0".repeat(64)).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn existing_manifest_parser_accepts_legacy_closure_wrapper() {
        let root = temp_root();
        let manifest = root.join("manifest.js");
        fs::write(
            &manifest,
            "(function register(global) { global.__READING_EXAM_MANIFEST__ = {\"legacy\": {\"schemaVersion\": \"ReadingExamManifestV1\"}}; })(window);\n",
        )
        .unwrap();
        let parsed = load_existing_manifest(&manifest).unwrap();
        assert_eq!(
            parsed
                .get("legacy")
                .and_then(Value::as_object)
                .and_then(|value| value.get("schemaVersion")),
            Some(&Value::String("ReadingExamManifestV1".to_string()))
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn publish_normalizes_publish_child_to_one_transaction_root() {
        let root = temp_root();
        let (source_path, _source) = test_export_bundle(&root);

        let nas_parent = root.join("nas");
        let publish_child = nas_parent.join("publish");
        let result = publish_nas_package_v2_core(
            &root,
            json!({
                "libraryRoot": publish_child,
                "sourcePath": source_path
            }),
        )
        .unwrap();
        assert_eq!(result["status"], "committed");
        assert!(nas_parent.join("manifest.js").is_file());
        assert!(nas_parent.join("early-approaches.js").is_file());
        assert!(nas_parent
            .join(CONTROL_DIR_NAME)
            .join("export.lock")
            .is_file());
        assert!(!publish_child.join("manifest.js").exists());
        assert!(!publish_child.join(CONTROL_DIR_NAME).exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn publisher_rejects_valid_json_without_v2_export_binding() {
        let root = temp_root();
        let authoring_value: Value = serde_json::from_str(include_str!(
            "../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
        ))
        .unwrap();
        let authoring: IeltsAuthoringIRV2 = serde_json::from_value(authoring_value).unwrap();
        let source = compile_reading_source_v2(&authoring).unwrap();
        let source_path = root.join("reading-source-v2.json");
        fs::write(&source_path, serde_json::to_vec(&source).unwrap()).unwrap();

        let error = publish_nas_package_v2_core(
            &root,
            json!({
                "libraryRoot": root.join("library"),
                "sourcePath": source_path
            }),
        )
        .unwrap_err();
        assert!(
            error.starts_with("nas_package_v2_export_receipt_missing:"),
            "{error}"
        );
        assert!(!root.join("library").join("manifest.js").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_version_policy_blocks_invalid_or_future_student_runtime() {
        assert!(validate_minimum_runtime_version("0.2.0").is_ok());
        assert!(validate_minimum_runtime_version("0.1.9").is_ok());
        assert!(validate_minimum_runtime_version("future").is_err());
        assert!(validate_minimum_runtime_version("0.3.0").is_err());
    }

    #[test]
    fn package_publish_probes_before_commit_and_fault_rolls_back_old_manifest() {
        let root = temp_root();
        let (source_path, _source) = test_export_bundle(&root);
        let library_root = root.join("library");
        let first = publish_nas_package_v2_core(
            &root,
            json!({
                "libraryRoot": library_root,
                "sourcePath": source_path,
                "minimumRuntimeVersion": "0.2.0"
            }),
        )
        .unwrap();
        assert_eq!(first["status"], "committed");
        let manifest_path = library_root.join("manifest.js");
        let old_manifest = fs::read(&manifest_path).unwrap();
        let old_source = fs::read(library_root.join("early-approaches.js")).unwrap();

        let error = publish_nas_package_v2_core(
            &root,
            json!({
                "libraryRoot": library_root,
                "sourcePath": source_path,
                "fault": "manifest_rename"
            }),
        )
        .unwrap_err();
        assert!(error.contains("rollback=complete"), "{error}");
        assert_eq!(fs::read(&manifest_path).unwrap(), old_manifest);
        assert_eq!(
            fs::read(library_root.join("early-approaches.js")).unwrap(),
            old_source
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn partial_resource_backup_fault_restores_old_visible_package() {
        let root = temp_root();
        let (source_path, _source) = test_export_bundle(&root);
        let library_root = root.join("library");
        publish_nas_package_v2_core(
            &root,
            json!({
                "libraryRoot": library_root,
                "sourcePath": source_path
            }),
        )
        .unwrap();

        let manifest_path = library_root.join("manifest.js");
        let exam_path = library_root.join("early-approaches.js");
        let resource_path = library_root
            .join("resources")
            .join("early-approaches")
            .join(ASSET_MANIFEST_FILE_NAME);
        let old_manifest = fs::read(&manifest_path).unwrap();
        let old_exam = fs::read(&exam_path).unwrap();
        let old_asset_manifest = fs::read(&resource_path).unwrap();

        let error = publish_nas_package_v2_core(
            &root,
            json!({
                "libraryRoot": library_root,
                "sourcePath": source_path,
                "fault": "backup_resources"
            }),
        )
        .unwrap_err();

        assert!(
            error.contains("nas_package_v2_fault_backup_resources"),
            "{error}"
        );
        assert!(error.contains("rollback=complete"), "{error}");
        assert_eq!(fs::read(&manifest_path).unwrap(), old_manifest);
        assert_eq!(fs::read(&exam_path).unwrap(), old_exam);
        assert_eq!(fs::read(&resource_path).unwrap(), old_asset_manifest);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn post_commit_report_failure_rolls_back_package_and_report() {
        let root = temp_root();
        let (source_path, _source) = test_export_bundle(&root);
        let library_root = root.join("library");
        let first = publish_nas_package_v2_core(
            &root,
            json!({
                "libraryRoot": library_root,
                "sourcePath": source_path
            }),
        )
        .unwrap();
        let old_manifest = fs::read(library_root.join("manifest.js")).unwrap();
        let old_exam = fs::read(library_root.join("early-approaches.js")).unwrap();
        let report_path = PathBuf::from(first["reportPath"].as_str().unwrap());
        let old_report = fs::read(&report_path).unwrap();

        let error = publish_nas_package_v2_core(
            &root,
            json!({
                "libraryRoot": library_root,
                "sourcePath": source_path,
                "fault": "report_write"
            }),
        )
        .unwrap_err();
        assert!(
            error.contains("nas_package_v2_fault_report_write"),
            "{error}"
        );
        assert!(error.contains("rollback=complete"), "{error}");
        assert_eq!(
            fs::read(library_root.join("manifest.js")).unwrap(),
            old_manifest
        );
        assert_eq!(
            fs::read(library_root.join("early-approaches.js")).unwrap(),
            old_exam
        );
        assert_eq!(fs::read(&report_path).unwrap(), old_report);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn v2_commit_fault_preserves_loadable_legacy_v1_manifest() {
        let root = temp_root();
        let (source_path, _source) = test_export_bundle(&root);

        let library_root = root.join("library");
        fs::create_dir_all(&library_root).unwrap();
        let legacy_manifest = json!({
            "legacy-v1": {
                "examId": "legacy-v1",
                "dataKey": "legacy-v1",
                "script": "./legacy-v1.js",
                "title": "Legacy V1",
                "category": "P1"
            },
            "_meta": {
                "schemaVersion": "ReadingExamManifestV1",
                "assetCount": 1
            }
        });
        let legacy_manifest_js = format!(
            "window.__READING_EXAM_MANIFEST__ = {};\n",
            serde_json::to_string_pretty(&legacy_manifest).unwrap()
        );
        let manifest_path = library_root.join("manifest.js");
        fs::write(&manifest_path, &legacy_manifest_js).unwrap();
        let legacy_exam_path = library_root.join("legacy-v1.js");
        fs::write(&legacy_exam_path, "legacy-v1-wrapper").unwrap();

        let error = publish_nas_package_v2_core(
            &root,
            json!({
                "libraryRoot": library_root,
                "sourcePath": source_path,
                "fault": "manifest_rename"
            }),
        )
        .unwrap_err();

        assert!(error.contains("rollback=complete"), "{error}");
        assert_eq!(
            fs::read(&manifest_path).unwrap(),
            legacy_manifest_js.as_bytes()
        );
        assert_eq!(
            load_existing_manifest(&manifest_path).unwrap(),
            legacy_manifest.as_object().unwrap().clone()
        );
        assert_eq!(fs::read(&legacy_exam_path).unwrap(), b"legacy-v1-wrapper");
        assert!(!library_root.join("early-approaches.js").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn package_publish_rejects_busy_lock_before_commit() {
        let root = temp_root();
        let (source_path, source) = test_export_bundle(&root);

        let library_root = root.join("library");
        let lock_path = make_paths(&library_root, &library_root, &source.exam_id)
            .unwrap()
            .lock_path;
        fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)
            .unwrap();
        lock.try_lock_exclusive().unwrap();

        let error = publish_nas_package_v2_core(
            &root,
            json!({
                "libraryRoot": library_root,
                "sourcePath": source_path
            }),
        )
        .unwrap_err();

        assert!(error.starts_with("nas_package_v2_lock_busy:"), "{error}");
        assert!(!library_root.join("manifest.js").exists());
        assert!(!library_root.join("early-approaches.js").exists());

        lock.unlock().unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn commit_cas_rejects_manifest_changed_after_staging_baseline() {
        let root = temp_root();
        let manifest = root.join("manifest.js");
        fs::write(&manifest, "old").unwrap();
        let baseline = manifest_sha256(&manifest).unwrap();
        fs::write(&manifest, "changed").unwrap();
        let error = verify_manifest_compare_and_swap(&manifest, &baseline).unwrap_err();
        assert!(
            error.starts_with("nas_package_v2_manifest_conflict:"),
            "{error}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recovery_restores_persistent_backup_after_manifest_commit_interruption() {
        let root = temp_root();
        let library_root = root.join("library");
        let reading_root = library_root.clone();
        let exam_id = "recovery-exam";
        let export_id = "recovery-export";
        let mut paths = make_paths(&library_root, &reading_root, exam_id).unwrap();
        paths.staging_root = reading_root.join(format!(".phase6-staging-{export_id}"));
        paths.backup_root = library_root
            .join(CONTROL_DIR_NAME)
            .join(BACKUP_DIR_NAME)
            .join(format!("{exam_id}-{export_id}"));
        paths.journal_path = library_root
            .join(CONTROL_DIR_NAME)
            .join(format!("{exam_id}-{export_id}.journal.json"));
        fs::create_dir_all(&paths.backup_root).unwrap();
        fs::create_dir_all(paths.resource_path.parent().unwrap()).unwrap();
        fs::create_dir_all(&paths.resource_path).unwrap();
        fs::write(&paths.exam_path, "old-exam").unwrap();
        fs::write(&paths.resource_path.join("asset.bin"), "old-asset").unwrap();
        fs::write(&paths.manifest_path, "old-manifest").unwrap();
        let state = json!({
            "schemaVersion": "NasBackupStateV1",
            "hadExam": true,
            "hadResources": true,
            "hadManifest": true
        });
        write_synced_file(
            &paths.backup_root.join("state.json"),
            &canonical_json_bytes(&state).unwrap(),
        )
        .unwrap();
        fs::rename(&paths.exam_path, paths.backup_root.join("exam.js")).unwrap();
        fs::create_dir_all(paths.backup_root.join("resources")).unwrap();
        fs::rename(
            &paths.resource_path,
            paths.backup_root.join("resources").join(exam_id),
        )
        .unwrap();
        fs::rename(&paths.manifest_path, paths.backup_root.join("manifest.js")).unwrap();
        fs::write(&paths.exam_path, "new-exam").unwrap();
        fs::create_dir_all(&paths.resource_path).unwrap();
        fs::write(&paths.resource_path.join("asset.bin"), "new-asset").unwrap();
        fs::write(&paths.manifest_path, "new-manifest").unwrap();
        fs::create_dir_all(&paths.staging_root).unwrap();
        let journal = json!({
            "schemaVersion": "NasCommitJournalV1",
            "exportId": export_id,
            "examId": exam_id,
            "status": "manifest_committed"
        });
        write_synced_file(
            &paths.journal_path,
            &canonical_json_bytes(&journal).unwrap(),
        )
        .unwrap();

        recover_incomplete_transactions(&paths).unwrap();

        assert_eq!(fs::read_to_string(&paths.exam_path).unwrap(), "old-exam");
        assert_eq!(
            fs::read_to_string(paths.resource_path.join("asset.bin")).unwrap(),
            "old-asset"
        );
        assert_eq!(
            fs::read_to_string(&paths.manifest_path).unwrap(),
            "old-manifest"
        );
        assert!(!paths.backup_root.exists());
        assert!(!paths.staging_root.exists());
        let recovered: Value =
            serde_json::from_str(&fs::read_to_string(&paths.journal_path).unwrap()).unwrap();
        assert_eq!(recovered["status"], "recovered");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recovery_keeps_visible_package_when_no_backup_moves_completed() {
        let root = temp_root();
        let library_root = root.join("library");
        let reading_root = library_root.clone();
        let exam_id = "incomplete-backup-exam";
        let export_id = "incomplete-backup-export";
        let mut paths = make_paths(&library_root, &reading_root, exam_id).unwrap();
        paths.staging_root = reading_root.join(format!(".phase6-staging-{export_id}"));
        paths.backup_root = library_root
            .join(CONTROL_DIR_NAME)
            .join(BACKUP_DIR_NAME)
            .join(format!("{exam_id}-{export_id}"));
        paths.journal_path = library_root
            .join(CONTROL_DIR_NAME)
            .join(format!("{exam_id}-{export_id}.journal.json"));

        fs::create_dir_all(paths.resource_path.parent().unwrap()).unwrap();
        fs::create_dir_all(&paths.resource_path).unwrap();
        fs::write(&paths.exam_path, "old-exam").unwrap();
        fs::write(paths.resource_path.join("asset.bin"), "old-asset").unwrap();
        fs::write(&paths.manifest_path, "old-manifest").unwrap();

        // Simulate a crash immediately after state.json was durable, before
        // any of the old files reached the backup directory.
        fs::create_dir_all(&paths.backup_root).unwrap();
        let state = json!({
            "schemaVersion": "NasBackupStateV1",
            "hadExam": true,
            "hadResources": true,
            "hadManifest": true,
            "hadReport": false
        });
        write_synced_file(
            &paths.backup_root.join("state.json"),
            &canonical_json_bytes(&state).unwrap(),
        )
        .unwrap();
        let journal = json!({
            "schemaVersion": "NasCommitJournalV1",
            "exportId": export_id,
            "examId": exam_id,
            "status": "committing"
        });
        write_synced_file(
            &paths.journal_path,
            &canonical_json_bytes(&journal).unwrap(),
        )
        .unwrap();

        recover_incomplete_transactions(&paths).unwrap();
        assert_eq!(fs::read_to_string(&paths.exam_path).unwrap(), "old-exam");
        assert_eq!(
            fs::read_to_string(paths.resource_path.join("asset.bin")).unwrap(),
            "old-asset"
        );
        assert_eq!(
            fs::read_to_string(&paths.manifest_path).unwrap(),
            "old-manifest"
        );
        assert!(!paths.backup_root.exists());
        let recovered: Value =
            serde_json::from_str(&fs::read_to_string(&paths.journal_path).unwrap()).unwrap();
        assert_eq!(recovered["status"], "recovered");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recovery_restores_partial_exam_backup_and_keeps_other_live_items() {
        let root = temp_root();
        let library_root = root.join("library");
        let reading_root = library_root.clone();
        let exam_id = "partial-backup-exam";
        let export_id = "partial-backup-export";
        let mut paths = make_paths(&library_root, &reading_root, exam_id).unwrap();
        paths.staging_root = reading_root.join(format!(".phase6-staging-{export_id}"));
        paths.backup_root = library_root
            .join(CONTROL_DIR_NAME)
            .join(BACKUP_DIR_NAME)
            .join(format!("{exam_id}-{export_id}"));
        paths.journal_path = library_root
            .join(CONTROL_DIR_NAME)
            .join(format!("{exam_id}-{export_id}.journal.json"));

        fs::create_dir_all(paths.resource_path.parent().unwrap()).unwrap();
        fs::create_dir_all(&paths.resource_path).unwrap();
        fs::write(paths.resource_path.join("asset.bin"), "old-asset").unwrap();
        fs::write(&paths.manifest_path, "old-manifest").unwrap();
        fs::create_dir_all(&paths.backup_root).unwrap();
        fs::write(paths.backup_root.join("exam.js"), "old-exam").unwrap();
        let state = json!({
            "schemaVersion": "NasBackupStateV1",
            "hadExam": true,
            "hadResources": true,
            "hadManifest": true,
            "hadReport": false
        });
        write_synced_file(
            &paths.backup_root.join("state.json"),
            &canonical_json_bytes(&state).unwrap(),
        )
        .unwrap();
        let journal = json!({
            "schemaVersion": "NasCommitJournalV1",
            "exportId": export_id,
            "examId": exam_id,
            "status": "committing"
        });
        write_synced_file(
            &paths.journal_path,
            &canonical_json_bytes(&journal).unwrap(),
        )
        .unwrap();

        recover_incomplete_transactions(&paths).unwrap();
        assert_eq!(fs::read_to_string(&paths.exam_path).unwrap(), "old-exam");
        assert_eq!(
            fs::read_to_string(paths.resource_path.join("asset.bin")).unwrap(),
            "old-asset"
        );
        assert_eq!(
            fs::read_to_string(&paths.manifest_path).unwrap(),
            "old-manifest"
        );
        assert!(!paths.backup_root.exists());

        let _ = fs::remove_dir_all(root);
    }

    fn batch_control_dir(library_root: &Path, batch_id: &str) -> PathBuf {
        library_root
            .join(CONTROL_DIR_NAME)
            .join(BACKUP_DIR_NAME)
            .join(format!("batch-{batch_id}"))
    }

    fn write_batch_backup_state(
        dir: &Path,
        committed: bool,
        exams: Value,
        pending_manifest_sha256: Option<&str>,
    ) {
        fs::create_dir_all(dir).unwrap();
        let state = json!({
            "schemaVersion": "NasBatchBackupStateV1",
            "manifestCommitted": committed,
            "hadManifest": true,
            "pendingManifestSha256": pending_manifest_sha256,
            "exams": exams,
        });
        fs::write(
            dir.join("state.json"),
            canonical_json_bytes(&state).unwrap(),
        )
        .unwrap();
    }

    /// 崩溃恢复同样必须备份感知：state 记录存在但没有备份 = 尚未移动。
    #[test]
    fn batch_recovery_keeps_live_resources_when_state_precedes_the_move() {
        let root = temp_root();
        let library_root = root.join("library");
        let reading_root = nas_reading_exams_dir(&library_root);
        let paths = make_paths(&library_root, &reading_root, "batch").unwrap();
        let live = reading_root.join("resources").join("v2-p1");
        fs::create_dir_all(&live).unwrap();
        fs::write(live.join("asset.bin"), "old-asset").unwrap();
        let dir = batch_control_dir(&library_root, "deadbeef");
        write_batch_backup_state(
            &dir,
            false,
            json!([{"examId": "v2-p1", "hadResources": true}]),
            None,
        );

        recover_interrupted_batches(&paths, &library_root.join(CONTROL_DIR_NAME)).unwrap();

        assert_eq!(
            fs::read_to_string(live.join("asset.bin")).unwrap(),
            "old-asset",
            "recovery must not delete resources it never backed up"
        );
        let _ = fs::remove_dir_all(root);
    }

    /// 清单替换后进程可能在写入 `manifestCommitted` 之前崩溃。恢复端必须靠
    /// 磁盘事实判定已提交，否则会回滚掉一个已经生效的包。
    ///
    /// 夹具必须复刻生产语义：提交用的是 `atomic_replace_file`，它把候选清单
    /// **移走**。早期版本用 `fs::write` 造一份 release 副本，恰好让「提交后
    /// release 里没有清单」这一真实情况消失，于是把 bug 掩盖掉了。
    #[test]
    fn batch_recovery_detects_commit_from_disk_and_keeps_the_package() {
        let root = temp_root();
        let library_root = root.join("library");
        let reading_root = nas_reading_exams_dir(&library_root);
        let paths = make_paths(&library_root, &reading_root, "batch").unwrap();
        let live = reading_root.join("resources").join("v2-p1");
        fs::create_dir_all(&live).unwrap();
        fs::write(live.join("asset.bin"), "new-asset").unwrap();
        fs::create_dir_all(&paths.manifest_path.parent().unwrap()).unwrap();
        let candidate = b"window.__READING_EXAM_MANIFEST__ = {};\n";
        let release = reading_root.join("releases").join("deadbeef");
        fs::create_dir_all(&release).unwrap();
        fs::write(release.join("manifest.js"), candidate).unwrap();
        let dir = batch_control_dir(&library_root, "deadbeef");
        // 状态文件停在「未提交」——正是崩溃窗口里的样子；候选哈希已记录。
        write_batch_backup_state(
            &dir,
            false,
            json!([{"examId": "v2-p1", "hadResources": true}]),
            Some(&sha256_hex(candidate)),
        );
        // 复刻生产的提交动作：移动（不是复制）候选清单到线上路径。
        atomic_replace_file(&release.join("manifest.js"), &paths.manifest_path).unwrap();
        assert!(
            !release.join("manifest.js").exists(),
            "提交后 release 里不应再有候选清单——恢复逻辑不能依赖它"
        );

        recover_interrupted_batches(&paths, &library_root.join(CONTROL_DIR_NAME)).unwrap();

        assert_eq!(
            fs::read_to_string(live.join("asset.bin")).unwrap(),
            "new-asset",
            "a committed batch must keep its live resources"
        );
        assert_eq!(fs::read(&paths.manifest_path).unwrap(), candidate);
        assert!(!dir.exists(), "committed batch backup must be cleaned up");
        let _ = fs::remove_dir_all(root);
    }

    /// 旧版本写下的状态文件没有 `pendingManifestSha256`。此时仍必须能判定「清单已被
    /// 本批次替换」——否则那些批次会被误判为未提交，把已经生效的包回滚掉。
    #[test]
    fn batch_recovery_detects_commit_without_a_recorded_hash() {
        let root = temp_root();
        let library_root = root.join("library");
        let reading_root = nas_reading_exams_dir(&library_root);
        let paths = make_paths(&library_root, &reading_root, "batch").unwrap();
        let live = reading_root.join("resources").join("v2-p1");
        fs::create_dir_all(&live).unwrap();
        fs::write(live.join("asset.bin"), "new-asset").unwrap();
        fs::create_dir_all(&paths.manifest_path.parent().unwrap()).unwrap();
        let new_manifest = b"window.__READING_EXAM_MANIFEST__ = {\"batch\":\"new\"};\n";
        fs::write(&paths.manifest_path, new_manifest).unwrap();
        let dir = batch_control_dir(&library_root, "deadbeef");
        fs::create_dir_all(&dir).unwrap();
        // 基线：批次开始前的旧清单，与线上不同 → 说明清单已被替换。
        fs::write(
            dir.join("manifest.js"),
            b"window.__READING_EXAM_MANIFEST__ = {\"batch\":\"old\"};\n",
        )
        .unwrap();
        // 旧格式状态：没有 pendingManifestSha256，且 manifestCommitted 仍是 false。
        fs::write(
            dir.join("state.json"),
            canonical_json_bytes(&json!({
                "schemaVersion": "NasBatchBackupStateV1",
                "manifestCommitted": false,
                "hadManifest": true,
                "exams": [{"examId": "v2-p1", "hadResources": true}],
            }))
            .unwrap(),
        )
        .unwrap();

        recover_interrupted_batches(&paths, &library_root.join(CONTROL_DIR_NAME)).unwrap();

        assert_eq!(
            fs::read_to_string(live.join("asset.bin")).unwrap(),
            "new-asset",
            "a committed batch must keep its live resources even without a recorded hash"
        );
        assert_eq!(
            fs::read(&paths.manifest_path).unwrap(),
            new_manifest,
            "a replaced manifest must not be reverted to the baseline"
        );
        assert!(!dir.exists(), "committed batch backup must be cleaned up");
        let _ = fs::remove_dir_all(root);
    }

    /// 反方向：线上清单与基线**相同**（未被替换）且没有候选哈希 → 未提交，必须还原。
    #[test]
    fn batch_recovery_restores_when_the_manifest_still_matches_the_baseline() {
        let root = temp_root();
        let library_root = root.join("library");
        let reading_root = nas_reading_exams_dir(&library_root);
        let paths = make_paths(&library_root, &reading_root, "batch").unwrap();
        let live = reading_root.join("resources").join("v2-p1");
        fs::create_dir_all(&live).unwrap();
        fs::write(live.join("asset.bin"), "new-asset").unwrap();
        fs::create_dir_all(&paths.manifest_path.parent().unwrap()).unwrap();
        let manifest = b"window.__READING_EXAM_MANIFEST__ = {\"batch\":\"same\"};\n";
        fs::write(&paths.manifest_path, manifest).unwrap();
        let dir = batch_control_dir(&library_root, "deadbeef");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("manifest.js"), manifest).unwrap();
        let backup_resource = dir.join("resources").join("v2-p1");
        fs::create_dir_all(&backup_resource).unwrap();
        fs::write(backup_resource.join("asset.bin"), "old-asset").unwrap();
        fs::write(
            dir.join("state.json"),
            canonical_json_bytes(&json!({
                "schemaVersion": "NasBatchBackupStateV1",
                "manifestCommitted": false,
                "hadManifest": true,
                "exams": [{"examId": "v2-p1", "hadResources": true}],
            }))
            .unwrap(),
        )
        .unwrap();

        recover_interrupted_batches(&paths, &library_root.join(CONTROL_DIR_NAME)).unwrap();

        assert_eq!(
            fs::read_to_string(live.join("asset.bin")).unwrap(),
            "old-asset",
            "an uncommitted batch must restore the backed-up resources"
        );
        let _ = fs::remove_dir_all(root);
    }

    /// 未提交且确有备份时必须还原旧资源，并清理 staging/release。
    #[test]
    fn batch_recovery_restores_backed_up_resources_when_not_committed() {
        let root = temp_root();
        let library_root = root.join("library");
        let reading_root = nas_reading_exams_dir(&library_root);
        let paths = make_paths(&library_root, &reading_root, "batch").unwrap();
        let live = reading_root.join("resources").join("v2-p1");
        fs::create_dir_all(&live).unwrap();
        fs::write(live.join("asset.bin"), "new-asset").unwrap();
        let dir = batch_control_dir(&library_root, "deadbeef");
        // 候选哈希与线上清单不同 → 未提交。用不同的哈希而不是 None，
        // 才能证明提交点判定真的在比较内容，而不是缺字段时凑巧走对分支。
        write_batch_backup_state(
            &dir,
            false,
            json!([{"examId": "v2-p1", "hadResources": true}]),
            Some(&sha256_hex(b"a-different-candidate-manifest")),
        );
        let backup_resource = dir.join("resources").join("v2-p1");
        fs::create_dir_all(&backup_resource).unwrap();
        fs::write(backup_resource.join("asset.bin"), "old-asset").unwrap();
        fs::create_dir_all(reading_root.join(".batch-staging-deadbeef")).unwrap();
        fs::create_dir_all(reading_root.join("releases").join("deadbeef")).unwrap();

        recover_interrupted_batches(&paths, &library_root.join(CONTROL_DIR_NAME)).unwrap();

        assert_eq!(
            fs::read_to_string(live.join("asset.bin")).unwrap(),
            "old-asset",
            "uncommitted batch must restore the backed-up resources"
        );
        assert!(!reading_root.join(".batch-staging-deadbeef").exists());
        assert!(!reading_root.join("releases").join("deadbeef").exists());
        assert!(!dir.exists());
        let _ = fs::remove_dir_all(root);
    }
}
