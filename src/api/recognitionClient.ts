import { command } from "./tauriCommands";

// 识别决策客户端（对齐《识别闭环：共享文件所有权 + 最小接口契约》§2.3 / §2.6）。
//
// 后端由识别/云端 agent 提供；本文件只做**类型化包装**，不解释业务规则。
// 契约要点（前端可依赖）：
//   - `agreed` 不产生逐项问题；`severity=info` 不进主问题列表；
//   - 同一 (targetType,targetId,field) 只有一条 decision；
//   - `dependencyGroup` 相同的项必须同时接受或同时拒绝；
//   - 接受走正式 V2 patch 事务（版本 CAS + requestId 幂等）；
//   - 过期（batch 基线早于用户修改）时整批进 `stale`，不覆盖用户修改。

export type RecognitionResolutionV1 = "agreed" | "auto_fixed" | "needs_review" | "unverifiable";
// `undone` 由后端 agent 随撤销协议一起加入（`DecisionStatusV1::Undone`）。
// 此前前端类型里没有它，撤销后的项会被当成未知状态，面板无法如实显示「已撤销」。
export type RecognitionDecisionStatusV1 =
  | "open"
  | "accepted"
  | "rejected"
  | "superseded"
  | "undone"
  | "failed";

export interface RecognitionEvidenceV1 {
  chain: "local" | "cloud" | "source";
  anchorKind: "page_quote" | "bbox" | "answer_key" | "asset";
  pageIndex?: number | null;
  quote?: string | null;
  anchor?: unknown;
}

export interface RecognitionDecisionTargetV1 {
  targetType: "document" | "task" | "response_group" | "slot" | "node" | "asset";
  targetId: string;
  taskId?: string | null;
  nodeId?: string | null;
  questionNumbers?: number[];
}

export interface RecognitionDecisionItemV1 {
  decisionId: string;
  resolution: RecognitionResolutionV1;
  code: string;
  severity: "blocker" | "warning" | "info";
  title: string;
  userMessage: string;
  target: RecognitionDecisionTargetV1;
  field: string;
  evidence: RecognitionEvidenceV1[];
  localValue?: unknown;
  cloudValue?: unknown;
  sourceValue?: unknown;
  proposedPatch?: unknown;
  autoApplied?: boolean;
  appliedAt?: string | null;
  undo?: unknown;
  status: RecognitionDecisionStatusV1;
  reasonCode?: string | null;
  dependencyGroup?: string | null;
}

export interface RecognitionDecisionSummaryV1 {
  agreed: number;
  autoFixed: number;
  needsReview: number;
  unverifiable: number;
}

/**
 * 云端自主修复的状态。取值与后端 `cloud_repair::REPAIR_STATUS_*` 一一对应
 * （`src-tauri/src/cloud_repair/mod.rs`）。
 *
 * `running` 目前只在批次行被中途更新时才会出现，先按契约保留：它代表「云端正在
 * 自动修复」，此时**不能**把中间差异计入用户待办——那些差异正是它正在处理的东西。
 */
export type CloudRepairStatusV1 =
  | "running"
  | "completed"
  | "needs_attention"
  | "cancelled"
  | "budget_exhausted"
  | "unavailable";

/** 修复后仍需要用户处理的单项（后端 `RepairRunReport::remaining_tasks`）。 */
export interface CloudRepairTaskV1 {
  userTaskId: string;
  targetIds?: string[];
  questionNumbers?: number[];
  message?: string | null;
  /**
   * 后端给出的**建议处理方式**（`fix_blocking_issue` / `review_difference` / `review_source`）。
   *
   * 它只用于决定按钮文案与「能不能定位过去」；能不能真正解决由用户在题面上做完决定。
   * 前端**不**据此自己算一遍剩余任务——那会与后端重算结果分叉。
   */
  action?: string;
  /** 后端内部的严重度标记（界面不据此显示门槛）。 */
  blocking?: boolean;
  /** 模型留下这条疑问时附的出处（可能与裁定一起给出）。 */
  evidence?: unknown;
}

