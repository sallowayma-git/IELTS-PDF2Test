# A7 — 第 8 章（本地/云端确定性对齐、合并、人工确认）与第 15 章（错误处理与用户文案）对抗审计

- 审计对象：`Plan With Files/IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md` 第 1404–1531 行（§8.1–§8.7）与第 2395–2449 行（§15.1–§15.3）
- 审计日期：2026-09-12
- 代码基线：`HEAD = 47a38064aef58e023bce58e5b6feafa99031b252`（`git rev-parse HEAD`）+ 33 个已跟踪未提交改动 + 7 个未跟踪新增目录/文件（`git status --porcelain`）
- 立场：默认每条论断为错/过时/未完成，用代码证据逐条证伪
- 只读审计；未修改任何产品代码；未运行 `cargo build/test`、`npm run build`；未发起任何真实 LLM 请求
- 参考背景：`audit-2026-09-07/report.md`（13 项）、`repair-2026-09-07/progress.md`（F-M0-3）、`task_plan.md`（M5 pending、已知差距 #4）、根 `AGENTS.md`

---

## 章节范围

第 8 章主张「本地与云端两路都不是最终稿」，由 `ReconciliationEngine` 产出 `Canonical DS + Actionable Issues`，并给出对齐键（§8.2）、10 行自动合并矩阵（§8.3）、`can_auto_fill` 8 条件（§8.4）、`ActionableIssueV1` 契约与 3 条文案（§8.5）、`ExamCanvas` 原位差异 UI（§8.6）、`user_edited` + `ProposalOnly` 迟到结果保护（§8.7）。

第 15 章主张错误分层（§15.1 `UserErrorCategory` 8 枚举）、用户错误与内部错误分离（§15.2 `AppErrorV2`）、7 行降级规则（§15.3）。

本次核查的总体结论：

1. **§8 的合并引擎整章不存在。** `ReconciliationEngine` / `align_task_groups` / `TaskAlignment` / `bipartite_max_weight_matching` / `can_auto_fill` / `CandidateField` / `MergeDecision::ProposalOnly` 在 `src-tauri/src/**` 与 `src/**` **零命中**；`CloudRecognitionCandidateV1` / `LocalRecognitionCandidateV1` 同样零命中（与同批 A2/A6 报告一致）。当前真实策略是「**本地唯一权威，云端只读对照**」，不存在字段级优先级决策。
2. **§8.5 的 `ActionableIssueV1` 三方（计划 / 后端 / 前端）命名与字段均不一致**，且后端仅有**一张从未读写的死表** `actionable_issues_v1`。
3. **§8.6 的原位 diff UI 完全不存在**；当前是中央问题列表 + `scrollIntoView`（`task_plan.md` S3.1 口径修正已自认，本次核实成立）。
4. **§8.7 的 `provenanceStatus = user_edited` 标记存在**（`repository.rs` / `authoring_v2_commands.rs`），但**没有任何消费者**：`ProposalOnly` / `MergeDecision` 不存在，「迟到云端结果不得覆盖用户编辑」的保护**靠的是「云端根本不写 canonical」这一巧合，而不是靠该标记**。一旦 M5 落地，该护栏为零。
5. **§15.1/§15.2 的类型（`UserErrorCategory`、`AppErrorV2`、`internal_detail_id`）全仓仅命中计划文档**，零实现。错误分层以「各处手写字符串映射」的散装形式存在，且 **F-M0-3 只修了一半**：workspace 加载错误已分层，但发布前 `flush()` 失败、删除/恢复、导入失败等路径仍把原始机器码/路径直送 UI。
6. **§15.3 的 7 行降级规则中，仅第 4 行（两者失败保留失败行可重试）完整成立**；第 2 行（本地失败、云端成功 → 提供云端稿）与现有调度顺序**直接矛盾**（本地失败即 `fail_job` 返回，云端从不运行）。

---

## 断言核对表

### 第 8 章

