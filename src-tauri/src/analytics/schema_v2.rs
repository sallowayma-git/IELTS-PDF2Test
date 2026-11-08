//! Additive local-only migrations. Student snapshots are never migrated by the author.
use rusqlite::Connection;
use crate::CommandResult;

pub fn migrate(conn: &Connection) -> CommandResult<()> {
    conn.execute_batch("SAVEPOINT analytics_v2").map_err(|e| e.to_string())?;
    let result = (|| {
        for (table, column, decl) in [
            ("attempts", "session_id", "TEXT"), ("attempts", "student_uuid", "TEXT"),
            ("attempts", "abandon_reason", "TEXT"), ("attempts", "counted", "INTEGER NOT NULL DEFAULT 0"),
            ("attempts", "counted_override", "INTEGER NOT NULL DEFAULT 0"),
            ("attempts", "exclude_reason", "TEXT"), ("attempts", "version", "INTEGER NOT NULL DEFAULT 1"),
            ("passages", "raw_submission_json", "TEXT"), ("passages", "highlights_json", "TEXT"),
            ("passages", "note_text", "TEXT"), ("passages", "timer_snapshot_json", "TEXT"),
            ("passages", "runtime_sha256", "TEXT"), ("passages", "script_sha256", "TEXT"),
            ("answers", "response_slot_id", "TEXT"), ("answers", "max_score", "REAL"), ("answers", "score", "REAL"),
            ("writings", "prompt_text", "TEXT"), ("writings", "content", "TEXT"), ("writings", "metadata_json", "TEXT"),
        ] { super::ensure_column(conn, table, column, decl)?; }
        conn.execute_batch(SQL).map_err(|e| format!("ANALYTICS_MIGRATION:分析库升级失败：{e}"))?;
        refresh_counted(conn)?;
        Ok(())
    })();
    match result {
        Ok(()) => conn.execute_batch("RELEASE analytics_v2").map_err(|e| e.to_string()),
        Err(e) => { let _ = conn.execute_batch("ROLLBACK TO analytics_v2; RELEASE analytics_v2"); Err(e) }
    }
}

