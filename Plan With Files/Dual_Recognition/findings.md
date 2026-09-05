# Findings — 代码级审计（对照计划正文逐条验证）

基线 commit：`bb978be`  ·  验证日期：2026-09-05  ·  验证方式：本地源码读取 + 体积统计

## F1. 计划第 1.2/1.3 节的模块清单与实际仓库一致

前端实际文件与体积（`src/`，字节）：

| 文件 | 体积 | 计划判断 | 复核 |
|---|---:|---|---|
| `services/devFallbackBackend.ts` | 173,269 | 生产路径应移除 | 确认：`api/tauriCommands.ts` 中 `command()` 在非 Tauri 环境无条件回退 `devFallbackInvoke` |
| `pages/UnifiedPreview.tsx` | 66,337 | 过度集中 | 确认 |
| `styles.css` | 64,766 | 单文件全局样式 | 确认（计划写“约 64 KB”，精确 64,766 B） |
| `pages/StructuredAuthoringEditorV2.tsx` | 54,721 | 功能完整但复杂 | 确认 |
| `services/authoringV2Patches.ts` | 33,857 | 可复用基础 | 确认 |
| `pages/ExportPage.tsx` | 25,860 | 独立发布页 | 确认 |
| `components/ExamCanvasV2.tsx` | 25,442 | 最值得保留的核心 | 确认 |
| `editor/authoringTiptap.tsx` | 23,573 | 与 Canvas 重复 | 确认 |

后端实际文件与体积（`src-tauri/src/`，字节）：

| 文件 | 体积 | 复核 |
|---|---:|---|
| `authoring_pipeline.rs` | 547,026 | 确认为最大单文件（计划称“超大 V1 启发式链”） |
| `lib.rs` | 449,181 | 确认（命令薄壳 + 巨量测试） |
| `pdf_geometry.rs` | 179,438 | 与 `pdf_facts_shadow.rs`(157,352) 并存 -> 计划 §17.3 要求合并 PDF 解析入口，成立 |
| `ielts_grammar/quality.rs` | 157,234 | 确认 |
| `auto_pipeline.rs` | 116,127 | 确认 |
| `ielts_grammar/mod.rs` | 112,627 | 确认（计划 §17.7 要求拆分，成立） |

## F2. 路由与页面数量：实测 15 个 RouteName

`src/app/router.ts` 的 `RouteName` 联合类型实际有 **15** 个成员：
`dashboard, jobs, new, document, split, groups, llm-review, preview, authoring-v2, phase5, export, writing, library, libraryExam, settings`。

计划 §1.2 写“11 个页面的路由分发”——`App.tsx` 实际 import 了 **11 个页面组件**（Dashboard/DocumentReview/ImportWizard/JobList/Settings/UnifiedPreview/ExportPage/WritingStudio/LibraryPage/LibraryExamDetail/StructuredAuthoringEditorV2），而路由名有 15 个（多个路由名共用 `UnifiedPreview`）。两个数字都对，指的是不同东西。

**收敛目标**（计划 §3.1）：`library` / `workspace` / `settings` = 3。

## F3. AppShell 确实把流水线阶段当成产品导航

`src/components/AppShell.tsx` 现状：
- `navEntries` 有 4 个一级项，其中「转化工具」是含 5 个子项的展开组（导题任务 / 新建导题 / 写作题创作 / 结构化编辑器（Phase 5）/ NAS 导出）。
- `steps` 常量把 `document -> preview -> export` 渲染成常驻 stepper。
- `job-strip` 直接向用户显示 `activeJob.category`、`frequency`、`issueCounts.errors 个错误`。
- 导航展开态写 `localStorage`（`ielts-author-studio.nav.expanded.*`）。

对应计划 §16.3 的删除清单，全部成立。

## F4. CSS 溢出根因逐条验证（计划 §10.1）

`src/styles.css` 实测：

