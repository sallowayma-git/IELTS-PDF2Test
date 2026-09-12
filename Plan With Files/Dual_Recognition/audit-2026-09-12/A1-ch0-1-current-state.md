# A1 审计报告 — 第 0 章（文档目的、结论与使用方式）与第 1 章（当前远端状态审计）

- 审计对象：`Plan With Files/IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md` 第 12–166 行（§0、§1）
- 审计日期：2026-09-12
- 审计立场：默认每条论断为错/过时/未完成，用当前代码证据证伪
- 实际审计基线：`git rev-parse HEAD` = `47a38064aef58e023bce58e5b6feafa99031b252`（m1），工作树有 **40 项未提交改动**（含 M2 `src-tauri/src/processing/`、修复轮改动）
- 约束遵守：只读审计，未修改任何产品代码/配置；未运行 `cargo build` / `cargo test` / `npm run build`

---

## 章节范围

- §0 文档目的、结论与使用方式（第 12–59 行）：五个核心问题、目标架构、三主表面、工程原则
- §1 当前远端状态审计（第 62–166 行）：§1.1 审计基线、§1.2 前端 19 行清单、§1.3 后端 28 行清单、§1.4 正向基础 8 条、§1.5 缺口矩阵 19 条

核心结论：**§0 的目标方向与当前产品收敛方向一致且大部分已落地；§1 作为"当前远端状态"快照已严重过时**——它冻结在 `bb978be`，而仓库 HEAD 已是 `47a3806` 且带 M2/修复轮未提交改动。§1.2/§1.5 的多数"当前判断/当前缺口"在 M0–M3 与修复轮后不再成立，属于系统性 stale snapshot 缺陷（A1-F01/F02）。

---

## 断言核对表

判定口径：`HOLDS` = 当前代码支持；`PARTIAL` = 部分成立或成立但描述已偏移；`FALSE` = 不实/已被后续提交推翻；`UNVERIFIABLE` = 静态证据不足。

### §0（文档目的与原则）

| # | 断言 | 判定 | 证据 | 说明 |
|---|---|---|---|---|
| 0-1 | §0.1 问题1「前端页面过多」 | PARTIAL | `src/pages/` 仍有 11 个 `.tsx`（`ls src/pages/`），但 `src/app/App.tsx:24-35` 只分发 3 主表面 + writing | 页面文件仍在，但已不在导航；问题是"残留"非"导航过多" |
| 0-2 | §0.1 问题3「本地识别仍经 V1 block/split/authoring 链路」 | HOLDS | `src-tauri/src/auto_pipeline.rs:187,208` `make_dynamic_split_candidates` → `build_authoring_v2_shadow` | M4 未交付，主链仍 V1 |
| 0-3 | §0.1 问题4「云端只产 `CloudReadingOutlineV1`」 | HOLDS | `src-tauri/src/llm_gateway.rs:484-486` prompt 明写 "comparison-only outline" | M5 pending |
| 0-4 | §0.1 问题4「队列由 `UnifiedPreview.tsx` 用 localStorage+lease+worker 管理」 | PARTIAL | `src/pages/UnifiedPreview.tsx:24,84,138` 仍有 lease；但主链已改 `src/features/import/useImportFiles.ts:3` → `processingClient` | 描述指向的是已退休页面，非当前主链 |
| 0-5 | §0.4「编辑 Canonical DS，不直接编辑生成 JS」 | HOLDS | `src-tauri/src/library/repository.rs`、`src/features/editor/useCanonicalEditor.ts` | M1 已落地 |

### §1.1 审计基线