/**
 * 云端自主修复摘要（后端 `recognition_batches_v1.repair_json`）。
 *
 * **`null` 表示「没有修复记录」，不是「已修复」**：旧批次、或本次无云导入都没有它。
 * 界面必须把它说成「未进行云端修复」，绝不能显示成完成。
 *
 * 「是否真的完成」也不由这里的 `status` 单独决定：`completed` 只说明修复循环自己认为
 * 收工了，最终判据始终是**当前稿重算出的剩余问题**（`items` / 剩余任务）。摘要的作用是
 * 让用户看见「跑到哪一步、还剩什么、能不能撤销」。
 */
export interface CloudRepairSummaryV1 {
  status: CloudRepairStatusV1 | string;
  /** 修复循环结束时的 canonical 编辑版本（诊断用，不参与完成判定）。 */
  editVersion?: number;
  /** 本次自动写入并被接受的修改数（0 表示没改到东西）。 */
  appliedCount?: number;
  /** 模型回合数。 */
  rounds?: number;
  /** 修复后仍需要用户处理的问题（后端重算，不是模型自报清单）。 */
  remainingTasks?: CloudRepairTaskV1[];
  /**
   * 本轮云端**替用户了结的争议条数**（含「当前稿对、候选错」与「原文件不足以定论」）。
   *
   * 这是「云端到底做了什么」的唯一可核对数字。没有它，用户只能看到「还剩几件事」，
   * 看不出云端是不是只是把差异原样丢回来给他。
   */
  adjudicatedCount?: number;
  finishNote?: string | null;
  lastError?: string | null;
  /** 是否存在可撤销的本轮自动修改（`appliedCount > 0`）。 */
  undoAvailable?: boolean;
  /** 撤销入口要用的 run 标识（`undoCloudRepair` 的原样入参）。 */
  repairRunId?: string | null;
  /** 摘要损坏时的降级标记（后端 `reasonCode=repair_json_corrupt`）。 */
  reasonCode?: string | null;
}

export interface RecognitionDecisionViewV1 {
  schemaVersion: string;
  itemId: string;
  batchId: string;
  baseEditVersion: number;
  currentEditVersion: number;
  stale: boolean;
  localStatus: string;
  cloudStatus: string;
  /**
   * 原文件核验（A3）与分歧裁决（A4）的**阶段状态**。
   *
   * 这两路此前被前端丢掉，于是「云端跑完了」就等于「核验完成」——而 A3/A4 之后
   * `chains.source` 会出现 `partial`（模型通道失败 / 预算耗尽），
   * `chains.adjudication` 会出现 `not_run`（有分歧但没有模型可用）与 `partial`
   * （部分分歧未获裁定）。丢掉它们就只能把「没核验」显示成「核验完成」。
   * 取值：`queued` / `running` / `succeeded` / `partial` / `unusable` / `not_run` / `failed` / `canceled`。
   */
  sourceStatus: string;
  adjudicationStatus: string;
  /**
   * 三路的原因码。**只作内部分类**（判断「未运行」是哪一种、日志与验收脚本比对），
   * 绝不作为用户文案——本仓库既有约定：reason code 不充当用户可见文本。
   *
   * A3/A4 新增：`ADJUDICATION_BUDGET_EXHAUSTED`、`ADJUDICATION_DECLINED`、
   * `ADJUDICATION_MODEL_UNAVAILABLE`、`ADJUDICATION_VALUE_NOT_CORROBORATED`、
   * `SOURCE_VERIFY_BUDGET_EXHAUSTED`。
   */
  cloudReasonCode?: string | null;
  sourceReasonCode?: string | null;
  adjudicationReasonCode?: string | null;
  summary: RecognitionDecisionSummaryV1;
  items: RecognitionDecisionItemV1[];
  /** 云端自主修复摘要。`null` = 没有修复记录（旧批次 / 无云导入），不是「已修复」。 */
  repair: CloudRepairSummaryV1 | null;
}

/**
 * 后端当前实际返回的**原始**形状（与契约 §2.3 有差异，见下）。
 *
 * 实测 `get_recognition_decision` 返回的是：
 *   `{ chains: { local|cloud|source|adjudication: { state } }, actionable, autoApplied,
 *      editVersion, jobId, baseEditVersion, batchId, stale, summary, generatedAt }`
 * 而契约 §2.3 写的是 `cloudStatus` / `localStatus` / `items`。
 *
 * 处理方式：**两种形状都认**，优先契约字段，缺了再回退到 `chains` / `actionable`。
 * 这样后端按契约补齐时前端无需改动，而现在也能正确显示，不会出现
 * 「云端核验状态未知（undefined）」这种假信息。
 */