| 行号 | 选择器与值 | 计划描述 | 复核 |
|---:|---|---|---|
| 112 | `.review-grid { grid-template-columns: 360px minmax(0,1fr) 380px; }` | 一致 | 确认 |
| 113 | `.editor-grid { 240px minmax(0,1fr) 420px }` | 一致 | 确认 |
| 114 | `.llm-grid { minmax(0,1fr) minmax(0,1fr) 320px }` | 一致 | 确认 |
| 115 | `.settings-grid { minmax(460px,1.35fr) minmax(320px,.9fr) 340px }` | 一致 | 确认 |
| 105 | `.metric-row { repeat(6, 1fr) }` | 一致 | 确认 |
| 87 | `.surface { ... overflow: hidden; }` | 一致 | 确认（且 `min-height: calc(100vh - 132px)`） |
| 29 | `.shell { grid-template-columns: 224px minmax(0,1fr) }` | 侧栏固定 224px | 补充发现 |
| 417 | 单栏塌陷断点在 `@media` 内 | — | **需查断点值：塌陷点与 Tauri `minWidth: 1100` 的关系是溢出关键** |

`src-tauri/tauri.conf.json`：`width: 1440`，**`minWidth: 1100`**。计划 §26.4-C 说“1100px 最小宽度与 1180px breakpoint 冲突”——需实测断点值确认（见 F4a）。

### F4a 断点实测 —— 修正计划的一处结论

媒体查询实测：`@media (max-width: 1180px)` 把 `.review-grid/.editor-grid/.llm-grid/.settings-grid/...` 全部塌陷为 `1fr`（styles.css:409,417）。
所以 **在 1100px 最小窗口下这些三栏并不生效**，计划 §10.1/§26.4-C 关于“1100px 无法容纳三栏”的表述不精确。

真实溢出窗口通过布局链精算得到：

```
viewport W
 -> .shell           侧栏 224px（styles.css:29）
 -> .workspace       padding 24px x2（:75）
 -> .surface         padding 20px x2 + border 1px x2（:87）
可用内容宽度 = W - 224 - 48 - 40 - 2 = W - 314
```

`.settings-grid` 最小内容宽度 = `460 + 320 + 340 + 2 x gap(18)` = **1156px**（:110,:115）
=> 需要 `W >= 1470px`。而塌陷点是 `W <= 1180px`。

**因此真实溢出区间是 1181px <= W <= 1469px —— 其中包含应用自己的默认窗口宽度 1440px**
（`src-tauri/tauri.conf.json: width 1440, minWidth 1100`）。
`.surface { overflow: hidden }` 会把第三栏直接裁掉，用户在默认窗口打开设置页就会丢内容。

这比计划描述的问题更严重也更容易复现，`scripts/ui/layout-matrix.mjs` 必须把 **1440x960** 和 **1280x800** 作为必测视口。

其余栅格精算（`W>=` 为不溢出所需视口宽度）：

| 选择器 | 固定列合计 + gap | 需要视口 | 1181-1469 区间是否溢出 |
|---|---:|---:|---|
| `.settings-grid` | 1156 | >= 1470 | **是** |
| `.review-grid` | 776 | >= 1090 | 否 |
| `.editor-grid` | 696 | >= 1010 | 否 |
| `.preview-two-pane` | 798 | >= 1112 | 否 |
| `.llm-grid` | 356 | >= 670 | 否 |
| `.metric-row` | `repeat(6,1fr)` = `minmax(auto,1fr)`，受 min-content 地板约束 | 取决于文案长度 | 长标签时是 |

`.metric-row` 用 `1fr`（等价 `minmax(auto,1fr)`）而不是 `minmax(0,1fr)`，6 列在长中文标签下会被 min-content 顶开——这是第二类溢出源，全局硬规则里必须把 `1fr` 改为 `minmax(0,1fr)`。

## F5. 双事实源与前端云端队列

- `src/api/tauriCommands.ts` 无 processing 队列命令；也没有 `listen()` 订阅。前端没有统一的后台任务事件源。
- `LibraryPage.tsx` 通过 `listLibraryExams / searchLibraryExams / getLibraryStats / listTrashedExams` 拉取，**每次筛选变化都重新拉四个接口**，没有增量更新通道。
- `LibraryPage.tsx` 的「新建导题任务」按钮 `go("/jobs/new")` 直接跳到 `ImportWizard` 页面 -> 计划 §2.1「导入应在题库内完成」尚未成立。
- `types/library.ts` 的 `LibraryStatus` 只有 `draft/needs_review/ready/exported`，**没有 processing / failed / action_required**，因此当前题库行**无法表达导入任务阶段**。这正是计划 §11.3 要新增 `LibraryItemStatusV2` 的原因。