| # | 断言 | 判定 | 证据 | 说明 |
|---|---|---|---|---|
| 1-1 | §1.1「本计划固定在 `bb978be`」 | **FALSE** | 文档第 4、69 行 vs `git rev-parse HEAD` = `47a3806`；`git status --porcelain` 40 项 | 实际 HEAD 领先 3 个提交（`5daa04f`→`401ca76`→`47a3806`）并有大量未提交改动 |
| 1-2 | 默认打开 Reading V2 | HOLDS | `src/config/featureFlags.ts:12-13` `documentIrV2:true` / `runtimeSourceV2:true` | 无同名 flag，语义对应 |
| 1-3 | 默认打开 Authoring V2 | HOLDS | `src/config/featureFlags.ts:12`；`src-tauri/src/environment.rs:223-225` | 前后端一致 |
| 1-4 | 默认打开 Runtime V2 | HOLDS | `src/config/featureFlags.ts:13` `runtimeSourceV2:true` | |
| 1-5 | 默认打开 NAS Package V2 | HOLDS | `src/config/featureFlags.ts:14` `nasPackageV2:true` | |
| 1-6 | 默认打开 Quality Gate V2 | HOLDS | `src/config/featureFlags.ts:42`；`environment.rs:227-229` `quality_gate_v2_enabled()==true` | |
| 1-7 | 默认打开 Phase 5 Editor | HOLDS | `src/config/featureFlags.ts:64` `authoringEditorV2:true`；`featureFlags.ts:79-81` `isPhase5EditorEnabled()==true` | |
| 1-8 | Listening 默认关闭 | HOLDS | `src/config/featureFlags.ts:15` `listeningV1:false` | |
| 1-9 | 逐题 PDF LLM Repair 强制关闭 | HOLDS | `src/config/featureFlags.ts:28,54,75` 三处 `resolve*` 强制置 false 且注释"not overridable" | |
| 1-10 | 「问题不是开关未启用，而是新旧链路并存」 | HOLDS | `src/pages/` 11 文件 + `src/app/legacyRoutes.tsx:10-18` 仍 import 8 旧页；`library_commands.rs:291,338` 仍 DB↔JSON 双写 | |

### §1.2 前端模块清单（19 行）

| # | 断言（文件/职责/判断） | 判定 | 证据 | 说明 |
|---|---|---|---|---|
| 2-1 | `src/app/App.tsx` 分发 **11 个页面** | **FALSE** | `src/app/App.tsx:24-35` 仅 `library`/`workspace`/`settings` + legacy writing | Phase 1（S1.2）已重写；`-61.1%` |
| 2-2 | `src/app/router.ts` hash 路由包含 document/split/groups/llm-review/preview/authoring-v2/export | **FALSE** | `src/app/router.ts:6` `RouteName = library\|workspace\|settings\|legacy`；`:42-60` 那些阶段仅存在于 `legacyRedirect` 映射 | 阶段路由已删除，只留重定向 |
| 2-3 | `src/components/AppShell.tsx` 有侧栏/转化工具分组/步骤条/job 技术状态 | **FALSE** | `src/components/AppShell.tsx:9-12` 仅 2 个导航项；`:7` 注释"已删除：转化工具展开组、stepper、activeJob 技术条"；51 行 | S1.3 已极简化 |
| 2-4 | `src/components/ExamCanvasV2.tsx` 存在 | **FALSE** | `ls src/components/ExamCanvasV2.tsx` → No such file；实际为 `src/exam-canvas/ExamCanvas.tsx`（475 行） | S3.2 改名迁移；**计划目标路径指向不存在的文件** |
| 2-5 | `src/editor/authoringTiptap.tsx` 存在 | HOLDS | 468 行，文件存在 | |
| 2-6 | `src/pages/Dashboard.tsx` 存在 | HOLDS | 文件存在，经 `legacyRoutes.tsx:10` 引用 | |
| 2-7 | `src/pages/JobList.tsx` 存在 | HOLDS | 文件存在，`legacyRoutes.tsx:14` | |
| 2-8 | `src/pages/ImportWizard.tsx` 存在 | HOLDS | 文件存在，`legacyRoutes.tsx:13` | |
| 2-9 | `src/pages/DocumentReview.tsx` 存在 | HOLDS | 文件存在，`legacyRoutes.tsx:11` | |
| 2-10 | `src/pages/UnifiedPreview.tsx` 存在且含云端队列/localStorage | HOLDS | 文件存在；`UnifiedPreview.tsx:24,84,138` | |
| 2-11 | `src/pages/StructuredAuthoringEditorV2.tsx` 存在 | HOLDS | 文件存在，`legacyRoutes.tsx:16` | |
| 2-12 | `src/pages/LibraryPage.tsx` 存在 | HOLDS | 文件存在（孤儿，`task_plan.md:178`） | |
| 2-13 | `src/pages/LibraryExamDetail.tsx` 存在 | HOLDS | 文件存在，`legacyRoutes.tsx:15` | |
| 2-14 | `src/pages/ExportPage.tsx` 存在 | HOLDS | 文件存在，`legacyRoutes.tsx:12` | |
| 2-15 | `src/pages/Settings.tsx` 存在 | HOLDS | 文件存在 | |
| 2-16 | `src/api/tauriCommands.ts`「过大且和 dev fallback 强耦合」 | PARTIAL | 364 行/12.8KB（非"过大"）；`tauriCommands.ts:69` 已被 `import.meta.env.DEV` 守卫，耦合解耦 | 判断列过时 |
| 2-17 | `src/services/devFallbackBackend.ts`「在浏览器 localStorage 中复制大量后端行为 / 容易分叉」 | PARTIAL | 文件 3888 行/176KB（复制行为属实）；但 `tauriCommands.ts:69`、`desktopDialogs.ts:218` 均有 `import.meta.env.DEV` 短路，`dist/assets/` 无该 chunk | **F13 已修复**；"从生产路径移除"的目标已达成，判断列未更新 |
| 2-18 | `src/services/authoringV2Patches.ts` 可复用良好基础 | HOLDS | 575 行，文件存在 | |
| 2-19 | `src/services/runtimeViewModelV2.ts` 同源构建 | HOLDS | 90 行，文件存在 | |
| 2-20 | `src/styles.css`「约 **64 KB**」 | **FALSE** | `wc -c src/styles.css` = **582 字节**；`src/styles.css:1-13` 仅 10 行 `@import` | S1.4 已拆为 `src/styles/*.css`（10 个分片文件） |

