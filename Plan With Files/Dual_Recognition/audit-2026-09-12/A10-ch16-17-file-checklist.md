# A10 — 第 16/17 章逐文件改造清单对抗审计

- 审计日期：2026-09-12
- 审计对象：`Plan With Files/IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md` 第 16 章（前端逐文件，2451–2739）与第 17 章（后端逐文件，2742–3058）
- 基线：`git HEAD = 47a3806` + 未提交改动（新增未跟踪：`src/exam-canvas/structureActions.ts`、`src/features/editor/SelectionInspector.tsx`、`src/api/processingClient.ts`、`src-tauri/src/processing/`、`src-tauri/src/recognition/`）
- 立场：默认每条论断为错/过时/未完成，用代码证据证伪。
- 约束遵守：只读审计，未运行 `cargo build`/`cargo test`/`npm run build`，未修改任何产品代码；仅写本报告。

---

## 章节范围

| 章节 | 计划标题 | 计划文件数 | 实际核对方式 |
|---|---|---|---|
| §16.1–§16.18 | 前端逐文件改造清单 | 18 小节 | Glob 全量 + Grep import 图 + Read 关键文件 |
| §17.1–§17.25 | 后端逐文件改造清单 | 25 小节（含 9 个 `pdf_ingest/*` 子项） | `wc -l` + Grep 符号 + Read 关键函数 |

核心结论：**§16 除 `AppShell.tsx` 外无一项完全达成；§17 核心文件（`lib.rs`/`parser.rs`/`pdf_facts_shadow.rs`/`authoring_pipeline.rs`/`llm_*`/`db.rs`）几乎零推进，§17.25 的 7 个新 schema 为 0/7。** 计划文本描述的"目标状态"与当前仓库"现实状态"存在系统性偏离，且多处**路径不一致**会让按计划施工者直接找错文件。

---

## 文件状态表（前端）

判定口径：`已达成` / `部分` / `未动` / `与计划描述不符`。证据层级见末节。

