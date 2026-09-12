# A12 对抗审计：第 20/21/22/23/25 章 + 附录 A/B/C

审计日期：2026-09-12
审计对象：`Plan With Files/IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md`
覆盖行段：§20（3670–3732）、§21（3735–3818）、§22（3821–3853）、§23（3856–3895）、§25（3954–3983）、附录 A（4235–4254）、附录 B（4256–4290）、附录 C（4292–4307）
仓库状态：`HEAD = 47a3806` + 大量未提交改动（`src-tauri/src/processing/`、`src-tauri/src/recognition/`、`src/api/processingClient.ts`、`src/exam-canvas/structureActions.ts`、`src/features/editor/SelectionInspector.tsx` 等为 untracked）
审计立场：默认每条论断错误/过时/未完成，用代码证据证伪
审计方式：只读（Read/Grep/Glob、`git log/show`、`wc`）；未运行 `cargo build`/`cargo test`/`npm run build`；未修改任何产品代码

---

## 章节范围

| 章节 | 计划清单项数 | 本报告核验方式 |
|---|---|---|
| §20.1 直接从普通产品面删除 | 12（任务书称 11，实际 12 行） | 逐项核对当前导航/路由/渲染可达性 |
| §20.2 迁移完成后删除的代码 | 12 | 逐文件 `ls`/行数 |
| §20.3 必须保留但隐藏的工程机制 | 9 | 逐机制 Grep 定义点 |
| §20.4 一版兼容期后删除 | 6 | 逐项 Grep 生产调用 |
| §21 PR-01~PR-16 | 16 | `git log` + 文件存在性 + 代码路径 |
| §22 三人分工 | 1 条硬约束 | 三个文件实际行数/拆分状态 |
| §23.1/23.2/23.3 | 10 + 7×3 + 6 | 指标名/灰度开关 Grep |
| §25.1/25.2/25.3 | 8 + 3 + 6 | 逐项代码证据 |
| 附录 A | 16 行（任务书称 15，实际 16 行） | 逐路径 Glob |
| 附录 B | 1 技能目录 + 7 contracts + 6 前端目录/文件组 + 6 后端模块 + 4 脚本 | 逐路径存在性 |
| 附录 C | 12 条决策 | 逐条实现状态 |

---

## 删除与保留清单核对表

### §20.1 直接从普通产品面删除（12 项）

判定口径：「产品面」= 普通用户可达的导航/路由/界面。代码文件是否仍在属 §20.2 范畴。

| # | 计划项 | 计划状态 | 当前状态 | 证据 | 判定 |
|---|---|---|---|---|---|
| 1 | Dashboard 入口 | 删除 | 已删除 | `src/components/AppShell.tsx:9-12` 导航仅「题库/设置」；`src/app/App.tsx:26-33` 无 Dashboard 分支 | 符合 |
| 2 | 导题任务入口 | 删除 | 已删除 | 同上；`router.ts:22-24` 的 `jobs` 仅在 `#/legacy/` 且被重定向 | 符合 |
| 3 | 独立新建导题页面 | 删除 | 已删除 | `router.ts:52-53` `#/jobs/new` → `/library?import=1`；`ImportDrawer` 为唯一入口 | 符合 |
| 4 | 源文档确认步骤页 | 删除 | 已删除 | `router.ts:47-49` `#/legacy/document/*` → `/library` 或 `/items/*`；`App.tsx` 不渲染该 legacy 页 | 符合 |
| 5 | 拆分步骤页 | 删除 | 已删除 | 无对应路由；`router.ts:1-5` 注释明确流水线阶段不再是路由 | 符合 |
| 6 | LLM review 步骤页 | 删除 | 已删除 | 同上；`llm-review` 仅出现在 `router.ts:54` 重定向注释 | 符合 |
| 7 | 独立 Preview 页 | 删除 | 已删除 | `UnifiedPreview.tsx` 无 importer；`App.tsx` 不渲染 | 符合（产品面）/ 代码仍在（§20.2） |
| 8 | 独立结构化编辑器入口 | 删除 | 已删除 | `StructuredAuthoringEditorV2.tsx` 无 importer | 符合（产品面）/ 代码仍在 |
| 9 | 独立 NAS 导出页 | 删除 | 已删除 | `router.ts:58` `#/export` → `/library?publish=1` | 符合 |
| 10 | 步骤条 | 删除 | 已删除 | `AppShell.tsx:7` 注释「已删除…流水线 stepper」；Grep 新表面无 stepper | 符合 |
| 11 | 每个题组的常驻置信度数字 | 删除 | 已删除（UI） | `grep -rn confidence src/features src/exam-canvas` 仅命中 `structureActions.ts:100` 数据字段，无渲染 | 符合 |
| 12 | 普通用户可见 hash/schema/manifest/revision | 删除 | 已删除（UI） | `LibraryItemRow.tsx:3` 注释；新表面无渲染；`describeLoadError` 仅开发者模式显示原始码 | 符合 |

**§20.1 小结：12/12 在产品面符合。** 但需注意：1/2/3/4/5/6/7/8/9 的「删除」目前靠「页面代码仍在、只是无人渲染」实现，属「产品面隐藏」而非「代码删除」。

### §20.2 迁移完成后删除的代码（12 项）

