# Progress — 会话日志

## 2026-09-05 · Session 1 · 启动实施

### 建立追踪
- 用户要求依据 `Plan With Files/IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md` 开始实施，追踪文档放 `Plan With Files/Dual_Recognition/`。
- 采用 `planning-with-files` 工作流（多阶段、跨上下文的长任务）。
- 已建立 `task_plan.md` / `findings.md` / `progress.md`。
- 仓库根已有一套 `task_plan.md/findings.md/progress.md`（属于上一个 Phase 5 目标），**不改动**，本轮全部记录在 `Dual_Recognition/` 内，避免和历史目标混淆（计划 §18 P0-T01 明确要求不再以旧 task_plan 为唯一状态源）。

### 审计复核（写入 findings.md）
- 确认前后端模块清单、体积与计划 §1.2/§1.3 一致（`authoring_pipeline.rs` 547KB、`lib.rs` 449KB、`devFallbackBackend.ts` 173KB、`styles.css` 64,766B）。
- 确认 `RouteName` 有 15 个成员、`App.tsx` import 11 个页面组件。
- 确认 `AppShell` 的「转化工具」展开组 + stepper + job-strip。
- **修正计划一处结论**：三栏栅格在 `@media (max-width: 1180px)` 已塌陷为单栏，所以 1100px 最小窗口不是溢出现场；精算后真实溢出区间是 **1181–1469px，包含应用默认窗口 1440px**，`.settings-grid` 需要 1470px 视口才不溢出，而 `.surface { overflow:hidden }` 会直接裁掉第三栏。
- 补充发现 `.metric-row` 用 `repeat(6, 1fr)`（= `minmax(auto,1fr)`），受 min-content 地板约束，是第二类溢出源。
- 确认 `getAuthoringV2` / `applyAuthoringV2Patches` 命令已存在 -> P3 工作区可先接现有命令上线，不必等 P4 后端重构。

### 基线验证
- `npm run check`（`tsc --noEmit`）：见下方结果记录。

### Phase 0 交付（complete，S0.6 延后）

新增文件：
- `scripts/lib/cdp.mjs` —— 可复用的 Chrome/CDP/Vite 夹具（裸 CDP，零新依赖，沿用 `sidecars/ui-flow-e2e` 的 Chrome 发现顺序）
- `scripts/ui/layout-matrix.mjs` —— 横向溢出验收矩阵
- `scripts/verify-product-baseline.mjs` —— 产品面快照与漂移报告
- `fixtures/ui/long-content.json` —— 敌意内容压力夹具 + 视口矩阵定义
- `fixtures/ui/layout-matrix-baseline.json` —— 空 accepted map（目标 = 零溢出）
- `fixtures/product-baseline.json` —— routes 15 / pages 11 / commands 59 / tables 8

新增 npm scripts：`verify:product-baseline`、`verify:product-baseline:strict`、`test:layout`、`test:layout:legacy`。

验证结果：
- `npm run check`（tsc --noEmit）：**通过**
- `node scripts/ui/layout-matrix.mjs --include-legacy`：**72 项检查，0 溢出，退出码 0**
- `node scripts/verify-product-baseline.mjs`：**no drift**
- `cargo check`：后台运行中（`cargo 1.96.1` 可用）

关键结论（写入 findings.md F9）：**计划的溢出前提没有复现。**
计划点名的 `.settings-grid`/`.editor-grid`/`.llm-grid` 三栏栅格在代码里**没有任何页面使用**；
唯一活跃的三栏 `.review-grid` 需要 ≥1090px 而塌陷点是 ≤1180px，两者不重叠。
`* { box-sizing: border-box }` 也早已存在（计划列为待新增）。
=> P1 的 CSS 工作从「修缺陷」重定义为「删死代码 + 建回归门」。

探针迭代中修掉两类误报（记录以免重复）：
1. `scrollWidth` 判定会把 `.hero-panel::after` 装饰伪元素误报为内容裁切 -> 改为比较真实后代矩形，排除 absolute/fixed。
2. 探针自带的 `<input type="radio">` 继承 app 的 `input{width:100%}` 叠加 UA `margin-left:5px`，稳定误报 5px -> 改为纯文本选项行。
   顺带暴露真实脆弱点：`input,select,textarea{width:100%}` 没配 `max-width`/margin 归零。

S0.6 延后理由：本机未验证 tauri-driver/WebDriver 可用，不写无法运行的推测脚本。
替代方案在 P4 落地 `src-tauri/tests/product_chain.rs`，直接驱动真实命令 + 临时 AppData + 真实 SQLite。

### Phase 1-3 交付（complete）

#### 新增文件