## F6. 现有可复用资产（计划 §1.4 确认）

- `src/services/runtimeViewModelV2.ts`（同源 runtime 投影）存在。
- `src/services/authoringV2Patches.ts`（细粒度 patch）存在。
- `src/types/{ielts-authoring-v2,content-doc-v2,document-ir-v2,runtime-view-model-v2}.ts` 语义模型齐备。
- 后端 `reading_source_v2.rs`(41,852) / `reading_runtime_v2.rs`(16,240) / `nas_package_v2.rs`(79,984) 存在。
- `getAuthoringV2(jobId)` / `applyAuthoringV2Patches(input)` 命令已存在 -> **P3 工作区可以先接这两个命令上线，不必等 P4 后端重构**。

## F7. Feature flag 现状：新链路已默认开启

`src/config/featureFlags.ts`：`documentIrV2/authoringV2/runtimeSourceV2/nasPackageV2` = true，`qualityGateV2` = true，`authoringEditorV2` = true，`listeningV1` = false，`pdfPerQuestionLlmRepair` 强制 false（`resolve*` 三个函数都硬写 false，不可被 override）。

结论与计划 §1.1 一致：**问题不是开关未启用，而是新旧链路并存、产品表面未收敛**。

## F8. 计划中需要按实际情况调整的点

1. **`isPhase5EditorEnabled()` 直接 `return true`**（`featureFlags.ts` 末尾），不读任何 flag。计划把它当作 flag 处理；实际它已是死开关，收敛路由时应连同 `phase5` fixture 路由一起清理。
2. **`LibraryStatus` 需要扩展而非替换**：现有 `draft/needs_review/ready/exported` 已被 Rust 端 `library_items` 使用，P2 前端先做 UI 层状态映射（processing/action_required/failed 由 processing job 提供），避免在 P2 就动数据库。
3. **计划 §16.1 要求删除全局 `listJobs()`**，但 `AppShell` 的 `activeJob` 依赖它；两者必须同一次改动，否则 topbar 标题会空。
4. **`npm run check` 只跑 `tsc --noEmit`**，没有 lint。收敛期靠 `tsc` 兜底，需要额外的溢出/结构断言脚本（S0.3/S0.5）。

---

## F9. 溢出问题实测结论 —— 与计划 UX-002 不一致（重要）

新增 `scripts/ui/layout-matrix.mjs`（真实 Chrome + CDP，无新依赖）跑完矩阵：
**6 视口 × 6 路由 × {正常, 敌意内容} = 72 项检查，横向溢出 0 项。**

矩阵覆盖：1100×760、1280×800、1440×960、1536×864@1.25×、1280×720@1.5×、2560×1440。
路由覆盖：`/library`、`/settings`、`/dashboard`、`/jobs`、`/jobs/new`、`/export`。
敌意内容：180+ 字符文件名、500+ 字符错误文本、20 个标签、12 个 A-L 长选项、10 列表格、超长 URL、中英混排。

### 为什么和计划结论不同

1. **计划点名的固定三栏栅格大部分是死代码。** 实测 `grep`：
   - `.settings-grid` —— **无任何页面使用**（Settings.tsx 用 `.settings-studio` = `300px minmax(0,1fr)`）
   - `.editor-grid` —— **无任何页面使用**
   - `.llm-grid` —— **无任何页面使用**
   - `.review-grid` —— 仅 `DocumentReview.tsx:97` 使用（唯一活跃三栏）
   所以“`.settings-grid` 在 1440px 裁掉第三栏”这一条我在 F4a 里的推算**不成立**：那条 CSS 根本没有被渲染。F4a 的布局链数学是对的，结论作废。

2. **`@media (max-width: 1180px)` 已把所有三栏塌陷为单栏**，而 Tauri `minWidth: 1100`。`.review-grid` 需要 ≥1090px 视口，塌陷点是 ≤1180px —— 两者不重叠，因此在受支持窗口范围内不会溢出。

3. **`* { box-sizing: border-box }` 已在 `styles.css:19` 存在**，计划 §10.3 把它列为待新增的硬规则；实际已有。

