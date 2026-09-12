# A3 对抗审计 — 第 4 章（Canonical Exam DS）与第 11 章（题库/数据库/过程文件重构）

- 审计日期：2026-09-12
- 审计对象：`Plan With Files/IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md` §4（333–462 行）、§11（1975–2124 行）
- 计划冻结：`bb978be` → 计划文件最后一次修改在 `5daa04f`（`git log -1 -- <plan>`）；实际 HEAD `47a3806` + 大量未提交改动
- 立场：默认每条论断为假/未完成，以代码证据证伪
- 约束遵守：只读审计，未运行 `cargo build/test`、`npm run build`；未修改任何产品代码

## 章节范围

本报告只裁决两个章节：

| 章节 | 主题 | 计划行 |
|---|---|---|
| §4.1 | `CanonicalExamDsV1 = IeltsAuthoringIRV2` 别名与 schemaVersion 不改名 | 333–360 |
| §4.2 | 派生关系与三条“禁止反向写入” | 362–380 |
| §4.3 | `library_items_v2` / `processing_jobs_v2` 最小 DDL | 382–415 |
| §4.4 | 有界恢复（snapshot / journal / debounce） | 417–444 |
| §4.5 | 资源三类与“保留原文件”默认开关 | 446–462 |
| §11.1 | 终止双事实源 | 1977–1997 |
| §11.2 | 迁移 M1–M4 | 1999–2035 |
| §11.3 | `LibraryItemStatusV2` 状态与映射 | 2037–2060 |
| §11.4 | 过程文件目录布局 | 2062–2080 |
| §11.5 | 清理机制（3 函数 / 4 触发点） | 2082–2109 |
| §11.6 | 资产 SHA-256 去重 | 2111–2123 |

对照文档：`audit-2026-09-07/report.md`（F02/F04/F12）、`repair-2026-09-07/progress.md`、`task_plan.md`（M1 complete）、仓库根 `AGENTS.md`、计划 §24 DoD（3898–3952）。

---

## 断言核对表

判定口径：`HOLDS` 证据支持；`PARTIAL` 部分成立或实现与文档有偏差；`FALSE` 被证伪或根本不存在；`UNVERIFIABLE` 无法在只读环境确证。

