# 云端自主修复闭环：真实产品链路验收（2026-09-18）

本轮做的是**收口与验收**，不扩架构。核心问题不是「有没有新代码」，而是三句话：

1. 云端**自己**解决了什么？（全程不点任何「采用」按钮）
2. 用户实际还需要做几次操作？每一次是什么？
3. 剩下的为什么**不能**自动处理？

跑法：`node scripts/e2e/tauri-cdp-cloud-repair-chain.mjs --keep`（真实 Tauri + WebView2 CDP）。

**本轮结论：12/12 场景通过，链条从头到尾跑通，包括导出与学生端加载。**

---

## 一、证据分层（按 AGENTS.md）

| 层级 | 本轮覆盖情况 |
| --- | --- |
| **产品行为（端到端）** | 导入 → 本地初稿可见 → 云端完整候选 → 修复循环 → 画布刷新 → 编辑保存 → 重开 → 学生预览 → **导出** → **学生端加载**，全部经真实 Tauri 界面/命令链跑通并断言 |
| 服务 / 命令处理器（UI 之下） | 权威稿前后差异、批次修复记录、模型工具往返记录，全部从**数据库原件与作业缓存**读取 |
| 学生端（另一仓的真实代码） | `NasJsDirectReadingAssetProvider`（学生端已编译产物）真实加载发布包，19/19 |
| 仅 CLI / schema | 本轮**没有**把任何 CLI 或 schema 检查当作验收结论 |

**仍然没有被端到端验证的**：Electron 学生端**界面**的渲染与作答一致性。本轮验证到的是学生端**服务端读取层**（provider + loader + asset-resolver）真实加载并解出内容。这一条属于 M6，如实标注 pending。

运行目录：`artifacts/e2e-cdp/run-cloud-repair-chain-2026-09-18T23-14-35-645Z/`
（`artifacts/` 与 `tmp/` 都在 `.gitignore` 里，是本地证据，不进版本库。）

钉住这次运行的五样东西：

- 提交 `bfeed9e1de66771a66496056aebc623d3f9925bf`，`worktreeClean = true`（工作区干净）
- exe sha256 `520a2be6420fd3bd36e645b0f1b1d5b94dd838ee932a0eb5d8637ee7df3daee8`
  （清单 `artifacts/build-manifests/520a2be6….json`，`backendInputs=74667bd93354`，`frontendInputs=e1cbd9884fd6`）
- 夹具 `demanding-reading-passage-3.pdf` sha256 `f13bd65cb5f5c76a178ff87fa212df30f79852bdb103249af9eb5804a4ccfe18`
- 发布包 `p1-8a614e1f`（批次 `7028c58488194564949c96bb1c58817f`）
- 四张界面截图 sha256 各不相同（面板不再遮住画布，见 §9）

---

## 二、云端自己解决了什么

**12 个场景全部通过**。关键五条：

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

这正是本轮最该坚持的产品判断：**首遍云端候选也只是输入**——它可以错，最终校核有权
保留当前稿、结束已经解决的争议，而不是把每条差异都丢给用户。

### 3. 它留下了一条真实未解疑问，而不是假装做完

`cloud-question:group-1:0`：第 27–31 题的题干要求从 A–H 词表选词，但当前稿没有选项库，
作答被当成自由输入。原文件里确实印着这份词表，但「要不要按匹配题重建词表」会改变这道题的
作答与判分方式——云端把它如实交出来，没有替用户定。它**不是** blocking，不拦导出。

### 4. 进度是**运行中**就能看见的，不是十分钟后才出现

轮询实测抓到三档（`recognition_batches_v1.repair_json` 与界面同步）：