export interface RecognitionDecisionRawV1 {
  schemaVersion?: string;
  itemId: string;
  batchId?: string | null;
  baseEditVersion?: number;
  currentEditVersion?: number;
  editVersion?: number;
  stale?: boolean;
  localStatus?: string;
  cloudStatus?: string;
  sourceStatus?: string;
  adjudicationStatus?: string;
  cloudReasonCode?: string | null;
  sourceReasonCode?: string | null;
  adjudicationReasonCode?: string | null;
  summary?: Partial<RecognitionDecisionSummaryV1>;
  items?: RecognitionDecisionItemV1[];
  /** 实现形状：四路链路状态（`StageStatusV1`：state + 可选 reasonCode/message/updatedAt）。 */
  chains?: Record<string, { state?: string; reasonCode?: string | null } | undefined>;
  /** 实现形状：需要用户处理的项 / 已自动应用的项。 */
  actionable?: RecognitionDecisionItemV1[];
  autoApplied?: RecognitionDecisionItemV1[];
  /**
   * 云端自主修复摘要。**旧批次没有这个字段**，缺失时归一为 `null`（= 没有修复记录），
   * 不能当成完成。
   */
  repair?: CloudRepairSummaryV1 | null;
}

const CHAIN_STATE_TO_STATUS: Record<string, string> = {
  // `StageStateV1` 的真实取值（`src-tauri/src/schema/recognition_v1.rs`）。
  // 漏掉 `queued` 最要紧：本地先出稿的流程里，本地识别完成的那一刻云端正是 `queued`
  // （已入队、还没开始）。若把它映射成 `not_started`，界面会说「云端没跑，只有本机结果」，
  // 而实际上云端正在排队并会跑——正好打掉「本地先出稿、云端仍排队」这个核心体验。
  queued: "queued",
  not_run: "not_started",
  not_started: "not_started",
  pending: "not_started",
  // 取消是**一次被中止的运行**，不是「没跑」：映射成 not_started 会让界面说「可以开始编辑」。
  canceled: "canceled",
  cancelled: "canceled",
  running: "running",
  in_progress: "running",
  done: "succeeded",
  ok: "succeeded",
  succeeded: "succeeded",
  success: "succeeded",
  partial: "partial",
  failed: "failed",
  error: "failed",
  unusable: "unavailable",
  unavailable: "unavailable",
  skipped: "skipped"
};

function chainStatus(chains: RecognitionDecisionRawV1["chains"], name: string): string {
  const state = chains?.[name]?.state;
  if (typeof state !== "string" || !state.trim()) return "not_started";
  return CHAIN_STATE_TO_STATUS[state] ?? state;
}

/** 某一路的稳定原因码（`StageStatusV1.reasonCode`）。只作内部分类，不做用户文案。 */
function chainReasonCode(chains: RecognitionDecisionRawV1["chains"], name: string): string | null {
  const code = chains?.[name]?.reasonCode;
  return typeof code === "string" && code.trim() ? code : null;
}