| 计划章节 | 文件（计划路径 → 实际路径） | 目标状态 | 当前状态判定 | 证据 file:line | 备注 |
|---|---|---|---|---|---|
| §16.1 | `src/app/App.tsx` | 只分发三页；不 import 退休页面；默认 hash `/library` | **部分** | `src/app/App.tsx:3-7`（分发三页 ✅）、`:6,31-33`（仍 import 并渲染 `WritingStudio`）、`:19`（默认 hash `/library` ✅） | 无全局 `listJobs`/`activeJob` ✅；但 `WritingStudio` 仍是退休页面且被 App 直接引用 |
| §16.2 | `src/app/router.ts` | `RouteState` 只剩三种；旧链接重定向表；保留兼容一版 | **部分** | `router.ts:6,33-39`（`RouteName` 含第 4 种 `"legacy"`）、`:42-60`（重定向表 `/jobs/:id/*`→`/items/:id`、`/library/:id`→`/items/:id`、`/export`→`/library?publish=1`、`/dashboard`→`/library` 均 ✅，另多出 `packs`/`phase5`/`jobs/new`）、`:47-49`（保留 `legacy` 逃生通道） | 文档 §16.2 自称"RouteState 只剩三种"，实际保留第四种 `legacy` 与 `#/legacy/...` 路由，**文档未提及**；重定向表本身与文档一致 |
| §16.3 | `src/components/AppShell.tsx` | 仅 Logo/题库/设置；workspace 折叠；不读 active job | **已达成** | `AppShell.tsx:9-12`（仅题库/设置）、`:16,20`（workspace 时 `sidebar` 完全隐藏）、`:14`（只依赖 `route`） | 无"转化工具"组、无 stepper、无 active job 读取 |
| §16.4 | `src/pages/LibraryPage.tsx` → `src/features/library/*` | 全面重写；新增 7 文件目录 | **部分** | `features/library/` 7 文件齐全（LibraryPage/Header/ItemList/ItemRow/BatchBar/libraryStore/libraryTypes）；`src/pages/LibraryPage.tsx`（176 行）仍存在 | **旧文件是孤儿**（全仓无 import，Grep 确认）；`tsconfig.json:19 include:["src"]` → 仍被 `npm run check` 编译 |
| §16.5 | `src/pages/ImportWizard.tsx` → `src/features/import/*` | 退休页面，逻辑迁移到 ImportDrawer/useImportFiles/FileDropzone | **部分** | `features/import/ImportDrawer.tsx`、`useImportFiles.ts` 存在；**`FileDropzone.tsx` 缺失**；`src/pages/ImportWizard.tsx`（325 行）仍在 | 旧 `ImportWizard.tsx` 仍被 `src/app/legacyRoutes.tsx:13,77` import |
| §16.6 | `src/pages/ExamWorkspacePage.tsx`（计划新建） | 新建于 `src/pages/` | **与计划描述不符** | 实际在 `src/features/editor/ExamWorkspacePage.tsx:1-24`；`src/pages/` 下无此文件 | **路径不一致**：按 §16.6 到 `src/pages/` 找不到；职责（加载/保存/发布/drawer + Canvas）已实现 |
| §16.7 | `src/components/ExamCanvasV2.tsx` → `src/exam-canvas/*`（11 文件） | ExamCanvas + ContentNodeRenderer + 5 renderer + 4 editor | **部分** | 实际仅 5 文件：`ExamCanvas.tsx`、`editorCommands.ts`、`renderers/MatchingMatrix.tsx`、`editors/InlineTextEditor.tsx`、`structureActions.ts` | **落地 3/11**：`ContentNodeRenderer.tsx`、`ChoiceTask/CompletionTask/TableTask/VisualTask`、`OptionEditor/TableCellEditor/SlotPlacementEditor` 全部缺失；renderer 仍内联于 `ExamCanvas.tsx`（`:389+`）。`structureActions.ts` 与 `SelectionInspector.tsx` 为计划外新增 |
| §16.8 | `src/editor/authoringTiptap.tsx` | 第一阶段停止生产路由使用；后续删除 + 去 Tiptap 依赖 | **部分** | `authoringTiptap.tsx`（468 行）仍在；唯一引用方 `src/pages/StructuredAuthoringEditorV2.tsx:6`（该页面仅经孤儿 `legacyRoutes` 可达） | 生产主路由已不用 ✅；`package.json:68-75` 仍保留 5 个 `@tiptap/*` 依赖（未删） |
| §16.9 | `src/pages/UnifiedPreview.tsx` | 拆除 7 项能力后删除 | **未动** | `UnifiedPreview.tsx`（1419 行）仍在；`:84` localStorage、`:309` generatePreviewAssets、`:428+` vision/llm 能力全在 | 7 项能力一项未拆；文件未删 |
| §16.10 | `src/pages/StructuredAuthoringEditorV2.tsx` | 提取 useCanonicalEditor/SourceDrawer/issueTargeting；删页面 | **部分** | `features/editor/useCanonicalEditor.ts` 已提取 ✅；**无 `SourceDrawer.tsx`、无 `issueTargeting.ts`**（Grep 全仓无匹配，SourceDrawer 内联于 `ExamWorkspacePage.tsx:226` 附近）；原页面（730 行）仍在 | 3 项提取完成 1 项；issue 定位逻辑落在 `actionableIssues.ts`（名称不同） |
| §16.11 / §16.12 | `src/pages/DocumentReview.tsx`、`src/pages/ExportPage.tsx` | 删正常路由；迁移后删除页面 | **部分** | 两文件仍在（143 / 564 行）；正常路由已删（`App.tsx` 不引用）；仅 `legacyRoutes.tsx:11-12,79-80` 引用 | 页面未删，仍在 `#/legacy/...`；`publishNasPackageV2` 已由 `publishClient.ts` 承载（§16.12 部分达成） |
| §16.13 | `src/pages/Settings.tsx` → `src/features/settings/SettingsPage.tsx` | 重写为 features 版 | **部分** | `features/settings/SettingsPage.tsx` 存在且被 `App.tsx:5,30` 使用 ✅；**`src/pages/Settings.tsx`（352 行）仍存在** | 旧 `Settings.tsx` 为孤儿（全仓无 import），仍参与 tsc |
| §16.14 | `src/api/tauriCommands.ts` → 6 client | 拆为 transport/libraryClient/processingClient/workspaceClient/publishClient/settingsClient | **部分** | 实存 `processingClient.ts`、`workspaceClient.ts`、`publishClient.ts`；**缺 `transport.ts`/`libraryClient.ts`/`settingsClient.ts`**；`tauriCommands.ts`（365 行，~80 导出）仍在 | `tauriCommands.ts:78-84` 仍内联 `isTauriRuntime()` + dev fallback 判断（计划要移到 `transport.ts`） |
| §16.15 | `src/services/devFallbackBackend.ts` + `src/test-support/fakeBackend.ts` | 移出生产 bundle；新建 test-support | **部分** | `devFallbackBackend.ts` 仍在，但两处动态 import 均以 `import.meta.env.DEV` 短路（`tauriCommands.ts:69`、`desktopDialogs.ts:218`）；**`src/test-support/` 目录不存在** | 生产 bundle 移出已达成（repair 进度佐证 chunk 消失）；`fakeBackend.ts` 未建 |
| §16.16 | `src/services/authoringV2Patches.ts` | 协议改名 `EditorCommandV1`；新增 `set_text`；Rust/TS 共享 JSON Schema | **部分** | `EditorCommandV1` 定义在 `src/exam-canvas/editorCommands.ts:15-18`（含 `set_text` ✅），**不在** `authoringV2Patches.ts`；后者仍导出 `AuthoringPatchV2`（19 op）；**无 `contracts/editor-command-v1.schema.json`** | 计划把协议归属写到 `authoringV2Patches.ts`，实际另起文件，**与计划描述不符**；共享 schema 缺失 |
| §16.17 | `src/services/runtimeViewModelV2.ts` → `src/exam-canvas/model/runtimeProjection.ts` | 移入 exam-canvas/model；加断言 | **未动** | `runtimeViewModelV2.ts` 仍在 `src/services/`；仍被 `ExamCanvas.tsx:5` 引用；**无 `src/exam-canvas/model/` 目录** | 文件未迁移，未加 host 断言 |
| §16.18 | `src/styles.css` | 只剩分层 import；删死选择器；`test:layout` | **部分** | `styles.css:1-13` 仅 10 行 `@import` ✅；`package.json:15` `test:layout` 存在 ✅；`src/styles/legacy.css:59,61,62,360,459+` 仍含 `.review-grid`/`.split-grid`/`.phase5-*` | `.editor-grid`/`.llm-grid` 已删（仅存于注释 `legacy.css:8`），但 `.review-grid`（DocumentReview 仍用）、`.split-grid`、`.phase5-*` 仍在 |

