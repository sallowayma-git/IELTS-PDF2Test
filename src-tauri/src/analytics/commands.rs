//! Tauri boundary; all database work runs off the WebView thread.
use super::{archive, detail, open_analytics, review};
use crate::CommandResult;
use serde_json::Value;
use std::path::Path;
use tauri::AppHandle;

async fn query<F>(app: AppHandle, f: F) -> CommandResult<Value>
where
    F: FnOnce(&mut rusqlite::Connection, &Path) -> CommandResult<Value> + Send + 'static,
{
    let root = crate::app_root(&app).map_err(|e| format!("DATA_ROOT:无法打开应用数据目录：{e}"))?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = open_analytics(&root)?;
        f(&mut conn, &root)
    })
    .await
    .map_err(|e| format!("DATA_TASK:数据任务失败：{e}"))?
    .map_err(|error| {
        let structured = error.split_once(':').is_some_and(|(code, _)| {
            !code.is_empty() && code.bytes().all(|b| b.is_ascii_uppercase() || b == b'_')
        });
        if structured {
            error
        } else {
            format!("DATA_ERROR:数据操作失败：{error}")
        }
    })
}
#[tauri::command]
pub async fn list_papers(destination: Option<String>, app: AppHandle) -> CommandResult<Value> {
    query(app, move |conn, root| {
        let recovery = destination
            .filter(|s| !s.trim().is_empty())
            .map(|d| archive::recover(root, &d))
            .transpose()?;
        let mut value = archive::list(conn)?;
        value["recovery"] = serde_json::to_value(recovery).map_err(|e| e.to_string())?;
        Ok(value)
    })
    .await
}
#[tauri::command]
pub async fn get_paper(paper_id: String, app: AppHandle) -> CommandResult<Value> {
    query(app, move |c, _| archive::get(c, &paper_id)).await
}
#[tauri::command]
pub async fn get_attempt_detail(attempt_id: String, app: AppHandle) -> CommandResult<Value> {
    query(app, move |c, _| detail::attempt_detail(c, &attempt_id)).await
}
#[tauri::command]
pub async fn get_question_context(
    exam_id: String,
    runtime_sha256: String,
    question_id: String,
    app: AppHandle,
) -> CommandResult<Value> {
    query(app, move |c, _| {
        archive::question(c, &exam_id, &runtime_sha256, &question_id)
    })
    .await
}
#[tauri::command]
pub async fn list_overrides(attempt_id: String, app: AppHandle) -> CommandResult<Value> {
    query(app, move |c, _| review::list_overrides(c, &attempt_id)).await
}
#[tauri::command]
pub async fn list_writing_queue(
    paper_id: Option<String>,
    status: Option<String>,
    app: AppHandle,
) -> CommandResult<Value> {
    query(app, move |c, _| {
        review::writing_queue(c, paper_id.as_deref(), status.as_deref())
    })
    .await
}
#[tauri::command]
pub async fn list_audit_log(filter: Value, app: AppHandle) -> CommandResult<Value> {
    query(app, move |c, _| review::list_audit(c, &filter)).await
}
#[tauri::command]
pub async fn preview_rejudge(rule: Value, app: AppHandle) -> CommandResult<Value> {
    query(app, move |c, _| review::preview_rejudge(c, &rule)).await
}
macro_rules! write_command {
    ($name:ident,$handler:expr) => {
        #[tauri::command]
        pub async fn $name(input: Value, app: AppHandle) -> CommandResult<Value> {
            query(app, move |c, _| {
                if !input.is_object() {
                    return Err("INVALID_INPUT:写入参数必须是对象".into());
                }
                let mut input = input;
                input["operator"] =
                    serde_json::json!(review::operator_name(c, input["operator"].as_str())?);
                ($handler)(c, &input)
            })
            .await
        }
    };
}
write_command!(create_score_override, review::create_override);
write_command!(revoke_score_override, review::revoke_override);
write_command!(set_attempt_counted, review::set_counted);
write_command!(save_writing_grade, review::save_grade);
write_command!(apply_rejudge, review::apply_rejudge);
write_command!(
    review_writing_grade,
    |c: &mut rusqlite::Connection, v: &Value| review::transition_grade(c, v, false)
);
write_command!(
    publish_writing_grade,
    |c: &mut rusqlite::Connection, v: &Value| review::transition_grade(c, v, true)
);
write_command!(
    save_annotation,
    |c: &mut rusqlite::Connection, v: &Value| review::annotation(c, v, false)
);
write_command!(
    delete_annotation,
    |c: &mut rusqlite::Connection, v: &Value| review::annotation(c, v, true)
);

write_command!(set_analytics_operator, review::set_operator);
#[tauri::command]
pub async fn analytics_listening(
    paper_id: String,
    original: Option<bool>,
    app: AppHandle,
) -> CommandResult<Value> {
    query(app, move |c, _| {
        super::listening::statistics(c, &paper_id, original.unwrap_or(false))
    })
    .await
}

write_command!(merge_candidate_identity, review::merge_identity);