/** 把后端原始 payload 归一成契约形状。缺字段一律降级成「没跑」，绝不猜成「完成」。 */
export function normalizeDecisionView(raw: RecognitionDecisionRawV1): RecognitionDecisionViewV1 {
  const items = Array.isArray(raw.items)
    ? raw.items
    : [
        ...(Array.isArray(raw.autoApplied) ? raw.autoApplied.map((item) => ({ ...item, resolution: item.resolution ?? ("auto_fixed" as const) })) : []),
        ...(Array.isArray(raw.actionable) ? raw.actionable : [])
      ];
  return {
    schemaVersion: raw.schemaVersion ?? "RecognitionDecisionViewV1",
    itemId: raw.itemId,
    batchId: raw.batchId ?? "",
    baseEditVersion: raw.baseEditVersion ?? 0,
    currentEditVersion: raw.currentEditVersion ?? raw.editVersion ?? 0,
    stale: Boolean(raw.stale),
    localStatus: raw.localStatus ?? chainStatus(raw.chains, "local"),
    cloudStatus: raw.cloudStatus ?? chainStatus(raw.chains, "cloud"),
    // 四路链状态都必须带上来：`source`/`adjudication` 决定「核验到底做完了没有」，
    // 缺了它们就只能把「没核验」说成「核验完成」（见类型注释）。
    sourceStatus: raw.sourceStatus ?? chainStatus(raw.chains, "source"),
    adjudicationStatus: raw.adjudicationStatus ?? chainStatus(raw.chains, "adjudication"),
    cloudReasonCode: raw.cloudReasonCode ?? chainReasonCode(raw.chains, "cloud"),
    sourceReasonCode: raw.sourceReasonCode ?? chainReasonCode(raw.chains, "source"),
    adjudicationReasonCode: raw.adjudicationReasonCode ?? chainReasonCode(raw.chains, "adjudication"),
    summary: {
      agreed: raw.summary?.agreed ?? 0,
      autoFixed: raw.summary?.autoFixed ?? (Array.isArray(raw.autoApplied) ? raw.autoApplied.length : 0),
      needsReview: raw.summary?.needsReview ?? (Array.isArray(raw.actionable) ? raw.actionable.length : 0),
      unverifiable: raw.summary?.unverifiable ?? 0
    },
    items,
    // 缺失 / 非对象一律归一为 `null`（「没有修复记录」）。绝不在缺失时编一个
    // `{status:"completed"}`——那会把一次无云导入显示成「云端已修好」。
    repair: isRepairSummary(raw.repair) ? raw.repair : null
  };
}

function isRepairSummary(value: unknown): value is CloudRepairSummaryV1 {
  return typeof value === "object" && value !== null && typeof (value as { status?: unknown }).status === "string";
}

/**
 * 写入请求的 **wire 形状**（= 后端 `ApplyRecognitionDecisionsRequestV1`）。
 *
 * 名字由 `scripts/recognition/contract-drift.mjs` 的 `TS_MAP` 钉死，必须与 Rust 逐字段一致，
 * 否则契约检查会报破坏性漂移。组件不直接用这个类型，用下面的 `DecisionBatchRequestV1`。
 */
export interface ApplyRecognitionDecisionsInputV1 {
  requestId: string;
  batchId: string;
  baseEditVersion: number;
  accept: string[];
  reject: string[];
  /**
   * 撤销已自动修正的项：后端会**回滚权威稿**到修正前的值，并把决策状态持久化为
   * `undone`（`ApplyRecognitionDecisionsRequestV1::undo`，`#[serde(default)]`）。
   *
   * 与 `accept`/`reject` 互斥：同一项不能同时接受又撤销（后端整批拒绝）。
   * 这是「撤销」的正式通道——此前面板只发编辑器 setAnswer 补丁，
   * 权威稿虽被改回，**决策状态仍是 `accepted`/`auto_fixed`**，于是界面继续显示
   * 「已自动修正」、重开后又冒出来。那条路是假完成，已废弃。
   */
  undo: string[];
}

/** 写入返回的 **wire 形状**（= 后端 `ApplyRecognitionDecisionsResultV1`）。 */
export interface ApplyRecognitionDecisionsResultV1 {
  schemaVersion: string;
  requestId: string;
  batchId: string;
  editVersionBefore: number;
  editVersionAfter: number;
  replayed: boolean;
  outcomes: Array<{
    decisionId: string;
    kind: "applied" | "rejected" | "superseded" | "undone" | "failed";
    reasonCode?: string | null;
    message: string;
    appliedAt?: string | null;
    undo?: unknown;
  }>;
  view: RecognitionDecisionRawV1 | { get?: RecognitionDecisionRawV1 };
}

/** 组件侧请求：按「一次用户意图」提交一组决策，不需要自己拆 accept/reject/undo。 */
export interface DecisionBatchRequestV1 {
  itemId: string;
  batchId: string;
  baseEditVersion: number;
  requestId: string;
  decisions: Array<{ decisionId: string; action: "accept" | "reject" | "undo" }>;
}