| # | 章节断言 | 判定 | 证据（绝对路径:行号） |
|---|---|---|---|
| 1 | §8.1 `LocalRecognitionCandidateV1` / `CloudRecognitionCandidateV1` 两路候选类型存在 | **FALSE** | 全仓 Grep `LocalRecognitionCandidateV1\|CloudRecognitionCandidateV1` → 仅命中计划文档 `IELTS_..._Plan_CN.md:1409-1410` 与 `task_plan.md:32,71`；`src-tauri/src/**`、`src/**` 零命中。云端唯一输出契约仍是 `F:\workspace\PDF2Test\src-tauri\src\llm_suggestions.rs:235`（`"schema": "CloudReadingOutlineV1"`） |
| 2 | §8.1 `ReconciliationEngine` 存在，产出 `Canonical DS + Actionable Issues` | **FALSE** | Grep `ReconciliationEngine\|reconciliation_engine` → `src-tauri/src` 零命中。唯一含 `Reconcil` 的产品代码是注释 `F:\workspace\PDF2Test\src-tauri\src\authoring_pipeline.rs:1907`（题号区间错位修复），与合并无关 |
| 3 | §8.2 `align_task_groups` 二分图最大权匹配（权重 0.45/0.20/0.15/0.15/0.05） | **FALSE** | Grep `align_task_groups\|bipartite\|TaskAlignment\|unique_threshold\|ambiguity_margin` → `src-tauri/src` 零命中。题组对齐在现有链中不存在 |
| 4 | §8.2 `unique_threshold=0.72` / `ambiguity_margin=0.08` 常量 | **FALSE** | 同上，常量零命中；`0.72`/`0.08` 未出现在任何 Rust 源码 |
| 5 | §8.2 无法唯一对齐则产生 conflict，不强行合并 | **UNVERIFIABLE（无对象）** | 因不存在对齐，无 conflict 概念。现有唯一「不合并」表现为云端永不进入 canonical：`F:\workspace\PDF2Test\src-tauri\src\auto_pipeline.rs:2067` 与 `:2451` 的 `build_authoring_v2_shadow(&job, &ir, &split, ...)` 只吃本地 `ir` |
| 6 | §8.3 自动合并矩阵 10 行 | **FALSE（10 行中 7 行 FALSE、2 行「靠缺席而成立」、1 行 PARTIAL）** | 见下「§8.3 逐行核对」 |
| 7 | §8.4 `can_auto_fill` 7 条件（含 `confidence >= 0.88`、`!is_answer_key`、`!changes_question_numbering`） | **FALSE** | `can_auto_fill` / `target_is_empty` / `incoming_is_empty` / `is_answer_key` / `changes_question_numbering` 全仓零命中。唯一近似物是 LLM 建议自动采用门 `F:\workspace\PDF2Test\src-tauri\src\llm_suggestions.rs:451-463`（阈值 **0.85**，非 0.88），且只覆盖 `kind`/`layout`/`questions` 三路径（`:469-473`），**无** answer-key / 题号变更条件 |
| 8 | §8.5 后端存在 `ActionableIssueV1`（含 `library_item_id`/`title`/`suggested_action`/`source_anchor`/`local_value`/`cloud_value`/`status`） | **FALSE（仅表结构）** | Rust 无 `ActionableIssueV1` 类型。仅有建表语句 `F:\workspace\PDF2Test\src-tauri\src\library\schema.rs:87-104`（表名 `actionable_issues_v1`，字段与计划 §8.5 一一对应），但全仓 Grep `actionable_issues_v1` 仅命中 `schema.rs:87,103,104,147`（DDL 与迁移断言），**无任何 INSERT/SELECT**。`actionable_count` 列亦恒为 0：`F:\workspace\PDF2Test\src-tauri\src\processing\scheduler.rs:344-354,445-455` 均传 `None` |
| 9 | §8.5 前端 `ActionableIssueV1` 与后端/计划一致 | **FALSE（字段严重缺失）** | 前端 `F:\workspace\PDF2Test\src\features\editor\actionableIssues.ts:19-27` 只有 `issueId/targetId/severity/code/userMessage` 5 字段；缺 `libraryItemId/title/suggestedAction/sourceAnchor/localValue/cloudValue/status` 7 字段。`severity` 只有 `"blocker"\|"warning"`（`:9`），计划为 `IssueSeverity` 枚举 |
| 10 | §8.5 用户界面展示 3 条示例文案（`第 12 题题干可能缺少下一行 [查看]` / `第 18 题 B 选项未识别 [补充]` / `Questions 27-28 被识别为两个题组 [合并]`） | **FALSE** | 3 条文案全仓零命中。前端实际产出 `第 N 题的题干是空的，需要补上。`（`actionableIssues.ts:123`）、`第 N 题 B 选项没有正文。`（`:86`）；**无「题组被识别为两个」这一 issue 类型**，也无 `[查看]/[补充]/[合并]` 动作按钮——UI 仅把 `userMessage` 渲染为一个 `scrollIntoView` 按钮（`F:\workspace\PDF2Test\src\features\editor\ExamWorkspacePage.tsx:175-184`） |
| 11 | §8.6 `ExamCanvas` Author Mode 原位差异（绿色浅底 + 采用/忽略、红色删除线、题组右上角标记 + popover） | **FALSE** | `src/exam-canvas/**` 仅 6 个文件（`ExamCanvas.tsx` 475 行、`editorCommands.ts`、`structureActions.ts`、`editors/`、`model/`、`renderers/`）。Grep `cloud\|diff\|suggestion\|popover\|line-through` 于 `src/exam-canvas` → 唯一命中 `ExamCanvas.tsx:339`（答案位 badge，与云端无关）。`采用/忽略` 按钮只存在于**退休页** `F:\workspace\PDF2Test\src\pages\UnifiedPreview.tsx:999-1000` |
| 12 | §8.6 不创建独立 LLM Review 页面 | **HOLDS** | `src/app/router.ts` 收敛为 library/workspace/settings 三路由（`task_plan.md:98`）；`UnifiedPreview` 仅在 `#/legacy/...` 可达。但**结论方向与 §8.6 相反**：不是「合并进 Canvas」，而是「能力尚未从退休页迁出」 |
| 13 | §8.7 用户编辑节点带 `provenanceStatus = user_edited` | **HOLDS** | 类型：`F:\workspace\PDF2Test\src-tauri\src\schema\content_doc_v2.rs:8-13`（`ProvenanceStatusV2::UserEdited`）。DB 链标记：`F:\workspace\PDF2Test\src-tauri\src\library\repository.rs:205-223`（`mark_command_target_user_edited`，无字段的新节点也补标记），在事务内逐条调用 `:304-307`。旧链标记：`F:\workspace\PDF2Test\src-tauri\src\authoring_v2_commands.rs:2091-2103`（`mark_user_edited`）。回归断言：`repository.rs:539-543` |
| 14 | §8.7 迟到 cloud result 不得自动修改，只能 `MergeDecision::ProposalOnly` | **FALSE** | Grep `ProposalOnly\|MergeDecision` 全仓零命中。**且不存在任何读取 `user_edited` 以阻止写入的代码**：Grep `UserEdited\|user_edited` 于 `src-tauri/src` 的命中全部是「写入/序列化/测试」，无一处是比较判定 |
| 15 | §8.7 存在真实的「迟到云端覆盖用户编辑」风险路径 | **PARTIAL（当前未兑现，但非因护栏）** | 见下「§8.7 风险路径核查」 |

### 第 15 章