**前端小计**：已达成 1（AppShell）；部分 13；未动 2（UnifiedPreview、runtimeViewModelV2）；与计划描述不符 1（ExamWorkspacePage 路径）。合计 17 行（§16.11/16.12 合并）。

---

## 文件状态表（后端）

| 计划章节 | 文件（计划路径 → 实际路径） | 目标状态 | 当前状态判定 | 证据 file:line | 备注 |
|---|---|---|---|---|---|
| §17.1 | `src-tauri/src/lib.rs` | 薄入口；旧命令入 `legacy_commands.rs`；测试按模块迁移 | **未动** | `lib.rs` **11151 行**（455 KB）；`lib.rs:45-86` 30+ `mod` 声明；`:1528` `generate_handler![...]` 注册 **~120 个命令**；`:1499` `pub fn run()`；`grep -c "#\[cfg(test)\]"` = **6**；**无 `legacy_commands.rs`** | 仍是巨型容器；8 命令薄入口目标（文档伪代码）未实现 |
| §17.2 | `parser.rs` | 新链路调 `build_document_ir_v2()`；新增 `ingest_source_to_document_v2`；去 `collapse_whitespace`/fake bbox/`markdownish_to_html` | **未动** | `grep "ingest_source_to_document_v2\|fn build_document_ir_v2"` **无匹配**；`parser.rs:2,128,275,325,1732` 仍用 `collapse_whitespace`/`markdownish_to_html`（2840 行） | 目标接口完全缺失；V1 文本处理仍为主线 |
| §17.3 | `pdf_facts_shadow.rs` → `pdf_facts.rs` | 改名；分离 shadow artifact；统一 `pdf_geometry` 入口 | **未动** | 文件名仍 `pdf_facts_shadow.rs`（**4149 行**）；`pdf_facts_shadow.rs:465,474` 仍 `write_pdf_facts_shadow(_with_v1)`；`:503` 仍写 compare artifact | 未改名、未拆分、未统一入口 |
| §17.4 | `pdf_ingest/*`（9 子项） | 逐文件增强 | **部分** | 9 文件齐全（coordinates/line_builder/region_builder/reading_order/table_detector/ocr_router/ocr_merge/compare_report/mod）；`reading_order.rs:252-268,287-288` 已输出 primary+alternatives ✅；`region_builder.rs` **无 adjacency graph**；`line_builder.rs:309,380,476` 有 `line_id`/`span_id` 但**无 `glyph_id`**；`compare_report.rs` 仍被 `pdf_facts_shadow.rs:503`/`docx_facts_shadow.rs:60` 作为产品 artifact 写盘；`pdf_ingest/mod.rs:5-6` 注释仍称 shadow | 仅 reading_order 一项部分达成；compare_report 未迁入开发者诊断 |
| §17.5 | `docx_facts_shadow.rs` / `docx_ingest/*` | 输出同一 DocumentIRV2；Word 原生 table 优先 physical table；删"XML 顺序=阅读顺序"假设 | **部分** | 13 个 `docx_ingest/*.rs` 存在；`docx_facts_shadow.rs`（2859 行）仍写 shadow；`docx_ingest/tables.rs` 中 `grep physical_table\|PhysicalTable\|DocumentIRV2` **无匹配** | 物理层产出存在（repair 已加 DOCX 回归），但"原生 table 优先 physical table"未实现 |
| §17.6 | `authoring_pipeline.rs` → `legacy/v1_authoring_pipeline.rs` | 冻结隔离；新 worker 不再调用 | **未动** | 仍在根目录，**13848 行**（最大文件）；**无 `src-tauri/src/legacy/`**；被 `auto_pipeline.rs:2`、`lib.rs:493,495,1605`、`parser.rs:1`、`pdf_geometry.rs:3156,3162` 等大量调用 | M4 关键判据：新链仍以 V1 为权威稿来源 |
| §17.7 | `ielts_grammar/mod.rs` → `recognition/local` | 拆为新模块；映射表 8 项；入口 `recognize_local` | **部分** | `recognition/local/` 仅 2 文件：`mod.rs`(233 行, QuestionLayoutGraphV1) + `question_blocks.rs`(1040 行)；`grep "fn recognize_local"` **无匹配**；`ielts_grammar/mod.rs` 仍 **3030 行**、`quality.rs` **4487 行**、`real_pdf_acceptance.rs` 2217 行 | 映射表 8 项基本未做（仅 `question_blocks.rs` 对应 `question_blocks`）；无 `recognize_local` 入口；`recognition/cloud`、`reconcile` 不存在 |
| §17.8 | `auto_pipeline.rs` | 拆 worker；不再由 blocking command 等整链；本地候选即时持久化 | **部分** | `processing/` 新目录存在（mod/queue/scheduler/commands）；但 `auto_pipeline.rs` 仍 **2813 行**且仍走 V1；`lib.rs:493` 仍同步调 `make_dynamic_split_candidates`；`grep publish_library_items` 无匹配 | 队列骨架落地（M2），但整链仍同步、本地候选未即时持久化 |
| §17.9 | `llm_gateway.rs` | 保留 transport；移出 IELTS prompt；新增 `LlmTransport` trait | **未动** | `grep "LlmTransport"` **无匹配**；`llm_gateway.rs:458,471,478,486` 仍内嵌 IELTS prompt 字符串（1365 行） | trait 缺失；prompt 未移出 |
| §17.10 | `llm_suggestions.rs` | 拆 5 文件；`CloudReadingOutlineV1` 降为诊断 | **未动** | `llm_suggestions.rs` 仍在根（737 行）；无 `recognition/cloud/*`、`legacy/v1_llm_suggestions.rs` | 未拆分 |
| §17.11 | `llm_commands.rs` | Profile CRUD 迁 `settings/model_profiles.rs`；新增 `retry_cloud_recognition` | **未动** | `grep "retry_cloud_recognition"` **无匹配**；`llm_commands.rs` 仍在根（750 行）；`llm_profiles.rs` 未迁入 settings 目录 | 新命令缺失 |
| §17.12 | `db.rs` | V2 单事实源；SQL 拆 `library/schema.rs`；删 `exams`/`library_items` COALESCE | **未动** | `db.rs` **1349 行**；`:83` 仍 `CREATE TABLE exams`；`:330-331` 仍 `COALESCE(li.title, e.title)`；`library/schema.rs` 存在但 db.rs 未收敛 | 混合查询仍在 |
| §17.13 | `library_commands.rs` | thin commands 调 Repository；删双向 best-effort 同步 | **未动** | `library_commands.rs:245,293` 仍 `write_back_meta_to_source`（DB→源文件回写） | 双向同步未删 |
| §17.14 | `job_store.rs` → `processing/repository.rs` | 替换；job.json 非权威 | **未动** | `job_store.rs`（105 行）仍在；`job_commands.rs:3,97` 仍用 `list_saved_jobs`；`processing/commands.rs:11` 仍 `use crate::job_store::{make_job, save_job}`；**无 `processing/repository.rs`** | 导入链仍写 job.json |
| §17.15 | `artifact_store.rs` | 短期收敛；atomic write 下沉 `util/atomic.rs` | **未动** | `artifact_store.rs`（888 行）仍在；**无 `src-tauri/src/util/`**（`util.rs` 为单文件） | 未拆 atomic helper |
| §17.16 | `authoring_v2_commands.rs` | 拆 5 文件；preflight 只查 current | **部分** | 文件仍根（**2766 行**）；`validate_authoring_v2_publish_readiness` 在 `:187`，现只查当前 revision + 当前 quality/issues/hard_failures（`：207-255`）✅ 已不再递归历史 JSON；`:449` 出现字面量 `"PublishCheckResultV1"`（无 schema 文件） | 预检逻辑已收敛（F 系列修复副产品），但文件未拆、typed 对象未落地 |
| §17.17 | `cleanup.rs` → `library/cleanup.rs` | 重写；引用驱动；四触发点 | **未动** | `cleanup.rs`（213 行）仍在根；**无 `library/cleanup.rs`** | 未迁移 |
| §17.18 | `source_review.rs` / `authoring_review.rs` | 输出 `ActionableIssue`；blocker 修完自动 Ready | **未动** | 仅 DB 表 `actionable_issues_v1`（`library/schema.rs:87`）；Rust 侧 `grep "ActionableIssue"` 无类型定义（1299 行 authoring_review 无该类型） | 后端未产出 ActionableIssue（前端 `actionableIssues.ts` 自行派生） |
| §17.19 | `authoring_validation.rs` / `runtime_validation.rs` / `validator.rs` | 合并为 3 个 validator；typed 错误 | **未动** | `grep "CanonicalDsValidator\|RecognitionCandidateValidator\|PublishPreflight"` **无匹配**；三文件仍在根（213/389/479 行） | 未合并 |
| §17.20 | `reading_source_v2.rs` | 唯一 runtime compiler；`compile_with_report()` | **未动** | `grep "compile_with_report"` **无匹配**；`reading_source_v2.rs`（1056 行）仍在根 | 目标函数缺失 |
| §17.21 | `nas_package_v2.rs` | batch compile；只接受过 preflight 的 DS；结果精简 | **部分** | `nas_package_v2.rs:261 publish_items_core`；`lib.rs:970 publish_items`；`product_chain.rs:1046 publish_items_is_all_or_nothing_across_a_batch`（原子性 ✅）；`grep batch_compile` 无匹配 | 原子批量已达成（M6）；batch compile / preflight 前置约束未验证 |
| §17.22 | `preview_commands.rs` | 删 HTML preview 主链；仅留 source/asset preview | **未动** | `preview_commands.rs:4,84,91` 仍 `resolve_external_unified_html` 生成 HTML preview（144 行） | 未拆 |
| §17.23 | `environment.rs` / `diagnostics.rs` | 启动后台一次；仅 blocking 显示；开发者模式全量 | **未动** | `environment.rs`（706 行）未改；`diagnostics.rs`（30 行）无开发者模式收敛迹象 | 未见 §17.23 改造 |
| §17.24 | `export_nas_library.rs` / `export_artifacts.rs` / `export_pack.rs` | 新 Reading 只调 V2；V1 export 计调用量后删 | **未动** | 三文件仍在根（1727/149/328 行）；`export_nas_library`/`export_pack` 仍注册于 `lib.rs` invoke_handler | 未收敛 |
| §17.25 | `schema/*` 与 `contracts/*` | 新增 7 个 schema；保留 4 个 | **未动** | `contracts/` 实际 9 个 schema + manifest：common-v2/content-doc-v2/document-ir-v2/ielts-authoring-ir-v2/quality-report-v2/reading-exam-source-v2/listening-*；**7 个新 schema 0/7**（`processing-item-v1`/`recognition-candidate-v1`/`cloud-recognition-candidate-v1`/`reconciliation-proposal-v1`/`actionable-issue-v1`/`editor-command-v1`/`publish-check-result-v1` 均不存在） | 保留的 4 个（document-ir-v2/content-doc-v2/ielts-authoring-ir-v2/reading-exam-source-v2）齐 ✅ |

