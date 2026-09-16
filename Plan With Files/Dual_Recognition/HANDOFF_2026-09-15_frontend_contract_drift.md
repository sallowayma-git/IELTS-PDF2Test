# 交接：识别闭环前后端契约状态（2026-09-15 更新）

> 面向：前端执行 agent（`src/**` 的唯一写入方）
> 来源：识别/云端后端 agent（`src-tauri/src/reconcile/**`、`src-tauri/src/schema/recognition_v1.rs`）
>
> **状态更新**：本文件首版写作时读取面完全未对齐；随后前端已通过
> `normalizeDecisionView` 修复读取面。**写入面（accept/reject）至今未修，是当前唯一阻塞。**

---

## 1. TL;DR

后端契约实际版本为 **v2**（`Plan With Files/Dual_Recognition/RECOGNITION_LOOP_CONTRACT.md`，
与 `src-tauri/src/schema/recognition_v1.rs` 逐字段一致）。

| 面 | 状态 | 说明 |
|---|---|---|
| 读取面 `get_recognition_decision` | ✅ **已对齐** | `RecognitionDecisionRawV1` + `normalizeDecisionView` 做双形状容错，读取面可用 |
| 写入面 `apply_recognition_decisions` 请求 | ❌ **未对齐** | 仍发 `{itemId, decisions:[{decisionId,action}]}`，后端要 `{requestId, batchId, baseEditVersion, accept[], reject[]}` |
| 写入面 `apply_recognition_decisions` 返回 | ❌ **未对齐** | 仍读 `accepted/rejected/stale/failed[]`，后端返回 `outcomes[]`（每项带 `kind`）+ `view` |

**后果**：点「采用修正」/「保持现状」**不会写入任何内容**，且不会弹出正确反馈。
这是「静默空操作 + 误报失败」的组合，见 §2。

验证命令：`npm run verify:recognition:contract` → 当前 **2 处破坏性不一致，均出在写入面**。

---

## 2. 写入面的失效链路（逐步）

以点击「采用修正」为例：

1. `RecognitionPanel.tsx:86-92` 发出
   `{ itemId, batchId, baseEditVersion, requestId, decisions: [{decisionId, action}] }`。
2. 后端 `ApplyRecognitionDecisionsRequestV1` 只有
   `request_id` / `batch_id` / `base_edit_version` / `accept` / `reject`，
   **且未设 `deny_unknown_fields`** → `itemId` 与 `decisions` 被**静默丢弃**，
   `accept` / `reject` 因 `#[serde(default)]` 取空数组。
3. 后端于是返回「成功」：`outcomes: []`，权威稿**一字未改**。
4. `RecognitionPanel.tsx:94` 读 `result.accepted.length` → `undefined.length`
   → **TypeError**，被 `catch` 吞掉，显示「这次处理没有生效，请重试。」

即：**用户看到的是失败，实际发生的是「成功但什么都没做」**。两者都不对。

更危险的是第 2 步的静默性——如果前端恰好不发 `itemId`（或后端补了 `deny_unknown_fields` 之外的容错），
面板会显示「已记录这次处理。」而权威稿依然没变。**必须靠契约检查而不是靠报错发现这类问题。**

---

## 3. 写入面最小修复清单

### 3.1 `src/api/recognitionClient.ts`

沿用读取面已经采用的思路（对外保持组件友好的形状，内部做归一化），写入面同样加一层转换：

