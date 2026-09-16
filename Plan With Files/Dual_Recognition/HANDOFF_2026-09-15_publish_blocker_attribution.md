# 交接：发布阻塞归因 —— 不是「数据没改对」，而是门禁把一份好题判成了坏题

日期：2026-09-15
来源：PDF2Test 前端/工作区 agent（`src/**`、`scripts/e2e/**`）
状态：**只做了诊断，没有改后端门禁的任何一行**（按任务书「不自行修改后端门禁」）
产物：`artifacts/e2e-cdp/run-publish-attribution-2026-09-15T22-55-55-165Z/report.json`、
`…T23-00-06-246Z/report.json`、`…T23-02-36-674Z/report.json`
探针：`scripts/e2e/tauri-cdp-publish-unblock-probe.mjs`（`isAcceptanceEvidence: false`，明确标注为诊断）

---

## 0. 一句话结论

本轮第一优先级是「先解除真实发布阻塞」。我写了一个归因探针去问一个二选一的问题：
**发布被拦，是因为数据没改对，还是因为门禁代码不放行？**

答案是**两者都有，但最后一公里卡在门禁**：

1. **数据问题是真的，而且产品确实能推进**：在 `demanding-reading-passage-3.pdf` 上，
   走真实界面把 14 个答案填上之后，`hardFailures` 从
   `["WORD_LIMIT_UNPARSED","ANSWER_KEY_MISSING_SLOT","RUNTIME_COMPILER_FAILED"]` **降到 `["WORD_LIMIT_UNPARSED"]`**
   —— `ANSWER_KEY_MISSING_SLOT` 与 `RUNTIME_COMPILER_FAILED` 都被真实数据修复清掉了，
   `ANSWER_MISSING` blocker 也一起消失。**「导入 → 人工修正」这段是真的通的。**
2. **剩下那一条是误判**：`WORD_LIMIT_UNPARSED` 出现在一个 **按字母选的 summary completion** 上，
   该题型在 IELTS 里本来就没有 word limit，源文里也确实没有（见 §2）。
3. **而它给的两个补救动作都是死路**（见 §3）：`edit_text` 改不动判定所依赖的字段，
   `confirm_table` 被门禁的 resolution 盲区吃掉。于是用户**看得到错误、却永远修不掉**，
   发布永远不可能发生。这正是任务书要我验证的那件事，答案是明确的 NO。

---

## 1. 探针方法与可复现性

三个阶段，全部走真实通道（真实导入、真实 DOM 交互、真实 IPC）：

| 阶段 | 做什么 | 为什么 |
|---|---|---|
| A | 按槽位 `interaction` 通过**真实 DOM 交互**把答案改成种类相符的值 | 检验「改数据能不能放行」 |
| B | 把所有 blocking 问题用 `resolveIssue` 标成 `ignored` | 检验「确认动作能不能放行」 |
| C | 按问题自己给出的 `suggestedActions` 去改说明文字 | 检验「建议的补救动作是否有效」 |

命令行（可复现）：

```
node scripts/e2e/tauri-cdp-publish-unblock-probe.mjs --diagnostic-args --tolerate-concurrent-edits \
  --pdf fixtures/parser/demanding-reading-passage-3.pdf
```

阶段 A 的写入是**可验证的**，不是空断言：探针逐槽位记录 `versionBefore → versionAfter`、
`answerAfter`、`saved`，并单独统计 `probeSkippedSlots` / `notSavedSlots`。
本轮三次运行这两个数组都是**空的**，即「能改的都改了」这件事本身有证据。

---

## 2. 关键证据一：`WORD_LIMIT_UNPARSED` 是误判

真实产物里的 `instructionSignature`（`run-…T23-02-36-674Z`，`baseline.instructionSignatures["group-1"]`）：

```
taskType             = summary_completion
wordLimit            = undefined            ← 门禁因此判 blocking
expectedQuestionNumbers = [27,28,29,30,31]
normalizedText       = "Questions 27 - 31 Complete the summary using the list of words and phrases,
                        A-H, below. Write the correct letter, A-H, in boxes 27-31 on your answer sheet. …"
```

题目是 **"Complete the summary using the list of words and phrases, A-H"** —— 答案是**字母 A–H**。
这类 summary completion **本来就没有 word limit**，源文里也不该有。
但 `quality.rs:1548-1563` 的判定是：

```rust
if is_completion_type(task_type) {
    if signature.and_then(|value| value.get("wordLimit")).is_none() {
        push_issue(issues, hard_failures, issue(WORD_LIMIT_UNPARSED, "blocking", …));
    }
}
```

`summary_completion` 属于 completion 类型，于是「没有 word limit」被当成缺陷。
**这一条不该是 blocking，甚至不该产生。**