| # | 章节断言 | 判定 | 证据（绝对路径:行号） |
|---|---|---|---|
| 16 | §15.1 `UserErrorCategory` 8 枚举（`SourceUnreadable`…`DestinationUnavailable`） | **FALSE** | 全仓 Grep `UserErrorCategory` → 仅命中计划文档 `IELTS_..._Plan_CN.md:2400`。`src-tauri/src/**`、`src/**` 零命中 |
| 17 | §15.2 `AppErrorV2`（code/category/user_message/retryable/target_id/internal_detail_id） | **FALSE** | Grep `AppErrorV2\|internal_detail_id` → 仅命中计划文档 `:2415,2421`。产品代码零命中 |
| 18 | §15.2 用户可见文案「云端返回内容不完整，已保留本地识别结果。可以继续编辑或重试云端识别。」 | **PARTIAL（语义近似，字面不存在）** | 该句零命中。实际云端失败文案是 `云端对照没有完成，请人工确认题组和答案。`（`F:\workspace\PDF2Test\src-tauri\src\auto_pipeline.rs:1144-1147`）与 `云端整卷对照仍有差异，请在题稿编辑页核对云端/本地差异。`（`F:\workspace\PDF2Test\src\pages\ExportPage.tsx:48`）。语义分层成立，字面未落地 |
| 19 | §15.2 内部细节「`JSON_SCHEMA_REQUIRED $.taskGroups[3].responseGroups[0].options`」只在日志记录 | **FALSE** | `JSON_SCHEMA_REQUIRED` 全仓零命中。且**内部细节并未进日志**：`F:\workspace\PDF2Test\src-tauri\src\processing\scheduler.rs:441-444` 的 `fail_job` 取 `error.split(':').next()` 截断到 80 字符后写入 `last_error_code`，**完整 error 被丢弃且无任何 `eprintln!`/日志调用**，与 `:442` 注释「完整错误在应用日志里」直接矛盾 |
| 20 | §15.2 错误分层：用户只看人话，机器码只在开发者模式 | **PARTIAL（F-M0-3 只修了一半）** | 已修：workspace 加载错误 `F:\workspace\PDF2Test\src\features\editor\ExamWorkspacePage.tsx:30-37` 映射 + `:203-205` 开发者模式附注；编辑保存 `useCanonicalEditor.ts:13-20`。**未修**见下「§15.2 原始错误泄漏残留」 |
| 21 | §15.3 第 1 行 本地成功、云端失败 → 立即提供本地稿；题库显示「云端未完成」但可编辑 | **PARTIAL** | 「提供本地稿且可编辑」HOLDS：`F:\workspace\PDF2Test\src-tauri\src\processing\scheduler.rs:340-359` 云端失败仍推进 `ready_for_review` 并 `set_item_status_ready`。「显示『云端未完成』」FALSE：`云端未完成` 全仓零命中；且 `:351` 把 `reconcile_status` **硬编码为 `"succeeded"`**，无论 `cloud_status` 是 `succeeded` 还是 `failed`（`:340-343`） |
| 22 | §15.3 第 2 行 本地失败、云端成功 → 提供云端稿 | **FALSE** | `F:\workspace\PDF2Test\src-tauri\src\processing\scheduler.rs:289-292`：本地失败即 `fail_job` 并 `return`，**云端阶段从不执行**。与计划 §6 第 626 行「两条主识别链并发」也矛盾。且云端从不能产出题稿（仅 `CloudReadingOutlineV1` 对照提纲） |
| 23 | §15.3 第 3 行 两者部分成功 → 合并有效题组，缺失题组保留 source visual/manual shell | **FALSE** | 无合并引擎；`salvage` 零命中（与 A6 报告一致） |
| 24 | §15.3 第 4 行 两者失败 → 题库保留失败行，可重试；不创建伪造题目 | **HOLDS** | `scheduler.rs:441-468`（`fail_job` → `STAGE_FAILED` + `set_item_status("failed")`）；`F:\workspace\PDF2Test\src\features\library\libraryTypes.ts:133`（`识别失败，可以重试`）；重试入口 `F:\workspace\PDF2Test\src-tauri\src\processing\queue.rs:257-271` |
| 25 | §15.3 第 5 行 云端 JSON 无法修复 → 内部记录 rejected；前端不展示 raw JSON | **PARTIAL** | 内部记录 HOLDS：`auto_pipeline.rs:1142-1149` 把原始错误写入 `report["failure"]`（`src/types/settings.ts:125-131` 暴露 `failure?: string`）。「记录 rejected」FALSE：无 rejected 状态，只有 `failure` 字符串。「前端不展示 raw JSON」在活动三表面成立（`ImportWizard.tsx:73` 只渲染 `issue.message`），但**退休页 `UnifiedPreview.tsx:1148-1149` 仍把 `caught.message` 原样渲染为 `llm-suggestion-error`**，且 `:290` 会把 `cloud.failure` 判为 `error` 态 |
| 26 | §15.3 第 6 行 保存冲突 → 自动拉取最新 DS；若只改不同 node 重放本地命令；否则原位提示 | **FALSE** | `F:\workspace\PDF2Test\src\features\editor\useCanonicalEditor.ts:121-130`：冲突时只 `setSaveState("conflict")` + 一句顶层 `saveMessage`，然后 `throw error`；**无自动拉取、无命令重放、无 node 级 diff**。提示也不「原位」，渲染在页面顶部（`ExamWorkspacePage.tsx:169`） |
| 27 | §15.3 第 7 行 发布被阻止 → 点击问题直接定位到 Canvas 对应节点 | **PARTIAL** | 工作区问题面板确可定位：`ExamWorkspacePage.tsx:177-181`（`scrollIntoView`）。但**发布被阻止这条链不行**：`check_publish_preflight` 产出的 blocker `targetId` 几乎全为 `null`（`F:\workspace\PDF2Test\src-tauri\src\authoring_v2_commands.rs:363,381`），且 `F:\workspace\PDF2Test\src\api\publishClient.ts:22-29` 只取 `userMessage`，**丢弃 `targetId` 与 `action`**，因此发布失败无法定位 |
| 28 | 交叉核查：`normalizeAppError` 把内部错误与用户错误分离 | **FALSE** | Grep `normalizeAppError` → **仅命中计划文档**。`F:\workspace\PDF2Test\src\api\tauriCommands.ts:78-84` 的 `command()` 直接 `return invoke<T>(...)`，不做任何错误归一化；`F:\workspace\PDF2Test\src\api\workspaceClient.ts:52-64` 亦直接透传。所有调用点各自 `error instanceof Error ? error.message : String(error)` |

