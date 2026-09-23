# 执行任务书：校核链路上下文管理（分支 `feat/repair-context-packets`）

> 基线：`wave3-listening` @ `34dfadd`。只在 `feat/repair-context-packets` 上开发，频繁提交，**不要 push**。
> 先读仓库根目录 `AGENTS.md`、`docs/handoff/2026-09-22-execution-prompt.md` 的 §1.2、§1.4、§3。
> 进度以同目录 `STATE.md` 为准；每完成一个可验证的小步就更新它并提交。

## 1. 目标

校核 Agent（`cloud_repair` 修复循环）不再每轮拿「整份 PDF + 整卷上下文 + 全部历史 observation」。改成三层：

1. **本地预切（L0）**：按差异把要校核的部分切成「校核包」，每包只带该范围的稿件切片、云端候选切片与原文范围（文本层逐行 + 区域页图）。
2. **不够就说（L1 入口）**：模型认为上下文不足时，必须返回结构化的 `report_insufficient_context`，不得硬猜。
3. **自己去取（L1）**：模型可用受限只读抓取工具主动拿本地内容（页、搜索、页图区域、文章段落、候选切片、稿件切片）；仍不够才逐级升级，最终如实交给用户。

成功标准：同一份卷子，单次校核总输入量显著下降、裁定质量不降，每一次「上下文不足」都有记录、可解释。

## 2. 不可动摇的边界（不得放宽）

- 权威稿只经 `cloud_repair/tools.rs::apply_cloud_edits` 事务写入（`baseVersion` CAS、`human_protected_targets`、证据校验）。`cloud_edit_rejects_protected_human_target` 必须保持绿。
- 证据引文校验**不缩小**：给模型看的范围缩小，但校验仍对照**完整原文**文本层。
- 原文没有的答案绝不编造；「上下文不足」绝不变成「猜一个」。
- 三态不坍缩：`not_executed` / `insufficient_context` / `passed` 分开。上下文不足不得算作已核对。
- **不改**：云端候选生成（`generate_authoring_candidate`）、云端优先级、调度器阶段顺序、`quality.rs`、`tools.rs` 的校验与写入逻辑。
- 网关校验器不为让模型通过而放宽；修的是 prompt。

## 3. 现状（动手前逐条核对行号）

- 每轮修复请求：`auto_pipeline.rs::repair_authoring_step_through_gateway`（~2866）→ `llm_suggestions.rs::make_repair_authoring_step_input`（~573）带 `pdfPath` → `llm_gateway.rs::run_openai_compatible_repair_step_llm`（~1650）经 `data_url_for_pdf` **每轮附整份 PDF base64**；DOCX 附全文。
- `cloud_repair/mod.rs::build_repair_context`（~843）每轮给 `documentIndex` + 全部 `differences` + 全部 `qualityIssues`；差异**没有页码锚点**。
- `MAX_REPAIR_OBSERVATIONS = 12` > `DEFAULT_MAX_REPAIR_ROUNDS = 6`，上限形同虚设。
- `read_source` 不传 `pageIndex` 时返回**全部页**（`cloud_repair/mod.rs` ~1103-1121）；页号 0/1-based 坑见 `source_page_texts` 注释。
- 可用锚点：canonical 节点 `SourceAnchorV2`（`schema/common.rs:67`，`pageIndex`/`bbox`/`nodeIds`）；页图 `cache/vision/pdf-images.json`（1-based）；逐行文本 `document-ir.json`（0-based）；依赖已有 `pdfium-render`、`png`。
- 受控假模型：`scripts/controlled-llm-service.mjs`（`repair_authoring_step` 剧本从 `read_draft` 开始）。
- 现有工具表：`schema/cloud_repair_v1.rs::CLOUD_REPAIR_TOOLS`（read_draft / read_source / apply_edits / record_ruling / finish）。

## 4. 设计

### 4.1 校核包（新模块 `cloud_repair/packets.rs`，纯函数，不调模型不写库）

输入：当前 canonical、云端候选、`candidate_differences`、阻塞质量问题、受保护目标、原文页映射。输出 `Vec<RepairPacketV1>`。

切分规则（依次）：
1. **归属**：差异/阻塞问题归到所属 `taskId`（slot、response_group 按所属题组；part 类单独成包；无目标的文档级问题进「文档包」，只带索引）。
2. **合并**：共用选项库、同一 stimulus 跨组、或本地与云端题号区间交叉但切法不同（本地 1-5 + 6-7、云端 1-7）的题组并为一包。
3. **拆分**：估算超预算时按 responseGroup 拆，子包都附共用选项库与说明文字。
4. **排序**：含阻塞问题 → 答案差异 → 文本差异。