```
running          applied=0  editVersion=1  rounds=0
running          applied=1  editVersion=2  rounds=3
needs_attention  applied=1  editVersion=2  rounds=5
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

修复结束时剩余任务 **16 条**（其中 **15 条 blocking**）：

| 动作 | 条数 | 是什么 | 为什么不能自动做 |
| --- | --- | --- | --- |
| `fix_blocking_issue` | 14 | `ANSWER_KEY_MISSING_SLOT`（q27–q40 缺答案） | 原文件**没有答案页**，编不出来 |
| `fix_blocking_issue` | 1 | `RUNTIME_COMPILER_FAILED` | 上面 14 条答案缺失的**连带结果**，补上答案即消失 |
| `review_difference` | 1 | A–H 词表要不要建成选项库 | 会改变作答与判分口径，属编辑决策 |

界面同步（`recognition-remaining-tasks.png`，面板原文）：

> 已自动修正 1 处、已了结 1 处差异，还有 **16** 处需要你确认
> 云端自动修复已完成，还有 16 处需要你处理（其中 15 处不处理不能导出）。

每条都带真实动作（「定位到题面 / 去题面修改」），不是只给一句话。

harness 用真实编辑器保存路径替用户补了 14 个答案（13 条命令：4 条 text + 9 条 option），
补完之后 `ANSWER_KEY_MISSING_SLOT`、`ANSWER_MISSING`、`RUNTIME_COMPILER_FAILED`
**全部消失**，`remainingBlocking = 0`，**发布成功**：

```
发布完成：p1-8a614e1f
status = published   qualityState = ready   remainingBlocking = 0
```

**所以用户实际要做的，就是「补 14 个答案」+「决定 1 个编辑口径」共 15 次操作。**
结构性问题（题面残留、候选读错的说明）云端已经自己处理掉了。

---

## 四、剩余任务为什么无法自动处理

### 答案（14 条）：原文件里没有答案页

不是「云端偷懒」，是**不能编造**。原文件没有答案，`answerKey` 初始全是
`{"kind":"unresolved"}`，任何自动填入都等于伪造判分依据。

### 那条编辑决策（1 条）：会改变作答与判分方式

把「自由输入的摘要填空」改成「从 A–H 里选字母」，是题型口径的改变，
属于编辑决策，不在云端权限内（`MODEL_ALLOWED_OPS` 有意不含 `resolveIssue` 等）。

### 那 1 条 `RUNTIME_COMPILER_FAILED`：是连带结果，不是独立问题

它由答案缺失派生。用户补完答案后它自动消失——这一点本轮已实测（见 §3）。

---

## 五、本轮发现并**修掉**的产品缺陷：题干写「A-H」时整卷发不出去

`infer_option_alphabet`（`ielts_grammar/instruction_signature.rs:286`）的区间表原来是
`[a-d, a-e, a-i, a-g]`——**缺 a-f 与 a-h**。

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

**判定它是 bug 而不是设计**的依据：`authoring_pipeline.rs::dynamic_declared_option_bank_labels`
（约 1670–1710 行）早就覆盖 `a-e … a-j`。两处回答的是同一个问题（「题干声明了哪一段字母」），
一侧认得、一侧认不得，就是不一致。

**修法**：往区间表里补 `a-f` / `a-h` / `a-j`，**追加在末尾**，让这次改动对既有输入是
**纯增量**——原来能认出来的四种仍最先匹配，任何既有输入的返回值都不变；方向仍然是
「认不出来就不认」。

**验证**：

| 证据 | 结果 |
| --- | --- |
| `cargo test --lib` | **825 passed / 0 failed**（基线 823 + 2 条新用例） |
| 真实链路 | **11/12 → 12/12**，`export-and-student-runtime: passed` |
| 发布结果 | `status=published` / `qualityState=ready` / `remainingBlocking=0` |
| 学生端真实代码加载 | 19/19 |

提交 `c9932aa`。回归用例：
`word_list_summary_completion_declaring_a_to_h_is_selection_type`、
`single_letter_ranges_cover_the_span_the_pipeline_already_supports`。

---

## 六、本轮发现但**没有修**的产品缺陷：对已有稿子的条目「重新识别」永远失败

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

**为什么没修**：这不是「补一个参数」那么简单，它牵扯产品语义——
「重新识别」到底是「**保留**我的编辑，只重跑识别」还是「**覆盖**我的编辑，全部重来」？
两种语义对用户数据的破坏程度完全不同，`allowOverwrite` 这个名字说明现有代码倾向后者，
但界面上没有任何地方告诉用户这一点。**这需要产品决策，不该由我替用户定。**
建议单独开一个改动，并同时决定：重跑前是否提示「会覆盖现有编辑」、是否保留可撤销快照。

这条缺陷也是本轮验收设计的原因：被断言的那一遍必须是**一次全新的导入**，
不能靠重试同一个条目。

---

## 七、DOCX 变体：前提不成立，如实记 not-executable

`fixtures/parser/demanding-reading-passage-1.docx` 的**本地初稿本身是退化的**：

```
taskGroups = 1, responseGroups = 13, 题面占位符 = 13/13
samplePrompt = "[prompt pending review]"
blocking = ["PROMPT_EMPTY","PROMPT_BOUNDARY_AMBIGUOUS","ANSWER_KEY_MISSING_SLOT","RUNTIME_COMPILER_FAILED"]
```

13 个题面**全是**占位符，没有任何可枚举的真实内容差异可供云端修复。
硬造一份候选只会得到假结论，所以记 `not-executable`（退出码 5），并把真实稿形状写进报告。

**这本身是一条发现**：DOCX 这条导入路径没有产出可用的初稿（`PROMPT_EMPTY` 直接阻断）。
按任务书「PDF 通过后再跑 DOCX」——PDF 现在**已通过**，但 DOCX 的阻断在**导入阶段**，
与云端修复闭环无关，因此 DOCX **仍未验收**，如实记录。

---

## 八、回归保障的状态（不当作验收）

- `cargo test --lib` → **825 passed / 0 failed**（本轮跑成，含 2 条新增用例）。
  依赖缓存此前被清空，本轮通过 `CARGO_HTTP_PROXY` 恢复下载后跑通。
- smoke / 保存回归 / 单测继续作为回归保障，**不再把它们称作完整产品验收**。
- 链路的 12 个场景**不是**单元测试：它们走真实 Tauri、真实网关、真实数据库、
  真实发布目录、真实学生端 provider。

---

## 九、怎么复跑

```bash
# 真实 Tauri 链路验收（PDF）—— 12 个场景
npm run e2e:cloud-repair-chain -- --keep

