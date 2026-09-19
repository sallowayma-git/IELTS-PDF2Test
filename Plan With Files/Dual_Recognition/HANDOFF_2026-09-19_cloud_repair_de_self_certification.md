# 云端自主修复闭环：拆掉验收链的自证结构（2026-09-19）

上一轮（`HANDOFF_2026-09-18_cloud_repair_chain_acceptance.md`）结论是 12/12 通过。本轮**不扩架构**，
只做一件事：**把那条链里「自己证明自己」的部分拆掉**，然后看它还能不能站住。

结论：**拆完之后 PDF 仍然 13/13 通过**（新增两条真正能失败的场景）；同时这一轮**在链条自身**和
**产品代码**里各抓到几个真问题，逐条列在下面。

跑法：

```bash
# PDF（被断言的那一遍）
node scripts/e2e/tauri-cdp-cloud-repair-chain.mjs --pdf fixtures/parser/demanding-reading-passage-3.pdf --port 11477

# DOCX 变体
node scripts/e2e/tauri-cdp-cloud-repair-chain.mjs --pdf fixtures/parser/demanding-reading-passage-1.docx --port 11478

# 「测试有没有牙」——把每处修复退回旧行为，确认对应用例会红
python scripts/e2e/lib/repro-check-cloud-repair-tests.py
```

---

## 一、上一轮那条链为什么会「自证」

旧链条的三步是同一个值在打转：

1. 脚本从**本地稿**里剥掉页脚残留，算出「正确题面」；
2. 把算出来的结果同时写进**候选样本**、**修复剧本**、以及**断言时比的字符串**；
3. 受控服务照剧本抄一遍，断言「等于剧本里的值」——当然相等。

于是「云端能依据原文件修正识别错误」这句话**无法被证伪**：剧本里一次 `read_source` 都没有，
而网关 prompt 却写着 "so it matches the ORIGINAL FILE"。这个缺口上一轮的交接报告里没有点出来。

**本轮改法**：

| 环节 | 旧 | 新 |
| --- | --- | --- |
| 期望值来源 | 脚本从本地稿派生 | 人工逐页核对原文件写下的 golden fixture（`fixtures/golden/cloud-repair/demanding-reading-passage-3.annotation.json`） |
| 期望值与文件的绑定 | 无 | fixture 记原文件 `sha256`，哈希对不上直接 FAILED（本轮实测拦下了 DOCX 那一遍） |
| 剧本 | 只有 4 轮，没有 `read_source` | 第 2 轮真的 `read_source`；正确题面与**引文**都只能从它的真实返回里读出来 |
| 剧本是否含期望值 | 含 | **断言剧本里不得出现期望值**，出现即判失败（防以后图省事塞回去） |
| 断言 | 等于剧本里的字符串 | 题面等于 golden 标注；**每条引文必须在它自己声明的那一页原文里逐字找到** |

新增两条场景（都能失败）：

- `correction-and-evidence-come-from-the-original-file`
- `model-corrected-itself-from-real-feedback`

---

## 二、拆掉自证之后，链条自己先暴露了两个缺陷

这两条都是**链条/断言的缺陷**，不是产品缺陷。列出来是因为它们正说明「能失败的断言」有价值。

### 2.1 引文检查把「合法引文」判成「找不到」

模型有两条引文：

| 轮次 | 引文 | 声明页 | 实际所在页 |
| --- | --- | --- | --- |
| `apply_edits` | `40 The writer recommends that to be effective, social history must` | 4 | 4 ✓ |
| `record_ruling` | `YES if the statement agrees with the claims of the writer NO if the statement contradicts … NOT GIVEN …` | 3 | 3 |

旧断言把**每一条**引文都拿去和 golden 标注的那一页（第 4 页）比，于是第 3 页那条合法引文被判
「在原文里找不到」。反方向也一样：模型把第 3 页的引文谎报成第 4 页，旧断言照样能过。

**修法**：按**引文自己声明的 `pageIndex`** 逐条核对，并要求该页在作业目录里真实存在。两个方向都堵住。

### 2.2 轮次断言没跟上新插入的 `read_source`

实际往返是：

```
[0] read_draft  [1] read_source  [2] apply_edits(无 baseVersion)
[3] apply_edits(baseVersion=1)  [4] record_ruling  [5] finish
```

断言里写的还是插入 `read_source` 之前的序号（`rounds[1]` 就该是 `apply_edits`），于是四条子断言
同时报错。**修法**：整体后移一位，并补一条「第 2 轮必须是 `read_source`」。

---

## 三、变异检查抓到一条「没有牙」的测试（已修）

新增/修改的回归用例，光看它绿不算数——要确认**把修复退回旧行为时它真的会红**。
为此写了 `scripts/e2e/lib/repro-check-cloud-repair-tests.py`：逐条把实现退回旧行为，跑对应用例，
要求「变红，且理由是**断言失败**而不是编译不过」，跑完从快照还原并逐字节核对。

第一次跑，11 条里有 **1 条是 GREEN（测试没有牙）**：