**后端小计**：部分 5（§17.4、§17.5、§17.7、§17.8、§17.16、§17.21 中归并为 5 组）；未动 7（§17.1/17.2/17.3/17.6/17.9-17.15/17.17-17.20/17.22-17.25 归并）。合计 12 行，覆盖 25 个文件条目。**无一项判定为"已达成"。**

---

## 发现清单

### A10-F01 [P1] 文档路径与仓库实际路径系统性不一致，按计划施工会找错文件
- 结论：§16.6 指定新建 `src/pages/ExamWorkspacePage.tsx`，实际在 `src/features/editor/ExamWorkspacePage.tsx`；§16.7 指定 `src/components/ExamCanvasV2.tsx`，实际在 `src/exam-canvas/ExamCanvas.tsx`；§16.16 要求把协议改名写在 `src/services/authoringV2Patches.ts`，实际 `EditorCommandV1` 定义在 `src/exam-canvas/editorCommands.ts`。
- 证据：`src/features/editor/ExamWorkspacePage.tsx:1`、`src/exam-canvas/ExamCanvas.tsx:389`、`src/exam-canvas/editorCommands.ts:15`；`src/pages/` 与 `src/components/` 下无对应文件（Glob 全量确认）。
- 影响：任何按 §16 清单直接编辑"计划路径"的施工/审计都会失败或误判；计划未记录改名事实。
- 建议：在 §16 增补"计划路径 vs 实际路径"映射列，或修正计划路径。

