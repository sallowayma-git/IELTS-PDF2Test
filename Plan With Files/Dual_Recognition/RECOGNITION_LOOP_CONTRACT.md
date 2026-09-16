# 识别闭环：共享文件所有权 + 真实接口契约（双 agent 对齐基线）

> 状态：**v2（已实现，可对着本文件写前端）**。v1 是设计意图，v2 与代码逐字段一致；
> 两者的差异见 §7「v1 → v2 的偏差与理由」。
> 基线 HEAD：`c8c3d3b`（分支 `main`）+ 本次识别闭环改动（未提交）。
> 改动本文件必须双方同意；字段一经发布只做**追加**，不做语义改写。

目标闭环：**本地先出稿 → 云端并行全量识别 → 后台统一裁决 → 用户只处理少量疑点**。

代码入口：`src-tauri/src/reconcile/`（`candidate` → `source` → `rules` → `adjudicate` →
`store` → `commands` → `engine`），契约类型在 `src-tauri/src/schema/recognition_v1.rs`。

---

## 1. 共享文件所有权（写入面互斥）

### 1.1 识别/云端后端 agent 独占写入（另一 agent 只读）

| 路径 | 说明 |
|---|---|
| `src-tauri/src/processing/**` | 持久化队列、双链调度、取消、恢复 |
| `src-tauri/src/auto_pipeline.rs` | 本地识别流水线 + `generate_cloud_reading_outline` |
| `src-tauri/src/recognition/**` | 本地识别（QLG / direct canonical） |
| `src-tauri/src/reconcile/**` | **新增**：统一对齐 + 确定性规则 + 统一裁决 + 安全应用 + 命令层 |
| `src-tauri/src/llm_gateway.rs`、`llm_suggestions.rs`、`llm_commands.rs`、`llm_profiles.rs` | 云端网关与候选 |
| `src-tauri/src/schema/recognition_v1.rs` | **新增**：本契约的 Rust 类型（唯一真源） |
| `src-tauri/src/library/schema.rs` | 仅**追加式**迁移（`LIBRARY_V2_SCHEMA_VERSION = 4`） |
| `contracts/recognition-*.json` | **新增**：契约 JSON Schema |
| `Plan With Files/Dual_Recognition/**` | 本文件与后续交接记录 |

### 1.2 前端执行 agent 独占写入（本 agent 只读）

| 路径 | 说明 |
|---|---|
| `src/**`（全部 React UI 与渲染层） | 题库/工作区/编辑器/设置/导入向导/学生预览 |
| `src/api/tauriCommands.ts`、`workspaceClient.ts`、`publishClient.ts` | 前端 API 包装（本 agent 只给命令名与 payload 形状） |
| `src-tauri/src/nas_package_v2.rs`、`export_*.rs`、`reading_runtime_v2.rs` | 发布器与运行时编译 |
| 学生端仓库 | 不在本仓 |

### 1.3 共享文件（**追加式**修改协议）

只追加，不改既有函数签名与错误码；同一提交内不改对方刚加的内容。

| 路径 | 本 agent 的写入范围 | 另一 agent 的写入范围 |
|---|---|---|
| `src-tauri/src/lib.rs` | `mod reconcile;`、两个命令薄壳、`generate_handler!` 注册行 | 其余 |
| `src-tauri/src/library/repository.rs` | 只**调用** `apply_editor_commands_tx` / `get_canonical_ds` / `open_library_connection`，不改签名与错误码 | 其余 |
| `src-tauri/src/authoring_v2_commands.rs` | 只**调用** `apply_patch` / `refresh_quality_report` / `validate_authoring`，不修改 | 其余 |
| `src-tauri/src/processing/queue.rs` | 追加 `STAGE_RECONCILING` 常量与既有 SQL 的注释 | 其余 |

**已确认的禁区**：不解除 V1 接口对 V2 的保护（`LLM_SUGGESTION_AUTHORITATIVE_STORE_IS_V2`
必须保留）；V2 权威稿只写 `library_items_v2.canonical_ds_json`；自动与人工修正一律经
`apply_editor_commands_tx`，不直接 UPDATE canonical。