```ts
// 对外形状保持不变（组件无需改动）
export interface ApplyRecognitionDecisionsInputV1 {
  itemId: string;
  batchId: string;
  baseEditVersion: number;
  requestId: string;
  decisions: Array<{ decisionId: string; action: "accept" | "reject" }>;
}

// 后端实际请求形状
interface ApplyRecognitionDecisionsWireRequest {
  requestId: string;
  batchId: string;
  baseEditVersion: number;
  accept: string[];
  reject: string[];
}

// 后端实际返回形状
interface ApplyRecognitionDecisionsWireResult {
  schemaVersion: string;
  requestId: string;
  batchId: string;
  editVersionBefore: number;
  editVersionAfter: number;
  replayed: boolean;
  outcomes: Array<{
    decisionId: string;
    kind: "applied" | "rejected" | "superseded" | "failed";
    reasonCode?: string | null;
    message: string;
    appliedAt?: string | null;
    undo?: unknown;
  }>;
  view: RecognitionDecisionRawV1;
}

export async function applyRecognitionDecisions(
  input: ApplyRecognitionDecisionsInputV1
): Promise<ApplyRecognitionDecisionsResultV1> {
  const accept = input.decisions.filter((d) => d.action === "accept").map((d) => d.decisionId);
  const reject = input.decisions.filter((d) => d.action === "reject").map((d) => d.decisionId);
  const wire = await command<ApplyRecognitionDecisionsWireResult>("apply_recognition_decisions", {
    requestId: input.requestId,
    batchId: input.batchId,
    baseEditVersion: input.baseEditVersion,
    accept,
    reject
  });

  const ids = (kind: string) => wire.outcomes.filter((o) => o.kind === kind).map((o) => o.decisionId);
  const failed = wire.outcomes.filter((o) => o.kind === "failed");
  const view = normalizeDecisionView(wire.view.get ?? wire.view);   // 若 view 已是原始形状则直接归一

  return {
    schemaVersion: wire.schemaVersion,
    itemId: input.itemId,
    batchId: wire.batchId,
    editVersion: wire.editVersionAfter,
    replayed: wire.replayed,
    accepted: ids("applied"),
    rejected: ids("rejected"),
    stale: ids("superseded"),
    failed: failed.map((o) => ({
      decisionId: o.decisionId,
      code: o.reasonCode ?? "UNKNOWN",
      message: o.message
    })),
    summary: {
      open: view.items.filter((item) => item.status === "open").length,
      accepted: ids("applied").length,
      rejected: ids("rejected").length,
      superseded: ids("superseded").length
    }
  };
}
```

**要点**

- `editVersion` 必须取 `editVersionAfter`（后端不再返回 `editVersion`）。
- 返回值里**带上归一化后的 `view`**，`RecognitionPanel` 可直接用它刷新，省掉 `await load()` 一次往返
  （契约 §5 明确「无需二次读取」）。
- `superseded` 不是失败。当前实现把它并进 `stale`（前端既有语义「题稿已经改过而没有应用」是**正确**的），
  文案上说清「你的修改赢了」即可。

### 3.2 `RecognitionPanel.tsx`

修完 3.1 后本组件**无需改动**（它消费的是 `ApplyRecognitionDecisionsInputV1` / `...ResultV1` 两个对外形状）。
可选的改进：用返回的 `view` 直接 `setView(...)` 替代第 101 行的 `await load()`。

---

## 4. 附带发现：`queued` / `partial` / `unusable` 被渲染成原文

`recognitionClient.ts` 的 `CHAIN_STATE_TO_STATUS` 未覆盖 `StageStateV1` 的三个真实取值，
导致 `describeCloudStatus` 走 `default` 分支，把英文原样显示给用户：

| 后端 `chains.cloud.state` | 当前渲染 | 应为 |
|---|---|---|
| `queued` | 「云端核验还没有运行，下面显示的都是本机识别结果。」 | 「云端识别排队中，本地结果已经可以编辑。」 |
| `partial` | 「云端核验状态：partial。」 | 「云端只核验了部分内容，其余需要人工确认。」 |
| `unusable` | 「云端核验状态：unusable。」 | 映射成 `unavailable`，走 `describeCloudReason` |

其中 **`queued` 最要紧**：本地优先流程下，本地识别完成的那一刻云端正是 `queued`
（契约 §3.2「已入队但未开始」）。当前映射成 `not_started` + 现文案，会让用户以为
「云端没跑，只有本机结果」——而实际上云端**正在排队并会跑**。这恰好打掉了
「本地先出稿、云端仍排队」这个核心体验。

建议：

```ts
const CHAIN_STATE_TO_STATUS: Record<string, string> = {
  queued: "queued",            // 新增：保留排队语义
  not_run: "not_started",
  // …其余不变
  partial: "partial",          // 新增
  unusable: "unavailable",     // 新增：复用现有 unavailable 文案分支
  canceled: "not_started"
};
```

并在 `describeCloudStatus` 补 `case "queued"` / `case "partial"`。

---

## 5. 后端侧同时修掉的一个自身缺陷

用契约交叉核对时发现**后端自己**的一处真实漂移（已在本次修复）：

`DecisionFieldV1::as_str()` 返回 camelCase（`optionBank`），而 `#[serde(rename_all = "snake_case")]`
产出 `option_bank`。该值同时出现在三处外部可见字符串：

