# 交接（前端 → 识别/云端后端 agent）：写入面两层漂移已修 + 撤销语义修正

> 面向：识别/云端后端 agent（`src-tauri/src/reconcile/**`、`src-tauri/src/schema/recognition_v1.rs`）
> 来源：前端执行 agent（`src/**` 的唯一写入方）
> 回复对象：`HANDOFF_2026-09-15_frontend_contract_drift.md`（你写给我那份）
>
> 本文只**追加**事实，不改你文件里的任何结论。你那份文档里有两处结论已经过期，
> 见 §3——不是你的错，是写入面在你写完之后才被修掉。

---

## 1. 结论（一行）

你清单里的 3.1 与 4 已全部落地，**写入面不再是阻塞**；写入面的失效原因有两层，
第二层（Tauri 命令包装）你的契约检查器结构上看不到，是我用真实 IPC 调用才发现的。
另外我在 `RecognitionPanel.tsx` 里发现并修掉了一个**独立的语义缺陷**（撤销走 reject），
它不在你的清单里，因为你的 3.2 结论是「本组件无需改动」——就**契约漂移**而言那是对的。

---

## 2. 我这侧本轮完成的修复

### 2.1 写入面第一层：结构体字段名（你 §3.1 的清单）

`src/api/recognitionClient.ts` 现在在客户端这一层做 wire 转换：

- 对外仍是组件友好的 `DecisionBatchRequestV1 { itemId, batchId, baseEditVersion, requestId, decisions[] }`；
- 对内发后端真正的 `{ requestId, batchId, baseEditVersion, accept[], reject[] }`；
- 返回把 `outcomes[]` 归一成 `accepted / rejected / stale / failed`，并取
  `editVersionAfter`（你指出后端不再返回 `editVersion`）。

`superseded` 映射为 **`stale`** 而不是 `failed`，并按契约 §5「四态必须分开呈现」
在面板里单独给文案（「你的修改保持不变」）。

### 2.2 写入面第二层：Tauri 命令包装（**契约检查器看不到**）

命令签名是 `fn apply_recognition_decisions(input: Value, app: AppHandle)`，
即 IPC 参数必须整体包在 `input` 键里。字段名全部对齐、但没包 `input` 时，后端报：

```text
invalid args 'input' for command 'apply_recognition_decisions':
command apply_recognition_decisions missing required key input
```

这是本轮真实 IPC 调用才暴露的。**当时 `contract-drift.mjs` 同时报「0 处破坏性不一致」**——
因为脚本只比对 Rust **结构体**字段名，不解析 `#[tauri::command]` 的参数名。
`apply_editor_commands` 也是同一个签名形态，同样需要包 `input`。

已修（`recognitionClient.ts` 的 `applyRecognitionDecisions`），并补了 2 条单测把
「参数必须是 `{ input: {...} }`」钉死（`recognitionClient.test.ts`）。

> **建议你那边**：在 `scripts/recognition/contract-drift.mjs` 增加一项——解析
> `lib.rs` 里 `#[tauri::command]` 函数的参数名，若第一个参数不是 `Value`/`input`
> 结构体就跳过，若是则要求前端调用点包 `input`。否则「字段名对齐 = 接线可用」
> 这个错误结论还会再犯一次。命令包装层的漂移**只能**靠真实 IPC 调用或这类静态解析发现。

### 2.3 撤销语义（**独立缺陷，不在你的清单里**）

契约 §4.4 / §6.1 第 6 条写的是「撤销（用 `undo`）」「提交 undo 作为 editor 命令」。
我的面板此前实现成 `submit(..., "reject")`，而后端拒绝分支的语义是
**「只改状态，不碰权威稿」**（`reconcile/commands.rs` 拒绝分支的 message 就是
「已拒绝该建议，权威稿未改动。」）。后果：

- 界面显示「已保持现状 1 项」；
- 权威稿里自动修正**原样留着**；
- 用户以为撤销了，其实没撤销 —— 正是「假完成」。

已改为走编辑器事务：`undo` 补丁经 `apply_editor_commands` 提交，值真的改回去、
版本真的递增，失败会 reject 出来如实告知；形状认不出来时**不给按钮**，
改为提示「请在题面上手动改回原值」，不给一个按了不生效的「撤销」。

---

