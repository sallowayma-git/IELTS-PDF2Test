# A8 — 第 9 章（WYSIWYG ExamCanvas 重构）与第 10 章（前端视觉与溢出专项整改）对抗审计

- 审计日期：2026-09-12
- 审计对象：`Plan With Files/IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md` 第 1533–1972 行
- 代码基线：`47a3806` + 未提交工作区（含未跟踪 `src/exam-canvas/structureActions.ts`、`src/features/editor/SelectionInspector.tsx`、`scripts/e2e/workspace-save-regressions.mjs`、`src-tauri/src/processing/`）
- 立场：默认每条论断为错/过时/未完成，用代码证据证伪。
- 约束遵守：只读审计；未运行 `cargo build`/`cargo test`/`npm run build`；未启动浏览器/服务；未修改产品代码。

---

## 章节范围

| 章节 | 行号 | 主题 |
|---|---|---|
| §9.1–§9.2 | 1535–1578 | 唯一 renderer、删除两套编辑器 |
| §9.3–§9.4 | 1580–1634 | 原位编辑、EditorCommandV1 协议 |
| §9.5–§9.6 | 1636–1692 | 保存链、不每次输入重写 JS |
| §9.7–§9.10 | 1694–1772 | Reading 布局、MatchingMatrix、复杂视觉编辑、Header |
| §10.1–§10.5 | 1777–1905 | 溢出原因、样式目录、硬规则、页面宽度、视觉基调 |
| §10.6–§10.8 | 1907–1971 | Library/设置布局、溢出验收矩阵 |

实际组件位置已按提示修正：`src/components/ExamCanvasV2.tsx` 不存在，renderer 在 `src/exam-canvas/ExamCanvas.tsx`。

---

## 断言核对表

图例：`HOLDS` 成立 / `PARTIAL` 部分成立或命名/位置不符 / `FALSE` 不成立 / `UNVERIFIABLE` 无法验证。
证据层级：`product` 真实产品链 / `command` 命令处理器 / `static` 静态源码 / `doc-only` 仅文档。

### 第 9 章