| # | 计划断言 | 判定 | 证据（绝对路径:行号） |
|---|---|---|---|
| 4.1-A | Rust 存在 `pub type CanonicalExamDsV1 = IeltsAuthoringIRV2;` | **FALSE** | 全仓 grep `CanonicalExamDsV1` 仅命中计划文档与 `task_plan.md`，`src-tauri/src/**` 零命中。别名从未落地 |
| 4.1-B | TS 存在 `export type CanonicalExamDsV1 = IeltsAuthoringIRV2;` | **FALSE** | 全仓 grep 仅命中计划文档；`src/**` 零命中 |
| 4.1-C | 磁盘 `schemaVersion` 不改名为 `CanonicalExamDSV1` | **HOLDS** | `src-tauri/src/schema/ielts_authoring_v2.rs`（`IeltsAuthoringIRV2`）；校验点 `src-tauri/src/authoring_v2_commands.rs:1027`。代码本来就未改名，属“遵守”而非“已决策落地” |
| 4.2-A | Canonical → RuntimeViewModelV2 / ReadingExamSourceV2 / NAS 编译产物单向派生 | **HOLDS** | 发布直读 DB 权威稿：`src-tauri/src/nas_package_v2.rs:261-269`（`get_canonical_ds`）；编译 `nas_package_v2.rs:190-202` |
| 4.2-B | NAS JS / Preview HTML / Cloud raw JSON 禁止反向写回 canonical | **HOLDS** | `canonical_ds_json` 全仓只有 3 处写入，全在 `library` 模块：`library/migration.rs:83`、`library/repository.rs:126-131`、`library/repository.rs:326-340`。`nas_package_v2.rs:154-189` 只读 shadow 且 `authoringSource=="canonical_ds"` 时跳过绑定，不写 canonical |
| 4.3-A | `library_items_v2` DDL 与实现一致 | **HOLDS** | 计划 385–396 与 `src-tauri/src/library/schema.rs:50-61` 逐字段一致（10 列） |
| 4.3-B | `processing_jobs_v2` DDL 与实现一致 | **PARTIAL** | 15 列一致，但实现多出 `event_seq INTEGER NOT NULL DEFAULT 0`（`schema.rs:44`，v2 迁移），计划 DDL 未列 |
| 4.3-C | `actionable_issues_v1` 存在 | **HOLDS** | `src-tauri/src/library/schema.rs:87-104` |
| 4.4-A | 数据库保存 `canonical_ds_json + current_edit_version` | **HOLDS** | `schema.rs:55-56`；`repository.rs:326-340` |
| 4.4-B | 停止输入 400–600ms 后保存 | **HOLDS** | `SAVE_DEBOUNCE_MS = 450`，`src/features/editor/useCanonicalEditor.ts:7,176` |
| 4.4-C | 每 30 秒 **或** 每 20 次成功保存刷新 `last_good_snapshot` | **PARTIAL** | “每 20 次”已实现：`repository.rs:203`（`RECOVERY_SNAPSHOT_EVERY = 20`）、`repository.rs:364-379`；“每 30 秒”无任何定时器，未实现 |
| 4.4-D | 只保留最近 **100** 条 journal | **FALSE** | 实现裁剪到 **200**：`repository.rs:381-385`（`ORDER BY id DESC LIMIT 200`）。文档与实现数量不一致（repair 记录也写 200） |
| 4.4-E | 成功关闭工作区后可压缩 journal | **FALSE** | 无工作区关闭钩子；裁剪只在每次保存时发生（`repository.rs:381`）。前端 `useCanonicalEditor.ts:237-244` 仅 `beforeunload` flush |
| 4.4-F | `library_item_recovery_v1` DDL 一致 | **HOLDS** | `schema.rs:107-112`（实现额外加 `REFERENCES library_items_v2(id)`） |
| 4.4-G | `editor_journal_v1` DDL 一致 | **PARTIAL** | 计划 435–441 无 `request_id`；实现多出 `request_id TEXT` 与唯一索引 `schema.rs:119,125-127`（幂等重放所需） |
| 4.5-A | 三类资源定义（canonical assets / source evidence / transient） | **PARTIAL** | canonical assets 与 transient 在代码中可辨（`assets/shadow/pdf/...` vs `cache/`），但 source evidence 独立保留策略未落地 |
| 4.5-B | 默认“保留原文件以便后续核对 = 开启” | **HOLDS** | `src/features/settings/appSettings.ts:16-27`（`keepSourceFiles: true`）；UI 文案 `src/features/settings/SettingsPage.tsx:272-285` |
| 4.5-C | 关闭开关后首次确认/成功发布即删除 source evidence | **FALSE** | `keepSourceFiles` 只写 `localStorage`（`appSettings.ts:59-73`），`src-tauri/src/**` 无任何读取点；后端无 source 保留/删除策略 |
| 11.1-A | “当前双事实源必须终止” | **FALSE（现状）** | 双写仍在：`src-tauri/src/job_store.rs:33-44`（`save_job` 写 `job.json` 后 `upsert_reading_job`）；`src-tauri/src/library_commands.rs:245,291-340`（`write_back_meta_to_source` 回写 JSON） |
| 11.1-B | `save_job()` 先写文件再 best-effort 双写 DB | **HOLDS** | `job_store.rs:33-43` |
| 11.2-M1 | 建立五张 V2 表，旧表继续可读 | **HOLDS** | `schema.rs:48-128`（五表齐全）；`db.rs:115-147` 旧表保留 |
| 11.2-M2 | 候选顺序 revision → shadow → **V1 convert** | **PARTIAL** | `library/migration.rs:29-46` 只实现 revision → shadow；V1 转换缺失（源码注释自承“V1 转换后续接入”）。仅 V1 `authoring-ir.json` 的题落为 `migration_required` 外壳（`migration.rs:63-66,147-148`） |
| 11.2-M2 | 迁移幂等，以 `migration_version` 标记 | **PARTIAL** | 幂等成立（`INSERT OR IGNORE`、`canonical_ds_json IS NULL` 守卫、`repair_shadow_seed` 版本=1 且无 journal），但**不存在** `migration_version` 标记；schema 版本用 `PRAGMA user_version`（`schema.rs:20-35`） |
| 11.2-M2 | 不覆盖用户后来编辑的 V2 行 | **HOLDS** | `repository.rs:99-133`；`migration.rs:76-91`；测试 `migration.rs:218-253` |
| 11.2-M3 | 新 UI 只读写新表；`job_store`/legacy `library_commands` 不再由新 UI 调用 | **FALSE** | 新题库仍调 legacy：`src/features/library/libraryStore.ts:3,62-77,118-126`（`listJobs`/`listLibraryExams`/`listTrashedExams`/`deleteLibraryExam`/`restoreLibraryExam`）；新导入仍 `save_job`：`src-tauri/src/processing/commands.rs:98` |
| 11.2-M4 | 删除 `save_job -> upsert_reading_job`、`write_back_meta_to_source`、legacy `exams` 查询 | **FALSE** | 三者均存活：`job_store.rs:37`、`library_commands.rs:245,293`、`library_commands.rs:211-225`（`list_library_exams_core` 仍注册于 `lib.rs:1414,1588`）。计划自述 M4 未做，属预期延期 |
| 11.3-A | 存在 `enum LibraryItemStatusV2`（7 变体） | **FALSE** | 无该 Rust 枚举；只有 `TEXT` 列 + 注释 `schema.rs:54`。实际写入的状态串为 `processing/ready/action_required/failed/published`（`processing/commands.rs:152,156`、`scheduler.rs:495`、`nas_package_v2.rs:345`）；`publishing`/`archived` 无任何写入点 |
| 11.3-B | “内部阶段 → 用户状态”映射与实现一致 | **PARTIAL** | 映射在前端而非后端：`src/features/library/libraryTypes.ts:145-154`（`processingStage`/`itemStage`/`deriveStage`）；后端不产出该映射 |
| 11.4 | 目录布局 = `authoring_hub.db + assets/sha256/<prefix>/<hash> + source-evidence/ + tmp/jobs/<job-id>/…` | **FALSE** | `ensure_app_dirs` 只建 `config/jobs/writing-jobs/logs/cache/{parser,thumbnails,preview-server}`（`src-tauri/src/util.rs:63-78`）——无 `assets/`、无 `source-evidence/`、无 `tmp/` |
| 11.4 | 不再保留 `preview/legacy/export-history/patches/revisions` 深层目录 | **FALSE** | 仍逐题创建：`util.rs:223-244`（`ensure_job_dirs`）与 `artifact_store.rs:142-165`（`ensure_job_artifact_layout`）均建 `preview/`、`legacy/`、`export-history/`、`authoring/patches/`、`authoring/revisions/` |
| 11.5-A | `cleanup_on_startup` 实现 | **FALSE** | 全仓无该符号（仅计划文档）。启动钩子 `lib.rs:1504-1527` 只做 `ensure_app_dirs` + 迁移，无清理 |
| 11.5-B | `cleanup_after_canonical_commit` 实现 | **FALSE** | 无该符号；提交后无清理调用（`library/commands.rs:62-80`） |
| 11.5-C | `cleanup_on_close_best_effort` 实现 | **FALSE** | 无该符号；无 `RunEvent::Exit`/窗口关闭清理 |
| 11.5-D | 四触发点（启动/成功/取消/退出） | **FALSE** | 仅“导出成功”路径有清理：`cleanup.rs:99`（`cleanup_transient_job_artifacts`）、`cleanup.rs:161`（`minimize_process_artifacts_after_authoring`）；启动/取消/退出均无 |
| 11.6 | `assets/sha256/<prefix>/<hash>` 内容寻址去重 | **FALSE** | 无该路径；资产落 `jobs/<id>/assets/shadow/pdf/<assetId>.<ext>`（`pdf_facts_shadow.rs:1561-1562`），SHA-256 仅作元数据（`pdf_facts_shadow.rs:1575`），无 `exists()` 去重分支 |
| 特别 | 修复轮“删除提交后整份写回与 shadow 回写”（F04） | **HOLDS** | `library/commands.rs:62-80` 单事务、提交后无二次 UPDATE；`refresh_quality_report` 仅读文件并改内存（`authoring_v2_commands.rs:935-950`）。未发现新丢稿竞态 |
| 特别 | F02“迁移读版本指针当题稿”已修 | **HOLDS** | `migration.rs:30-36` 走 `read_current_revision` → `read_revision`；测试 `migration.rs:255-272` |
| §24 DoD | 数据面四条 | **PARTIAL/FALSE** | “新题只有一个权威稿”部分成立；“job.json/legacy exams 不再参与新题写入”FALSE（`processing/commands.rs:98`、`job_store.rs:37`）；“临时 artifact 成功/取消/启动/退出后清理”FALSE（§11.5） |