1. `decisionId` 末段 → `d:slot:slot-14:optionBank`（**不满足契约 schema 的 `^d:[a-z_]+:.+:[a-z_]+$`**）；
2. 原文件核验证据的 `anchorKind` → `source_finding:Slot:slot-14:optionBank`；
3. `recognition_decisions_v1` 的字段键。

受影响 6 个字段（`group_kind`/`option_bank`/`slot_interaction`/`slot_placement`/`source_coverage`/
`slot_participation`）。已改为 snake_case，并新增护栏测试
`schema::recognition_v1::tests::as_str_matches_serde_for_every_wire_enum`：对 6 个线上枚举的
**全部**变体断言 `as_str() == serde 渲染`，并断言 `decisionId` 末段满足 schema 字符集。

---

## 6. 复现与回归

```bash
npm run verify:recognition:contract          # 或 node scripts/recognition/contract-drift.mjs
```

三面交叉核对：Rust 类型（真源，`src-tauri/src/schema/recognition_v1.rs`）↔ JSON Schema ↔ 前端 TS。
判定口径：

- `✗ 消费方缺少/读不到` = 消费方读了生产方永不发送的字段 → **破坏性**（运行时 `undefined`）；
- `! 多余` = 生产方发了但消费方未消费 → 仅提示。

**当前输出**：2 处破坏性（均在写入面）+ 3 处提示；**Rust ↔ Schema 为 0 处不一致**。

读取面的 v1 契约名（`currentEditVersion`/`items`/`localStatus`/`cloudStatus`/`cloudReasonCode`）已登记为
**有意容错**（`TS_MAP` 的 `allow` 列表）——它们是双形状回退的分支，不算漂移。

修完写入面后应输出 `契约漂移检查：0 处破坏性不一致`，退出码 0。

```bash
cd src-tauri && cargo test --lib                              # 660 passed / 0 failed / 11 ignored
cd src-tauri && cargo test --lib reconcile::
cd src-tauri && cargo test --lib schema::recognition_v1::
```

---

## 7. 验收标准的影响

| # | 通过标准 | 当前状态 |
|---|---|---|
| 1 | 本地先完成，云端仍排队，草稿已可读写 | 后端已实现（`scheduler::run_job_inner` 本地成功后先 `set_item_status_ready`）；**但 §4 的 `queued` 文案会误导用户以为云端没跑** |
| 2 | 云端完整识别与本地稿一致，不产生多余待确认项 | 后端测试通过；读取面已修，可验证 |
| 3 | 本地与云端有分歧，核验后只形成一份统一建议 | 后端测试通过；读取面已修，**建议卡可显示** |
| 4 | 各路一致但缺原文证据，不被误判为已验证 | 后端测试通过；读取面已修 |
| 5 | 云端运行时用户修改，迟到结果不覆盖 | 后端 `superseded` 路径已有；**前端 `stale[]` 可接收（3.1 已映射），但写入面不修则到不了这一步** |
| 6 | 自动修正符合规则且持久化 | 后端测试通过；`autoApplied` 已可显示 |
| 7 | 接受、拒绝及重启恢复正确，重复操作不重复写入 | 后端 journal 幂等已测；**写入面不修则整条不成立** |
| 8 | 批量单失败不阻塞其他文件，取消可持久化 | 后端已测 |
| 9 | PDF 与 DOCX 均走完整链路 | 后端已测 |
| 10 | 模型不支持/超时/非法输出/裁决失败有明确状态 | 后端已测 |

**结论**：读取面修复后，验收标准 2/3/4/6 已具备 UI 层可验证条件；**5/7 仍被写入面阻塞**。

---

## 8. 剩余缺口（照实声明）

- 本文覆盖**服务与命令层 + 类型契约层**的一致性。
- **未验证**：写入面修复后的真实端到端链路、真实云服务往返、跨仓学生端计分。
  需两 agent 集成后按 `AGENTS.md` 走真实产品链路
  （导入 → 本地出稿 → 云端裁决 → 接受/拒绝 → 预览 → 发布 → 学生端计分）。
  **CLI/单测通过不等价于产品链路通过。**
- `ajv` 虽在 devDependencies，但本次**有意不新增手写样例 payload**——那会引入第三份真源并再次漂移。
  契约正确性由「Rust 单测 + 本检查脚本」两处保证。
- 按所有权约定（契约 §1.2）`src/**` 归前端 agent 独占写入，后端 agent **未改动任何前端文件**，
  只提供本清单。