| # | 断言 | 判定 | 证据（绝对路径:行号） | 层级 |
|---|---|---|---|---|
| 9.1a | 唯一 renderer，签名 `<ExamCanvas ds={canonicalDs} mode>` | **PARTIAL** | 实际 prop 为 `authoring`（非 `ds`），另有 `selectedId/onSelect/onTextCommand/onAnswerChange/onStructureAction`。`F:\workspace\PDF2Test\src\exam-canvas\ExamCanvas.tsx:19-30`；唯一生产调用点 `F:\workspace\PDF2Test\src\features\editor\ExamWorkspacePage.tsx:216-231` | static |
| 9.1b | author/student 使用同一 DOM 结构，作者仅加轻量编辑行为 | **PARTIAL** | author 模式额外渲染 `div.v2-author-tools`、`div.v2-table-frame`、`span.v2-answer-slot-frame`，并把 `span.v2-text` 替换为 `textarea`，加 `tabIndex/role=button`；student 无这些节点。`ExamCanvas.tsx:45-63,148-188,304-314,330-332` | static |
| 9.2a | `ExamCanvasV2.tsx` 重命名为 `src/exam-canvas/ExamCanvas.tsx` | **PARTIAL** | 新文件存在，但旧名以别名保留：`export const ExamCanvasV2 = ExamCanvas`（`ExamCanvas.tsx:475`），且仍被 `F:\workspace\PDF2Test\src\pages\StructuredAuthoringEditorV2.tsx:7,724` 使用 | static |
| 9.2b | `authoringTiptap.tsx` 从主流程删除 | **PARTIAL** | 文件仍存在 `F:\workspace\PDF2Test\src\editor\authoringTiptap.tsx`（导出 `AuthoringTiptapEditor` `:407,415`），仍被 `StructuredAuthoringEditorV2.tsx:6,678,707,708` 导入。该页面经 `F:\workspace\PDF2Test\src\app\legacyRoutes.tsx:16` 挂载，而 `LegacyRoutes` 已不被 `F:\workspace\PDF2Test\src\app\App.tsx` 引用 → 不在生产路由，但未删除、Tiptap 依赖仍在 | static |
| 9.2c | `UnifiedPreview` 的题目输入框全部删除 | **FALSE** | 文件仍存在（1400+ 行），仍含表单编辑：`F:\workspace\PDF2Test\src\pages\UnifiedPreview.tsx:1084,1105,1331,1376,1388,1394`（`updateQuestion`/`updateQuestionAnswer`）；仍被 `legacyRoutes.tsx:17` 与 `F:\workspace\PDF2Test\src\pages\ImportWizard.tsx:8` 引用 | static |
| 9.2d | `LibraryExamDetail` 删除 | **FALSE** | 文件存在 `F:\workspace\PDF2Test\src\pages\LibraryExamDetail.tsx:34`，被 `legacyRoutes.tsx:15` 导入 | static |
| 9.2e | V1 HTML 只在迁移适配器临时转 ContentDoc，不直接渲染 | **FALSE** | 仍以 `dangerouslySetInnerHTML` 直接渲染 HTML：`UnifiedPreview.tsx:1035,1293,1298`、`LibraryExamDetail.tsx:176` | static |
| 9.3a | `contentEditable` 与 `document.execCommand` 完全消失 | **HOLDS** | 全仓 `src` 唯一命中为注释：`F:\workspace\PDF2Test\src\exam-canvas\editors\InlineTextEditor.tsx:5`；主链与 legacy 均无真实调用 | static |
| 9.3b | `AutoSizeTextarea` + composition/IME + 粘贴纯文本 | **PARTIAL** | 组件实名 `InlineTextEditor`（非 `AutoSizeTextarea`），auto-size 用 `scrollHeight`；IME 用 `composing` ref + `onCompositionStart/End`；`onPaste` `preventDefault` 只取 `text/plain`。`InlineTextEditor.tsx:11-107` | static |
| 9.3c | 解决「blur 前崩溃丢内容」 | **FALSE** | 输入中的草稿只存在 `InlineTextEditor` 本地 `useState`（`InlineTextEditor.tsx:24`），提交仍靠 `onBlur`/Enter（`:99-104`）。恢复检查点只序列化已提交的 `draftRef`（`F:\workspace\PDF2Test\src\features\editor\useCanonicalEditor.ts:80-95`），未提交的输入不在 localStorage | static |
| 9.4a | `EditorCommandV1` 含 9 个 op | **FALSE** | 实际仅 3 个：`set_text` / `set_answer` / `set_slot_placement`。`F:\workspace\PDF2Test\src\exam-canvas\editorCommands.ts:15-18`。缺失 6 个（`set_option_text/add_option/delete_option/move_option/set_title/set_table_cell_text`）。功能上选项增删移/表格行列/答案位插删由另一套协议 `ExamCanvasStructureAction` 承担（`F:\workspace\PDF2Test\src\exam-canvas\structureActions.ts:22-107`），标题走 `batch.title`（`useCanonicalEditor.ts:188`），选项/单元格文本走通用 `set_text` | static |
| 9.4b | `expectedText` 用于服务端乐观并发校验 | **FALSE** | `expectedText` 只在客户端 `compileEditorCommand` 内比对并抛 `EditorCommandConflictError`（`editorCommands.ts:63-68`），编译产物是 `AuthoringPatchV2.replaceText`；后端只接收 patch，从不读 `expectedText`（`F:\workspace\PDF2Test\src-tauri\src\library\repository.rs:304-307`）。服务端并发唯一保护是 `base_version`（`repository.rs:294`） | static |
| 9.5a | 450ms debounce | **HOLDS** | `useCanonicalEditor.ts:7,172-177` | static |
| 9.5b | baseVersion 校验 | **HOLDS** | `repository.rs:294-299`（`EDIT_VERSION_CONFLICT:current=..:base=..`） | command |
| 9.5c | 只对「变化目标」做增量校验 | **FALSE** | 实际对整份 DS 跑 `refresh_quality_report` + `validate_authoring`。`F:\workspace\PDF2Test\src-tauri\src\library\commands.rs:67-70`。计划伪代码 `input.commands.targets()` 在实现中不存在（commands 是 patch 数组，无 `targets()`） | command |
| 9.5d | 返回已保存版本与具体问题变化 | **FALSE** | `ApplyEditorCommandsResultV1` 无 `issues` 字段（`F:\workspace\PDF2Test\src\api\workspaceClient.ts:42-50`；`commands.rs:72-79` 仅返回 editVersion/appliedCount/replayed/recoverySnapshotSaved/status） | command |
| 9.5e | 上一轮 F05（慢保存期间编辑不发）已修复 | **HOLDS** | `persist` 改为 `while` 循环排空 pending/batch 后才 `saved`：`useCanonicalEditor.ts:101-120`；回归断言 `F:\workspace\PDF2Test\scripts\e2e\workspace-save-regressions.mjs:106-109` | static（脚本未执行） |
| 9.5f | 上一轮 F06（保存失败仍继续发布）已修复 | **HOLDS** | 失败时 `throw`（`useCanonicalEditor.ts:129`），`flush()` 即 `persist`（`:254`），发布前 `await editor.flush()` 且异常被 `withBusy` 捕获后不再调用发布（`ExamWorkspacePage.tsx:86-102`）；回归断言 `exportCalls===0`（`workspace-save-regressions.mjs:128`） | static（脚本未执行） |
| 9.6 | 不在每次输入重写 JS，只在发布时编译 | **HOLDS** | 前端只发 DS 命令；JS 编译仅在发布链 `compile_reading_source_v2`：`F:\workspace\PDF2Test\src-tauri\src\nas_package_v2.rs:192`。前端每次编辑仅写 localStorage 检查点（`useCanonicalEditor.ts:84-94`），非 JS | static |
| 9.7a | `grid-template-columns: minmax(0,1fr) minmax(360px,0.86fr)` | **HOLDS** | 选择器非计划所写 `.exam-workspace-canvas`，实际 `.workspace-body > .exam-canvas-v2`：`F:\workspace\PDF2Test\src\styles\workspace.css:125-132` | static |
| 9.7b | 两栏 `min-width:0` + `overflow:auto` | **HOLDS** | `workspace.css:134-139` | static |
| 9.7c | `<980px` 切顶部 tab `原文/题目` | **HOLDS** | `workspace.css:146-166`；组件 `ExamWorkspacePage.tsx:193-196,198` | static |
| 9.7d | `height: calc(100dvh - var(--workspace-header-height))` | **FALSE** | `--workspace-header-height` 全仓无定义；实际 `.workspace-page{height:100dvh}` + `.workspace-body{flex:1;overflow:auto}`（`workspace.css:3-10,112`） | static |
| 9.7e | 未靠提高 `tauri.conf.json` minWidth 掩盖溢出 | **HOLDS** | `minWidth` 仍为 1100：`F:\workspace\PDF2Test\src-tauri\tauri.conf.json:18`。注：计划 §9.7 自述「最小可用宽度 1024」与 1100 不一致 | static |
| 9.8 | `MatchingMatrix` + radio/checkbox 语义 | **HOLDS** | `F:\workspace\PDF2Test\src\exam-canvas\renderers\MatchingMatrix.tsx:21-41`（per_slot + 单 slot + ≥2 行才成矩阵），`type={row.interaction}`（`:98`），interaction 仅 `checkbox` 才 checkbox（`ExamCanvas.tsx:429`） | static |
| 9.9a | 拖动裁剪边界 | **FALSE** | 仅数字输入 `RectFields`：`F:\workspace\PDF2Test\src\features\editor\SelectionInspector.tsx:6-18,34`，无拖拽 | static |
| 9.9b | 拖动/调整 slot placement | **PARTIAL** | 有 `RectFields` 数值调整（`SelectionInspector.tsx:43`），无拖动 | static |
| 9.9c | 点击 slot 编辑 display label | **FALSE** | `displayLabel` 全仓只读渲染（`ExamCanvas.tsx:267,326-339`、`SelectionInspector.tsx:39,41`），无编辑入口 | static |
| 9.9d | 替换视觉资源 | **PARTIAL** | 有 `assetId` 下拉（`SelectionInspector.tsx:31-33`），非资源文件选择/上传 | static |
| 9.9e | 切换「按原图显示 / 使用结构化重建」 | **FALSE** | 全仓无该开关（grep `原图`/`结构化重建`/`renderMode`/`displayMode` 均无命中） | static |
| 9.10a | Header 字段集合 | **PARTIAL** | 存在：返回题库、可编辑标题、`本地已完成 · 云端识别中`、`已保存`、查看原文件、发布（`ExamWorkspacePage.tsx:109-161`）。额外多出「问题 N」「撤销」「重做」三个按钮 | static |
| 9.10b | 右上角 `...` 菜单 4 项 | **FALSE** | 实际仅 2 项：`重新识别`、`停止识别`（`ExamWorkspacePage.tsx:148-160`）。缺「重新运行云端识别」「查看技术日志（开发者模式）」「删除题目」 | static |