---

## 2. 概念模型

一次导入 = 一个 **job** = 一个 **批次（batch）**。

- **本地链**：`run_auto_pipeline_core(localOnly, editableDraft)` → `IeltsAuthoringIRV2` 草稿。
  **本地完成即发布可编辑状态**，不等待云端。
- **云端链**：`generate_cloud_reading_outline` → `CloudReadingOutlineV1` 原始 JSON →
  归一为 `RecognitionCandidateV1`（**不写权威稿**）。
- **原文件核验**：`verify_against_source`，以 `document-ir.json` 为证据面，**只产出问题与建议**。
- **统一裁决**：`reconcile_batch` 三路对齐 → **一份** `RecognitionDecisionV1`。

### 2.1 批次 id 与幂等

```text
batchId = "rec-<sanitized(jobId)>-v<baseEditVersion>-<sha256[:12] | "nosha">"
```

- `baseEditVersion` = **本地稿定稿时**的 canonical `current_edit_version`（冻结值）。
- 同一输入 + 同一版本重试 ⇒ 同一 `batchId` ⇒ 复用同一份本地识别快照与同一
  `decisionId`；自动应用用固定 `requestId = "recognition-auto:<batchId>"`，
  重放不重复写入权威稿。
- 用户改稿 ⇒ 版本变化 ⇒ 新批次，旧建议不会被拿来覆盖新稿。

---

## 3. 阶段与状态

### 3.1 `processing://item-updated` 事件（payload **未变**，仅文案更精确）

沿用既有字段：`libraryItemId`、`jobId`、`stage`、`localStatus`、`cloudStatus`、
`reconcileStatus`、`progressPercent`、`actionableCount`、`displayMessage`、`stateVersion`。

**阶段序列（实际实现）**：

```text
queued → running → local_recognition → cloud_recognition → ready_for_review
                                                  ↘ failed / cancelled
```

| stage | 何时进入 | 前端可做什么 |
|---|---|---|
| `local_recognition` | 本地识别中 | 只显示进度 |
| `cloud_recognition` + `cloudStatus=queued` | **本地已完成，云端排队中** | **草稿已可读写，可打开编辑** |
| `cloud_recognition` + `cloudStatus=running` | 云端识别中 | 可以打开编辑 |
| `ready_for_review` | 云端 + 核验 + 裁决已落库 | 读 §4 的统一建议 |

`displayMessage` 在 `localStatus=succeeded` 时明确写「本地识别完成，可以打开编辑 ·
云端识别排队中/中」，前端可直接显示。

> **前端不得**因为 `stage != ready_for_review` 就禁用编辑；`localStatus=succeeded`
> 即代表 `library_items_v2.canonical_ds_json` 已填充、`get_workspace_item` 返回非空 `ds`。

### 3.2 四阶段状态（**来自命令，不来自事件**）

`stage`/`cloudStatus` 是粗粒度进度。精细的四阶段状态（含原因码与文案）由
`get_recognition_decision` 返回的 `chains` 给出，避免在事件里重复维护状态：

```jsonc
"chains": {
  "local":        { "state": "succeeded" },
  "cloud":        { "state": "partial", "reasonCode": "SALVAGE_PARTIAL",
                    "message": "云端识别只有部分题组通过校验，其余需要人工确认。" },
  "source":       { "state": "not_run",  "reasonCode": "EVIDENCE_MISSING",
                    "message": "原文件没有可核验的文本证据，结论只能标记为无法判断。" },
  "adjudication": { "state": "succeeded" }
}
```

`state` 取值（`StageStateV1`）：`queued | running | succeeded | partial | unusable | not_run | failed | canceled`。
**`state=succeeded` 才代表该链路全量可用**；`partial` 表示部分可用，`unusable` 表示
完全不可用，`not_run` 表示没跑（未配置 / 不支持输入 / 被取消）。

