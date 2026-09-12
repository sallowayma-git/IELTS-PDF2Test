# A6 — 第 7 章（云端完整识别、Prompt/Skill 与 JSON 修复链）对抗审计

- 审计对象：`Plan With Files/IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md` 第 1023–1401 行（§7.1–§7.14）
- 审计日期：2026-09-12
- 代码基线：`HEAD = 47a38064aef58e023bce58e5b6feafa99031b252` + 41 个未提交改动文件（`git status --porcelain`）
- 立场：默认每条论断为错/过时/未完成，用代码证据逐条证伪
- 只读审计；未修改任何产品代码；未运行 cargo/npm 构建，未发起任何真实 LLM 请求

---

## 章节范围

第 7 章描述的是「云端第二条完整 PDF→题链」：versioned Skill Bundle（§7.2）、`CloudRecognitionCandidateV1`（§7.3）、`CloudRecognitionRequestV1`（§7.4）、System Prompt 10 约束（§7.5）、`CloudResponseState` 8 态状态机与一次修复（§7.6/§7.8）、分组 salvage（§7.9）、第二条校对链（§7.10）、Evidence Resolver 与 ID 重分配（§7.11）、大文件分片（§7.12）、校对触发条件（§7.13）、Provider 收敛（§7.14）。

本次核查的总体结论：**§7 除 §7.1 现状描述与 §7.14 的部分前提外，全部为纸面设计，`src-tauri` 中零实现。** 现有云端能力仍是 §7.1 所述的 `CloudReadingOutlineV1`「只读对照提纲」，与 `task_plan.md` 中 M5 = `pending`、`audit-2026-09-07/report.md` 的 M5 评估一致。

---

## 断言核对表