**判定统计（28 条）**：`FALSE` 18、`PARTIAL` 6、`HOLDS` 3、`UNVERIFIABLE` 1。
- `FALSE`：§8.1(×2)、§8.2(×2)、§8.3 总判定、§8.4、§8.5(×3)、§8.6、§8.7 的 `ProposalOnly`、§15.1、§15.2 的 `AppErrorV2`、§15.2 的日志分层、§15.3 第 2/3/6 行、`normalizeAppError`。
- `PARTIAL`：§8.3 第 4 行、§8.7 风险路径、§15.2 用户文案（字面）、§15.2 分层覆盖度（F-M0-3）、§15.3 第 1/5/7 行。
- `HOLDS`：§8.6「不创建独立 LLM Review 页面」、§8.7 `user_edited` 标记写入、§15.3 第 4 行。
- `UNVERIFIABLE`：§8.2「无法唯一对齐产生 conflict」（无对齐对象）。
- §8.3 子表 10 行：`FALSE` 7、`PARTIAL` 2、`HOLDS` 1（靠缺席成立）。

---

## 合并与错误分层核查

### §8.3 自动合并矩阵逐行核对

前置事实：云端输出为 `CloudReadingOutlineV1` 只读对照提纲（`F:\workspace\PDF2Test\src-tauri\src\llm_suggestions.rs:235,253` "for comparison only"）；`build_authoring_v2_shadow` 只吃本地 `ir`（`auto_pipeline.rs:2067,2451`）。因此**没有任何一行是「合并」实现**。

| 行 | 字段情况 | 计划自动处理 | 判定 | 证据 |
|---|---|---|---|---|
| 1 | 两边完全相同 | 接受本地，记录 agreement | **FALSE** | `agreement` 全仓零命中；无 agreement 记录 |
| 2 | 本地题干为空、云端非空且有 evidence | 补入云端题干，标记 `cloud_fill` | **FALSE** | `cloud_fill` 全仓零命中；云端从不回填 canonical |
| 3 | 本地有内容、云端多一个明显续行 | 合并续行（前提 local source ledger 未分配文本） | **FALSE** | 无续行合并；「未分配文本账本」属 M4 未开始项（`task_plan.md:31` P4-T06） |
| 4 | 两边题干不同且都非空 | 不自动覆盖；提示原位差异 | **PARTIAL（靠缺席成立）** | 「不自动覆盖」因云端永不写入而平凡成立；「原位差异提示」FALSE（见 #11） |
| 5 | 本地缺 B 选项，云端有 B 且证据有效 | 补 B；轻提示 | **FALSE** | 无 option 级云端补齐 |
| 6 | 两边 option label 数量不同且无法对应 | 不自动合并结构；blocker | **FALSE** | 前端只对「选项为空」产 blocker（`actionableIssues.ts:69-77`），无「label 数量与云端不一致」判定 |
| 7 | task type 不同但 instruction signature 明确 | 采用 deterministic instruction result | **FALSE** | 无 deterministic instruction 决策；云端 kind 不一致时整组对照只产 `cloud_group_*` 观测项（`auto_pipeline.rs:826-890`） |
| 8 | answer 冲突 | 不自动合并；必须确认 | **PARTIAL** | 无「answer 冲突」判定。唯一近似是视觉答案候选需人工 `采用/忽略`，但只在退休页 `UnifiedPreview.tsx:979-1000`，且属 M5 前的旧链 |
| 9 | 云端只有 outline 无正文 | 不进入 canonical | **HOLDS（平凡）** | 云端恒不进入 canonical（`auto_pipeline.rs:2067`） |
| 10 | cloud partial group invalid | 丢弃该 group；题库显示「有 1 项待检查」 | **FALSE** | 无 group 级 salvage/丢弃。云端 group 校验失败走**整份** Err（`llm_gateway.rs:1182-1318` 的 `validate_cloud_outline_output`），不是按组丢弃；题库文案「有 N 项待检查」来自 `libraryTypes.ts:132` 的本地 actionable 计数 |

**内部矛盾**：§8.3 第 2/5 行把「云端补空字段」列为自动行为，而 §8.4 `can_auto_fill` 要求 `confidence >= 0.88`；但现有唯一自动采用门是 **0.85**（`llm_suggestions.rs:461`），且仅作用于 LLM 建议的 `kind/layout/questions`，**不含题干、选项、答案**。两个阈值（0.88 vs 0.85）在计划内未做任何调和，实施时必然二选一。

### §8.5 issue 命名四方一致性

| 计划 §6.8 的 7 个 code | 后端 `issue_codes.rs` | 前端 `actionableIssues.ts` | 后端 `authoring_review.rs` 实际发射 |
|---|---|---|---|
| `QUESTION_PROMPT_MISSING` | 无（最近为 `PROMPT_EMPTY`，`issue_codes.rs:19`） | `QUESTION_PROMPT_MISSING`（`:122`） | `QUESTION_PROMPT_MISSING`（`:445,739`） |
| `QUESTION_PROMPT_BOUNDARY_AMBIGUOUS` | `PROMPT_BOUNDARY_AMBIGUOUS`（`:20`） | 无 | 无 |
| `OPTION_LABEL_MISSING` | **无** | **无** | 无 |
| `OPTION_TEXT_MISSING` | **无** | `OPTION_TEXT_MISSING`（`:85`） | 无 |
| `OPTION_RUN_INCOMPLETE` | `OPTION_RUN_INCOMPLETE`（`:21`） | `OPTION_RUN_INCOMPLETE`（`:74`） | `OPTION_RUN_INCOMPLETE`（`:457,917`） |
| `SHARED_OPTION_BANK_MISSING` | 无（最近为 `OPTION_BANK_MISSING`，`:23`） | `SHARED_OPTION_BANK_MISSING`（`:98`） | 无 |
| `SIGNIFICANT_SOURCE_TEXT_UNASSIGNED` | 无（最近为 `SIGNIFICANT_REGION_UNASSIGNED`，`:53`） | **无** | 无 |
| （计划外） | 47 个 code（`issue_codes.rs:7-53`） | `ANSWER_MISSING` / `ANSWER_UNRESOLVED`（`:140,153`） | — |