| # | 计划项 | 计划状态 | 当前状态 | 证据 | 判定 |
|---|---|---|---|---|---|
| 1 | `src/pages/Dashboard.tsx` | 待删 | 仍在（96 行） | `ls -la src/pages/` | 未完成 |
| 2 | `src/pages/JobList.tsx` | 待删 | 仍在（39 行） | 同上 | 未完成 |
| 3 | `src/pages/ImportWizard.tsx` | 待删 | 仍在（325 行） | 同上 | 未完成 |
| 4 | `src/pages/DocumentReview.tsx` | 待删 | 仍在（143 行） | 同上 | 未完成 |
| 5 | `src/pages/UnifiedPreview.tsx` | 待删 | 仍在（1419 行） | 同上 | 未完成 |
| 6 | `src/pages/StructuredAuthoringEditorV2.tsx` | 待删 | 仍在（730 行） | 同上 | 未完成 |
| 7 | `src/pages/LibraryExamDetail.tsx` | 待删 | 仍在（207 行） | 同上 | 未完成 |
| 8 | `src/pages/ExportPage.tsx` | 待删 | 仍在（564 行） | 同上 | 未完成 |
| 9 | `src/editor/authoringTiptap.tsx` | 待删（若无调用） | 仍在（468 行） | `ls src/editor/`；无 importer | 未完成 |
| 10 | `src/services/devFallbackBackend.ts`（生产路径） | 待删 | 源文件仍在（3888 行）；生产 bundle 已移除 | `tauriCommands.ts:66-74` `import.meta.env.DEV` 短路；无 dist 产物可验 | 部分完成 |
| 11 | legacy V1 page routes | 待删 | 仍在 | `router.ts:11-24` `LEGACY_PAGES` 9 项 | 未完成 |
| 12 | browser localStorage cloud queue | 待删 | 仍在（不可达） | `UnifiedPreview.tsx:174` `pendingCloudReviewKey`；`devFallbackBackend.ts:208` localStorage store | 未完成 |

**§20.2 小结：0/12 已删除；1 项部分（devFallback 仅退出生产 bundle）。** 与计划一致（删除被指派给 P10/PR-16，当前 pending），但构成最大体量的待办：合计约 8144 行前端代码。
附带证据：`src/app/legacyRoutes.tsx`（89 行）已成孤儿（全仓无 importer），`src/pages/LibraryPage.tsx`（176 行）亦无 importer——两个「死入口」仍在 `tsc` 参与编译。

### §20.3 必须保留但隐藏的工程机制（9 项）

| # | 机制 | 当前状态 | 证据 | 判定 |
|---|---|---|---|---|
| 1 | SQLite transaction | 存在 | `library/repository.rs:245` `apply_editor_commands_tx` | 保留 |
| 2 | atomic file write | 存在 | `artifact_store.rs:504` `write_bytes_atomic`、`:600` `fs::rename`；`nas_package_v2.rs:328` 原子清单替换 | 保留 |
| 3 | safe relative path | 存在 | `artifact_store.rs:445` `safe_relative_path`；`reading_runtime_v2.rs:154` `reject_unsafe_relative_path` | 保留 |
| 4 | asset reference closure | 存在 | `ielts_grammar/quality.rs:401` `validate_identifier_and_reference_closure`（`:52` 调用） | 保留 |
| 5 | minimal content hash for asset dedupe | **部分** | 有 sha256 计算（`artifact_store.rs:539/572`），未发现按内容哈希去重资产的实现（仅字符串 `dedup()`） | 部分 |
| 6 | NAS staging and final commit | 存在 | `nas_package_v2.rs:231-234` staging 路径、`:287` `.batch-staging-{batch_id}`、`:313` `stage_package_files` | 保留 |
| 7 | schema validation | 存在 | `recognition/local/mod.rs:202` producer schema gate（`is_supported_schema_version`）；`validator.rs` | 保留 |
| 8 | runtime compiler validation | 存在 | `runtime_compiler.rs:11-23` `ExamCompiler::validate` → `validate_reading_source_v2` | 保留 |
| 9 | bounded crash recovery | 存在 | `processing/queue.rs:232` `recover_on_startup`；`scheduler.rs:30` `MAX_AUTO_RECOVERY = 3` | 保留 |

**§20.3 小结：8/9 保留，1 项（内容哈希去重）未证实。**

### §20.4 一版兼容期后删除（6 项）

| # | 计划项 | 当前状态 | 证据 | 判定 |
|---|---|---|---|---|
| 1 | DocumentIRV1 新写入 | 仍在写入 | `parser.rs:349/594/1746` 生产构造 `DocumentIRV1`；`authoring_commands.rs:232/267/307` 写 `document-ir.json` | 未删除 |
| 2 | V1 authoring pipeline 新任务入口 | 仍是主入口 | `auto_pipeline.rs:1692` `run_auto_pipeline_core` → `make_dynamic_split_candidates`；`processing/scheduler.rs` 直接调用 | 未删除 |
| 3 | legacy exams 双写 | 仍活跃 | `job_store.rs:33-40` `save_job` 双写 DB；`library_commands.rs:89/138/156` 双写钩子；`db.rs:83` 旧 `exams` 表保留 | 未删除 |
| 4 | job.json 权威状态 | 仍权威 | `job_store.rs:30` `load_job` 读 `job.json`；被 `auto_pipeline`/`cleanup`/`export_*` 依赖 | 未删除 |
| 5 | V1 preview HTML 主路径 | 仍在 | `runtime_validation.rs:27` `resolve_preview_runtime_html_path`、`:271` `runtimeHtml` | 未删除 |
| 6 | V1 Reading JS exporter | 仍在 | `export_pack.rs:197` `export_reading_js_core`；`lib.rs:1289` 命令注册、`:1579` handler 列表 | 未删除 |

**§20.4 小结：0/6 已删除。** 六项兼容层全部仍在生产路径上活跃，M7/PR-16 未启动。

### §25.1 本轮必须完成（8 项）

