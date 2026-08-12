use crate::export_artifacts::{build_manifest, build_wrapper};
use crate::export_writing_library::export_writing_library_core;
use crate::util::ensure_app_dirs;
use crate::writing_store::{save_writing_job, WritingJob, WritingJobStatus};
use chrono::Utc;
use serde_json::{json, Map, Value};
use std::{env, fs, path::PathBuf};

fn reading_source(exam_id: &str, category: &str, question_id: &str, answer: Option<&str>) -> Value {
    let mut answer_key = Map::new();
    if let Some(answer) = answer {
        answer_key.insert(question_id.to_string(), Value::String(answer.to_string()));
    }

    let mut question_display_map = Map::new();
    question_display_map.insert(
        question_id.to_string(),
        Value::String(question_id.trim_start_matches('q').to_string()),
    );

    json!({
        "schemaVersion": "ReadingExamSourceV1",
        "examId": exam_id,
        "meta": {
            "title": format!("Contract fixture {category}"),
            "category": category,
            "frequency": "contract",
            "pdfFilename": "",
            "legacyPath": "",
            "legacyFilename": "",
            "questionIntroHtml": "<h3>Questions</h3>",
            "questionUmbrellaRanges": []
        },
        "passage": {
            "blocks": [{
                "blockId": format!("{exam_id}-passage"),
                "kind": "html",
                "html": format!("<p>{category} contract passage.</p>")
            }]
        },
        "questionGroups": [{
            "groupId": format!("{exam_id}-group"),
            "kind": "short-answer",
            "questionIds": [question_id],
            "bodyHtml": format!(
                "<div><label for=\"{question_id}\">{question_id}</label><input id=\"{question_id}\" name=\"{question_id}\" type=\"text\" /></div>"
            ),
            "leadHtml": "<h3>Questions</h3>",
            "allowOptionReuse": false
        }],
        "answerKey": answer_key,
        "sourceRefs": {
            "primaryHtml": "",
            "primaryProvider": "author-contract-fixture",
            "shuiHtml": null,
            "shuiPdf": "",
            "ieltsHtml": null
        },
        "audit": {
            "matchStatus": "verified",
            "matchConfidence": 1.0,
            "verifiedAt": Utc::now().to_rfc3339(),
            "notes": "cross-repository contract fixture"
        },
        "questionOrder": [question_id],
        "questionDisplayMap": question_display_map
    })
}

fn writing_job(task_type: &str, prompt_text: &str) -> WritingJob {
    let now = Utc::now();
    WritingJob {
        job_id: format!("cross-contract-{task_type}"),
        title: format!("Contract fixture {task_type}"),
        task_type: task_type.to_string(),
        exam_id: format!("cross-contract-{task_type}"),
        prompt_text: prompt_text.to_string(),
        suggested_word_count: if task_type == "task2" { 250 } else { 150 },
        status: WritingJobStatus::ExportReady,
        created_at: now,
        updated_at: now,
    }
}

#[test]
#[ignore = "run through the student client's cross-repository contract test"]
fn writes_cross_repo_contract_fixture() {
    let output_root = PathBuf::from(
        env::var("IELTS_CONTRACT_FIXTURE_DIR")
            .expect("IELTS_CONTRACT_FIXTURE_DIR must point to an isolated fixture directory"),
    );
    fs::create_dir_all(&output_root).expect("create contract fixture directory");

    let reading_sources = vec![
        reading_source("cross-contract-p1", "P1", "q1", Some("alpha")),
        reading_source("cross-contract-p2", "P2", "q2", None),
        reading_source("cross-contract-p3", "P3", "q3", Some("gamma")),
    ];
    for source in &reading_sources {
        let exam_id = source
            .get("examId")
            .and_then(Value::as_str)
            .expect("reading fixture examId");
        fs::write(
            output_root.join(format!("{exam_id}.js")),
            build_wrapper(source).expect("build reading wrapper"),
        )
        .expect("write reading fixture");
    }
    fs::write(
        output_root.join("manifest.js"),
        build_manifest(&reading_sources).expect("build reading manifest"),
    )
    .expect("write reading manifest");

    let author_state_root = output_root.join(".author-state");
    ensure_app_dirs(&author_state_root).expect("create author fixture state");
    let task1 = writing_job("task1", "Summarise the contract fixture chart.");
    let task2 = writing_job("task2", "Discuss both contract fixture views.");
    save_writing_job(&author_state_root, &task1).expect("save task1 fixture");
    save_writing_job(&author_state_root, &task2).expect("save task2 fixture");
    export_writing_library_core(
        &author_state_root,
        &json!({
            "jobIds": [task1.job_id, task2.job_id],
            "exportDir": output_root.to_string_lossy()
        }),
    )
    .expect("export writing contract fixture");

    fs::write(
        output_root.join("cross-repo-contract.json"),
        serde_json::to_string_pretty(&json!({
            "schemaVersion": "AuthorStudentContractFixtureV1",
            "readingExamIds": reading_sources
                .iter()
                .filter_map(|source| source.get("examId").and_then(Value::as_str))
                .collect::<Vec<_>>(),
            "writingTaskTypes": ["task1", "task2"]
        }))
        .expect("serialize contract marker"),
    )
    .expect("write contract marker");
}