# DOCX 变体
npm run e2e:cloud-repair-chain -- --pdf fixtures/parser/demanding-reading-passage-1.docx --port 11466 --keep

# 发布包的学生端检查（两层，互补）
npm run e2e:nas-contract -- --package <run>/nas-library          # 镜像规则：22 项
npm run e2e:student-real-provider -- --package <run>/nas-library # 学生端真实代码：19 项

# 退出码：0 全通过 / 1 有场景失败 / 2 无场景 / 3 环境不满足 / 5 全部无法执行
```

两个学生端检查**不是重复**：

- `nas-student-contract.mjs` 是**镜像**——按学生端规则重新实现一遍校验，证明「包符合规则」。
- `student-real-provider-load.mjs` 是**真代码**——直接 require 学生端仓库已编译的
  `NasJsDirectReadingAssetProvider`，把发布包当 NAS 挂上去跑真实入口，证明「学生端能读」。
  它自带一条**反向对照**（低运行时版本必须被过滤掉），用来证明跑的是真实代码而非被 stub 的成功路径。

若环境需要重建 exe（改了 `src-tauri/` 或 `package.json` 之后），注意：

- `npx` 在无网络时会卡在 registry 检查上 → `npm_config_offline=true npm_config_prefer_offline=true`
- cargo 不认 `HTTP_PROXY`，只认 `CARGO_HTTP_PROXY=http://127.0.0.1:49870`
- `taskkill //F` 会被 MSYS 路径转换搞坏 → `MSYS_NO_PATHCONV=1 taskkill /F /IM cargo.exe /T`

---

## 十、留存的证据（都在运行目录里）

| 文件 | 内容 |
| --- | --- |
| `source-file/demanding-reading-passage-3.pdf` | 原文件副本 |
| `db-before.json` / `db-after.json` / `db-after-export.json` | 权威稿、批次、作业的**前后**原件 |
| `scenario/authoring-candidate.json` / `scenario/repair-plan.json` | 派生出的候选样本与修复剧本 |
| `report.json` | 全量报告：12 个场景、模型工具记录、进度采样、findings |
| `appdata/data/jobs/<job>/cache/llm/*-output-*.json` | 五轮工具调用**原件** |
| `appdata/data/jobs/<job>/llm-calls.jsonl` | 网关调用记录（5/5 ok） |
| `student-real-provider-load.json` | 学生端真实代码加载结果（19/19） |
| `nas-student-contract.json` | 学生端契约镜像结果（22/22） |
| `canvas-after-cloud-repair.png` | 画布：修复后的题面 |
| `recognition-remaining-tasks.png` | 识别面板：「已自动修正 1 处、已了结 1 处差异，还有 16 处需要你确认」 |
| `student-preview.png` | 学生预览 |
| `after-export-attempt.png` | 发布成功的界面 |
| `db-retry-probe.json` | 缺陷「重新识别」的复现证据 |
| `nas-library/` | **真实发布出来的 NAS 包**（manifest / release 脚本 / 快照 / 资源清单） |

---

## 十一、本轮提交

| 提交 | 内容 |
| --- | --- |
| `d23a5f4` | 真实 Tauri 链路验收脚本 + 受控服务扩展 + 场景派生 + 数据库转储 |
| `302dca9` | 画布截图必须真的拍到画布；面板重开要等渲染完成 |
| `2c917a5` | 交接报告（当时为 11/12，导出被产品缺陷拦下） |
| `c9932aa` | **产品修复**：选项字母区间补齐 a-f/a-h/a-j；学生端真实代码加载验收 |
| `bfeed9e` | 归档本轮实施计划；会话草稿移出版本库视野 |