| # | 计划项 | 当前状态 | 证据 | 判定 |
|---|---|---|---|---|
| 1 | Reading PDF/DOCX 导入 | 完成 | `useImportFiles.ts:3,19` → `processingClient.importFiles` → `lib.rs:977` `import_files` | 完成 |
| 2 | 本地 DocumentIRV2 直接识别 | **未完成** | 主链仍 V1（`auto_pipeline.rs:1692`）；`recognition/local/mod.rs` 仅产出 `QuestionLayoutGraphV1` 中间层，未接入主链 | 未完成 |
| 3 | 云端完整识别和校对 | **未完成** | 仅 `llm_suggestions.rs:235` `CloudReadingOutlineV1`（对照提纲）；无 `CloudRecognitionCandidateV1`、无 reconcile 模块 | 未完成 |
| 4 | Reading Canonical DS | 完成 | `library/schema.rs:50` `library_items_v2.canonical_ds_json` 唯一权威稿 | 完成 |
| 5 | 题库/批量任务 | 完成 | `processing/queue.rs` 阶段机 + `libraryStore.ts:106` 事件订阅 | 完成 |
| 6 | Reading WYSIWYG 工作区 | 部分 | `ExamWorkspacePage.tsx` + `ExamCanvas.tsx` + `structureActions.ts` 已接入结构操作；renderer 拆分仅 2/10（`renderers/` 仅 `MatchingMatrix.tsx`，`model/` 为空） | 部分 |
| 7 | Reading V2 NAS 发布 | 部分 | `publishClient.ts` + `nas_package_v2.rs` 整批原子提交；NAS Electron 实测未做（progress.md 自述） | 部分 |
| 8 | 前端简化和溢出治理 | 完成 | 三表面 + `scripts/ui/layout-matrix.mjs` | 完成 |

**§25.1 小结：4/8 完成，3 部分，2 未完成（本地/云端识别）。**

### §25.2 本轮不扩展但必须保持兼容（3 项）

| # | 计划项 | 当前状态 | 证据 | 判定 |
|---|---|---|---|---|
| 1 | Writing 数据和已有发布能力 | 保留 | `src/pages/WritingStudio.tsx`（`App.tsx:6,31-33` 仍渲染）、`export_writing_library.rs`、`writing_store.rs` | 兼容 |
| 2 | Listening 已有 contract/runtime | 保留 | `src/services/listeningRuntimeV1.ts`、`listeningPlaybackControllerV1.ts`、`schema/listening_runtime_v1.rs`、`contracts/listening-*.schema.json` | 兼容 |
| 3 | 旧 V1 题库只读迁移 | 存在 | `library/migration.rs`（`revision → shadow`，幂等）；未发现删除用户数据的逻辑 | 兼容（静态） |

「不能删除用户已有数据」：未发现迁移/清理路径删除既有题稿；`cleanup.rs` 只删过程产物。**未发现违反**（静态证据，未跑真实迁移）。

### §25.3 明确不做（6 项）

| # | 计划项 | 当前状态 | 证据 | 判定 |
|---|---|---|---|---|
| 1 | 不让模型直接生成或执行 JS | 未见违反 | `grep -rn "eval(\|new Function(\|execScript\|run_js"` 于 `src-tauri/src` 零命中 | 未违反（静态） |
| 2 | 不让云端结果无审核覆盖用户编辑 | 机制存在，场景未实现 | `authoring_v2_commands.rs:2091` `mark_user_edited`；但无 reconcile/cloud 覆盖路径可证伪 | 未违反（不可证实） |
| 3 | 不在本轮实现多人实时协同 | 未违反 | 全仓无协同/OT/CRDT 代码 | 未违反 |
| 4 | 不追求完美反编译流程图，source-faithful hybrid | 未违反 | `structureActions.ts` + 原文件抽屉；无流程图反编译 | 未违反 |
| 5 | 不把全部研发诊断移入普通用户界面 | 部分符合 | `ExamWorkspacePage.tsx:28-37` `describeLoadError` 已分层；`appSettings.ts` 仍有开发者模式开关 | 符合 |
| 6 | 不通过提高最小窗口宽度掩盖 CSS 溢出 | 未违反 | `tauri.conf.json:18` `minWidth: 1100`（与 task_plan「保持不变」一致）；`layout-matrix.mjs` 在 1100×760 断言 | 未违反 |

---

## PR 完成度表

证据来源：`git log --oneline`（仅 3 个计划相关提交：`5daa04f` 基线冻结、`401ca76` M0、`47a3806` M1；其余实现均为未提交工作区改动）+ 文件存在性 + 代码路径。
判定口径：`已落地` = 目标能力在当前工作区可用；`部分` = 骨架/子集存在；`未落地` = 无对应代码。

| PR | 目标 | 证据 | 判定 |
|---|---|---|---|
| PR-01 | 产品基线与真实 Tauri E2E；**只增测试和基线，不改行为** | `5daa04f` 新增 `product_chain.rs`/`test_support.rs`/`verify-product-baseline.mjs`/`layout-matrix.mjs`；`401ca76` 新增 `tauri-import-edit-publish.mjs`、`nas-student-contract.mjs` | **已落地但违反「不改行为」**（见 A12-F01） |
| PR-02 | 三路由 + 极简 AppShell + overflow hotfix | `router.ts`、`App.tsx`、`AppShell.tsx`、`src/styles/*` | 已落地 |
| PR-03 | LibraryPage + ImportDrawer 统一入口 | `src/features/library/*`（8 文件）、`src/features/import/*`（2 文件） | 已落地 |
| PR-04 | LibraryRepositoryV2 + ProcessingJob schema | `library/schema.rs:50` `library_items_v2` 等 5 表；`library/migration.rs` | 已落地（`47a3806`） |
| PR-05 | Workspace API + ExamWorkspacePage 骨架 | `workspaceClient.ts`、`ExamWorkspacePage.tsx`、`library/commands.rs:62` | 已落地 |
| PR-06 | ExamCanvas 稳定文本编辑与 EditorCommand | `exam-canvas/editorCommands.ts`、`editors/InlineTextEditor.tsx`、`useCanonicalEditor.ts` | 已落地 |
| PR-07 | Choice/Matching/Completion/Table/Visual renderers | `renderers/` 仅 `MatchingMatrix.tsx`；`model/` 空；其余内联于 `ExamCanvas.tsx` | **部分**（2/10，与 task_plan 自述一致） |
| PR-08 | Durable Processing Queue | `processing/{queue,scheduler,commands,mod}.rs`（untracked）；`lib.rs:1511-1515` 启动；`libraryStore.ts:106` 事件订阅；无 2s 轮询 | 已落地（未提交） |
| PR-09 | DocumentIRV2 Direct Local Recognizer 基础题型 | `recognition/local/mod.rs` + `question_blocks.rs` 仅 `QuestionLayoutGraphV1` 中间层；**未接入主链**（`scheduler` 仍调 `run_auto_pipeline_core`） | **部分/未落地** |
| PR-10 | Physical Table + Hybrid Visual | 无独立实现；`recognition/local` 无 table/visual 专属逻辑 | 未落地 |
| PR-11 | Versioned Cloud Skill + Full Candidate | 无 `CloudRecognitionCandidateV1`；无 skill bundle；仅 `CloudReadingOutlineV1` | 未落地 |
| PR-12 | Cloud JSON Repair/Salvage | 无 repair/salvage 模块 | 未落地 |
| PR-13 | Reconciliation + Localized Proposal | 无 `recognition/reconcile/*`、无 proposal 契约 | 未落地 |
| PR-14 | Batch Publish + Typed Preflight | `publishClient.ts`、`nas_package_v2.rs` 整批原子提交 + 故障注入测试；`check_publish_preflight` | 部分（NAS 实测未做） |
| PR-15 | Cleanup/Settings/Legacy Dual-write Removal | `cleanup.rs` 无 TTL；legacy 双写仍活跃（`job_store.rs:33-40`） | 未落地 |
| PR-16 | 删除退休页面和 dead code | 12 项待删代码全部仍在（§20.2） | 未落地 |