/** 组件侧结果：把 wire 的 `outcomes[]` 归一成五类 id 列表。 */
export interface DecisionBatchOutcomeV1 {
  schemaVersion: string;
  itemId: string;
  batchId: string;
  editVersion: number;
  replayed: boolean;
  accepted: string[];
  rejected: string[];
  /** 已成功撤销并回滚权威稿的项（后端 `kind="undone"`）。 */
  undone: string[];
  stale: string[];
  failed: Array<{ decisionId: string; code: string; message: string }>;
  summary: { open: number; accepted: number; rejected: number; superseded: number; undone: number };
  /** 归一化后的最新视图，调用方可直接用它刷新，省一次往返（契约 §5）。 */
  view: RecognitionDecisionViewV1;
}

export async function getRecognitionDecision(itemId: string): Promise<RecognitionDecisionViewV1> {
  const raw = await command<RecognitionDecisionRawV1>("get_recognition_decision", { itemId });
  return normalizeDecisionView(raw);
}

/**
 * 修复状态行：**只有**用户需要知道的这几句。
 *
 * 三条纪律：
 *  1. `null`（没有修复记录）说成「未进行云端修复」，**绝不**说成完成——旧批次、无云导入
 *     都会走到这里。
 *  2. `completed` 只说「修复循环收工了」，不等于「整份题稿没问题」：`remainingTasks`
 *     非空时照样要把剩余条数说出来，否则用户会以为没别的事了。
 *  3. `budget_exhausted` / `unavailable` 是**降级**，不是失败：编辑照常，只是云端没帮上。
 *
 * 另外两条来自本轮任务书：
 *  - `running` 是**进行中**，此时 `remainingTasks` 必然为空，不能说「还剩 0 处」——
 *    那会被读成「没问题了」，而云端其实还在改；
 *  - 有裁定条数时要说出来。用户有权知道云端替他**了结**了多少争议，否则「云端自动修复」
 *    就只是一个说法：他看到的全是剩下的，看不到已经解决掉的。
 */
export function describeRepairStatus(repair: CloudRepairSummaryV1 | null | undefined): string {
  if (!repair) return "未进行云端修复";
  const remaining = repair.remainingTasks?.length ?? 0;
  const fixed = repair.appliedCount ?? 0;
  const adjudicated = repair.adjudicatedCount ?? 0;
  const settled = [
    fixed > 0 ? `已自动修正 ${fixed} 处` : "",
    adjudicated > 0 ? `已了结 ${adjudicated} 处差异` : ""
  ].filter(Boolean).join("、");
  switch (repair.status) {
    case "running":
      // 进行中：只说正在做、做了多少，**不报**剩余条数（此刻还没有可信的剩余清单）。
      return settled ? `云端正在自动修复…（${settled}）` : "云端正在自动修复…";
    case "completed":
      // 修复循环收工但仍有剩余：把剩余说出来，不能只说「已修复」。
      if (remaining > 0) return `${settled || "云端已自动修复"}，还有 ${remaining} 处待处理`;
      return settled || "云端已自动修复";
    case "needs_attention":
      return remaining > 0
        ? `${settled || "云端已自动修复"}，还有 ${remaining} 处需要你确认`
        : `${settled || "云端已自动修复"}，还有内容需要你确认`;
    case "budget_exhausted":
      return "云端修复达到本轮上限，剩余问题需要你处理";
    case "cancelled":
      return "云端修复已取消";
    case "unavailable":
      return "云端修复未能完成，不影响继续编辑";
    default:
      return "云端修复状态未知";
  }
}

/** 本轮自动修复是否可以撤销：只有真的写入过修改才有可撤销的东西。 */
export function canUndoRepair(repair: CloudRepairSummaryV1 | null | undefined): boolean {
  return Boolean(repair?.undoAvailable && repair.repairRunId);
}

/**
 * 撤销整轮云端自动修复。
 *
 * 走 Rust 批次撤销（后端 `cloud_repair::tools::undo_repair`），**不是**前端本地
 * undoStack：本地栈只覆盖用户自己的编辑，用它回滚云端写入会与批次 journal 错位。
 *
 * `repairRunId` 必须原样取自 `view.repair.repairRunId`——不要自己拼字符串。
 */
export async function undoCloudRepair(
  itemId: string,
  repairRunId: string,
  baseVersion: number
): Promise<unknown> {
  return command("undo_cloud_repair", { itemId, repairRunId, baseVersion });
}

