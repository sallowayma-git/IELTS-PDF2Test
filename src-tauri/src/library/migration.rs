//! M1（原 P2-T02）：旧题迁移到 `library_items_v2`。
//!
//! 候选优先级（计划 §11.2 M2）：当前 V2 revision → `authoring-ir-v2.shadow.json`
//! →（V1 转换后续接入）→ 无候选时保留 `migration_required` 外壳。
//! 幂等：`upsert_item_shell` 不覆盖已存在行；`seed_canonical_ds` 只填空缺权威稿，
//! 已被用户编辑的稿件永不被迁移覆盖。
//!
//! 两个入口，职责分离：
//! - [`ensure_initial_canonical`]：**后台导入链**的首次初始化（只播种，不做兼容修复），
//!   由 `processing::scheduler::run_job_inner` 在冻结本地基线**之前**调用；
//! - [`migrate_single_item`]：存量数据的**按需**入口（工作区打开、发布预检），
//!   已有稿时才会走 `repair_shadow_seed` 兼容修复。

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
fn candidate_authoring(root: &Path, job_id: &str) -> Option<(Value, &'static str)> {
    if let Ok(current) = crate::artifact_store::read_current_revision(root, job_id) {
        if let Ok(authoring) = crate::artifact_store::read_revision(root, job_id, current.revision) {
            if is_authoring_shape(&authoring) {
                return Some((authoring, "revision"));
            }
        }
    }
    let shadow_path = job_dir(root, job_id).join(AUTHORING_V2_SHADOW_FILE);
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

/// The library row carries the modality the user confirmed at import. Local recognition
/// still builds a reading-shaped draft, so the first seed adopts the row's modality: a
/// listening import must never surface as a reading draft.
pub(crate) fn align_draft_modality(authoring: &mut Value, item_modality: &str) {
    if item_modality == "listening" {
        if let Some(object) = authoring.as_object_mut() {
            object.insert("modality".to_string(), Value::String("listening".to_string()));
        }
    }
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

fn repair_shadow_seed(conn: &rusqlite::Connection, root: &Path, job_id: &str) -> CommandResult<bool> {
    let Some((mut revision, "revision")) = candidate_authoring(root, job_id) else { return Ok(false) };
    let mut shadow: Option<Value> = fs::read(job_dir(root, job_id).join(AUTHORING_V2_SHADOW_FILE))
        .ok().and_then(|bytes| serde_json::from_slice(&bytes).ok());
    if let Some(item) = get_item(conn, job_id)? {
        align_draft_modality(&mut revision, &item.modality);
        if let Some(shadow) = shadow.as_mut() { align_draft_modality(shadow, &item.modality); }
    }
    let Some((current, 1)) = super::repository::get_canonical_ds(conn, job_id)? else { return Ok(false) };
    if shadow.as_ref() != Some(&current) || current == revision { return Ok(false); }
    let updated = conn.execute(
        "UPDATE library_items_v2 SET canonical_ds_json = ?2,
         title = CASE WHEN title = ?3 THEN ?4 ELSE title END
         WHERE id = ?1 AND current_edit_version = 1 AND canonical_ds_json = ?5
         AND NOT EXISTS (SELECT 1 FROM editor_journal_v1 WHERE library_item_id = ?1)",
        rusqlite::params![job_id, revision.to_string(), current.pointer("/exam/title").and_then(Value::as_str),
            revision.pointer("/exam/title").and_then(Value::as_str), current.to_string()],
    ).map_err(|error| format!("library_migration_repair:{error}"))?;
    Ok(updated > 0)
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
                if repair_shadow_seed(&conn, root, &job_id)? { report.seeded_ds += 1; }
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
        let candidate = candidate_authoring(root, &job_id);
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

/// **首次初始化**首稿（幂等）：把已成功产出的本地 artifact 播种成权威稿。
///
/// 这是后台导入链的初始化入口。它与 [`migrate_single_item`] 的区别在于**不做存量
/// 兼容修复**（[`repair_shadow_seed`]）：那一步会把 job 目录里既有的旧 shadow 写回
/// 权威稿，是给历史数据准备的修复路径；新导入若走它，一次迟到的 shadow 覆写就会
/// 盖掉用户编辑。新导入只需要「没有稿就播种」这一件事。
///
/// 幂等性由 [`seed_canonical_ds`] 的 `canonical_ds_json IS NULL` 写入条件保证：
/// 已有稿（先前导入播的、或用户编辑过的）一律原样保留，重复调用零副作用。
///
/// 返回值语义（调用方必须区分，不得混淆）：
/// - `Ok(true)`：调用后权威稿**可用**；
/// - `Ok(false)`：artifact 里没有可辨认的题稿候选（如非阅读稿）——没什么可播；
/// - `Err`：DB / 文件读取失败——**本该有稿却读不到**，一律上抛，不得用 `false` 冒充。
pub(crate) fn ensure_initial_canonical(root: &Path, job_id: &str) -> CommandResult<bool> {
    let conn = super::repository::open_library_connection(root)?;
    if let Some(existing) = get_item(&conn, job_id)? {
        if existing.has_canonical_ds {
            return Ok(true);
        }
    }
    let Some((mut authoring, _source)) = candidate_authoring(root, job_id) else {
        // 没有候选：不建壳、不写稿，如实返回 false。
        return Ok(false);
    };
    if let Some(item) = get_item(&conn, job_id)? {
        align_draft_modality(&mut authoring, &item.modality);
    }
    if get_item(&conn, job_id)?.is_none() {
        let job_json: Value = fs::read(job_dir(root, job_id).join("job.json"))
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
    seed_canonical_ds(&conn, job_id, &authoring.to_string(), "action_required")?;
    // 写入条件带 `IS NULL`：并发的另一次播种可能先到，因此以**重新读取**为准，
    // 而不是把 `seed_canonical_ds` 的返回值当成「现在有没有稿」。
    Ok(get_item(&conn, job_id)?.map(|item| item.has_canonical_ds).unwrap_or(false))
}

/// 迁移单个 item（按需填充入口：工作区首次访问 / 发布预检）。
///
/// 返回是否**本次填充**了权威稿。已有稿的条目只走存量兼容修复，返回值是该修复是否
/// 生效；没有稿的条目委托给首次初始化 [`ensure_initial_canonical`]。
pub(crate) fn migrate_single_item(root: &Path, job_id: &str) -> CommandResult<bool> {
    let conn = super::repository::open_library_connection(root)?;
    if let Some(existing) = get_item(&conn, job_id)? {
        if existing.has_canonical_ds {
            return repair_shadow_seed(&conn, root, job_id);
        }
    }
    drop(conn);
    ensure_initial_canonical(root, job_id)
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

    #[test]
    fn migration_resolves_saved_revision_and_repairs_unedited_shadow_seed() {
        let root = temp_root();
        crate::util::ensure_app_dirs(&root).unwrap();
        seed_job(&root, "job-a", true);
        migrate_existing_items(&root).unwrap();
        let conn = super::super::repository::open_library_connection(&root).unwrap();
        let (mut saved, _) = get_canonical_ds(&conn, "job-a").unwrap().unwrap();
        saved["exam"]["title"] = serde_json::json!("Saved revision");
        crate::artifact_store::append_revision(&root, "job-a", 0,
            crate::artifact_store::RevisionSourceV2::User, &saved, &[]).unwrap();
        assert!(migrate_single_item(&root, "job-a").unwrap());
        assert_eq!(get_canonical_ds(&conn, "job-a").unwrap().unwrap().0, saved);
        conn.execute("DELETE FROM library_items_v2 WHERE id = 'job-a'", []).unwrap();
        migrate_existing_items(&root).unwrap();
        assert_eq!(get_canonical_ds(&conn, "job-a").unwrap().unwrap().0, saved);
        let _ = fs::remove_dir_all(root);
    }

    /// G1/A7-F02 故障注入：用户编辑 canonical 之后，迟到的识别收尾
    /// （调度器 set_item_status_ready 会重跑 migrate_single_item）与迟到的
    /// 云端候选都不得覆盖用户编辑。锁定「user_edited 只增不覆盖」不变量。
    /// 强化版：先有真实 current revision（内容不同），用户编辑带 request_id
    /// （journal 落库），再跑迟到迁移——canonical、version、journal 全部保留。
    #[test]
    fn late_pipeline_completion_preserves_user_edited_canonical() {
        let root = temp_root();
        crate::util::ensure_app_dirs(&root).unwrap();
        seed_job(&root, "job-a", true);
        migrate_existing_items(&root).unwrap();

        // 用户编辑标题（真实编辑事务路径，带 request_id → journal 落库）。
        let conn = super::super::repository::open_library_connection(&root).unwrap();
        let input = crate::library::repository::ApplyEditorCommandsInput {
            item_id: "job-a".into(),
            base_version: 1,
            request_id: Some("e2e-journal-1".into()),
            commands: vec![],
            title: Some("用户改的标题".into()),
        };
        let mut tx_conn = conn;
        crate::library::repository::apply_editor_commands_tx(
            &mut tx_conn,
            &input,
            &|_, _| Ok(()),
            &|_| Ok(()),
        )
        .unwrap();

        // 迟到的识别收尾（模拟）：追加一条内容不同的 current revision——
        // 与 canonical 不一致的迟到结果同样不得改写权威稿。
        let mut late_revision = get_canonical_ds(&tx_conn, "job-a").unwrap().unwrap().0;
        late_revision["exam"]["title"] = serde_json::json!("迟到识别的不同内容");
        crate::artifact_store::append_revision(
            &root,
            "job-a",
            0,
            crate::artifact_store::RevisionSourceV2::AutoExtract,
            &late_revision,
            &[],
        )
        .unwrap();

        // 迟到的识别收尾：调度器在 ready 阶段会再次调用 migrate_single_item
        // （返回值 false = 无需修复，不表示失败；关键是不覆盖 canonical）。
        let _ = migrate_single_item(&root, "job-a").unwrap();
        let (ds, version) = get_canonical_ds(&tx_conn, "job-a").unwrap().unwrap();
        assert_eq!(
            ds.pointer("/exam/title").and_then(Value::as_str),
            Some("用户改的标题"),
            "迟到收尾/迟到 revision 不得覆盖用户编辑"
        );
        assert_eq!(version, 2, "用户编辑推进的版本不得被重置");

        // journal 保留：编辑事务的幂等重放记录不得被迟到迁移清掉。
        use rusqlite::OptionalExtension;
        let journal: Option<(String, i64)> = tx_conn
            .query_row(
                "SELECT command_json, base_version FROM editor_journal_v1 WHERE request_id = 'e2e-journal-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .unwrap();
        let (_, journal_base) = journal.expect("迟到迁移必须保留 editor journal");
        assert_eq!(journal_base, 1, "journal 的 base_version 不得被改写");

        // 幂等重放仍生效：同一 request_id 重放命中，不产生新版本。
        let replay = crate::library::repository::apply_editor_commands_tx(
            &mut tx_conn,
            &input,
            &|_, _| Ok(()),
            &|_| Ok(()),
        )
        .unwrap();
        assert!(replay.replayed, "迟到迁移后 journal 幂等重放必须仍可用");
        let (_, version_after_replay) = get_canonical_ds(&tx_conn, "job-a").unwrap().unwrap();
        assert_eq!(version_after_replay, 2);
        let _ = fs::remove_dir_all(&root);
    }

    /// 复核 B/D：真实"迟到识别"是覆写 job 目录的 shadow 文件
    /// （auto_pipeline 写 AUTHORING_V2_SHADOW_FILE），不走 append_revision。
    /// 用户编辑后 shadow 被迟到覆写，迁移必须拒绝把 shadow 内容写回 canonical。
    #[test]
    fn late_shadow_overwrite_does_not_reach_user_edited_canonical() {
        use crate::authoring_v2_commands::AUTHORING_V2_SHADOW_FILE;
        let root = temp_root();
        crate::util::ensure_app_dirs(&root).unwrap();
        seed_job(&root, "job-a", true);
        migrate_existing_items(&root).unwrap();

        // 用户编辑标题（版本推进到 2）。
        let conn = super::super::repository::open_library_connection(&root).unwrap();
        let input = crate::library::repository::ApplyEditorCommandsInput {
            item_id: "job-a".into(),
            base_version: 1,
            request_id: None,
            commands: vec![],
            title: Some("用户改的标题".into()),
        };
        let mut tx_conn = conn;
        crate::library::repository::apply_editor_commands_tx(
            &mut tx_conn,
            &input,
            &|_, _| Ok(()),
            &|_| Ok(()),
        )
        .unwrap();

        // 迟到识别覆写 shadow 文件（不同内容）。
        let mut late_shadow = get_canonical_ds(&tx_conn, "job-a").unwrap().unwrap().0;
        late_shadow["exam"]["title"] = serde_json::json!("迟到识别覆写的 shadow");
        let shadow_path = crate::util::job_dir(&root, "job-a").join(AUTHORING_V2_SHADOW_FILE);
        fs::write(&shadow_path, serde_json::to_vec(&late_shadow).unwrap()).unwrap();

        // 迟到收尾：迁移重跑，canonical 必须保持用户编辑。
        let _ = migrate_single_item(&root, "job-a").unwrap();
        let (ds, version) = get_canonical_ds(&tx_conn, "job-a").unwrap().unwrap();
        assert_eq!(
            ds.pointer("/exam/title").and_then(Value::as_str),
            Some("用户改的标题"),
            "迟到覆写的 shadow 不得写回用户编辑过的 canonical"
        );
        assert_eq!(version, 2);
        let _ = fs::remove_dir_all(&root);
    }

    /// 首次初始化：无稿则播种、有稿则原样保留；**绝不做存量兼容修复**。
    ///
    /// 与 `migrate_single_item` 的分工是这条测试的核心。后者对已有稿会走
    /// `repair_shadow_seed`（把 job 目录里的 shadow 写回 canonical），那是给历史数据
    /// 的修复路径；后台导入链若走它，一次迟到的 shadow 覆写就会盖掉用户编辑。
    #[test]
    fn ensure_initial_canonical_seeds_once_and_never_repairs_over_user_edits() {
        let root = temp_root();
        crate::util::ensure_app_dirs(&root).unwrap();
        seed_job(&root, "job-a", true);

        // (1) 无 DB 行 + 有 artifact ⇒ 建壳并播种。
        assert!(ensure_initial_canonical(&root, "job-a").unwrap());
        {
            let conn = super::super::repository::open_library_connection(&root).unwrap();
            let item = get_item(&conn, "job-a").unwrap().expect("必须建立外壳行");
            assert!(item.has_canonical_ds, "必须播种出权威稿");
            assert_eq!(item.current_edit_version, 1);
        }

        // (2) 用户编辑（真实编辑事务，版本推进到 2）。
        let conn = super::super::repository::open_library_connection(&root).unwrap();
        let mut tx_conn = conn;
        crate::library::repository::apply_editor_commands_tx(
            &mut tx_conn,
            &crate::library::repository::ApplyEditorCommandsInput {
                item_id: "job-a".into(),
                base_version: 1,
                request_id: None,
                commands: vec![],
                title: Some("用户改的标题".into()),
            },
            &|_, _| Ok(()),
            &|_| Ok(()),
        )
        .unwrap();

        // (3) 迟到识别覆写 shadow（内容不同）。
        let mut late_shadow = serde_json::json!({
            "schemaVersion": "IeltsAuthoringIRV2",
            "exam": { "title": "迟到识别覆写的 shadow" },
            "taskGroups": []
        });
        late_shadow["exam"]["title"] = serde_json::json!("迟到识别覆写的 shadow");
        fs::write(
            crate::util::job_dir(&root, "job-a").join(AUTHORING_V2_SHADOW_FILE),
            serde_json::to_vec(&late_shadow).unwrap(),
        )
        .unwrap();

        // (4) 再次初始化：幂等，且**不得**把 shadow 写回 canonical。
        assert!(ensure_initial_canonical(&root, "job-a").unwrap());
        let (ds, version) = get_canonical_ds(&tx_conn, "job-a").unwrap().unwrap();
        assert_eq!(
            ds.pointer("/exam/title").and_then(Value::as_str),
            Some("用户改的标题"),
            "首次初始化绝不覆盖已有稿（含用户编辑），也不得走 shadow 修复路径"
        );
        assert_eq!(version, 2, "重复初始化不得重置用户编辑推进的版本");

        // (5) 对照：同一个方向上 `migrate_single_item` 才会尝试兼容修复（这里因
        // canonical 已被编辑、且 journal 非空而不动），证明两条路径确实不同。
        let _ = migrate_single_item(&root, "job-a").unwrap();
        let (ds_after, version_after) = get_canonical_ds(&tx_conn, "job-a").unwrap().unwrap();
        assert_eq!(ds_after.pointer("/exam/title").and_then(Value::as_str), Some("用户改的标题"));
        assert_eq!(version_after, 2);

        let _ = fs::remove_dir_all(&root);
    }

    /// 没有 artifact 候选时必须如实返回 `false`，且**不留下**空壳行——
    /// 空壳行会让题库多出一条永远打不开的条目。
    #[test]
    fn ensure_initial_canonical_reports_false_without_a_candidate() {
        let root = temp_root();
        crate::util::ensure_app_dirs(&root).unwrap();
        seed_job(&root, "job-empty", false);

        assert!(!ensure_initial_canonical(&root, "job-empty").unwrap());
        let conn = super::super::repository::open_library_connection(&root).unwrap();
        assert!(
            get_item(&conn, "job-empty").unwrap().is_none(),
            "没有候选时不得建壳"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// 预检不得比发布更严。`publish_items_core` 在读权威稿之前会先跑
    /// `migrate_single_item` 把它播种出来，所以「只有影子稿、还没有题库行」的条目
    /// 对发布是**可发**的。预检若跳过这一步就会报 `ITEM_DS_NOT_SEEDED`，
    /// 把一件能发布的事说成不能发布——反方向的假信号，同样是界面与门禁不一致。
    #[test]
    fn publish_preflight_is_not_stricter_than_publish_for_an_unmigrated_item() {
        let root = temp_root();
        crate::util::ensure_app_dirs(&root).unwrap();
        seed_job(&root, "job-shadow-only", true);

        let result =
            crate::authoring_v2_commands::get_publish_preflight_core(&root, "job-shadow-only")
                .unwrap();
        let codes = result["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|blocker| blocker["code"].as_str())
            .collect::<Vec<_>>();
        assert!(
            !codes.contains(&"ITEM_DS_NOT_SEEDED"),
            "preflight must not block an item that publishing would migrate and accept: {codes:?}"
        );
        // 预检顺手把权威稿播种出来了——这正是发布路径依赖的前置条件。
        let conn = crate::library::repository::open_library_connection(&root).unwrap();
        assert!(
            get_canonical_ds(&conn, "job-shadow-only").unwrap().is_some(),
            "the preflight must leave the item in the state publishing expects"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// 反方向：连影子稿都没有的条目，发布必然以 `ITEM_DS_NOT_SEEDED` 失败，
    /// 预检必须报出同一个码，而不是给一个绿色的空列表。
    #[test]
    fn publish_preflight_reports_the_publish_blocker_when_there_is_nothing_to_migrate() {
        let root = temp_root();
        crate::util::ensure_app_dirs(&root).unwrap();
        seed_job(&root, "job-empty", false);

        let result =
            crate::authoring_v2_commands::get_publish_preflight_core(&root, "job-empty").unwrap();
        assert_eq!(result["passed"], serde_json::json!(false));
        let codes = result["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|blocker| blocker["code"].as_str())
            .collect::<Vec<_>>();
        assert!(codes.contains(&"ITEM_DS_NOT_SEEDED"), "{codes:?}");
        let _ = fs::remove_dir_all(&root);
    }
}