原文范围 `scope.pages`（统一 1-based）：
- 包内题组节点（instructions / stimulus / prompt / options）的 `sourceAnchors` 页；
- ∪ 云端候选在这些目标上的引文页与 `unresolvedRegions` 页；
- 有答案差异时 ∪ **答案页**（优先答案页识别产物，否则文本层搜答案区题号模式；定位不到写 `answerPagesUnknown`，不猜）；
- 锚点贴页上/下边缘（跨页）时左右各扩一页。

包内容：`packetId`（由 batchId + taskIds + 差异键派生，稳定）、`paperMap`（整卷极简索引：各组题号区间/题型、每页有哪些题号/段落/答案区，几百字符内）、`differences`（保留 `contextDigest`）、`draftSlice`（含 `editVersion`）、`candidateSlice`、`sourceEvidence`（范围内逐行文本，行 id 如 `p4:l12`；锚点 bbox 外扩边距后的区域图，锚点缺失退整页图；DOCX 按段落 id 给窗口）、`scopeManifest`（含了什么、省略了什么、用哪个工具取）、范围内受保护目标与阻塞问题。

### 4.2 回退协议

新回复形态（写进 `schema/cloud_repair_v1.rs` 与工具表）：

```json
{"callId":"c3","tool":"report_insufficient_context",
 "arguments":{"packetId":"...","reason":"answer page not in scope",
  "needs":[{"kind":"pages","from":7,"to":7},
           {"kind":"search","quote":"Questions 14-20"},
           {"kind":"page_region","pageIndex":3,"bbox":[0,0,1,1]},
           {"kind":"passage","paragraphLabels":["C","D"]},
           {"kind":"candidate","taskIds":["..."]},
           {"kind":"draft","questionNumbers":[14,15]}]}}
```

后端在预算内自动满足，下一轮把内容并入本包证据。

抓取工具（新模块 `cloud_repair/grab.rs`；全部只读、无路径、仅本 job）：

| 工具 | 约束 |
|---|---|
| `read_source` | **必须**有页范围或 `quote`；单次 ≤ 3 页；不再允许一次拿全卷 |
| `search_source` | `query` → 匹配行 id + 页 + 上下文若干行 |
| `read_page_region` | `pageIndex` + 可选 `bbox` → 区域页图 |
| `read_passage` | 按段落标号或题号取文章段落 |
| `read_candidate` | 按 `taskIds` / `questionNumbers` 取候选切片 |
| `read_draft` | 行为保留，默认范围限定本包 |

升级阶梯（每包独立计数，级别写进诊断）：
- **L0** 预切包；
- **L1** 模型 `report_insufficient_context` 或抓取工具；每包 ≤ 3 次抓取、累计 ≤ 6 页、另有字节上限；
- **L2** 后端扩到该 Part/Section 整页图；
- **L3**（最后手段，常量控制，默认开）本包附一次整份 PDF，每次运行最多 1 次，记 `legacy_full_source`；
- **L4** 仍不足：模型须对本包剩余差异 `record_ruling cannot_resolve`，理由码 `CONTEXT_INSUFFICIENT`；模型没做则后端代记。进入用户清单，文案「云端没能拿到足够的原文来判断第 N 题，请对照原文确认」，**不得**写成已核对。

### 4.3 编排（改造 `run_repair_loop`，对外签名尽量不变）

- **逐包推进**：每包轮数预算 5；全局受 `DEFAULT_REPAIR_TIMEOUT_MS` 与总轮数上限约束；cloud permit 不变，包顺序执行。
- 每包请求 = **固定前缀**（规则 + 工具表 + 本包证据，稳定顺序，便于将来 prompt cache）+ **包内** observation；换包清空，不跨包累积。
- 包内 `apply_edits` 成功 → 重算差异，只重切受影响的包，刷新 `editVersion`。
- 新增 `finish_packet` 结束一个包；全部包结束后后端汇总成原最终摘要。`remaining_tasks` 仍按当前 canonical 重算，判据不变。
- 预算估算：字符/4 估 token，图片按固定值；单包上限 **24k**；超出依次：整页图→区域图 → 缩小裁剪 → 拆包。
- 常量 `REPAIR_CONTEXT_MODE = packets | legacy`，默认 `packets`；`legacy` 仅作 L3 实现与回归对照，不对用户暴露。

### 4.4 Prompt（`llm_gateway.rs::repair_step_prompt`）