**PR 依赖关系自洽性核对**

计划依赖图（`§21` 3808-3815）画成单链：
```
PR-01 -> PR-02 -> PR-03 -> PR-04 -> ... -> PR-16
```
但同段（3817）又声明「PR-08 和 PR-09 可在 PR-06/07 期间由不同工程师并行」「PR-13 必须等 canonical workspace 和 cloud candidate 都稳定」。单链图与并行声明**互相矛盾**；实际可交付顺序（M2 队列早于 M4/M5 识别）也证明该图不是线性依赖。判定：依赖图**不自洽**（A12-F06）。

**PR-16 gate 核对**：计划要求「只有在调用量和迁移 gate 证明安全后执行」。全仓未发现任何「调用量统计」或「迁移 gate」实现（无 telemetry、无调用计数、无 gate 脚本）；`scripts/verify-*` 中亦无 PR-16 对应 gate。判定：**gate 零实现**（A12-F04）。

---

## 附录路径有效性核对

### 附录 A：审计证据索引（16 行）

| 行 | 结论 | 计划源码 | 存在性 |
|---|---|---|---|
| 1 | 路由和页面过多 | `src/app/App.tsx`、`src/app/router.ts`、`src/components/AppShell.tsx` | 全部存在 |
| 2 | 固定列宽和 overflow 风险 | `src/styles.css`、`src-tauri/tauri.conf.json` | 全部存在 |
| 3 | 导入 localOnly + browser cloud queue | `src/pages/ImportWizard.tsx`、`src/pages/UnifiedPreview.tsx` | 全部存在 |
| 4 | 作者端多套编辑/渲染 | `ExamCanvasV2.tsx`、`authoringTiptap.tsx`、`UnifiedPreview.tsx`、`LibraryExamDetail.tsx` | **`ExamCanvasV2.tsx` 全仓不存在** |
| 5 | current ExamCanvas 可复用 | `src/components/ExamCanvasV2.tsx` | **不存在**（`src/components/` 仅 `AppShell.tsx`、`StatusPill.tsx`） |
| 6 | V2 runtime 同源投影 | `src/services/runtimeViewModelV2.ts`、`src-tauri/src/reading_source_v2.rs` | 全部存在 |
| 7 | V1-first 识别残留 | `parser.rs`、`authoring_pipeline.rs`、`ielts_grammar/mod.rs` | 全部存在 |
| 8 | 题号/题干/选项 line-first | `ielts_grammar/anchors.rs`、`prompt_assembler.rs`、`option_run.rs` | 全部存在 |
| 9 | Physical V2 基础可复用 | `pdf_facts_shadow.rs`、`pdf_ingest/*`、`schema/document_ir_v2.rs` | 全部存在 |
| 10 | Cloud 只是 outline | `llm_gateway.rs` | 存在（注：`CloudReadingOutlineV1` 实际出现在 `llm_suggestions.rs:235`，`llm_gateway.rs` 未命中该字符串——**行内指向偏差**） |
| 11 | Cloud worker/队列复杂 | `auto_pipeline.rs`、`UnifiedPreview.tsx` | 全部存在 |
| 12 | Provider UI/adapter 不一致 | `Settings.tsx`、`llm_commands.rs`、`llm_gateway.rs` | 全部存在 |
| 13 | 多事实源和双写 | `job_store.rs`、`db.rs`、`library_commands.rs`、`artifact_store.rs` | 全部存在 |
| 14 | 过程文件保留过多 | `cleanup.rs`、`artifact_store.rs` | 全部存在 |
| 15 | 发布门禁过度依赖历史痕迹 | `authoring_v2_commands.rs` | 存在 |
| 16 | UI E2E 使用 dev fallback | `sidecars/ui-flow-e2e/ui-flow-e2e.mjs`、`devFallbackBackend.ts` | 全部存在 |

**失效路径：2 行（第 4、5 行，均指向 `ExamCanvasV2`）。** 计划正文 §16.7（第 1574、2609 行）已声明 `ExamCanvasV2.tsx` 重命名为 `src/exam-canvas/ExamCanvas.tsx`，附录 A 未同步（A12-F02）。
**指向偏差：1 行（第 10 行）** `CloudReadingOutlineV1` 实际在 `llm_suggestions.rs`。
其余 13 行路径有效。

### 附录 B：交付物清单

