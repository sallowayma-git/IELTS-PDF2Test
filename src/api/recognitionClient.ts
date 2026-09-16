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
export type RecognitionDecisionStatusV1 = "open" | "accepted" | "rejected" | "superseded" | "failed";

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

export interface RecognitionDecisionViewV1 {
  schemaVersion: string;
  itemId: string;
  batchId: string;
  baseEditVersion: number;
  currentEditVersion: number;
  stale: boolean;
  localStatus: string;
  cloudStatus: string;
  cloudReasonCode?: string | null;
  summary: RecognitionDecisionSummaryV1;
  items: RecognitionDecisionItemV1[];
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
  cloudReasonCode?: string | null;
  summary?: Partial<RecognitionDecisionSummaryV1>;
  items?: RecognitionDecisionItemV1[];
  /** 实现形状：四路链路状态。 */
  chains?: Record<string, { state?: string } | undefined>;
  /** 实现形状：需要用户处理的项 / 已自动应用的项。 */
  actionable?: RecognitionDecisionItemV1[];
  autoApplied?: RecognitionDecisionItemV1[];
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
  canceled: "not_started",
  cancelled: "not_started",
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
    cloudReasonCode: raw.cloudReasonCode ?? null,
    summary: {
      agreed: raw.summary?.agreed ?? 0,
      autoFixed: raw.summary?.autoFixed ?? (Array.isArray(raw.autoApplied) ? raw.autoApplied.length : 0),
      needsReview: raw.summary?.needsReview ?? (Array.isArray(raw.actionable) ? raw.actionable.length : 0),
      unverifiable: raw.summary?.unverifiable ?? 0
    },
    items
  };
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
    kind: "applied" | "rejected" | "superseded" | "failed";
    reasonCode?: string | null;
    message: string;
    appliedAt?: string | null;
    undo?: unknown;
  }>;
  view: RecognitionDecisionRawV1 | { get?: RecognitionDecisionRawV1 };
}

/** 组件侧请求：按「一次用户意图」提交一组决策，不需要自己拆 accept/reject。 */
export interface DecisionBatchRequestV1 {
  itemId: string;
  batchId: string;
  baseEditVersion: number;
  requestId: string;
  decisions: Array<{ decisionId: string; action: "accept" | "reject" }>;
}

/** 组件侧结果：把 wire 的 `outcomes[]` 归一成四类 id 列表。 */
export interface DecisionBatchOutcomeV1 {
  schemaVersion: string;
  itemId: string;
  batchId: string;
  editVersion: number;
  replayed: boolean;
  accepted: string[];
  rejected: string[];
  stale: string[];
  failed: Array<{ decisionId: string; code: string; message: string }>;
  summary: { open: number; accepted: number; rejected: number; superseded: number };
  /** 归一化后的最新视图，调用方可直接用它刷新，省一次往返（契约 §5）。 */
  view: RecognitionDecisionViewV1;
}

export async function getRecognitionDecision(itemId: string): Promise<RecognitionDecisionViewV1> {
  const raw = await command<RecognitionDecisionRawV1>("get_recognition_decision", { itemId });
  return normalizeDecisionView(raw);
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
  const accept = input.decisions.filter((decision) => decision.action === "accept").map((decision) => decision.decisionId);
  const reject = input.decisions.filter((decision) => decision.action === "reject").map((decision) => decision.decisionId);
  const wire = await command<ApplyRecognitionDecisionsResultV1>("apply_recognition_decisions", {
    // 必须包一层 `input`：命令签名是 `fn apply_recognition_decisions(input: Value, ...)`。
    input: {
      requestId: input.requestId,
      batchId: input.batchId,
      baseEditVersion: input.baseEditVersion,
      accept,
      reject
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
      superseded: idsOf("superseded").length
    },
    view
  };
}

/** 云端不可用原因码 → 用户能看懂的一句话（契约 §2.5 的稳定码表）。 */
const CLOUD_REASON_TEXT: Record<string, string> = {
  NO_PROFILE: "还没有配置可用的模型，云端核验没有运行。",
  CLOUD_DISABLED: "云端核验没有开启。",
  MODEL_UNSUPPORTED_INPUT: "当前模型不支持这份文件的输入形式，云端核验没有结果。",
  MODEL_TIMEOUT: "云端核验超时了，本地结果不受影响。",
  MODEL_INVALID_OUTPUT: "云端返回的内容不符合要求，已忽略，本地结果不受影响。",
  SALVAGE_PARTIAL: "云端只核验了部分内容。",
  ADJUDICATION_FAILED: "云端结果与本地结果的核对没有完成，本地结果不受影响。"
};

export function describeCloudReason(reasonCode?: string | null): string | undefined {
  if (!reasonCode) return undefined;
  return CLOUD_REASON_TEXT[reasonCode] ?? `云端核验未完成（${reasonCode}）。`;
}

/**
 * 云端状态行：必须区分「没跑 / 在跑 / 完成 / 失败 / 不可用」，
 * 不允许把「没结果」显示成「没有问题」，也不允许把未知状态渲染成 `undefined`。
 */
export function describeCloudStatus(cloudStatus: string | undefined, reasonCode?: string | null): string {
  switch (cloudStatus) {
    case "not_started":
      return "云端核验还没有运行，下面显示的都是本机识别结果。";
    case "queued":
      return "云端识别排队中，本地结果已经可以编辑。";
    case "running":
      return "云端核验进行中，本地结果已经可以编辑。";
    case "succeeded":
      return reasonCode === "SALVAGE_PARTIAL" ? "云端核验完成（部分内容）。" : "云端核验完成。";
    case "partial":
      return "云端只核验了部分内容，其余需要人工确认。";
    case "failed":
      return "云端核验失败，本地结果不受影响。";
    case "unavailable":
      return describeCloudReason(reasonCode) ?? "云端核验不可用。";
    case "skipped":
      return "这次没有运行云端核验。";
    default:
      return cloudStatus ? `云端核验状态：${cloudStatus}。` : "云端核验状态未知。";
  }
}
