# 交接单：手工接受的项撤销闭环不成立（R13）

日期：2026-09-16　　提出方：前端 agent　　归属：**后端独占区**（`reconcile/**`、`schema/recognition_v1.rs`）
对应 findings：`F-R13-2`　　相关文档：`Plan With Files/Dual_Recognition/CONTROLLED_LLM_SCENARIO_2026-09-16.md` §4

## 1. 期望（来自你们自己的文档）

`CONTROLLED_LLM_SCENARIO_2026-09-16.md` §4 写的是：

> - 点**接受** → 题稿 `q14` 写入 `stencilling`，该项离开待办，撤销入口出现
> - 点**撤销** → 题稿 `q14` 回到空，该项离开「已自动修正」区
> - 重要：撤销对**手工接受**的项也应按同一闭环退出

场景里的项 `autoApplyEligible = false`，所以它**只会**被用户手工接受 ——
也就是说 §4 的第三条要求正是这个场景的主路径，不是边角情况。

## 2. 实际（代码定位）

两个**互相独立**的阻断点，各自都足以让闭环不成立。

### 阻断点 A：呈现层把该项丢掉了

手工接受时后端只改两个字段（`reconcile/commands.rs:786-791`）：

```rust
let mut persisted = items[*index].clone();
persisted.auto_applied = false;
persisted.applied_at = Some(now.clone());
persisted.undo = super::adjudicate::undo_patch_for(&items[*index]);
// status 由下面的 set_decision_status 写成 Accepted
```

**`resolution` 没有动**，仍是 `NeedsReview`。而 `build_view`
（`reconcile/commands.rs:202-215`）的两个出口是：

| 出口 | 条件 | 手工接受的 `NeedsReview` 项 |
| --- | --- | --- |
| `auto_applied` | `resolution == AutoFixed && status == Accepted` | ✗（resolution 是 NeedsReview） |
| `actionable` | `is_actionable()` | ✗（见下） |

`is_actionable()`（`schema/recognition_v1.rs:481-484`）：

```rust
self.resolution != DecisionResolutionV1::AutoFixed
    && matches!(self.status, DecisionStatusV1::Open | DecisionStatusV1::Failed)
```

status 已是 `Accepted` → ✗。

**结论：该项两份列表都不进，从视图里整体消失。**

前端 `normalizeDecisionView`（`src/api/recognitionClient.ts:147-153`）在
`raw.items` 缺省时用 `autoApplied + actionable` 拼 `items` —— 这正是
`get_recognition_decision` 的实现形状。该项不存在于任何一份列表，
前端**没有任何可渲染的对象**，撤销入口无从出现。

### 阻断点 B：命令层也会拒绝

即便前端绕过 A、硬发 `undo[]`，撤销分支的门（`reconcile/commands.rs:662-671`）是：

```rust
if item.status != DecisionStatusV1::Accepted || item.resolution != DecisionResolutionV1::AutoFixed {
    // → RECOGNITION_NOT_UNDOABLE
}
```

手工接受的项 `resolution == NeedsReview` → 判 `RECOGNITION_NOT_UNDOABLE`。

### 值得注意：数据层面其实是齐的

`commands.rs:790` 在**接受时就算好了撤销补丁**（`adjudicate::undo_patch_for`，定义在
`adjudicate.rs:404-418`），而它只用 `field / proposed_patch / local_value`，**不看 resolution**。
所以「可回滚的目标值」已经躺在库里了 —— 缺的只是上面 A、B 两步。

## 3. 两种最小修法（择一，都在后端独占区）

### 方案 1（推荐）：让「已被用户确认」成为一个一等状态，而不是靠 resolution 兼职

`resolution` 表达的是「**三路比较的结论**」（一致/自动修正/待确认/无法验证），
`status` 表达的是「**用户/系统对它做了什么**」。手工接受是后者的事件，
把它塞进前者（翻成 `AutoFixed`）会污染语义：`AutoFixed` 的字面意思是
「系统自动写入的」，而这条是**用户**确认的 —— 之后任何按 `AutoFixed` 分支的
逻辑（比如「自动修正可以整体重算」）都会把用户的手工决定当成系统产物。

因此建议：

1. `build_view` 增加第三个出口（或把 `auto_applied` 的判据改成
   「有生效中的修正」= `status == Accepted && applied_at.is_some() && undo.is_some()`），
   让手工接受的项**带着撤销补丁**呈现出来，并让前端能区分
   「系统自动写入」与「用户确认写入」（建议加一个 `appliedBy: "auto" | "user"` 之类的字段，
   否则前端只能靠 `auto_applied` 布尔值反推，而它现在恒为 `false`）。
2. 撤销分支的门从「必须 `AutoFixed`」放宽为「**必须有可回滚的补丁且在位**」，
   即把 `resolution != AutoFixed` 这一条去掉，保留
   `status == Accepted` 与 `applied_answer_still_in_place != Some(false)` 两条。
   第二条是真正的保护（用户后来改过就拒绝），不该动。

### 方案 2（最小改动，但语义有代价）：手工接受时把 resolution 也翻成 `AutoFixed`

只改 `commands.rs:786-791`，`persisted.resolution = AutoFixed`。
一行就能让 A、B 同时通过。**代价**是上面说的语义混淆：
`summary.auto_fixed` 会把用户手工确认的项计进「已自动修正」，
界面文案也会变成「已自动修正」——用户明明是自己点的「采用修正」，
却被告知是系统自动改的。如果你们选这条路，建议至少同时把
`auto_applied` 保留为 `false` 并在前端用它区分文案（前端已按 `auto_applied` 语义预留）。

## 4. 前端的现状（不需要你们配合改）

前端**已经**按「后端会给出可撤销的手工接受项」写好了：

- `src/features/editor/recognitionDecisions.ts::undoState()` 的判据优先级是
  `status === "undone"` → 权威稿值已等于撤销目标 → 有无可回滚补丁；
  **不要求 `resolution === "auto_fixed"`**（只在第 2 条兜历史数据时限定 `auto_fixed`）；
- 撤销按钮走 `apply_recognition_decisions` 的 `undo[]`（正式命令，与接受同一事务），
  失败时**原样显示后端给的 message**，不自己编文案；
- 后端加了 `appliedBy` 之类的区分字段，前端可以直接消费；不加也能跑。

所以只要 A 让该项出现在 `actionable` 或 `autoApplied` 任一份列表里、
B 放开那道门，前端**无需改动**即可完成闭环。

## 5. 实证入口

`node scripts/e2e/tauri-cdp-controlled-service.mjs`

该脚本会：起受控服务 → 写 profile → 真实导入 → 等云端 `succeeded` →
对照 `expected-decisions.json` → 在界面上点「采用修正」→ 记录
**撤销入口是否出现**、以及后端视图里该项落在哪一份列表。

场景 `undo-manual-accept` 在入口缺失时**判 failed**（不是 not-executable），
并在 `detail` 里带上 `panel`（DOM 三态）与 `backendView`（`inActionable` / `inAutoApplied`）
两组对照，用于区分「前端没渲染」与「后端没给」。