| 交付物 | 计划状态 | 当前状态 | 证据 |
|---|---|---|---|
| `recognition/skills/ielts-reading-v1/*` | 应形成 | **不存在** | 仓库根无 `recognition/` 目录 |
| `contracts/processing-item-v1.schema.json` | 应形成 | **不存在** | `ls contracts/` 无 |
| `contracts/recognition-candidate-v1.schema.json` | 应形成 | **不存在** | 同上 |
| `contracts/cloud-recognition-candidate-v1.schema.json` | 应形成 | **不存在** | 同上 |
| `contracts/reconciliation-proposal-v1.schema.json` | 应形成 | **不存在** | 同上 |
| `contracts/actionable-issue-v1.schema.json` | 应形成 | **不存在** | 同上 |
| `contracts/editor-command-v1.schema.json` | 应形成 | **不存在** | 同上 |
| `contracts/publish-check-result-v1.schema.json` | 应形成 | **不存在** | 同上 |
| `src/features/library/*` | 应形成 | 存在（8 文件） | `find src/features` |
| `src/features/import/*` | 应形成 | 存在（2 文件） | 同上 |
| `src/features/editor/*` | 应形成 | 存在（4 文件） | 同上 |
| `src/features/settings/*` | 应形成 | 存在（2 文件） | 同上 |
| `src/exam-canvas/*` | 应形成 | 存在（5 文件，拆分未完成） | 同上 |
| `src/api/{transport,libraryClient,processingClient,workspaceClient,publishClient,settingsClient}.ts` | 应形成 | **3/6 存在**；`transport.ts`/`libraryClient.ts`/`settingsClient.ts` **缺失** | `ls src/api/` |
| `src/styles/*` | 应形成 | 存在（10 文件） | `ls src/styles/` |
| `src-tauri/src/processing/*` | 应形成 | 存在（4 文件，untracked） | `ls` |
| `src-tauri/src/recognition/local/*` | 应形成 | 存在（2 文件） | `find` |
| `src-tauri/src/recognition/cloud/*` | 应形成 | **不存在** | `find` 无 |
| `src-tauri/src/recognition/reconcile/*` | 应形成 | **不存在** | 同上 |
| `src-tauri/src/library/*` | 应形成 | 存在（5 文件） | `ls` |
| `src-tauri/src/editor/*` | 应形成 | **不存在** | 目录不存在 |
| `src-tauri/src/publish/*` | 应形成 | **不存在** | 目录不存在 |
| `scripts/e2e/tauri-library-import.mjs` | 应形成 | **不存在**（等价物 `tauri-import-edit-publish.mjs` 存在） | `ls scripts/e2e/` |
| `scripts/e2e/tauri-workspace-edit.mjs` | 应形成 | **不存在** | 同上 |
| `scripts/e2e/tauri-publish.mjs` | 应形成 | **不存在** | 同上 |
| `scripts/ui/layout-matrix.mjs` | 应形成 | 存在 | `ls scripts/ui/` |

**附录 B 小结：已产出 8 组前端/后端目录 + layout-matrix；未产出全部 7 个 contracts、recognition 技能包、recognition/cloud、reconcile、src-tauri/editor、src-tauri/publish、3 个命名 e2e 脚本、3 个 api 客户端。**

### 附录 C：最终决策摘要（12 条）

| # | 决策 | 当前实现状态 | 证据 | 判定 |
|---|---|---|---|---|
| 1 | 产品入口：Library | 已实现 | `App.tsx:19,26`；`AppShell.tsx:9-12` | 符合 |
| 2 | 编辑方式：最终考试界面内直接编辑 | 已实现 | `ExamWorkspacePage.tsx` + `ExamCanvas.tsx`，无 edit/preview 开关 | 符合 |
| 3 | 权威数据：IeltsAuthoringIRV2-based Canonical DS | 已实现 | `library/schema.rs:50` `canonical_ds_json` | 符合 |
| 4 | 本地识别：DocumentIRV2 direct geometry-first | **未实现** | 主链仍 `make_dynamic_split_candidates`（V1）；`DocumentIRV2` 仅为 `build_authoring_v2_shadow` 辅助输入 | 不符 |
| 5 | 云端识别：versioned full candidate | **未实现** | 仅 `CloudReadingOutlineV1` 对照提纲（`llm_suggestions.rs:235`） | 不符 |
| 6 | 云端校对：diff-triggered proposal only | **未实现** | 无 reconcile 模块、无 proposal 契约 | 不符 |
| 7 | 队列：Rust + SQLite durable scheduler | 已实现 | `processing/queue.rs`、`scheduler.rs`（untracked） | 符合 |
| 8 | 预览：Canonical DS direct render | 部分 | `ExamCanvas` 直渲 canonical DS；但 V1 preview HTML 路径仍在（`runtime_validation.rs:27`） | 部分 |
| 9 | 发布：Library/Workspace action -> ReadingExamSourceV2 -> NAS | 部分 | `publishClient.ts` + `nas_package_v2.rs` 原子批量；NAS 实测未做 | 部分 |
| 10 | 过程文件：transient + TTL cleanup | **不符** | `cleanup.rs` 无 TTL/时间保留策略（仅 diagnostics 开关），无过期清理 | 不符 |
| 11 | 用户可见安全/诊断：最小化 | 已实现 | `ExamWorkspacePage.tsx:28-37` 文案分层；`LibraryItemRow.tsx:3` | 符合 |
| 12 | 底层原子性/路径/资源闭包：保留但隐藏 | 已实现 | 见 §20.3 第 1/2/3/4/6/7/8/9 项 | 符合 |

**附录 C 小结：7 符合，2 部分，3 不符（第 4、5、6 条——本地识别、云端候选、云端校对）。**

---

## 发现清单

### A12-F01 [P1] PR-01 声称「只增测试和基线，不改行为」，但基线冻结提交改了两处产品判定行为