```
src/app/legacyRoutes.tsx                     兼容期旧页面的唯一装配点
src/features/library/{LibraryPage,LibraryHeader,LibraryItemList,LibraryItemRow,LibraryBatchBar}.tsx
src/features/library/{libraryStore,libraryTypes}.ts
src/features/import/{ImportDrawer.tsx,useImportFiles.ts}
src/features/editor/{ExamWorkspacePage.tsx,useCanonicalEditor.ts,actionableIssues.ts}
src/features/settings/SettingsPage.tsx
src/exam-canvas/ExamCanvas.tsx               （从 src/components/ExamCanvasV2.tsx git mv）
src/exam-canvas/editorCommands.ts            EditorCommandV1 + 编译到现有 AuthoringPatchV2
src/exam-canvas/editors/InlineTextEditor.tsx
src/exam-canvas/renderers/MatchingMatrix.tsx
src/api/publishClient.ts
src/styles/{tokens,reset,app-shell,library,workspace,exam-canvas,settings,overlays,utilities,legacy}.css
scripts/e2e/library-workspace-smoke.mjs
```

#### 重写文件

- `src/app/router.ts`：`RouteName` 15 -> 4（`library` / `workspace` / `settings` / 不可导航的 `legacy`），
  新增 `legacyRedirect` + `applyLegacyRedirect`，旧链接原地 `location.replace` 并打日志。
- `src/app/App.tsx`：只分发三页 + legacy；删除全局 `listJobs()`、`activeJob`、`refreshToken` 全页重载。
- `src/components/AppShell.tsx`：两入口；删除转化工具展开组、stepper、job-strip、侧栏折叠态与 localStorage。
- `src/styles.css`：64,766 B -> 582 B，只剩 10 行分层 `@import`。

#### 关键实现决定

- **`legacy` 是第四个路由，这是对计划「三路由」的有意偏离。** 云端复核触发、视觉答案候选、
  LLM 题组建议、手工转录、写作创作的等价实现分别要等 P6-P8；在替代能力落地前删除入口
  会静默移除用户已有能力。这些页面移到 `#/legacy/...`（不在导航、打开时打印降级日志、带返回横幅），
  按计划 §20.2 在 P10 删除。
- **`EditorCommandV1.set_text` 编译到现有 `replaceText from 0 to <code point 数>`。**
  后端 `replace_text` 用 `chars().count()`（Unicode scalar），所以必须用 `Array.from(text).length`
  而不是 JS `.length`（UTF-16 code unit），否则含 emoji 的节点会报 `TEXT_RANGE_INVALID`。
  现有 `StructuredAuthoringEditorV2` 已经是这么做的，本轮沿用。
- **`expectedText` 在进入编辑那一刻快照**，不是提交时读当前值 —— 否则乐观并发校验是同义反复。
- **两阶段导入在前端实现**：阶段 A（建行）串行但极快，阶段 B（识别）固定并发池（本地 2、云端 2）。
  这不是计划 §5 的 Rust/SQLite durable queue，应用退出仍会中断；但相比 `UnifiedPreview` 的
  `localStorage queue + lease + window.__IELTS_CLOUD_REVIEW_WORKER__`，新路径没有任何跨页全局状态，
  P5 迁到 Rust 时只需把阶段 B 换成 enqueue 调用。
- **批量发布是逐条发布 + 汇总**，不是计划 §13.6 的整批 staging 一次提交（`nas_package_v2` 目前只支持单条）。
  已在 `publishClient.ts` 顶部注明，P9 收敛。

#### 顺带修掉的三个既有缺陷

1. **F13 题面选项行**：`input{width:100%}` 让选项 radio/checkbox 占 636.5px，文字 span 被挤到 14.66px。
   修复后 input 13px / span 623px。
2. **F14 `takeDevPickedPath` 丢内容**：预置文件的 `textContent`/`binaryContentBase64` 被丢弃，
   浏览器开发预览下预置文件永远无法真实解析。
3. **F15 `InlineTextEditor` 闭包陈旧**：输入与失焦同批更新时最后一次输入会被静默丢弃。

#### 验证结果

| 检查 | 结果 |
|---|---|
| `npm run check`（tsc --noEmit） | 通过 |
| `npx vite build` | 通过（142 modules，CSS 64.10 kB） |
| `cargo check` | 通过，exit 0（83 个既有 dead_code 警告，未改 Rust） |
| `node scripts/ui/layout-matrix.mjs --include-legacy` | **84 项检查，0 溢出** |
| `node scripts/e2e/library-workspace-smoke.mjs` | **13 步全过** |
| `node scripts/verify-product-baseline.mjs` | 已重新记录收敛后基线 |

产品面基线漂移（收敛前 -> 收敛后）：