### 探针本身踩过的两个坑（已修，记录以免重复）

- 早期用 `el.scrollWidth > el.clientWidth` 判定裁切，把 `.hero-panel::after` 这类**装饰性伪元素**误报为内容被裁。改为遍历真实后代元素矩形与容器内容框比较，并排除 `position: absolute/fixed`。
- 探针自己的 `<input type="radio">` 继承了 app 的 `input { width: 100% }`（styles.css:191）叠加 UA 默认 `margin-left: 5px`，稳定产生 5px 溢出。这是探针 markup 的人工产物，不是产品缺陷；已改为纯文本选项行。
  **但这暴露一个真实脆弱点**：`input, select, textarea { width: 100% }` 没有配 `max-width`/margin 归零，任何带 UA 默认外边距的控件都会顶出容器。新样式层必须处理。

### 对 Phase 1 的影响

CSS 工作**不再是修复线上可复现缺陷**，改为三项仍然有价值的工作：
1. 删除死 CSS（`.settings-grid/.editor-grid/.llm-grid/.split-grid/.wizard-grid` 等），减少 64KB 单文件的耦合面。
2. 建立硬规则与 `src/styles/*` 分层，**防止**新建的 Library/Workspace 表面引入溢出（`layout-matrix` 从“缺陷复现器”转为“回归门”）。
3. 把 `.surface { overflow: hidden }` 换成不裁切内容的方案，并给宽表格/长 URL 明确策略。

### 尚未覆盖的溢出面（诚实声明）

`layout-matrix` 需要真实 job id 才能测 `/jobs/:id/document`（唯一活跃三栏 `.review-grid`）、`/jobs/:id/preview`、`/jobs/:id/authoring-v2`。
已加 `--job-id` / `--item-id` 参数，但**本轮没有真实 job 数据可测**，这三条路由的溢出状态未知。
另外全部测量运行在浏览器 + `devFallbackBackend`，不是真实 Tauri WebView，内容密度可能低于真实数据。

## F10. 产品面基线快照（`fixtures/product-baseline.json`）

`scripts/verify-product-baseline.mjs` 记录：**routes 15 · App 页面 11 · Tauri 命令 59 · 前端命令调用 56 · SQLite 表 8**。

SQLite 表实测：`exams`, `ingest_jobs`, `job_artifact_index`, `library_item_revisions`, `library_items`, `library_meta`, `publish_records`, `source_assets`。

`exams`（legacy）与 `library_items` + `library_item_revisions` 并存 —— **计划 LIB-001「多事实源」在数据库层面得到确认**。

## F11. 环境能力

- `cargo 1.96.1` 可用 -> P4+ 后端阶段本机可验证。
- Chrome 可用（`C:\Program Files\Google\Chrome\Application\chrome.exe`）-> 浏览器级 UI 验证可跑。
- 无 `src-tauri/tests/` 目录；`lib.rs` 内含 136 个 `#[test]`。
- **无 puppeteer/playwright 依赖**，UI 自动化沿用 `sidecars/ui-flow-e2e` 的裸 CDP 方案（已抽出 `scripts/lib/cdp.mjs` 复用）。
- 真实 Tauri GUI 驱动（tauri-driver/WebDriver）**本机未验证可用**，S0.6 因此延后（见 task_plan）。

---

## F12. library item id == job id（关键，影响所有新 API 形状）

`src-tauri/src/library_commands.rs` 的 `exam_record_from_reading_job` / `exam_record_from_writing_job` 都用
`ExamRecord.id = job.job_id`，`upsert_library_item_from_exam` 再以同一 id 写入 `library_items`。

因此：

- `/items/:itemId` 里的 `itemId` 可以直接当 job id 用；
- `getAuthoringV2(itemId)`、`applyAuthoringV2Patches({ jobId: itemId })`、`exportAuthoringV2({ jobId: itemId })` 全部可直接透传；
- 旧链接 `/jobs/:id/*` -> `/items/:id` 和 `/library/:id` -> `/items/:id` 都是 1:1 映射，不需要查表；
- 计划 §4.3 的 `library_items_v2` 新表在 P4 建立时，应保留这个 id 关系或明确写迁移映射，否则所有前端路由都要改。