### A10-F02 [P1] 第 16 章整体落地率极低，退休页面整套仍在并被孤儿文件导入
- 结论：§16.9 `UnifiedPreview`（1419 行）、§16.10 `StructuredAuthoringEditorV2`（730 行）、§16.11 `DocumentReview`、§16.12 `ExportPage`（564 行）、§16.13 旧 `Settings.tsx`（352 行）全部仍存在；除 `UnifiedPreview` 外均由**孤儿** `src/app/legacyRoutes.tsx:10-18` 统一 import，导致 `npm run check` 仍编译整套退休 UI。
- 证据：`src/app/legacyRoutes.tsx:10-18`；`src/app/App.tsx:6,31-33`（直接 import `WritingStudio`）；`tsconfig.json:19 include:["src"]`。
- 影响：§16 的"删除/迁移完成后删除"验收条件（App 入口不再 import 退休页面）未达成；退休代码持续产生维护与误用风险。
- 建议：明确 §16 各项属 P10 范围并在计划中标注"未开工"，而非以目标态描述；优先删除 `App.tsx` 对 `WritingStudio` 的直接引用。

### A10-F03 [P1] 第 17 章核心文件零推进，M4/M5 主链仍为 V1
- 结论：§17.1/17.2/17.3/17.6/17.9/17.12/17.13/17.14 全部未动。`lib.rs` 仍 11151 行、注册 ~120 命令；`parser.rs` 无 `ingest_source_to_document_v2`；`pdf_facts_shadow.rs` 未改名；`authoring_pipeline.rs` 仍在根目录 13848 行且被 `lib.rs:493-495` 的 V1 split 路径调用；`llm_gateway.rs` 无 `LlmTransport`、prompt 仍内嵌。
- 证据：见上表 §17.1/17.2/17.3/17.6/17.9 行；`lib.rs:493`、`authoring_pipeline.rs:4193`。
- 影响：M4"本地识别主链替换"、M5"云端完整候选"无实现基础；任务书验收无法推进。
- 建议：§17 各节标注真实进度（`未开工`/`部分`），并优先落地 §17.2 目标接口与 §17.6 隔离。

