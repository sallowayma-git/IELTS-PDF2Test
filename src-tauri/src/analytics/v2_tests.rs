use super::*;
use serde_json::{json, Value};
use std::path::PathBuf;
fn fixture(version: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/golden/synthetic/backend-data-v2")
        .join(version)
}
fn load(version: &str) -> (PathBuf, rusqlite::Connection, Value) {
    let path = fixture(version);
    let ev: Value =
        serde_json::from_slice(&std::fs::read(path.join("evidence.json")).unwrap()).unwrap();
    let app = std::env::temp_dir().join(format!("analytics-v2-test-{}", uuid::Uuid::new_v4()));
    let mut c = open_analytics(&app).unwrap();
    let report =
        import::import_submission_snapshots(&mut c, &app, &path.join("submissions")).unwrap();
    assert_eq!(report.ok, 1, "{report:?}");
    archive::recover(&app, path.join("papers").to_str().unwrap()).unwrap();
    (app, c, ev)
}
#[test]
fn actual_student_v1_and_v2_snapshots_import_with_frozen_content() {
    for version in ["v1", "v2"] {
        let (app, c, ev) = load(version);
        let id = ev["attemptId"].as_str().unwrap();
        let d = detail::attempt_detail(&c, id).unwrap();
        assert_eq!(d["passages"].as_array().unwrap().len(), 3);
        assert_eq!(
            d["writings"][0]["content"],
            "This is the complete student essay. 全文😀"
        );
        assert_eq!(d["passages"][0]["note_text"], "Student note");
        assert!(d["anomalies"].as_array().unwrap().is_empty(), "{d}");
        assert!(d["passages"][0]["passage"]["blocks"][0]["html"]
            .as_str()
            .unwrap()
            .contains("Original passage"));
        assert!(!d["passages"][0]["answers"][0]["context"]["taskGroups"]
            .as_array()
            .unwrap()
            .is_empty());
        if version == "v2" {
            assert_eq!(d["events"].as_array().unwrap().len(), 12); // retried autosave did not duplicate telemetry
            assert_eq!(d["listening"][0]["answers"].as_array().unwrap().len(), 9);
            assert_eq!(d["attempt"]["session_id"], "session-fixture");
            assert_eq!(
                d["passages"][0]["activityIntervals"][0]["durationMs"],
                3000.0
            );
        } else {
            assert!(d["events"].as_array().unwrap().is_empty());
        }
        drop(c);
        std::fs::remove_dir_all(app).unwrap();
    }
}
#[test]
fn override_statistics_revoke_cas_and_raw_facts() {
    let (app, mut c, ev) = load("v2");
    let id = ev["attemptId"].as_str().unwrap();
    let original = queries::load_attempts_with_mode(&c, &queries::Scope::Attempt(id), true)
        .unwrap()[0]
        .raw_score();
    let v = json!({"attemptId":id,"modality":"reading","passageIndex":1,"responseSlotId":"q-data-p1","newIsCorrect":true,"newScore":1,"reason":"接受变体","operator":"老师","expectedVersion":1});
    let over = review::create_override(&mut c, &v).unwrap();
    assert_eq!(
        queries::load_attempts(&c, &queries::Scope::Attempt(id)).unwrap()[0].raw_score(),
        original + 1.0
    );
    assert_eq!(
        queries::load_attempts_with_mode(&c, &queries::Scope::Attempt(id), true).unwrap()[0]
            .raw_score(),
        original
    );
    assert!(review::create_override(&mut c, &v)
        .unwrap_err()
        .starts_with("VERSION_CONFLICT"));
    let mut duplicate = v.clone();
    duplicate["expectedVersion"] = json!(2);
    assert!(review::create_override(&mut c, &duplicate).is_err());
    assert!(review::revoke_override(&mut c,&json!({"overrideId":over["override_id"],"expectedVersion":2,"reason":"撤销","operator":"老师"})).is_err());
    review::revoke_override(&mut c,&json!({"overrideId":over["override_id"],"expectedVersion":1,"reason":"撤销","operator":"老师"})).unwrap();
    assert_eq!(
        queries::load_attempts(&c, &queries::Scope::Attempt(id)).unwrap()[0].raw_score(),
        original
    );
    assert_eq!(review::list_audit(&c, &json!({})).unwrap()["n"], 2);
    assert_eq!(
        detail::attempt_detail(&c, id).unwrap()["passages"][0]["answers"][0]["user_answer_json"],
        "\"wrong\""
    );
    drop(c);
    std::fs::remove_dir_all(app).unwrap();
}
#[test]
fn writing_grade_cas_annotations_review_publish_and_frozen_versions() {
    let (app, mut c, ev) = load("v2");
    let id = ev["attemptId"].as_str().unwrap();
    let input = json!({"attemptId":id,"writingSubmissionId":ev["writingSubmissionId"],"taTr":6.5,"cc":7,"lr":6,"gra":7.5,"operator":"老师","overallComment":"总评","expectedVersion":0});
    let grade = review::save_grade(&mut c, &input).unwrap();
    assert_eq!(grade["task_band"], 6.75);
    let gid = grade["grade_id"].as_str().unwrap();
    let anno = json!({"gradeId":gid,"operator":"老师","expectedVersion":1,"startOffset":0,"endOffset":4,"quotedText":"This","category":"语法","comment":"批注"});
    let saved = review::annotation(&mut c, &anno, false).unwrap();
    assert_eq!(saved["gradeVersion"], 2);
    assert!(review::annotation(&mut c, &anno, false).is_err());
    let review_input = json!({"gradeId":gid,"operator":"复核老师","expectedVersion":2});
    review::transition_grade(&mut c, &review_input, false).unwrap();
    assert!(review::annotation(&mut c,&json!({"gradeId":gid,"operator":"老师","expectedVersion":3,"annotationId":saved["annotation"]["annotation_id"]}),true).is_err());
    review::transition_grade(
        &mut c,
        &json!({"gradeId":gid,"operator":"老师","expectedVersion":3}),
        true,
    )
    .unwrap();
    assert!(review::transition_grade(
        &mut c,
        &json!({"gradeId":gid,"operator":"老师","expectedVersion":4}),
        true
    )
    .is_err());
    let second = review::save_grade(&mut c, &input).unwrap();
    review::transition_grade(
        &mut c,
        &json!({"gradeId":second["grade_id"],"operator":"老师","expectedVersion":1}),
        false,
    )
    .unwrap();
    review::transition_grade(
        &mut c,
        &json!({"gradeId":second["grade_id"],"operator":"老师","expectedVersion":2}),
        true,
    )
    .unwrap();
    let n: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM writing_grades WHERE status='published'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
    assert_eq!(
        review::writing_queue(&c, Some(ev["paperId"].as_str().unwrap()), Some("published"))
            .unwrap()["n"],
        1
    );
    drop(c);
    std::fs::remove_dir_all(app).unwrap();
}
#[test]
fn rejudge_preview_is_read_only_and_apply_detects_drift() {
    let (app, mut c, ev) = load("v2");
    let rule = json!({"scope":"slot","modality":"reading","examId":"data-p1","responseSlotId":"q-data-p1","acceptedAnswers":["wrong"]});
    let preview = review::preview_rejudge(&c, &rule).unwrap();
    assert_eq!(preview["affectedCount"], 1);
    assert_eq!(review::list_audit(&c, &json!({})).unwrap()["n"], 0);
    let input = json!({"rule":rule,"operator":"老师","reason":"新增可接受变体","previewToken":preview["previewToken"]});
    let result = review::apply_rejudge(&mut c, &input).unwrap();
    assert_eq!(result["affectedCount"], 1);
    assert!(review::apply_rejudge(&mut c, &input)
        .unwrap_err()
        .starts_with("VERSION_CONFLICT"));
    let id = ev["attemptId"].as_str().unwrap();
    let d = detail::attempt_detail(&c, id).unwrap();
    let version = d["attempt"]["version"].as_i64().unwrap();
    review::set_counted(&mut c,&json!({"attemptId":id,"counted":false,"reason":"练习作答","operator":"老师","expectedVersion":version})).unwrap();
    assert!(!queries::load_attempts(&c, &queries::Scope::Attempt(id)).unwrap()[0].is_valid());
    review::set_counted(&mut c,&json!({"attemptId":id,"counted":true,"reason":"恢复","operator":"老师","expectedVersion":version+1})).unwrap();
    assert!(queries::load_attempts(&c, &queries::Scope::Attempt(id)).unwrap()[0].is_valid());
    let report = export::write_export_audited(
        &c,
        "data-v2-fixture",
        ev["paperId"].as_str().unwrap(),
        &app.join("report.xlsx"),
        "老师",
    )
    .unwrap();
    assert_eq!(report.sheets.len(), 8);
    // All four decision types have audit rows; preview/export helper do not mutate facts.
    assert!(
        review::list_audit(&c, &json!({})).unwrap()["n"]
            .as_u64()
            .unwrap()
            >= 4
    );
    drop(c);
    std::fs::remove_dir_all(app).unwrap();
}