这也是 P3 工作区能在 P4 后端重构之前先上线的原因：不需要新命令。

## F13. 题面选项行渲染缺陷（本轮修复）

作者/学生题面的选项行本来就是坏的，与本轮改动无关：

```
legacy styles.css:191   input, select, textarea { width: 100% }
legacy styles.css:1146  .exam-canvas-v2 .v2-choice-item input { flex: 0 0 auto; margin-top: 3px }
```

`.v2-choice-item` 是 flex 行，`input` 拿到 `width: 100%` 且 `flex: 0 0 auto`（basis 取 width），
实测占据 **636.5px**，把选项文字的 `<span>` 挤到 **14.66px**。

改动前的表现是文字横向溢出后被 `.surface { overflow: hidden }` 裁掉；
本轮加了 `overflow-wrap: anywhere` 硬规则后，同一个缺陷变成「每行一个字符的竖排文字」——
更显眼，但根因相同。

修复（`src/styles/reset.css` + `src/styles/exam-canvas.css`）：

```css
input[type="radio"], input[type="checkbox"] { width: auto; flex: 0 0 auto; }
.exam-canvas-v2 .v2-choice-item { display: flex; align-items: flex-start; gap: 8px; min-width: 0; }
.exam-canvas-v2 .v2-choice-item > span { flex: 1 1 auto; min-width: 0; }
```

修复后实测：input 13px、span 623px，选项文字正常单行显示。

## F14. `takeDevPickedPath` 丢弃预置文件内容（本轮修复）

`src/api/desktopDialogs.ts` 的 `takeDevPickedPath()` 接受 `Array<string | Partial<PickedPath>>`，
但构造返回值时**没有复制 `textContent` / `binaryContentBase64`**。
结果：浏览器开发预览里预置的文件永远拿不到真实内容，`devFallbackBackend.makeDocumentIr` 直接抛
「没有拿到真实解析结果」。这使得任何基于预置文件的浏览器级 E2E 都无法跑通导入链。

已修复为透传这两个字段与 `requiresDesktopParser`。

## F15. `InlineTextEditor` 提交时的闭包陈旧风险（本轮修复）

`onBlur={commit}` 里的 `commit` 闭包读 `draft` state。当最后一次输入与失焦落在同一批 React 更新里
（快速输入后立刻点走），闭包里的 `draft` 还是上一次渲染的旧值，最后一次输入会被静默丢弃。
已改为从 DOM 读实时值：`onBlur={(event) => commit(event.currentTarget.value)}`，Enter 路径同理。

## F16. devFallbackBackend 不为真实导入生成 V2 题稿（环境限制，不是产品缺陷）

`store.authoringV2[jobId]` 只在两处被写入：`phase5-editor-fixture` 按需造夹具，
以及 `apply_authoring_v2_patches`。所以浏览器开发预览下，真实导入的 job 调用 `get_authoring_v2`
必然返回 `AUTHORING_V2_NOT_AVAILABLE`。

影响：**浏览器级 E2E 无法把「导入」与「编辑」连成一条链**，只能分两段验证（已按此实现）。
真实 Tauri 下 `auto_pipeline` 会写 V2 shadow（`authoringV2Shadow` 默认开启），
`ImportWizard` 原本也是 `try { getAuthoringV2 } catch` 后才降级，说明真实链路通常有 V2。

工作区已对这种情况做降级：显示人话说明 + 「运行本地识别」按钮 + 兼容页入口，不白屏、不抛错。

## F17. `src/pages/LibraryPage.tsx` 已成孤儿

新题库在 `src/features/library/LibraryPage.tsx`，旧 `src/pages/LibraryPage.tsx` 不再被任何路由引用
（`legacyRoutes.tsx` 没有收录它，因为它的能力已被完全取代）。
它仍参与 `tsc`，但不进入 bundle。按计划 §20.2 在 P10 统一删除，此处记录以免遗漏。

---

# Session 2 — 后端审计与真实链路

12 个并行 reader + 1 个 completeness critic 的完整输出见
[backend-map/summary.md](backend-map/summary.md)（588KB，含逐子系统签名/改动点/测试方式）
与 [backend-map/critic.md](backend-map/critic.md)（17 步实施顺序）。以下只记录**改变了本轮决策**的发现。

