use super::*;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn setup(version: u8) -> (PathBuf, Connection, String) {
    setup_fixture(&format!("student-v{version}"))
}

fn setup_fixture(name: &str) -> (PathBuf, Connection, String) {
    let root = std::env::temp_dir().join(format!("analytics-real-{name}-{}", uuid::Uuid::new_v4()));
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../fixtures/analytics/{name}"));
    let provenance: Value =
        serde_json::from_slice(&fs::read(fixture.join("provenance.json")).unwrap()).unwrap();
    let target = root.join("nas/snapshots/fixture-terminal");
    fs::create_dir_all(&target).unwrap();
    for entry in fs::read_dir(&fixture).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() != "provenance.json" {
            fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
        }
    }
    let mut conn = open_analytics(&root).unwrap();
    let report = import::import_submission_snapshots(&mut conn, &root, &root.join("nas")).unwrap();
    assert_eq!(report.failed, 0, "{report:?}");
    assert_eq!(report.attempts_imported, 1, "{report:?}");
    review::save_settings(&root, &json!({"operatorName":"复核教师"})).unwrap();
    (
        root,
        conn,
        provenance["attemptId"].as_str().unwrap().to_string(),
    )
}

#[test]
fn imports_listening_facts_from_the_real_electron_final_submission() {
    let (root, conn, attempt) = setup_fixture("student-electron-listening-v2");
    let (correct, total): (f64, f64) = conn
        .query_row(
            "SELECT score_correct,score_total FROM listening_attempts WHERE attempt_id=?1",
            [&attempt],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((correct, total), (9.0, 9.0));
    let (slots, correct_slots): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*),SUM(is_correct) FROM listening_answers WHERE attempt_id=?1",
            [&attempt],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((slots, correct_slots), (9, 9));
    assert_eq!(
        queries::build_student_view(&conn, &attempt)
            .unwrap()
            .writings
            .len(),
        2
    );
    drop(conn);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn imports_snapshot_created_by_the_real_electron_reading_and_writing_flow() {
    let (root, mut conn, attempt) = setup_fixture("student-electron-v2");
    let view = queries::build_student_view(&conn, &attempt).unwrap();
    assert_eq!(view.raw_score, 3.0);
    assert_eq!(view.raw_total, 3.0);
    assert_eq!(view.writings.len(), 2);
    let (events, clocks): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*),COUNT(DISTINCT clock_id) FROM answer_events WHERE attempt_id=?1",
            [&attempt],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((events, clocks), (19, 1));
    let (raw, note, highlights): (String, String, String) = conn.query_row(
        "SELECT raw_submission_json,note_text,highlights_json FROM passages WHERE attempt_id=?1 ORDER BY passage_index LIMIT 1",
        [&attempt], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).unwrap();
    assert!(raw.contains("alpha"));
    assert!(note.contains('>'));
    assert!(
        serde_json::from_str::<Value>(&highlights)
            .unwrap()
            .as_array()
            .unwrap()
            .len()
            >= 2
    );
    let duplicate =
        import::import_submission_snapshots(&mut conn, &root, &root.join("nas")).unwrap();
    assert_eq!(duplicate.failed, 0);
    assert_eq!(duplicate.attempts_imported, 0);
    assert_eq!(duplicate.skipped, 1);
    drop(conn);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn consumes_snapshots_generated_by_real_v1_and_v2_student_services() {
    for version in [1, 2] {
        let (root, conn, attempt) = setup(version);
        let writing: String = conn
            .query_row(
                "SELECT content FROM writings WHERE attempt_id=?1",
                [&attempt],
                |r| r.get(0),
            )
            .unwrap();
        assert!(writing.contains("中文🙂 retained"));
        let note: String = conn
            .query_row(
                "SELECT note_text FROM passages WHERE attempt_id=?1 LIMIT 1",
                [&attempt],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(note, "保留笔记");
        let slots: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM answers WHERE attempt_id=?1 AND response_slot_id IS NOT NULL",
                [&attempt],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(slots, 3);
        let events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM answer_events WHERE attempt_id=?1",
                [&attempt],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(events, if version == 2 { 3 } else { 0 });
        let detail = detail::get(&conn, &root, &attempt).unwrap();
        assert_eq!(detail["n"], 1);
        assert_eq!(detail["writings"].as_array().unwrap().len(), 1);
        if version == 2 {
            assert!(!detail["listeningAnswers"].as_array().unwrap().is_empty());
            let stats =
                statistics::listening(&conn, "fixture-batch", "static-test-paper", false).unwrap();
            assert_eq!(stats["n"], 1);
            assert_eq!(stats["parts"][0]["partId"], "part-1");
            assert_eq!(stats["kinds"][0]["kind"], "form_completion");
            assert_eq!(stats["distribution"][9]["n"], 1);
        }
        drop(conn);
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn later_exports_reject_changes_to_original_body_listening_and_events() {
    for (table, mutation) in [
        (
            "writings",
            "UPDATE writing_submissions SET content='tampered essay'",
        ),
        (
            "listening_answers",
            "UPDATE listening_answers SET user_answer_json='\"tampered\"'",
        ),
        (
            "answer_events",
            "UPDATE answer_events SET new_answer_json='\"tampered\"'",
        ),
        (
            "passages",
            "UPDATE reading_passages SET raw_submission_json='{\"answers\":{\"q1\":\"tampered\"}}'",
        ),
    ] {
        let (root, mut conn, _) = setup(2);
        let before = records::query(&conn, &format!("SELECT * FROM {table}"), &[]).unwrap();
        let dir = root.join("nas/snapshots/fixture-terminal");
        let file = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "sqlite"))
            .unwrap();
        let source = Connection::open(&file).unwrap();
        source.execute_batch(mutation).unwrap();
        drop(source);
        let meta_file = file.with_extension("meta.json");
        let mut meta: Value = serde_json::from_slice(&fs::read(&meta_file).unwrap()).unwrap();
        let bytes = fs::read(&file).unwrap();
        meta["sha256"] = json!(import::sha256_hex(&bytes));
        meta["sizeBytes"] = json!(bytes.len());
        let previous =
            chrono::DateTime::parse_from_rfc3339(meta["exportedAt"].as_str().unwrap()).unwrap();
        meta["exportedAt"] = json!((previous + chrono::Duration::seconds(1)).to_rfc3339());
        fs::write(meta_file, meta.to_string()).unwrap();
        let report =
            import::import_submission_snapshots(&mut conn, &root, &root.join("nas")).unwrap();
        assert_eq!(report.failed, 1, "{table}: {report:?}");
        assert!(report.outcomes[0]
            .error
            .as_deref()
            .unwrap()
            .contains("SNAPSHOT_FACT_CONFLICT"));
        assert_eq!(
            before,
            records::query(&conn, &format!("SELECT * FROM {table}"), &[]).unwrap()
        );
        drop(conn);
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn concurrent_imports_keep_one_set_of_original_facts_and_preserve_review_state() {
    let (root, conn, attempt) = setup(2);
    let expected: i64 = conn
        .query_row("SELECT COUNT(*) FROM answers", [], |r| r.get(0))
        .unwrap();
    drop(conn);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let root = root.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut conn = open_analytics(&root).unwrap();
                barrier.wait();
                import::import_submission_snapshots(&mut conn, &root, &root.join("nas")).unwrap()
            })
        })
        .collect();
    for worker in workers {
        let report = worker.join().unwrap();
        assert_eq!(report.failed, 0, "{report:?}");
    }
    let conn = open_analytics(&root).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM answers WHERE attempt_id=?1",
            [&attempt],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        expected
    );
    assert_eq!(review::settings(&root).unwrap()["operatorName"], "复核教师");
    drop(conn);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn rejudge_preview_has_no_side_effects_and_apply_revalidates_its_token() {
    let (root, mut conn, attempt) = setup(2);
    let rule = json!({"scope":"paper","paperId":"static-test-paper","acceptedAnswers":[" b "],"reason":"新增可接受拼写"});
    let preview = rejudge::preview(&conn, &rule).unwrap();
    assert_eq!(preview["n"], 1);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM score_overrides", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let input = json!({"rule":rule,"previewToken":preview["previewToken"]});
    let result = rejudge::apply(&mut conn, &root, &input).unwrap();
    assert_eq!(result["n"], 1);
    assert_eq!(
        queries::build_student_view(&conn, &attempt)
            .unwrap()
            .effective_score,
        3.0
    );
    assert!(rejudge::apply(&mut conn, &root, &input)
        .unwrap_err()
        .contains("REJUDGE_CONFLICT"));
    let run: String = conn
        .query_row("SELECT run_id FROM rejudge_runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        review::list_audit(&conn, &json!({"targetId":run})).unwrap()["n"],
        1
    );
    let invalid = json!({"scope":"slot","examId":"fixture-reading-1","acceptedAnswers":["B"],"reason":"缺少锁定版本"});
    assert!(rejudge::preview(&conn, &invalid).is_err());
    drop(conn);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn overrides_are_cas_scoped_audited_and_revoke_restores_statistics() {
    let (root, mut conn, attempt) = setup(2);
    let original = queries::build_student_view(&conn, &attempt).unwrap();
    assert_eq!(original.raw_score, 2.0);
    let input = json!({"attemptId":attempt,"passageIndex":1,"responseSlotId":"q1","newScore":1,"newIsCorrect":true,"reason":"接受教师核对结果","expectedVersion":0});
    let applied = review::mutate(&mut conn, &root, "create_score_override", &input).unwrap();
    let view = queries::build_student_view(&conn, &attempt).unwrap();
    assert_eq!(view.raw_score, 2.0);
    assert_eq!(view.effective_score, 3.0);
    assert!(
        review::mutate(&mut conn, &root, "create_score_override", &input)
            .unwrap_err()
            .contains("VERSION_CONFLICT")
    );
    let raw: Option<i64> = conn
        .query_row(
            "SELECT is_correct FROM answers WHERE attempt_id=?1 AND passage_index=1",
            [&attempt],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(raw, Some(0));
    review::mutate(
        &mut conn,
        &root,
        "revoke_score_override",
        &json!({"overrideId":applied["overrideId"],"expectedVersion":1,"reason":"撤销核对"}),
    )
    .unwrap();
    assert_eq!(
        queries::build_student_view(&conn, &attempt)
            .unwrap()
            .effective_score,
        2.0
    );
    assert_eq!(
        review::list_audit(&conn, &json!({"targetId":attempt})).unwrap()["n"],
        2
    );
    review::mutate(
        &mut conn,
        &root,
        "set_attempt_counted",
        &json!({"attemptId":attempt,"counted":false,"reason":"测试排除","expectedVersion":2}),
    )
    .unwrap();
    assert!(!queries::build_student_view(&conn, &attempt).unwrap().valid);
    // A fresher real snapshot must not reset teacher-owned state or remove the history.
    let dir = root.join("nas/snapshots/fixture-terminal");
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/analytics/student-v2-reexport");
    for entry in fs::read_dir(fixture).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() != "provenance.json" {
            fs::copy(entry.path(), dir.join(entry.file_name())).unwrap();
        }
    }
    let reimport =
        import::import_submission_snapshots(&mut conn, &root, &root.join("nas")).unwrap();
    assert_eq!(reimport.attempts_imported, 1, "{reimport:?}");
    assert!(!conn
        .query_row(
            "SELECT counted FROM attempts WHERE attempt_id=?1",
            [&attempt],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM score_overrides WHERE attempt_id=?1",
            [&attempt],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    drop(conn);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn writing_cas_unicode_annotation_review_publish_and_scope_are_enforced() {
    let (root, mut conn, attempt) = setup(2);
    let wid: String = conn
        .query_row(
            "SELECT writing_submission_id FROM writings WHERE attempt_id=?1",
            [&attempt],
            |r| r.get(0),
        )
        .unwrap();
    let base = json!({"attemptId":attempt,"writingSubmissionId":wid,"taTr":6.5,"cc":7,"lr":6,"gra":7,"overallComment":"有明确论证","expectedVersion":0});
    let grade = writing_review::mutate(&mut conn, &root, "save_writing_grade", &base).unwrap();
    let gid = grade["grade_id"].as_str().unwrap();
    assert_eq!(grade["task_band"], 6.625);
    let content: String = conn
        .query_row(
            "SELECT content FROM writings WHERE writing_submission_id=?1",
            [&wid],
            |r| r.get(0),
        )
        .unwrap();
    let start = content.find("中文").unwrap();
    let offset = content[..start].encode_utf16().count();
    let annotation = json!({"gradeId":gid,"expectedVersion":1,"startOffset":offset,"endOffset":offset+4,"quotedText":"中文🙂","category":"词汇","comment":"保留原文"});
    let next = writing_review::mutate(&mut conn, &root, "save_annotation", &annotation).unwrap();
    assert_eq!(next["version"], 2);
    assert!(
        writing_review::mutate(&mut conn, &root, "save_annotation", &annotation)
            .unwrap_err()
            .contains("VERSION_CONFLICT")
    );
    writing_review::mutate(
        &mut conn,
        &root,
        "review_writing_grade",
        &json!({"gradeId":gid,"expectedVersion":2}),
    )
    .unwrap();
    let published = writing_review::mutate(
        &mut conn,
        &root,
        "publish_writing_grade",
        &json!({"gradeId":gid,"expectedVersion":3}),
    )
    .unwrap();
    assert_eq!(published["status"], "published");
    let mut wrong = base.clone();
    wrong["attemptId"] = json!("other");
    assert!(writing_review::mutate(&mut conn, &root, "save_writing_grade", &wrong).is_err());
    let mut invalid = base.clone();
    invalid["taTr"] = json!(6.25);
    assert!(
        writing_review::mutate(&mut conn, &root, "save_writing_grade", &invalid)
            .unwrap_err()
            .contains("INVALID_WRITING_SCORE")
    );
    let grade2 = writing_review::mutate(&mut conn, &root, "save_writing_grade", &base).unwrap();
    let gid2 = grade2["grade_id"].as_str().unwrap();
    writing_review::mutate(
        &mut conn,
        &root,
        "review_writing_grade",
        &json!({"gradeId":gid2,"expectedVersion":1}),
    )
    .unwrap();
    writing_review::mutate(
        &mut conn,
        &root,
        "publish_writing_grade",
        &json!({"gradeId":gid2,"expectedVersion":2}),
    )
    .unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM writing_grades WHERE status='published'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT status FROM writing_grades WHERE grade_id=?1",
            [gid],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "superseded"
    );
    let export = export::write_export(
        &conn,
        "fixture-batch",
        "static-test-paper",
        &root.join("grades.xlsx"),
    )
    .unwrap();
    assert_eq!(export.sheets.len(), 8);
    drop(conn);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn rejected_teacher_writes_and_failed_audits_leave_no_partial_changes() {
    let (root, mut conn, attempt) = setup(2);
    let base = json!({"attemptId":attempt,"passageIndex":1,"responseSlotId":"q1","newScore":1,"newIsCorrect":true,"reason":"核对","expectedVersion":0});
    let mut invalid = base.clone();
    invalid["responseSlotId"] = json!("absent");
    assert!(
        review::mutate(&mut conn, &root, "create_score_override", &invalid)
            .unwrap_err()
            .contains("SLOT_NOT_FOUND")
    );
    invalid = base.clone();
    invalid["newScore"] = json!(2);
    assert!(
        review::mutate(&mut conn, &root, "create_score_override", &invalid)
            .unwrap_err()
            .contains("INVALID_SCORE")
    );
    conn.execute_batch("CREATE TRIGGER reject_audit BEFORE INSERT ON audit_log BEGIN SELECT RAISE(ABORT,'simulated audit failure'); END;").unwrap();
    assert!(review::mutate(&mut conn, &root, "create_score_override", &base).is_err());
    assert_eq!(
        conn.query_row(
            "SELECT review_version FROM attempts WHERE attempt_id=?1",
            [&attempt],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM score_overrides", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    conn.execute_batch("DROP TRIGGER reject_audit").unwrap();
    for (version, counted) in [(0, false), (1, true)] {
        review::mutate(&mut conn,&root,"set_attempt_counted",&json!({"attemptId":attempt,"counted":counted,"reason":"核对统计","expectedVersion":version})).unwrap();
        assert_eq!(
            queries::build_student_view(&conn, &attempt).unwrap().valid,
            counted
        );
    }
    let slot: String = conn
        .query_row("SELECT slot_id FROM listening_answers LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    let original = statistics::listening(&conn, "fixture-batch", "static-test-paper", true)
        .unwrap()["score"]["mean"]
        .as_f64()
        .unwrap();
    let applied=review::mutate(&mut conn,&root,"create_score_override",&json!({"attemptId":attempt,"modality":"listening","responseSlotId":slot,"newScore":0.5,"newIsCorrect":false,"reason":"听力部分分","expectedVersion":2})).unwrap();
    let effective =
        statistics::listening(&conn, "fixture-batch", "static-test-paper", false).unwrap();
    assert_eq!(effective["score"]["mean"].as_f64(), Some(original - 0.5));
    assert_eq!(
        statistics::listening(&conn, "fixture-batch", "static-test-paper", true).unwrap()["score"]
            ["mean"]
            .as_f64(),
        Some(original)
    );
    review::mutate(
        &mut conn,
        &root,
        "revoke_score_override",
        &json!({"overrideId":applied["overrideId"],"reason":"恢复","expectedVersion":3}),
    )
    .unwrap();
    assert_eq!(
        statistics::listening(&conn, "fixture-batch", "static-test-paper", false).unwrap()["score"]
            ["mean"]
            .as_f64(),
        Some(original)
    );
    assert!(review::mutate(
        &mut conn,
        &root,
        "revoke_score_override",
        &json!({"overrideId":applied["overrideId"],"reason":"重复撤销","expectedVersion":4})
    )
    .unwrap_err()
    .contains("OVERRIDE_NOT_FOUND"));
    drop(conn);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn annotations_are_scoped_and_deleted_with_cas_and_audit() {
    let (root, mut conn, attempt) = setup(2);
    let (wid, content): (String, String) = conn
        .query_row(
            "SELECT writing_submission_id,content FROM writings LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let input = json!({"attemptId":attempt,"writingSubmissionId":wid,"taTr":6,"cc":6,"lr":6,"gra":6,"expectedVersion":0});
    let first = writing_review::mutate(&mut conn, &root, "save_writing_grade", &input).unwrap();
    let second = writing_review::mutate(&mut conn, &root, "save_writing_grade", &input).unwrap();
    let quote = &content[..1];
    let annotation = json!({"annotationId":"annotation-scope","gradeId":first["grade_id"],"startOffset":0,"endOffset":1,"quotedText":quote,"category":"其他","comment":"核对","expectedVersion":1});
    writing_review::mutate(&mut conn, &root, "save_annotation", &annotation).unwrap();
    let wrong =
        json!({"annotationId":"annotation-scope","gradeId":second["grade_id"],"expectedVersion":1});
    assert!(
        writing_review::mutate(&mut conn, &root, "delete_annotation", &wrong)
            .unwrap_err()
            .contains("ANNOTATION_NOT_FOUND")
    );
    let deletion =
        json!({"annotationId":"annotation-scope","gradeId":first["grade_id"],"expectedVersion":2});
    let result = writing_review::mutate(&mut conn, &root, "delete_annotation", &deletion).unwrap();
    assert_eq!(result["version"], 3);
    assert!(
        writing_review::mutate(&mut conn, &root, "delete_annotation", &deletion)
            .unwrap_err()
            .contains("VERSION_CONFLICT")
    );
    assert_eq!(
        review::list_audit(
            &conn,
            &json!({"targetId":"annotation-scope","action":"delete_annotation"})
        )
        .unwrap()["n"],
        1
    );
    assert_eq!(
        writing_review::queue(&conn, Some("static-test-paper"), Some("draft")).unwrap()["n"],
        1
    );
    assert_eq!(
        writing_review::queue(&conn, Some("other-paper"), None).unwrap()["n"],
        0
    );
    drop(conn);
    let _ = fs::remove_dir_all(root);
}