/// Fixture preparation only; the E2E must still publish it through the Tauri UI.
#[test]
#[ignore = "explicit E2E fixture generation; requires BACKEND_V2_LISTENING_FIXTURE_OUT"]
fn dump_backend_v2_listening_authoring() {
    let out = PathBuf::from(
        std::env::var("BACKEND_V2_LISTENING_FIXTURE_OUT").expect("output directory required"),
    );
    std::fs::create_dir_all(&out).unwrap();
    let mut authoring =
        serde_json::to_value(crate::test_support::complete_listening_exam()).unwrap();
    for answer in authoring["answerKey"].as_object_mut().unwrap().values_mut() {
        answer["values"] = json!(["answer"]);
    }
    authoring["quality"] = json!({"state":"ready","issues":[]});
    std::fs::write(
        out.join("authoring.json"),
        serde_json::to_vec_pretty(&authoring).unwrap(),
    )
    .unwrap();
    crate::test_support::stage_listening_audio(&out, &[1, 2, 3, 4]);
    println!("BACKEND_V2_LISTENING_FIXTURE={}", out.display());
}

#[test]
fn mutation_boundaries_reject_missing_operator_bad_targets_and_stale_versions() {
    let (app, mut c, ev) = load("v2");
    let id = ev["attemptId"].as_str().unwrap();
    let mut input = json!({"attemptId":id,"modality":"listening","responseSlotId":"q1","newIsCorrect":false,"newScore":0,"reason":"复核","operator":"老师","expectedVersion":1});
    let mut invalid = input.clone();
    invalid["operator"] = json!("");
    assert!(review::create_override(&mut c, &invalid).is_err());
    invalid = input.clone();
    invalid["responseSlotId"] = json!("missing");
    assert!(review::create_override(&mut c, &invalid).is_err());
    invalid = input.clone();
    invalid["newScore"] = json!(2);
    assert!(review::create_override(&mut c, &invalid).is_err());
    let over = review::create_override(&mut c, &input).unwrap();
    assert_eq!(
        detail::attempt_detail(&c, id).unwrap()["listening"][0]["effectiveScore"],
        8.0
    );
    assert_eq!(
        listening::statistics(&c, ev["paperId"].as_str().unwrap(), true).unwrap()["students"][0]
            ["score"],
        9.0
    );
    review::revoke_override(&mut c,&json!({"overrideId":over["override_id"],"operator":"老师","expectedVersion":1,"reason":"恢复"})).unwrap();
    input["modality"] = json!("writing");
    input["expectedVersion"] = json!(3);
    assert!(review::create_override(&mut c, &input).is_err());
    let grade=review::save_grade(&mut c,&json!({"attemptId":id,"writingSubmissionId":ev["writingSubmissionId"],"operator":"老师","taTr":6,"cc":6,"lr":6,"gra":6,"expectedVersion":0})).unwrap();
    let annotation=review::annotation(&mut c,&json!({"gradeId":grade["grade_id"],"operator":"老师","expectedVersion":1,"startOffset":0,"endOffset":4,"quotedText":"This","category":"其他","comment":"删除测试"}),false).unwrap();
    review::annotation(&mut c,&json!({"gradeId":grade["grade_id"],"operator":"老师","expectedVersion":2,"annotationId":annotation["annotation"]["annotation_id"]}),true).unwrap();
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM writing_annotations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let before: i64 = c
        .query_row("SELECT COUNT(*) FROM audit_log", [], |r| r.get(0))
        .unwrap();
    assert!(review::annotation(&mut c,&json!({"gradeId":grade["grade_id"],"operator":"老师","expectedVersion":3,"startOffset":0,"endOffset":3,"quotedText":"wrong","category":"其他","comment":"bad"}),false).is_err());
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM audit_log", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before
    );
    review::set_operator(&mut c, &json!({"operator":"操作人"})).unwrap();
    assert_eq!(review::operator_name(&c, None).unwrap(), "操作人");
    drop(c);
    std::fs::remove_dir_all(app).unwrap();
}