- 开头说明：你看到的是一个**校核包**，不是整卷；`scopeManifest` 列出省略内容与取回方法。
- 证据不在范围内时先 `report_insufficient_context` 或抓取，**禁止**凭印象下结论；引文必须从返回的行文本逐字复制并带行 id 与页码。
- 保留现有全部规则（baseVersion、稳定 ID、不编答案、保护目标、裁定语义），不放宽。
- 新工具的信封形状全部写进 prompt，并**对照校验器逐项核对**（本项目吃过「prompt 没写信封键」的亏，见 `findings.md` F-REAL-MODEL 节）。

### 4.5 可观测性

- `llm-calls.jsonl` 每条加：`packetId`、`escalationLevel`、`pagesIncluded`、`imageCount`、`estimatedInputTokens`、`requestBytes`。
- `repair_json` 诊断区加逐包结果（轮数、升级级别、裁定数、编辑数、上下文不足条数）。前端展示不变；如需「上下文不足」文案，只改 `src/features/editor/userTasks.ts` 一类映射，不加 UI。

## 5. 先红后绿的测试

1. 切分：`complex-reading` canonical + 构造候选 → 包数与每包题组符合规则 2；`scope.pages` 与锚点一致；包内无范围外页。
2. 交叉切分：本地 1-5 / 6-7、云端 1-7 → 三组同一包。
3. 答案页：答案差异的包必含答案页；定位不到出 `answerPagesUnknown`。
4. 页号：document-ir 0-based 与页图 1-based 下，同一行文本在包里标对页。
5. 请求体：`packets` 模式下修复请求**无** `data:application/pdf` 整份附件，只有范围内页文本与区域图；`legacy`/L3 才有。
6. 上下文不足：假模型先 `report_insufficient_context`（缺答案页）→ 下一轮请求含该页；预算用尽 → 差异成为带 `CONTEXT_INSUFFICIENT` 的用户任务，摘要状态非 `completed`。
7. 抓取边界：无页范围的 `read_source` 被拒（原因具体）；超页数上限被拒；越界页被拒；`search_source` 返回行 id。
8. 编辑后重切：`apply_edits` 成功后下一轮 `editVersion` 与目标 ID 为新值。
9. 回归：`cloud_repair/tests.rs`、`tools.rs` 既有测试全绿（保护目标、旧版本、证据页码）。

## 6. 受控模型与防自证

- `scripts/controlled-llm-service.mjs` 加剧本：首包**故意**不含答案页 → `report_insufficient_context` → 拿到答案页后 `apply_edits` → `finish_packet`。
- 假模型**只能依据请求里真实出现的内容**行动；答案页不在请求里时它不能知道答案（证明回退真的在传内容）。
- 同一 fixture 跑 `legacy` 与 `packets`，对比 `estimatedInputTokens`、`requestBytes` 总和。

## 7. 验收（证据等级分清）

- **命令处理器层**：第 5 节测试 + `cd src-tauri && cargo test --lib`、`npx vitest run`、`npx tsc --noEmit`，数字与基线（P0 记录）对比。
- **命令处理器层 + 真实 HTTP**：用真实网关代码（`run_llm_gateway`）打受控服务（`scripts/controlled-llm-service.mjs` 起 HTTP），驱动 `run_repair_loop` 走完 L0→L1→finish。任何平台都能跑，必须做。
- **产品端到端（受控模型）**：仅 Windows（WebView2 CDP）。`npm run build:app` 后 `node scripts/e2e/tauri-cdp-cloud-repair-chain.mjs` 必须保持 **13/13**，并新增一步断言至少一个包走了 L1 且最终稿正确。非 Windows 环境如实记「未执行：平台不支持 CDP 通道」，不得报通过。
- **真实模型**：有额度才跑（`complex-reading.pdf` + 一份多题组长文），记录逐包级别、token、耗时、收敛、是否主动调用 `report_insufficient_context`。无额度如实报「未执行」。

## 8. 可改文件

新增 `cloud_repair/packets.rs`、`cloud_repair/grab.rs`；修改 `cloud_repair/mod.rs`、`cloud_repair/tests.rs`、`schema/cloud_repair_v1.rs`、`llm_suggestions.rs`（修复输入构造）、`llm_gateway.rs`（仅 `repair_step_prompt` 与修复请求执行函数、调用记录字段）、`auto_pipeline.rs`（仅 `repair_authoring_step_through_gateway`）、`scripts/controlled-llm-service.mjs`、`scripts/e2e/tauri-cdp-cloud-repair-chain.mjs`（仅新增步骤）、`src/features/editor/userTasks.ts`（仅文案）、`contracts/` 下相关 schema（如有）、本目录文档。其余不改；需要改时在 `STATE.md` 的「越界需求」里写清精确改动。
