# 阅读 Part 归属同步验收（2026-10-16）

已实现阅读题库、权威稿、编辑器、预览和 NAS 导出的 Part 同步。未提交 git，未修改 golden baseline、发布门禁或 NAS 仓库。

## 行为与实现决策

- 阅读 `exam.category` 为权威；题库选择仅接受 `P1/P2/P3`，`Part 1` 等值明确拒绝。
- 题库设置经 `apply_editor_commands_tx_with` 写 `setExamCategory`：同一 Immediate 事务完成版本 CAS、递增、journal、人工保护范围和题库标签更新。编辑器保存同样在 CAS 事务内投影 `part_label`，显式修改记 `manual`；这样列表和筛选无需在前端维护另一份稿件状态。
- 清除手动值立即重新自动判定。判不出时保留稿件原 category，标签仍投影该 category，来源记 `none`；不会清空成非法值。
- 编辑器标题旁的徽章/下拉读取 `editor.draft.exam.category`。下拉走既有 patch/inverse、undo/redo 和自动保存；云端校核时禁用，后端人工写入锁也继续生效。工作区副标题、学生预览和底部导航不再固定显示 Part 1。
- 内容变更通知继续使用 `processing://item-updated`。无处理任务行时使用 `versionKind=content` 和稿件版本号，前端按独立时钟去重，再进入既有 `remoteVersion/conflictRecovery`。列表、工作区和单行读取引起的懒对齐也会发通知。
- 存量 `manual` 阅读标签与稿件不一致时，在列出/打开/读取单行时懒对齐，走上述版本化写入。冲突或云端校核锁期间推迟，后续读取重试；对齐完成后重复读取不增版。常规列表只对可能不一致的行读取完整稿件。
- 导入共享 `part_detection` 的标题行→题号范围→文件名优先级；本地 V1 构造、本地 canonical V2 builder、云端直出 normalization 都已接入。无法判定才默认 P1。文件名不会被提升为原文标题证据。
- 听力 Part 1–4、写作 Task 1/2 继续仅修改题库标签列，本次没有改变其稿件结构。
- 修改 Part 不修改 `examId`；标题和源题干自带的 P1/Passage 1 文本也不自动改写。

## 产品端到端证据

运行：`node scripts/e2e/tauri-cdp-part-assignment-sync.mjs`，最终结果 **passed，10/10 assertions**。

[机器报告](F:/workspace/PDF2Test/artifacts/e2e-cdp/part-assignment-sync-2026-10-16T21-31-38-793Z/report.json)

真实当前源码 Tauri exe、Rust、SQLite 和文件导出，使用既有真实阅读导入稿的隔离副本。CDP 启动参数为 `--no-sandbox --disable-gpu`，`diagnosticRun=true`，这是诊断通道验证，不能表述为默认启动参数也已验证。

1. Tea：题库下拉改 P2 → 权威稿版本 1→2、category P2 → 点击打开，标题旁显示 P2。
2. 测试副本预先删除 Tea 的 processing row；打开编辑器后经真实 Tauri 题库命令改 P1→P2，编辑器都即时跟随版本通知。
3. 点击学生预览，真实预览画布出现，无编译错误；副标题 `READING · Part 2`，底部导航 `Part 2`。
4. 点击发布，真实 NAS 包结果 `published`（附 1 条待核对提示；没有强制发布或修改门禁）。runtime `meta.category=P2`，manifest 对应条目 `category=P2`，两者 `examId=p1-d35bb0b9`。
5. 编辑器改 P3 → 撤销回 P2 → 重做回 P3 → 返回题库，列表 P3、权威稿 P3，最终版本 5，examId 和标题均未变化。
6. The Pearls：另起隔离 profile，使用真实用户库的 SQLite 备份。`import-20261014104038-86fcf23e` 原始 `manual=P3/category=P1/version=5`；列出/打开后变为 `category=P3/version=6`。重复 list/get 仍版本 6，`examId=p1-86fcf23e`，标题保持原文。实际工作区徽章 P3。

截图：

- [编辑器 P2](F:/workspace/PDF2Test/artifacts/e2e-cdp/part-assignment-sync-2026-10-16T21-31-38-793Z/editor-P2.png)
- [学生预览 P2](F:/workspace/PDF2Test/artifacts/e2e-cdp/part-assignment-sync-2026-10-16T21-31-38-793Z/student-preview-P2.png)
- [返回题库 P3](F:/workspace/PDF2Test/artifacts/e2e-cdp/part-assignment-sync-2026-10-16T21-31-38-793Z/library-P3.png)
- [The Pearls 对齐后 P3](F:/workspace/PDF2Test/artifacts/e2e-cdp/part-assignment-sync-2026-10-16T21-31-38-793Z/pearls/Pearls-P3-aligned.png)

真实用户库只读查询并做 SQLite online backup 到 `F:\workspace\PDF2Test\artifacts\part-sync-user-db-backup.db`；未直接改写用户库。用户库的对齐将在运行更新后的应用、列出或打开条目时发生。

最终构建 `npm run build:app` 成功，输入哈希无漂移；exe SHA256 `4fd846bfe190c5ae1baf1a4f6d786905b3607e61afb14e8f35c143c386a7d8ca`。

## 命令层与前端组件证据