### §1.3 后端模块清单（28 行）

| # | 断言（模块/职责） | 判定 | 证据 | 说明 |
|---|---|---|---|---|
| 3-1 | `lib.rs` 巨型文件（类型+command+注册+巨量测试） | HOLDS | `wc -l` = **11151** 行/455KB；`grep -c "#[test]"` = 136；`grep -c "#[tauri::command]"` = 67 | |
| 3-2 | `parser.rs` V1 DocumentIR 构建/多格式入口/HTML+启发式 role | HOLDS | `src-tauri/src/parser.rs:349,594,1746,2629` 均输出 `"schemaVersion": "DocumentIRV1"`；`:2519 parse_source_document` | |
| 3-3 | `pdf_facts_shadow.rs` PDF glyph/vector/image/geometry facts | HOLDS | `pdf_facts_shadow.rs:465 write_pdf_facts_shadow`、`:474 write_pdf_facts_shadow_with_v1` | |
| 3-4 | 仍带 shadow 语义 | HOLDS | `pdf_facts_shadow.rs:19-22` `SHADOW_ARTIFACT_FILE`/`SHADOW_OVERLAY_FILE` 等；`environment.rs:219-221` 名为 `document_ir_v2_shadow_enabled` | de-shadowing（P4-T01 剩余项）未做 |
| 3-5 | `pdf_ingest/*` line/region/reading order/table/OCR merge/坐标 | HOLDS | 目录存在 | |
| 3-6 | `pdf_geometry.rs` PDFium 几何提取 | HOLDS | 文件存在 | |
| 3-7 | `docx_facts_shadow.rs` / `docx_ingest/*` | HOLDS | 目录/文件存在 | |
| 3-8 | `authoring_pipeline.rs` 超大 V1 split/authoring 启发式 | HOLDS | `wc -l` = **13848** 行/**547,026 字节** | 超出计划 §1.5 CODE-001 的"100KB-500KB"上界，见 A1-F09 |
| 3-9 | `ielts_grammar/*` 从 V1 candidate + physical lines 构建 V2 | HOLDS | `src-tauri/src/ielts_grammar/mod.rs:135,414` 读取 `"questionGroupCandidates"`（V1 产物） | |
| 3-10 | `auto_pipeline.rs` 大型本地/云编排 | HOLDS | 2813 行 | |
| 3-11 | `llm_gateway.rs` OpenAI-compatible/Ollama、JSON 提取、重试 | HOLDS | `llm_gateway.rs:121 llm_timeout`、`:251 balanced_json_end`、`:306-308 retry_after`、`:484 cloud_outline_prompt` | |
| 3-12 | `llm_suggestions.rs` V1 group repair/outline input/候选落盘 | HOLDS | 文件存在 | |
| 3-13 | `llm_commands.rs` Profile 管理/题组建议/应用建议 | HOLDS | 文件存在 | |
| 3-14 | `artifact_store.rs` 不可变 revision/patch/hash/锁 | HOLDS | `artifact_store.rs:45-46 revisions_dir/patches_dir`、`:73 current_revision_path`、`:77 revision_path` | |
| 3-15 | `job_store.rs` `job.json` 事实源 + best-effort 双写 DB | HOLDS | `job_store.rs:30,33-35` 读写 `job.json`；`library_commands.rs:338` 注释"save_job 会触发双写 DB" | LIB-001 仍成立 |
| 3-16 | `db.rs` legacy `exams` + `library_items` 双模型 | HOLDS | `db.rs:78-83` "兼容层：旧 exams 表…逐步迁移"；`:108 library_items` | |
| 3-17 | `library_commands.rs` Job/Writing 与 DB 双写 + 元数据回写 JSON | HOLDS | `library_commands.rs:291` "把题库元数据编辑回写对应的 JSON 源文件" | |
| 3-18 | `cleanup.rs` 仍保留 job/authoring/project/source review/uploads/exports | HOLDS | `cleanup.rs:38 write_authoring_project`、`:43-46` 读 job/authoring-ir/source_review | |
| 3-19 | `authoring_v2_commands.rs` Session/patch/revision/publish readiness | HOLDS | 文件存在（工作树已修改） | |
| 3-20 | `authoring_review.rs` / `source_review.rs` | HOLDS | 文件存在 | |
| 3-21 | `reading_source_v2.rs` 从 IeltsAuthoringIRV2 编译 Runtime V2 + slot closure | HOLDS | `reading_source_v2.rs:89-97,144-156` 排序/校验 `answer_slots` | |
| 3-22 | `reading_runtime_v2.rs` attempt/scoring/runtime | HOLDS | 文件存在 | |
| 3-23 | `nas_package_v2.rs` V2 staging/manifest/commit | HOLDS | `nas_package_v2.rs:5` "manifest last"、`:68 manifest-v2.json`、`:203 DB 直通发布` | |
| 3-24 | `runtime_validation.rs` / `authoring_validation.rs` | HOLDS | 文件存在 | |
| 3-25 | `preview_commands.rs`「生成另一个预览产物」 | PARTIAL | `preview_commands.rs:20,37,72` 只有 `validate_authoring_ir_core` / `generate_preview_assets_core` / `run_preview_e2e_core` | 描述偏窄，且无 `#[tauri::command]` |
| 3-26 | `export_*` V1 JS/pack/writing/NAS 多套发布 | HOLDS | `export_artifacts.rs`/`export_pack.rs`/`export_writing_library.rs`/`export_nas_library.rs` 均存在 | |
| 3-27 | `environment.rs` / `diagnostics.rs` 环境预检与诊断 | HOLDS | 文件存在；`environment.rs:197-229` flag/env 解析 | |

### §1.4 正向基础（8 条）

| # | 断言 | 判定 | 证据 | 说明 |
|---|---|---|---|---|
| 4-1 | `DocumentIRV2` glyph/span/line/region/vector/table/asset/reading-order 模型 | HOLDS | `src-tauri/src/schema/document_ir_v2.rs:458 pub struct DocumentIRV2`；`:306 glyphs`、`:319-321 reading_order/graph` | |
| 4-2 | `pdf_ingest` 坐标/行/区域/表格/OCR merge | HOLDS | 目录存在 | |
| 4-3 | `IeltsAuthoringIRV2` task/response group/option bank/answer slot/ContentDoc/asset | HOLDS | `src-tauri/src/schema/ielts_authoring_v2.rs:524 pub struct IeltsAuthoringIRV2`；`:253 option_bank`、`:534 answer_slots` | |
| 4-4 | `ExamCanvasV2` student/author 双模式 + 表格/图形/热点/答案槽渲染 | PARTIAL | 双模式属实：`src/exam-canvas/ExamCanvas.tsx:20-21` `mode: "student" \| "author"`；但文件名不是 `ExamCanvasV2` | 能力存在，命名过时 |
| 4-5 | `authoringV2Patches.ts` 细粒度 patch 概念 | HOLDS | 575 行 | |
| 4-6 | `runtimeViewModelV2.ts` + `reading_source_v2.rs` 同源方向 | HOLDS | 两文件存在 | |
| 4-7 | `llm_gateway.rs` 超时/部分重试/Retry-After/平衡 JSON/兼容路由 | HOLDS | `llm_gateway.rs:121,251,339,357` | |
| 4-8 | `nas_package_v2.rs` staging/commit 原子机制 | HOLDS | `nas_package_v2.rs:5,68` | |

### §1.5 缺口矩阵（文档实际 18 行）

| ID | 断言（缺口/用户影响/根因） | 判定 | 证据 | 说明 |
|---|---|---|---|---|
| UX-001 | 入口与流程页面过多 | **FALSE** | `App.tsx:24-35` 仅 3 主表面；`AppShell.tsx:9-12` 仅 2 入口 | Phase 1 已修复；仍以 P0 缺口呈现属失真 |
| UX-002 | 固定 240/360/380/420px 列宽 + 主 surface `overflow:hidden` 导致溢出 | PARTIAL | 固定列宽仅存 `src/styles/legacy.css:60-61,685`（退休页）；`app-shell.css:87` 明确"不再 overflow:hidden"；`task_plan.md:92-94` 实测 72 项 0 溢出 | 根因与当前活跃表面不符 |
| UX-003 | 编辑/预览/题库详情不是同一界面 | PARTIAL | `src/features/editor/ExamWorkspacePage.tsx` 已统一编辑；但 `src/pages/UnifiedPreview.tsx`/`StructuredAuthoringEditorV2.tsx`/`ExamCanvasV2` 多 renderer 仍存 | 主表面已收敛，残留未删 |
| REC-001 | 新本地识别仍绕回 V1 candidate | HOLDS | `auto_pipeline.rs:187,208` | M4 未交付 |
| REC-002 | 题号/题干/选项以行序和字符串为主 | HOLDS | `ielts_grammar/mod.rs:135` 消费 `questionGroupCandidates` | |
| REC-003 | 复杂表格/流程图语义无法闭包 | HOLDS | 无 DocumentIRV2 直驱 QuestionBlock 证据 | M4 未交付 |
| CLD-001 | 云端只返回 outline | HOLDS | `llm_gateway.rs:484-486` | |
| CLD-002 | 云端队列在 React 页面 localStorage | PARTIAL | 主链已迁：`useImportFiles.ts:3`→`processingClient`、`libraryStore.ts:2,106 subscribeProcessing`；仅 `UnifiedPreview.tsx:24,138`（退休页）残留 | 主链已修复 |
| CLD-003 | Prompt/Skill 硬编码在 Rust | HOLDS | `llm_gateway.rs:484` 内联 prompt 字符串 | M5 pending |
| CLD-004 | 云端畸形 JSON 不能分组 salvage | HOLDS | `llm_gateway.rs` 仅 `balanced_json_end` 提取 | |
| LIB-001 | job.json/authoring JSON/legacy exams/library_items 多事实源 | HOLDS | `job_store.rs:33`、`library_commands.rs:291,338`、`db.rs:78-83` | 双写未除 |
| LIB-002 | 清理后仍保留大量 job 项目文件 | HOLDS | `cleanup.rs:38-46` | |
| BAT-001 | 批量导入串行后跳进第一题 | PARTIAL | `useImportFiles.ts:3` 已改后端入队（`importFiles`），非串行+跳转；但 UI 细节未静态确认 | 大幅缓解 |
| PUB-001 | 发布独立页面且门禁错误是长字符串 | PARTIAL | `src/pages/ExportPage.tsx` 仍存在（legacy）；批量发布已并入题库（`src/api/publishClient.ts`，工作树已改） | |
| SET-001 | 设置展示过多高级字段和不一致 Provider | **FALSE** | `SettingsPage.tsx:17-19` 注释"多 Profile 管理器…forceJson 复选框…temperature 输入"均已移除；`:30-31` 仅 OpenAI 兼容/Ollama；`:139 temperature: 0` | `task_plan.md:175-177` 已确认完整实现 §14 |
| TEST-001 | UI E2E 主要运行 Vite + dev fallback | PARTIAL | `package.json` 新增 `e2e:tauri`（`scripts/e2e/tauri-import-edit-publish.mjs`）；但 `e2e:library-workspace` 仍为浏览器+devFallback | 已部分补足，非"主要" |
| CODE-001 | 多个 100KB-500KB 单文件 | HOLDS | `authoring_pipeline.rs` 547KB、`lib.rs` 455KB、`quality.rs` 158KB、`mod.rs` 112KB、`parser.rs` 104KB | 上界被突破，见 A1-F09 |
| OBS-001 | 主界面暴露过多置信度/hash/source review/内部状态 | PARTIAL | 活动题库行不显示：`LibraryItemRow.tsx:3`"普通行不显示 hash…"；但退休页仍暴露：`DocumentReview.tsx:84,110,122,136` | 主表面已收敛 |

---

## 发现清单

### A1-F01 [P0] §1 冻结基线（`bb978be`）与实际 HEAD（`47a3806`+40 项未提交）不一致，全章"当前"描述系统性过时

- 证据：文档第 4、69 行声明 `bb978be`；`git rev-parse HEAD` = `47a38064aef58e023bce58e5b6feafa99031b252`；`git status --porcelain` 40 项（含 `src-tauri/src/processing/`、`src/exam-canvas/structureActions.ts`、`src/api/processingClient.ts`）
- 结论：§1 是 2026-09-04 对 `bb978be` 的快照，而 M0–M3 + 修复轮已推进 3 个提交并叠加未提交改动。§1.2/§1.5 的"当前判断"多已失效。
- 影响：任何按 §1 判断"当前状态"的后续任务（尤其 P10 旧链删除、UX 收敛）都会基于错误前提重复劳动或漏项。
- 建议：§1 重写为"审计时点 `bb978be` 的历史快照"并显式标注，另设"实施基线 `47a3806`+工作树"一节；或整体刷新至当前 HEAD。
- 层级：`static`

### A1-F02 [P1] §1.2 目标文件 `src/components/ExamCanvasV2.tsx` 不存在，实际已改名为 `src/exam-canvas/ExamCanvas.tsx`

- 证据：`ls src/components/ExamCanvasV2.tsx` → No such file；`src/exam-canvas/ExamCanvas.tsx`（475 行）；`task_plan.md:127` 记录"S3.2 改名迁移完成"
- 结论：§1.2 与 §1.4 均把 `ExamCanvasV2` 当作现存核心并作为"唯一 WYSIWYG 引擎"升级目标，路径已失效。
- 影响：按计划书直接定位文件会失败；后续 renderer 拆分（M3 剩余项）会误判基线。
- 建议：全文将 `ExamCanvasV2` 更正为 `src/exam-canvas/ExamCanvas.tsx`（含 §1.2 第 82 行、§1.4 第 138 行及 §16/§19 相关引用）。
- 层级：`static`

### A1-F03 [P1] §1.2 `App.tsx`「11 个页面路由分发」与 `styles.css`「约 64 KB」两处硬数据均已被推翻

- 证据：`App.tsx:24-35` 仅 3 主表面 + writing；`wc -c src/styles.css` = 582 字节、13 行 `@import`
- 结论：Phase 1（S1.2/S1.4）已完成页面与 CSS 收敛，计划书未回写。
- 影响：低估已完成的 P1 工作量；"CSS 64KB 全局耦合"是当前不存在的问题。
- 建议：更新为「App.tsx 分发 3 路由（+legacy writing）」与「styles.css 为 13 行分层入口，样式在 `src/styles/*.css`」。
- 层级：`static`

### A1-F04 [P1] §1.2 `router.ts`/`AppShell.tsx` 职责描述已被 Phase 1 推翻

- 证据：`router.ts:6` `RouteName = library|workspace|settings|legacy`；`AppShell.tsx:9-12` 仅 2 导航项、`:7` 注释列出已删除的 stepper/activeJob 条
- 结论："暴露内部流水线阶段""导航过深"均不成立于当前代码。
- 影响：§1.5 UX-001 的根因（历史 Phase 页面直接成为产品导航）已不成立，但矩阵仍标 P0。
- 建议：§1.2 这两行改为历史状态，或并入 A1-F01 的统一刷新。
- 层级：`static`

### A1-F05 [P1] §1.2 `devFallbackBackend.ts` 判断列过时：F13 已修复，测试替身已出生产 bundle

- 证据：`tauriCommands.ts:67-72` 与 `desktopDialogs.ts:216-219` 均以 `import.meta.env.DEV` 短路动态 import；`dist/assets/` 无 `devFallbackBackend` chunk（`grep -rl devFallbackBackend dist/` 为空）；修复轮 `progress.md:32` 记录修复
- 结论：源码文件仍在（3888 行）但已不在生产路径。§1.2 "从生产路径移除"这一"目标处理"实际已完成。
- 影响：把已完成项当作待办，重复劳动。
- 建议：标注为已修复（F13 关闭），保留"最小 fake adapter"作为后续 P10 收尾。
- 层级：`static`

### A1-F06 [P1] §1.5 缺口矩阵 6 条（UX-001/UX-002/SET-001/CLD-002/BAT-001/TEST-001）已在 M0–M3 修复或大幅缓解，仍以 P0/P1 当前缺口呈现

- 证据：见上表 UX-001（FALSE）、SET-001（FALSE）、UX-002/CLD-002/BAT-001/TEST-001（PARTIAL）；`task_plan.md:92-94,175-177`、修复轮 `progress.md:29-34`
- 结论：缺口矩阵是 `bb978be` 时点的历史结论，未随 M0–M3 更新，导致"当前 P0 缺口"严重高估。
- 影响：排期与资源分配失真；真实剩余缺口（REC/CLD/LIB，M4/M5）被淹没。
- 建议：矩阵增加"状态/关闭里程碑"列，逐条标注 已修复 / 部分 / 仍成立。
- 层级：`static`

### A1-F07 [P1] §1.5 UX-002 的根因（固定列宽 + 主 surface `overflow:hidden`）与当前代码不符

- 证据：固定列宽仅存在于 `src/styles/legacy.css:60-61,685`（退休页）；`src/styles/app-shell.css:87` 明确"内容区不再是大圆角卡片 + overflow:hidden"；`task_plan.md:92-94` 实测溢出矩阵 0 溢出、结论为"死 CSS 才是真实问题"
- 结论：活跃产品表面的溢出前提不成立，计划 §0.1 问题2 与 §1.5 UX-002 均基于旧 CSS。
- 影响：把"删除死 CSS + 建回归门"误判为"修复线上缺陷"，优先级与验收口径错误。
- 建议：UX-002 降级并改写根因；§0.1 问题2 同步修正。
- 层级：`static`

### A1-F08 [P1] §1.5 LIB-001（多事实源）经 M1 后**仍成立**，与 `task_plan.md` 的 M1「complete」口径存在张力

- 证据：`job_store.rs:33-35` 仍以 `job.json` 为事实源；`library_commands.rs:291` 元数据回写 JSON、`:338` 注释确认 `save_job` 触发 DB 双写；`db.rs:78-83` legacy `exams` 表仍在
- 结论：M1 建立了 `library_items_v2` 权威稿，但 job.json↔DB 双写与 legacy exams 双模型未除，LIB-001 未关闭。
- 影响：计划书把 LIB-001 列为"当前 P0 缺口"是准确的，但 `task_plan.md` 的 M1 complete 易被误读为数据面已收敛。
- 建议：明确 LIB-001 归属 M7/P10，不与 M1 完成混淆。
- 层级：`static`

### A1-F09 [P1] §1.5 CODE-001 低估：`authoring_pipeline.rs` 547 KB，突破计划自设的「100KB-500KB」上界

- 证据：`wc -c src-tauri/src/authoring_pipeline.rs` = 547,026 字节（13848 行）；`lib.rs` 455KB/11151 行；`ielts_grammar/quality.rs` 158KB
- 结论：CODE-001 的量化区间已不准确，且拆分成本被低估（`lib.rs` 含 136 测试 + 67 command）。
- 影响：M7「超大文件迁移」工作量估算偏低。
- 建议：更新区间为「100KB-550KB」，并把 `lib.rs` 测试外迁与 `authoring_pipeline` 冻结拆分分别排期。
- 层级：`static`

### A1-F10 [P2] §1.5 OBS-001 部分不成立：活动题库行已不暴露 hash/技术字段，仅退休页仍暴露置信度

- 证据：`src/features/library/LibraryItemRow.tsx:3` 注释"普通行不显示 hash、source path、schema、revision 或错误技术码"；`src/pages/DocumentReview.tsx:84,110,122,136` 仍显示 confidence/低置信块（退休页）
- 结论：OBS-001 对主表面已不成立，对 legacy 页仍成立。
- 影响：若按 OBS-001 再改活动 UI 属无效工作；正确动作是 P10 删退休页。
- 建议：OBS-001 改为"随 P10 删除 `DocumentReview` 一并消除"。
- 层级：`static`

### A1-F11 [P2] §1.3 `preview_commands.rs` 职责描述偏窄且与 Tauri 命令层脱节

- 证据：`preview_commands.rs` 144 行，仅 3 个 `pub(crate) fn`（`:20,37,72`），无 `#[tauri::command]`
- 结论：描述"生成另一个预览产物"不准确，实际含 validate/生成/e2e 三类 core。
- 影响：低；影响 §1.3 清单精度。
- 建议：更新为"authoring IR 校验 + 预览资源生成 + 预览 e2e core"。
- 层级：`static`

### A1-F12 [P2] §0.1 问题1/问题4 与 §1.5 UX-001/CLD-002 内部不一致

- 证据：§0.1 第 21 行称队列由 `UnifiedPreview.tsx` 管理，但主链已在 `useImportFiles.ts:3`；§0.1 问题1 与 §1.5 UX-001 同指"页面过多"，但 UX-001 已 FALSE
- 结论：§0 与 §1.5 是同一时点的重复表述，未随实现更新，形成同章内互证错误。
- 影响：读者可能据 §0 误判当前主链。
- 建议：§0 增加"问题清单为 `bb978be` 时点"注记。
- 层级：`static`

### A1-F13 [P2] §1.1 与 `task_plan.md` 的基线记录三者不一致（bb978be / 5daa04f / 47a3806）

- 证据：计划书第 4、69 行 `bb978be`；`task_plan.md:4` "实施基线 `bb978be` → 冻结基线 `5daa04f`"；实际 HEAD `47a3806`
- 结论：三个"基线"口径并存且无映射说明。
- 影响：追踪与验收口径混乱。
- 建议：在追踪文档中给出 commit 链 `bb978be → 5daa04f → 401ca76 → 47a3806(+worktree)` 与各自用途。
- 层级：`doc-only`

---

## 与历史审计的关系

### 新增（本轮相对 audit-2026-09-07 首次指出）

- A1-F01/F13：§1 冻结基线与实际 HEAD 脱节（历史报告只声明自身基线为 `47a3806`，未审计计划书基线不一致）。
- A1-F02：`ExamCanvasV2.tsx` 路径失效（历史报告 F11/F13 未覆盖命名漂移）。
- A1-F03/F04/F05：§1.2 前端清单硬数据（11 页面、64KB、AppShell/router 职责、devFallback 生产路径）过时。
- A1-F06/F07/F10：§1.5 缺口矩阵 6 条已修复/缓解仍标 P0/P1；UX-002 根因不成立。
- A1-F11：`preview_commands.rs` 职责描述偏差。

### 历史未修复（本轮确认仍成立）

- A1-F08 = LIB-001 多事实源：`job.json`↔DB 双写、legacy `exams` 仍在（历史 F02/F12 相关，未彻底关闭）。
- §1.5 REC-001/002/003、CLD-001/003/004、LIB-002：对应 audit-2026-09-07 的 M4/M5/M6 未交付结论，本轮静态复核一致。
- F11（结构修复未接入工作区）：`task_plan.md:30` 与修复轮 `progress.md` 称已接入；本轮确认 `src/exam-canvas/structureActions.ts`、`src/features/editor/SelectionInspector.tsx` 已新增（未提交），但 renderer 按题型拆分与 NAS parity 仍缺（M3 部分完成）。

### 已修复确认（本轮证伪"仍为缺口"的旧结论）

- F01（无法编译）：修复轮 `progress.md:24` 记录 `cargo check --locked` 通过；本轮未复跑构建（约束禁止），标注为 `doc-only` 佐证，未独立验证。
- F13（生产含测试替身）：`tauriCommands.ts:69`、`desktopDialogs.ts:218` 的 `import.meta.env.DEV` 守卫 + `dist/assets/` 无对应 chunk，**静态确认已修复**。
- F07/F08/F09（队列认领/源登记/恢复）：`src-tauri/src/processing/` 存在且 `task_plan.md:29` 记录修复，但本轮未跑 Rust 测试，标注 `static` 未运行验证。
- SET-001、UX-001：经 `SettingsPage.tsx`、`App.tsx`/`AppShell.tsx` 静态确认已落地。
- F10（批量发布非原子）：`nas_package_v2.rs:203` 出现 canonical_ds 直通路径；`task_plan.md:33` 记录整批原子 + 故障注入测试，本轮未运行验证。

---

## 证据层级与局限

- 本报告全部结论为 `static` / `doc-only` 层级：仅使用 `Read`/`Grep`/`Glob`/`git log|status|rev-parse`/`wc`/`node -e`，未运行 `cargo build`/`cargo test`/`npm run build`（遵守主线程统一构建约束）。
- **未验证项**：M2 后端队列（F07/F08/F09）的实际运行行为、F10 批量发布原子性、F01 编译恢复、`e2e:tauri` 真实链路——均只有修复轮文字记录或源码存在性，**不得计为产品行为通过**（遵循 `AGENTS.md` 证据分层规则）。
- `dist/assets/` 为历史构建产物，其"无 devFallback chunk"只能证明最近一次构建，不能替代当前工作树构建；但源码守卫 `import.meta.env.DEV` 是可静态确认的根因证据。
- 未做运行时验证：真实 Tauri import/edit/publish、NAS 学生端、SQLite 事务、100 PDF 语料与故障矩阵。
- 行号引用基于当前工作树文件内容（含未提交改动），若后续提交会漂移。
- 判定统计（本报告断言核对表逐行计数）：

| 判定 | 条数 | 分布 |
|---|---:|---|
| HOLDS | 67 | §0:3、§1.1:9、§1.2:13、§1.3:26、§1.4:7、§1.5:9 |
| PARTIAL | 13 | §0:2、§1.2:2、§1.3:1、§1.4:1、§1.5:7 |
| FALSE | 8 | §1.1:1、§1.2:5、§1.5:2 |
| UNVERIFIABLE | 0 | — |
| **合计** | **88** | — |

- 计数偏差说明：计划书 §1.2 表实际 20 行（任务书称 19）、§1.5 表实际 18 行（任务书称 19），本报告按实际行数逐条核对。§1.5 缺口矩阵"19 条"的标题口径与正文 18 行不符，属文档内部小瑕疵（`doc-only`，不单列发现）。