#[test]
fn actual_electron_ui_snapshot_imports_listening_reading_and_writing() {
    let (app, c, ev) = load("v2-electron");
    let detail = detail::attempt_detail(&c, ev["attemptId"].as_str().unwrap()).unwrap();
    assert_eq!(
        detail["listening"][0]["answers"].as_array().unwrap().len(),
        9
    );
    assert_eq!(detail["events"].as_array().unwrap().len(), 14);
    assert_eq!(detail["passages"].as_array().unwrap().len(), 3);
    assert!(detail["writings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w["content"]
            .as_str()
            .unwrap_or_default()
            .contains("Complete listening reading and writing")));
    assert!(
        detail["anomalies"].as_array().unwrap().is_empty(),
        "{detail}"
    );
    drop(c);
    std::fs::remove_dir_all(app).unwrap();
}

#[test]
fn identity_alias_cas_and_cycle_protection_preserve_original_candidate() {
    let (app, mut c, ev) = load("v2");
    c.execute("INSERT INTO attempts(attempt_id,terminal_id,batch_id,candidate_id,candidate_name,status,exported_at,sha256,source_file,imported_at) VALUES('other','t','b','V2-002','另一名学生','submitted','now','hash','file','now')",[]).unwrap();
    let alias = json!({"candidateId":"V2-001","candidateName":"数据测试学生","mergedInto":"V2-002","expectedVersion":0,"operator":"老师","reason":"确认同一人"});
    let merged = review::merge_identity(&mut c, &alias).unwrap();
    assert_eq!(merged["version"], 1);
    assert!(review::merge_identity(&mut c, &alias)
        .unwrap_err()
        .starts_with("VERSION_CONFLICT"));
    assert!(review::merge_identity(&mut c,&json!({"candidateId":"V2-002","candidateName":"另一名学生","mergedInto":"V2-001","expectedVersion":0,"operator":"老师","reason":"循环测试"})).unwrap_err().starts_with("IDENTITY_CYCLE"));
    assert_eq!(
        detail::attempt_detail(&c, ev["attemptId"].as_str().unwrap()).unwrap()["attempt"]
            ["candidate_id"],
        "V2-001"
    );
    assert_eq!(
        review::list_audit(&c, &json!({"targetType":"candidate"})).unwrap()["n"],
        1
    );
    let denied = app.join("denied.xlsx");
    assert!(export::write_export_audited(
        &c,
        "data-v2-fixture",
        ev["paperId"].as_str().unwrap(),
        &denied,
        ""
    )
    .is_err());
    assert!(!denied.exists());
    drop(c);
    std::fs::remove_dir_all(app).unwrap();
}
