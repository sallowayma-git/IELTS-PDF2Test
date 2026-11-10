use crate::{analytics, app_root, CommandResult};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::Path;
use tauri::AppHandle;

async fn with_analytics(
    app: AppHandle,
    work: impl FnOnce(&mut Connection, &Path) -> CommandResult<Value> + Send + 'static,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = analytics::open_analytics(&root)?;
        work(&mut conn, &root)
    })
    .await
    .map_err(|e| format!("ANALYTICS_JOIN: 数据处理失败：{e}"))?
    .map_err(|error| {
        let code = error.split(':').next().unwrap_or("");
        if !code.is_empty()
            && code
                .chars()
                .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
        {
            error
        } else {
            format!("ANALYTICS_OPERATION_FAILED: 数据处理失败：{error}")
        }
    })
}
#[tauri::command]
pub async fn list_papers(app: AppHandle) -> CommandResult<Value> {
    with_analytics(app, |_, root| {
        crate::library::paper_archive::list(&crate::library::repository::open_library_connection(
            root,
        )?)
    })
    .await
}
#[tauri::command]
pub async fn get_paper(paper_id: String, app: AppHandle) -> CommandResult<Value> {
    with_analytics(app, move |_, root| {
        crate::library::paper_archive::get(
            &crate::library::repository::open_library_connection(root)?,
            &paper_id,
        )
    })
    .await
}
#[tauri::command]
pub async fn get_question_context(
    exam_id: String,
    runtime_sha256: String,
    question_id: String,
    app: AppHandle,
) -> CommandResult<Value> {
    with_analytics(app, move |_, root| {
        crate::library::paper_archive::question(
            &crate::library::repository::open_library_connection(root)?,
            &exam_id,
            &runtime_sha256,
            &question_id,
        )
    })
    .await
}
#[tauri::command]
pub async fn get_attempt_detail(attempt_id: String, app: AppHandle) -> CommandResult<Value> {
    with_analytics(app, move |conn, root| {
        analytics::detail::get(conn, root, &attempt_id)
    })
    .await
}
#[tauri::command]
pub async fn list_overrides(attempt_id: String, app: AppHandle) -> CommandResult<Value> {
    with_analytics(app, move |conn, _| {
        let rows = analytics::records::query(
            conn,
            "SELECT * FROM score_overrides WHERE attempt_id=?1 ORDER BY created_at",
            &[&attempt_id],
        )?;
        Ok(json!({"n":rows.len(),"items":rows}))
    })
    .await
}
#[tauri::command]
pub async fn list_audit_log(filter: Value, app: AppHandle) -> CommandResult<Value> {
    with_analytics(app, move |conn, _| {
        analytics::review::list_audit(conn, &filter)
    })
    .await
}
#[tauri::command]
pub async fn list_writing_queue(
    paper_id: Option<String>,
    status: Option<String>,
    app: AppHandle,
) -> CommandResult<Value> {
    with_analytics(app, move |conn, _| {
        analytics::writing_review::queue(conn, paper_id.as_deref(), status.as_deref())
    })
    .await
}
#[tauri::command]
pub async fn preview_rejudge(rule: Value, app: AppHandle) -> CommandResult<Value> {
    with_analytics(app, move |conn, _| analytics::rejudge::preview(conn, &rule)).await
}
#[tauri::command]
pub async fn apply_rejudge(input: Value, app: AppHandle) -> CommandResult<Value> {
    with_analytics(app, move |conn, root| {
        analytics::rejudge::apply(conn, root, &input)
    })
    .await
}
#[tauri::command]
pub async fn get_review_settings(app: AppHandle) -> CommandResult<Value> {
    with_analytics(app, |_, root| analytics::review::settings(root)).await
}
#[tauri::command]
pub async fn analytics_listening_exam(
    batch_id: String,
    paper_id: String,
    original: Option<bool>,
    app: AppHandle,
) -> CommandResult<Value> {
    with_analytics(app, move |conn, _| {
        analytics::statistics::listening(conn, &batch_id, &paper_id, original.unwrap_or(false))
    })
    .await
}
#[tauri::command]
pub async fn save_review_settings(input: Value, app: AppHandle) -> CommandResult<Value> {
    with_analytics(app, move |_, root| {
        analytics::review::save_settings(root, &input)
    })
    .await
}
macro_rules! review_command {
    ($name:ident,$module:ident) => {
        #[tauri::command]
        pub async fn $name(input: Value, app: AppHandle) -> CommandResult<Value> {
            with_analytics(app, move |conn, root| {
                analytics::$module::mutate(conn, root, stringify!($name), &input)
            })
            .await
        }
    };
}
review_command!(create_score_override, review);
review_command!(revoke_score_override, review);
review_command!(set_attempt_counted, review);
review_command!(save_writing_grade, writing_review);
review_command!(review_writing_grade, writing_review);
review_command!(publish_writing_grade, writing_review);
review_command!(save_annotation, writing_review);
review_command!(delete_annotation, writing_review);
