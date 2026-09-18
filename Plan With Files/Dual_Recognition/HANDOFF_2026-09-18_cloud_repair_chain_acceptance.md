# 云端自主修复闭环：真实产品链路验收（2026-09-18）

本轮做的是**收口与验收**，不扩架构。核心问题不是「有没有新代码」，而是三句话：

1. 云端**自己**解决了什么？（全程不点任何「采用」按钮）
2. 用户实际还需要做几次操作？每一次是什么？
3. 剩下的为什么**不能**自动处理？

跑法：`node scripts/e2e/tauri-cdp-cloud-repair-chain.mjs --keep`（真实 Tauri + WebView2 CDP）。

---

## 一、证据分层（按 AGENTS.md）

| 层级 | 本轮覆盖情况 |
| --- | --- |
| **产品行为（端到端）** | 导入 → 本地初稿可见 → 云端完整候选 → 修复循环 → 画布刷新 → 编辑保存 → 重开 → 学生预览，**全部经真实 Tauri 界面/命令链跑通并断言** |
| 服务 / 命令处理器（UI 之下） | 权威稿前后差异、批次修复记录、模型工具往返记录，全部从**数据库原件与作业缓存**读取 |
| 仅 CLI / schema | 本轮**没有**把任何 CLI 或 schema 检查当作验收结论 |

**没有被端到端验证的**：`导出 → 学生端加载` 这一步本次**没做成**，原因见第五节（产品缺陷，不是脚本问题）。

运行目录：`artifacts/e2e-cdp/run-cloud-repair-chain-2026-09-18T22-44-20-455Z/`
（`artifacts/` 与 `tmp/` 都在 `.gitignore` 里，是本地证据，不进版本库。）

钉住这次运行的三样东西：

- exe sha256 `59534ac7dd2698b9c25ed1250fdacf9afa4c0b956e81db6056ca4a522c28d6b1`
  （清单 `artifacts/build-manifests/59534ac7….json`，`backendInputs=439b640a25f2`）
- 夹具 `demanding-reading-passage-3.pdf` sha256 `f13bd65cb5f5c76a178ff87fa212df30f79852bdb103249af9eb5804a4ccfe18`
- 验收脚本代码：HEAD `302dca9`（报告里的 `worktreeClean=false` 来自仓库里**先前就存在**的
  未跟踪草稿 `_*.txt` / `.planning/`，与本次验收相关的 `src-tauri/`、`scripts/` 均已提交。）

---

## 二、云端自己解决了什么

**12 个场景里 11 个通过**。关键三条：

### 1. 它改对了内容，且没有让用户点任何按钮

| | 值 |
| --- | --- |
| 权威稿题面（改前） | `The writer recommends that to be effective, social history must 14 BLANK PAGE` |
| 权威稿题面（改后） | `The writer recommends that to be effective, social history must` |
| 权威稿 editVersion | `1 → 2` |
| `appliedCount` | `1` |
| 权威稿差异（数据库原件比对） | 恰好 1 条：`response_group.prompt @ group-3-response-40` |

`14 BLANK PAGE` 是原文件的页脚被转录进题面的残留，不是题面内容。云端按原文件把它去掉了，
用户从头到尾没有点过「采用」。

### 2. 它了结了一条争议，没把它变成用户任务

候选把 `group-2` 的题干说明读错了（只抄了首句）。云端记了一条裁定
`task_group:group-2:instructions = current_is_correct`，于是这条差异**没有**出现在用户的
剩余任务里（`adjudicatedCount = 1`）。

### 3. 它留下了一条真实未解疑问，而不是假装做完

`cloud-question:group-1:0`：第 27–31 题的题干要求从 A–H 词表选词，但当前稿没有选项库，
作答被当成自由输入。原文件里确实印着这份词表，但「要不要按匹配题重建词表」会改变这道题的
作答与判分方式——云端把它如实交出来，没有替用户定。

### 4. 进度是**运行中**就能看见的，不是十分钟后才出现

轮询实测抓到三档（`recognition_batches_v1.repair_json` 与界面同步）：

```
running         applied=0  editVersion=1  rounds=0
running         applied=1  editVersion=2  rounds=3
needs_attention applied=1  editVersion=2  rounds=5
```

### 5. 模型确实是**照着真实反馈**改的

作业缓存 `cache/llm/repair_authoring_step-output-*.json` 里的五轮工具往返：

```
read_draft → apply_edits(不带 baseVersion，被拒) → apply_edits(baseVersion=1) → record_ruling → finish
```

