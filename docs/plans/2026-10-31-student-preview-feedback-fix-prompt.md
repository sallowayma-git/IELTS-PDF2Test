# 任务：用户反馈的三个问题（Note / 段落匹配题显示成单选 / Part 切换没反应）

先读 `F:\workspace\PDF2Test\AGENTS.md`：以真实 Tauri 产品路径为准，CLI/schema 检查不能替代产品验证。当前 `main` 是 `0880eba`。用户反馈来自这个版本：截图里已经有 A−/A/A+ 字号按钮和单个 Part 下拉。

用户反馈的题目叫「80」，标题 *Learning by example: evidence from studies of rats and birds*，Questions 1–4 是 "Which paragraph contains the following information? A–G"。这道题不在本机库里，复现时请用结构相同的阅读题：段落信息匹配题，A–G 段落，题号 1–13，文件名不含 P1/P2/P3。

## 1. 学生预览缺 Note 功能（直接修）

现状：作者端学生预览（`src/exam-canvas/StudentReadingTools.tsx`）只有 Highlight / Remove highlight。真实学生端 `F:\workspace\IELTS-NASfor-WenDao\apps\student-exam\src\pages\ReadingExamPage.vue` 有两种 Note：

- 顶栏 `Note` 按钮（约 41 行）打开 Notes 面板（约 82–88 行，textarea）；
- 选中文字后工具条上的 `Note`（约 148 行，`captureSelectionNote`，约 937 行）：把选区标成 `kind=note` 的高亮，自动打开面板，并把选中文字追加进笔记（`appendSelectionToNotes`）。点已有高亮再点 Note，会把该高亮转成 note。

要求：

- 在作者端学生预览里补上同样的两种 Note，行为、文案对齐 NAS（英文 `Note` / `Notes` / `Close`），复用现有的 `highlightDom.ts`（已支持 `kind`）。note 高亮的样式要和普通高亮区分开，与 NAS 的 CSS 保持一致。
- 和高亮一样是**预览专用的临时状态**：不写入草稿或权威稿，不影响答题控件；切换编辑/预览或刷新后清空即可。
- 只在 student 模式出现。

## 2. 段落信息匹配题显示成逐题单选，应显示成表格（直接修，两个仓库）

根因：

- 稿件里 `matching_information` 题组没有题组级 `optionBank`。A–G 选项是编译器 `src-tauri/src/reading_source_v2.rs` 的 `normalize_runtime_paragraph_options`（约 211 行）按段落为**每个 responseGroup 单独生成**的 inline `options`。
- 作者端 `src/exam-canvas/ExamCanvas.tsx`（约 1041–1046 行）只有在 `task.optionBank.options` 非空时才走 `MatchingMatrix` 表格。没有 optionBank 时退回逐题单选列表，就是用户截图里每题一列 A–G 圆点。
- NAS `apps/student-exam/src/modules/reading-engine/renderers/ReadingExamV2Renderer.vue` 完全没有表格版式，`responseOptions()`（约 258 行）按每个 response 渲染列表。

要求：

- 判定条件改为：题组内每个 responseGroup 都是 `kind=matching`、`assignment=per_slot`、只有 1 个槽位，**所有行的选项集合相同**（选项来自 optionBank，或者各行 inline options 的 `optionId`/label 完全一致），且槽位交互是 radio/checkbox（不是 `dragdrop`）。满足条件就渲染成表格：行是题号 + 题干，列是选项字母，每格一个单选。这是雅思机考段落匹配题的标准形式。不满足条件的保持现有逐组列表。
- 作者端：扩展 `matchingRowsFor` / `MatchingMatrix`（`src/exam-canvas/renderers/MatchingMatrix.tsx`）支持 inline 选项一致的情况。student 和 author 模式都适用；author 模式下原有的选项编辑入口不能丢（`optionSourceEditable` 对 matching_information 的特殊处理要保留）。
- NAS：在 `ReadingExamV2Renderer.vue` 实现同样的表格版式和判定（可新建组件），答案写入沿用现有的 `onSetAnswer`，提交、计分、草稿恢复不变。改渲染器不改契约，已发布的包不用重新导出也能生效。
- 两边渲染结果要一致（列顺序、题号、题干、选中态、字号缩放、深色模式）。表格在窄屏下要能横向滚动或换行，不能挤坏。

## 3. 编辑器里切换 P1/P2/P3 没反应（必须先复现、定位根因再修）

用户截图：学生预览模式，下拉选了 P3，副标题显示 `READING · Part 3`，同时出现提示条「这份题稿在别处已被更新（版本 4）。你的修改正在保存，保存完成后会自动加载最新版本。」用户说切换没反应。顶部显示本次云端用量 20,523 tokens，说明这份稿走过云端识别。