---

## DDL 字段级差异

逐字段比对计划 §4.3/§4.4 DDL 与 `src-tauri/src/library/schema.rs` 实际建表语句（`processing/*` 不建表，只读写 `processing_jobs_v2`）。

### `library_items_v2`（计划 385–396 / 实现 schema.rs:50-61）
**无差异**。10 列、类型、`CHECK (modality IN (...))`、`DEFAULT 1`、可空性全部一致。

### `processing_jobs_v2`（计划 398–414 / 实现 schema.rs:64-84）
| 字段 | 计划 | 实现 | 差异 |
|---|---|---|---|
| id, library_item_id, source_asset_id, stage, local_status, cloud_status, reconcile_status, progress_json, actionable_count, last_error_code, retry_count, lease_owner, lease_expires_at, created_at, updated_at | 有 | 有 | 一致 |
| **event_seq** | 无 | `INTEGER NOT NULL DEFAULT 0`（schema.rs:44，v2 迁移追加） | **实现多 1 列**；计划 DDL 未更新 |
| 索引 `idx_processing_jobs_v2_item` / `_stage` | 无 | 有（schema.rs:81-84） | 实现多 2 索引 |

### `actionable_issues_v1`（计划未给 DDL，仅 §11.2 提及 / 实现 schema.rs:87-104）
文档只有表名，无字段级契约；实现 14 列 + 1 索引。**属 doc-only 缺规格**，无法逐字段核对。