---

## 3. 关键证据二：它给的两个补救动作都无效

`WORD_LIMIT_UNPARSED` 的 `suggestedActions = ["edit_text","confirm_table"]`（真实产物）。逐个看：

**（诚实边界：这一条现在有运行时证据，不再是纯代码推论。见下。）**

**(a) `edit_text` 改不动它 —— 运行时已证实。** 判定读的是 `taskGroups[].instructionSignature.wordLimit`
（`quality.rs:1362: let signature = group.get("instructionSignature");`），
而 `instructionSignature` 的写入者只有两个：

| 写入点 | 改的字段 |
|---|---|
| `authoring_v2_commands.rs:1407-1421` `set_task_type` | 只改 `taskType` |
| `authoring_v2_commands.rs:1430-1451` `set_question_expression` | 只改 `expectedQuestionNumbers` / `expectedSlotCount` |

保存路径是 `apply_patch` → `refresh_quality_report`（只读存储的 signature）→ `validate_authoring`
（纯 serde schema 校验），**三者都不会重算 signature**。

**实测**（`run-publish-attribution-2026-09-15T23-14-01-281Z`，`textEditPhase`）：
把 `group-1-instructions-text` 这个说明文字节点改成
`原文 + " Write ONE WORD ONLY."`（正是 `edit_text` 建议的动作），补丁成功、**版本 16 → 17（真的保存了）**，
结果：

```text
instructionSignature.normalizedText  unchanged = true   (670 字 → 670 字，逐字节相同)
instructionSignature.wordLimit       undefined → undefined
WORD_LIMIT_UNPARSED                  still blocking
wordLimitCleared                     = false
```

**说明文字改了、存了，签名一个字都没动。** 所以 `edit_text` 是一条死路 ——
不是「用户没改对」，是改了也不会被看见。

**顺带澄清一件事（避免误报产品缺陷）**：这个节点**是**可以在真实 UI 里原位编辑的。
第一次尝试失败是**探针自己的问题**：`clickSelector` 点的是联合包围盒中心，
而这个节点是 670 字、跨多行的**行内** span，中心点会落在空白区域。
改成点「首行靠左边缘」后编辑器正常打开（`path = real-ui-inline-editor`）。
**这一点不构成产品缺陷**，已在探针里记 `clickStrategy`。

**(b) `confirm_table` 也无效。** 因为门禁无视 `resolution`（见 §4）。
用户点了「确认」，`resolution` 确实写进去了，但 `hardFailures` 与 `quality.state` 都不会变。

**结论：这条问题在界面上是「可点、可见、不可解决」的。** 这正是任务书 §任务二
「需要继续验证用户能完成修复，而不只是看到错误」的答案。

---

## 4. 关键证据三：同一个函数两套标准（运行时复现 ×2）

阶段 B 是受控 A/B：唯一变量是 `resolution`。两个独立夹具、三次运行，结果一致：

| blocker 码 | 标成 `ignored` 之后 |
|---|---|
| `ISSUE_UNRESOLVED` | **清掉了**（`authoring_v2_commands.rs:455-468` 那条判断确实读了 resolution） |
| `QUALITY_HARD_FAILURE` | **仍在**（`:430-438` 那条循环不读 resolution） |
| `QUALITY_NOT_READY` | **仍在**（`quality.rs:211` 的 `has_blocking = !hard_failures.is_empty()` 不读 resolution） |

实测输出（`run-…T22-55-55-165Z`）：

```
resolution-phase applyResult   = ok
resolution-phase hardFailures  = ["SLOT_HOST_MISSING","SIGNIFICANT_REGION_UNASSIGNED","RUNTIME_COMPILER_FAILED"]
resolution-phase preflight.passed = false
resolution-phase blockerCodes  = ["QUALITY_NOT_READY","QUALITY_HARD_FAILURE"]   ← ISSUE_UNRESOLVED 消失了
```

`ISSUE_UNRESOLVED` 消失、`QUALITY_HARD_FAILURE` 不动 —— **同一份预检、同一个动作、两个答案**。
这为你们那份静态审读提供了运行时证据。

---

## 5. 另一份夹具：阻塞是结构性的

`complex-reading.pdf` 上，阶段 A（改答案）对 `hardFailures` **完全没有影响**，
四条 blocking 问题分别是：

| 码 | targetId | suggestedActions | message |
|---|---|---|---|
| `SLOT_HOST_MISSING` | group-2 | `["edit_text","confirm_table"]` | completion slot 没有可渲染的宿主节点。 |
| `SLOT_HOST_MISSING` | group-2 | `["confirm_table","edit_text"]` | table completion 没有可渲染的 table stimulus。 |
| `SIGNIFICANT_REGION_UNASSIGNED` | document | `["assign_role"]` | 仍有显著源区域未被题目、passage 或有理由的忽略记录解释。 |
| `RUNTIME_COMPILER_FAILED` | document | `["edit_text"]` | ReadingExamSourceV2 runtime compiler 或其 schema validation 失败。 |