/**
 * 提交决策。
 *
 * **写入面曾有破坏性契约漂移**：前端发 `{itemId, decisions[]}`，后端要
 * `{requestId, batchId, baseEditVersion, accept[], reject[]}`，且后端没开
 * `deny_unknown_fields` → `decisions` 被静默丢弃、`accept/reject` 取空数组，
 * 后端返回「成功但 outcomes 为空」，权威稿一字未改；前端读 `result.accepted.length`
 * 又抛 TypeError，最终显示「这次处理没有生效」。
 *
 * 现在在**这一层**做 wire 转换：对外保持组件友好的 `decisions[]`，
 * 对内发后端真正的字段，并把 `outcomes[]` 归一成 `accepted/rejected/stale/failed`。
 *
 * **还有第二层漂移（本轮 E2E 才发现）**：Tauri 命令签名是
 * `apply_recognition_decisions(input: Value, app: AppHandle)`，即 IPC 参数必须整体包在
 * `input` 键里。字段名对齐但没包 `input` 时，后端报
 * `invalid args 'input' for command 'apply_recognition_decisions': missing required key input`。
 * 契约漂移检查器只比对 Rust **结构体**的字段名，看不到**命令包装层**，所以它当时报
 * 「0 处破坏性不一致」而写路径其实是坏的 —— 这类漂移只能靠真实 IPC 调用发现。
 */
export async function applyRecognitionDecisions(
  input: DecisionBatchRequestV1
): Promise<DecisionBatchOutcomeV1> {
  const idsFor = (action: "accept" | "reject" | "undo") =>
    input.decisions.filter((decision) => decision.action === action).map((decision) => decision.decisionId);
  const accept = idsFor("accept");
  const reject = idsFor("reject");
  const undo = idsFor("undo");
  const wire = await command<ApplyRecognitionDecisionsResultV1>("apply_recognition_decisions", {
    // 必须包一层 `input`：命令签名是 `fn apply_recognition_decisions(input: Value, ...)`。
    input: {
      requestId: input.requestId,
      batchId: input.batchId,
      baseEditVersion: input.baseEditVersion,
      accept,
      reject,
      // 撤销走**同一个**命令：后端在同一个编辑事务里回滚权威稿并持久化 `undone` 状态。
      // 三个数组都发（即使为空）与 Rust 侧 `#[serde(default)]` 兼容，且让 journal 里的
      // 请求体完整反映用户意图，重放时不会因为缺字段而语义不同。
      undo
    }
  });
  const outcomes = Array.isArray(wire.outcomes) ? wire.outcomes : [];
  const idsOf = (kind: string) => outcomes.filter((outcome) => outcome.kind === kind).map((outcome) => outcome.decisionId);
  const failed = outcomes.filter((outcome) => outcome.kind === "failed");
  const rawView = (wire.view as { get?: RecognitionDecisionRawV1 })?.get ?? (wire.view as RecognitionDecisionRawV1);
  const view = normalizeDecisionView(rawView);
  return {
    schemaVersion: wire.schemaVersion ?? "ApplyRecognitionDecisionsResultV1",
    itemId: input.itemId,
    batchId: wire.batchId ?? input.batchId,
    // 后端不再返回 `editVersion`，必须取 `editVersionAfter`。
    editVersion: wire.editVersionAfter ?? wire.editVersionBefore ?? view.currentEditVersion,
    replayed: Boolean(wire.replayed),
    accepted: idsOf("applied"),
    rejected: idsOf("rejected"),
    undone: idsOf("undone"),
    // `superseded` 不是失败：它表示题稿已被用户改过、这条建议不再适用。
    stale: idsOf("superseded"),
    failed: failed.map((outcome) => ({
      decisionId: outcome.decisionId,
      code: outcome.reasonCode ?? "UNKNOWN",
      message: outcome.message
    })),
    summary: {
      open: view.items.filter((item) => item.status === "open").length,
      accepted: idsOf("applied").length,
      rejected: idsOf("rejected").length,
      superseded: idsOf("superseded").length,
      undone: idsOf("undone").length
    },
    view
  };
}