已知事实（请复核）：

- 编辑器保存时，`src-tauri/src/lib.rs` 的 `apply_editor_commands`（约 990–1004 行）会**先** `notify_item_content_changed` 发出 `processing://item-updated`（带新版本号），**再**把保存结果返回前端。前端 `src/features/editor/remoteVersion.ts` 的 `decideRemoteVersionAction` 收到事件时本地版本还是旧的、又有未保存改动，于是判 `defer` 并显示上面那条提示。正常情况下保存完成后 `shouldApplyDeferredRemoteRefresh` 返回 false，提示会消失。所以这条提示**可能只是自己这次保存的回声**，但它把自己的保存说成「别处已被更新」，本身就会误导用户。
- 如果提示一直不消失，或者重新打开后 Part 回到原值，就说明是真问题。候选原因（未证实）：
  a. 保存被后端拒绝（如 `EDIT_VERSION_CONFLICT`、`CLOUD_REVIEW_IN_PROGRESS`、校验失败），前端没有如实提示，下拉值随后被服务端内容覆盖；
  b. 有并发写入推进了版本：题库列表/单行刷新/打开工作区时的懒对齐（`src-tauri/src/library/commands.rs` 的 `align_reading_part`，包括 `PartBackfill` 系统回填），或云端处理后续阶段（云端修复、答案页识别、候选采纳）写权威稿时把 `exam.category` 改回自动值。重点检查：机器来源的写入是否尊重 `setExamCategory` 登记的 `exam` 人工保护范围；用户保存因冲突失败时人工保护根本没有写入，会不会被下一次机器写入覆盖；
  c. 学生预览模式下的下拉 patch 路径和编辑模式不同。

要求：

- **先复现**：用一份云端直出导入的阅读题（结构同上，文件名不含 Part 信息，题号 1–13 会被判成 P1），分别在编辑模式和学生预览模式下切换 P1→P3→P2。也要覆盖处理任务仍在进行（云端后续阶段没跑完）的情况。观察提示条是否消失、下拉是否回弹、重开后的值、题库列表的值、数据库里 `exam.category` / `part_label` / `part_source` / 版本号，以及 journal 里的写入来源。用 `scripts/e2e/tauri-cdp-*.mjs` 的 CDP 方式写成可重复的验收脚本，可以在 `tauri-cdp-part-assignment-sync.mjs` 上扩展。
- 定位到根因后再修，并在报告里写清楚是哪条路径。
- 无论根因是什么，都要消除「自己保存的回声被说成别处已更新」：前端识别自己正在进行的保存（例如保存请求在途时，事件版本等于预期的新版本就忽略），或者后端改为先返回结果再通知，二选一并说明理由。真正来自别处的更新仍然必须提示和处理，不能被这个改动吞掉。
- 如果用户的切换因为冲突或锁定没有保存成功，界面必须明确告诉用户，并保留或恢复用户的选择，不能悄悄回弹。

## 不要做

- 不提交 git，不改发布门禁和 golden baseline，不改写真实用户库 `%APPDATA%\com.ielts.author.studio\authoring_hub.db`（需要真实数据时先做备份副本）。
- 不为了让测试通过而改测试期望。
- NAS 仓库同样只改工作区、不提交。

## 交付

1. 改动文件清单，两个仓库分开列。
2. 测试：
   - 作者端 vitest：Note 的两个入口和面板、note 高亮不写入草稿；inline 选项一致时渲染表格、不一致时回退列表、dragdrop 不走表格；自身保存回声不触发提示、真正的远端更新仍然提示；保存失败时 Part 下拉的提示和状态。
   - Rust：第 3 项定位出的根因对应的回归测试。
   - NAS：`ReadingExamV2Renderer.test.ts` 覆盖表格判定、选中、答案写入。
3. 已运行命令及结果：`npm run check`、`npm test`、`cargo test --manifest-path src-tauri/Cargo.toml --lib -- library:: authoring_v2 export cloud_direct:: processing::scheduler::tests`，以及 NAS 的 typecheck + test。
4. 真实产品验证（按 AGENTS.md 区分端到端 / 命令层 / 仅 schema）：
   - Tauri 应用学生预览里实际使用 Note（两个入口）、段落匹配题表格作答，附截图；
   - 第 3 项的 CDP 复现脚本：修复前失败、修复后通过，附报告；
   - NAS 学生端用导出包实际加载一份段落匹配题，看到表格并能作答提交，附截图。做不到的部分明确写出缺口。