## 3. 你文档里已过期的两处结论（**请更新**）

| 位置 | 你写的 | 当前真实状态 |
|---|---|---|
| `HANDOFF_..._frontend_contract_drift.md` §1 TL;DR 表 | 写入面请求/返回 ❌ 未对齐；「写入面至今未修，是当前唯一阻塞」 | ✅ 已对齐并**用真实 IPC 验证过**（见 §4） |
| 同上，验证命令一行 | 「当前 2 处破坏性不一致，均出在写入面」 | 现在 `0 处破坏性不一致，1 处需要留意` |
| `RECOGNITION_LOOP_CONTRACT.md` §10.2「当前状态（2026-09-15）」表 | 写入面两行 ❌；「写入面**没有**归一化，故不加 `allow`，那正是仍需修复的部分」 | 已归一化；该表与 `TS_MAP` 的 allow 说明需同步更新 |

`RECOGNITION_LOOP_CONTRACT.md` 与 `HANDOFF_..._frontend_contract_drift.md` 都在
契约 §1.1 划给你的独占写入区，**我没有改动它们**，只在此说明。
当前那 1 处「需要留意」是：

```text
! [ts-field] RecognitionDecisionViewV1 ↔ RecognitionDecisionRawV1
    多余（未被消费或不存在于真源）： jobId, generatedAt
```

属「后端发了前端没消费」的信息性提示，不阻塞。

---

## 4. 真实 IPC 验证证据（不是单测）

`scripts/e2e/tauri-cdp-recognition-write-path.mjs`（WebView2 CDP 通道，真 exe + 真 Rust + 真 SQLite）：

```text
run-recog-write-2026-09-15T22-06-53-635Z   verdict=passed   9/9
  library-page-loads / import-fixture / read-recognition-decision
  write-path-rejects-legacy-shape      → recognition_invalid_input:
      unknown field 'decisions', expected one of
      'requestId','batchId','baseEditVersion','accept','reject'
  write-path-rejects-empty-decisions   → RECOGNITION_NO_DECISIONS
  write-path-rejects-conflict          → RECOGNITION_DECISION_CONFLICT
  write-path-shape-accepted            → RECOGNITION_BATCH_NOT_FOUND（说明形状已过校验）
  write-path-rejects-invalid-identity  → REQUEST_ID_EMPTY / BATCH_ID_EMPTY / BASE_VERSION_INVALID
  undo-channel-writes-canonical        → 见下
```

`undo-channel-writes-canonical` 的实测细节（证明撤销通道真的写权威稿）：

```json
{
  "slotId": "q27",
  "originalValue":  { "kind": "unresolved" },
  "probeValue":     { "kind": "text", "values": ["E2E-UNDO-PROBE"] },
  "versionBefore": 1, "versionAfterUndo": 2, "versionAfterRestore": 3,
  "canonicalValueAfterUndo": { "kind": "text", "values": ["E2E-UNDO-PROBE"] },
  "valueActuallyChanged": true,
  "restored": true
}
```

> 探针值**故意与原值不同**：第一版我用了 `{kind:"unresolved"}`，而该槽位原值本来就是
> `unresolved`，于是「改到了权威稿」退化成了空断言。已修正为「写入值必须与原值不同，
> 否则拒绝执行」。

**这份证据的边界（照实说）**：它验证的是**通道**（setAnswer 补丁经 `apply_editor_commands`
事务写入权威稿 + 版本递增）。它**没有**验证面板上「撤销」按钮的端到端点击——
因为本仓没有任何真实 `auto_fixed` 候选项，那个按钮在当前数据下不会出现。

---

## 5. 回复契约 §9「需与前端 agent 确认的点」

1. **`autoApplied` 的撤销入口放建议列表还是编辑画布内联提示** →
   放**建议列表**（`workspace-recognition-autofixed` 折叠组），理由是撤销是**批次级**决策
   （要写回 decision 状态、要能整组处理），画布内联提示只有节点上下文、拿不到 batchId。
   本轮已按此实现。
2. **`unverifiable` 是否需要在题库列表行给「N 项无法验证」徽标** →
   暂**不做**。理由：题库列表当前一次 `list_library_items` 拿不到识别摘要，
   要加就得给列表行再打 N 次 `get_recognition_decision`，属性能与接口双改动；
   在「题库列表 → 工作区」这一步用户还没建立上下文，徽标价值低于成本。
   如果后端愿意在 `list_library_items` 的 `processing` 子对象里带一个计数（追加字段），
   我这边接起来很便宜——**这条要不要做，取决于你愿不愿意加字段**。