/**
 * 核验状态行：**只有**用户需要知道的这几句（本轮任务书第四节 + A3/A4 契约同步）。
 *
 * 三件事在这里定下来：
 *  1. **不显示内部状态表**。本地链/云端链/source 链/adjudication 链各自的 state、
 *     批次号、模型调用次数都不进普通界面；reason code 只用于内部分类。
 *  2. **「部分完成」有专门的说法**。`chains.source` / `chains.adjudication` 出现
 *     `partial` 时（模型通道失败、预算耗尽、部分分歧未获裁定）说明「部分内容尚未完成校验」，
 *     而不是笼统的「校验完成」。
 *  3. **空列表 ≠ 核验成功**（这是本轮最容易写错的一条）。只要还有一路没真正跑完，
 *     即使一条待处理项都没有，也**不**承诺「没有发现需要处理的问题」——
 *     「没有卡片」只说明没有需要用户动手的东西，不说明内容被核对过了。
 *
 * 状态取值（`CHAIN_STATE_TO_STATUS` 归一化之后）：
 * `queued` / `running` / `succeeded` / `partial` / `failed` / `unavailable` /
 * `not_started` / `skipped`。
 */
export interface VerificationStatusInputV1 {
  localStatus?: string;
  cloudStatus?: string;
  sourceStatus?: string;
  adjudicationStatus?: string;
  /** 待用户处理的条数（`needs_review` + `unverifiable` + `failed`）。 */
  pendingCount?: number;
  /** 云端链的原因码（只用于区分凭据错误，不进文案）。 */
  cloudReasonCode?: string | null;
  /**
   * 云端自主修复摘要。**有记录时优先**：修复循环才是新主链上云端真正做的事，
   * 批次行的链状态可能仍停在本地周期写下的值。
   */
  repair?: CloudRepairSummaryV1 | null;
}

/** 明确「只做了一部分」：模型通道失败、预算耗尽、部分分歧未获裁定。 */
const PARTIAL_STATES = new Set(["partial"]);
/** 云端没交出结果（失败 / 完全不可用）：如实说不可用，但**不**拖住编辑。 */
const CLOUD_UNAVAILABLE_STATES = new Set(["failed", "unavailable"]);
/** 这一路没有真正完成：不能据此承诺「未发现问题」。 */
const UNFINISHED_STATES = new Set(["partial", "failed", "unavailable", "not_started", "skipped"]);

export function describeVerificationStatus(input: VerificationStatusInputV1): string {
  const {
    localStatus,
    cloudStatus,
    sourceStatus,
    adjudicationStatus,
    pendingCount = 0,
    cloudReasonCode,
    repair
  } = input;
  // 本机还在读：唯一「什么都还不能做」的状态，也是用户最先看到的。
  if (localStatus === "queued" || localStatus === "running") return "正在本机识别…";
  // 有修复记录就以它为准：批次行 cloud 阶段可能仍是本地周期写下的 not_run，
  // 拿它说「可以开始编辑」会与「已自动修正 N 处」自相矛盾。
  if (repair) return describeRepairStatus(repair);
  if (cloudStatus === "queued" || cloudStatus === "running") return "云端正在校验…";
  if (cloudStatus === "canceled") return "云端检查已取消，当前是本机识别的题稿";
  // 云端根本没跑（未配置模型 / 未启用）：这是「可以开始编辑」，不是「校验通过」。
  if (!cloudStatus || cloudStatus === "not_started") return "题稿已生成，可以开始编辑";
  if (CLOUD_UNAVAILABLE_STATES.has(cloudStatus)) {
    // 凭据错误重试没用，必须去设置页修正——不能说「暂时」。
    if (cloudReasonCode === "MODEL_CREDENTIALS_INVALID") return "云端密钥无效，请到设置里重新填写；题稿仍可编辑";
    return "云端校验暂时不可用，不影响继续编辑";
  }

  const verification = [cloudStatus, sourceStatus ?? "not_started", adjudicationStatus ?? "not_started"];
  const partial = verification.some((state) => PARTIAL_STATES.has(state));
  const unfinished = verification.some((state) => UNFINISHED_STATES.has(state));

  if (pendingCount > 0) {
    // 「部分完成」时先说清楚「有的内容还没校验完」，再让用户去看标出的题目。
    if (partial) return "部分内容尚未完成校验，请检查标出的题目";
    return `云端发现 ${pendingCount} 处建议`;
  }
  // 一条待处理项都没有：**只有**三路都确实跑完，才敢说「没有发现问题」。
  if (unfinished) return "部分内容尚未完成校验";
  return "云端校验完成，没有发现需要处理的问题";
}
