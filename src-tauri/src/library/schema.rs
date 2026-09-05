//! M1（原 P2-T01）：单一权威稿的数据库表示。
//!
//! 计划 §4.3/§4.4 的五张 V2 表；迁移用 `PRAGMA user_version` 做版本化推进，
//! 旧表（exams/library_items/...）保持只读兼容，不再承载新主链写入。
//!
//! 权威语义（计划 §4.2，禁止反向写入）：
//! - `library_items_v2.canonical_ds_json` 是唯一可编辑权威稿；
//! - 运行时/发布产物只由它编译；artifact 文件树、cloud raw、preview 不回写本表。

use rusqlite::Connection;

use crate::CommandResult;

/// 当前 V2 schema 版本。每次追加 DDL 时 +1，并在 [`migrations`] 增加对应步骤。
pub(crate) const LIBRARY_V2_SCHEMA_VERSION: i64 = 1;

pub(crate) fn ensure_v2_schema(conn: &Connection) -> CommandResult<()> {
    let current: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| format!("library_v2_user_version:{error}"))?;
    let mut applied = current;
    for (version, statement) in migrations() {
        if applied < version {
            conn.execute_batch(statement)
                .map_err(|error| format!("library_v2_migrate_v{version}:{error}"))?;
            applied = version;
        }
    }
    if applied != current {
        conn.execute_batch(&format!("PRAGMA user_version = {applied};"))
            .map_err(|error| format!("library_v2_set_user_version:{error}"))?;
    }
    Ok(())
}

/// 版本化迁移步骤：`(target_version, DDL)`。只追加，不修改历史步骤。
fn migrations() -> Vec<(i64, &'static str)> {
    vec![(LIBRARY_V2_SCHEMA_VERSION, LIBRARY_V2_SCHEMA_SQL)]
}

const LIBRARY_V2_SCHEMA_SQL: &str = r#"
-- ── 唯一权威稿（计划 §4.3）──────────────────────────────────────────
CREATE TABLE IF NOT EXISTS library_items_v2 (
    id                   TEXT PRIMARY KEY,          -- 与历史 job/item id 相同，迁移保留旧 id
    modality             TEXT NOT NULL CHECK (modality IN ('reading','listening','writing')),
    title                TEXT NOT NULL,
    status               TEXT NOT NULL,             -- processing/action_required/ready/publishing/published/failed/archived/migration_required
    current_edit_version INTEGER NOT NULL DEFAULT 1,
    canonical_ds_json    TEXT,                      -- NULL = 尚无可编辑稿（外壳/迁移中）
    source_asset_id      TEXT,
    created_at           TEXT NOT NULL,
    updated_at           TEXT NOT NULL,
    deleted_at           TEXT
);

-- ── 处理任务（M2 队列的落点；M1 先建表，队列逻辑后续接入）──────────
CREATE TABLE IF NOT EXISTS processing_jobs_v2 (
    id               TEXT PRIMARY KEY,
    library_item_id  TEXT NOT NULL REFERENCES library_items_v2(id),
    source_asset_id  TEXT NOT NULL,
    stage            TEXT NOT NULL,
    local_status     TEXT NOT NULL,
    cloud_status     TEXT NOT NULL,
    reconcile_status TEXT NOT NULL,
    progress_json    TEXT NOT NULL DEFAULT '{}',
    actionable_count INTEGER NOT NULL DEFAULT 0,
    last_error_code  TEXT,
    retry_count      INTEGER NOT NULL DEFAULT 0,
    lease_owner      TEXT,
    lease_expires_at TEXT,
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_processing_jobs_v2_item
    ON processing_jobs_v2(library_item_id);
CREATE INDEX IF NOT EXISTS idx_processing_jobs_v2_stage
    ON processing_jobs_v2(stage);

-- ── 可执行问题（计划 §8.5；P8 从前端派生切换到后端落库）────────────
CREATE TABLE IF NOT EXISTS actionable_issues_v1 (
    issue_id         TEXT PRIMARY KEY,
    library_item_id  TEXT NOT NULL REFERENCES library_items_v2(id),
    target_id        TEXT,
    severity         TEXT NOT NULL,                 -- blocker/warning
    code             TEXT NOT NULL,
    title            TEXT NOT NULL,
    user_message     TEXT NOT NULL,
    suggested_action_json TEXT,
    source_anchor_json    TEXT,
    local_value_json      TEXT,
    cloud_value_json      TEXT,
    status           TEXT NOT NULL DEFAULT 'open',  -- open/resolved/ignored
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_actionable_issues_v1_item
    ON actionable_issues_v1(library_item_id, status);

-- ── 有界恢复（计划 §4.4：隐藏的崩溃恢复基础设施，不是第二用户版本）──
CREATE TABLE IF NOT EXISTS library_item_recovery_v1 (
    library_item_id TEXT PRIMARY KEY REFERENCES library_items_v2(id),
    edit_version    INTEGER NOT NULL,
    snapshot_json   TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

-- ── 编辑日志（有界 journal；request_id 支撑幂等重放）────────────────
CREATE TABLE IF NOT EXISTS editor_journal_v1 (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    library_item_id TEXT NOT NULL REFERENCES library_items_v2(id),
    base_version    INTEGER NOT NULL,
    request_id      TEXT,
    command_json    TEXT NOT NULL,
    created_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_editor_journal_v1_item
    ON editor_journal_v1(library_item_id, id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_editor_journal_v1_request
    ON editor_journal_v1(request_id)
    WHERE request_id IS NOT NULL;
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_schema_is_idempotent_and_versioned() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_v2_schema(&conn).unwrap();
        // 幂等：重复执行不再推进版本，也不报错。
        ensure_v2_schema(&conn).unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, LIBRARY_V2_SCHEMA_VERSION);
        for table in [
            "library_items_v2",
            "processing_jobs_v2",
            "actionable_issues_v1",
            "library_item_recovery_v1",
            "editor_journal_v1",
        ] {
            let name: String = conn
                .query_row(
                    "SELECT name FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(name, table);
        }
    }

    #[test]
    fn canonical_ds_rejects_missing_modality() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_v2_schema(&conn).unwrap();
        let insert = conn.execute(
            "INSERT INTO library_items_v2 (id, modality, title, status, created_at, updated_at)
             VALUES ('it-1', 'poetry', 't', 'ready', '2026-01-01', '2026-01-01')",
            [],
        );
        assert!(insert.is_err(), "modality CHECK 必须拒绝非法值");
    }
}