**结论**：`§21` PR-01 明确写「只增测试和基线，不改行为」。作为 PR-01 载体的 `5daa04f`（"chore: freeze baseline"）修改了两处会影响出题/发布结果的产品逻辑，该声明不成立。
**证据**：
- `git show 5daa04f -- src-tauri/src/authoring_v2_commands.rs`：`mark_user_audit` 由硬编码 `audit.humanVerified = false` 改为继承既有值（`already_verified`）。当前代码 `authoring_v2_commands.rs:1046-1051` 仍为该逻辑，注释自述「That inverted the intent… the FIRST edit permanently blocked publishing」。
- `git show 5daa04f -- src-tauri/src/ielts_grammar/quality.rs`：`evaluate_quality` 的 readiness 条件由 `!issues.is_empty()` 改为 `unresolved_blocking_issues > 0`。当前代码 `quality.rs:197-213`。
**影响**：PR-01 的「不改行为」是后续基线漂移判定的前提。行为变更被混入基线冻结提交后，`verify-product-baseline.mjs` 的漂移检测与「基线可信」结论失去区分力；后续审计无法判断某行为差异来自「计划实施」还是「未记录的基线修复」。
**建议**：把该提交的产品行为变更拆分/补记为独立修复项并更新 task_plan 的基线口径；在 PR-01 描述中删除「不改行为」或改为「除已记录的发布门修复外不改行为」。

### A12-F02 [P1] 附录 A 的「current ExamCanvas 可复用」与「作者端多套编辑/渲染」两行源码路径已失效

**结论**：附录 A 第 4、5 行引用 `ExamCanvasV2.tsx` / `src/components/ExamCanvasV2.tsx`，两者在全仓均不存在；计划正文 §16.7 已把该文件重命名为 `src/exam-canvas/ExamCanvas.tsx`。附录 A 未同步。
**证据**：`find . -name "ExamCanvasV2*"` 零命中；`ls src/components/` 仅 `AppShell.tsx`、`StatusPill.tsx`；正文第 1574 行「`ExamCanvasV2.tsx` 重命名为 `src/exam-canvas/ExamCanvas.tsx`」、第 2604/2609 行。
**影响**：附录 A 是「按当前提交」的证据索引，失效路径使审计结论无法回溯；第 5 行结论「current ExamCanvas 可复用」实际应指向 `src/exam-canvas/ExamCanvas.tsx`。
**建议**：更新附录 A 两行路径；同时把第 10 行 `llm_gateway.rs` 更正为 `llm_suggestions.rs`（`CloudReadingOutlineV1` 实际位置）。

### A12-F03 [P1] 附录 B 交付物清单绝大部分未产出：7/7 contracts 缺失、recognition 技能包缺失、多个后端模块目录不存在

**结论**：附录 B 以「完成本计划后，仓库应至少新增或形成」列出交付物。当前 7 个 contracts 全部不存在，`recognition/skills/ielts-reading-v1/*` 不存在（无 `recognition/` 目录），`src-tauri/src/recognition/cloud/*`、`reconcile/*`、`src-tauri/src/editor/*`、`src-tauri/src/publish/*` 均不存在，`src/api/{transport,libraryClient,settingsClient}.ts` 缺失，3 个命名 e2e 脚本缺失。
**证据**：`ls contracts/`（无 `*-v1.schema.json`）；`find recognition` 无目录；`find src-tauri/src/recognition` 仅 `local/`；`ls src-tauri/src/editor src-tauri/src/publish` 报错；`ls src/api/`；`ls scripts/e2e/`。
**影响**：附录 B 是完成度验收的对照物。当前交付物缺口集中在 M4/M5（识别）与跨仓契约，与 §25.1 第 2/3 项未完成一致；若以附录 B 作为交付门，计划远未达门。
**建议**：把附录 B 拆为「已产出 / 未产出」两栏并标注归属 PR，避免清单被误读为已完成。

### A12-F04 [P1] 第 23 章监控指标与发布门零实现：10 项产品指标无度量、7 项质量目标无度量脚本、6 步灰度无开关

**结论**：§23 三节均为「应有」，代码中无任何对应实现。
**证据**：
- §23.1：`grep -rn -i "metric|telemetry"` 于 `src/`、`src-tauri/src` 仅命中 legacy `Dashboard.tsx` 的 UI 卡片与类型字段，无 `Import-to-first-draft time`、`Publish success rate`、`Crash/restart recovery rate` 等指标采集。
- §23.2：`grep -rn -i "recall|parity|merge.rate|prompt.missing|option.missing"` 于 `scripts/` 无度量脚本；仅 `verify-phase4-eight-pdf-acceptance.mjs` 等历史 gate。
- §23.3：`src/config/featureFlags.ts` 定义 `Phase0/1/5FeatureFlags`（`documentIrV2Shadow`、`authoringV2Shadow` 等），但 `grep -rn "resolvePhase*FeatureFlags|DEFAULT_PHASE"` 排除自身后**零调用者**——整文件是死代码；且不存在「10% 新导入 direct V2 / 50% / 100%」的灰度维度。
- PR-16 的「调用量和迁移 gate」同样无实现（无调用计数、无 gate 脚本）。
**影响**：§23 的发布门与灰度计划不可执行；无法按计划判定「Alpha/Beta/正式发布」是否达标，也无法执行「关闭 V1 新写入」的灰度收敛。
**建议**：明确 §23 为「尚未实现的目标值」；若本轮需要门，至少落 `layout-matrix`（已有）与一个可自动化的质量指标采集脚本；把 featureFlags 死代码删除或接入。

### A12-F05 [P1] §23.3「不允许同一新题同时由 V1/V2 双写修改」无代码强制，且当前新题确实同时经过 V1 与 V2 写入

