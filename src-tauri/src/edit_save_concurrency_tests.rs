//! C1 并发压力回归：编辑保存与后台写库并发时，不得出现 `SQLITE_BUSY` /
//! `database is locked` / `SQLITE_BUSY_SNAPSHOT`，也不得丢写。
//!
//! 复现的真实场景（负责人反馈「使用中编辑时不时保存失败变红」）：
//! - 导入后识别/迁移仍在后台写库（`save_job` / `save_writing_job` 的双写钩子走
//!   `db::upsert_library_item`，这是 WAL 下的「先读后写」DEFERRED 事务）；
//! - 同时用户在工作区连续保存（`apply_editor_commands_core`，事务已是 IMMEDIATE）；
//! - 每次保存都新开连接，而 `open_library_connection` 每次都跑 `ensure_v2_schema`
//!   （一个空的 BEGIN IMMEDIATE 也要抢写锁）。
//!
//! 这些条件叠加会让后台 DEFERRED 写在读快照过期后拿 `SQLITE_BUSY_SNAPSHOT`
//! （不受 `busy_timeout` 保护），或让保存自身抢不到写锁。本测试用**真实命令层
//! 函数**制造这个并发，断言全程无锁错误、无丢写。属于命令层证据（UI 之下）。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::{json, Value};

use crate::db::{self, LibraryItemRecord};
use crate::library::commands::apply_editor_commands_core;
use crate::library::repository::{
    open_library_connection, seed_canonical_ds, upsert_item_shell, ApplyEditorCommandsInput,
    UpsertItemInput,
};

const EDITOR_ITEMS: usize = 4;
const EDITS_PER_ITEM: usize = 40;
const BG_WRITE_THREADS: usize = 3;
const BG_WRITE_ITERS: usize = 220;
const OPEN_CHURN_THREADS: usize = 2;
const OPEN_CHURN_ITERS: usize = 220;