### 第 10 章

| # | 断言 | 判定 | 证据（绝对路径:行号） | 层级 |
|---|---|---|---|---|
| 10.1a | `.review-grid { 360px … 380px }` 是当前溢出原因 | **PARTIAL** | 规则存在（`F:\workspace\PDF2Test\src\styles\legacy.css:61`），使用点 `F:\workspace\PDF2Test\src\pages\DocumentReview.tsx:97`，但该页经 `legacyRoutes.tsx:11` 挂载且 `LegacyRoutes` 已不被 `App.tsx` 引用 → 不可达 | static |
| 10.1b | `.editor-grid { 240px … 420px }` | **FALSE** | 选择器不存在（仅 `.settings-editor-grid`/`.phase5-editor-grid`/`.slot-editor-grid`）；无 240/420 列宽规则 | static |
| 10.1c | `.llm-grid { … 320px }` | **FALSE** | 选择器不存在，仅出现在注释 `legacy.css:8` | static |
| 10.1d | `.settings-grid { 460px … 320px … 340px }` | **FALSE** | 选择器不存在，仅出现在注释 `legacy.css:8` | static |
| 10.1e | `.metric-row { repeat(6,1fr) }` | **PARTIAL** | 规则存在 `legacy.css:54`，使用点 `F:\workspace\PDF2Test\src\pages\Dashboard.tsx:33,51`、`F:\workspace\PDF2Test\src\pages\LibraryPage.tsx:102`，均为不可达/孤儿页 | static |
| 10.1f | `.surface { overflow: hidden }` | **PARTIAL** | 规则存在 `legacy.css:36`；无 TSX 使用 `.surface`（grep 无命中） | static |
| 10.2 | `src/styles/` 9 个文件，删除单一 `src/styles.css` | **PARTIAL** | 实际 10 个文件，多出未列出的 `legacy.css`（1406 行/63KB）；`styles.css` 未删除但已降为 13 行分层导入（`F:\workspace\PDF2Test\src\styles.css:4-13`），导入清单亦未含 legacy.css | static |
| 10.3a | `box-sizing: border-box` 全局 | **HOLDS** | `F:\workspace\PDF2Test\src\styles\reset.css:8` | static |
| 10.3b | `html, body, #root { width:100%; min-width:0; min-height:100% }` | **HOLDS** | `reset.css:10-15` | static |
| 10.3c | `min-width:0` 应用于 `.app-main/.panel/.grid-child/...` | **PARTIAL** | 实际选择器组为 `:is(.app-main, .app-main-content, .panel, .grid-child, .flex-child, .table-wrap)`（`reset.css:21`），与文档列举不完全一致 | static |
| 10.3d | `overflow-wrap:anywhere` 于 p/td/th/label/button/... | **HOLDS** | `reset.css:24-27`（额外含 `.error-text`） | static |
| 10.3e | `img,svg,video,canvas { max-width:100% }` | **HOLDS** | `reset.css:29` | static |
| 10.3f | `button,input,select,textarea { max-width:100% }` | **HOLDS** | `reset.css:32` | static |
| 10.3g | 主 surface 改 `overflow: clip`；`.app-surface-content` 存在 | **FALSE** | `.app-surface`/`.app-surface-content` 全仓不存在；`.surface` 仍 `overflow:hidden`（`legacy.css:36`），`.exam-canvas-v2` 仍 `overflow:hidden`（`legacy.css:1050`），工作区覆盖规则未改 overflow（`workspace.css:125-132`） | static |
| 10.4a | `.library-page { width: min(100%,1440px) }` | **HOLDS** | `F:\workspace\PDF2Test\src\styles\library.css:3-5` | static |
| 10.4b | `.settings-page { width: min(100%,1440px) }` | **FALSE** | 实际 720px（`F:\workspace\PDF2Test\src\styles\settings.css:3-4`），与本章 §10.7 的 720px 一致、与 §10.4 自相矛盾 | static |
| 10.4c | `.workspace-page { height:100dvh; background: var(--surface) }` | **PARTIAL** | `height:100dvh` 成立，但背景是 `var(--paper-2)`（`workspace.css:3-10`）；`--surface` 在 tokens.css 无定义（`F:\workspace\PDF2Test\src\styles\tokens.css`） | static |
| 10.4d | `.library-shell { container-type: inline-size }` | **FALSE** | `.library-shell` 不存在；实际 `container-type: inline-size` 在 `.library-page`（`library.css:9`） | static |
| 10.4e | `@container (max-width:760px)` 行布局切换 | **HOLDS** | `library.css:171-175` | static |
| 10.5a | 8–12px 圆角 | **FALSE** | 大量 20–28px 圆角残留：`legacy.css:36,40,54,74,82,119,250,269,284`（`.surface` 28px、`.hero-panel` 28px、`.metric-row` 24px 等） | static |
| 10.5b | 不使用大面积渐变/超大圆形装饰 | **FALSE** | 全局 body 仍为 radial+linear 大面积渐变（`reset.css:45-49`）；样式目录共 6 处 gradient（含 `legacy.css:40`） | static |
| 10.5c | 题面字号 15–17px、行高 1.55–1.7 | **HOLDS** | `.exam-canvas-v2 .v2-paragraph/.v2-list { line-height:1.7 }`（`legacy.css:1058,1060`），未覆盖 body 默认 16px | static |
| 10.5d | 主操作按钮最多一个强调色 | **PARTIAL** | 工作区用 `primary` + `ghost`，但 `stage-pill`/`library-tabs`/`save-state`/`settings-advanced-toggle` 大量 999px 胶囊与多强调色（`library.css:37-48,109-127`、`workspace.css:38-44`），与「不过多胶囊按钮」相悖 | static |
| 10.5e | tokens.css 设计令牌 | **PARTIAL** | 文件存在 18 行（`tokens.css:1-18`），但代码引用的 `--surface`/`--surface-raised` 未定义（`workspace.css:176`） | static |
| 10.6 | Library 行不显示 hash/path/schema/revision | **HOLDS** | `F:\workspace\PDF2Test\src\features\library\LibraryItemRow.tsx:3,37-75`（stage-pill/issue-count/progress，无技术码） | static |
| 10.7 | 设置页单列、最大 720px、720–1600px 均单列 | **HOLDS** | `settings.css:3-4,109-113` | static |
| 10.8a | `npm run test:layout` 存在 | **HOLDS** | `F:\workspace\PDF2Test\package.json:15-16` | static |
| 10.8b | 5 个视口全部覆盖 | **HOLDS** | 1100×760@1、1440×960@1、1536×864@1.25(=1920@125%)、1280×720@1.5(=1920@150%)、2560×1440@1 全在 `F:\workspace\PDF2Test\fixtures\ui\long-content.json:137-174`，另多 1280×800@1 | static |
| 10.8c | 页面覆盖 Library/Workspace/Settings/Import drawer | **FALSE** | `PRODUCT_ROUTES` 默认仅 library+settings（`F:\workspace\PDF2Test\scripts\ui\layout-matrix.mjs:25-28`）；Workspace 仅当传 `--item-id`（`:45,201`）；Import drawer 完全未覆盖 | static |
| 10.8d | 180 字符文件名 | **PARTIAL** | fixture 实测 177 字符（`long-content.json:4` 自述 177） | static |
| 10.8e | 500 字符错误说明 | **FALSE** | fixture 实测 240 字符（`long-content.json:5-6`） | static |
| 10.8f | 20 tags / 12 选项 / 10 列表格 / 超长 URL / 中英混排 | **HOLDS** | `long-content.json:8-29,30-79,80-131,7,132` | static |
| 10.8g | 200% 浏览器文本缩放 | **FALSE** | fixture 声明 `browserTextZoomPercent:[100,200]`（`long-content.json:133-136`），但 `layout-matrix.mjs` 从不读取该字段（grep 无命中），从未被施加 | static |
| 19.6 | author/student parity 自动化测试 | **FALSE** | 无任何 DOM/截图 parity 测试；仅 `F:\workspace\PDF2Test\scripts\verify-phase5-editor.mjs:96` 对源码做 token 存在性 grep（含字符串 `mode="student"`）。`src` 下无 `*.test.ts*` 文件 | static |