### A10-F04 [P1] 孤儿文件仍参与构建，且删除会连锁
- 结论：`src/pages/LibraryPage.tsx`、`src/pages/Settings.tsx`、`src/app/legacyRoutes.tsx` 全仓无 import（真孤儿），但因 `tsconfig include:["src"]` 仍被 `npm run check` 编译；`legacyRoutes.tsx` 一旦单独删除会连带 8 个页面组件成为孤儿。
- 证据：Grep `from ".*pages/LibraryPage"` / `pages/Settings` 均无匹配；`task_plan.md:178`、`repair-2026-09-07/progress.md:64`。
- 影响：孤儿代码无法被死代码检测发现，类型错误会阻塞 `npm run check`；删除必须成套进行。
- 建议：P10 按"退休页面集合"整体删除，并同步 `tsconfig` 排除或删除文件。

### A10-F05 [P2] 同一职责两份实现并存
- 结论：题库（`src/pages/LibraryPage.tsx` vs `src/features/library/LibraryPage.tsx`）、设置（`src/pages/Settings.tsx` vs `src/features/settings/SettingsPage.tsx`）、ExamCanvas renderer（内联 `ContentNodes` vs 计划拆分文件）、EditorCommand（`authoringV2Patches.ts` 的 `AuthoringPatchV2` vs `exam-canvas/editorCommands.ts` 的 `EditorCommandV1`）、dev fallback（生产 `devFallbackBackend.ts` vs 计划 `test-support/fakeBackend.ts` 缺失）均存在双份或缺口。
- 证据：见前端状态表 §16.4/16.13/16.7/16.16/16.15 行。
- 影响：修改一处易漏另一处；测试替身缺失使测试仍依赖生产 fallback 代码。
- 建议：明确"唯一实现"归属并删除旧份；补建 `test-support/fakeBackend.ts`。

