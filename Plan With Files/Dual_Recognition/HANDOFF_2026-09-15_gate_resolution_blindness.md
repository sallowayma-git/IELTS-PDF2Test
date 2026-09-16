# 交接：质量门禁无视 resolution（前端 agent → 识别/校验后端 agent）

日期：2026-09-15
来源：PDF2Test 前端/工作区 agent（`src/**`、`scripts/e2e/**`、`src/api/*Client.ts`）
状态：**已定位根因，未修改后端代码**（按任务书「停止扩展识别/云端后端」）

---

## 1. 一句话结论

只要一份题稿曾产生过任意一个 `severity == "blocking"` 的质量问题，它的 `quality.state` 就恒为 `blocked`，
`check_publish_preflight` 就恒返回 `QUALITY_HARD_FAILURE`。用户「采用修正」或「保留当前内容」**都不起作用**——
因为 `resolution` 在两个决定性的地方被完全忽略。这不是「正确拦下坏题」，而是「好题也永远出不去」。

影响：本 agent 负责的「真实 PDF/DOCX → 发布 → 学生端」验收因此无法完成（发布永远被拦，拿不到 NAS 包）。

---

## 2. 证据链（全部来自真实运行产物，非静态推断）

### 2.1 运行时观测
`fixtures/parser/complex-reading.pdf` 与 `complex-reading.docx` 导入后，工作区发布按钮走真实 IPC：

```
get_publish_preflight({ jobId }) →
  passed: false
  blockers:
    { code: "QUALITY_NOT_READY",     internal: "quality_state=blocked" }
    { code: "QUALITY_HARD_FAILURE",  internal: "SLOT_HOST_MISSING" }
    { code: "QUALITY_HARD_FAILURE",  internal: "RUNTIME_COMPILER_FAILED" }
    { code: "ISSUE_UNRESOLVED", targetId: "group-2",  internal: "phase4-SLOT_HOST_MISSING-group-2" }
    { code: "ISSUE_UNRESOLVED", targetId: "document", internal: "phase4-RUNTIME_COMPILER_FAILED-document" }
```

报告：`artifacts/e2e-cdp/run-chain-2026-09-15T21-43-03-427Z/report.json`

### 2.2 代码定位（两个独立的无视点 + 一个执行顺序问题）

**(a) 状态推导无视 resolution** — `src-tauri/src/ielts_grammar/quality.rs:211`

```rust
let has_blocking = !hard_failures.is_empty();
...
let state = if has_blocking { "blocked" } else if ... { "review_required" } else { "ready" };
```

`hard_failures` 由同文件 `push_issue`（`3711-3725`）在 `severity == "blocking"` 时**无条件**写入，
从不读 `details.resolution`：

```rust
if value.get("severity").and_then(Value::as_str) == Some("blocking") {
    if !hard_failures.contains(&code) { hard_failures.push(code); }
}
```

同文件 `213-220` 的注释声称已修掉「resolving or ignoring an issue changed nothing」，
但该修复只作用于 `unresolved_blocking_issues`（`221-231`）；`has_blocking` 分支在前、优先级更高，
仍然无视 resolution。**修复不完整。**

**(b) 发布门禁无视 resolution** — `src-tauri/src/authoring_v2_commands.rs:425-438`

```rust
let hard_failures = quality.get("hardFailures")...;
for failure in hard_failures.iter().take(20) {
    blockers.push(json!({ "code": "QUALITY_HARD_FAILURE", ... "internal": failure }));
}
```

对比**同一个函数**里 `455-468` 的 `unresolved_blockers` 是**有** resolution 判断的：

```rust
.filter(|issue| {
    issue.get("severity").and_then(Value::as_str) == Some("blocking")
        && !matches!(issue.pointer("/details/resolution").and_then(Value::as_str),
                     Some("resolved") | Some("ignored"))
})
```

同一函数两套标准，属明显不一致。

**(c) 执行顺序使 resolution 永远追不上** — `src-tauri/src/authoring_v2_commands.rs:977-992`

```rust
pub(crate) fn refresh_quality_report(root, job_id, authoring) -> CommandResult<()> {
    let previous_quality = authoring.get("quality").cloned();
    let mut quality = evaluate_quality(authoring, physical_shadow.as_ref());  // ← hardFailures 在此定型
    preserve_issue_resolutions(&mut quality, previous_quality.as_ref());      // ← 之后才盖回 resolution
    authoring.as_object_mut()...insert("quality", quality);
    Ok(())
}
```

`preserve_issue_resolutions`（`1017-1065`）只把 `details.resolution` 盖回 `issues` 数组，
**不可能**再影响已经定型的 `hardFailures`。所以即使 (a)(b) 都修了，这里也必须是修的一部分。

---

## 3. 建议的修复方向（供后端判断，不是本 agent 的结论）

1. `hard_failures` 需要能按 issue 过滤：要么让 `push_issue` 只在「未 resolved」时写入，
   要么让 `hardFailures` 携带 issueId（当前只是 `Vec<String>` 的 code，无法按 issue 精确过滤，
   这也是为什么修起来必须动 `quality.rs` 的数据结构或过滤时机）。
2. `check_publish_preflight` 的 `hardFailures` 循环应复用与 `unresolved_blockers` 相同的 resolution 判断。
3. 顺序上，resolution 的合并必须在 `state`/`hardFailures` 定型**之前**发生，或在定型后重新过滤。
4. 回归测试必须覆盖：
   - 把某个 blocking issue 标成 `ignored` 后，`quality.state` 从 `blocked` 变为 `ready`；
   - `check_publish_preflight` 的 `passed` 变为 `true`，且不再出现该 `QUALITY_HARD_FAILURE`；
   - 未处理的 blocking issue 仍然拦下（不能把门禁改松）。

