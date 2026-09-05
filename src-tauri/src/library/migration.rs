//! M1（原 P2-T02）：旧题迁移到 `library_items_v2`。
//!
//! 候选优先级（计划 §11.2 M2）：当前 V2 revision → `authoring-ir-v2.shadow.json`
//! →（V1 转换后续接入）→ 无候选时保留 `migration_required` 外壳。
//! 幂等：`upsert_item_shell` 不覆盖已存在行；`seed_canonical_ds` 只填空缺权威稿，
//! 已被用户编辑的稿件永不被迁移覆盖。

use std::fs;
use std::path::Path;

use serde_json::Value;

use super::repository::{get_item, seed_canonical_ds, upsert_item_shell, UpsertItemInput};
use crate::authoring_v2_commands::AUTHORING_V2_SHADOW_FILE;
use crate::util::job_dir;
use crate::CommandResult;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MigrationReportV1 {
    pub scanned_jobs: usize,
    pub created_shells: usize,
    pub seeded_ds: usize,
    pub migration_required: usize,
    pub skipped_existing: usize,
}

/// 读取一个 job 的迁移候选稿：优先 current revision，其次 V2 shadow。
fn candidate_authoring(job_path: &Path) -> Option<(Value, &'static str)> {
    let revision_path = job_path.join("authoring").join("current-revision.json");
    if let Ok(bytes) = fs::read(&revision_path) {
        if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
            // current-revision.json 是 `{ "authoring": ..., "current": ... }` 形态时取 authoring；
            // 否则假定它本身就是 authoring 稿。
            let authoring = value
                .get("authoring")
                .cloned()
                .filter(|candidate| candidate.is_object())
                .unwrap_or(value);
            if is_authoring_shape(&authoring) {
                return Some((authoring, "revision"));
            }
        }
    }
    let shadow_path = job_path.join(AUTHORING_V2_SHADOW_FILE);
    if let Ok(bytes) = fs::read(&shadow_path) {
        if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
            if is_authoring_shape(&value) {
                return Some((value, "shadow"));
            }
        }
    }
    None
}

/// 最小形态校验：拒绝把 job.json/compare 报告误当题稿。
fn is_authoring_shape(value: &Value) -> bool {
    value.get("schemaVersion").and_then(Value::as_str) == Some("IeltsAuthoringIRV2")
        || (value.get("exam").is_some() && value.get("taskGroups").is_some())
}