pub fn refresh_counted(conn: &Connection) -> CommandResult<()> {
    conn.execute_batch("UPDATE attempts SET counted = CASE WHEN status='submitted'
      AND EXISTS(SELECT 1 FROM suites s WHERE s.attempt_id=attempts.attempt_id)
      AND (SELECT COUNT(*) FROM passages p WHERE p.attempt_id=attempts.attempt_id) >=
          (SELECT MAX(MAX(total_passages,1)) FROM suites s WHERE s.attempt_id=attempts.attempt_id)
      AND NOT EXISTS(SELECT 1 FROM passages p WHERE p.attempt_id=attempts.attempt_id AND (gradable=0 OR total<=0))
      THEN 1 ELSE 0 END WHERE counted_override=0;")
      .map_err(|e| e.to_string())
}

const SQL: &str = r#"
UPDATE answers SET response_slot_id=question_id WHERE response_slot_id IS NULL;
UPDATE answers SET max_score=weight WHERE max_score IS NULL;
UPDATE answers SET score=is_correct*weight WHERE score IS NULL AND is_correct IS NOT NULL;
CREATE TABLE IF NOT EXISTS papers (
 paper_id TEXT PRIMARY KEY, batch_id TEXT NOT NULL, published_at TEXT NOT NULL,
 superseded_at TEXT, destination TEXT NOT NULL, pointer_json TEXT NOT NULL,
 status TEXT NOT NULL CHECK(status IN ('current','superseded'))
);
CREATE TABLE IF NOT EXISTS paper_items (
 paper_id TEXT NOT NULL REFERENCES papers(paper_id), modality TEXT NOT NULL,
 part TEXT NOT NULL, exam_id TEXT NOT NULL, library_item_id TEXT, edit_version INTEGER, title TEXT,
 script_sha256 TEXT NOT NULL, runtime_sha256 TEXT NOT NULL, asset_manifest_sha256 TEXT,
 PRIMARY KEY(paper_id,modality,part)
);
CREATE TABLE IF NOT EXISTS item_content_snapshots (
 exam_id TEXT NOT NULL, runtime_sha256 TEXT NOT NULL, runtime_source_json TEXT NOT NULL,
 PRIMARY KEY(exam_id,runtime_sha256)
);
CREATE TABLE IF NOT EXISTS answer_events (
 attempt_id TEXT NOT NULL, event_seq INTEGER NOT NULL, event_type TEXT NOT NULL,
 passage_index INTEGER, question_id TEXT, old_answer_json TEXT, new_answer_json TEXT,
 wall_time TEXT NOT NULL, monotonic_ms REAL NOT NULL, event_id TEXT,
 PRIMARY KEY(attempt_id,event_seq)
);
CREATE TABLE IF NOT EXISTS listening_attempts (
 attempt_id TEXT PRIMARY KEY, exam_id TEXT NOT NULL, source_revision INTEGER NOT NULL,
 started_at TEXT, submitted_at TEXT NOT NULL, score_correct REAL NOT NULL, score_total REAL NOT NULL,
 playback_snapshot_json TEXT NOT NULL, runtime_sha256 TEXT, script_sha256 TEXT, payload_hash TEXT
);
CREATE TABLE IF NOT EXISTS listening_answers (
 attempt_id TEXT NOT NULL REFERENCES listening_attempts(attempt_id), slot_id TEXT NOT NULL,
 question_kind TEXT, user_answer_json TEXT NOT NULL, correct_answer_json TEXT NOT NULL,
 is_correct INTEGER, weight REAL NOT NULL, max_score REAL NOT NULL, score REAL,
 PRIMARY KEY(attempt_id,slot_id)
);
CREATE TABLE IF NOT EXISTS writing_grades (
 grade_id TEXT PRIMARY KEY, attempt_id TEXT NOT NULL, writing_submission_id TEXT NOT NULL,
 task_type TEXT NOT NULL, grader TEXT NOT NULL,
 ta_tr REAL NOT NULL CHECK(ta_tr BETWEEN 0 AND 9 AND ta_tr*2=CAST(ta_tr*2 AS INTEGER)),
 cc REAL NOT NULL CHECK(cc BETWEEN 0 AND 9 AND cc*2=CAST(cc*2 AS INTEGER)),
 lr REAL NOT NULL CHECK(lr BETWEEN 0 AND 9 AND lr*2=CAST(lr*2 AS INTEGER)),
 gra REAL NOT NULL CHECK(gra BETWEEN 0 AND 9 AND gra*2=CAST(gra*2 AS INTEGER)),
 task_band REAL NOT NULL, overall_comment TEXT NOT NULL,
 status TEXT NOT NULL CHECK(status IN ('draft','reviewed','published')),
 version INTEGER NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
 reviewed_by TEXT, published_at TEXT,
 FOREIGN KEY(attempt_id,writing_submission_id) REFERENCES writings(attempt_id,writing_submission_id)
);
CREATE UNIQUE INDEX IF NOT EXISTS ux_writing_published ON writing_grades(attempt_id,writing_submission_id) WHERE status='published';
CREATE TABLE IF NOT EXISTS writing_annotations (
 annotation_id TEXT PRIMARY KEY, grade_id TEXT NOT NULL REFERENCES writing_grades(grade_id),
 start_offset INTEGER NOT NULL, end_offset INTEGER NOT NULL, quoted_text TEXT NOT NULL,
 category TEXT NOT NULL CHECK(category IN ('语法','词汇','衔接','任务回应','其他')), comment TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS score_overrides (
 override_id TEXT PRIMARY KEY, attempt_id TEXT NOT NULL REFERENCES attempts(attempt_id),
 modality TEXT NOT NULL CHECK(modality IN ('reading','listening')), passage_index INTEGER NOT NULL,
 response_slot_id TEXT NOT NULL, original_is_correct INTEGER NOT NULL, original_score REAL NOT NULL,
 new_is_correct INTEGER NOT NULL CHECK(new_is_correct IN (0,1)), new_score REAL NOT NULL,
 reason TEXT NOT NULL, operator TEXT NOT NULL, created_at TEXT NOT NULL, revoked_at TEXT,
 version INTEGER NOT NULL DEFAULT 1, run_id TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS ux_active_override ON score_overrides(attempt_id,modality,passage_index,response_slot_id) WHERE revoked_at IS NULL;
CREATE TABLE IF NOT EXISTS rejudge_runs (
 run_id TEXT PRIMARY KEY, scope TEXT NOT NULL CHECK(scope IN ('paper','exam','slot')), reason TEXT NOT NULL,
 old_scorer_version TEXT, new_rule_json TEXT NOT NULL, affected_count INTEGER NOT NULL,
 operator TEXT NOT NULL, created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS audit_log (
 id TEXT PRIMARY KEY, at TEXT NOT NULL, operator TEXT NOT NULL, action TEXT NOT NULL,
 target_type TEXT NOT NULL, target_id TEXT NOT NULL, before_json TEXT, after_json TEXT, reason TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_audit_target ON audit_log(target_type,target_id,at);
CREATE TABLE IF NOT EXISTS candidate_aliases (
 candidate_id TEXT NOT NULL, candidate_name TEXT NOT NULL, student_uuid TEXT, merged_into TEXT,
 operator TEXT NOT NULL, at TEXT NOT NULL, PRIMARY KEY(candidate_id,candidate_name)
);
PRAGMA user_version=2;
"#;
