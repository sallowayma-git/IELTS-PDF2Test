//! User confirmation is a library fact, independent of content versions and publish gates.
use std::path::Path;

use rusqlite::{params, Connection, TransactionBehavior};
use serde_json::{json, Value};

use super::repository::{get_item, open_library_connection};
use crate::CommandResult;

/// Kernel file lock coordinates confirmation with the entire NAS publication,
/// including other app processes. A crashed process releases the lock automatically.
pub(crate) struct PublicationGuard(std::fs::File);
impl PublicationGuard {
    pub(crate) fn acquire(root: &Path) -> CommandResult<Self> {
        std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(root.join(".library-confirmation-publish.lock"))
            .map_err(|e| format!("library_confirmation_lock:{e}"))?;
        fs2::FileExt::lock_exclusive(&file)
            .map_err(|e| format!("library_confirmation_lock:{e}"))?;
        Ok(Self(file))
    }
}
impl Drop for PublicationGuard {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

fn item_in_current_paper(conn: &Connection, item_id: &str) -> CommandResult<bool> {
    let exam_id: Option<String> = conn.query_row(
        "SELECT CASE WHEN json_valid(canonical_ds_json) THEN json_extract(canonical_ds_json,'$.exam.examId') END
         FROM library_items_v2 WHERE id=?1", [item_id], |row| row.get(0),
    ).map_err(|e| format!("library_confirmation_exam:{e}"))?;
    let mut statement = conn.prepare(
        "SELECT destination FROM paper_archive_destinations UNION SELECT destination FROM publish_records_v2 UNION SELECT destination FROM papers"
    ).map_err(|e| e.to_string())?;
    let destinations = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    for destination in destinations {
        let destination = destination.map_err(|e| e.to_string())?;
        let normalized =
            crate::export_nas_library::normalize_nas_library_root(Path::new(&destination));
        let reading = crate::export_nas_library::nas_reading_exams_dir(&normalized);
        match std::fs::read(reading.join(crate::nas_package_v2::CURRENT_PAPER_FILE_NAME)) {
            Ok(bytes) => {
                let pointer: Value = serde_json::from_slice(&bytes).map_err(|e| {
                    format!("LIBRARY_ITEM_NOT_CONFIRMABLE:current_paper_invalid:{e}")
                })?;
                if pointer["schemaVersion"] != "CurrentPaperV1"
                    || ["reading", "listening"].iter().any(|kind| {
                        pointer[*kind].as_array().is_none_or(|parts| {
                            parts.iter().any(|part| {
                                part["examId"]
                                    .as_str()
                                    .is_none_or(|id| id.trim().is_empty())
                            })
                        })
                    })
                {
                    return Err("LIBRARY_ITEM_NOT_CONFIRMABLE:current_paper_invalid".into());
                }
                if ["reading", "listening"].iter().any(|kind| {
                    pointer[*kind].as_array().is_some_and(|parts| {
                        parts.iter().any(|p| {
                            p["examId"].as_str() == exam_id.as_deref() && exam_id.is_some()
                        })
                    })
                }) {
                    return Ok(true);
                }
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && std::fs::read_dir(&reading).is_ok() => {}
            Err(_) => {
                // Only use the archived pointer when its NAS location cannot be read.
                let archived:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM paper_items i JOIN papers p USING(paper_id)
                  WHERE p.status='current' AND p.archive_state='confirmed' AND (p.destination=?2 OR p.destination=?3)
                    AND (i.library_item_id=?1 OR i.exam_id=?4))",
                    params![item_id,destination,normalized.to_string_lossy(),exam_id],|r|r.get(0)).map_err(|e|e.to_string())?;
                if archived {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

fn update_confirmation(conn: &Connection, item_id: &str, confirmed: bool) -> CommandResult<()> {
    let row = get_item(conn, item_id)?.ok_or("LIBRARY_ITEM_NOT_CONFIRMABLE:not_found")?;
    if !confirmed && (row.status.starts_with("published") || item_in_current_paper(conn, item_id)?)
    {
        return Err("LIBRARY_ITEM_IN_USE".into());
    }
    if confirmed {
        let reason = if row.deleted_at.is_some() {
            Some("deleted")
        } else if row.status == "processing" {
            Some("processing")
        } else if row.status == "failed" {
            Some("failed")
        } else if !row.has_canonical_ds {
            Some("canonical_missing")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(format!("LIBRARY_ITEM_NOT_CONFIRMABLE:{reason}"));
        }
        let processing:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM processing_jobs_v2 WHERE library_item_id=?1
        AND stage IN ('queued','running','preparing_source','local_recognition','cloud_recognition','reconciling'))",
        [item_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        if processing {
            return Err("LIBRARY_ITEM_NOT_CONFIRMABLE:processing".into());
        }
        // Empty strings are not a draft, even if a legacy writer stored one.
        let has_draft: bool = conn
            .query_row(
                "SELECT length(trim(canonical_ds_json))>0 FROM library_items_v2 WHERE id=?1",
                [item_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !has_draft {
            return Err("LIBRARY_ITEM_NOT_CONFIRMABLE:canonical_missing".into());
        }
    }
    conn.execute(
        "UPDATE library_items_v2 SET confirmed_at=CASE WHEN ?2 THEN COALESCE(confirmed_at,?3) ELSE NULL END,
         confirmed_by=CASE WHEN ?2 THEN COALESCE(confirmed_by,'user') ELSE NULL END WHERE id=?1",
        params![item_id, confirmed, chrono::Utc::now().to_rfc3339()],
    ).map_err(|e| format!("library_confirmation_update:{e}"))?;
    Ok(())
}

pub(crate) fn set_library_item_confirmed_core(
    root: &Path,
    item_id: &str,
    confirmed: bool,
) -> CommandResult<Value> {
    let _publication_guard = PublicationGuard::acquire(root)?;
    let mut conn = open_library_connection(root)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    update_confirmation(&tx, item_id, confirmed)?;
    let row = super::commands::library_row_value(
        &tx,
        get_item(&tx, item_id)?.ok_or("LIBRARY_ITEM_NOT_CONFIRMABLE:not_found")?,
    )?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(row)
}

pub(crate) fn set_library_items_confirmed_core(
    root: &Path,
    item_ids: &[String],
    confirmed: bool,
) -> CommandResult<Value> {
    let _publication_guard = PublicationGuard::acquire(root)?;
    let mut conn = open_library_connection(root)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let mut results = Vec::new();
    for id in item_ids {
        match update_confirmation(&tx, id, confirmed) {
            Ok(()) => results.push(json!({"itemId":id,"ok":true,"row":super::commands::library_row_value(&tx,get_item(&tx,id)?.unwrap())?})),
            Err(error) if error.starts_with("LIBRARY_ITEM_") => results.push(json!({"itemId":id,"ok":false,"error":error})),
            Err(error) => return Err(error),
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(json!({"results":results}))
}

pub(crate) fn import_local_confirmed_ids_core(
    root: &Path,
    item_ids: &[String],
) -> CommandResult<Value> {
    let result = set_library_items_confirmed_core(root, item_ids, true)?;
    let accepted: std::collections::BTreeSet<&str> = result["results"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["ok"] == true)
        .filter_map(|r| r["itemId"].as_str())
        .collect();
    Ok(json!(accepted))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::repository::{seed_canonical_ds, upsert_item_shell, UpsertItemInput};

    fn fixture() -> (std::path::PathBuf, Connection) {
        let root = std::env::temp_dir().join(format!("confirm-{}", uuid::Uuid::new_v4()));
        crate::util::ensure_app_dirs(&root).unwrap();
        let conn = open_library_connection(&root).unwrap();
        for (id, status, draft) in [
            ("ready", "action_required", true),
            ("empty", "ready", false),
            ("processing", "processing", true),
            ("failed", "failed", true),
            ("deleted", "ready", true),
            ("published", "published_forced", true),
        ] {
            upsert_item_shell(
                &conn,
                &UpsertItemInput {
                    id,
                    modality: "reading",
                    title: id,
                    status,
                    source_asset_id: None,
                },
            )
            .unwrap();
            if draft {
                seed_canonical_ds(
                    &conn,
                    id,
                    &json!({"exam":{"examId":id},"taskGroups":[]}).to_string(),
                    status,
                )
                .unwrap();
            }
        }
        conn.execute(
            "UPDATE library_items_v2 SET deleted_at='trash' WHERE id='deleted'",
            [],
        )
        .unwrap();
        (root, conn)
    }

    #[test]
    fn confirmation_is_idempotent_persistent_and_does_not_edit_or_reorder() {
        let (root, conn) = fixture();
        let before = get_item(&conn, "ready").unwrap().unwrap();
        let row = set_library_item_confirmed_core(&root, "ready", true).unwrap();
        assert!(row["confirmedAt"].is_string());
        assert_eq!(row["questionTypes"], json!([]));
        assert_eq!(row["examId"], "ready");
        assert_eq!(
            set_library_item_confirmed_core(&root, "ready", true).unwrap()["confirmedAt"],
            row["confirmedAt"]
        );
        let reopened = open_library_connection(&root).unwrap();
        let after = get_item(&reopened, "ready").unwrap().unwrap();
        assert_eq!(before.updated_at, after.updated_at);
        assert_eq!(before.status, after.status);
        assert_eq!(before.current_edit_version, after.current_edit_version);
        assert_eq!(
            reopened
                .query_row("SELECT COUNT(*) FROM editor_journal_v1", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            super::super::commands::list_library_items_core(&root, false)
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["id"] == "ready")
                .unwrap()["confirmedAt"],
            row["confirmedAt"]
        );
        let wrapper = crate::library_commands::get_library_row_core(&root, "ready").unwrap();
        assert_eq!(wrapper["item"]["confirmedAt"], row["confirmedAt"]);
        assert!(
            set_library_item_confirmed_core(&root, "ready", false).unwrap()["confirmedAt"]
                .is_null()
        );
    }

    #[test]
    fn confirmation_rejects_ineligible_rows_and_batch_reports_each_result() {
        let (root, conn) = fixture();
        for (id, reason) in [
            ("absent", "not_found"),
            ("empty", "canonical_missing"),
            ("processing", "processing"),
            ("failed", "failed"),
            ("deleted", "deleted"),
        ] {
            assert_eq!(
                set_library_item_confirmed_core(&root, id, true).unwrap_err(),
                format!("LIBRARY_ITEM_NOT_CONFIRMABLE:{reason}")
            );
        }
        conn.execute(
            "UPDATE library_items_v2 SET canonical_ds_json='' WHERE id='empty'",
            [],
        )
        .unwrap();
        assert_eq!(
            set_library_item_confirmed_core(&root, "empty", true).unwrap_err(),
            "LIBRARY_ITEM_NOT_CONFIRMABLE:canonical_missing"
        );
        assert_eq!(
            set_library_item_confirmed_core(&root, "published", false).unwrap_err(),
            "LIBRARY_ITEM_IN_USE"
        );
        let ids = vec![
            "ready".into(),
            "absent".into(),
            "ready".into(),
            "processing".into(),
        ];
        let result = set_library_items_confirmed_core(&root, &ids, true).unwrap();
        assert_eq!(result["results"][0]["ok"], true);
        assert_eq!(
            result["results"][1]["error"],
            "LIBRARY_ITEM_NOT_CONFIRMABLE:not_found"
        );
        assert_eq!(
            result["results"][2]["row"]["confirmedAt"],
            result["results"][0]["row"]["confirmedAt"]
        );
        assert_eq!(
            import_local_confirmed_ids_core(&root, &ids).unwrap(),
            json!(["ready"])
        );
        assert_eq!(
            import_local_confirmed_ids_core(&root, &ids).unwrap(),
            json!(["ready"])
        );
        assert_eq!(
            import_local_confirmed_ids_core(&root, &[]).unwrap(),
            json!([])
        );
    }

    #[test]
    fn cancellation_checks_live_pointer_even_after_status_changed_or_archive_is_missing() {
        let (root, conn) = fixture();
        set_library_item_confirmed_core(&root, "ready", true).unwrap();
        let nas = root.join("nas");
        let reading = crate::export_nas_library::nas_reading_exams_dir(&nas);
        std::fs::create_dir_all(&reading).unwrap();
        std::fs::write(
            reading.join("current-paper.json"),
            json!({"schemaVersion":"CurrentPaperV1","reading":[{"examId":"ready"}],"listening":[]})
                .to_string(),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO paper_archive_destinations VALUES(?1)",
            [nas.to_string_lossy().as_ref()],
        )
        .unwrap();
        assert_eq!(
            set_library_item_confirmed_core(&root, "ready", false).unwrap_err(),
            "LIBRARY_ITEM_IN_USE"
        );
        std::fs::remove_file(reading.join("current-paper.json")).unwrap();
        assert!(set_library_item_confirmed_core(&root, "ready", false).is_ok());
        set_library_item_confirmed_core(&root, "ready", true).unwrap();
        std::fs::write(reading.join("current-paper.json"), "{}").unwrap();
        assert_eq!(
            set_library_item_confirmed_core(&root, "ready", false).unwrap_err(),
            "LIBRARY_ITEM_NOT_CONFIRMABLE:current_paper_invalid"
        );
    }

    #[test]
    fn trash_restore_preserves_confirmation_and_content_replacement_clears_it() {
        let (root, conn) = fixture();
        let row = set_library_item_confirmed_core(&root, "ready", true).unwrap();
        crate::db::soft_delete_library_item(&conn, "ready").unwrap();
        assert_eq!(
            get_item(&conn, "ready")
                .unwrap()
                .unwrap()
                .confirmed_at
                .as_deref(),
            row["confirmedAt"].as_str()
        );
        crate::db::restore_library_item(&conn, "ready").unwrap();
        assert_eq!(
            get_item(&conn, "ready")
                .unwrap()
                .unwrap()
                .confirmed_at
                .as_deref(),
            row["confirmedAt"].as_str()
        );
        let ds: Value = serde_json::from_str(include_str!(
            "../../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
        ))
        .unwrap();
        assert!(crate::cloud_direct::write_direct_canonical_leased(
            &root,
            "ready",
            &ds,
            "lost-worker"
        )
        .is_err());
        assert_eq!(
            get_item(&conn, "ready")
                .unwrap()
                .unwrap()
                .confirmed_at
                .as_deref(),
            row["confirmedAt"].as_str()
        );
        crate::cloud_direct::write_direct_canonical(&root, "ready", &ds).unwrap();
        assert!(get_item(&conn, "ready")
            .unwrap()
            .unwrap()
            .confirmed_at
            .is_none());
        let by: Option<String> = conn
            .query_row(
                "SELECT confirmed_by FROM library_items_v2 WHERE id='ready'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(by.is_none());
    }

    #[test]
    fn retry_job_state_blocks_confirmation_even_when_library_status_is_ready() {
        let (root, conn) = fixture();
        crate::processing::queue::enqueue(&conn, "retry", "ready", "source", &Value::Null).unwrap();
        assert_eq!(
            set_library_item_confirmed_core(&root, "ready", true).unwrap_err(),
            "LIBRARY_ITEM_NOT_CONFIRMABLE:processing"
        );
    }

    #[test]
    fn reachable_live_pointer_overrides_stale_archive() {
        let (root, conn) = fixture();
        set_library_item_confirmed_core(&root, "ready", true).unwrap();
        let nas = root.join("nas");
        let reading = crate::export_nas_library::nas_reading_exams_dir(&nas);
        std::fs::create_dir_all(&reading).unwrap();
        conn.execute("INSERT INTO papers(paper_id,batch_id,published_at,destination,pointer_json,status) VALUES('p','b','now',?1,'{}','current')",[nas.to_string_lossy().as_ref()]).unwrap();
        conn.execute("INSERT INTO paper_items(paper_id,modality,part,exam_id,library_item_id,title,script_sha256,runtime_sha256) VALUES('p','reading','P1','ready','ready','Ready','hash','hash')",[]).unwrap();
        std::fs::write(
            reading.join("current-paper.json"),
            json!({"schemaVersion":"CurrentPaperV1","reading":[{"examId":"other"}],"listening":[]})
                .to_string(),
        )
        .unwrap();
        assert!(set_library_item_confirmed_core(&root, "ready", false).is_ok());
        set_library_item_confirmed_core(&root, "ready", true).unwrap();
        std::fs::remove_file(reading.join("current-paper.json")).unwrap();
        // Reachable directory and no current pointer is an explicit withdrawal.
        assert!(set_library_item_confirmed_core(&root, "ready", false).is_ok());
    }

    #[test]
    fn batch_database_failure_rolls_back_earlier_success_and_lock_is_exclusive() {
        let (root, conn) = fixture();
        conn.execute_batch("CREATE TRIGGER confirm_failure BEFORE UPDATE OF confirmed_at ON library_items_v2 WHEN NEW.id='ready' BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        assert!(set_library_items_confirmed_core(
            &root,
            &["published".into(), "ready".into()],
            true
        )
        .unwrap_err()
        .contains("injected"));
        assert!(get_item(&conn, "published")
            .unwrap()
            .unwrap()
            .confirmed_at
            .is_none());
        let guard = PublicationGuard::acquire(&root).unwrap();
        let second = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(root.join(".library-confirmation-publish.lock"))
            .unwrap();
        assert!(fs2::FileExt::try_lock_exclusive(&second).is_err());
        drop(guard);
        assert!(fs2::FileExt::try_lock_exclusive(&second).is_ok());
    }

    #[test]
    fn rename_part_and_editor_save_preserve_confirmation() {
        let (root, conn) = fixture();
        let ds: Value = serde_json::from_str(include_str!(
            "../../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json"
        ))
        .unwrap();
        conn.execute("UPDATE library_items_v2 SET canonical_ds_json=?1,part_label='P1',part_source='canonical' WHERE id='ready'",[ds.to_string()]).unwrap();
        let at =
            set_library_item_confirmed_core(&root, "ready", true).unwrap()["confirmedAt"].clone();
        super::super::commands::rename_library_item_core(&root, "ready", "New title", 1).unwrap();
        super::super::commands::set_library_item_part_core(&root, "ready", Some("P2")).unwrap();
        let version = get_item(&conn, "ready")
            .unwrap()
            .unwrap()
            .current_edit_version;
        super::super::commands::apply_editor_commands_core(&root,super::super::repository::ApplyEditorCommandsInput {
            item_id:"ready".into(),base_version:version,request_id:None,title:None,
            commands:vec![json!({"op":"setAnswer","slotId":"q14","value":{"kind":"option","labels":["A"],"assignment":"unordered_set"}})],
        }).unwrap();
        assert_eq!(
            json!(get_item(&conn, "ready").unwrap().unwrap().confirmed_at),
            at
        );
    }
}
