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
pub(crate) const LIBRARY_V2_SCHEMA_VERSION: i64 = 9;

pub(crate) fn ensure_v2_schema(conn: &Connection) -> CommandResult<()> {
    let transaction = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| format!("library_v2_migrate_begin:{error}"))?;
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
    transaction.commit().map_err(|error| format!("library_v2_migrate_commit:{error}"))
}

/// 版本化迁移步骤：`(target_version, DDL)`。只追加，不修改历史步骤。
fn migrations() -> Vec<(i64, &'static str)> {
    vec![
        (1, LIBRARY_V2_SCHEMA_SQL),
        // v2：事件序号（M2）。`processing://item-updated` 携带可比较的状态版本，
        // 前端据此丢弃重复/乱序事件（计划 §3 接口契约）。
        (2, "ALTER TABLE processing_jobs_v2 ADD COLUMN event_seq INTEGER NOT NULL DEFAULT 0;"),
        // v3：durable 取消标记（G1/A4-F02 + P0-2）。运行中任务的取消此前只存在
        // 内存 HashSet，重启即丢并被恢复逻辑重新入队；现在落库，启动恢复时兑现。
        // 同时修正旧版存量数据：旧恢复逻辑对"重试耗尽"路径也写 interrupted，
        // 导致 UI 谎称已自动重试（stage=ready_for_review + action_required 组合
        // 只可能来自旧耗尽路径；新代码该路径写 retry_exhausted）。
        (
            3,
            "ALTER TABLE processing_jobs_v2 ADD COLUMN cancel_requested_at TEXT;
             UPDATE processing_jobs_v2 SET last_error_code = 'retry_exhausted'
             WHERE stage = 'ready_for_review' AND local_status = 'action_required'
               AND last_error_code = 'interrupted';",
        ),
        // v4：识别闭环（生成批次 / 统一裁决 / 决策幂等）。数据库是**读取权威**，
        // job 目录下的 JSON 只作证据留痕（可被清理策略回收而不影响前端读取）。
        (4, RECOGNITION_CLOSED_LOOP_SQL),
        // v5：人工编辑保护目标（云端自动修复的授权边界）。
        //
        // 为什么需要独立一列，而不是继续靠稿件里的 `provenanceStatus == user_edited`：
        //  1. 那个标记只覆盖「命令自己带的 nodeId」，`setAnswer` / `setResponseGroup`
        //     这类命令真正保护的是**槽位、答案项与整组结构**，标记落在别处；
        //  2. `editor_journal_v1` 只保留最近 200 条，靠它算保护范围会随时间静默失效；
        //  3. 云端修复需要一个**能在同一事务里比对**的目标集合，而不是每次重新
        //     遍历整份稿件。
        // 历史数据首读时由稿件里的 `user_edited` 标记惰性导出（见
        // `repository::human_protected_targets`），不改历史 DDL。
        (
            5,
            "ALTER TABLE library_items_v2 ADD COLUMN protected_edits_json TEXT;",
        ),
        // v6：编辑日志补来源与批次归属。
        //
        // 旧列只有一个 `command_json`，无法回答「这次改动是人做的还是云端修复做的」，
        // 于是撤销整轮自动修复时无法把云端写的东西与人写的东西区分开。新增四列：
        //   edit_origin   human | cloud_repair | undo（由调用入口决定，绝不来自请求体）
        //   repair_run_id 属于哪一次自动修复 run（人工编辑为空）
        //   change_json   受影响目标的 before/after（撤销的依据）
        //   result_json   实际提交结果（apply / reject 及原因）
        // 历史行为 NULL，按「来源不明」处理：不参与自动撤销，但也不被当成 Human 保护。
        (
            6,
            "ALTER TABLE editor_journal_v1 ADD COLUMN edit_origin TEXT;
             ALTER TABLE editor_journal_v1 ADD COLUMN repair_run_id TEXT;
             ALTER TABLE editor_journal_v1 ADD COLUMN change_json TEXT;
             ALTER TABLE editor_journal_v1 ADD COLUMN result_json TEXT;
             CREATE INDEX IF NOT EXISTS idx_editor_journal_v1_repair_run
                 ON editor_journal_v1(repair_run_id)
                 WHERE repair_run_id IS NOT NULL;",
        ),
        // v7：云端自主修复的摘要（批次级）。
        //
        // 为什么放批次行而不是只留 artifact：artifact（`repair.json`）是诊断副本，
        // 会被清理策略回收，且 job 目录重建后不再存在；而「这次修复跑到什么状态、
        // 还剩哪些用户任务、能不能撤销」是前端每次打开都要读的产品状态，必须和批次
        // 一起在同一事务里提交。旧行取 NULL，前端按「无修复记录」降级——注意
        // **不能**把 NULL 读成 completed。
        (
            7,
            "ALTER TABLE recognition_batches_v1 ADD COLUMN repair_json TEXT;",
        ),
        // Listening managed audio bindings. Self-contained and idempotent (CREATE ... IF NOT
        // EXISTS), so the step can be renumbered when parallel migrations merge.
        (8, crate::listening_audio::store::LISTENING_AUDIO_ASSETS_SQL),
        // v9：发布记录与「题库保存」的最终版证据（自包含步骤，合并时可重新编号）。
        //
        // - `publish_records_v2`：每次发布一行。`forced = 1` 表示这次发布时门禁结论
        //   不是 Ready、由用户点击发布显式放行；`verdict_json` 是门禁原样结论，**从不**
        //   被改写成 Ready。
        // - `library_final_versions_v2`：每个条目只有一份最终版（权威稿本身仍是
        //   `library_items_v2.canonical_ds_json`，这里不复制第二份稿），只存发布时冻结的
        //   证据摘要与原文件清理状态。原文件与过程文件被删除后，质量评估靠这份冻结证据
        //   继续核对题号集合。
        (9, PUBLISH_RECORDS_AND_FINAL_VERSION_SQL),
    ]
}

