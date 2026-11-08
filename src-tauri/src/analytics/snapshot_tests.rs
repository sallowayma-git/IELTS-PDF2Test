//! Snapshot fixtures are copied from real student services and real Electron UI runs.
use super::*;
use serde_json::Value;
use std::path::PathBuf;
#[test]
fn imports_real_v1_v2_and_electron_snapshots_with_archived_source() {
    for version in ["v1", "v2", "v2-electron"] {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../fixtures/golden/synthetic/backend-data-v2")
            .join(version);
        let evidence: Value =
            serde_json::from_slice(&std::fs::read(fixture.join("evidence.json")).unwrap()).unwrap();
        let root = std::env::temp_dir().join(format!("snapshot-v2-{}", uuid::Uuid::new_v4()));
        let mut conn = open_analytics(&root).unwrap();
        let report =
            import::import_submission_snapshots(&mut conn, &root, &fixture.join("submissions"))
                .unwrap();
        assert_eq!(report.ok, 1, "{report:?}");
        archive::recover(&root, fixture.join("papers").to_str().unwrap()).unwrap();
        let passages = snapshot_v2::rows(
            &conn,
            "SELECT * FROM passages WHERE attempt_id=? ORDER BY passage_index",
            &[&evidence["attemptId"].as_str()],
        )
        .unwrap();
        assert_eq!(passages.len(), 3);
        for passage in passages {
            let source = archive::content(
                &conn,
                passage["exam_id"].as_str().unwrap(),
                passage["runtime_sha256"].as_str().unwrap(),
            )
            .unwrap();
            assert!(source["passage"]["blocks"]
                .as_array()
                .is_some_and(|a| !a.is_empty()));
            assert!(passage["raw_submission_json"].as_str().is_some());
        }
        let content = snapshot_v2::rows(
            &conn,
            "SELECT content FROM writings WHERE attempt_id=?",
            &[&evidence["attemptId"].as_str()],
        )
        .unwrap();
        assert!(content
            .iter()
            .any(|w| !w["content"].as_str().unwrap_or_default().is_empty()));
        if version != "v1" {
            assert_eq!(
                conn.query_row("SELECT COUNT(*) FROM listening_answers", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                9
            );
            assert!(
                conn.query_row("SELECT COUNT(*) FROM answer_events", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap()
                    >= 12
            );
        }
        drop(conn);
        std::fs::remove_dir_all(root).unwrap();
    }
}