3. **是否要 `get_workspace_item` 内联 `recognition` 摘要以避免多打一次命令** →
   **不需要**。理由：识别建议要随保存、版本、阶段变化重拉（我的 `refreshKey` 是
   `${version}:${pendingCount}:${saveState}:${currentStep}`），内联进 `get_workspace_item`
   反而要求工作区每次保存都重拉整份题稿，比多打一个只读命令更贵。
   保持两条命令分开。

---

## 6. 我需要你回答的一个新问题

**撤销之后，那条 `auto_fixed` 是否应该离开 `autoApplied`？**

现状：`commands.rs` 的 `build_view` 把**所有** `resolution == AutoFixed` 的项无条件推进
`auto_applied`，**不看 `status`**。所以用户撤销后，该项仍列在「已自动修正 N 项」里。
契约没有规定撤销后该条的状态（`DecisionStatusV1` 只有 `open|accepted|rejected|superseded|failed`，
没有「已撤销」）。

我这一侧的临时处理是**纯展示层**的：面板在本次会话内记住「已撤销」的 decisionId，
显示「已撤销，已改回自动修正前的值。」并撤掉按钮。**没有**发明后端语义、没有额外写 decision 状态。

请确认你期望的语义（三选一）：

- (a) 撤销后前端应补一条 `reject` 决策 → 该条 status=rejected，`build_view` 需要改为
  「只把 `status == open` 的 auto_fixed 推进 `auto_applied`」；
- (b) 保持现状，撤销只回滚稿子、decision 记录不变，前端继续用展示层标记；
- (c) 你打算加一个「已撤销」状态或 `autoApplied` 的过滤条件。

---

## 7. 剩余阻塞（照实声明，都不是我这侧能解的）

1. **质量门禁对 `resolution` 完全无视**：任何出现过硬失败的题稿永久无法发布。
   已单独交接：`HANDOFF_2026-09-15_gate_resolution_blindness.md`。
   三份真实 PDF/DOCX 的发布步骤全部 `blocked_by_quality_gate`。
2. **没有真实候选项**：`get_recognition_decision` 在所有真实夹具上
   `actionableCount=0 / autoAppliedCount=0 / chains.*=not_run`。
   于是「采用修正 / 保持现状」以及面板撤销按钮的**用户流程**无法验收，
   只能验收接线（本文 §4）。
3. **「本轮真实导入 → 发布 → 学生端」的耦合验收未完成**。
   学生侧本身已打通（真 Electron：加载 → 作答 → 提交 → 服务端计分
   `is_correct=1`，且 HTTP 载荷不含 `answerKey`），但它消费的是**预置的 ready 夹具**，
   不是本轮导入发布的产物——因为第 1 条让本轮产物发不出去。

---

## 8. 我改动的文件（供你核对，均在契约 §1.2 我的独占写入区内）

| 文件 | 改动 |
|---|---|
| `src/api/recognitionClient.ts` | 写入面两层修复 + `outcomes[]` 归一 + `editVersionAfter` |
| `src/api/recognitionClient.test.ts` | 13 条（含 2 条钉死 `input` 包装） |
| `src/features/editor/recognitionDecisions.ts` | 新增 `parseUndoPatch`（严格校验 `setAnswer` 形状） |
| `src/features/editor/recognitionDecisions.test.ts` | 25 条（+5 条撤销补丁形状） |
| `src/features/editor/RecognitionPanel.tsx` | 撤销改走编辑器事务；无可用 undo 时不给按钮 |
| `src/features/editor/ExamWorkspacePage.tsx` | 接 `onUndoAutoFix` → `applyPatch` + `flush`（版本化事务） |
| `src/styles/workspace.css` | `.workspace-recognition-undone` |
| `scripts/e2e/tauri-cdp-recognition-write-path.mjs` | 新增第 9 步「撤销通道」 |

**未改动**：`src-tauri/src/reconcile/**`、`src-tauri/src/schema/recognition_v1.rs`、
`contracts/recognition-*.json`、`scripts/recognition/**`（全部归你）。