**结论：四方互不一致。** 计划 7 个 code 中，后端 `issue_codes.rs` 只与 2 个同名；前端只与 4 个同名（另加 2 个计划外 code）；`authoring_review.rs` 实际发射的 code 又是第三套子集。此外前端 `OPTION_TEXT_MISSING` 与 `ANSWER_MISSING` 在计划 §6.8 清单里根本没有，而后端有 47 个 code 覆盖 `ASSET_*`、`SLOT_*`、`HOTSPOT_*` 等前端完全没有的类别。**前端注释声称「检查项与将来后端一致，P8 迁移时可直接对比结果」（`actionableIssues.ts:6-8`）不成立。**

另有一处结构性重复：`get_workspace_item` 已经把后端 quality report 的 `/quality/issues`（`ReviewIssueV2` 数组，含 `sourceAnchors`/`suggestedActions`）返回给前端（`F:\workspace\PDF2Test\src-tauri\src\library\commands.rs:33-40`、`:57`），但 `ExamWorkspacePage.tsx:67` **完全忽略 `workspace.issues`**，改用 `deriveActionableIssues(editor.draft)` 前端重算。即同一工作区同时存在两套互不相干的 issue 体系，后端那套的 `sourceAnchors`/`suggestedActions` 被浪费。

### §8.7 风险路径核查：迟到云端结果是否覆盖用户编辑

1. **主链安全，但不是因为护栏。** `apply_llm_suggestion_core` 确实会把云端建议写回题稿（`F:\workspace\PDF2Test\src-tauri\src\llm_commands.rs:263-451`，`:440` `write_json(authoring-ir.json)`），`apply_suggestion_to_authoring` **不做任何 provenance 检查**（`F:\workspace\PDF2Test\src-tauri\src\llm_suggestions.rs:664-737`，只按 `selected_paths` 白名单替换 `kind`/`layout/template`/`questions[].prompt`）。但：
   - 它写的是**旧链 V1 工件** `authoring-ir.json`，**不写** `library_items_v2.canonical_ds_json`；
   - 工作区读的是 DB 权威稿（`library/commands.rs:33-40`），解析顺序已修为 revision → DB → shadow（`repair-2026-09-07/progress.md:40`）。
2. **因此当前不存在「云端覆盖 canonical 用户编辑」的已兑现路径**——但这是**架构缺席的副作用，不是 §8.7 保护生效**。
3. **风险一旦兑现即成 P0：** 只要 M5 按计划引入 `CloudRecognitionCandidateV1` + 三方合并，而 `provenance_status == UserEdited` 在 `src-tauri/src` **无任何读取点**（Grep 证实），合并器就没有任何依据跳过用户已编辑节点。计划 §8.7 的 `MergeDecision::ProposalOnly` 是**唯一**设计中的护栏，而它不存在。届时迟到云端结果会直接覆盖用户编辑。

### §15.2 原始错误泄漏残留（F-M0-3 复核）

F-M0-3 描述的「降级 UI 把原始错误码 + 路径暴露给用户」**在 workspace 加载路径已修复**（`ExamWorkspacePage.tsx:30-37` + `:203-205` 开发者模式门）。但下列路径仍把原始机器码/内部标识直送 UI：

| 位置 | 场景 | 泄漏内容 |
|---|---|---|
| `F:\workspace\PDF2Test\src\features\editor\ExamWorkspacePage.tsx:80` | 发布 / 返回题库时 `editor.flush()` 失败 | `setNotice(error.message)` 原样透传 `persist()` 抛出的后端串（`useCanonicalEditor.ts:129` `throw error`），例如 `EDIT_VERSION_CONFLICT:current=2:base=1`、`ITEM_DS_NOT_SEEDED:<itemId>`、`library_v2_tx:<sqlite 错误>`（`library/repository.rs:257,293,300,302`） |
| `F:\workspace\PDF2Test\src\features\editor\ExamWorkspacePage.tsx:100` | 发布结果 `outcome.message` | `describePublishError` 未命中任何分支时 `return raw`（`F:\workspace\PDF2Test\src\api\publishClient.ts:34`），原始错误串进 notice |
| `F:\workspace\PDF2Test\src\features\library\LibraryPage.tsx:113,122` | 删除/恢复失败 | `删除失败：${error.message}` 直拼原始串 |
| `F:\workspace\PDF2Test\src\features\import\useImportFiles.ts:8,34-35` | 导入失败 | `formatImportError` 只特判 `source_file_too_large`，其余 `return message` 原样进入 `ImportDrawer` |
| `F:\workspace\PDF2Test\src\features\settings\SettingsPage.tsx:150,176,191` | 设置保存/测试失败 | `setError(caught.message)` 直送 |
| `F:\workspace\PDF2Test\src\pages\UnifiedPreview.tsx:1148-1149`（退休页） | LLM 建议获取失败 | `llmSuggestionError` 原样渲染 |

同时 `F:\workspace\PDF2Test\src-tauri\src\processing\scheduler.rs:442-444` 的注释「完整错误在应用日志里」**不成立**：函数内无任何日志调用，`last_error_code` 只保留首个 `:` 前的片段并截断 80 字符，完整错误被静默丢弃。也就是说 §15.2「用户看人话、日志留细节」的两半——**用户侧仍泄漏，日志侧根本没留**。

### §15.3 与 §12.4/§12.5 的重试语义冲突