### `library_item_recovery_v1`（计划 428–433 / 实现 schema.rs:107-112）
4 列一致；实现额外 `REFERENCES library_items_v2(id)`。**约束多 1 项**。

### `editor_journal_v1`（计划 435–441 / 实现 schema.rs:115-127）
| 字段 | 计划 | 实现 | 差异 |
|---|---|---|---|
| id, library_item_id, base_version, command_json, created_at | 有 | 有 | 一致 |
| **request_id** | 无 | `TEXT` + 部分唯一索引 `WHERE request_id IS NOT NULL`（schema.rs:119,125-127） | **实现多 1 列 + 1 索引**；为幂等重放所必需，计划 DDL 未反映 |
| 外键 | 无 | `REFERENCES library_items_v2(id)` | 实现多 1 约束 |

结论：五张表**表名与主体列**与文档一致，但 §4.3/§4.4 的 DDL 是**过时快照**——缺 `event_seq`、`request_id` 两列及全部索引/外键。任何据文档 DDL 生成迁移或做对比的下游都会漂移。

---

## 发现清单

### A3-F01 [P1] §11.5 清理机制三个函数、四个触发点全部未实现
- 结论：`cleanup_on_startup` / `cleanup_after_canonical_commit` / `cleanup_on_close_best_effort` 在 `src-tauri/src/**` 中零命中；启动钩子 `lib.rs:1504-1527` 无清理，无退出/取消钩子。仅导出成功路径存在 `cleanup.rs:99,161`。
- 证据：`Plan...:2082-2109`（目标）；`src-tauri/src/lib.rs:1504-1527`；`src-tauri/src/cleanup.rs:99,161`。
- 影响：DoD §24“临时 artifact 能在成功/取消/启动/退出后清理”不成立；`tmp/`（本就不存在）与 job 目录长期只增不减，磁盘无界增长；崩溃/强杀残留无回收。
- 建议：在 `setup` 增启动清理、`RunEvent::Exit`/窗口关闭增 best-effort 清理、取消收尾时增 per-job 清理；先明确 `tmp/` 与 `jobs/` 的职责边界。

### A3-F02 [P1] §11.4 目录布局与 §11.6 内容寻址去重均未实现，深层目录树仍在
- 结论：`ensure_app_dirs` 不建 `assets/`、`source-evidence/`、`tmp/`；`ensure_job_dirs` 与 `ensure_job_artifact_layout` 仍逐题创建 `preview/legacy/export-history/patches/revisions`；资产落 `assets/shadow/pdf/<assetId>`，SHA-256 只写元数据，无按哈希去重。
- 证据：`util.rs:63-78`、`util.rs:223-244`、`artifact_store.rs:142-165`、`pdf_facts_shadow.rs:1561-1575`。
- 影响：计划宣称的“收敛为内容寻址存储 + 浅目录”未发生；重复资产按 assetId 重复落盘；`source-evidence` 生命周期无载体。
- 建议：实现或明确降级——若保留现状，需修改 §11.4/§11.6 使其与实现一致，否则视为未完成工作。