## F18. Provider 不一致比计划描述更深（不只是 UI）

`src/types/settings.ts:2` 允许 4 个 provider；`src-tauri/src/llm_gateway.rs:146` 只路由
`OpenAiCompatible | Ollama`。计划 §7.14 说这是 UI 问题；实际 `llm_commands.rs:78-83` 的**写入路径**
也接受不受支持的值，所以坏配置会被持久化。新设置页只暴露两个真实协议（本轮已做），
但后端校验收敛仍待 P9。

## F19. `ingest_jobs` 表已存在，形状几乎就是 ProcessingJob

`db.rs` 的 `SCHEMA_SQL` 里有一张**没有任何生产代码使用**的 `ingest_jobs` 表，字段与计划 §4.3 的
`processing_jobs_v2` 高度重合，但缺 lease / worker_id / attempt_count / stage。
=> P5 建队列时应评估扩展它而不是新建第五张表。

## F20. 没有 migration 机制，只有一个布尔标记

`ensure_schema` 在**每次** `open_connection` 时无条件 `execute_batch(SCHEMA_SQL)`，全是
`CREATE TABLE IF NOT EXISTS`。全仓没有 `PRAGMA user_version`、没有 migrations 表、
**没有任何 `ALTER TABLE`**。唯一的迁移标记是 `library_meta` 里的布尔键 `migration_done_v1`。

后果：**给已发布的表加一列没有受支持的路径。** 计划 §11.2-M2 说的 `migration_version` 不存在。
另外计划正文 §4.3/§4.4 的 DDL 用的是裸 `CREATE TABLE`，直接粘贴会在第二次 `ensure_schema` 抛错。

## F21. `migrate_existing_into_library` 只写 legacy `exams`

计划 §11.2-M2 假设迁移会写 `library_items_v2`。实际它**只写 `exams`**，从不写 `library_items`、
不写 revision。而且 `library_item_revisions.payload_json` 是 `exams.payload_json` 的逐字节副本 ——
**数据库里根本没有 canonical V2 内容**，V2 只存在于 `jobs/<id>/authoring/revisions/*`。

## F22. 我在 Session 1 交付的题库页仍依赖三个读源

`src/features/library/libraryStore.ts` 同时调 `listJobs()`（jobs/ 目录扫描）、`listLibraryExams()`、
`listTrashedExams()`，并且优先取 job 的 title/updatedAt/stage。原因是数据库**答不出页面要的东西**：
`LibraryExamSummary` 没有 `currentStep`，`status_from_reading` 丢掉了 `issue_counts.needs_review`。
=> 先给 DB 补这两项，才能把 `listJobs()` 摘掉。这是 P4 的第一个真实约束。

## F23. 编辑会让一个可发布的题目永久不可发布（本轮修复，严重）

两个互相独立的原因，任一都足以让计划的核心流程「导入 -> 编辑 -> 发布」走不通：

### F23a `mark_user_audit` 每次保存都把 `humanVerified` 写成 false

`authoring_v2_commands.rs` 的 `mark_user_audit` 无条件 `audit.humanVerified = false`，
而导出门禁要求它为 `true`（否则 `authoring_v2_export_blocked:human_verification_required`）。
V1 在 `authoring_review::refresh_authoring_review_state` 里**派生**这个标记；
**V2 没有任何路径把它写回 true**。所以它单调为假：第一次编辑之后永久无法发布。

这不是在保护学生，而是逼作者「要么发布未编辑的稿，要么别发布」。

修复：保存不再下调已经为 true 的值（保留 revision/source/updatedAt 的更新）。
已验证过的稿编辑后仍可发布；从未验证过的稿仍然被拦住。
内容安全仍由门禁的其余部分保证 —— 未解决的 blocker、未解决答案、quality ready、
compiler 通过、资源闭包，全部按**当前** DS 重算。

### F23b `evaluate_quality` 把「有任何 issue」当作不可 ready

`ielts_grammar/quality.rs` 的状态计算用 `!issues.is_empty()`。后果：