**判定统计**：HOLDS 17 / PARTIAL 16 / FALSE 19 / UNVERIFIABLE 0（共 52 条）。

---

## CSS 溢出主张的反证核查

### 独立复核 task_plan Phase 0 的反证结论

task_plan（`Plan With Files/Dual_Recognition/task_plan.md:92-94`）称：溢出矩阵 72 项 0 溢出，**计划 UX-002 的溢出前提在受支持窗口范围内不成立，死 CSS 才是真实问题**（`.settings-grid/.editor-grid/.llm-grid` 无人使用）。

独立验证结果——**该反证成立，且比 task_plan 表述更强**：

1. §10.1 列举的 6 条「当前溢出直接技术原因」中，**3 条选择器在代码库中根本不存在**：
   - `.editor-grid`、`.llm-grid`、`.settings-grid` 仅作为注释出现在 `src/styles/legacy.css:8`，无任何规则体。文档给出的 `240px minmax(0,1fr) 420px`、`minmax(0,1fr) minmax(0,1fr) 320px`、`minmax(460px,1.35fr) minmax(320px,.9fr) 340px` 在样式中零命中。
2. 剩余 3 条（`.review-grid`、`.metric-row`、`.surface`）确实存在，但**全部只服务不可达的 legacy 页面**：
   - `.review-grid` → `DocumentReview.tsx:97`，而 `LegacyRoutes` 已不被 `App.tsx` 引用（`App.tsx:1-7` 无 `legacyRoutes` 导入），除 `#/legacy/writing` 外的 legacy 页面不再渲染。
   - `.metric-row` → `Dashboard.tsx`、`LibraryPage.tsx`（后者为孤儿，见 task_plan:178）。
   - `.surface` → 无任何 TSX 使用点。