### A3-F03 [P1] §11.1/§11.2 M3/M4 双事实源未终止，新主链仍写 legacy
- 结论：`save_job` 仍写 `job.json` 并 best-effort 双写 legacy DB（`job_store.rs:33-44`）；新导入路径直接调用它（`processing/commands.rs:98`）；新题库仍调用 `listJobs`/`listLibraryExams`/`listTrashedExams`/`deleteLibraryExam`/`restoreLibraryExam`（`libraryStore.ts:3,62-77,118-126`）；`write_back_meta_to_source` 存活（`library_commands.rs:245,293`）；legacy `exams` 查询仍注册并参与生产列表（`lib.rs:1414,1588`）。
- 证据：同上；计划 §11.1（1977–1997）、§11.2 M3/M4（2026–2035）、DoD `Plan...:3912-3913`。
- 影响：与新 UI 并存两套事实源；短时/永久分叉风险仍在；DoD 数据面“job.json/legacy exams 不再参与新题写入”不成立。计划把删除推迟到 M4，属**已知延期**，但 M1 标 complete 与 §11.1“必须终止”的口径冲突，容易误读为已收敛。
- 建议：在 M1/M2 状态旁显式标注“双写仍活跃”，并在 M4 前不要宣称“唯一权威”。

### A3-F04 [P1] §4.5 “保留原文件”开关是空操作
- 结论：`keepSourceFiles` 默认 `true`（与计划一致，HOLDS），但仅存 `localStorage`，Rust 侧零读取；关闭后“首次确认/成功发布删除 source evidence”未实现。
- 证据：`src/features/settings/appSettings.ts:16-32,59-73`；`src/features/settings/SettingsPage.tsx:272-285`；`src-tauri/src/**` 无 `keepSourceFiles`。
- 影响：用户以为关闭开关能删除原 PDF，实际无任何后端行为变化——**误导性 UI**，且与 §11.5 `source_if_retention_enabled` 直接矛盾（默认 ON 时 source 永不被清理）。
- 建议：要么实现后端保留策略，要么在设置页标注“暂未生效”。

### A3-F05 [P2] §4.4 有界恢复数量/触发与实现不符
- 结论：journal 计划 100 条、实现 200 条（`repository.rs:381-385`）；计划“每 30 秒或每 20 次”只实现 20 次（`repository.rs:203,364`）；“成功关闭工作区后压缩 journal”无钩子。
- 证据：`Plan...:423-424`；`repository.rs:203,364,381-385`；`useCanonicalEditor.ts:237-244`。
- 影响：崩溃恢复窗口语义与文档不符；恢复基础设施的实际行为需以代码为准。
- 建议：统一文档与常量（改文档为 200 或改代码为 100），并补 30 秒兜底快照。

### A3-F06 [P2] §4.1 `CanonicalExamDsV1` 别名在 Rust/TS 均不存在
- 结论：全仓仅计划与 `task_plan.md` 出现该名；产品代码零命中。所谓“业务代码中使用别名”是 doc-only 决策，未落地。
- 证据：全仓 grep `CanonicalExamDsV1`；`src-tauri/src/**`、`src/**` 零命中。
- 影响：低（不影响运行），但任何“已按计划统一 Canonical DS 概念”的表述不成立；后续以别名做类型收敛的工作仍需从头做。
- 建议：要么落地别名，要么把 §4.1 降级为“拟议”。

### A3-F07 [P2] §11.2 M2 缺 V1 转换回退，迁移覆盖不完整
- 结论：`candidate_authoring` 只有 revision → shadow；`convert_v1_authoring(legacy.authoring_ir)` 未实现。仅有 `authoring-ir.json`（V1）而无 V2 revision/shadow 的旧题会落 `migration_required` 外壳（`migration.rs:63-66,147-148`），无法自动获得可编辑稿。
- 证据：`library/migration.rs:29-46`；`Plan...:2008-2021`。
- 影响：旧题迁移“完成度”被高估；存量 V1-only 题需人工或后续里程碑补。
- 建议：实现 V1→V2 转换或明确列入后续里程碑并给出存量统计。