### A10-F06 [P2] 计划内部矛盾（§16/§17 与 §5.1、§20 不一致）
- 结论：
  1. §16.2 要求 `RouteState` 只剩三种，同节又要求保留旧链接兼容重定向一个版本；实际实现保留第四种 `"legacy"` 与 `#/legacy/...`，文档未提及。
  2. §16.4/§16.13 把 `src/pages/LibraryPage.tsx`、`src/pages/Settings.tsx` 描述为"重写为 features 版"，但 §20.2 删除清单**未列**这两个旧文件。
  3. §17.7 的 `recognition/local` 文件列表（含 `current_ds_validation.rs`）与 §5.1 模块布局（含 `visual_stimulus.rs`、无 `current_ds_validation`）不一致。
  4. §17 描述大量"新增/移入"目标，但 §20.2 未同步这些后端删除项。
- 证据：`router.ts:6`；§20.2（计划 3689-3704）；§5.1（计划 484-494）与 §17.7（计划 2839-2846）。
- 影响：施工范围与删除范围对不齐，验收口径模糊。
- 建议：统一 §5.1/§16/§17/§20 的文件清单，并单列"未开工"标记。

### A10-F07 [P2] §17.4 `compare_report` 仍作为产品 artifact 写盘
- 结论：计划要求"迁入开发者诊断，不作为产品必需 artifact"，实际 `build_compare_report` 仍被 PDF/DOCX 物理事实写盘路径调用。
- 证据：`src-tauri/src/pdf_facts_shadow.rs:503`、`src-tauri/src/docx_facts_shadow.rs:60`、`pdf_ingest/mod.rs:17`。
- 影响：产品链路持续产出诊断 artifact，与"零冗余 artifact"目标相悖。
- 建议：将 compare report 置于开发者开关下。

### A10-F08 [P2] §17.9/17.11 LLM 网关改造缺失
- 结论：`LlmTransport` trait 不存在；IELTS prompt 字符串仍内嵌于 `llm_gateway.rs:458/471/478/486`；`retry_cloud_recognition` 命令不存在。
- 证据：Grep 无匹配；`llm_gateway.rs:458-486`。
- 影响：M5 云端完整候选无 transport 抽象与重试入口。
- 建议：先落 `LlmTransport` trait 与 prompt 外置，再补 `retry_cloud_recognition`。

### A10-F09 [P3] 后端"shadow"命名未去除
- 结论：`pdf_facts_shadow.rs`、`docx_facts_shadow.rs` 未改名 `pdf_facts.rs`/`docx_facts.rs`；`pdf_ingest/mod.rs:5-6` 注释仍称 shadow representation；`write_pdf_facts_shadow`/`write_docx_facts_shadow` 函数名保留。
- 证据：`pdf_facts_shadow.rs:465,474`、`docx_facts_shadow.rs:28,37`、`pdf_ingest/mod.rs:5-6`。
- 影响：命名与"去 shadow 化"目标不一致，易误判链路已切换。
- 建议：随 §17.3/17.5 一并改名并更新注释。

### A10-F10 [P3] 数据面双写与混合查询未清除
- 结论：`db.rs:83` 仍建 `exams` 表、`:330-331` 仍 `COALESCE(li.title, e.title)` 混合；`library_commands.rs:245,293` 仍 DB→源文件回写；`job_store.rs` 仍被导入链使用。
- 证据：`db.rs:83,330-331`；`library_commands.rs:245,293`；`processing/commands.rs:11`。
- 影响：§17.12/17.13/17.14 的"单事实源"目标未达成，仍存双写竞态面。
- 建议：按 §17.12-17.14 逐步摘除兼容层。