fn temp_root() -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("ielts-c1-concurrency-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// 用真实的 synthetic 权威稿夹具做种子——`validate_authoring` 会反序列化成
/// `IeltsAuthoringIRV2`（要求 `examId` 等字段），手搓的极简稿过不了校验。
fn seed_ds(title: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json");
    let mut ds: Value = serde_json::from_slice(
        &std::fs::read(&path).expect("synthetic authoring fixture must exist"),
    )
    .expect("synthetic authoring fixture must be valid JSON");
    if let Some(exam) = ds.get_mut("exam").and_then(Value::as_object_mut) {
        exam.insert("title".to_string(), json!(title));
    }
    ds
}

fn library_item_record(id: &str, title: &str) -> LibraryItemRecord {
    let now = chrono::Utc::now().to_rfc3339();
    LibraryItemRecord {
        id: id.to_string(),
        subject: "reading".to_string(),
        content_type: "reading_exam".to_string(),
        title: title.to_string(),
        category: None,
        difficulty: None,
        status: "draft".to_string(),
        task_type: None,
        tags: vec![],
        source_asset_id: None,
        linked_ingest_job_id: None,
        created_at: now.clone(),
        updated_at: now,
        revision_payload_json: json!({ "schemaVersion": "ReadingAuthoringIRV1" }).to_string(),
        schema_version: "ReadingAuthoringIRV1".to_string(),
        created_from_job_id: None,
        change_reason: Some("concurrency stress".to_string()),
    }
}

fn is_lock_error(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    lowered.contains("database is locked")
        || lowered.contains("busy")
        || lowered.contains("snapshot")
        || lowered.contains("locked")
}

/// 编辑保存（IMMEDIATE 事务）与后台双写（DEFERRED 先读后写）+ 连接抢开并发时，
/// 全程不得出现锁错误，也不得丢写（每个条目最终版本 = 1 + 保存次数）。
#[test]
fn concurrent_edit_saves_and_background_writes_never_hit_sqlite_busy() {
    let root = temp_root();

    // 预置 v2 编辑条目（外壳 + 权威稿，版本从 1 起）。
    {
        let conn = open_library_connection(&root).unwrap();
        for k in 0..EDITOR_ITEMS {
            let id = format!("edit-item-{k}");
            assert!(upsert_item_shell(
                &conn,
                &UpsertItemInput {
                    id: &id,
                    modality: "reading",
                    title: &format!("Item {k}"),
                    status: "ready",
                    source_asset_id: None,
                },
            )
            .unwrap());
            assert!(seed_canonical_ds(&conn, &id, &seed_ds(&format!("Item {k}")).to_string(), "ready").unwrap());
        }
    }

    let lock_errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();

    // 编辑线程：每个条目一个线程，连续保存（仅改标题即可推进版本，走完整保存事务）。
    for k in 0..EDITOR_ITEMS {
        let root = root.clone();
        let lock_errors = Arc::clone(&lock_errors);
        handles.push(thread::spawn(move || {
            let id = format!("edit-item-{k}");
            let mut base: i64 = 1;
            for i in 0..EDITS_PER_ITEM {
                let input = ApplyEditorCommandsInput {
                    item_id: id.clone(),
                    base_version: base,
                    request_id: Some(uuid::Uuid::new_v4().to_string()),
                    commands: vec![],
                    title: Some(format!("Item {k} edit {i}")),
                };
                match apply_editor_commands_core(&root, input) {
                    Ok(value) => {
                        base = value.pointer("/editVersion").and_then(Value::as_i64).unwrap_or(base);
                    }
                    Err(error) => {
                        if is_lock_error(&error) {
                            lock_errors.lock().unwrap().push(format!("editor[{k}]: {error}"));
                        } else {
                            panic!("unexpected editor save error: {error}");
                        }
                    }
                }
            }
        }));
    }

    // 后台双写线程：真实的 `db::upsert_library_item`（旧库 save_job/save_writing_job 钩子）——
    // 这是 WAL 下的先读后写 DEFERRED 事务，正是 BUSY_SNAPSHOT 的触发点。
    for b in 0..BG_WRITE_THREADS {
        let root = root.clone();
        let lock_errors = Arc::clone(&lock_errors);
        handles.push(thread::spawn(move || {
            let conn = open_library_connection(&root).unwrap();
            let id = format!("bg-legacy-{b}");
            for i in 0..BG_WRITE_ITERS {
                let record = library_item_record(&id, &format!("bg {b} rev {i}"));
                if let Err(error) = db::upsert_library_item(&conn, &record) {
                    if is_lock_error(&error) {
                        lock_errors.lock().unwrap().push(format!("bg_write[{b}]: {error}"));
                    } else {
                        panic!("unexpected background write error: {error}");
                    }
                }
            }
        }));
    }

    // 连接抢开线程：模拟识别/门禁面板频繁读库，每次 open 都会跑 ensure_v2_schema。
    for c in 0..OPEN_CHURN_THREADS {
        let root = root.clone();
        let lock_errors = Arc::clone(&lock_errors);
        handles.push(thread::spawn(move || {
            for _ in 0..OPEN_CHURN_ITERS {
                match open_library_connection(&root) {
                    Ok(conn) => {
                        let _ = crate::library::repository::get_item(&conn, "edit-item-0");
                    }
                    Err(error) => {
                        if is_lock_error(&error) {
                            lock_errors.lock().unwrap().push(format!("open_churn[{c}]: {error}"));
                        }
                    }
                }
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }

    let errors = lock_errors.lock().unwrap();
    assert!(
        errors.is_empty(),
        "并发编辑保存 + 后台写库出现 {} 次锁错误（应为 0）：\n{}",
        errors.len(),
        errors.join("\n")
    );

    // 无丢写：每个编辑条目的最终版本 = 1（初始）+ 保存次数。
    let conn = open_library_connection(&root).unwrap();
    for k in 0..EDITOR_ITEMS {
        let id = format!("edit-item-{k}");
        let item = crate::library::repository::get_item(&conn, &id).unwrap().unwrap();
        assert_eq!(
            item.current_edit_version,
            1 + EDITS_PER_ITEM as i64,
            "条目 {id} 有丢写：最终版本 {} != {}",
            item.current_edit_version,
            1 + EDITS_PER_ITEM as i64
        );
    }
}