---

## 4. 一并交接：`RUNTIME_COMPILER_FAILED` 的真实原因（识别侧产出问题）

门禁只给 `internal: "RUNTIME_COMPILER_FAILED"`，不说原因。同一份产物里保存了探针明细
（`authoring-ir-v2.shadow.json` → `quality.compilerProbes.v2Runtime`）：

| 夹具 | `issueCodes` | 明细摘要 |
| --- | --- | --- |
| `complex-reading.pdf` / `.docx` | `RUNTIME_CHOICE_SLOT_ANSWER_NOT_OPTION`, `RUNTIME_RESPONSE_ANSWER_KIND_MISMATCH` | `RUNTIME_CHOICE_SLOT_ANSWER_NOT_OPTION:q1/q2/q3:Slot interaction and answer key kind must agree; the student runtime rejects a mismatched key and the whole submission fails.` |
| `demanding-reading-passage-1.docx` | `RUNTIME_ANSWER_UNRESOLVED`, `RUNTIME_RESPONSE_KIND_OPTION_SOURCE_MISMATCH` | 13 个答案位全部未填 |

题稿实体对照（`complex-reading.docx`）：

| 字段 | 实际值 | 应为 |
| --- | --- | --- |
| `answerSlots.q1.interaction` | `radio` | — |
| `group-1.taskType` | `true_false_not_given` | — |
| `group-1.optionBank.options` | `["TRUE","FALSE","NOT GIVEN"]`（齐全） | — |
| `group-1-responses.kind` | `choice`，`optionBankRef` 正确匹配 | — |
| **`answerKey.q1`** | **`{kind:"text", values:["TRUE"]}`** | **`{kind:"option", labels:["TRUE"]}`** |

即：题面与选项都对，**只有答案键的类型错了**。请识别/自动产答案键的路径改为按槽位 interaction 产出对应类型。

**另一处不一致**：`recognitionBlockers: ["QUESTION_NUMBER_MISSING"]`（targets `group-1`/`group-2`），
但 `answerSlots` 每个都有 `questionNumber`（1..5）。请确认该 blocker 的判定依据。

---

## 5. 本 agent 已完成的对接工作（不回滚）

| 文件 | 状态 | 说明 |
| --- | --- | --- |
| `src/api/recognitionClient.ts` | 完成 | 按契约 §2.3/§2.6 包装；**wire 形状与 Rust 逐字段对齐**（`ApplyRecognitionDecisionsInputV1` / `ResultV1`），组件侧用 `DecisionBatchRequestV1` / `DecisionBatchOutcomeV1`。契约漂移检查：0 处破坏性不一致。 |
| `src/features/editor/recognitionDecisions.ts` | 完成 | 呈现规则纯函数（不生成逐项提示、单条合并建议、unverifiable 不给接受）。 |
| `src/features/editor/RecognitionPanel.tsx` | 完成（只读路径已验） | 写路径无真实候选项可验。 |
| `src/features/editor/studentPreview.ts` | 完成 | 新增 `answerKeyIssues` 与 `runtimeIssueCount`。 |
| `src/services/readingRuntimeV2.ts` | 完成 | 新增 `validateReadingAnswerKeyKinds()`，与 Rust 运行时同一判定。 |
| `scripts/e2e/**` | 完成 | CDP 真实通道 + 12 步产品链；第 12 步 `preview-and-gate-agree` 把「预览不得假完成」变成断言。 |

**本 agent 未改** `src-tauri/src/ielts_grammar/**`、`src-tauri/src/authoring_v2_commands.rs` 的任何逻辑
（只读定位）。若后端确认归属，请按第 3 节修复；本 agent 可配合补前端回归。

---

## 6. 需要后端回复的问题

1. `resolution` 的语义最终由谁落地？若由前端提供「确认/忽略」入口，需先修第 3 节，否则按钮无效。
2. `hardFailures` 是否可以改为携带 issueId（便于按 resolution 精确过滤）？
3. `get_recognition_decision` 是否按契约补齐 `cloudStatus` / `localStatus` / `items`，还是双方确认以 `chains` / `actionable` 为准并改契约文档？
4. `recognitionBlockers: QUESTION_NUMBER_MISSING` 与 `answerSlots[].questionNumber` 已存在的矛盾如何解释？

---

## 7. 补充交接：契约漂移检查器存在**结构性盲区**（命令包装层）

`scripts/recognition/contract-drift.mjs` 在写路径上曾报「0 处破坏性不一致，1 处需要留意」，
但写路径**当时仍然是坏的**。原因是检查器只比对 Rust **结构体**的字段名（`TS_MAP` ↔ struct），
看不到 **Tauri 命令包装层**：

```rust
// src-tauri/src/lib.rs:980
async fn apply_recognition_decisions(input: Value, app: AppHandle) -> CommandResult<Value>
```

命令只接受一个 `input` 参数，因此 IPC 参数必须整体包在 `input` 键里。字段名全对但没包 `input` 时，
真实后端返回：

```
invalid args `input` for command `apply_recognition_decisions`: command apply_recognition_decisions missing required key input
```

**结论：字段名对齐 ≠ 接线可用。** 凡是跨 IPC 边界的形状，静态检查无法覆盖，
必须至少有一次真实调用的证据。本 agent 已补上这条真实调用验证
（`scripts/e2e/tauri-cdp-recognition-write-path.mjs`，8 步全通过），并加了 2 个单测把
「顶层只能有 `input` 一个键」钉死。

若后续要让漂移检查器覆盖这一层，建议它额外解析 `lib.rs` 里 `#[tauri::command]` 函数的
**参数名列表**，与前端 `command(name, args)` 的顶层键集合比对。