---

## 4. 统一建议读取：`get_recognition_decision`

```ts
invoke("get_recognition_decision", { itemId: string }) => RecognitionDecisionViewV1
```

尚未产生批次时**不报错**，返回全 `not_run` 的空视图（`batchId=null`、`actionable=[]`）。

```jsonc
{
  "schemaVersion": "RecognitionDecisionViewV1",
  "itemId": "item-1",
  "jobId": "job-1",
  "batchId": "rec-job-1-v7-3f9a1c2b4d5e",
  "baseEditVersion": 7,          // 批次基线（生成建议时的版本）
  "editVersion": 9,              // 当前 canonical 版本
  "stale": true,                 // baseEditVersion < editVersion
  "generatedAt": "2026-09-15T20:00:00.123+00:00",
  "chains": { /* 见 §3.2 */ },
  "summary": { "agreed": 12, "autoFixed": 2, "needsReview": 1, "unverifiable": 3 },
  "actionable": [ /* DecisionItemV1[]：只含 needs_review + unverifiable 且 status=open */ ],
  "autoApplied": [ /* DecisionItemV1[]：resolution=auto_fixed，供解释与撤销 */ ]
}
```

**关键约定**

1. `actionable` **不含** `agreed`（三路一致，后台留记录）与 `info` 级项。
2. `autoApplied` 是**已完成的修正记录**，不是「问题」；前端应显示「已自动修正 N 项」并提供撤销。
3. `stale=true` 表示建议基于旧版本：接受时后端会逐项复核，被改过的项落 `superseded`。
   `stale` **不禁止**接受，只是提示。

### 4.1 `DecisionItemV1`（`actionable` / `autoApplied` 的元素）

```jsonc
{
  "decisionId": "d:slot:slot-14:answer",   // 稳定去重键 = d:<targetType>:<targetId>:<field>
  "resolution": "needs_review",            // agreed|auto_fixed|needs_review|unverifiable
  "code": "ANSWER_SOURCE_CONFLICT",        // 见 §4.2
  "severity": "blocker",                   // info|warning|blocker
  "title": "第 14 题答案与原文不一致",
  "userMessage": "第 14 题的本地与云端结果一致，但原文件中是「carving」，请确认采用哪一个。",
  "target": {
    "targetType": "slot",                  // document|task|response_group|slot|node|asset
    "targetId": "slot-14",
    "taskId": "task-1",                    // 可缺省
    "nodeId": null,                        // 可缺省
    "questionNumbers": [14]
  },
  "field": "answer",                       // 见 §4.3（**snake_case**）
  "evidence": [
    { "chain": "local",  "anchorKind": "slot_anchor" },
    { "chain": "cloud",  "anchorKind": "group_quote" },
    { "chain": "source", "anchorKind": "source_finding:Slot:slot-14:answer" }
  ],
  "localValue":  { "kind": "text", "values": ["stencilling"] },   // 可缺省
  "cloudValue":  { "kind": "text", "values": ["stencilling"] },   // 可缺省
  "sourceValue": { "kind": "text", "values": ["carving"] },       // 可缺省
  "proposedPatch": {                                              // 可缺省（unverifiable 一定缺省）
    "op": "setAnswer", "slotId": "slot-14",
    "value": { "kind": "text", "values": ["carving"] }
  },
  "undo": { "op": "setAnswer", "slotId": "slot-14",
            "value": { "kind": "unresolved" } },                  // 仅 autoApplied 项可能有
  "autoApplied": true,
  "appliedAt": "2026-09-15T20:00:00Z",
  "status": "open",                        // open|accepted|rejected|superseded|failed
  "reasonCode": "SUBSTANTIVE_DIVERGENCE",
  "dependencyGroup": "dep:response:rg-1"    // 可缺省；同组必须整组接受/拒绝
}
```

### 4.2 `code` 词表（`reconcile::rules` 实际产出，共 19 个）

