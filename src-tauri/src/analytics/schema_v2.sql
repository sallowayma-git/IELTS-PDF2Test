CREATE UNIQUE INDEX IF NOT EXISTS ux_analytics_slot ON answers(attempt_id,passage_index,response_slot_id);
CREATE TABLE IF NOT EXISTS answer_events (
 attempt_id TEXT NOT NULL, event_seq INTEGER NOT NULL, client_event_id TEXT NOT NULL,
 event_type TEXT NOT NULL, passage_index INTEGER, question_id TEXT, old_answer_json TEXT, new_answer_json TEXT,
 wall_time TEXT NOT NULL, monotonic_ms REAL NOT NULL, clock_id TEXT NOT NULL,
 PRIMARY KEY(attempt_id,event_seq), UNIQUE(attempt_id,client_event_id)
);
CREATE TABLE IF NOT EXISTS listening_attempts (
 attempt_id TEXT PRIMARY KEY, exam_id TEXT NOT NULL, source_revision INTEGER NOT NULL,
 runtime_sha256 TEXT, script_sha256 TEXT, started_at TEXT, submitted_at TEXT NOT NULL,
 score_correct REAL NOT NULL, score_total REAL NOT NULL, playback_snapshot_json TEXT NOT NULL, payload_hash TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS listening_answers (
 attempt_id TEXT NOT NULL, slot_id TEXT NOT NULL, question_kind TEXT, part_id TEXT, user_answer_json TEXT NOT NULL,
 correct_answer_json TEXT NOT NULL, is_correct INTEGER NOT NULL, weight REAL NOT NULL,
 PRIMARY KEY(attempt_id,slot_id)
);
CREATE TABLE IF NOT EXISTS score_overrides (
 override_id TEXT PRIMARY KEY, attempt_id TEXT NOT NULL REFERENCES attempts(attempt_id),
 modality TEXT NOT NULL CHECK(modality IN ('reading','listening')), passage_index INTEGER NOT NULL,
 response_slot_id TEXT NOT NULL, original_is_correct INTEGER, original_score REAL, original_answer_json TEXT,
 new_is_correct INTEGER NOT NULL CHECK(new_is_correct IN (0,1)), new_score REAL NOT NULL CHECK(new_score>=0),
 reason TEXT NOT NULL, operator TEXT NOT NULL, created_at TEXT NOT NULL, revoked_at TEXT, run_id TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS ux_active_override
 ON score_overrides(attempt_id,modality,passage_index,response_slot_id) WHERE revoked_at IS NULL;
CREATE TABLE IF NOT EXISTS rejudge_runs (
 run_id TEXT PRIMARY KEY, scope TEXT NOT NULL, reason TEXT NOT NULL, old_scorer_version TEXT,
 new_rule_json TEXT NOT NULL, affected_count INTEGER NOT NULL, operator TEXT NOT NULL, created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS writing_grades (
 grade_id TEXT PRIMARY KEY, attempt_id TEXT NOT NULL REFERENCES attempts(attempt_id), writing_submission_id TEXT NOT NULL,
 task_type TEXT NOT NULL, grader TEXT NOT NULL, ta_tr REAL NOT NULL, cc REAL NOT NULL, lr REAL NOT NULL, gra REAL NOT NULL,
 task_band REAL NOT NULL, overall_comment TEXT NOT NULL, status TEXT NOT NULL CHECK(status IN ('draft','reviewed','published','superseded')),
 version INTEGER NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, reviewed_by TEXT, published_at TEXT,
 CHECK(ta_tr BETWEEN 0 AND 9 AND cc BETWEEN 0 AND 9 AND lr BETWEEN 0 AND 9 AND gra BETWEEN 0 AND 9)
);
CREATE UNIQUE INDEX IF NOT EXISTS ux_published_writing ON writing_grades(attempt_id,writing_submission_id) WHERE status='published';
CREATE TABLE IF NOT EXISTS writing_annotations (
 annotation_id TEXT PRIMARY KEY, grade_id TEXT NOT NULL REFERENCES writing_grades(grade_id),
 start_offset INTEGER NOT NULL CHECK(start_offset>=0), end_offset INTEGER NOT NULL CHECK(end_offset>=start_offset),
 quoted_text TEXT NOT NULL, category TEXT NOT NULL, comment TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS audit_log (
 id TEXT PRIMARY KEY, at TEXT NOT NULL, operator TEXT NOT NULL, action TEXT NOT NULL,
 target_type TEXT NOT NULL, target_id TEXT NOT NULL, before_json TEXT, after_json TEXT, reason TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_audit_target ON audit_log(target_type,target_id,at);
CREATE TABLE IF NOT EXISTS review_settings (
 id INTEGER PRIMARY KEY CHECK(id=1), operator_name TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS candidate_aliases (
 candidate_id TEXT NOT NULL, candidate_name TEXT NOT NULL, student_uuid TEXT, merged_into TEXT,
 operator TEXT NOT NULL, at TEXT NOT NULL, PRIMARY KEY(candidate_id,candidate_name)
);