3. §10.1 的因果句「这些列宽在 1100px Tauri 最小窗口中无法同时容纳 … `overflow:hidden` 又将溢出内容裁掉」把不存在或不可达的规则描述为**在线缺陷**，与 Phase 0 的 0 溢出实测直接冲突。

结论：§10.1 标题「**当前**溢出的直接技术原因」是过时且错误的；它把 P0 之前的旧 `styles.css` 快照当成当前事实。task_plan 已把 P1 重新定义为「删除死代码 + 建立回归门」，但计划正文 §10.1 未同步修订 → 这是本计划第 9/10 章最大的自我矛盾。

### 未被 §10.1 承认的残余裁切容器

`.exam-canvas-v2 { overflow: hidden }`（`legacy.css:1050`）在工作区覆盖规则中**未**被改成 `visible`：`workspace.css:125-132` 只覆盖 `height/min-width/border/border-radius/box-shadow/grid-template-columns`。因此「主 surface 不再使用 `overflow:hidden`」（§10.3）在真实工作区并未完全兑现；当前不产生可见缺陷仅因为两栏 pane 各自 `overflow:auto`（`workspace.css:134-139`）。

### 溢出矩阵真实覆盖（与 §10.8 逐格比对）

| 文档要求 | 实际 | 缺口 |
|---|---|---|
| 5 视口 | 6 视口（多 1280×800@1） | 无缺口 |
| 1440×960「全部」 | 默认仅 library+settings | Workspace、Import drawer 缺失 |
| 1920@125%「全部」 | 默认仅 library+settings | 同上 |
| 1920@150%「Library、Workspace」 | 默认无 Workspace | Workspace 缺失（需 `--item-id`） |
| Import drawer | 无任何路由/脚本 | **完全未覆盖** |
| 180 字符文件名 | 177 | 近似 |
| 500 字符错误 | 240 | **未覆盖** |
| 200% 文本缩放 | fixture 声明但脚本不读 | **未覆盖** |