### A3-F08 [P2] `library_items_v2` 无软删除/恢复路径，删除仍走 legacy
- 结论：`deleted_at` 在 V2 表中从未被写入（全仓只有 legacy `library_items` 的软删除 `db.rs:902-920`）；新 UI 的删除/恢复调 `deleteLibraryExam`/`restoreLibraryExam`（legacy），V2 行不动。
- 证据：`schema.rs:60`；`repository.rs` 无 `deleted_at` 写入；`libraryStore.ts:118-126`；`db.rs:906,917`。
- 影响：数据面“新 UI 只读写新表”不成立；V2 与 legacy 删除态分叉，`list_items` 的 `deleted_at IS NULL` 过滤形同虚设。
- 建议：把软删除/恢复收敛到 V2 仓库。

### A3-F09 [P2] §11.3 `LibraryItemStatusV2` 未实现为枚举，Publishing/Archived 为死状态
- 结论：无 Rust 枚举；状态以裸字符串散落在 `processing/commands.rs:152,156`、`scheduler.rs:495`、`nas_package_v2.rs:345`。`publishing`/`archived` 无写入点；映射逻辑在前端（`libraryTypes.ts:145-154`）。
- 证据：`schema.rs:54`（注释）；上述写入点；`libraryTypes.ts:6-14,145-154`。
- 影响：计划 §11.3 的“单一状态机”未建立；发布中/归档态无产品语义；状态一致性靠约定而非类型。
- 建议：后端建枚举 + 单一映射函数，前端只消费。

### A3-F10 [P2] §4.3/§4.4 DDL 过时（缺 `event_seq`、`request_id`）
- 结论：实现比文档多 `processing_jobs_v2.event_seq` 与 `editor_journal_v1.request_id`（含唯一索引）及多组索引/外键。
- 证据：`schema.rs:44,119,125-127,81-84,108,117`。
- 影响：据文档 DDL 做迁移对比/生成会漂移；文档权威性受损。
- 建议：同步 §4.3/§4.4 DDL。

### A3-F11 [P3] §11.2 “以 `migration_version` 标记”不存在且自相矛盾
- 结论：无 `migration_version` 列/标记；schema 版本用 `PRAGMA user_version`（`schema.rs:20-35`），迁移幂等靠 SQL 守卫。计划 §4.3 DDL 也未定义该列，属计划自相矛盾。
- 证据：`schema.rs:20-35`；全仓 grep `migration_version` 零命中。
- 影响：低；但“迁移以 migration_version 标记”的表述会误导实现者。
- 建议：改为“以 `PRAGMA user_version` + SQL 守卫保证幂等”。

### 内部矛盾汇总
1. **§4.4 vs §11.4/§11.5**：§4.4 依赖前端 `localStorage` 恢复草稿（`useCanonicalEditor.ts:78,86-95,147-159`），既不在“DB 权威稿”也不在 §11.4 的“可丢弃过程目录”里，是未被两章承认的**第四存储**；§11.5 的 `flush_editor_buffers` 也未实现。恢复模型三处口径不齐。
2. **§11.1 vs §11.2 M4**：§11.1 要求“双事实源必须终止”，§11.2 却把删除双写推迟到 M4；§24 DoD 又要求“job.json/legacy exams 不再参与新题写入”。完成语义在“必须/延期/DoD”三处冲突。
3. **§11.2 vs §4.3**：§11.2 要求 `migration_version` 标记，§4.3 五张表 DDL 无此列。
4. **§4.5 vs §11.5**：§4.5 默认保留原文件（ON），§11.5 `cleanup_after_canonical_commit` 的 `source_if_retention_enabled` 在默认态永不删除 source，与 §24“临时 artifact 能清理”冲突。
5. **§24 DoD vs §11.1/§11.5**：DoD 数据面两条（job.json/legacy 不写入、四触发点清理）在 M1 complete 的当前代码下均不成立。

### 遗漏与不可行 / 风险
- **数据丢失风险（中）**：`keepSourceFiles=false` 被宣传为可删除原文件，但无实现——不存在“误删”风险；真正风险在**反向**：用户以为已清理，原 PDF 仍在盘上（隐私/合规口径错误）。
- **迁移不可逆（低-中）**：`repair_shadow_seed` 在 `current_edit_version=1` 且无 journal 时**覆盖** `canonical_ds_json`（`migration.rs:82-89`），依赖“shadow == current 且 current != revision”三重守卫；守卫正确但无回滚路径，误判会静默替换权威稿。
- **成本低估**：§11.4/§11.5/§11.6 三项被写成“目标策略”，实际是从零新建（目录、清理、去重三套基础设施），工作量远超文档呈现；`tmp/jobs/<job-id>/{input,render,local,cloud,reconcile}` 的五段结构与现有 `cache/`、`extraction/`、`preview/` 需整体重排，属破坏性迁移。
- **遗漏工作**：V1→V2 转换（F07）、V2 软删除（F08）、后端状态枚举（F09）、source 保留策略（F04）均无归属里程碑的显式条目。