| # | 章节断言 | 判定 | 证据（绝对路径:行号） |
|---|---|---|---|
| 1 | §7.1 当前云端 contract = title/groups[].range/kind/layoutHint/questionIds/notesText/answerKey/confidence/warnings，且限定 comparison only | **HOLDS** | `F:\workspace\PDF2Test\src-tauri\src\llm_suggestions.rs:234-251`（`outputContract.schema="CloudReadingOutlineV1"` 与 shape）；`F:\workspace\PDF2Test\src-tauri\src\llm_gateway.rs:484-491`（prompt 首句 "comparison-only outline"）；`F:\workspace\PDF2Test\src-tauri\src\llm_suggestions.rs:253`（"Return an outline for comparison only"）；`F:\workspace\PDF2Test\src-tauri\src\llm_gateway.rs:1176-1365`（`validate_cloud_outline_output` 即事实上的 schema）。注：实际 group 还含 `confidence`/`evidence`/`rawKind`，§7.1 清单未列，非实质性偏差 |
| 2 | §7.2 `recognition/skills/ielts-reading-v1/` 目录与 8 类题型示例存在 | **FALSE** | `Glob recognition/**` → 0 命中；`Glob **/ielts-reading-v1/**` → 0 命中；`ls recognition` → 无此目录。manifest.json/system.md/conversion-rules.md/task-taxonomy.json/output.schema.json/renderer-contract.md/error-policy.md/examples/*.json 全部不存在。**整个 skill bundle 是纸面设计（doc-only）** |
| 3 | §7.3 `CloudRecognitionCandidateV1` / `CloudTaskGroupCandidate` / `CloudEvidence` / `unresolved_regions` / `answer_key_candidates` 结构体存在 | **FALSE** | 对 `src-tauri/src` Grep `CloudRecognitionCandidate\|CloudTaskGroupCandidate\|CloudEvidence\|unresolved_regions\|CloudResponseState` → 0 命中。`answer_key_candidates` 仅命中 `F:\workspace\PDF2Test\src-tauri\src\authoring_pipeline.rs:134`，是**本地 V1** `SplitCandidatesV1.answer_key_candidates: Vec<AnswerKeyCandidateV1>`（`:123-126`），与云端无关。无任何 Rust/TS/JSON Schema 定义 |
| 4 | §7.4 `CloudRecognitionRequestV1`（skill_bundle/source/page_manifest/local_hints/output_contract）与 `local_hints` 白名单 | **PARTIAL** | 现有请求体见 `F:\workspace\PDF2Test\src-tauri\src\llm_suggestions.rs:212-264`：`mode/job/profile/sourceFile/pdfPath/pages/extractionWarnings/outputContract`。有 `output_contract`（弱化为 `outputContract`）、`source`（弱化为 `sourceFile`）、`page_manifest`（弱化为 `pages`）；**无 `skill_bundle`、无 `local_hints`**（`extractionWarnings` 是其弱化替身），白名单未实现、不可强制 |
| 5 | §7.5 System Prompt 10 条约束 | **FALSE** | 现有 system message 为字面量 `"Return valid JSON only."`：`F:\workspace\PDF2Test\src-tauri\src\llm_gateway.rs:630,661,692,725`（+`:752` 图像回退）。10 条中仅第 10 条（不生成 JS/HTML/Markdown）与第 4 条（evidence，且只有 quote 无 bbox）部分出现在 user prompt；第 3/5/6/7/8/9 条（逐字保留题干选项、visual region 回退、Questions X and Y 多答案位、option bank、物理顺序≠左右栏、unresolved）完全缺席。且 outline 契约根本没有 prompt/options 字段，约束 3 不可执行 |
| 6 | §7.6 `CloudResponseState` 8 态与 `process_cloud_response`（extract→normalize→schema→semantic→repair→salvage→rejected） | **FALSE** | `CloudResponseState`/`process_cloud_response` 全仓 0 命中。现有链仅为：JSON 提取 `F:\workspace\PDF2Test\src-tauri\src\llm_gateway.rs:219-288`（`parse_llm_json_content`/`balanced_json_end`）→ HTTP 层 3 次重试 `:290-318` → 单次全量校验 `:1176-1365` → 失败即整份 Err（或对 PDF 直传失败时改走图像重发 `:733-761`）。**无 normalize 阶段入口、无 JSON Schema 校验、无 semantic 校验、无 repair、无 salvage** |
| 7 | §7.7 安全归一化白名单（single-choice→single_choice、TFNG→true_false_not_given、number string→int、null→[]、trim label、1-based→0-based）与 5 条「不得自动补」 | **PARTIAL** | 已实现：连字符/空格→下划线、`note_completion→summary_completion`（`F:\workspace\PDF2Test\src-tauri\src\llm_gateway.rs:1076-1085`）；答案 key 字符串→整数且限 1..=200（`:1000-1007`）；缺失 `warnings`→`[]`（`:1201`）；答案 trim/大写归一（`:1009-1030`）。**未实现：`TFNG` 别名**（`normalize_cloud_outline_kind("TFNG")` 走 `allowed_question_kind` 失败→整组 reject，`:1082`）、**1-based→0-based 未实现**。5 条「不得自动补」：未发现「缺失选项自动生成 A/B/C/D」「缺失 slot 默默补齐」「缺失 evidence 伪造 anchor」「未知 task type 降级为 short_answer」的云端行为（未知 kind 一律 reject，`:1221-1222`/`:1325-1326`）。**未构成 P0** |
| 8 | §7.8 一次受约束修复请求 payload 与「最多一次」 | **FALSE** | 全仓无 `request_constrained_repair`/`originalOutput`/`validationErrors` 修复请求构造。现有唯一重试是 HTTP 层对 429/5xx 的同请求重发（`F:\workspace\PDF2Test\src-tauri\src\llm_gateway.rs:293-318`），不是 JSON 修复 |
| 9 | §7.9 `salvage_valid_task_groups` 与前端「云端识别已补充 N 个题组；M 个未采用」；不显示 serde/schema 原始错误 | **FALSE** | `salvage_valid_task_groups` 0 命中。前端 Grep `云端识别已补充\|未采用\|CLOUD_RESULT_UNUSABLE\|salvage` → 0 命中。当前行为是整份 outline 校验失败→`cloud_outline_report_from_output` 置 `failure`（`F:\workspace\PDF2Test\src-tauri\src\auto_pipeline.rs:1142-1149`），并原样写入 canonical `authoring-ir.json` 的 `audit.issues[].failure`（`:2391`）；`src/types/settings.ts:130` 暴露 `failure?: string \| null`。活动 UI（`src/features/**`）未渲染该字段，但原始错误串**已进入权威稿审计**，与 §7.9「不显示原始错误」精神相悖 |
| 10 | §7.10 `ReconciliationReviewRequestV1` / `ReconciliationProposalV1` / `FieldProposal` | **FALSE** | 三者 0 命中。当前「第二条调用」实为 `vision_answer_candidate_for_job`（答案抽取），与 outline 在同一 worker 内**顺序执行**（`F:\workspace\PDF2Test\src-tauri\src\auto_pipeline.rs:1254-1261`），不是基于 deterministic diff 的字段级校对；云端结果只写 `cloud_comparison_summary` 审计项（`F:\workspace\PDF2Test\src-tauri\src\auto_pipeline.rs:2374-2413`）。**无 proposal、无 three-way merge** |
| 11 | §7.11 `resolve_cloud_evidence`、bbox/quote 一致性、`IdAllocator` 重分配 | **FALSE** | 三者 0 命中；`normalize_bbox`/`nodes_overlapping`/`find_quote_normalized` 0 命中。当前**风险未兑现**，因为云端输出从不进入 canonical（`F:\workspace\PDF2Test\src-tauri\src\auto_pipeline.rs:2367-2413` 只 replay 审计项，`src/types/settings.ts:125-131` 明确 comparison-only）。但前置护栏缺失：一旦按 §7.3 引入完整候选，模型 `candidateId/nodeId/slotId` 将无重分配机制直接落地。仅本地建议链有 quote↔source 校验（`F:\workspace\PDF2Test\src-tauri\src\llm_suggestions.rs:402-449`），云端无对应物 |
| 12 | §7.12 <=15 页整卷 / >15 页按题组分片；禁止截断后当完整结果 | **FALSE** | 无 15 页阈值、无分片、无 page manifest/coverage 断言（Grep `15 pages\|page_limit\|shard\|分片\|page_manifest` → 0 命中）。响应体有硬上限 `MAX_LLM_RESPONSE_BYTES=16MB`（`F:\workspace\PDF2Test\src-tauri\src\llm_gateway.rs:20,407-417`）会报错而非静默截断；但**provider 端静默丢弃多页 PDF 时本地无覆盖校验**，仍会被当作完整结果参与对照 |
| 13 | §7.13 校对调用 5 条触发条件 | **FALSE** | 无触发判断。只要存在启用 profile 且主源为 PDF，`run_cloud_review_core` 即无条件调用（`F:\workspace\PDF2Test\src-tauri\src\auto_pipeline.rs:2260-2271`），并由 `processing/scheduler.rs:337` 后台排队 |
| 14 | §7.14 当前 UI 可选 `AnthropicCompatible`/`Custom`，第一阶段从普通 UI 删除 | **PARTIAL（前提已过时）** | **活动设置页早已只列两种**：`F:\workspace\PDF2Test\src\features\settings\SettingsPage.tsx:29-32`（`SUPPORTED_PROVIDERS`）+ `:320-322`，注释 `:21` 明确「移除 AnthropicCompatible / Custom（findings F18）」。`AnthropicCompatible`/`Custom` 只残留在**孤儿页** `F:\workspace\PDF2Test\src\pages\Settings.tsx:288-292`（全仓无 import，`src/app/App.tsx:5` 用的是 features 版）。后端 `F:\workspace\PDF2Test\src-tauri\src\llm_commands.rs:78-83` 仍接受 4 种，`src/types/settings.ts:2` `LlmProvider` 仍为 4 种；gateway 只接受 2 种（`F:\workspace\PDF2Test\src-tauri\src\llm_gateway.rs:146`）。即「UI 删除」已完成，但 plan 的现状描述与代码不一致 |
| 15 | §19.3 Cloud Contract 17 个用例的测试覆盖 | **FALSE（≈3/17）** | 无 `src-tauri/tests/` 目录、无 `fixtures/**/*cloud*`。见下「§19.3 覆盖对照」 |
| 16 | §24 DoD 云端 5 条 | **1/5** | 「cloud 失败不阻止打开本地稿」成立（`F:\workspace\PDF2Test\src-tauri\src\auto_pipeline.rs:2318-2333,2336-2339`）；versioned skill bundle / 完整 `CloudRecognitionCandidateV1` / validate-repair-salvage / 第二条校对只产 proposal 四条均未实现 |
| 17 | §26.3 攻击 D：`llm_gateway.rs` 改 `LlmTransport: Send + Sync`，不再 `Mutex<FnMut>` | **FALSE** | `LlmTransport` 0 命中。仍是 `Mutex<F>` + `F: FnMut(...)`：`F:\workspace\PDF2Test\src-tauri\src\auto_pipeline.rs:1215-1219`（`lock_gateway`）、`:1231-1235`、`:1254-1261`（两次 `lock_gateway` 串行）、`:1389`（`F: ... + Send` 仅单参数约束）。计划自身在 §26.3-D 提出的修订要求**未落地** |

---

## 契约与状态机核查

### §7.3 候选契约 vs §4.2「禁止 Cloud raw JSON 写回 canonical」
**一致，且当前未违背。** §4.2（计划 `:374-380`）把 `Cloud raw JSON -X-> Canonical DS` 列为禁止反向写入；§7.3 的 candidate 通过 §7.11 的 evidence resolver + ID 重分配后再入 canonical。当前实现里云端输出只进 `audit.issues`（`F:\workspace\PDF2Test\src-tauri\src\auto_pipeline.rs:2374-2413`），确实没有写回 canonical，符合 §4.2。**但**：§7.11 的「不得直接成为 canonical sourceAnchors」这一步没有任何实现，§4.2 的约束目前是靠「候选根本不存在」而非靠护栏实现的——一旦补上 §7.3，护栏缺口立即成为真实风险。

### §7.7 白名单 vs §7.6 状态机
**内部基本自洽，但伪代码有一处与 §7.8 冲突。** §7.6 伪代码在提取失败分支调用 `request_repair_or_fail(raw, vec![err], ctx)`，即用「无 parsedCandidate」的原始文本请求修复；而 §7.8 规定修复 payload 必须含 `parsedCandidate`。当 JSON 完全不可解析时 `parsedCandidate` 只能为空，§7.8 的受约束修复契约对该分支不适用。此外 §7.6 伪代码在 `normalize` 之后才做 schema 校验，与 §7.7「本地可自动修正」的定位一致，无矛盾。

### §7 与第 24 章 DoD 云端条款
第 24 章云端 5 条（计划 `:3924-3930`）与 §7 一一对应，但仅第 5 条成立（见核对表 #16）。DoD 未定义「候选在何处持久化」「proposal 的接受/拒绝 UI」，§7.11 只提到 transient workspace + TTL，缺少与 §4.3/§4.4 的存储契约衔接。

### 与 §26.3 攻击修订要求
§26.3 的四项修订中，仅 A（Evidence Resolver，计划 `:4081-4084`）与 B/C/E 已写成 §7.11/§7.13/§7.12/§7.9 文本，但**均未实现**；D（`LlmTransport: Send + Sync`，计划 `:4110`）连文本对应物都没有出现在 §7，且代码仍是 `Mutex<FnMut>`。§26.3 结论「通过」（计划 `:4124`）是对**设计文本**的通过，不是对代码的通过。

### §19.3 Cloud Contract 17 用例覆盖对照

| # | 用例 | 现状 | 证据 |
|---|---|---|---|
| 1 | 纯 JSON 正常结果 | 部分 | `F:\workspace\PDF2Test\src-tauri\src\llm_gateway.rs:219-223`；测试 `lib.rs:10561` 未覆盖「纯 JSON」分支（用的是包裹文本） |
| 2 | Markdown JSON 代码围栏 | 代码可处理，无测试 | 围栏被 `parse_llm_json_content` 的花括号扫描吞掉（`:224-248`），无专门用例 |
| 3 | JSON 前后说明文字 | 有测试 | `F:\workspace\PDF2Test\src-tauri\src\lib.rs:10561-10570` |
| 4 | 两个 JSON 对象必须拒绝歧义 | 代码有分支，无测试 | `F:\workspace\PDF2Test\src-tauri\src\llm_gateway.rs:244-248`（`ambiguous_wrapped_json`） |
| 5 | 非法枚举 | 代码有分支，无测试 | `:1221-1222`、`:1325-1326` |
| 6 | 缺 taskGroups | 代码有分支，无测试 | `:1190-1191` |
| 7 | 缺一个 option 正文 | **不适用/未实现** | outline 契约无 options |
| 8 | range 与 slot 数不一致 | 代码有分支，无测试 | `:1253-1258` |
| 9 | bbox 超出 0-1 | **不适用/未实现** | outline 无 bbox |
| 10 | evidence quote 不在 source page | **未实现**（仅校验非空） | `:1296-1311` 只查 `pageIndex!=0` 与 text 非空 |
| 11 | repair 成功 | **未实现** | 无 repair |
| 12 | repair 失败但 3/4 groups salvage | **未实现** | 无 salvage |
| 13 | 全部不可用 | 部分 | 整份 Err 路径 `:1142-1149`，无 salvage 语义 |
| 14 | HTTP 429 + Retry-After | 已实现 + 有测试 | `F:\workspace\PDF2Test\src-tauri\src\llm_gateway.rs:320-355`；`lib.rs:10505-10533` |
| 15 | timeout | 已实现，无专门测试 | `:121-129`、`:292-313` |
| 16 | provider 不支持 PDF → image fallback | 已实现，无测试 | `:733-761` |
| 17 | prompt 改写被 evidence/quote 拒绝 | **未实现**（云端） | 仅本地建议链有 `F:\workspace\PDF2Test\src-tauri\src\llm_suggestions.rs:402-449` |

相关既有测试仅覆盖「云端对照不覆盖本地内联 notes」与「对照差异写入审计」两个业务行为：`F:\workspace\PDF2Test\src-tauri\src\lib.rs:6120`、`:6358`，与 §19.3 的 17 条契约用例不重叠。

---

## 发现清单

### A6-F01 [P0] 云端完整候选契约零实现，云端仍是「只读对照提纲」
- 结论：§7.3/§7.4 的 `CloudRecognitionCandidateV1`、`CloudTaskGroupCandidate`、`CloudEvidence`、`unresolved_regions`、`CloudRecognitionRequestV1` 在 `src-tauri` 零命中；云端唯一输出仍是 `CloudReadingOutlineV1`（`F:\workspace\PDF2Test\src-tauri\src\llm_suggestions.rs:235`）。
- 证据：`F:\workspace\PDF2Test\src-tauri\src\llm_suggestions.rs:234-251`；`F:\workspace\PDF2Test\src-tauri\src\llm_gateway.rs:484-491,1176-1365`；`task_plan.md` M5=`pending`。
- 影响：用户要求的「第二条完整 PDF 转题链」不存在；云端无法提供 passage ContentDoc、逐题 prompt、options、option bank、answer slot、visual stimulus、可渲染结构。
- 建议：按 §7.3 落地 typed struct + JSON Schema，并把 `outputContract` 从 prose shape 升级为可校验 schema；在此之前第 7 章不应被视为可实施规格。

### A6-F02 [P0] 无 JSON 修复/分组 salvage，单点校验失败即整份丢弃
- 结论：§7.6/§7.8/§7.9 全部未实现；现有链是「一次提取 + 单次全量校验 + 失败即 Err」。
- 证据：`F:\workspace\PDF2Test\src-tauri\src\llm_gateway.rs:219-288,290-318,1176-1365`；`F:\workspace\PDF2Test\src-tauri\src\auto_pipeline.rs:1142-1149`。
- 影响：与 §26.3-E 描述完全一致——39 个正确题组会因 1 个坏题组被整体拒绝，云端对照价值被结构性削弱。
- 建议：先实现分组级 salvage（成本最低、收益最大），再实现一次受约束 repair。

### A6-F03 [P1] 版本化 Skill Bundle 目录完全不存在
- 结论：`recognition/skills/ielts-reading-v1/` 及其 8 类题型示例不存在；`manifest.json` 的 `skillId/version/outputSchema/schemaSha256/minimumClientVersion` 无任何代码读取点。
- 证据：`Glob recognition/**`=0、`Glob **/ielts-reading-v1/**`=0；请求体仅含 `outputContract`（`F:\workspace\PDF2Test\src-tauri\src\llm_suggestions.rs:234-262`）。
- 影响：prompt/schema/题型定义仍硬编码在 Rust 字符串（`F:\workspace\PDF2Test\src-tauri\src\llm_gateway.rs:440-491`），13 种题型枚举在仓库中被复制 4 份、无版本锚点，schema 漂移不可检测。
- 建议：先建目录与 `output.schema.json`，并把 `schemaSha256` 与请求绑定；否则「同步 prompt/skill/题型/schema」不可验收。

### A6-F04 [P1] 第二条校对链缺失，且未评估 100 份批量成本
- 结论：`ReconciliationReviewRequestV1`/`ReconciliationProposalV1`/`FieldProposal` 零命中；当前第二条调用是答案抽取，与 outline 顺序执行，不是 diff 驱动校对。
- 证据：`F:\workspace\PDF2Test\src-tauri\src\auto_pipeline.rs:1254-1261,2374-2413`；`src/types/settings.ts:125-131`。
- 影响：§7.13 的触发条件无从谈起（当前无条件调用，`:2260-2271`）；100 份 PDF 场景下每份至少 2 次云端调用，§26.3-B 声称的成本翻倍风险在当前代码中**未缓解**。
- 建议：实现 deterministic diff + 条件触发（§7.13）后再接校对调用；对 100 份批量给出显式调用预算与并发上限（现仅前端 `cloudConcurrency` 设置，`src/features/settings/appSettings.ts:30`）。

### A6-F05 [P1] Evidence Resolver 与 IdAllocator 缺失，canonical 入口无护栏
- 结论：`resolve_cloud_evidence`/`normalize_bbox`/`IdAllocator` 零命中。
- 证据：Grep 全 `src-tauri/src` 0 命中；对照本地链仅有 `F:\workspace\PDF2Test\src-tauri\src\llm_suggestions.rs:402-449`。
- 影响：当前风险未兑现（候选不入 canonical），但 §7.3 一旦落地，模型 `candidateId/taskId/nodeId/slotId` 将无重分配机制；§26.3-A 提出的 ID 冲突/路径注入/覆盖现有节点风险会立即变为真实缺陷。
- 建议：把 Evidence Resolver 与 ID 重分配列为 §7.3 的前置条件（blocker），而不是 §7.11 的后续项。

### A6-F06 [P1] §26.3-D 要求的 `LlmTransport: Send + Sync` 未改，gateway 仍串行
- 结论：仍是 `Mutex<F>` + `FnMut`；`LlmTransport` 0 命中。
- 证据：`F:\workspace\PDF2Test\src-tauri\src\auto_pipeline.rs:1215-1219,1231-1235,1254-1261,1389`。
- 影响：§7.13「proofreader 在 deterministic diff 后独立调度」在单 gateway mutex 下无法与第一条识别解耦；计划自身标注的修订要求未落地，§26.3「通过」结论不成立。
- 建议：先做 `LlmTransport: Send + Sync` + 可 clone 的 HTTP client 重构，再谈 §7.13 调度；并发改由 cloud semaphore 控制。

### A6-F07 [P1] 大文件阈值与页覆盖校验缺失
- 结论：无 15 页阈值、无分片、无 page manifest/coverage。
- 证据：Grep `15 pages|page_limit|shard|分片|page_manifest|PageDescriptor` 0 命中；响应体上限见 `F:\workspace\PDF2Test\src-tauri\src\llm_gateway.rs:20,407-417`。
- 影响：响应侧不会静默截断（会报 `llm_http_response_too_large`），但**provider 侧丢弃 PDF 后段页面时本地无检测**，缺失页会被当作完整结果参与对照，正是 §7.12 明令禁止的情形。
- 建议：至少引入「请求页数 = 返回覆盖页数」断言与 `page_manifest`，超限时显式报 `CLOUD_PAGE_COVERAGE_INCOMPLETE`。

### A6-F08 [P2] §7.5 的 10 条 System Prompt 约束基本未落地
- 结论：system message 仅 `"Return valid JSON only."`；10 条中 6 条完全缺席。
- 证据：`F:\workspace\PDF2Test\src-tauri\src\llm_gateway.rs:630,661,692,725,752`；约束散落在 user prompt（`:486-490`）且缺 bbox/unresolved/option bank/多答案位规则。
- 影响：视觉/文本转录质量依赖单条长 user prompt，模型行为不可预期；§7.5 的「共享题干只出现一次」「List of Headings 是 option bank」等语义规则无对应校验，即使模型违反也无人拦截。
- 建议：把 §7.5 写入 skill bundle 的 `system.md` 并版本化，同时为第 6/7/9 条补 semantic validator。

### A6-F09 [P2] §19.3 的 17 个 Cloud Contract 用例仅约 3 条有覆盖
- 结论：无 `src-tauri/tests/`、无 `fixtures/**/*cloud*`；17 条中仅 #3、#14 有直接测试，#1/#13 部分覆盖，其余无。
- 证据：`Glob src-tauri/tests/**`=0、`Glob fixtures/**/*cloud*`=0；`F:\workspace\PDF2Test\src-tauri\src\lib.rs:10505,10561`。
- 影响：§19.3 是第 7 章的验收门，当前等于无门；任何 §7 实现都无法被回归证明。
- 建议：先补 #4（歧义拒绝）、#5/#6/#8（枚举/结构拒绝）、#10（quote 不在 source）、#15/#16（timeout / image fallback），这 7 条不依赖未实现的 repair/salvage，可立即落地。

### A6-F10 [P2] §7.7 白名单部分实现，`TFNG` 别名与 1-based→0-based 缺失
- 结论：连字符归一、数字键归一、null→[]、trim 已实现；`TFNG` 与 1-based→0-based 未实现。
- 证据：`F:\workspace\PDF2Test\src-tauri\src\llm_gateway.rs:1000-1007,1009-1030,1076-1085,1201`。
- 影响：模型输出 `"TFNG"` 会触发 `cloud_outline_group_kind_invalid` 并使整份对照失败，而非按 §7.7 归一为 `true_false_not_given`；这是「本可本地修复却整份丢弃」的具体来源。
- 建议：在 `normalize_cloud_outline_kind` 增加 `tfng|t/f/ng|true_false_not_given` 等别名；1-based→0-based 需与 §7.4 request 声明联动，当前无声明字段，属设计缺口。

### A6-F11 [P3] §7.14 前提过时：UI 已收敛，但后端白名单与类型仍为 4 项
- 结论：计划描述「当前 UI 可选择 AnthropicCompatible 和 Custom」已不成立。
- 证据：活动页 `F:\workspace\PDF2Test\src\features\settings\SettingsPage.tsx:29-32,320-322`；孤儿页 `F:\workspace\PDF2Test\src\pages\Settings.tsx:288-292`；后端 `F:\workspace\PDF2Test\src-tauri\src\llm_commands.rs:78-83`；类型 `src/types/settings.ts:2`。
- 影响：文档与代码漂移；后端 `save_llm_profile_core` 仍接受 4 种 provider，用户/脚本可保存 gateway 无法路由的 profile（失败延后到 HTTP 请求期，`F:\workspace\PDF2Test\src-tauri\src\llm_gateway.rs:146`）。
- 建议：更新 §7.14 现状描述为「UI 已完成」；把 `SUPPORTED_LLM_PROVIDERS` 收敛为 2 项并同步 `LlmProvider` 类型，使保存期即拒绝。

### A6-F12 [P2] 云端原始错误串写入 canonical 审计，且前端存在无后端支撑的 `reconciling` 阶段
- 结论：`cloudComparison.failure`（原始错误文本）被写入 `authoring-ir.json` 的 `audit.issues[]`，并被类型暴露；前端 `libraryTypes` 声明了 `reconciling` 阶段但后端无 reconciliation。
- 证据：`F:\workspace\PDF2Test\src-tauri\src\auto_pipeline.rs:1143,2338,2391`；`src/types/settings.ts:130`；`src/features/library/libraryTypes.ts:39,130,146`。
- 影响：与 §7.9「不显示 serde/schema 原始错误」及 §24「普通 UI 不展示技术细节」的方向相悖；`reconciling` 阶段在 UI 上可能展示「正在合并本地与云端结果」，但该合并能力不存在（§7.10 未实现），构成误导。
- 建议：审计项只保留稳定错误码；`reconciling` 阶段在 §7.10 落地前从阶段机移除或改为诊断标记。

---

## 与历史审计的关系

- 与 `audit-2026-09-07/report.md` 的 M5 结论（「Cloud gateway emits comparison-only CloudReadingOutlineV1 ... No full candidate/skill/repair/salvage/three-way merge path was found」）**完全一致，无变化**。
- 与 `repair-2026-09-07/progress.md` 的 `Still Open` 一致：M5 属计划自身里程碑，非回归；该轮未触碰云端链。新增证据：本轮确认 §26.3-D 的 `LlmTransport: Send + Sync` 修订要求**也**未落地（历史审计未单列此点）。
- 与 `task_plan.md` M5=`pending` 一致。
- 与同批 `audit-2026-09-12/A2-ch2-3-acceptance-ia.md` 的云端条目结论一致（该报告从第 2/3 章需求侧得出 `CloudRecognitionCandidateV1` 全仓零命中）；本报告从第 7 章规格侧逐条补全 §7.2–§7.14 的字段级判定与 §19.3 用例映射。
- 新增（历史审计未覆盖）：§7.14 前提过时（活动 UI 已收敛）、§7.7 的 `TFNG`/1-based 缺口、`cloudComparison.failure` 进 canonical 审计、前端 `reconciling` 阶段无后端支撑。

---

## 证据层级与局限

| 层级 | 本报告使用范围 |
|---|---|
| product（真实 Tauri 运行时刻） | 未使用。未运行 Tauri，未发起真实 LLM 请求；无 product 级证据 |
| command（Rust 命令处理器静态/测试） | `llm_gateway.rs`、`llm_suggestions.rs`、`auto_pipeline.rs`、`llm_commands.rs`、`lib.rs` 测试为 command 级静态证据 |
| static（前端静态） | `SettingsPage.tsx`、`src/pages/Settings.tsx`、`types/settings.ts`、`libraryTypes.ts` 为 static 级 |
| doc-only（计划文本） | §7.2/§7.3/§7.4/§7.5/§7.6/§7.8/§7.9/§7.10/§7.11/§7.12/§7.13 的全部设计物均为 doc-only，无代码承载 |

局限：
1. 未编译、未运行测试；「零命中」基于对 `src-tauri/src` 与全仓的 Grep，若契约名由动态字符串拼接或序列化别名生成，理论上可能漏检——已用 `CloudReadingOutlineV1` 作为阳性对照确认云端输出契约唯一。
2. `sidecars/llm-gateway/gateway.mjs`（Node 诊断侧车）同样只有 outline 契约（`sidecars/llm-gateway/gateway.mjs:315,420-421`），与 Rust 一致，未额外展开。
3. 「前端不显示原始错误」的判定基于 `src/features/**` 无 `cloudComparison` 渲染点；未逐屏核对浏览器渲染结果。
4. 未评估 provider 真实多页 PDF 行为，「静默丢页」为设计风险推断（§7.12 所禁止情形），非实测结论。