第 3 轮的 `baseVersion=1` 正是第 1 轮 `read_draft` 真实读回来的 `editVersion`，
不是脚本里写死的常数；`llm-calls.jsonl` 五次调用全部 `ok=true`。

---

## 三、用户实际还需要做几次操作

把「用户能做的都做了」之后，剩余任务 **17 条**（其中 **16 条 blocking**，不处理不能导出）：

| 动作 | 条数 | 是什么 |
| --- | --- | --- |
| `fix_blocking_issue` | 16 | 14 条 `ANSWER_KEY_MISSING_SLOT`（q27–q40 缺答案）+ 1 条 `WORD_LIMIT_UNPARSED` + 1 条 `RUNTIME_COMPILER_FAILED` |
| `review_difference` | 1 | 上面那条 A–H 词表的编辑决策 |

**这 14 个答案，云端不能编。** 原文件没有答案页，所以它只能由用户提供——这是
`remaining-work-has-a-real-reason` 场景通过的原因。

harness 用真实编辑器保存路径替用户补了 14 个答案（13 条命令：4 条 text + 9 条 option），
补完之后 `ANSWER_KEY_MISSING_SLOT`、`ANSWER_MISSING`、`RUNTIME_COMPILER_FAILED`
**全部消失**，只剩 **1 条** blocking：

```
WORD_LIMIT_UNPARSED @ group-1   completion 题组未解析出 IELTS word limit。
```

也就是说：**用户把能做的都做了之后，这份卷子仍然发不出去**。原因见下节。

---

## 四、剩余任务为什么无法自动处理

### 答案（14 条）：原文件里没有答案页

不是「云端偷懒」，是**不能编造**。原文件没有答案，`answerKey` 初始全是
`{"kind":"unresolved"}`，任何自动填入都等于伪造判分依据。

### 那条编辑决策（1 条）：会改变作答与判分方式

把「自由输入的摘要填空」改成「从 A–H 里选字母」，是题型口径的改变，
属于编辑决策，不在云端权限内（`MODEL_ALLOWED_OPS` 有意不含 `resolveIssue` 等）。

### 最后那条 `WORD_LIMIT_UNPARSED`：**产品缺陷，用户也没有手段处理**

见下节缺陷 2。它不是「用户还没做」，而是「产品当前做不到」。

---

## 五、本轮发现的 2 个产品缺陷

两个都在报告里留了复现证据；**本轮没有改产品代码**（理由见 5.3）。

### 缺陷 1：对已有稿子的条目，「重新识别」永远不会重新识别

`retry_processing` → `retry_job`（`processing/scheduler.rs:253`）只把 stage 从
`failed` 改回 `queued`；工作线程随后调用的本地闭包（`scheduler.rs:450-461`）用
`AutoPipelineInput { ..Default::default() }`，**不带** `allow_overwrite`，
而 `run_auto_pipeline_core`（`auto_pipeline.rs:2101`）在 `authoring-ir.json` 已存在时
以 `editable_draft_exists` 拒绝。

实测（`db-retry-probe.json`）：

```
retry_processing 返回 ok
12 秒后：stage=failed  retry_count=1
last_error_code = "editable_draft_exists; pass allowOverwrite=true before regenerating draft"
```

用户点「重新识别」只会**白白消耗重试次数**，永远拿不到新的识别结果。
这直接改变了本轮验收的设计：被断言的那一遍必须是**一次全新的导入**，不能靠重试同一个条目。

### 缺陷 2：题干写「A-H」时整卷发不出去

`infer_option_alphabet`（`ielts_grammar/instruction_signature.rs:286`）的区间表是
`[a-d, a-e, a-i, a-g]`——**没有 a-h**。

这份稿子的题干原文是：

> Questions 27 - 31 Complete the summary using the list of words and phrases, **A-H**, below.
> Write the correct letter, **A-H**, in boxes 27-31 on your answer sheet.

`instructionSignature.normalizedText` 里确实含 `a-h`，但推导表认不出来，于是：

```
optionAlphabet = None  →  该题组被判成「非选择型」
                       →  题干里又没有 word limit
                       →  WORD_LIMIT_UNPARSED（blocking）
```

`quality.rs:1566` 的判定只认 `optionAlphabet` 或 `wordLimit` 两者之一，而这两个字段
**用户界面上都没有任何入口可以设置**。所以这份卷子无论用户怎么操作都发布不出去。

报告里的自动判定（`findings[defect-option-alphabet-missing-range]`）：

```json
{"groupId":"group-1","matchedRange":"A-H","supportedRanges":["a-d","a-e","a-i","a-g"],
 "signatureKeys":["answerAssignment","confidence","evidenceAnchors","expectedQuestionNumbers",
                  "expectedSlotCount","normalizedText","taskType"]}
```