**结论**：计划把该约束作为灰度期的硬规则。代码中不存在任何阻止同一新题被 V1 与 V2 同时写入的 gate/flag。
**证据**：
- 新导入主链仍是 V1：`processing/scheduler.rs` 调 `run_auto_pipeline_core`；`auto_pipeline.rs:1692` `make_dynamic_split_candidates` 产出 V1 IR，再编译 V2 shadow。
- 同时 `job_store.rs:33-40` `save_job` 双写 legacy `exams` DB（`db.rs:83`），`library_commands.rs:89/138/156` 双写钩子仍在；V2 侧写 `library_items_v2`。
- 无灰度开关（见 A12-F04），无「同一 item 禁止双写」的断言或校验。
**影响**：与 task_plan 自述的 M4「主链仍走 make_dynamic_split_candidates → V1 IR → 编译 V2 authoring shadow」一致，即当前正是计划想禁止的状态；审计报告 F02（迁移读到 shadow 而非最新 revision）正是双写/双事实源的直接后果类型。
**建议**：在灰度前先落一个显式的「单一写入方」约束（例如新导入走 V1 时禁止 V2 canonical DS 被外部写入，或反之），并在 §23.3 标注该约束的实现位置。

### A12-F06 [P2] §21 依赖关系图与实际可交付顺序不自洽

**结论**：§21 依赖图把 PR-01→PR-16 画成单链，同段文字却声明 PR-08/PR-09 可与 PR-06/07 并行、PR-13 需等 workspace 与 cloud candidate 稳定。单链图与并行声明矛盾。
**证据**：计划第 3808-3815 行依赖图；第 3817 行并行说明；实际实现顺序（M2 队列早于 M4/M5 识别，`processing/` 已落地而 `recognition/cloud` 不存在）证明并非线性。
**影响**：按图排期会把 PR-08 错误地排在 PR-07 之后，与 §22 的「B 做 PR-04/08/09」并行分工冲突。
**建议**：把依赖图改为带并行分支的 DAG，并与 §22 分工对齐。

### A12-F07 [P2] §22「先拆文件再并行」对 `lib.rs` 未成立：`lib.rs` 仍为 11151 行单文件

**结论**：§22 共同责任要求「不在同一时间多人修改 `lib.rs`/`styles.css`/`App.tsx`；先拆文件再并行」。其中 `styles.css`（13 行，纯 `@import`）与 `App.tsx`（36 行）确已拆分，但 `lib.rs` 仍是 11151 行单文件，未拆分。计划自身 §18 P8-T04 也要求「完成 `auto_pipeline/authoring_pipeline/quality/lib.rs` 迁移」。
**证据**：`wc -l src-tauri/src/lib.rs` = 11151；`wc -l src/styles.css` = 13（仅 import 10 个分层文件）；`wc -l src/app/App.tsx` = 36；`src-tauri/src/authoring_pipeline.rs` = 547 KB、`auto_pipeline.rs` = 114 KB。
**影响**：§22 对 `lib.rs` 的并行约束在现实中不可执行；三人分工中 A/B/C 都要改 `lib.rs`（命令注册、processing、publish 命令），必然串行或冲突。
**建议**：把 `lib.rs` 拆分（如按命令域拆 `commands/{library,processing,publish,editor}.rs`）列为 PR-04/PR-08 的前置任务，或把 §22 约束改为「先按命令域拆 `lib.rs` 再并行」。

### A12-F08 [P2] §20.3 第 5 项「minimal content hash for asset dedupe」只有哈希计算，无资产去重

**结论**：计划要求保留「按最小内容哈希做资产去重」的机制。代码中能找到 sha256 计算与记录，但未发现按内容哈希去重资产的逻辑（现有 `dedup()` 都是字符串向量去重）。
**证据**：`artifact_store.rs:539/572` `hash_bytes` 计算 sha256；`grep -rn -i "dedupe|dedup|by_sha"` 仅命中 `Vec::dedup`（如 `authoring_pipeline.rs:1934`、`parser.rs:292`），无「同哈希资产复用/跳过写入」。
**影响**：若计划依赖该机制控制过程文件体积（§附录 C 第 10 条 TTL cleanup 的配套），则该前提不成立。
**建议**：确认该机制是否已在别处实现；若未实现，在 §20.3 标注为「待建」而非「保留」。

### A12-F09 [P2] §附录 C 第 10 条「过程文件：transient + TTL cleanup」无 TTL 实现

**结论**：`cleanup.rs` 的清理由 diagnostics 设置驱动（保留全部过程产物），没有基于时间/TTL 的过期清理。
**证据**：`grep -rn -i "ttl|retention|expire|older_than|max_age" src-tauri/src/cleanup.rs` 仅命中两条「Developer diagnostics retention is enabled」消息；无时间比较或 TTL 常量。
**影响**：与附录 C 决策不符；过程文件体积随使用累积。
**建议**：标注为未实现，或在 P9/PR-15 落 TTL 策略。

### A12-F10 [P3] §20.2 相关死代码：`legacyRoutes.tsx` 与 `pages/LibraryPage.tsx` 已成孤儿但仍参与编译

**结论**：`src/app/legacyRoutes.tsx`（89 行）无任何 importer，`src/pages/LibraryPage.tsx`（176 行）无任何 importer，但仍在 `tsc` 范围内；`legacyRoutes.tsx` 一次 import 了 8 个待删页面，因此不能单独删除。
**证据**：`grep -rn "LegacyRoutes" src/` 仅命中定义自身；`grep -rn "pages/LibraryPage"` 零命中；`App.tsx` 不 import 二者。
**影响**：死代码增加维护面，且让「§20.2 待删」体量虚高（真实可达代码更少）。
**建议**：与 P10/PR-16 一并处理；在此之前可先从 `tsc` include 中排除或加注释标记。

### A12-F11 [P3] PR-16 的「调用量与迁移 gate」无任何实现证据