- 一条 `info` 级说明就能让一份完美的卷子永远无法发布；
- **解决/忽略一个 issue 完全没有作用** —— 而 `preserve_issue_resolutions` 每次保存都小心地把
  `resolution: resolved|ignored` 传递下去，却没有任何代码读它。这是代码内部自相矛盾。

同时导出门禁自己的 `unresolved_blockers` 判定**已经**是 severity + resolution 感知的
（`severity == Blocking` 且 resolution 不在 {resolved, ignored}）。两处判定不一致。

修复：状态计算改用与导出门禁完全相同的判定。计划 §8.5 定义 severity 只有 blocker/warning，
§6.8 明确列出「直接阻止 Ready」的是具体 blocker 清单，§13.3 要求门禁基于当前可执行问题
而不是历史痕迹 —— 所以这是实现计划，不是放宽门禁。

三个测试锁定新契约（`product_chain.rs`）：编辑后仍可发布、`humanVerified` 不被下调、
info/warning 不阻止 ready。

## F24. `stream_data_start` 对单字节 EOL 永远返回 None（本轮修复）

`pdf_facts_shadow.rs` 的 `stream_data_start` 只用 `bytes.get(start..start+2)` 匹配，
而 `Some(b"\n")` / `Some(b"\r")` 这两个单字节模式**永远不可能**等于一个两字节切片。
所以任何 `stream` 后跟单个 LF 的 PDF（PDF 规范允许，且 LF 归一化文件必然如此）都返回 None，
`repair_stream_lengths` 直接跳过该 stream，**留下错误的 `/Length` 不修**。

修复：先匹配两字节序列，再退回单字节 EOL。

顺带修掉的测试问题：`repaired_classic_structure_has_canonical_xref_and_stream_length` 断言字面
`/Length 882`，那个数字只在夹具是 CRLF 时成立。git 的 text/binary 猜测 + 各机器的 `core.autocrlf`
决定夹具在工作区里是 LF 还是 CRLF，所以这个断言在一台机器上过、在另一台机器上挂，而被测代码两边都对。
改为断言**不变量**（`/Length` 等于真实 stream 字节数、xref 用 CRLF、每个 in-use 条目指向真实
`N 0 obj`、`startxref` 与 xref 偏移一致），并新增 `.gitattributes` 把二进制夹具钉为 `binary`。

## F25. Rust 测试套件此前结构性为红（本轮修复）

`fixtures/golden/private-real/README.md` 明确说这些 PDF 是私有/受版权的回归输入，被 git 忽略。
但 6 个测试硬失败于它们缺失，1 个硬失败于缺 Python `pypdf`。
**因此任何干净 checkout 与任何 CI 的 `cargo test` 都必然是红的**，真实回归被淹没在噪音里。

修复：新增 `src-tauri/src/test_support.rs`，缺私有语料时**可见地跳过**（打印 SKIP 行），
并提供 `EPIC8_REQUIRE_PRIVATE_CORPUS=1` 让挂载了语料的机器/CI 把缺失变回硬失败。
`pypdf` 同理（本机无法安装：PyPI 镜像不可达）。

---

# Session 3 — M0：真实验收基础（2026-09-05，续建计划启动）

## F-M0-1. tauri-driver 环境事实（推翻 S0.6 旧前提）

`tauri-driver.exe` 实际在 `C:\Users\25788\.cargo\bin\`；WebView2 运行时 152.0.4191.62。
匹配的 msedgedriver 从 `https://msedgedriver.microsoft.com/<ver>/edgedriver_win64.zip` 下载成功
（azureedge 已 403/SSL 失败，blob storage 禁公开访问）。E2E 脚本内置：注册表读 WebView2 版本 →
无本地驱动时自动下载缓存到 `%LOCALAPPDATA%\pdf2test-e2e-drivers`。

## F-M0-2. Windows 数据隔离必须走产品钩子，不能只改环境变量

Tauri v2 的 `app_data_dir()` 在 Windows 走 known-folder API（SHGetKnownFolderPath），
**不读 APPDATA/LOCALAPPDATA 环境变量**——第一版 E2E 因此把测试 job 写进了真实用户 AppData（已全部清理，
见 F-M0-8）。修复：`lib.rs app_root()` 增加 `PDF2TEST_AUTOMATION_DATA_DIR` 覆盖（45 个调用点全部经由它，
无旁路），配合 WebView2 官方变量 `WEBVIEW2_USER_DATA_FOLDER` 隔离 localStorage/WebView 配置。
与 job_commands.rs 既有的 `PDF2TEST_AUTOMATION_PDF_DIR/EXPORT_DIR` 同族，未设置时产品行为零变化。