---

## 与历史审计的关系

| 历史发现 | 本轮结论 | 证据 |
|---|---|---|
| F02 迁移忽略当前保存的 revision | **已修复（HOLDS）** | `migration.rs:30-36` 解析 revision 指针；测试 `migration.rs:255-272` |
| F04 提交后质量刷新整份覆盖 DB | **已修复（HOLDS）**，未见新竞态 | `library/commands.rs:62-80` 单事务；无提交后二次 UPDATE；`refresh_quality_report` 只读文件改内存 |
| F12 发布绑定 mutable shadow | **部分缓解**：`authoringSource=="canonical_ds"` 时跳过 shadow 绑定（`nas_package_v2.rs:154,205`），但 legacy 路径仍读 shadow；发布状态写 `nas_package_v2.rs:345` | 同左 |
| M1 标 complete | **口径过高**：五表与事务编辑成立，但 §11.1 双写未终止、§4.1 别名缺失、§11.3 枚举缺失、§4.4 journal 数量不符 | 见上 |
| repair 声称“删除提交后整份写回与 shadow 回写” | **属实** | `commands.rs:62-80` |
| repair 声称“bounded journal to 200” | **属实但与计划 100 冲突** | `repository.rs:383` |
| audit-2026-09-07 未覆盖 §11.4/§11.5/§11.6 | 本轮新增：三项均未实现（A3-F01/F02） | 见上 |

**F04 修复的边界说明**：修复彻底移除了“提交后读-刷新-整份写回”这一丢稿竞态，且 `refresh_quality_report` 不再产生文件写入，事务语义干净。未发现修复引入新的并发缺陷（`Immediate` 事务 + `WHERE current_edit_version = ?5` 乐观锁 + request_id 唯一索引三层保护）。唯一残留是 `repair_shadow_seed` 的迁移期整份覆盖，但它被版本=1、无 journal、内容精确匹配三重守卫，且只在迁移入口触发，不属提交后竞态。

---

## 证据层级与局限

| 证据 | 层级 | 说明 |
|---|---|---|
| `library/schema.rs`、`repository.rs`、`migration.rs`、`commands.rs` 的建表/事务/迁移代码 | **product**（命令级可执行路径） | 新主链真实后端逻辑 |
| `processing/commands.rs:98`、`job_store.rs:33-44`、`library_commands.rs:245` | **product** | 新导入实际触发双写 |
| `libraryStore.ts`、`libraryTypes.ts`、`useCanonicalEditor.ts`、`appSettings.ts` | **product**（前端运行路径） | 题库/编辑器/设置真实行为 |
| `util.rs`、`artifact_store.rs`、`pdf_facts_shadow.rs` 目录与资产写入 | **static** | 落盘路径由代码常量确定，未在运行实例上采样 |
| 计划 §4/§11 文本 | **doc-only** | 被审计对象 |
| DoD §24 | **doc-only** | 验收口径 |

局限：
1. **未运行产品**：遵守约束，未执行 `cargo test`/`npm run build`/真实 Tauri；所有结论为静态代码 + 只读 grep/read 推导。F04/F02 的“已修复”为静态确认，未做并发压测复现。
2. **未采样真实 AppData**：`assets/sha256`、`source-evidence`、`tmp` 是否存在“历史遗留目录”未核实；本报告只断言**代码不再创建**它们（`ensure_app_dirs`/`ensure_job_dirs` 是唯一创建入口）。
3. **工作区未提交**：`library/*`、`processing/*`、前端多文件均处于未提交状态（`git status`），审计对象为工作区当前内容而非 `47a3806` 提交快照。
4. **`src-tauri/src/processing/` 为未跟踪目录**，`schema.rs:44` 的 `event_seq` 迁移已在工作区生效但未入库，未来提交时需确认该列与 `queue.rs:43-45` 的读写一致（已核对：一致）。