> `read_source_pages_carry_the_text_layer_extracted_from_the_original_file`
> 把 `attach_page_texts` 的调用删掉（退回「PDF 只回页图」），测试**照样通过**。

**根因**：这条用例直接调助手函数 `attach_page_texts`，测到了助手，却**没测到接线**——
`read_source_evidence` 里那行调用删掉，谁也不会发现。之所以这样写，是因为
`read_source_evidence` 要先跑 PDF 视觉抽取（Python sidecar），单测驱动不起来。

**修法**：把 PDF 分支抽成纯函数 `pdf_read_source_response(evidence, page_from, page_to, page_texts)`
（`cloud_repair/mod.rs`），`read_source_evidence` 只负责取证据再委托给它；用例改为驱动这个分支函数，
并补上 `pagesWithText` 计数、`note` 随事实改口、页范围过滤三类断言。

修完再跑：**11/11 全部变红，源文件与快照逐字节一致。**

顺带修掉一个自己踩的坑：还原**必须连 mtime 一起放回**。第一版只还原了字节，
mtime 被顶到当前时刻，于是「后端源码比 exe 新」——真实链路验收的构建新鲜度闸门
（`build-freshness`）据此判定 exe 过期并拒绝开跑。内容逐字节相同却要白白重建一次。
现在快照存 `st_mtime_ns`，还原用 `os.utime` 放回，且一致性检查同时比对字节与 mtime。

---

## 四、产品缺陷：对已有稿子的条目点「重新识别」永远失败（仍未修，需产品决策）

与上一轮一致，本轮再次实测复现（`findings: defect-retry-cannot-rerun`）：

```
retry_processing 返回 ok
12 秒后：stage=failed  retry_count=1
last_error_code = "editable_draft_exists; pass allowOverwrite=true before regenerating draft"
```

`retry_processing` 只把 stage 改回 `queued`，工作线程随后调用的本地闭包不带 `allowOverwrite`，
而 `run_auto_pipeline_core` 在 `authoring-ir.json` 已存在时以 `editable_draft_exists` 拒绝。
**用户点「重新识别」只会白白消耗重试次数。**

**为什么仍然没修**：这不是补一个参数，而是产品语义——
「重新识别」是「**保留**我的编辑，只重跑识别」还是「**覆盖**我的编辑，全部重来」？
`allowOverwrite` 这个名字倾向后者，但界面上没有任何地方告诉用户这一点。
需要产品决策（是否提示「会覆盖现有编辑」、是否保留可撤销快照），不该由实现方替用户定。

---

## 五、DOCX 变体：这次把「为什么跑不了」写清楚了

DOCX 这一遍**仍然没通过**，但本轮把它从「含混的 not-executable」变成了**有据可查的失败**。

以前：报告只写「golden 哈希对不上」，读者容易以为是脚本配置问题。
现在：哈希不一致之外，**同时报出这份输入的初稿本身不可用**：

```json
"prepassDraftShape": {
  "taskGroups": 1, "responseGroups": 13,
  "placeholderPrompts": 13, "emptyPrompts": 0,
  "qualityState": "blocked",
  "blockingCodes": ["PROMPT_EMPTY","PROMPT_BOUNDARY_AMBIGUOUS","ANSWER_KEY_MISSING_SLOT","RUNTIME_COMPILER_FAILED"],
  "samplePrompt": "[prompt pending review]",
  "readPath": "get_workspace_item.ds"
}
```

> 而且这份输入的本地初稿本身不可用：13 个题面里 13 个是占位符、0 个是空的（blocking [...]）。
> 这是导入阶段的产品退化，先把初稿修好，才谈得上这份输入能不能验收。

**这是一条产品缺陷**：DOCX 导入路径产出的初稿没有任何题面内容（`PROMPT_EMPTY` 直接阻断）。
它发生在**导入阶段**，与云端修复闭环无关——所以 DOCX 仍**未验收**，如实记录。

### 5.1 顺带发现：两条读取路径对同一份稿子说法不同

同一份 DOCX、同一次运行：

| 读取层 | 13 个题面 |
| --- | --- |
| 产品读取路径 `get_workspace_item`（界面用的） | 13 个 `[prompt pending review]` **占位符** |
| 库里 `item.canonical`（`dump-authoring-db.py` 读的） | 13 个**空串** |

`[prompt pending review]` 由 `ielts_grammar/mod.rs:1060` 在题面拼不出来时注入，
`quality.rs:2862` 也把它当作空题面（`PROMPT_EMPTY`）。**两者都表示「题面没有内容」，结论不变**，
但报告必须写清楚读的是哪一层，否则「占位符 13」和「空串 13」会被当成两个事实。
所以形状记录里加了 `readPath` 字段。

---

## 六、复核一条怀疑：质量报告的两种口径混用会不会误拒合法修复

任务书里有一条怀疑：基线用 `refresh_quality_report`（全量），事务内用
`refresh_quality_report_for_targets`（只刷受影响目标）——口径不同，会不会让一次合法修复被
`CLOUD_EDIT_INTRODUCED_HARD_FAILURES` 误拒？