| code | 触发 |
|---|---|
| `ANSWER_AGREED` | 三路一致且原文确认（→ `agreed`） |
| `ANSWER_AGREED_UNVERIFIED` | 本地=云端但缺原文证据（→ `unverifiable`） |
| `ANSWER_SOURCE_CONFLICT` | 与原文不一致（含「两路一致但与原文矛盾」）→ `needs_review` |
| `ANSWER_FILL_FROM_SOURCE` | 本地为空、原文可断言 → 低风险可自动补全 |
| `ANSWER_CONFLICT` | 本地与云端分歧（原文沉默） |
| `ANSWER_CONFLICT_UNVERIFIED` | 本地与云端分歧且无证据 |
| `ANSWER_SOURCE_CONFIRMED` | 原文确认了本地值（无云端对照时） |
| `ANSWER_EVIDENCE_MISSING` | 完全无证据面 |
| `ANSWER_MISSING_LOCAL` | 本地缺该题，云端有 |
| `CLOUD_SLOT_MISSING` | 云端缺该题，本地有 |
| `LOCAL_SLOT_MISSING` | 本地缺该题（云端给出） |
| `TASK_TYPE_CONFLICT` | 题组题型不一致 |
| `PROMPT_CONFLICT` | 题干不一致 |
| `OPTION_BANK_CONFLICT` | 选项库不一致 |
| `OPTION_BANK_MISSING_LOCAL` | 选项库缺失 |
| `SLOT_INTERACTION_ANSWER_MISMATCH` | 答案位交互与答案形状矛盾 |
| `ASSET_INTEGRITY_MISMATCH` | 资源 hash 不一致 |
| `SIGNIFICANT_SOURCE_TEXT_UNASSIGNED` | 原文内容未归入任何题目 |
| `ANSWER_FILL_FROM_SOURCE` 之外的补全路径 | 见上 |

`reasonCode` 取值见 §5.2。

### 4.3 `field` 取值（**snake_case**，注意与 v1 文档不同）

```text
answer | prompt | options | option_bank | slot_interaction
| group_kind | slot_placement | asset | source_coverage | slot_participation
```

### 4.4 四种 `resolution` 的前端义务

| resolution | 含义 | 前端义务 |
|---|---|---|
| `agreed` | 三路一致且原文确认 | **不产生逐项问题**（不出现在 `actionable`） |
| `auto_fixed` | 符合自动规则、已原子写入权威稿 | 出现在 `autoApplied`；显示「已自动修正」+ 撤销（用 `undo`） |
| `needs_review` | 实质分歧 | 生成**一条**建议卡；接受/拒绝 |
| `unverifiable` | 证据不足，**不得强行选版本** | 显示「无法验证」+ `reasonCode`；**不提供「接受」**，只提供「保持现状」 |

`unverifiable` 的项**保证** `proposedPatch` 缺省（后端契约强制，有测试覆盖）。

### 4.5 依赖组

`dependencyGroup` 相同的项必须**整组接受或整组拒绝**。后端已经保证：只要组内有任一项
不满足自动应用条件，整组都不会自动应用（`reasonCode=DEPENDENCY_BLOCKED`）。
前端在组内任一项被选中时应提示整组。

---

## 5. 决策提交：`apply_recognition_decisions`

```ts
invoke("apply_recognition_decisions", {
  requestId: string,          // 幂等键（uuid）；重试必须复用同一个
  batchId: string,            // 来自 view.batchId
  baseEditVersion: number,    // 用户看到的版本（= view.editVersion）
  accept: string[],           // decisionId 列表
  reject: string[]            // decisionId 列表
}) => ApplyRecognitionDecisionsResultV1
```