- §12.4：「重试只重跑失败阶段，已有 valid local candidate 不重复解析。」
- 实现：`F:\workspace\PDF2Test\src-tauri\src\processing\queue.rs:257-271` 的 `retry` 把 `stage='queued'`、`local_status='not_started'`、`cloud_status='not_started'`、`reconcile_status='not_started'`、`last_error_code=NULL` 全部重置——**整条链从头重跑**，本地识别必然重复执行。
- 附带冲突：`reconcile_status` 在成功路径被硬编码 `"succeeded"`（`scheduler.rs:351`），在 `retry` 中又被重置为 `not_started`（`queue.rs:263`）；而**调度器从不进入 `reconciling` 阶段**（`queue.rs:14-20` 无 `STAGE_RECONCILING` 常量；`scheduler.rs` 的 `advance` 调用只出现在 `local_recognition`、`cloud_recognition`、`ready_for_review`、`failed`、`cancelled`）。因此 `F:\workspace\PDF2Test\src\features\library\libraryTypes.ts:130` 的文案「正在合并本地与云端结果」是**不可达死代码**，`queue.rs:103,157,240,249` 中 SQL 里的 `'reconciling'` 亦为防御性死分支。

---

## 发现清单

### A7-F01 [P0] §8 的合并引擎整章零实现，计划把「云端只读对照」误述为「确定性对齐合并」

- 结论：`ReconciliationEngine`、`align_task_groups`、`TaskAlignment`、`bipartite_max_weight_matching`、`can_auto_fill`、`CandidateField`、`MergeDecision::ProposalOnly`、`LocalRecognitionCandidateV1`、`CloudRecognitionCandidateV1` 全部零命中。§8.1–§8.4 是纸面设计（`doc-only`）。
- 证据：全仓 Grep（见核对表 #1–#7）；`F:\workspace\PDF2Test\src-tauri\src\llm_suggestions.rs:235,253`（唯一云端契约 = comparison only）；`F:\workspace\PDF2Test\src-tauri\src\auto_pipeline.rs:2067,2451`（canonical 只由本地 `ir` 编译）；`task_plan.md:32`（M5 = pending）。
- 影响：§8.3 的 10 行矩阵与 §8.4 的 8 条件全部没有落点；任何依赖「云端补全缺失字段」的验收项都无法通过。计划第 8 章若被当作已完成章节，会直接误导 M5 的工作量估计。
- 建议：把 §8 整章显式标注为 `M5 scope / not started`；在 §8.1 图中把当前真实数据流（Local → Canonical DS；Cloud → comparison-only audit）画成「现状」，与「目标」分离。

### A7-F02 [P0] §8.7 的 `user_edited` 保护只有写入没有读取，`ProposalOnly` 不存在——M5 落地后必然覆盖用户编辑

- 结论：`provenanceStatus = user_edited` 的**写入**已实现且有测试（`repository.rs:205-223,306,539-543`；`authoring_v2_commands.rs:2091-2103`），但 `src-tauri/src` 中**没有任何一处读取该字段做写入拦截**，`ProposalOnly`/`MergeDecision` 零命中。当前不出现覆盖，仅因为云端不写 canonical。
- 证据：`F:\workspace\PDF2Test\src-tauri\src\library\repository.rs:205-223`（只写）；`F:\workspace\PDF2Test\src-tauri\src\llm_suggestions.rs:664-737`（`apply_suggestion_to_authoring` 无 provenance 检查）；`F:\workspace\PDF2Test\src-tauri\src\llm_commands.rs:440`（写回 `authoring-ir.json`）；Grep `ProposalOnly|MergeDecision` → 0 命中。
- 影响：这是一个**假护栏**。M5 引入完整云端候选后，合并器会直接覆盖用户已编辑的题干/选项/答案，且没有任何测试会失败——因为保护逻辑从未存在。
- 建议：在 M5 之前先落地 `MergeDecision` 与「`provenance_status == UserEdited → ProposalOnly`」的**纯函数 + 单元测试**，即使合并器尚未存在；并在 `apply_suggestion_to_authoring` 的旧链入口补 provenance 检查（该入口当前可被退休页直接调用）。

### A7-F03 [P1] §8.5 issue 命名四方不一致，前端自述的「与后端一致」为假；后端死表从未读写

- 结论：计划 §6.8 / 后端 `issue_codes.rs` / 前端 `actionableIssues.ts` / `authoring_review.rs` 四方 code 集合互不相同（详见上表）；`actionable_issues_v1` 表只建不用；`get_workspace_item` 已返回的后端 `ReviewIssueV2[]` 被前端丢弃。
- 证据：`F:\workspace\PDF2Test\src-tauri\src\library\schema.rs:87-104`（DDL，无读写）；`F:\workspace\PDF2Test\src-tauri\src\ielts_grammar\issue_codes.rs:7-53`（47 个 code）；`F:\workspace\PDF2Test\src\features\editor\actionableIssues.ts:11-27,105-162`（6 个 code，自述与后端一致）；`F:\workspace\PDF2Test\src-tauri\src\library\commands.rs:33-40,57`（返回 issues）；`F:\workspace\PDF2Test\src\features\editor\ExamWorkspacePage.tsx:67`（忽略之）。
- 影响：P8 迁移时「两边对比结果」不可执行——没有共同的 code 词表；工作区同时跑两套 issue 体系，后端的 `sourceAnchors`/`suggestedActions` 能力被浪费。
- 建议：先冻结一份权威 code 词表（建议以 `issue_codes.rs` 为超集、§6.8 为必须子集），让前端从后端 `issues` 渲染，删除 `deriveActionableIssues`；把 `actionable_issues_v1` 要么接入读写，要么删除以免误导。

### A7-F04 [P1] §15.1/§15.2 的类型零实现，错误分层退化为各处手写字符串映射，且原始机器码仍在多条路径直送 UI