注意 group-2 上有**两条** `SLOT_HOST_MISSING`（同一个 code、同一个 target、不同事实）。
这是任务书「同一个 code 在两个目标上必须分别处理」的**镜像情形**：同一个 code 在同一个目标上
是两条不同的事实，因此按 target 去重是错的。

> **⚠ 本节结论已被 §9.1 修正，请以 §9.1 为准。** 「两条不同事实」成立；
> 但「前端已按这个约束实现」不成立 —— 两条的 `issueId` 撞了，preflight 只带 issueId，
> 前端在数据层就分不开这两条，只能收成一行（即吞掉一条）。这是后端需要修的。

---

## 6. 需要你们决定/修的三件事（按优先级）

1. **`WORD_LIMIT_UNPARSED` 的判定要按题型收窄。** 带选项库的 summary completion（答案是字母）
   不该要求 word limit。这是当前唯一挡住真实发布的误判，也是本轮第一优先级的直接目标。
2. **`resolution` 的语义要在决定性路径上一致。** 三处（`quality.rs:211`、`authoring_v2_commands.rs:430-438`、
   `:455-468`）必须用同一个「有效阻断」判断，否则「确认」这个动作是半生效的——
   比完全没生效更糟，因为它会让用户以为处理完了。
   按任务书的约束：**未知问题不能靠 `ignored` 绕过**；`resolution` 只能继承到「仍然是同一个事实」的问题上
   （不能只按旧 `issueId`）；同一个 code 落在两个目标上要分别判定。
3. **`suggestedActions` 要能被前端落地，否则不要给。** 现在前端完全不读这个字段（我刚接手时的现状），
   而其中 `edit_text` / `confirm_table` 对 `WORD_LIMIT_UNPARSED` 都是死路。
   建议：只有当动作在**当前门禁语义下确实有效**时才给出，前端再据此渲染按钮。
   否则宁可给一句「这条问题需要识别侧重跑」，也不要给一个点了没反应的按钮。

---

## 7. 我在自己范围内做的改动（与门禁无关）

| 文件 | 改动 |
|---|---|
| `src/features/editor/actionableIssues.ts` | 新增 `rootCauseOf()`；`mergePublishGateIssues` 去掉「已被具体记录表达过的泛化 `QUALITY_HARD_FAILURE`」 |
| `src/features/editor/ExamWorkspacePage.tsx` | `locateTarget` 返回是否真的定位到；定位失败时显示「这条问题不在题面上（文档级）」，不再静默无反应 |
| `src/features/editor/publishGateIssues.test.ts` | +7 个单测（`rootCauseOf` 的三种形状；泛化去重；无具体记录时**不清空**；根因不同不去重；**同一个 code 落在两个目标上仍分别显示**） |
| `scripts/e2e/tauri-cdp-publish-unblock-probe.mjs` | 新增归因探针（诊断用，非验收证据） |

用**真实产物回放**验证过去重效果（临时测试，跑完已删）：
两次独立运行的 blockers 都是 `34 → 31` 条，被去掉的正是 3 条泛化 `QUALITY_HARD_FAILURE`。

**仍未合并**的重复：`ISSUE_UNRESOLVED`(16) 与 `ANSWER_MISSING`(14) 描述同一个事实
（同一题位的答案未填，两个子系统各起了一个码）。合并它们需要一条跨子系统的「码别名」权威表，
那是你们的领域（与 `answer_compare_key` 形状敏感是同一类问题）。前端目前**故意不合并**，
因为按 target 合并会踩到 §5 那种「同码同目标两条事实」的情形。若你们给出别名表，前端按表合并。

---

## 8. 我没有做的事（诚实边界）

- 没有修改 `quality.rs`、`authoring_v2_commands.rs` 的任何逻辑。
- 阶段 C 没能打开原位编辑器，因此「改文字无效」只有代码层结论，没有运行时证据。
- 探针是**诊断**，`isAcceptanceEvidence: false`；它不是验收，也不构成任何 `passed`。
- 全部运行都是 `runProfile = cdp-diagnostic`（`--no-sandbox --disable-gpu`）；
  本沙箱下 `cdp-default` 无法运行（见 `findings.md` F-R8-2）。

---

## 9. 追加更正与新增事项（同一轮内，晚于上文）

上文有几处已被我自己后续核对推翻，**以此节为准**（原文保留，便于追溯我错在哪）。