`scripts/e2e/workspace-save-regressions.mjs` 与 `scripts/ui/layout-matrix.mjs` 均**未注册进 `package.json`** 中的 `test:layout`（layout-matrix 已注册，save-regressions 未注册）——F05/F06 的回归证据无法通过 npm 复现。

---

## 发现清单

### A8-F01 [P1] §10.1 以虚假前提描述「当前溢出原因」，与项目自身 Phase 0 实测矛盾

- 结论：§10.1 列出的 6 条 CSS 中 3 条选择器不存在，另 3 条只服务不可达 legacy 页；文档称其为「当前溢出直接技术原因」属错误陈述。
- 证据：计划 1777–1790；`src/styles/legacy.css:61,54,36,8`；grep `editor-grid|llm-grid|settings-grid` 仅注释；`src/app/App.tsx:1-7`；`task_plan.md:92-94`。
- 影响：以不成立的前提驱动整改与验收，掩盖真实待办（legacy.css 1406 行死代码、`styles.css` 未删、不可达 legacy 页面未删）。
- 建议：把 §10.1 改写为「历史原因（已不成立）＋当前真实问题：死 CSS 与 legacy 未清理」，并删除不存在选择器的描述。

### A8-F02 [P1] §9.4 `EditorCommandV1` 协议与实现不符（3/9 op），`expectedText` 不做服务端校验

- 结论：文档定义的 9 op 联合类型实际只有 3 个；`expectedText` 完全在客户端消费，后端不校验。
- 证据：`src/exam-canvas/editorCommands.ts:15-18,63-68`；`src-tauri/src/library/repository.rs:294,304-307`；`src/api/workspaceClient.ts:56-64`。
- 影响：跨窗口同节点并发保护实际依赖 `baseVersion`，而非文档暗示的 `expectedText`；协议文档会误导后续 P4 原生 `set_text` 实现。
- 建议：修订 §9.4 为实际协议，或明确标注「其余 op 由 `ExamCanvasStructureAction` 承载」；如确需服务端 `expectedText`，在 `apply_editor_commands` 入参中显式携带并校验。

### A8-F03 [P1] §9.2 声称删除的 4 个产物全部仍存在并仍被引用，V1 HTML 仍直接渲染