### A10-F11 [P2] 关键新增代码处于未跟踪状态，审计/构建基线不稳定
- 结论：`src/exam-canvas/structureActions.ts`、`src/features/editor/SelectionInspector.tsx`、`src/api/processingClient.ts`、`src-tauri/src/processing/`、`src-tauri/src/recognition/` 均未 `git add`；`recognition/local` 虽在 `lib.rs:85` 声明并编译，但仅内部自引用，未接入任何生产路径。
- 证据：`git status --porcelain`；`lib.rs:85`；Grep `recognition::` 仅命中其自身注释。
- 影响：M3/M4 的新增成果未纳入版本控制，回滚/移交有丢失风险；`recognition/local` 现为未接线模块。
- 建议：提交这批新增文件，并在计划中记录 `recognition/local` 仅完成 QuestionLayoutGraph 起步。

### 正向确认（对抗立场下的"证伪失败"项）
- F11 修复属实：结构编辑已接入工作区——`ExamWorkspacePage.tsx:9,227,235` 使用 `compileStructureAction` + `SelectionInspector`，`structureActions.ts` 覆盖选项/表格/答案位等操作，不再依赖退休页面。
- F13 部分属实：`devFallbackBackend` 两处动态 import 均已用 `import.meta.env.DEV` 短路（`tauriCommands.ts:69`、`desktopDialogs.ts:218`），生产 bundle 已移出（repair 进度佐证）。
- M6 批量原子发布属实：`product_chain.rs:1046` 有整批故障注入测试。
- §16.18 `test:layout` 与分层 import 属实。

---

## 与历史审计的关系

| 历史发现 | 本轮（A10）核对结果 |
|---|---|
| F11 结构修复未接入工作区 | **已修复**：`structureActions.ts` + `SelectionInspector.tsx` 已接入 `ExamWorkspacePage.tsx:227/235`（属未提交改动） |
| F13 生产 bundle 含测试替身与退休 UI | **部分修复**：测试替身已由 `import.meta.env.DEV` 短路移出生产 bundle ✅；但"退休 UI"未清——`UnifiedPreview`/`StructuredAuthoringEditorV2`/`DocumentReview`/`ExportPage`/旧 `Settings.tsx` 全在，且 `App.tsx:6` 仍直接 import `WritingStudio` |
| `task_plan.md` 已知差距第 7 条：`src/pages/LibraryPage.tsx` 孤儿 | **确认**：全仓无 import，但 `tsconfig include:["src"]` 使其仍参与 tsc |
| `task_plan.md` 已知差距第 8 条：`.review-grid` 仅兼容页 DocumentReview 用 | **确认**：`src/styles/legacy.css:59,61,360` 仍定义 `.review-grid` |
| `repair-2026-09-07/progress.md`：`src/app/legacyRoutes.tsx` 孤儿 | **确认**：`App.tsx` 不再 import；但该文件仍 import 8 个退休页面，使整组仍被编译 |

**与任务书里程碑的一致性**：`task_plan.md` 已诚实记录 M4 部分完成、M5 pending、M7 pending；但**权威计划 §16/§17 未同步这一口径**，仍以"改造/迁移为"的目标态书写，容易被误读为已完成。这是本轮最需修正的文档缺陷。

---

## 证据层级与局限

| 证据层级 | 本轮使用 | 说明 |
|---|---|---|
| `product`（真实 Tauri 端到端） | 未使用 | 未运行 `npm run e2e:tauri`，无真实 UI/SQLite/NAS 运行证据 |
| `command`（命令/服务层） | 间接使用 | 未运行 `cargo test`/`npm run build`；仅静态追踪命令注册（`lib.rs:1528`）与 Rust 符号 |
| `static`（静态代码证据） | **主要依据** | Glob/Grep/Read/`wc -l` 覆盖全部 18+25 条目；所有判定均可回溯到 file:line |
| `doc-only`（文档间一致性） | 使用 | §16/§17 与 §5.1/§20.2、`task_plan.md`、历史 audit/repair 的交叉比对 |

局限：
1. 未编译、未运行测试，故"是否可构建/是否通过验收"不在本报告结论内（历史 repair 声称 `cargo test` 567 通过、`npm run check` 通过，本轮未复核）。
2. 部分"部分达成"判定基于关键字存在性（如 `physical_table`），未深入逐行验证语义等价；`glyph_id` 缺失、`region adjacency graph` 缺失按 Grep 无匹配判定。
3. `pdf_ingest/*` 与 `docx_ingest/*` 的逐文件增强属细粒度语义判断，本轮仅覆盖计划中显式命名的能力点，未做全量行为回归。
4. 未跟踪文件的内容以磁盘现状为准，可能与作者后续意图存在差异。