fn title_of(job_json: &Value, fallback: &str) -> String {
    job_json
        .get("title")
        .and_then(Value::as_str)
        .filter(|title| !title.trim().is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn status_of(job_json: &Value, has_ds: bool) -> &'static str {
    if !has_ds {
        return "migration_required";
    }
    match job_json.get("status").and_then(Value::as_str) {
        Some("Exported") | Some("Cleaned") => "published",
        Some("NeedsReview") | Some("Working") => "action_required",
        Some("DraftSaved") | Some("ExportReady") => "ready",
        Some("Failed") => "failed",
        _ => "action_required",
    }
}

/// 扫描 `<root>/jobs/` 并迁移。幂等：可重复执行；已存在行与已有权威稿不会被覆盖。
pub(crate) fn migrate_existing_items(root: &Path) -> CommandResult<MigrationReportV1> {
    let jobs_root = root.join("jobs");
    let mut report = MigrationReportV1 {
        scanned_jobs: 0,
        created_shells: 0,
        seeded_ds: 0,
        migration_required: 0,
        skipped_existing: 0,
    };
    let entries = match fs::read_dir(&jobs_root) {
        Ok(entries) => entries,
        Err(_) => return Ok(report),
    };
    let conn = super::repository::open_library_connection(root)?;
    for entry in entries.flatten() {
        let job_path = entry.path();
        let job_id = entry.file_name().to_string_lossy().to_string();
        if job_id.starts_with('.') || !job_path.is_dir() {
            continue;
        }
        // Writing 与未来 modality 各有独立目录；本迁移只处理 reading 的 jobs/。
        if let Ok(existing) = get_item(&conn, &job_id) {
            if existing.is_some() {
                report.skipped_existing += 1;
                report.scanned_jobs += 1;
                continue;
            }
        }
        report.scanned_jobs += 1;

        let job_json: Value = fs::read(job_path.join("job.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or(Value::Null);
        let title = title_of(&job_json, &job_id);
        let candidate = candidate_authoring(&job_path);
        let status = status_of(&job_json, candidate.is_some());
        upsert_item_shell(
            &conn,
            &UpsertItemInput {
                id: &job_id,
                modality: "reading",
                title: &title,
                status,
                source_asset_id: None,
            },
        )?;
        report.created_shells += 1;
        if let Some((authoring, _source)) = candidate {
            if seed_canonical_ds(&conn, &job_id, &authoring.to_string(), status)? {
                report.seeded_ds += 1;
            }
        } else {
            report.migration_required += 1;
        }
    }
    Ok(report)
}

/// 迁移单个 item（get_workspace_item 首次访问的按需填充）：返回是否填充了权威稿。
pub(crate) fn migrate_single_item(root: &Path, job_id: &str) -> CommandResult<bool> {
    let job_path = job_dir(root, job_id);
    let conn = super::repository::open_library_connection(root)?;
    if let Some(existing) = get_item(&conn, job_id)? {
        if existing.has_canonical_ds {
            return Ok(false);
        }
    }
    let Some((authoring, _source)) = candidate_authoring(&job_path) else {
        return Ok(false);
    };
    if get_item(&conn, job_id)?.is_none() {
        let job_json: Value = fs::read(job_path.join("job.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or(Value::Null);
        upsert_item_shell(
            &conn,
            &UpsertItemInput {
                id: job_id,
                modality: "reading",
                title: &title_of(&job_json, job_id),
                status: status_of(&job_json, true),
                source_asset_id: None,
            },
        )?;
    }
    seed_canonical_ds(&conn, job_id, &authoring.to_string(), "action_required")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::repository::{get_canonical_ds, get_item};
    use uuid::Uuid;

    fn temp_root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("library-migrate-{}", Uuid::new_v4().simple()))
    }

    fn seed_job(root: &Path, job_id: &str, with_shadow: bool) {
        let dir = crate::util::job_dir(root, job_id);
        fs::create_dir_all(&dir).unwrap();
        let job = serde_json::json!({
            "jobId": job_id,
            "title": "Migration Paper",
            "status": "NeedsReview"
        });
        fs::write(dir.join("job.json"), serde_json::to_vec(&job).unwrap()).unwrap();
        if with_shadow {
            let authoring = serde_json::json!({
                "schemaVersion": "IeltsAuthoringIRV2",
                "exam": { "title": "Migration Paper" },
                "taskGroups": []
            });
            fs::write(
                dir.join(AUTHORING_V2_SHADOW_FILE),
                serde_json::to_vec(&authoring).unwrap(),
            )
            .unwrap();
        }
    }

    #[test]
    fn migration_is_idempotent_and_never_overwrites_user_edits() {
        let root = temp_root();
        crate::util::ensure_app_dirs(&root).unwrap();
        seed_job(&root, "job-a", true);
        seed_job(&root, "job-b", false);

        let report = migrate_existing_items(&root).unwrap();
        assert_eq!(report.scanned_jobs, 2);
        assert_eq!(report.created_shells, 2);
        assert_eq!(report.seeded_ds, 1);
        assert_eq!(report.migration_required, 1);

        // 用户随后编辑了 job-a（推进版本，模拟 apply_editor_commands 之后的行）。
        {
            let conn = super::super::repository::open_library_connection(&root).unwrap();
            conn.execute(
                "UPDATE library_items_v2 SET current_edit_version = 7 WHERE id = 'job-a'",
                [],
            )
            .unwrap();
        }

        // 重复迁移：不重建、不覆盖，job-b 仍无稿。
        let second = migrate_existing_items(&root).unwrap();
        assert_eq!(second.skipped_existing, 2);
        assert_eq!(second.created_shells, 0);
        assert_eq!(second.seeded_ds, 0);

        let conn = super::super::repository::open_library_connection(&root).unwrap();
        let item = get_item(&conn, "job-a").unwrap().unwrap();
        assert_eq!(item.current_edit_version, 7, "用户编辑不得被迁移覆盖");
        let (_, version) = get_canonical_ds(&conn, "job-a").unwrap().unwrap();
        assert_eq!(version, 7, "迁移后版本保持用户编辑后的值，不重置");
        let _ = fs::remove_dir_all(&root);
    }
}