**结论：不会，这条怀疑不成立。** 两个函数是同一个实现——`refresh_quality_report` 就是
`refresh_quality_report_for_targets(..., &BTreeSet::new())`，两边都调 `evaluate_quality` 做**全量重算**，
差别只在「哪些目标上的人工 resolution 被继承」；而比较用的 `blocking_diagnostic_fingerprints`
只看 severity / code / 目标 / 锚点，**不读 resolution**，所以继承与否根本不改变被比较的集合。

新增用例把这条等价关系钉住：
`the_baseline_and_the_in_transaction_quality_pass_agree_on_blocking_diagnostics`
（`cloud_repair/tools.rs`）。以后若有人给 `_for_targets` 加上「只重算受影响目标」的优化
（那才会真的引入误拒），这里会立刻变红。

---

## 七、证据分层（按 AGENTS.md）

| 层级 | 本轮覆盖 |
| --- | --- |
| **产品行为（端到端）** | 导入 → 本地初稿可见 → 云端完整候选 → 修复循环（含真实 `read_source`）→ 画布刷新 → 编辑保存 → 重开 → 学生预览 → 导出 → 学生端加载，13 个场景经真实 Tauri 界面/命令链断言通过 |
| 服务 / 命令处理器（UI 之下） | 权威稿前后差异、模型工具往返、引文与页码，从**数据库原件与作业缓存**读取 |
| 测试自身的可信度 | 变异检查 11/11：每条修复退回旧行为，对应用例都**因断言失败**变红 |
| 学生端（另一仓真实代码） | 沿用上一轮的 `NasJsDirectProvider` 真实加载（19/19），本轮未改动该部分 |
| 仅 CLI / schema | 本轮**没有**把任何 CLI 或 schema 检查当作验收结论 |

**仍未端到端验证的**：Electron 学生端**界面**的渲染与作答一致性（沿用上一轮，属 M6，pending）。

### 钉住这次运行的几样东西

| 项 | 值 |
| --- | --- |
| PDF 运行目录 | `artifacts/e2e-cdp/run-cloud-repair-chain-2026-09-19T10-55-24-012Z/`（13/13 passed，退出码 0） |
| DOCX 运行目录 | `artifacts/e2e-cdp/run-cloud-repair-chain-2026-09-19T10-58-08-740Z/`（failed，退出码 1，导入阶段退化） |
| 夹具 | `demanding-reading-passage-3.pdf` sha256 `f13bd65cb5f5c76a178ff87fa212df30f79852bdb103249af9eb5804a4ccfe18` |
| golden 标注 | `fixtures/golden/cloud-repair/demanding-reading-passage-3.annotation.json`（绑定上面这个哈希） |
| `cargo test --lib` | **838 passed / 0 failed / 11 ignored** |
| 变异检查 | **11/11** 条在旧行为下变红，源文件与快照逐字节一致 |
| exe | sha256 前缀 `aab17ad8530b9ed5`，`buildFresh.ok = true` |

`artifacts/`、`.scratch/`、`*.log` 都在 `.gitignore` 里，是本地证据，不进版本库。

---

## 八、本轮改动清单

| 文件 | 改了什么 |
| --- | --- |
| `fixtures/golden/cloud-repair/demanding-reading-passage-3.annotation.json` | **新增**：人工核对原文件写下的期望值，绑定原文件 sha256 |
| `scripts/controlled-llm-service.mjs` | 剧本插入第 2 轮 `read_source`；正确题面与引文只能从它的真实返回派生 |
| `scripts/e2e/lib/cloud-repair-scenario.mjs` | 场景派生改为**核对** golden，不再自产期望值 |
| `scripts/e2e/tauri-cdp-cloud-repair-chain.mjs` | 新增两条场景；引文按声明页核对；轮次断言后移；稿子形状无条件记录（含 `readPath`） |
| `src-tauri/src/cloud_repair/mod.rs` | 抽出 `pdf_read_source_response`，让 PDF 分支可被真实驱动 |
| `src-tauri/src/cloud_repair/tests.rs` | `read_source` 用例改驱动分支函数，补 `pagesWithText` / `note` / 页范围断言 |
| `src-tauri/src/cloud_repair/tools.rs` | 新增两种质量口径等价的用例 |
| `scripts/e2e/lib/repro-check-cloud-repair-tests.py` | **新增**：变异检查工具（快照式还原，不依赖 git 状态） |

---

## 九、下一步建议

1. **DOCX 导入**：修 `PROMPT_EMPTY`（13/13 题面为空）——这是 DOCX 变体无法验收的**唯一前置**。
2. **「重新识别」语义**：做产品决策后单独开一个改动（是否提示覆盖、是否留快照）。
3. **学生端界面**：补 Electron 端的渲染/作答一致性验收（M6）。
4. 若后续再改云端修复实现，**跑一遍变异检查**——绿灯不等于有牙。