- 结论：`authoringTiptap.tsx`、`UnifiedPreview.tsx`、`LibraryExamDetail.tsx`、`ExamCanvasV2` 别名均未删除；`dangerouslySetInnerHTML` 直接渲染 V1 HTML 仍在。
- 证据：`src/editor/authoringTiptap.tsx:407`；`src/pages/UnifiedPreview.tsx:1084,1105,1035,1293,1298`；`src/pages/LibraryExamDetail.tsx:176`；`src/exam-canvas/ExamCanvas.tsx:475`；`src/pages/StructuredAuthoringEditorV2.tsx:6-7`；`src/app/legacyRoutes.tsx:15-17`。
- 影响：§9.2 的「不再保留两套编辑器」不成立；Tiptap 依赖与 legacy 表单编辑仍在 `tsc` 编译范围内，维护成本未消除。
- 建议：明确「已退出生产路由」与「已删除」的区别；把删除动作挂到 P10 并在正文标注当前状态。

### A8-F04 [P2] §9.5 保存链两处承诺未实现：返回问题变化、增量校验

- 结论：保存结果不含 `issues`；校验对象是整份 DS，而非变化目标。
- 证据：`src/api/workspaceClient.ts:42-50`；`src-tauri/src/library/commands.rs:67-79`。
- 影响：前端无法在保存后即时反映问题变化；大稿每次保存做全量校验，与「轻量增量」目标不符。
- 建议：`ApplyEditorCommandsResultV1` 增加 `issues`；或修订 §9.5 伪代码与实现一致。

### A8-F05 [P2] §9.9 复杂视觉编辑 5 项中 3 项未实现

- 结论：「拖动裁剪边界」「点击 slot 编辑 display label」「切换原图/结构化重建」无实现；「拖动 slot placement」仅有数值输入。
- 证据：`src/features/editor/SelectionInspector.tsx:6-18,31-49`；`displayLabel` 全仓只读；无 `renderMode/displayMode/结构化重建` 命中。
- 影响：Hybrid 图形「不要求老师手工重画流程图」的可达性存疑。
- 建议：降级 §9.9 为「数值调整 + 资源下拉」，或补实现。

### A8-F06 [P2] §10.3 主 surface 去裁切未落地，`.app-surface*` 类不存在

- 结论：文档指定的 `.app-surface { overflow: clip }` / `.app-surface-content { overflow: visible }` 在代码库中不存在；`.surface` 与 `.exam-canvas-v2` 仍 `overflow:hidden`。
- 证据：grep `.app-surface` 无命中；`src/styles/legacy.css:36,1050`；`src/styles/workspace.css:125-132`。
- 影响：硬规则文档与实现脱节；未来长内容可能被静默裁切。
- 建议：在 `workspace.css` 显式 `overflow: visible` 覆盖，或改写 §10.3。

### A8-F07 [P2] §10.8 溢出验收矩阵存在多处未覆盖格

- 结论：Import drawer 完全未测；Workspace 默认未测（仅 `--item-id`）；错误文案仅 240 字符（要求 500）；200% 文本缩放声明未被脚本读取。
- 证据：`scripts/ui/layout-matrix.mjs:25-45,205`；`fixtures/ui/long-content.json:5-6,133-136`。
- 影响：§10.8 作为验收门存在假绿空间。
- 建议：把 Import drawer 纳入 route 列表；错误文案加长到 500；在脚本中实施 `Emulation.setPageScaleFactor`/文本缩放并纳入断言。

### A8-F08 [P2] §19.6 author/student parity 无自动化测试

- 结论：无 DOM/截图 parity 测试，仅有源码 token grep。
- 证据：`scripts/verify-phase5-editor.mjs:96`；`src` 下无 `*.test.ts*`；`scripts/e2e/` 无 parity 脚本。
- 影响：§9.1「author/student 同 DOM 结构」与 §26.4 A 的跨仓截图 parity 均无证据；且当前 student 模式唯一消费者是不可达的 `StructuredAuthoringEditorV2.tsx:724`。
- 建议：至少补「去 editor overlay 后语义 DOM 对比」的本仓测试，并明确 NAS 跨仓 parity 归属与状态。

### A8-F09 [P2] §9.3「blur 前崩溃丢内容」并未真正解决

- 结论：输入中的草稿只存在 `InlineTextEditor` 本地 state，未提交前不进入恢复检查点。
- 证据：`src/exam-canvas/editors/InlineTextEditor.tsx:24,99-104`；`src/features/editor/useCanonicalEditor.ts:80-95`（仅序列化 `draftRef`）。
- 影响：崩溃/强退会丢失正在输入、尚未 blur 的文本；文档将其列为 textarea 方案收益属过度声明。
- 建议：在 `InlineTextEditor` 内对 draft 做节流持久化，或修订 §9.3 表述。