```jsonc
{
  "schemaVersion": "ApplyRecognitionDecisionsResultV1",
  "requestId": "8f2c…",
  "batchId": "rec-job-1-v7-3f9a1c2b4d5e",
  "editVersionBefore": 7,
  "editVersionAfter": 8,
  "replayed": false,          // true = 幂等重放，未重复写入
  "outcomes": [
    { "decisionId": "d:slot:slot-14:answer", "kind": "applied",
      "message": "已接受并写入题稿。", "appliedAt": "2026-09-15T20:00:00Z",
      "undo": { "op": "setAnswer", "slotId": "slot-14", "value": { "kind": "unresolved" } } },
    { "decisionId": "d:slot:slot-15:answer", "kind": "superseded",
      "reasonCode": "USER_EDITED", "message": "目标已被修改，建议过期；未覆盖你的改动。" },
    { "decisionId": "d:slot:slot-16:answer", "kind": "rejected", "message": "已拒绝该建议，权威稿未改动。" },
    { "decisionId": "d:slot:slot-17:answer", "kind": "failed",
      "reasonCode": "EDIT_VERSION_CONFLICT:…", "message": "写入失败，题稿未改动，请重试。" }
  ],
  "view": { /* 处理后的最新 RecognitionDecisionViewV1，无需二次读取 */ }
}
```

`kind` 取值（`DecisionOutcomeKindV1`）：`applied | rejected | superseded | failed`。
**四态必须分开呈现**：`superseded` 不是失败，是「你的修改赢了」。

**必须成立的行为（前端可依赖）**

1. 接受走**正式 V2 patch 路径**（`apply_editor_commands_tx`）：版本 CAS + `requestId`
   幂等 + journal 落库，单事务原子写入。
2. 相同 `requestId` 重复调用 → `replayed=true`，`editVersion` 不变，不重复写入。
   **同 id 不同请求体** → 报错 `RECOGNITION_REQUEST_ID_REUSED`（这是 bug，不是重试）。
3. 接受前**逐项复核**目标当前值：与建议生成时的 `localValue` 不一致 ⇒ 该条落
   `superseded`（`reasonCode=USER_EDITED`），**不覆盖**用户修改。
4. `resolution=unverifiable` 的项即使被 accept 也落 `failed`（`reasonCode=EVIDENCE_MISSING`），
   因为它本来就不该有 `proposedPatch`。
5. 拒绝不改权威稿，但决策持久化（重启后 `status=rejected`，不再出现在 `actionable`）。
6. 写入失败是**按条**上报的；同一批次内其余条不受影响。
7. 已处理（`status != open`）的项再次 accept → `superseded` + `RECOGNITION_ALREADY_RESOLVED`。

### 5.1 失败与错误码（命令层）

| 错误串 | 含义 |
|---|---|
| `RECOGNITION_BATCH_NOT_FOUND:<batchId>` | 批次不存在（可能被新批次替换） |
| `RECOGNITION_REQUEST_ID_REUSED` | 同一 `requestId` 换了请求体 |
| `RECOGNITION_DECISION_NOT_FOUND:<decisionId>` | 条目不存在（也会以 `failed` outcome 出现） |
| `recognition_invalid_input:<serde>` | payload 结构不合法 |
| `ITEM_DS_NOT_SEEDED:<itemId>` | 该条目还没有权威稿，无法裁决（先完成本地识别） |

### 5.2 `reason` 稳定原因码（`schema::recognition_v1::reason`）

```text
NO_PROFILE              未配置启用的模型 profile
CLOUD_DISABLED          未开启云端
MODEL_UNSUPPORTED_INPUT 模型/供应商不支持该输入形态
MODEL_TIMEOUT           超时
MODEL_INVALID_OUTPUT    输出无法通过校验
SALVAGE_PARTIAL         部分题组可用
ADJUDICATION_FAILED     裁决阶段失败
RULES_MATCH             规则命中（一致）
SUBSTANTIVE_DIVERGENCE  实质分歧
NO_SOURCE_EVIDENCE      缺原文证据
EVIDENCE_MISSING        完全无证据面
USER_EDITED             目标已被用户修改
AUTO_APPLY_RULE_REJECTED 低风险条件不成立
DEPENDENCY_BLOCKED      依赖组内有项不可自动应用
SOURCE_FILE_UNREADABLE  原文件无可读文本
```