## F-M0-3. 处理中打开工作区会把原始错误码+路径暴露给用户（真实 UX 缺陷）

证据：run-2026-09-05T09-45-07 截图与 report.json。job 尚在 `Working/DocumentReview` 时打开工作区，
降级 UI 显示「这道题还没有可编辑的题稿：AUTHORING_V2_NOT_AVAILABLE:shadow_missing:C:\...\<path>」
——直接违反计划 §3.4 技术词禁令。注意 `deriveStage` 映射本身正确（Working/DocumentReview→local 阶段）；
问题在「行可随时点击打开」与「错误文案未分层」的组合。修复排期：M2（workspace live update）+ M3（用户错误/内部错误分离 §15.2）。

## F-M0-4. 真实 Tauri E2E 全链路结论（M0-T3/T4 定论）

`npm run e2e:tauri` 隔离运行：library 加载 → 免对话框导入（PDF 目录钩子）→ 真实流水线到稳定阶段 →
**工作区打开（V2 shadow 正常写出）** → 原位编辑+「已保存」→ **重开持久化（真实 SQLite/文件链）** 全部 PASS；
发布按钮点击后被质量门如实阻止（友好文案，语料未达 ready）→ 记 `blocked` 状态，verdict
`passed-with-publish-gate-blocked`。**`AUTHORING_V2_NOT_AVAILABLE` 在完整流水线后的真实链路不复现**；
F16 的「devFallback 限制」结论仅限浏览器级。附带观察：TFNG 选项渲染成 "TRUE TRUE"（label+正文重复），
记 M3 renderer 拆分时核对 DS 是否 label==text 重复入档。

## F-M0-5. E2E 竞态教训：行身份必须用 item-id 定位

首两轮失败是 `rows[0]` 身份漂移（乐观插入 + 2s 轮询 + 真实库里已有旧行），不是产品缺陷。
修法：导入前 diff 行集合拿新 item id，后续全部按 `[data-item-id=...]` 定位。另：selenium-webdriver
远程会话必须在 capabilities 里显式 `browserName`（forBrowser() 不写入远程 caps）；Enter 提交后
textarea 立即卸载，最后按键要容忍 stale element（提交本身已发生）。

## F-M0-6. 基线固化升级（M0-T2）

`verify-product-baseline.mjs` 新增：commitSha（git rev-parse）、schemaHash（contracts/ + schema/*.rs +
types/*.ts 内容寻址）、corpus（15 份公开合成 PDF sha256 + 私有语料就绪标志 + e2e 主夹具）。
漂移存在时 `--update` 无 `--reason` 直接拒绝——「重录即绿」的弱门禁收口。

## F-M0-7. phase7 listening 门禁的 peer 路径误置（已修，门禁仍红但原因变真）

门禁硬编码 `../NAS`（不存在的目录），真实学生端在 `../IELTS-NASfor-WenDao`，peer 依赖文件 10/10 齐全
（含 ListeningExamPage.vue、StartupCheckPage.vue）。改为 `NAS_PEER_REPO` env → 旧路径 → 真实路径 的解析链。
门禁当前仍失败，但原因从「缺目录」变为真实契约哈希漂移（`ListeningExamSourceV1` 等，与 phase1 同类）——
哈希重录需与 peer 仓对齐后单独决策，不顺手改绿（AGENTS.md）。

## F-M0-8. 测试污染清理（已完成）

两次无隔离运行把 `pdf-two-column` 测试 job 写进真实 AppData。清理：`scripts/e2e/cleanup-tauri-test-jobs.mjs`
走产品路径移入回收站 + job.json 标题校验后删目录；`authoring_hub.db` 的两条 `library_items` 行按产品同款
软删除语义置 `deleted_at`（ exams 行保留，恢复语义不变）。真实 AppData 现只剩用户自有 `profile-test`。
