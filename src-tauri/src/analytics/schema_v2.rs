use crate::CommandResult;
use rusqlite::{Connection, Transaction, TransactionBehavior};

pub const SCHEMA_VERSION: i64 = 2;

pub fn migrate(conn: &Connection) -> CommandResult<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let version: i64 = tx
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if version > SCHEMA_VERSION {
        return Err("ANALYTICS_SCHEMA_TOO_NEW: 分析库版本过新，请升级作者端".into());
    }
    tx.execute_batch(super::ANALYTICS_SCHEMA_SQL)
        .map_err(|e| e.to_string())?;
    for (table, name, declaration) in [
        ("attempts", "install_id", "TEXT"),
        ("attempts", "session_id", "TEXT"),
        ("attempts", "student_uuid", "TEXT"),
        ("attempts", "abandon_reason", "TEXT"),
        (
            "attempts",
            "counted",
            "INTEGER NOT NULL DEFAULT 1 CHECK(counted IN (0,1))",
        ),
        ("attempts", "exclude_reason", "TEXT"),
        ("attempts", "review_version", "INTEGER NOT NULL DEFAULT 0"),
        ("passages", "highlights_json", "TEXT"),
        ("passages", "raw_submission_json", "TEXT"),
        ("passages", "note_text", "TEXT"),
        ("passages", "timer_snapshot_json", "TEXT"),
        ("passages", "runtime_sha256", "TEXT"),
        ("passages", "script_sha256", "TEXT"),
        ("answers", "response_slot_id", "TEXT"),
        ("answers", "max_score", "REAL"),
        ("answers", "score", "REAL"),
        ("writings", "prompt_text", "TEXT"),
        ("writings", "content", "TEXT"),
        ("writings", "metadata_json", "TEXT"),
    ] {
        super::ensure_column(&tx, table, name, declaration)?;
    }
    tx.execute_batch(include_str!("schema_v2.sql"))
        .map_err(|e| e.to_string())?;
    super::ensure_column(&tx, "listening_answers", "part_id", "TEXT")?;
    if version < SCHEMA_VERSION {
        tx.execute_batch(
            "UPDATE answers SET response_slot_id=question_id,max_score=weight,
            score=CASE is_correct WHEN 1 THEN weight WHEN 0 THEN 0 ELSE NULL END;
            PRAGMA user_version=2;",
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upgrades_legacy_data_and_reopens_without_overwriting_review() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::analytics::ANALYTICS_SCHEMA_SQL)
            .unwrap();
        conn.execute_batch("INSERT INTO answers(attempt_id,passage_index,question_id,is_correct,weight) VALUES('a',0,'q1',1,2),('a',0,'q2',NULL,1);").unwrap();
        migrate(&conn).unwrap();
        let score: (String, f64, Option<f64>) = conn
            .query_row(
                "SELECT response_slot_id,max_score,score FROM answers WHERE question_id='q1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(score, ("q1".into(), 2.0, Some(2.0)));
        let unknown: Option<f64> = conn
            .query_row(
                "SELECT score FROM answers WHERE question_id='q2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(unknown, None);
        conn.execute("UPDATE answers SET score=0.5 WHERE question_id='q1'", [])
            .unwrap();
        migrate(&conn).unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT score FROM answers WHERE question_id='q1'",
                [],
                |r| r.get::<_, f64>(0)
            )
            .unwrap(),
            0.5
        );
    }
    #[test]
    fn migration_failure_rolls_back_columns_and_version() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::analytics::ANALYTICS_SCHEMA_SQL)
            .unwrap();
        conn.execute_batch("CREATE TABLE writing_grades(x TEXT)")
            .unwrap();
        assert!(migrate(&conn).is_err());
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(conn.prepare("SELECT score FROM answers").is_err());
    }
    #[test]
    fn rejects_future_schema_without_downgrade() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA user_version=3").unwrap();
        assert!(migrate(&conn).unwrap_err().contains("TOO_NEW"));
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            3
        );
    }
}