云端失败分类：`classify_cloud_error` 把网关错误串映射到
`MODEL_UNSUPPORTED_INPUT`（→ `not_run`）/ `MODEL_TIMEOUT`（→ `unusable`）/ 其余
`MODEL_INVALID_OUTPUT`（→ `unusable`）。**未知错误保守归为非法输出**，绝不静默跳过验证。

---

## 6. 前端需要调用的清单（最小集合）

| 类型 | 名称 | 说明 |
|---|---|---|
| 事件 | `processing://item-updated` | 已存在；payload 未变，`displayMessage` 更精确 |
| 命令 | `get_recognition_decision` | **新增**，只读，`{ itemId }` |
| 命令 | `apply_recognition_decisions` | **新增**，写入，见 §5 |
| 命令 | `list_library_items` | 已存在；`processing` 子对象未变 |
| 命令 | `get_workspace_item` | 已存在；`localStatus=succeeded` 后 `ds` 非空即可编辑 |
| 命令 | `apply_editor_commands` | 已存在；撤销 `autoApplied` 用 `undo` 作为一条命令提交 |

### 6.1 典型前端流程

```text
1. 订阅 processing://item-updated，看到 localStatus=succeeded
   → 立即可打开编辑器（不要等 stage=ready_for_review）
2. 看到 stage=ready_for_review
   → invoke get_recognition_decision({ itemId })
3. actionable.length == 0 且 autoApplied.length == 0 → 无事可做，直接进入预览/发布
4. 有 actionable → 渲染建议卡；用户勾选后一次性提交
   → invoke apply_recognition_decisions({ requestId, batchId, baseEditVersion: view.editVersion, accept, reject })
5. 用返回的 result.view 覆盖本地视图（不要再打一次命令）
6. autoApplied 非空 → 显示「已自动修正 N 项」+ 撤销按钮（提交 undo 作为 editor 命令）
```

---

## 7. v1 → v2 的偏差与理由

| v1 设计 | v2 实际 | 理由 |
|---|---|---|
| 事件 payload 追加 `baseEditVersion`/`batchId`/`cloudReasonCode` | **未追加** | 这些值随批次变化，放进事件会造成第二份状态，容易不同步；改由 `get_recognition_decision` 单一来源提供。事件仍靠 `stateVersion` 触发前端刷新。 |
| `reconciling` 作为独立 stage | **未新增 stage**，云端阶段内完成 | 核验与裁决在同一次阻塞调用内完成，自造阶段只会增加无意义的中间态；四阶段状态改由 `chains` 精确表达。 |
| `cloudStatus=unavailable` | 实际用 `failed` | `processing_jobs_v2.cloud_status` 是既有列，值域由既有前端消费；精确语义放 `chains.cloud.state/reasonCode`。 |
| view 用 `items[]` | 拆成 `actionable[]` + `autoApplied[]` | 「已自动修正」不是待办，混在一个列表会让前端需要二次过滤，容易漏显示或误当问题。 |
| `field` 用 camelCase（`optionBank`） | 实际 **snake_case**（`option_bank`） | 与权威 schema `DecisionFieldV1` 的 `serde(rename_all="snake_case")` 一致，避免两套词表。 |
| apply 请求用 `itemId` + `decisions[{decisionId, action}]` | 请求用 `batchId` + `accept[]`/`reject[]` | 批次是幂等与版本的自然作用域；分成两个数组让「整批接受」变成一次调用，且不会出现同一条目既 accept 又 reject 的歧义。 |
| apply 结果用 `accepted/rejected/stale/failed` 四个数组 | 实际是 `outcomes[]`，每项带 `kind` | 单一有序列表保留了提交顺序，便于按条呈现结果与原因码。 |
| `dependencyGroup` 形如 `dep:q14-q15` | 实际 `dep:response:<responseGroupId>` | 依赖的真实来源是 response group，不是题号区间。 |

---

## 8. 验收对照（10 条通过标准 → 证据位置）