const PUBLISH_RECORDS_AND_FINAL_VERSION_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS publish_records_v2 (
    id                TEXT PRIMARY KEY,
    library_item_id   TEXT NOT NULL REFERENCES library_items_v2(id),
    edit_version      INTEGER NOT NULL,
    batch_id          TEXT NOT NULL,
    destination       TEXT NOT NULL,
    forced            INTEGER NOT NULL CHECK (forced IN (0, 1)),
    verdict_json      TEXT NOT NULL,
    reasons_json      TEXT NOT NULL,
    student_loadable  INTEGER NOT NULL CHECK (student_loadable IN (0, 1)),
    status            TEXT NOT NULL,
    created_at        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_publish_records_v2_item
    ON publish_records_v2(library_item_id, created_at);

CREATE TABLE IF NOT EXISTS library_final_versions_v2 (
    library_item_id   TEXT PRIMARY KEY REFERENCES library_items_v2(id),
    edit_version      INTEGER NOT NULL,
    published_at      TEXT NOT NULL,
    publish_record_id TEXT,
    evidence_json     TEXT NOT NULL,
    source_purged_at  TEXT,
    purge_report_json TEXT,
    updated_at        TEXT NOT NULL
);
"#;

const RECOGNITION_CLOSED_LOOP_SQL: &str = r#"
-- 一次导入 = 一个生成批次。重试复用同一 batch_id（幂等 apply 的前提）。
CREATE TABLE IF NOT EXISTS recognition_batches_v1 (
    batch_id          TEXT PRIMARY KEY,
    library_item_id   TEXT NOT NULL REFERENCES library_items_v2(id),
    job_id            TEXT NOT NULL,
    base_edit_version INTEGER NOT NULL,
    source_sha256     TEXT NOT NULL DEFAULT '',
    local_status      TEXT NOT NULL DEFAULT 'not_started',
    cloud_status      TEXT NOT NULL DEFAULT 'not_started',
    source_status     TEXT NOT NULL DEFAULT 'not_started',
    cloud_reason_code TEXT,
    source_reason_code TEXT,
    -- 四阶段完整状态（RecognitionChainStateV1）。churn 低、字段演进频繁，
    -- 因此整块存 JSON；上面的 status/reason 列保留，供 SQL 过滤与索引使用。
    stages_json       TEXT NOT NULL DEFAULT '{}',
    agreed_count       INTEGER NOT NULL DEFAULT 0,
    auto_fixed_count   INTEGER NOT NULL DEFAULT 0,
    needs_review_count INTEGER NOT NULL DEFAULT 0,
    unverifiable_count INTEGER NOT NULL DEFAULT 0,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_recognition_batches_v1_item
    ON recognition_batches_v1(library_item_id, created_at);

-- 统一裁决逐项落库：前端读取的权威来源。同一 batch 内 decision_id 唯一，
-- 因此「同一问题只有一张建议卡」在存储层也被强制。
CREATE TABLE IF NOT EXISTS recognition_decisions_v1 (
    batch_id        TEXT NOT NULL,
    decision_id     TEXT NOT NULL,
    library_item_id TEXT NOT NULL REFERENCES library_items_v2(id),
    resolution      TEXT NOT NULL,
    code            TEXT NOT NULL,
    severity        TEXT NOT NULL,
    target_type     TEXT NOT NULL,
    target_id       TEXT NOT NULL,
    field           TEXT NOT NULL,
    status          TEXT NOT NULL,
    item_json       TEXT NOT NULL,
    applied_at      TEXT,
    updated_at      TEXT NOT NULL,
    PRIMARY KEY (batch_id, decision_id)
);
CREATE INDEX IF NOT EXISTS idx_recognition_decisions_v1_item
    ON recognition_decisions_v1(library_item_id, batch_id);

-- 决策提交幂等日志：同一 request_id 的重复提交不重复写入权威稿。
CREATE TABLE IF NOT EXISTS recognition_decision_journal_v1 (
    request_id        TEXT PRIMARY KEY,
    library_item_id   TEXT NOT NULL,
    batch_id          TEXT NOT NULL,
    base_edit_version INTEGER NOT NULL,
    payload_json      TEXT NOT NULL,
    result_json       TEXT NOT NULL,
    created_at        TEXT NOT NULL
);
"#;

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
            "recognition_batches_v1",
            "recognition_decisions_v1",
            "recognition_decision_journal_v1",
            "listening_audio_assets_v1",
            "publish_records_v2",
            "library_final_versions_v2",
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

    /// v7 迁移必须真的把 `repair_json` 加上。
    ///
    /// 这一列是前端读取「这次修复到什么状态 / 还剩哪些用户任务」的权威来源，而批次读取
    /// 用的是**固定列序**的 SELECT——列不存在时不是降级，而是整行读取直接失败
    /// （`recognition_load_batch:Invalid column index`）。所以列的存在必须被显式钉住，
    /// 不能只靠 `user_version` 对得上。
    #[test]
    fn v7_migration_adds_the_batch_repair_column() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_v2_schema(&conn).unwrap();
        let present: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('recognition_batches_v1')
                 WHERE name = 'repair_json'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(present, 1, "repair_json 列必须存在");
    }

    #[test]
    fn canonical_ds_rejects_missing_modality() {        let conn = Connection::open_in_memory().unwrap();
        ensure_v2_schema(&conn).unwrap();
        let insert = conn.execute(
            "INSERT INTO library_items_v2 (id, modality, title, status, created_at, updated_at)
             VALUES ('it-1', 'poetry', 't', 'ready', '2026-01-01', '2026-01-01')",
            [],
        );
        assert!(insert.is_err(), "modality CHECK 必须拒绝非法值");
    }
}