| 已运行命令 | 最终结果 | 日志 |
| --- | --- | --- |
| `npm run check` | 通过 | `artifacts/part-sync-check.log` |
| `npm test` | 55 files / 556 passed | `artifacts/part-sync-vitest.log` |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib library::` | 71 passed | `artifacts/part-sync-rust-library-final.log` |
| 同上，filter `authoring_v2` | 38 passed | `artifacts/part-sync-rust-authoring.log` |
| 同上，filter `export` | 30 passed / 1 ignored | `artifacts/part-sync-rust-export.log` |
| 同上，filter `cloud_direct::` | 32 passed / 2 ignored | `artifacts/part-sync-rust-cloud-direct.log` |
| 同上，filter `processing::scheduler::tests` | 21 passed | `artifacts/part-sync-rust-notifications.log` |
| 同上，filter `ielts_grammar::listening_draft_builder_tests` | 4 passed | `artifacts/part-sync-rust-modality.log` |
| 同上，filter `reading_import_initializes_category` | 1 passed | `artifacts/part-sync-rust-import.log` |
| 同上，filter `reading_direct_initializes_category` | 1 passed（也包含于 cloud_direct 套件） | `artifacts/part-sync-rust-direct-import.log` |
| `npm run build:app` | 通过 | `artifacts/part-sync-build-final.log` |
| `git diff --check` | 通过 | 无空白错误 |

Rust 新增覆盖：阅读同时更新稿件和题库、版本递增、旧版本拒绝、清除重新判定、无法判定保留原值、非法值拒绝、存量幂等、编辑器保存反向同步、云端锁与解锁后对齐、听力/写作不改稿件、导入初始化、文件名不可压过题号范围。前端覆盖徽章/下拉、云端锁、patch JSON 往返/inverse、撤销重做/自动保存、Part 导航以及无 processing row 的独立事件时钟。

首次探索性全量 `cargo test --lib` 未跑完：新增测试构造漏了 passage.sourceAnchors，出现 3 个测试夹具错误，已修复；随后长时间私有 PDF 语料测试仍运行，旧测试 exe 被 Windows 占用导致后续链接 LNK1104，因此停止该次测试进程，改为串行完成上述相关模块。**没有宣称全量 Rust 套件通过。** 中间源码仍在变化时的构建被输入漂移检查拒绝，最终采用冻结源码后的成功构建。未改测试期望或 golden baseline。

## examId 消费方审计与验证边界

PDF2Test：`reading_source_v2.rs:105` 和 `readingRuntimeV2.ts:672` 读取 `exam.category`；`export_artifacts.rs:90` 从 runtime `meta.category` 写 manifest；`export_nas_library.rs:546/1320` 也读取 meta.category。未发现从 examId 前缀推断 Part。

NAS 仓库未修改。已抽查：

- `server/src/lib/library/reading/NasJsDirectReadingAssetProvider.ts:143/218`：目录摘要读取 manifest.category；`:502`：详情读取 payload.meta.category。
- `server/src/lib/library/reading-suite-sessions.ts:100`：按 asset.category 的 P1/P2/P3 筛选组套。
- `server/src/lib/exam/ExamRuntimeFacade.ts:221`：按 category 计数；学生页/导航显示当前套题序号。
- provider 中 assetId 正则只用于 frequency 回退；`p1-low-30.js` 等名称属于身份/路径，不决定 Part。

证据边界：本轮实际验证了 Tauri 编辑器、内嵌学生预览、真实包导出和存量副本对齐。新导入的两条 category 初始化路径有 Rust 构造/normalization 回归；未重新调用付费云端识别服务。未启动外部 NAS 学生端重做目录筛选/作答，其消费逻辑为源码审计，加导出 manifest/runtime 字段核验。没有使用 CLI/schema-only 检查替代上述产品操作。

## 改动文件清单

PDF2Test 产品代码：

- `src-tauri/src/library/commands.rs`：设置/清除、懒对齐、读取投影及回归测试。
- `src-tauri/src/library/repository.rs`：同事务标签投影、人工保护和 journal 目标。
- `src-tauri/src/library/part_detection.rs`：共享证据提取、导入默认值及优先级测试。
- `src-tauri/src/authoring_v2_commands.rs`：Rust setExamCategory patch。
- `src-tauri/src/authoring_pipeline.rs`：本地 category 初始化和测试。
- `src-tauri/src/ielts_grammar/mod.rs`：本地 canonical V2 category 初始化。
- `src-tauri/src/cloud_direct.rs`：直出源证据与 category 初始化和测试。
- `src-tauri/src/lib.rs`：Part 操作与懒对齐的变更通知。
- `src-tauri/src/processing/scheduler.rs`：无 processing row 时的内容通知。
- `src/api/processingClient.ts`：内容/队列通知独立去重。
- `src/types/authoring-editor-v2.ts`：setExamCategory patch 类型。
- `src/services/authoringV2Patches.ts`：前端 patch/inverse。
- `src/features/editor/ExamWorkspacePage.tsx`：徽章、下拉、锁和副标题。
- `src/exam-canvas/ExamCanvas.tsx`：传递阅读 category。
- `src/exam-canvas/questionNavModel.ts`：导航真实 Part。
- `src/styles/workspace.css`：紧凑 Part 控件及标题布局。

前端测试及产品验收脚本：

- `src/api/processingClient.test.ts`
- `src/exam-canvas/questionNavModel.test.ts`
- `src/features/editor/ExamWorkspacePage.cloudReviewLock.test.tsx`
- `src/features/editor/useCanonicalEditor.undoLimit.test.tsx`
- `src/services/authoringV2Patches.part.test.ts`（新增）
- `scripts/e2e/tauri-cdp-part-assignment-sync.mjs`（新增）

交付记录：本报告，以及 `.planning/part-assignment-sync/{task_plan,findings,progress}.md`。原有未跟踪任务文档未改动。