| # | 通过标准 | 证据 |
|---|---|---|
| 1 | 本地先完成，云端仍排队，草稿已可读写 | `scheduler::run_job_inner` 本地成功后先 `set_item_status_ready` + `advance(cloud_recognition, cloud=queued)`；`display_message_for` 给出「本地识别完成，可以打开编辑 · 云端识别排队中」 |
| 2 | 云端完整识别与本地稿一致，不产生多余待确认项 | `adjudicate::tests::agreement_with_source_evidence_produces_no_review_items` |
| 3 | 本地与云端有分歧，核验后只形成一份统一建议 | `adjudicate::tests::divergence_merges_into_a_single_review_item` + `store::tests::decision_id_is_unique_within_a_batch_and_latest_batch_wins` |
| 4 | 各路一致但缺原文证据，不被误判为已验证 | `adjudicate::tests::agreement_without_source_evidence_is_unverifiable_not_agreed` |
| 5 | 云端运行时用户修改，迟到结果不覆盖 | `rules::compare_slots` 的 `user_edited` 判定（canonical ≠ 批次快照）+ `commands::tests::auto_apply_recheck_rejects_a_slot_the_user_already_filled` + `store`/`commands` 的 `Superseded` 路径 |
| 6 | 自动修正符合规则且持久化；不符合转人工 | `adjudicate::tests::empty_answer_is_auto_filled_only_when_the_source_asserts_the_value` / `non_empty_answer_is_never_auto_overwritten` / `batch_that_breaks_structure_downgrades_the_whole_group` |
| 7 | 接受、拒绝及重启恢复正确，重复操作不重复写入 | `apply_recognition_decisions_core` 的 journal 重放；`queue::tests::durable_cancel_survives_restart` / `recover_on_startup_marks_retry_exhausted` |
| 8 | 批量任务中单个失败不阻塞其他文件，取消可持久化 | 调度循环逐任务 `spawn`；`fail_job` 只影响单任务；`queue::tests::advance_with_durable_cancel_marker_lands_cancelled` |
| 9 | PDF 与 DOCX 均走完整链路 | `scheduler` 的 `cloud_source_supported` 含 `.docx`；`generate_cloud_reading_outline` 对非 PDF 注入 `sourceText`，网关在无 PDF 附件时用原文文本作证据面 |
| 10 | 模型不支持输入、超时、非法输出及裁决失败均有明确状态 | `engine::tests::unsupported_input_is_not_run_but_timeout_is_unusable` / `missing_profile_is_reported_as_unsupported_not_silently_skipped`；`classify_cloud_error` |

**运行方式**

```bash
cd src-tauri
cargo test --lib reconcile::             # 识别闭环单测
cargo test --lib schema::recognition_v1:: # 契约类型与线上渲染护栏
cargo test --lib                         # 全量（当前 660 passed / 0 failed / 11 ignored）
cargo check                              # 类型与契约
```

契约一致性（三面交叉核对，见 §10）：

```bash
node scripts/recognition/contract-drift.mjs     # 或 npm run verify:recognition:contract
```

需要注意的**剩余缺口**（交付时必须声明，不能算已通过）：
- 上述是**服务与命令层**证据。真实 UI 集成、真实云服务往返、跨仓学生端计分
  尚未验证，需要两 agent 集成后按 AGENTS.md 的要求走真实产品链路。
- `document-ir.json` 在扫描件（无文本层）场景下 `source.status=not_run`，
  云端核验只能给出 `unverifiable`；这是设计上的诚实降级，不是 bug。

---

## 9. 未决/需与前端 agent 确认的点

1. `autoApplied` 的撤销入口放在建议列表还是编辑画布内联提示。
2. `unverifiable` 是否需要在题库列表行上给出「N 项无法验证」的徽标。
3. 是否需要 `get_workspace_item` 内联 `recognition` 摘要，避免前端多打一次命令
   （接口已可支持，改动需跨 §1.3 共享文件，需双方同意）。

---

## 10. 契约一致性护栏

