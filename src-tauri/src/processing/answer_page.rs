//! 答案页识别这**一步**（不连带整条识别链）。
//!
//! 为什么单独成一步：答案页识别失败时，工作区的「重试」以前会重新入队整条流水线，
//! 连十分钟的云端修复一起重跑——用户只是想再请求一次视觉服务。这里只跑
//! `recognize_and_apply_pdf_answers`，并且：
//!
//! - 服务暂时不可用（`not_executed / service_unavailable`）时**自动再试一次**；
//! - 视觉输出没通过校验（`failed`）**不重试**，答案保持未解析——绝不为了「有结果」
//!   去碰运气写入一个没核验过的答案；
//! - 结果写回 `pipeline-report.json` 的 `parser.visionAnswerExtraction`，工作区的答案页
//!   提示读的就是这一格；不写回的话，调度器里那次真实的答案页识别在界面上根本不存在。

use std::path::Path;

use serde_json::{json, Value};

use crate::util::{job_dir, read_json_opt, write_json};
use crate::CommandResult;

/// 这次失败值不值得自动再试一次：只有「服务暂时不可用」。
pub(crate) fn answer_page_failure_is_retryable(report: &Value) -> bool {
    report.get("state").and_then(Value::as_str) == Some("not_executed")
        && report.get("stateReason").and_then(Value::as_str) == Some("service_unavailable")
}

/// 跑一次答案页识别（可自动重试一次），并把结果写回流水线报告。
///
/// `runner` 是注入点：生产传 `recognize_and_apply_pdf_answers`，测试传确定性桩。
pub(crate) fn run_answer_page_step(
    root: &Path,
    job_id: &str,
    profile_id: &str,
    runner: &mut dyn FnMut(&Path, &str, &str) -> CommandResult<Value>,
) -> CommandResult<Value> {
    let mut report = runner(root, job_id, profile_id)?;
    if answer_page_failure_is_retryable(&report) {
        report = runner(root, job_id, profile_id)?;
        if let Some(object) = report.as_object_mut() {
            object.insert("autoRetried".to_string(), json!(true));
        }
    }
    persist_answer_page_state(root, job_id, &report)?;
    Ok(report)
}

/// 工作区「重试答案页识别」：按**此刻**的云端设置只重跑这一步。
pub(crate) fn retry_answer_page_at_root(
    root: &Path,
    job_id: &str,
    runner: &mut dyn FnMut(&Path, &str, &str) -> CommandResult<Value>,
) -> CommandResult<Value> {
    let profiles = crate::llm_profiles::load_profiles(root).unwrap_or_default();
    let Some(profile) = super::scheduler::current_cloud_profile(&profiles) else {
        // 没有可用的云端连接：什么都不跑，如实返回，前端据此引导去设置页。
        return Ok(json!({
            "source": "answer_page_recognition",
            "attempted": false,
            "applied": false,
            "state": "not_executed",
            "stateReason": "no_cloud_profile",
        }));
    };
    run_answer_page_step(root, job_id, &profile, runner)
}

/// 把答案页识别结果写进 `pipeline-report.json` 的 `parser.visionAnswerExtraction`。
fn persist_answer_page_state(root: &Path, job_id: &str, report: &Value) -> CommandResult<()> {
    let path = job_dir(root, job_id).join("pipeline-report.json");
    let mut pipeline = read_json_opt(&path)?.unwrap_or_else(|| json!({}));
    if !pipeline.is_object() {
        pipeline = json!({});
    }
    let mut extraction = report.clone();
    if let Some(object) = extraction.as_object_mut() {
        // 前端读 `answerCount`；应用报告里叫 `appliedCount`。
        if !object.contains_key("answerCount") {
            let applied = object.get("appliedCount").cloned().unwrap_or(json!(0));
            object.insert("answerCount".to_string(), applied);
        }
    }
    if !pipeline.get("parser").map(Value::is_object).unwrap_or(false) {
        pipeline["parser"] = json!({});
    }
    pipeline["parser"]["visionAnswerExtraction"] = extraction;
    write_json(&path, &pipeline)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::ensure_app_dirs;

    fn temp_root(job_id: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("answer-page-step-{}", uuid::Uuid::new_v4().simple()));
        ensure_app_dirs(&root).unwrap();
        std::fs::create_dir_all(job_dir(&root, job_id)).unwrap();
        root
    }

    fn state_of(root: &Path, job_id: &str) -> Value {
        read_json_opt(&job_dir(root, job_id).join("pipeline-report.json"))
            .unwrap()
            .unwrap()
            .pointer("/parser/visionAnswerExtraction")
            .cloned()
            .unwrap()
    }

    #[test]
    fn a_temporary_outage_is_retried_once_and_the_result_reaches_the_workspace() {
        let root = temp_root("job-1");
        let mut calls = 0;
        let report = run_answer_page_step(&root, "job-1", "p1", &mut |_, _, _| {
            calls += 1;
            Ok(if calls == 1 {
                json!({"attempted": true, "state": "not_executed", "stateReason": "service_unavailable", "appliedCount": 0})
            } else {
                json!({"attempted": true, "state": "succeeded", "stateReason": "ok", "appliedCount": 13})
            })
        })
        .unwrap();
        assert_eq!(calls, 2, "服务暂时不可用必须自动再试一次");
        assert_eq!(report["state"], "succeeded");
        let persisted = state_of(&root, "job-1");
        assert_eq!(persisted["state"], "succeeded", "结果必须写回工作区读的那一格");
        assert_eq!(persisted["answerCount"], 13);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_output_that_fails_its_checks_is_not_retried_and_stays_unresolved() {
        let root = temp_root("job-1");
        let mut calls = 0;
        let report = run_answer_page_step(&root, "job-1", "p1", &mut |_, _, _| {
            calls += 1;
            Ok(json!({"attempted": true, "applied": false, "state": "failed", "stateReason": "invalid_response", "appliedCount": 0}))
        })
        .unwrap();
        assert_eq!(calls, 1, "视觉输出没通过校验不能碰运气重试");
        assert_eq!(report["applied"], false);
        assert_eq!(state_of(&root, "job-1")["state"], "failed");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn retry_without_a_cloud_profile_runs_nothing_and_says_so() {
        let root = temp_root("job-1");
        crate::llm_profiles::save_profiles(&root, &[json!({"profileId": "off", "enabled": false})]).unwrap();
        let report = retry_answer_page_at_root(&root, "job-1", &mut |_, _, _| panic!("没有云端连接时不能调用视觉服务")).unwrap();
        assert_eq!(report["stateReason"], "no_cloud_profile");
        let _ = std::fs::remove_dir_all(&root);
    }
}