**结论**：计划要求 PR-16「只有在调用量和迁移 gate 证明安全后执行」。全仓无调用量统计、无迁移 gate 脚本/断言。
**证据**：`grep -rn -i "gate" scripts/` 仅命中历史 phase gate（`verify-phase2-shadow.mjs` 等），无 PR-16 专用 gate；`fixtures/product-baseline.json` 只记录 route/命令/表/flag 快照，不记录调用量。
**影响**：PR-16 缺少可执行的前置条件，删除退休页面的「安全性」无法客观判定。
**建议**：定义 gate 内容（如 legacy 路由访问计数、V1 写入计数为 0）并落脚本，或把该前置条件改为人工验收。

---

## 内部矛盾

| # | 矛盾双方 | 结论 |
|---|---|---|
| 1 | §20.2「迁移完成后删除」 vs §18 Phase 8（P8-T01~T03）删除时点 | **不矛盾**。§18 Phase 8 是 P4–P7 之后最后一阶段；§21 中 PR-15（数据面收敛）→ PR-16（删退休页面），与「迁移完成后删除」一致。但两者清单不完全相同：§18 P8-T01 额外含「Phase5 fixture route」，§20.2 额外含 `authoringTiptap.tsx`、`devFallbackBackend.ts`、legacy V1 page routes、localStorage cloud queue。**清单不一致（低危）**。 |
| 2 | §25.3「不通过提高 minWidth 掩盖溢出」 vs `tauri.conf.json` 实际 minWidth | **不矛盾**。`tauri.conf.json:18` `minWidth: 1100`，与 task_plan「`minWidth: 1100` 保持不变」及 `layout-matrix.mjs` 的 1100×760 断言一致；未发现为提高 minWidth 而掩盖溢出的改动。 |
| 3 | 附录 A 失效路径 vs 正文引用 | **矛盾**。附录 A 第 4/5 行仍用 `ExamCanvasV2.tsx` / `src/components/ExamCanvasV2.tsx`，正文 §16.7（第 1574、2609 行）已声明重命名为 `src/exam-canvas/ExamCanvas.tsx`。附录 A 未同步（A12-F02）。 |
| 4 | §21 依赖图（单链） vs 同段并行声明与 §22 分工 | **矛盾**（A12-F06）。 |
| 5 | §22「先拆文件再并行」 vs `lib.rs` 未拆分 | **矛盾**（A12-F07）。 |

---

## 证据层级与局限

| 结论/发现 | 证据层级 | 说明 |
|---|---|---|
| §20.1 全部 12 项「产品面已删除」 | `static` + `product`（路由/渲染可达性） | 通过 `App.tsx`/`router.ts`/`AppShell.tsx` 静态可达性判定；未在真实 Tauri 窗口逐一点击验证 |
| §20.2 12 项仍在 | `static` | 文件存在性与行数；未构建，无法验证是否进入 bundle（`devFallbackBackend` 的生产剔除为静态判定） |
| §20.3 8/9 机制存在 | `static` | 定义点 Grep；未运行以验证行为 |
| §20.4 6 项未删除 | `static` + `command`（命令注册） | `lib.rs:1289/1579` 命令注册为 `command` 层；其余为静态调用点 |
| PR-01 违反「不改行为」 | `command`（`git show` 差异） | 高置信 |
| PR-04/05/06/08 已落地 | `static` + `command` | `lib.rs` 命令注册 + 前端调用点 |
| PR-07/09/10/11/12/13/14/15/16 部分或未落地 | `static` | 目录/文件存在性 |
| §23 零实现 | `static`（Grep 零命中） | 零命中可证伪「有实现」，但不能排除以别名/动态名存在；已交叉检查 featureFlags 无调用者 |
| §25.3 第 1 项「不让模型执行 JS」 | `static`（Rust 零命中） | 未检查前端 `eval`（`grep` 仅限 `src-tauri/src`）；实际风险路径为 V1 exporter 生成 JS，属数据编译非模型执行 |
| §25.2「未删除用户数据」 | `static` | 未跑真实迁移，无法证明迁移在真实数据上无损 |
| 附录 A/B/C 路径 | `static` | 逐个 Glob/`ls`；`contracts/` 内 11 个既有 schema 已列，7 个计划 schema 确认缺失 |
| 生产 bundle 不含 devFallback | `static`（`import.meta.env.DEV` 短路） | `dist/` 不存在，未能验证产物；与 repair progress 的「chunk 消失」自述一致 |

**未执行项（限制）**：
- 未运行 `cargo build`/`cargo test`/`npm run build`（审计约束），因此「代码可编译」「测试通过」均未独立验证，只能引用 repair progress 的自述。
- 未运行真实 Tauri/NAS 端到端，因此 §25.2「兼容未被破坏」、§25.1「发布可用」仅为静态判定。
- `lib.rs` 等超大文件未逐行阅读，命令注册与调用点通过 Grep 定位，可能遗漏动态派发路径。

---

## 判定统计

| 清单 | 符合/已落地 | 部分 | 未完成/未落地 | 合计 |
|---|---:|---:|---:|---:|
| §20.1 产品面删除 | 12 | 0 | 0 | 12 |
| §20.2 待删代码 | 0 | 1 | 11 | 12 |
| §20.3 保留机制 | 8 | 1 | 0 | 9 |
| §20.4 兼容期后删除 | 0 | 0 | 6 | 6 |
| §25.1 必须完成 | 4 | 2 | 2 | 8 |
| §25.2 兼容 | 3 | 0 | 0 | 3 |
| §25.3 不做 | 6 | 0 | 0 | 6 |
| §21 PR | 6 | 4 | 6 | 16 |
| 附录 A 路径 | 13 | 1（指向偏差） | 2 | 16 |
| 附录 B 交付物 | 8 组 | 1（api 3/6） | 7 contracts + 6 目录/脚本组 | — |
| 附录 C 决策 | 7 | 2 | 3 | 12 |

**总体**：§20.1/§20.3/§25.2/§25.3 基本符合；§20.2/§20.4/§21 后半/§23/附录 B 与附录 C 第 4–6、10 条为最大缺口，且 PR-16 的删除 gate 与 §23 的发布门零实现。