`signatureKeys` 里没有 `optionAlphabet`，就是这条链路的断点。
画布截图 `canvas-after-cloud-repair.png` 里也能直接看到：A–H 词表是以**正文**形式
留在 stimulus 里的，不是选项库。

### 5.3 为什么没顺手改掉

改动本身很小（往区间表里补 `a-h`），也确实在本地改出来验证过思路。但**本环境无法重建**：
cargo 的 `registry/cache` 在本次会话中被清空（`~/.cargo/registry/src` 里 816 个 crate
仍在，但 `cache/` 为空），且没有网络，`cargo build --offline` 直接报
`failed to download adler2 v2.0.1`。既然重建不了，就无法给出「改完确实能发布」的证据——
那就**不能留下未经验证的改动**，已如实回退（`src-tauri/` 当前是干净的）。

建议单独开一个改动来做，并补上：区间表扩展 + `WORD_LIMIT_UNPARSED` 不再误报的回归用例。

---

## 六、DOCX 变体：前提不成立，如实记 not-executable

`fixtures/parser/demanding-reading-passage-1.docx` 的**本地初稿本身是退化的**：

```
taskGroups = 1, responseGroups = 13, 题面占位符 = 13/13
samplePrompt = "[prompt pending review]"
blocking = ["PROMPT_EMPTY","PROMPT_BOUNDARY_AMBIGUOUS","ANSWER_KEY_MISSING_SLOT","RUNTIME_COMPILER_FAILED"]
```

13 个题面**全是**占位符，没有任何可枚举的真实内容差异可供云端修复。
硬造一份候选只会得到假结论，所以记 `not-executable`（退出码 5），并把真实稿形状写进报告。

**这本身是一条发现**：DOCX 这条导入路径没有产出可用的初稿（`PROMPT_EMPTY` 直接阻断）。
按任务书「PDF 通过后再跑 DOCX」，PDF 的导出这一环尚未通过，DOCX 因此**不算验收过**。

---

## 七、回归保障的状态（不当作验收）

- **Rust 单元测试本轮没跑成**：`cargo test` 需要下载 dev 依赖，本环境无网络、缓存又被清空，
  报 `failed to download adler2 v2.0.1`。这不是代码问题，是环境问题，但结论必须如实写：
  **本轮没有单元测试层面的回归证据**。
- 上一轮的 `cargo test --lib 823 passed / 0 failed`（`5e2884c`）仍然成立，但它对应的是
  改动前的代码；本轮 `src-tauri/` 无改动（已回退），因此那份结论仍然适用。
- smoke / 保存回归 / 单测继续作为回归保障，**不再把它们称作完整产品验收**。

---

## 八、怎么复跑

```bash
# 真实 Tauri 链路验收（PDF）
node scripts/e2e/tauri-cdp-cloud-repair-chain.mjs --keep

# DOCX 变体
node scripts/e2e/tauri-cdp-cloud-repair-chain.mjs \
  --pdf fixtures/parser/demanding-reading-passage-1.docx --port 11466 --keep

# 退出码：0 全通过 / 1 有场景失败 / 2 无场景 / 3 环境不满足 / 5 全部无法执行
```

若环境需要重建 exe（改了 `src-tauri/` 之后），注意 `npx` 会在无网络时卡在 registry 检查上，
用 `npm_config_offline=true npm_config_prefer_offline=true node scripts/e2e/build-app.mjs`。

---

## 九、留存的证据（都在运行目录里）

| 文件 | 内容 |
| --- | --- |
| `source-file/demanding-reading-passage-3.pdf` | 原文件副本 |
| `db-before.json` / `db-after.json` / `db-after-export.json` | 权威稿、批次、作业的**前后**原件 |
| `scenario/authoring-candidate.json` / `scenario/repair-plan.json` | 派生出的候选样本与修复剧本 |
| `report.json` | 全量报告：12 个场景、模型工具记录、进度采样、findings |
| `appdata/data/jobs/<job>/cache/llm/*-output-*.json` | 五轮工具调用**原件** |
| `appdata/data/jobs/<job>/llm-calls.jsonl` | 网关调用记录 |
| `canvas-after-cloud-repair.png` | 画布：修复后的题面（A–H 词表以正文形式可见） |
| `recognition-remaining-tasks.png` | 识别面板：「已自动修正 1 处、已了结 1 处差异，还有 17 处需要你确认」 |
| `student-preview.png` | 学生预览 |
| `after-export-attempt.png` | 发布被门禁拦下时的界面 |
| `db-retry-probe.json` | 缺陷 1 的复现证据 |