- 结论：`UserErrorCategory`、`AppErrorV2`、`internal_detail_id`、`normalizeAppError`、`JSON_SCHEMA_REQUIRED` 全仓仅命中计划文档。F-M0-3 只修了 workspace 加载路径。
- 证据：核对表 #16–#20、#28；泄漏点见「§15.2 原始错误泄漏残留」表（`ExamWorkspacePage.tsx:80,100`、`LibraryPage.tsx:113,122`、`useImportFiles.ts:8`、`SettingsPage.tsx:150,176,191`、`UnifiedPreview.tsx:1148`）；`tauriCommands.ts:78-84`（无归一化）。
- 影响：用户在「发布前保存失败」「删除失败」「导入失败」「设置保存失败」等高频路径仍会看到 `EDIT_VERSION_CONFLICT:current=2:base=1`、`ITEM_DS_NOT_SEEDED:<id>`、`library_v2_tx:<sqlite>` 这类内部串。§15.2 的核心承诺（用户/内部错误分离）未达成。
- 建议：在 `src/api/tauriCommands.ts` 的 `command()` 出口做一次集中归一化（稳定 code 前缀 → 用户文案 + `internalDetail`），各调用点只渲染用户文案；开发者模式另开附注。

### A7-F05 [P1] 内部细节「记录到日志」不成立：`fail_job` 丢弃完整错误且无日志写入

- 结论：`scheduler.rs:442` 注释称「完整错误在应用日志里」，但函数体内无任何日志调用，完整 error 被丢弃，DB 只留首段 code。
- 证据：`F:\workspace\PDF2Test\src-tauri\src\processing\scheduler.rs:441-455`；`src-tauri/src/processing/` 全目录 Grep `eprintln|log::|tracing` 仅命中 `scheduler.rs:122,132,136,138` 的恢复/事件日志，**无失败任务日志**。
- 影响：§15.2「日志中才记录 JSON_SCHEMA_REQUIRED…request_id」的设计前提（有内部日志通道）不存在；真实故障无法事后定位，且与「用户看人话」配套的内部留痕缺失。
- 建议：`fail_job` 至少 `eprintln!` 完整 error（或写入 job 目录的诊断文件），并在 DB 保留稳定 code。

### A7-F06 [P1] §15.3 第 2 行与调度实现直接矛盾；第 1 行的「云端未完成」缺失且 `reconcile_status` 硬编码为成功

- 结论：本地失败时云端从不运行（`scheduler.rs:289-292` 提前 `fail_job` + `return`），故「本地失败、云端成功 → 提供云端稿」不可能发生；云端失败时 `reconcile_status` 仍被写成 `"succeeded"`（`:351`），题库也无「云端未完成」文案。
- 证据：`F:\workspace\PDF2Test\src-tauri\src\processing\scheduler.rs:289-292,340-359`；`云端未完成` 全仓零命中；`F:\workspace\PDF2Test\src\features\library\libraryTypes.ts:127-136`（`detailFor` 不引用 cloud 状态）。
- 影响：用户在云端失败时看到「已就绪」，无从得知云端未完成；计划 §6「两条链并发」与 §15.3 第 2 行的降级承诺均未兑现。
- 建议：`reconcile_status` 应由真实的本地/云端状态推导（至少区分 `succeeded/skipped/failed`），题库行增加「云端未完成（可重试）」提示；若确实要「本地失败云端仍跑」，需先改调度顺序。

### A7-F07 [P1] §8.6 原位 diff UI 完全不存在，`reconciling` 阶段为不可达死代码

- 结论：`ExamCanvas` 无任何云端 diff 渲染；`采用/忽略` 仅在退休页。DB/前端宣称的 `reconciling` 阶段调度器从不进入。
- 证据：`src/exam-canvas` Grep `cloud|diff|suggestion|popover|line-through` → 唯一命中 `ExamCanvas.tsx:339`（答案位 badge）；`F:\workspace\PDF2Test\src-tauri\src\processing\queue.rs:14-20`（无 `STAGE_RECONCILING`）；`scheduler.rs` 的 `advance` 调用集合；`F:\workspace\PDF2Test\src\features\library\libraryTypes.ts:130`（不可达文案）；`task_plan.md:125-126`（自认口径修正）。
- 影响：用户看不到任何云端差异，「云端差异原位显示」这一 M3/M5 出口条件无法验收；`reconciling` 死阶段会让进度条出现永远走不到的分支（`scheduler.rs:82` 权重 85）。
- 建议：在 M5 落地前删除或明确标注 `reconciling` 为保留阶段；把 §8.6 的验收拆为「云端 diff 数据可用」与「Canvas 渲染」两步。

### A7-F08 [P2] §15.3 第 6 行保存冲突处理只做了「一句提示」，无自动拉取与重放

- 结论：冲突时仅设置 `saveState="conflict"` 与顶层文案，无 `reload()`、无命令重放、无 node 级 diff。
- 证据：`F:\workspace\PDF2Test\src\features\editor\useCanonicalEditor.ts:121-130`；`F:\workspace\PDF2Test\src\features\editor\ExamWorkspacePage.tsx:169`（顶部渲染）。
- 影响：多窗口场景（F03 已修 request id，但版本冲突仍会发生）用户只能手工处理；计划承诺的「自动拉取 + 重放」缺失。
- 建议：至少在冲突时自动 `reload()` 并保留本地 pending 供用户选择「重放/放弃」。

### A7-F09 [P2] §15.3 第 7 行发布阻止无法定位：blocker 的 `targetId` 恒为 null 且被前端丢弃

- 结论：`check_publish_preflight` 产出的 blocker 有 `targetId`/`action` 字段，但当前实现几乎全部填 `null`；`publishClient` 只取 `userMessage`，丢弃定位信息。
- 证据：`F:\workspace\PDF2Test\src-tauri\src\authoring_v2_commands.rs:360-390`（`"targetId": null`）；`F:\workspace\PDF2Test\src\api\publishClient.ts:22-29`（只读 `userMessage`）。
- 影响：发布失败时用户只得到一句话，必须自己在 Canvas 里找问题；与 §13.4/§15.3 的「点击定位」目标不符。
- 建议：为每个 blocker 填充真实 `targetId`（slot/responseGroup/task id），前端透传并在点击时 `scrollIntoView`。

### A7-F10 [P2] §12.4「重试只重跑失败阶段」与 `retry` 实现冲突