```
routeNames        15 -> 4      （移除 13 个流水线阶段路由，新增 workspace + legacy）
App 装配的页面     11 -> 10     （LibraryPage 被 features/library 取代，成为孤儿，P10 删除）
Tauri 命令         59 -> 59     （本轮未动后端）
SQLite 表           8 -> 8      （本轮未动数据库）
src/app/App.tsx           -61.1%
src/components/AppShell   -73.7%
src/styles.css            -99.1%
新增 src/features 13 文件 · src/exam-canvas 4 文件 · src/styles 10 文件
```

#### 冒烟覆盖层级声明（按 AGENTS.md）

`library-workspace-smoke` 是 **浏览器 + devFallbackBackend** 级验证，
证明路由收敛、ImportDrawer、题库行、工作区、原位编辑保存链在 UI 层是通的。
它**不能**证明真实 Tauri/SQLite/文件系统/NAS 链路。
由于 devFallbackBackend 不为真实导入的 job 生成 V2 题稿（findings F16），
导入链与编辑链在浏览器下必须分两段验证，无法连成一条。
真实 Tauri 端到端（导入 -> 打开 -> 改一个字符 -> 保存 -> 发布）仍是待办，见 task_plan 的 S0.6 与 P4。

---

# Session 3 — M0 真实验收基础（2026-09-05）

## 交付物

1. **真实 Tauri E2E**（M0-T3 / 原 P0-T02）：`scripts/e2e/tauri-import-edit-publish.mjs` + `npm run e2e:tauri`。
   tauri-driver + selenium-webdriver 驱动真实 `ielts-author-studio.exe`（tauri build --debug --no-bundle），
   隔离 `PDF2TEST_AUTOMATION_DATA_DIR` + `WEBVIEW2_USER_DATA_FOLDER`，复用 `PDF2TEST_AUTOMATION_PDF_DIR/EXPORT_DIR`
   免对话框钩子。结果：6 步 PASS + 发布步 BLOCKED（质量门），verdict `passed-with-publish-gate-blocked`，
   report.json + 截图落 `artifacts/e2e-tauri/run-*/`（`--keep` 保留）。
   覆盖层级：**真实进程 + WebView2 + SQLite + 文件系统**；与浏览器冒烟、product_chain 分层报告。
2. **AUTHORING_V2_NOT_AVAILABLE 复现结论**（M0-T4）：完整流水线后不复现；处理中打开会暴露内部错误码
   （F-M0-3，排 M2/M3）。product_chain 4 测试继续全绿（命令级证据）。
3. **基线固化**（M0-T2 / 原 P0-T01）：commitSha + schemaHash + corpus 入 `fixtures/product-baseline.json`
   （含 changeLog）；漂移重录强制 `--reason`。strict 门禁绿。
4. **跨仓 NAS 契约入口**（M0-T5）：`src-tauri/src/product_chain.rs::dump_published_package_for_nas_contract`
   (--ignored) 落真实发布产物到 `artifacts/nas-contract-fixture/`；`scripts/e2e/nas-student-contract.mjs`
   按学生端 manifest 规则镜像校验 13/13 PASS；Electron 真实加载实测如实标 pending 到 M6
   （学生端仓库 `F:/workspace/IELTS-NASfor-WenDao` 已确认，node_modules 未装）。
5. **phase7 门禁 peer 路径修正**：`../NAS` → `NAS_PEER_REPO` env → `../IELTS-NASfor-WenDao` 解析链；
   门禁由「缺目录」变为真实哈希漂移（不改绿，独立决策）。
6. **追踪口径修正**（M0-T1）：task_plan 增加 M0-M7 里程碑表；S0.6 旧前提作废并记录落地证据；
   S3.1/S3.2 与设置页状态按审计实况改写；Errors 表更新 AUTHORING_V2_NOT_AVAILABLE 结论。

## 实测记录

- `npm run e2e:tauri` → PASSED×6 + BLOCKED×1（run-2026-09-05T10-03-41-651Z）
- `npm run verify:product-baseline:strict` → 无漂移（含新字段）
- `cargo test product_chain` → 4 passed
- `node scripts/e2e/nas-student-contract.mjs` → 13/13 PASS
- `npm run verify:phase7:listening-contract` → 仍红：`ListeningExamSourceV1 contract hash is stale`
  （真实漂移，peer 文件已全部可达）
- 测试污染清理：2 job 目录 + DB 2 行软删除完成，真实 AppData 复原

## 覆盖层级声明（按 AGENTS.md）

- 真实产品端到端：`e2e:tauri`（本轮新增，导入→编辑→持久化真实链路已证；发布链由 product_chain 命令级证明，
  UI 级发布成功路径待 M1 typed preflight + ready 语料后补）
- Rust 命令级：`product_chain.rs` 4 测试
- 浏览器级：`e2e:library-workspace`（devFallback，继续如实标注）
- NAS 学生端：manifest 契约镜像校验已通；Electron 实测 pending（M6）