### A8-F10 [P3] §10.2 样式清单遗漏 legacy.css；§10.4 与 §10.7 对 settings-page 宽度自相矛盾

- 结论：`src/styles/` 实为 10 文件（多 `legacy.css` 1406 行）；§10.4 称 settings-page 1440px，§10.7 与实现均为 720px。
- 证据：`src/styles/legacy.css`；`src/styles.css:4-13`；`src/styles/settings.css:3-4`。
- 影响：文档清单不可用于验收；同一文档内两处互相冲突。
- 建议：补齐清单与导入顺序；统一 settings-page 宽度表述。

### A8-F11 [P3] 保存链回归脚本未接入 npm，F05/F06 证据不可复现

- 结论：`scripts/e2e/workspace-save-regressions.mjs` 不在 `package.json` 的 scripts 中。
- 证据：`package.json:5-54`（无该脚本条目）。
- 影响：task_plan 声称的「保存链故障回归 3 项通过」无法通过标准命令复跑。
- 建议：注册 `test:save-regressions` 并纳入 CI。

### A8-F12 [P3] §9.7/§9.1 文档中的选择器、变量与 prop 名均非实际

- 结论：`.exam-workspace-canvas`、`--workspace-header-height`、`ds` prop 均不存在。
- 证据：`src/styles/workspace.css:125-132`（实际 `.workspace-body > .exam-canvas-v2`）；grep `--workspace-header-height` 无命中；`src/exam-canvas/ExamCanvas.tsx:19-30`（`authoring`）。
- 影响：读者按文档无法定位代码。
- 建议：以实际选择器/变量名更新 §9.1、§9.7。

---

## 与历史审计的关系

| 历史项 | 本组结论 |
|---|---|
| audit-2026-09-07 **F05**（慢保存编辑不发） | 前端循环排空已实现（`useCanonicalEditor.ts:101-120`），并有浏览器级回归断言；判定**已修复（静态）**，但回归脚本未接入 npm（A8-F11）。 |
| audit-2026-09-07 **F06**（保存失败仍发布） | `persist` 抛错 + 发布前 `await flush()` 已实现（`useCanonicalEditor.ts:129`；`ExamWorkspacePage.tsx:88`）；判定**已修复（静态）**。 |
| audit-2026-09-07 **F11**（结构修复未接入工作区） | 部分改善：选项增删移、表格行列、答案位插删、资源替换、裁剪/热点、单元格属性已接入（`structureActions.ts`、`SelectionInspector.tsx`、`ExamWorkspacePage.tsx:225-230`）。但 §9.9 声明的拖动裁剪、display label 编辑、结构化切换仍未接入（A8-F05）。 |
| audit-2026-09-07 **F13**（生产含测试替身与退休 UI） | `App.tsx` 不再渲染 `LegacyRoutes`（`App.tsx:1-7`），退休页面退出生产路由；但文件与 Tiptap 依赖仍在编译范围，且 `styles.css` 仍导入 `legacy.css`（1406 行死代码进入生产 CSS）。 |
| task_plan **Phase 0 溢出结论** | 独立复核**成立且更强**：6 条所谓溢出原因中 3 条选择器不存在、3 条不可达（见上文反证核查）。 |
| task_plan **Phase 1「styles.css -99.1%」** | 成立：`styles.css` 13 行，仅分层导入；但导入链引入未在 §10.2 列出的 `legacy.css`。 |
| task_plan **M3「部分完成」** | 与证据一致；§9.4 协议、§9.9 视觉编辑、NAS renderer parity 仍未完成，且 §9.2 的删除项未执行。 |

---

## 证据层级与局限

- 本次为**只读静态审计**。所有判定基于 `Read`/`Grep`/`Glob`、只读 `git status/log`、`node -e` 读取 fixture 与 `wc`。
- **未执行**：`cargo build`/`cargo test`、`npm run build`/`npm run check`、`npm run test:layout`、`scripts/e2e/workspace-save-regressions.mjs`（需浏览器/服务）。因此 F05/F06 的「已修复」判定为**静态 + 既有脚本断言**层级，不是本轮运行时刻的验收。
- 未验证真实 Tauri/SQLite/NAS 学生端运行时；§19.6 跨仓 parity、§9.9 NAS renderer parity 均无法在本环境核实，标为静态缺失而非运行时缺陷。
- `command` 层级证据来自 Rust 源码阅读（`repository.rs`/`commands.rs`/`nas_package_v2.rs`），未编译运行。
- 判定统计基于 52 条可核对断言；`PARTIAL` 多因文档选择器/命名/位置与实际不符，而非功能完全缺失。
- 本报告只写审计结论，未改动任何产品代码或计划文档。