### 9.1 更正 §5：group-2 确实是两个不同事实，但「前端已按这个约束实现」是错的

§5 说 group-2 上那两条是「同一个 code、同一个 target、不同事实」—— **结论对**。
但我随后一度把它「更正」成「逐字节相同、后端重复推送」，那次更正是错的：
我比的是 **preflight 的 blocker 条目**，而 preflight 只携带 `internal = issueId`。
回**质量报告**的 `issues` 数组核对，两条确实是两个不同事实：

```json
{"issueId":"phase4-SLOT_HOST_MISSING-group-2","suggestedActions":["edit_text","confirm_table"],
 "message":"completion slot 没有可渲染的宿主节点。"}
{"issueId":"phase4-SLOT_HOST_MISSING-group-2","suggestedActions":["confirm_table","edit_text"],
 "message":"table completion 没有可渲染的 table stimulus。"}
```

**关键点：两条的 `issueId` 撞了。** 而 preflight 的 `ISSUE_UNRESOLVED.internal` 只放 issueId，
于是两条不同事实在 API 边界上变成**两条不可区分的记录**。

所以 §5 最后那句「前端已按这个约束实现」**不成立**：前端拿到的数据里
已经没有「这是两条」的信息了，它**分不开**，只能收成一行 —— 也就是**吞掉其中一个事实**。
这不是前端能修的。

### 9.2 新增到 §6 的第 4 件事：`issueId` 必须真的是唯一键

`issue()` 把 issueId 拼成 `phase4-{code}-{target_id}`，同一个 code + 同一个 target 上的
两个不同事实会得到**同一个 issueId**。这同时造成三个后果：

1. preflight 丢事实（§9.1）；
2. **`resolution` 按 issueId 存 → 撞键时会把「解决了一条」串到另一条上**。
   这正好印证你们 §6.2 那条约束「`resolution` 只能继承到『仍然是同一个事实』的问题上
   （不能只按旧 `issueId`）」—— 现在的 issueId 连「同一个事实」都保证不了；
3. `ISSUE_UNRESOLVED` 的 `internal` 无法作为前端去重的可信依据。

建议：issueId 加判别位（稳定序号 / message hash），并让 `internal` 至少能区分两条。

### 9.3 更正 §7：跨来源别名已落地（但只落地了**可证明**的那一族）

§7 说「仍未合并 `ISSUE_UNRESOLVED`(16) 与 `ANSWER_MISSING`(14)」。这一条仍然成立，
但**同一轮里我合并了另一对**，而且它的别名是**可证明的**，不是猜的：

| 来源 | code | 级别 | 文案 |
|---|---|---|---|
| 本地闭包 | `ANSWER_UNRESOLVED` | warning | 第 27 题还没有答案。 |
| 发布门禁 | `ANSWER_MISSING` | blocker | 这道题还有答案没有填写。 |

两边用的是**同一个谓词**：你们在 `authoring_v2_commands.rs:439-446` 用
`answerKey[slot].kind == "unresolved"` 筛，本地闭包用的是同一个条件。
同谓词 + 同目标 → 同事实、同用户动作。于是前端登记一条别名
`ANSWER_UNRESOLVED → ANSWER_MISSING`（**只登记这一族**），真实界面上 46 行 → 32 行。

`ISSUE_UNRESOLVED`(16) 与 `ANSWER_MISSING`(14) 仍**不合并**：它们是两个不同的事实
（一个是「答案位没有可渲染的宿主」/「答案键缺失」，一个是「答案没填」）。
如果你们能给一张权威别名表，前端按表合并。

### 9.4 更正 §8：「改文字无效」现在有运行时证据了

§8 说阶段 C 没能打开原位编辑器、只有代码层结论。**已补上**：说明文字改后确实保存
（版本 16→17），但 `instructionSignature.normalizedText` 逐字节不变、`wordLimit` 仍为 `null`、
`WORD_LIMIT_UNPARSED` 仍 blocking（`run-publish-attribution-2026-09-15T23-14-01-281Z`）。
`edit_text` 对这条问题是死路，已成实测结论。

### 9.5 新增事项：`BLOCKER_LIST_TRUNCATED` 说「仅显示前 20 条」，但返回的是完整数组

`authoring_v2_commands.rs:430 / 447 / 469` 各自 `take(20)`（**按类**截断），
但 `:480` 的触发条件是**总数** `>= 20`，且返回的是**完整** `blockers` 数组、界面也全部渲染。
实测本夹具三类各 3/14/16 都没到 20（**什么都没截**），警告照样发，
界面于是出现「仅显示前 20 条」旁边列着 46 行的自相矛盾。
真被砍时用户也不知道少看的是哪一类。建议：只在**确实发生截断**时发，
且文案指明是哪一类被截。