- 结论：`retry` 重置全部阶段与状态，整链从头重跑。
- 证据：`F:\workspace\PDF2Test\src-tauri\src\processing\queue.rs:257-271`。
- 影响：本地识别（最贵的阶段）在重试时必然重复执行，与 §12.4「已有 valid local candidate 不重复解析」冲突，也影响批量重试耗时。
- 建议：重试时保留 `local_status='succeeded'` 与已产出的本地工件，只重置 `cloud_status`/`reconcile_status`。

### A7-F11 [P3] §8.5 前端 `ActionableIssueV1` 缺 7 个字段，且 3 条示例文案与动作按钮均未落地

- 结论：前端结构仅 5 字段；计划 3 条示例文案零命中；无 `[查看]/[补充]/[合并]` 动作。
- 证据：`F:\workspace\PDF2Test\src\features\editor\actionableIssues.ts:19-27`；`ExamWorkspacePage.tsx:175-184`。
- 影响：issue 列表只能滚动定位，不能一键执行建议动作；`suggestedAction` 的能力缺失。
- 建议：随 A7-F03 的 P8 迁移一并补 `suggestedAction` 与动作按钮。

---

## 与历史审计的关系

| 历史项 | 本次核实 |
|---|---|
| `audit-2026-09-07/report.md` M5「Cloud gateway emits comparison-only CloudReadingOutlineV1 plus answer diagnostics. No full candidate/skill/repair/salvage/three-way merge path was found.」 | **一致且更细**：本次从 §8 规格侧确认 `ReconciliationEngine`/`align_task_groups`/`can_auto_fill` 亦零实现；并补充「§8.7 保护只有写入无读取」这一新发现（A7-F02） |
| `task_plan.md` 已知差距 #4「`ActionableIssue` 是前端派生的」 | **成立**，并补充：后端存在一张从未读写的 `actionable_issues_v1` 表（`schema.rs:87-104`），以及 `get_workspace_item` 已返回的后端 issues 被前端忽略（`commands.rs:33-40,57` vs `ExamWorkspacePage.tsx:67`）——比 task_plan 的描述更严重 |
| `task_plan.md` S3.1 口径修正「IssuePopover 实为中央问题列表 + scrollIntoView」 | **成立**：`ExamWorkspacePage.tsx:171-186` |
| `repair-2026-09-07/progress.md` F-M0-3「workspace load errors and editor patch/conflict failures showed raw machine codes and paths. Users now get user-level text」 | **部分修复**：加载与编辑保存路径已分层（`ExamWorkspacePage.tsx:30-37`、`useCanonicalEditor.ts:13-20`）；但**发布前 `flush()` 失败（`:80`）、发布结果兜底（`:100`）、删除/恢复（`LibraryPage.tsx:113,122`）、导入（`useImportFiles.ts:8`）、设置（`SettingsPage.tsx:150,176,191`）仍泄漏原始串**——修复范围比 progress.md 声称的窄 |
| `task_plan.md:157` 记录 AUTHORING_V2_NOT_AVAILABLE「降级 UI 把原始错误码+路径暴露给用户」 | **该具体点已修复**（`ExamWorkspacePage.tsx:32-33` 映射 + `:203-205` 开发者模式门），但同类问题在其它命令路径仍存在 |
| 同批 `audit-2026-09-12/A2`、`A6` 关于 `CloudRecognitionCandidateV1` 零命中 | **一致**：本报告从 §8 消费端独立复核，结论相同 |
| 同批 `audit-2026-09-12/A4` 第 27 条「云端失败仍推进 ready_for_review」 | **一致**；本次进一步指出 `reconcile_status` 被硬编码 `"succeeded"`（`scheduler.rs:351`），比 A4 的描述更具体 |

---

## 证据层级与局限

| 层级 | 覆盖内容 |
|---|---|
| `product` | 无。本次为纯静态审计，未启动 Tauri 应用、未执行真实导入/编辑/发布，未验证运行时刻行为 |
| `command` | 无。未运行 `cargo test`、`npm run check/build`、任何 E2E 脚本 |
| `static` | 全部核对项（Grep/Read/Glob 于 `HEAD 47a3806` + 未提交工作区）。关键判定均给出绝对路径:行号 |
| `doc-only` | §8.1–§8.4、§8.5 的后端结构、§8.6、§8.7 的 `ProposalOnly`、§15.1、§15.2 的 `AppErrorV2`/`normalizeAppError`、§15.3 第 2/3/6 行的自动行为 |

局限：

1. 未运行 `cargo build`/`cargo test`（按要求禁止），因此「零命中」结论基于源码文本搜索，不排除宏/代码生成产生的符号；但已用 `CloudReadingOutlineV1`（唯一已知云端契约）做阳性对照，确认搜索口径有效。
2. `src-tauri/target/**` 下的 `.d` 依赖清单文件未纳入搜索范围；本次所有 Grep 均限定 `src-tauri/src` 与 `src`，若存在 `build.rs` 生成代码可能漏检（已确认无 `build.rs` 相关生成物命中这些符号）。
3. 未核实「云端失败但本地成功」在真实 Tauri 运行时的题库渲染结果——`reconcile_status` 硬编码为 `"succeeded"` 是静态结论，未做运行时刻观测。
4. F-M0-3 的「原始错误码泄漏」本次只做了静态调用链追踪，未在真实 UI 中复现（`repair-2026-09-07` 的原始复现证据未在本次环境重跑）。
5. 未评估 §8/§15 与第 19 章测试用例清单的覆盖映射（属其它审计子代理范围）。
6. 工作区新增了未跟踪的 `src-tauri/src/recognition/{mod.rs,local/{mod.rs,question_blocks.rs}}`（M4/P4-T02 本地 Question Layout Graph 起步件，产出 `QuestionBlockCandidateV1`/`OptionBankCandidateV1`/`VisualStimulusCandidateV1` 等**纯本地**候选）。已专门 Grep 该目录的 `Reconcil|align_task_groups|MergeDecision|ActionableIssue|provenance`：**仅命中 `mod.rs:9` 关于「不得伪造 provenance」的注释**，无任何合并/云端候选/issue 类型。因此本报告 §8/§15 的全部结论不受该新目录影响。