契约有三个消费面，任何一面单独演进都会造成静默破坏。已建立两处护栏：

### 10.1 Rust 侧：`as_str` 必须等于 serde 渲染

`schema::recognition_v1::tests::as_str_matches_serde_for_every_wire_enum` 对
`DecisionFieldV1` / `DecisionTargetTypeV1` / `DecisionResolutionV1` / `DecisionStatusV1` /
`ChainStatusV1` / `StageStateV1` 的**全部**变体断言 `as_str() == serde 渲染`，并断言
`decisionId` 末段满足契约 schema 的 `^d:[a-z_]+:.+:[a-z_]+$`。

**为什么需要**：`as_str()` 是 `decisionId` 末段、原文件核验 `anchorKind` 末段、
`recognition_decisions_v1` 字段键三处字符串的唯一来源，但它与 serde 的 `rename_all`
是两份独立声明。**这曾真实漂移**：`DecisionFieldV1::as_str` 返回 camelCase（`optionBank`），
而线上 `field` 是 `option_bank`，导致 `d:slot:slot-14:optionBank` 被本契约 schema 判为非法。
受影响字段共 6 个（`group_kind`/`option_bank`/`slot_interaction`/`slot_placement`/
`source_coverage`/`slot_participation`），已修正。

### 10.2 三面交叉核对：`scripts/recognition/contract-drift.mjs`

```bash
node scripts/recognition/contract-drift.mjs   # 退出码 0 = 一致
```

比对 `src-tauri/src/schema/recognition_v1.rs`（真源）↔ `contracts/recognition-*.schema.json`
↔ `src/api/recognitionClient.ts`：

- **字段名**：Rust ↔ Schema 双向必须精确一致；
- **枚举值域**：Rust 变体 ↔ Schema `enum` 双向必须精确一致；
- **Rust ↔ 前端**：`✗ 消费方缺少/读不到`（前端读了后端不发的字段 → 静默 `undefined`）判为
  **破坏性**；`! 多余`（后端发了前端没消费）仅提示。

**为什么需要**：前端第一版按 v1 设计文档写成，读 `view.items` / `view.currentEditVersion`，
而 v2 是 `actionable`+`autoApplied` / `editVersion`。因为 serde 未设 `deny_unknown_fields`、
TS 类型在运行期被擦除，两边**都不报错**，结果是识别建议面板恒为「没有问题」（静默漏报）、
「采用修正」成为**静默空操作**。

**当前状态（2026-09-15）**：

| 面 | 状态 |
|---|---|
| 读取面 `get_recognition_decision` | ✅ 已对齐（`RecognitionDecisionRawV1` + `normalizeDecisionView` 双形状容错） |
| 写入面 `apply_recognition_decisions` 请求 | ❌ 仍发 `{itemId, decisions:[…]}`，后端要 `accept[]`/`reject[]` |
| 写入面返回 | ❌ 仍读 `accepted/rejected/stale/failed[]`，后端返回 `outcomes[]` + `view` |

读取面**有意**的 v1 契约名（`currentEditVersion` / `items` / `localStatus` / `cloudStatus` /
`cloudReasonCode`）登记在本脚本 `TS_MAP` 的 `allow` 列表里——它们是双形状回退的分支，不算漂移。
写入面**没有**归一化，故不加 `allow`，那正是仍需修复的部分。

详见 [HANDOFF_2026-09-15_frontend_contract_drift.md](HANDOFF_2026-09-15_frontend_contract_drift.md)
（含逐字段修复清单与 `queued`/`partial`/`unusable` 文案缺口）。

### 10.3 JSON Schema 的定位声明

`contracts/recognition-*.schema.json` **有意不列入** `contracts/contract-manifest.json`：
该 manifest 是跨仓（NAS peer）契约包，新增条目会触发对端 hash 校验失败。识别闭环契约目前
只在本仓消费。未列入并不降低其效力——它由 §10.2 的脚本持续校验，且脚本会直接读这两个文件。

