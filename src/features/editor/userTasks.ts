import type { IeltsAuthoringIRV2, ResponseGroupV2 } from "../../types";
import { issueRootCause, type ActionableIssueV1, type IssueSeverity } from "./actionableIssues";
import { formatDecisionValue } from "./recognitionDecisions";

// 把「原始问题行」收敛成**用户可以完成的任务**。
//
// 背景（本轮任务书）：普通界面此前直接把内部问题行铺开——同一道题因为本地闭包与发布门禁
// 各写一行而重复；「连续 3 道题缺答案」铺成 3 行；题干/stimulus/source boundary/slot host
// 这类**同一题组内部**的毛病各占一行；泛化的 `QUALITY_NOT_READY`（「还有未确认的内容」）
// 与 `RUNTIME_COMPILER_FAILED` 又在具体问题旁边再占两行。用户看到的是一屏内部术语，
// 而不是「我还有几件事要做」。
//
// 本模块的产出是**唯一**进入普通界面的问题形状：一条任务 = 一句用户话 + 至少一个真能
// 解决问题的按钮。原始问题行只作为任务的「覆盖证据」（`covers`），不再直接渲染。
//
// 三条硬约束（都来自任务书，且互相牵制，改动时别只顾一条）：
//   1. 同一问题不重复显示 —— 所以按「题号区间」「题组」聚合，而不是逐行渲染；
//   2. 不同问题不被隐藏 —— 所以**未知码一律降级成「处理失败」而不是丢弃**，
//      且泛化行只在**确实有具体任务**时才隐藏（没有具体任务时它就是唯一的线索）；
//   3. 按钮必须真的解决问题 —— 所以每个动作都绑到工作区里真实存在的操作
//      （定位答案控件 / 打开原文件）。工作区可另行记录已查看状态；确认提示不代表修复内容或通过发布检查。

export type UserTaskKind =
  /** 缺答案：去填写，定位到答案控件。 */
  | "missing-answer"
  /** 答案和题目形式对不上：去填写，定位到答案控件。 */
  | "answer-mismatch"
  /** 题组/整篇没有识别完整：查看原文 + 重新识别。 */
  | "incomplete-recognition"
  /** 结构不完整（编译器说不出更具体的原因时）：重新识别。 */
  | "structure-incomplete"
  /** 图片/资源缺失：重新识别。 */
  | "missing-asset"
  /** 其余处理失败：对照原文件核对。 */
  | "processing-failed"
  /** 当前稿与云端读到的不一样（云端没能定论，交给用户看一眼）。 */
  | "cloud-difference"
  /** 云端留下的疑问 / 原文件有云端没读全的部分。 */
  | "cloud-note"
  /**
   * 空位是按阅读顺序**推断**出来的（原卷没在这几行印题号）。
   *
   * 这是识别自己记录的推断来源（草稿的 `recognitionWarnings`），不是缺内容：
   * 位置可能对，也可能错，所以只提示、不阻断发布。
   */
  | "inferred-slot";

export type UserTaskActionId =
  /** 定位到答案控件（题面上的输入框）。 */
  | "fill-answer"
  /** 打开原文件并定位到题组。 */
  | "view-source";

export interface UserTaskActionV1 {
  id: UserTaskActionId;
  label: string;
  /** 点击后要定位到的 DS 节点/实体 id。 */
  targetId: string;
}

export interface UserTaskV1 {
  taskId: string;
  kind: UserTaskKind;
  severity: IssueSeverity;
  /** 一句话，用户照着做就行。例如「第 11–13 题缺少答案」。 */
  title: string;
  /** 需要时补一句怎么做。 */
  detail?: string;
  /** 云端差异两侧的原文；工作区用 del/ins 标记直接并列呈现。 */
  comparison?: {
    current: string;
    cloud: string;
    cloudLabel: string;
  };
  actions: UserTaskActionV1[];
  /** 这条任务覆盖的原始问题行 id（诊断与验收证据用，**不进**普通界面）。 */
  covers: string[];
}

export interface UserTaskSummaryV1 {
  /**
   * 题稿是否已经读进来。
   *
   * 为 `false` 时 `tasks` 必然为空，而且这个「空」**不代表没有问题**：`slotIdsOfTarget` 要靠
   * `answerSlots` 才能把一条问题落到具体题位上，草稿没读进来时它一律返回空，带题号的缺答问题
   * 会退化成 `missing-answer:unnumbered`——任务卡看着正常，点「去填写」却定位不到任何元素
   * （题面此刻也还没渲染出那道题）。所以这里如实返回「还没准备好」，由界面显示加载中，
   * 而不是先给用户一批点不动的按钮（F-R15-5）。
   */
  ready: boolean;
  tasks: UserTaskV1[];
  /** 顶部那一行：还有 N 处可以补充 / 没有需要补充的内容。**不是**能不能发布的结论。 */
  headline: string;
  /** 内部计数（不进普通界面：界面不再展示阻断 / 门槛）。 */
  blockerCount: number;
  /** 被合并掉的原始问题行数（验收报告用，普通界面不显示）。 */
  mergedRowCount: number;
}

/**
 * 草稿里记录「空位按阅读顺序推断」的 `recognitionWarnings` 前缀。
 *
 * 后端在 `completion.rs::assign_drawn_blank_slots` 里，把原卷**没有印题号**的
 * 空位按阅读顺序派给剩余题号时会写下 `slot_order_inferred:<题号…>`。
 * 前缀必须与后端常量 `SLOT_ORDER_INFERRED_WARNING` 逐字一致。
 */
const SLOT_ORDER_INFERRED_WARNING = "slot_order_inferred:";

// ── 质量码分类 ────────────────────────────────────────────────────────
//
// 分类**只依赖根因码**（`issueRootCause`），不依赖后端文案：后端文案里有
// 「completion slot」「table stimulus」「ReadingExamSourceV2 runtime compiler」
// 这类内部对象名，必须由前端改写，绝不能原样透传（本轮任务书第二节第 7 条）。

/** 缺答案。 */
const MISSING_ANSWER = new Set([
  "ANSWER_MISSING",
  "ANSWER_UNRESOLVED",
  "ANSWER_EMPTY",
  "ANSWER_MISSING_SLOT",
  "ANSWER_KEY_MISSING",
  "ANSWER_KEY_MISSING_SLOT",
  "ANSWER_KEY_ABSENT",
  // 答案键里多出一个不属于任何题位的槽位：对用户同样是「答案和题目对不上」，要去改答案。
  "ANSWER_KEY_ORPHAN_SLOT"
]);

/** 答案与题目形式不匹配（学生端提交会判无效）。 */
const ANSWER_MISMATCH = new Set([
  "RUNTIME_TEXT_SLOT_ANSWER_NOT_TEXT",
  "RUNTIME_CHOICE_SLOT_ANSWER_NOT_OPTION",
  "RUNTIME_HOTSPOT_ANSWER_NOT_OPTION",
  "RUNTIME_ANSWER_KEY_POLICY_INVALID",
  // 答案本身合法、但不符合这道题的要求（字数超限 / 选项不在选项库里 / 数量对不上）：
  // 用户要改的是**答案内容**，不是「重新识别」。
  "ANSWER_WORD_LIMIT_VIOLATION",
  "ANSWER_OPTION_NOT_IN_BANK",
  "CARDINALITY_SLOT_MISMATCH"
]);

/**
 * 题组/整篇没有识别完整：题干、选项、stimulus、source boundary、slot host 都算这一类。
 *
 * 码表取自 `src-tauri/src/ielts_grammar/issue_codes.rs`（全仓质量码的唯一定义处）。
 * **未知码不丢弃**（见 `classify` 的兜底）：宁可多给一条「处理失败」也不藏起一条阻断。
 */
const INCOMPLETE_RECOGNITION = new Set([
  "PROMPT_EMPTY",
  "QUESTION_PROMPT_MISSING",
  "PROMPT_BOUNDARY_AMBIGUOUS",
  "OPTION_TEXT_MISSING",
  "OPTION_RUN_INCOMPLETE",
  "OPTION_LABEL_MISSING",
  "OPTION_ALPHABET_MISMATCH",
  "OPTION_BANK_MISSING",
  "OPTION_BANK_SCOPE_AMBIGUOUS",
  "OPTION_BANK_REFERENCE_MISSING",
  "SHARED_OPTION_BANK_MISSING",
  "OPTION_BANK_DECLARATION_MISMATCH",
  "OPTION_LABELS_DUPLICATE",
  "SLOT_HOST_MISSING",
  "SLOT_HOST_DUPLICATE",
  "SLOT_OUTSIDE_FIGURE",
  "HOTSPOT_GEOMETRY_INVALID",
  "SLOT_ID_MISMATCH",
  "SLOT_REFERENCE_MISSING",
  "SLOT_GROUP_ASSIGNMENT_INVALID",
  "STIMULUS_MISSING",
  "QUESTION_STRUCTURE_NOT_DETECTED",
  "QUESTION_NUMBER_MISSING",
  "QUESTION_RANGE_UNPARSED",
  "QUESTION_NUMBER_DUPLICATE",
  "SIGNIFICANT_REGION_UNASSIGNED",
  "SOURCE_BOUNDARY_UNASSIGNED",
  "GROUP_VERIFICATION_REQUIRED",
  "TABLE_TOPOLOGY_LOW_CONFIDENCE_NO_VISUAL_FALLBACK",
  "TABLE_ROW_MAPPING_INFERRED",
  "PASSAGE_ONLY_SOURCE",
  "PASSAGE_CONTENT_MISSING",
  "QUESTION_SHEET_MISSING",
  "PROVENANCE_MISSING",
  "RUNTIME_SLOT_UNASSIGNED",
  "RUNTIME_SLOT_ASSIGNED_TWICE",
  "RUNTIME_DISPLAY_MAP_MISMATCH",
  "RUNTIME_QUESTION_ORDER_INVALID",
  "RUNTIME_TASK_ID_DUPLICATE",
  "TASK_ID_DUPLICATE",
  "RESPONSE_GROUP_ID_DUPLICATE",
  "TASK_TYPE_SIGNATURE_MISMATCH",
  "TASK_TYPE_CONFLICT",
  "SOURCE_OWNERSHIP_CONFLICT",
  "RESPONSE_GROUP_POLICY_MISMATCH",
  // 指令里「ONE WORD ONLY」这类字数限制没解析出来：属于题面没读全，要对着原文核对。
  "WORD_LIMIT_UNPARSED",
  "INSTRUCTION_SIGNATURE_UNRESOLVED",
  "INSTRUCTION_PROVENANCE_MISSING",
  "INSTRUCTION_SIGNATURE_EVIDENCE_MISSING",
  // 图里有题目区域但没 OCR 到内容：同样是「这组题没读全」。
  "DIAGRAM_QUESTION_REGION_OCR_REQUIRED"
]);

/** 资源缺失。 */
const MISSING_ASSET = new Set([
  "ASSET_MISSING",
  "ASSET_REFERENCE_MISSING",
  "ASSET_PATH_UNSAFE",
  "ASSET_HASH_MISMATCH",
  "ASSET_ID_DUPLICATE",
  "IMAGE_MISSING"
]);

/**
 * 结构不完整：**只有说不出更具体的原因时**才显示（任务书第二节第 5 条）。
 *
 * `RUNTIME_COMPILER_FAILED` 是编译器的兜底结论——它本身不告诉用户哪里有问题，
 * 而具体原因（缺答案、题组没识别完整）通常也在同一批里。两者并存时只显示具体的那些。
 */
const STRUCTURE_INCOMPLETE = new Set([
  "RUNTIME_COMPILER_FAILED",
  "V1_COMPATIBILITY_COMPILER_FAILED",
  "RUNTIME_SCHEMA_UNSUPPORTED",
  "AUTHORING_SCHEMA_INVALID",
  "PHYSICAL_SHADOW_MISSING",
  "EXAM_ID_INVALID",
  "SCORING_POLICY_UNRESOLVED",
  "EXAMPLE_SCORING_CONFLICT",
  "QUALITY_GATE_EVALUATION_FAILED",
  "QUALITY_REPORT_MISSING"
]);

/** 泛化汇总行：本身不指向任何具体目标，具体任务存在时一律隐藏。 */
const GENERIC_ONLY = new Set(["QUALITY_NOT_READY", "QUALITY_HARD_FAILURE"]);

/** 列表被截断的提示：这是给内部看的，普通界面不显示。 */
const TRUNCATION_CODES = new Set(["BLOCKER_LIST_TRUNCATED"]);

type Classification = UserTaskKind | "generic" | "truncated";

function classify(issue: ActionableIssueV1): Classification {
  if (TRUNCATION_CODES.has(issue.code) || TRUNCATION_CODES.has(issueRootCause(issue))) return "truncated";
  const root = issueRootCause(issue);
  if (GENERIC_ONLY.has(root)) return "generic";
  if (MISSING_ANSWER.has(root)) return "missing-answer";
  if (ANSWER_MISMATCH.has(root)) return "answer-mismatch";
  if (INCOMPLETE_RECOGNITION.has(root)) return "incomplete-recognition";
  if (MISSING_ASSET.has(root)) return "missing-asset";
  if (STRUCTURE_INCOMPLETE.has(root)) return "structure-incomplete";
  // 未知码**不丢弃**：丢弃等于把一条阻断问题藏起来（「不同问题不被隐藏」）。
  // 归到「处理失败 → 重试处理」，用户至少有一个真能按的按钮，且不会误以为已经没问题。
  return "processing-failed";
}

// ── 目标 → 题号 ───────────────────────────────────────────────────────

function slotIdsOfTarget(ds: IeltsAuthoringIRV2 | undefined, targetId: string): string[] {
  if (!ds || !targetId) return [];
  if (ds.answerSlots[targetId]) return [targetId];
  for (const task of ds.taskGroups) {
    if (task.taskId === targetId) {
      return task.responseGroups.flatMap((group) => group.slotIds);
    }
    for (const group of task.responseGroups) {
      if (group.responseGroupId === targetId) return [...group.slotIds];
    }
  }
  return [];
}

function numberOf(ds: IeltsAuthoringIRV2 | undefined, slotId: string): number | undefined {
  const slot = ds?.answerSlots[slotId];
  if (!slot) return undefined;
  const value = Number(slot.questionNumber);
  return Number.isFinite(value) ? value : undefined;
}

function answerMismatchDetail(ds: IeltsAuthoringIRV2, slotIds: readonly string[]): string {
  const interactionValues: string[] = slotIds.flatMap((id) => {
    const interaction = ds.answerSlots[id]?.interaction;
    return interaction ? [interaction] : [];
  });
  const interactions = new Set<string>(interactionValues);
  if (interactions.size === 1 && interactions.has("text")) {
    return "这些题需要文字答案；请按题面给出的字数和答案格式填写。";
  }
  const labels = [...new Set(slotIds.flatMap((slotId) => {
    const group = groupForTarget(ds, slotId);
    return group?.optionBank?.options.map((option) => option.label.trim()).filter(Boolean) ?? [];
  }))];
  const choiceInteractions = ["radio", "checkbox", "select", "dragdrop", "hotspot"];
  if (interactions.size === 1 && choiceInteractions.some((value) => interactions.has(value))) {
    return labels.length
      ? "这些题需要选项答案形式，请从题面允许的标签中选择：" + labels.join("、") + "。"
      : "这些题需要选项答案形式，请从题面列出的选项中选择。";
  }
  return "请按每道题的作答控件填写对应形式：文字题填写文字，选择题填写选项。";
}

/** 题号区间的用户话术：「第 11 题」「第 11–13 题」。 */
function questionRangeLabel(numbers: readonly number[]): string {
  const runs = contiguousRuns(numbers);
  return runs
    .map((run) => run.length === 1
      ? `第 ${run[0]} 题`
      : `第 ${run[0]}–${run[run.length - 1]} 题`)
    .join("、");
}

/** 一串题号里**连续**的段落（1,2,3,7 → [[1,2,3],[7]]）。 */
function contiguousRuns(numbers: readonly number[]): number[][] {
  const sorted = [...new Set(numbers)].sort((a, b) => a - b);
  const runs: number[][] = [];
  for (const value of sorted) {
    const last = runs[runs.length - 1];
    if (last && value === last[last.length - 1] + 1) last.push(value);
    else runs.push([value]);
  }
  return runs;
}

// ── 任务构造 ─────────────────────────────────────────────────────────

interface Bucket {
  issues: ActionableIssueV1[];
}

/**
 * 把一批原始问题行收敛成任务。
 *
 * @param ds 当前草稿（用来把「目标 id」翻成题号；缺失时退化成不带题号的文案）
 * @param issues 合并后的原始问题行（`mergePublishGateIssues` 的产物）
 */
export function buildUserTasks(
  ds: IeltsAuthoringIRV2 | undefined,
  issues: readonly ActionableIssueV1[]
): UserTaskSummaryV1 {
  // 草稿还没读进来：如实说「还没准备好」，不生成任务。
  // 这里**不能**退化成「没问题」——那会让界面在题稿打开之前就宣称「可以导出」，
  // 也会让「去填写」指向题面上还不存在的元素（F-R15-5）。
  if (!ds) {
    return { ready: false, tasks: [], headline: "正在打开这道题…", blockerCount: 0, mergedRowCount: 0 };
  }
  const buckets = new Map<Classification, Bucket>();
  for (const issue of issues) {
    const kind = classify(issue);
    const bucket = buckets.get(kind) ?? { issues: [] };
    bucket.issues.push(issue);
    buckets.set(kind, bucket);
  }

  const tasks: UserTaskV1[] = [];
  const take = (kind: Classification): ActionableIssueV1[] => buckets.get(kind)?.issues ?? [];
  const drop = (kind: Classification): number => take(kind).length;

  // 1) 缺答案：把**连续**题号并成一条区间任务（任务书第二节第 1 条）。
  //    同一道题被本地闭包与发布门禁各写一行时，这里自然只出一条（同一个 slot 只落一次）。
  const byNumber = new Map<number, { slotId: string; issues: ActionableIssueV1[] }>();
  const unnumberedAnswerIssues: ActionableIssueV1[] = [];
  for (const issue of take("missing-answer")) {
    const slotId = slotIdsOfTarget(ds, issue.targetId)[0];
    const number = ds && slotId ? numberOf(ds, slotId) : undefined;
    if (number === undefined || !slotId) {
      unnumberedAnswerIssues.push(issue);
      continue;
    }
    const entry = byNumber.get(number) ?? { slotId, issues: [] };
    entry.issues.push(issue);
    byNumber.set(number, entry);
  }
  for (const run of contiguousRuns([...byNumber.keys()])) {
    const entries = run.map((number) => byNumber.get(number)!);
    const covers = entries.flatMap((entry) => entry.issues.map((issue) => issue.issueId));
    tasks.push({
      taskId: `missing-answer:${entries.map((entry) => entry.slotId).join("+")}`,
      kind: "missing-answer",
      severity: "blocker",
      title: `${questionRangeLabel(run)}缺少答案`,
      actions: [{ id: "fill-answer", label: "去填写", targetId: entries[0].slotId }],
      covers
    });
  }

  // 答案与题目形式不匹配：同样定位到答案控件，但话术不同（要改的是「答案形式」，不是「没填」）。
  const mismatchNumbers: number[] = [];
  const mismatchSlots: string[] = [];
  const mismatchCovers: string[] = [];
  let mismatchFirstSlot: string | undefined;
  for (const issue of take("answer-mismatch")) {
    const slotId = slotIdsOfTarget(ds, issue.targetId)[0];
    const number = ds && slotId ? numberOf(ds, slotId) : undefined;
    if (number === undefined || !slotId) {
      logUnlocalized(issue.issueId);
      continue;
    }
    mismatchNumbers.push(number);
    mismatchSlots.push(slotId);
    if (!mismatchFirstSlot) mismatchFirstSlot = slotId;
    mismatchCovers.push(issue.issueId);
  }
  if (mismatchCovers.length) {
    const range = questionRangeLabel(mismatchNumbers);
    tasks.push({
      taskId: "answer-mismatch",
      kind: "answer-mismatch",
      severity: "blocker",
      title: `${range}的答案和题目形式对不上`,
      detail: answerMismatchDetail(ds, mismatchSlots),
      actions: [{ id: "fill-answer", label: "去填写", targetId: mismatchFirstSlot ?? "document" }],
      covers: mismatchCovers
    });
  }

  // 2) 同一题组的内部问题并成一条「这组题没有识别完整」（任务书第二节第 2 条）。
  //    题干、stimulus、source boundary、slot host 都指向**同一个题组**，对用户是同一件事：
  //    这一组没读全，要对着原文件核对。
  const byGroup = new Map<string, ActionableIssueV1[]>();
  for (const issue of take("incomplete-recognition")) {
    const slotIds = slotIdsOfTarget(ds, issue.targetId);
    // 目标能落到题组就用题组 id 聚合；落到答案位就归到它的题组；都落不到（整篇级）用 `document`。
    const groupKey = groupKeyOfTarget(ds, issue.targetId, slotIds);
    const list = byGroup.get(groupKey) ?? [];
    list.push(issue);
    byGroup.set(groupKey, list);
  }
  for (const [groupKey, groupIssues] of byGroup) {
    const numbers = groupIssues.flatMap((issue) =>
      slotIdsOfTarget(ds, issue.targetId)
        .map((slotId) => (ds ? numberOf(ds, slotId) : undefined))
        .filter((value): value is number => value !== undefined)
    );
    const range = questionRangeLabel(numbers);
    if (!range) {
      groupIssues.forEach((issue) => logUnlocalized(issue.issueId));
      continue;
    }
    tasks.push({
      taskId: `incomplete-recognition:${groupKey}`,
      kind: "incomplete-recognition",
      severity: "blocker",
      title: `${range}没有识别完整`,
      detail: "请对照原文件检查题干和答案。",
      // 不给「重新识别」：重跑不会替换已经生成的题稿，按了也改不掉这里的问题。
      actions: [{ id: "view-source", label: "查看原文", targetId: groupKey }],
      covers: groupIssues.map((issue) => issue.issueId)
    });
  }

  // 3) 资源缺失。仓内暂无「重新选择图片」入口，所以给的是**真能按**的「重新识别」
  //    （任务书第三节允许「重新选择图片**或**重新识别」）。不给假按钮。
  const assetIssues = take("missing-asset");
  const assetGroups = new Map<string, ActionableIssueV1[]>();
  for (const issue of assetIssues) {
    const where = localizedTarget(ds, issue.targetId);
    if (!where) {
      logUnlocalized(issue.issueId);
      continue;
    }
    const list = assetGroups.get(where) ?? [];
    list.push(issue);
    assetGroups.set(where, list);
  }
  for (const [where, entries] of assetGroups) {
    tasks.push({
      taskId: `missing-asset:${where}`,
      kind: "missing-asset",
      severity: "blocker",
      title: `${where}有图片没有识别到`,
      detail: "请对照原文件确认图片。",
      actions: [{ id: "view-source", label: "查看原文", targetId: entries[0].targetId }],
      covers: entries.map((issue) => issue.issueId)
    });
  }

  // 4) 其余处理失败：一条任务，动作是重试处理。
  const failedIssues = take("processing-failed");
  const failedGroups = new Map<string, ActionableIssueV1[]>();
  for (const issue of failedIssues) {
    const where = localizedTarget(ds, issue.targetId);
    if (!where) {
      logUnlocalized(issue.issueId);
      continue;
    }
    const list = failedGroups.get(where) ?? [];
    list.push(issue);
    failedGroups.set(where, list);
  }
  for (const [where, entries] of failedGroups) {
    tasks.push({
      taskId: `processing-failed:${where}`,
      kind: "processing-failed",
      severity: "blocker",
      title: `${where}有一处内容没有处理好`,
      detail: `请对照原文件检查${where}。`,
      actions: [{ id: "view-source", label: "查看原文", targetId: entries[0].targetId }],
      covers: entries.map((issue) => issue.issueId)
    });
  }

  // 5) 泛化行与结构行：**只有其阻塞原因已被具体任务完整表达时**才隐藏。
  //
  //    这是本轮修正的一条（任务书第 4 条）。上一版写的是「有任意一个具体任务就隐藏全部
  //    泛化行」——那会藏掉**尚未被解释的发布失败**：门禁报出 3 个阻断，界面只把其中 1 个
  //    表达成了任务，另外 2 个（例如编译器失败）就被顺手抹掉，用户看到「还有 1 处需要处理」，
  //    而发布其实还被另外两条拦着。隐藏的前提必须是**原因被完整表达**，而不是「有别的任务」。
  const genericIssues = take("generic");
  const structureIssues = take("structure-incomplete");
  // 已被具体任务表达过的根因（复用 `rootCausesOf`，与渲染 `data-task-covers` 用的是同一份映射）。
  const explained = new Set<string>();
  for (const task of tasks) {
    for (const code of rootCausesOf(issues, task)) explained.add(code);
  }
  // 尚未被任何任务表达的原因。泛化汇总行本身不算「原因」（它只是「有硬失败」的汇总），
  // 所以它不参与这个判定——真正要保护的是那些**说得出原因、却没有对应任务**的行。
  const unexplained = [...genericIssues, ...structureIssues].filter(
    (issue) => !GENERIC_ONLY.has(issueRootCause(issue)) && !explained.has(issueRootCause(issue))
  );

  for (const issue of genericIssues) {
    if (!localizedTarget(ds, issue.targetId)) logUnlocalized(issue.issueId);
  }

  const unexplainedGroups = new Map<string, ActionableIssueV1[]>();
  for (const issue of unexplained) {
    const where = localizedTarget(ds, issue.targetId);
    if (!where) {
      logUnlocalized(issue.issueId);
      continue;
    }
    const list = unexplainedGroups.get(where) ?? [];
    list.push(issue);
    unexplainedGroups.set(where, list);
  }
  for (const [where, entries] of unexplainedGroups) {
    tasks.push({
      taskId: `structure-incomplete:${where}`,
      kind: "structure-incomplete",
      severity: "blocker",
      title: `${where}还有内容可能没有识别完整`,
      detail: `请对照原文件检查${where}的题面、材料或答案。`,
      actions: [{ id: "view-source", label: "查看原文", targetId: entries[0].targetId }],
      covers: entries.map((issue) => issue.issueId)
    });
  }
  // 被隐藏的行数仍记入 `mergedRowCount`：验收脚本据此断言「泛化行确实被合并掉了」，
  // 而不是「渲染时把行吞掉了」。
  const hiddenGenerics = issues.length > 0 && tasks.length > 0
    ? genericIssues.length + structureIssues.length - unexplained.length
    : 0;
  // 5c) 没有题号/原卷区域就无法告诉用户去哪里检查：只记日志，不生成泛化任务。
  if (unnumberedAnswerIssues.length) {
    unnumberedAnswerIssues.forEach((issue) => logUnlocalized(issue.issueId));
  }

  const order: Record<UserTaskKind, number> = {
    "missing-answer": 0,
    "answer-mismatch": 1,
    "incomplete-recognition": 2,
    "structure-incomplete": 3,
    "missing-asset": 4,
    "processing-failed": 5,
    "cloud-difference": 6,
    "cloud-note": 7,
    // 这里排不出 `inferred-slot`：它在 `buildEditingAids` 末尾追加，不经过本函数排序。
    // 类型上仍要给一个位置，取一个不影响任何已排序任务的号。
    "inferred-slot": 8
  };
  tasks.sort((a, b) => {
    if (a.severity !== b.severity) return a.severity === "blocker" ? -1 : 1;
    return order[a.kind] - order[b.kind];
  });

  const blockerCount = tasks.filter((task) => task.severity === "blocker").length;
  // 被合并掉的原始行数 = 原始行数 − 任务实际覆盖到的行数（+ 被隐藏的泛化行）。
  // 这不是展示用的数字，而是**验收证据**：脚本据此断言「合并真的发生了」，
  // 而不是「渲染时把行吞掉了」。
  const coveredRows = tasks.reduce((sum, task) => sum + task.covers.length, 0);
  const mergedRowCount = Math.max(0, issues.length - coveredRows) + hiddenGenerics;
  return {
    ready: true,
    tasks,
    headline: headlineFor(tasks.length),
    blockerCount,
    mergedRowCount
  };
}

/** 一个目标归属的题组 id：能落到题组就落到题组，落不到就按答案位所属题组，最后退成 `document`。 */
function groupKeyOfTarget(ds: IeltsAuthoringIRV2 | undefined, targetId: string, slotIds: readonly string[]): string {
  if (!ds) return targetId || "document";
  for (const task of ds.taskGroups) {
    if (task.taskId === targetId) return task.taskId;
    for (const group of task.responseGroups) {
      if (group.responseGroupId === targetId) return group.responseGroupId;
    }
  }
  if (slotIds.length) {
    const group = groupOwningSlot(ds, slotIds[0]);
    if (group) return group.responseGroupId;
  }
  return targetId || "document";
}

function groupOwningSlot(ds: IeltsAuthoringIRV2 | undefined, slotId: string): ResponseGroupV2 | undefined {
  if (!ds) return undefined;
  for (const task of ds.taskGroups) {
    for (const group of task.responseGroups) {
      if (group.slotIds.includes(slotId)) return group;
    }
  }
  return undefined;
}

/** 任务列表在普通界面里的折叠阈值：先分组，仍然很多时显示「还有 N 组问题」允许展开。 */
export const USER_TASK_VISIBLE_LIMIT = 6;

/**
 * 这条任务覆盖了哪些**根因码**。
 *
 * 只给验收脚本用（渲染成 `data-task-covers`，不可见）：合并之后原始问题行不再单独渲染，
 * 但「门禁报出的每个根因都被某条任务接住」这条**不隐藏事实**的要求必须仍然可验证。
 * 靠它就能断言「合并发生了」而不是「渲染时把行吞掉了」。
 */
export function rootCausesOf(issues: readonly ActionableIssueV1[], task: UserTaskV1): string[] {
  const byId = new Map(issues.map((issue) => [issue.issueId, issue]));
  const codes = task.covers
    .map((issueId) => byId.get(issueId))
    .filter((issue): issue is ActionableIssueV1 => Boolean(issue))
    .map((issue) => issueRootCause(issue));
  return [...new Set(codes)];
}

export function splitVisibleTasks(tasks: readonly UserTaskV1[], expanded: boolean): {
  visible: UserTaskV1[];
  hiddenCount: number;
} {
  if (expanded || tasks.length <= USER_TASK_VISIBLE_LIMIT) {
    return { visible: [...tasks], hiddenCount: 0 };
  }
  return {
    visible: tasks.slice(0, USER_TASK_VISIBLE_LIMIT),
    hiddenCount: tasks.length - USER_TASK_VISIBLE_LIMIT
  };
}

/**
 * 列表顶部那一句。**编辑辅助，不是门槛**：不说「可以导出 / 不能导出」，也不报阻断数——
 * 按「发布」本身就是用户的确认（产品决定 2）。
 */
export function headlineFor(count: number): string {
  return count === 0 ? "没有需要补充的内容" : `还有 ${count} 处可以补充`;
}

/** 云端修复后剩下的一条（后端 `remainingTasks` 的单项，按读取时的当前稿重算过）。 */
export interface RepairAidInputV1 {
  comparisonUnitId?: string;
  localSelectable?: boolean;
  cloudSelectable?: boolean;
  taskIds?: string[];
  localCandidate?: IeltsAuthoringIRV2;
  cloudCandidate?: IeltsAuthoringIRV2;
  decisionStatus?: "user_choice" | "need_context";
  sourcePages?: number[];
  userTaskId: string;
  targetIds?: string[];
  message?: string | null;
  action?: string;
  field?: string;
  currentValue?: unknown;
  cloudValue?: unknown;
  challengerLabel?: string;
  questionNumbers?: number[];
  pageIndex?: number;
  pageNumber?: number;
  region?: string;
  regionLabel?: string;
  sourceRegion?: unknown;
  evidence?: unknown;
  /**
   * 云端**没能拿到足够的原文**来判断这一处（后端理由码 `CONTEXT_INSUFFICIENT`）。
   *
   * 它与「云端查过原文件但定不了论」是两件事：前者材料根本没到手，后者是看过之后仍
   * 无定论。两者在界面上必须说成不同的话——否则用户会以为云端看过原文。
   */
  contextInsufficient?: boolean;
}

const FIELD_LABEL: Record<string, string> = {
  answer: "答案",
  prompt: "题面",
  instructions: "作答说明",
  stimulus: "材料",
  option_bank: "选项",
  task_group: "整组题",
  part_boundary: "分段范围",
  part_label: "分段名称",
  part_tasks: "分段归属"
};

/**
 * 一个听力分段在界面上的名字：「SECTION 3（第 21–30 题）」。
 *
 * 刻意**不用** `partId`：`part-5` 是后端分配的内部身份，用户认不出，也不该看到。
 * 名字取用户自己看得到的那两样——段落标签与题号范围。
 */
function partRangeLabel(value: unknown): string {
  if (!value || typeof value !== "object") return "没有这一段";
  const entry = value as { displayLabel?: unknown; expectedQuestionNumbers?: unknown };
  const label =
    typeof entry.displayLabel === "string" && entry.displayLabel.trim()
      ? entry.displayLabel.trim()
      : "未命名分段";
  const numbers = Array.isArray(entry.expectedQuestionNumbers)
    ? entry.expectedQuestionNumbers.filter((number): number is number => typeof number === "number")
    : [];
  const range = questionRangeLabel(numbers);
  return range ? `${label}（${range}）` : label;
}

/** 按 `partId` 在当前稿里找到那一段，给出它的界面名字。 */
function partLabelOf(ds: IeltsAuthoringIRV2 | undefined, partId: string): string | undefined {
  const parts = ds?.listening?.parts;
  if (!Array.isArray(parts)) return undefined;
  const part = parts.find((candidate) => candidate.partId === partId);
  if (!part) return undefined;
  const label = part.displayLabel?.trim() ? part.displayLabel.trim() : "未命名分段";
  const range = questionRangeLabel(part.expectedQuestionNumbers ?? []);
  return range ? `${label}（${range}）` : label;
}

function placeLabel(ds: IeltsAuthoringIRV2, targetId: string): string {
  const part = partLabelOf(ds, targetId);
  if (part) return part;
  const number = numberOf(ds, targetId);
  if (number !== undefined) return `第 ${number} 题`;
  const slots = slotIdsOfTarget(ds, targetId)
    .map((slotId) => numberOf(ds, slotId))
    .filter((value): value is number => value !== undefined);
  return slots.length ? questionRangeLabel(slots) : "这一处";
}

function localizedTarget(ds: IeltsAuthoringIRV2, targetId: string): string | undefined {
  if (!targetId || targetId === "document") return undefined;
  const label = placeLabel(ds, targetId);
  return label === "这一处" ? undefined : label;
}

function groupForTarget(ds: IeltsAuthoringIRV2, targetId: string) {
  return ds.taskGroups.find((task) =>
    task.taskId === targetId
    || task.responseGroups.some((group) =>
      group.responseGroupId === targetId
      || group.slotIds.includes(targetId)
    )
  );
}

function questionNumbersFor(ds: IeltsAuthoringIRV2, targetId: string, explicit?: readonly number[]): number[] {
  const given = (explicit ?? []).filter((number) => Number.isInteger(number) && number > 0);
  if (given.length) return [...new Set(given)].sort((a, b) => a - b);
  return [...new Set(slotIdsOfTarget(ds, targetId)
    .map((slotId) => numberOf(ds, slotId))
    .filter((number): number is number => number !== undefined))]
    .sort((a, b) => a - b);
}

function rawText(value: unknown): string {
  if (typeof value === "string") return value;
  if (Array.isArray(value)) return value.map(rawText).filter(Boolean).join(" ");
  if (!value || typeof value !== "object") return value == null ? "" : String(value);
  const record = value as Record<string, unknown>;
  if (record.kind === "text" && Array.isArray(record.values)) return record.values.map(rawText).join(" ");
  if (record.kind === "option" && Array.isArray(record.labels)) return record.labels.map(rawText).join(" ");
  if (typeof record.text === "string") return record.text;
  if (Array.isArray(record.content)) return rawText(record.content);
  if (Array.isArray(record.children)) return rawText(record.children);
  return formatDecisionValue(value);
}

function normalizedReviewText(value: unknown, field: string): string {
  let text = rawText(value)
    .normalize("NFKC")
    .replace(/[\u2018\u2019\u02bc]/g, "'")
    .replace(/[\u2010-\u2015\u2212]/g, "-")
    .replace(/\s+/g, " ")
    .replace(/\s*-\s*/g, "-")
    .trim()
    .toLocaleLowerCase("en");
  if (field === "instructions") {
    text = text.replace(/^questions\s+\d+(?:-\d+)?(?:\s*[:.)]\s*|\s+)/, "").trim();
  }
  return text;
}

function hasPdfCharacterSpacing(value: string): boolean {
  return /(?:^|\s)(?:[a-z]\s+){4,}[a-z](?:$|\s)/i.test(value);
}

function answerTokens(value: unknown): { values: string[]; assignment: string } | undefined {
  if (typeof value === "string") return { values: [value], assignment: "per_slot" };
  if (!value || typeof value !== "object" || Array.isArray(value)) return undefined;
  const record = value as Record<string, unknown>;
  const rawValues = record.kind === "option" ? record.labels : record.kind === "text" ? record.values : undefined;
  if (!Array.isArray(rawValues)) return undefined;
  return {
    values: rawValues.filter((entry): entry is string => typeof entry === "string"),
    assignment: typeof record.assignment === "string" ? record.assignment : "per_slot"
  };
}

function normalizeAnswerToken(value: string, taskType: string): string {
  const normalized = normalizedReviewText(value, "answer");
  if ((taskType === "true_false_not_given" || taskType === "yes_no_not_given")
    && ["not stated", "not mentioned"].includes(normalized)) return "not given";
  return normalized;
}

function optionLabelForToken(ds: IeltsAuthoringIRV2, targetId: string, token: string): string | undefined {
  const options = groupForTarget(ds, targetId)?.optionBank?.options ?? [];
  const normalized = normalizedReviewText(token, "answer");
  const matches = options.filter((option) =>
    [option.optionId, option.label].some((candidate) => normalizedReviewText(candidate, "answer") === normalized)
  );
  return matches.length === 1 ? matches[0].label : undefined;
}

function sameRepairValue(ds: IeltsAuthoringIRV2, task: RepairAidInputV1, targetId: string, field: string): boolean {
  if (field === "answer") {
    const current = answerTokens(task.currentValue);
    const cloud = answerTokens(task.cloudValue);
    if (current && cloud) {
      const taskType = groupForTarget(ds, targetId)?.taskType ?? "";
      const key = (entry: { values: string[]; assignment: string }) => entry.values
        .map((value) => normalizeAnswerToken(optionLabelForToken(ds, targetId, value) ?? value, taskType))
        .filter(Boolean)
        .sort()
        .join("|") + `::${entry.assignment}`;
      return key(current) === key(cloud);
    }
  }
  const current = normalizedReviewText(task.currentValue, field);
  const cloud = normalizedReviewText(task.cloudValue, field);
  if (current === cloud) return true;
  return Boolean(current && cloud
    && (hasPdfCharacterSpacing(current) || hasPdfCharacterSpacing(cloud))
    && current.replace(/\s/g, "") === cloud.replace(/\s/g, ""));
}

function valueForReview(value: unknown, ds: IeltsAuthoringIRV2, targetId: string): string {
  const answer = answerTokens(value);
  if (!answer) return rawText(value) || "（空）";
  return answer.values
    .map((token) => optionLabelForToken(ds, targetId, token) ?? token.trim())
    .filter(Boolean)
    .join(", ") || "（空）";
}

function sourceRegionLabel(task: RepairAidInputV1): string | undefined {
  const direct = [task.region, task.regionLabel].find((value) => typeof value === "string" && value.trim());
  if (direct) return direct!.trim();
  const region = task.sourceRegion;
  if (typeof region === "string" && region.trim()) return region.trim();
  if (region && typeof region === "object") {
    const record = region as Record<string, unknown>;
    for (const key of ["label", "name", "title", "region", "description"]) {
      if (typeof record[key] === "string" && (record[key] as string).trim()) return (record[key] as string).trim();
    }
  }
  const evidence = Array.isArray(task.evidence) ? task.evidence : [task.evidence];
  for (const item of evidence) {
    if (!item || typeof item !== "object") continue;
    const record = item as Record<string, unknown>;
    for (const key of ["regionLabel", "region", "areaLabel", "area"]) {
      if (typeof record[key] === "string" && (record[key] as string).trim()) return (record[key] as string).trim();
    }
  }
  return undefined;
}

function safeCoverageDescription(message: string): string | undefined {
  const cleaned = message
    .trim()
    .replace(/[（(][A-Z][A-Z0-9_]{2,}[）)](?:\s*[:：].*)?$/u, "")
    .replace(/\s*[:：]\s*(?:ocr|pdfium|recognition|page unreadable|processing failed)\b.*$/iu, "")
    .trim();
  if (!cleaned || /(?:PAGE_[A-Z_]+|ocr\s+failed|pdfium|stack trace)/i.test(cleaned)) return undefined;
  if (/^原文件(?:第\s*\d+\s*页)?(?:云端)?(?:没能|未能)?读全[。！!]?$/u.test(cleaned)) return undefined;
  return cleaned;
}

function sourcePageNumber(task: RepairAidInputV1): number | undefined {
  if (Number.isInteger(task.pageNumber) && task.pageNumber! > 0) return task.pageNumber;
  if (Number.isInteger(task.pageIndex) && task.pageIndex! >= 0) return task.pageIndex! + 1;
  const evidence = Array.isArray(task.evidence) ? task.evidence : [task.evidence];
  for (const item of evidence) {
    if (!item || typeof item !== "object") continue;
    const record = item as Record<string, unknown>;
    if (Number.isInteger(record.pageNumber) && (record.pageNumber as number) > 0) return record.pageNumber as number;
    if (Number.isInteger(record.pageIndex) && (record.pageIndex as number) >= 0) return (record.pageIndex as number) + 1;
  }
  return undefined;
}

function logUnlocalized(id: string): void {
  console.warn("[authoring-review] skipped unlocalized task", id);
}

/**
 * **唯一**一份编辑辅助清单：本地检查 + 发布前检查 + 学生预览的答案形式问题 + 云端修复后剩下的，
 * 合成一份，每个题位 / 每件事只出一条，修好了就消失（云端那一路在读取时按当前稿重算）。
 *
 * 云端那一路的后端文案带内部词（「expected question numbers 与 slots」「计分 slot」「第 q27 题」），
 * 这里一律不透传：质量问题按质量码走与本地同一套分类与话术；差异写成「现在是 X，云端读到的是 Y」；
 * 覆盖缺口只说「原文件有一部分云端没读全」。
 */
export function buildEditingAids(
  ds: IeltsAuthoringIRV2 | undefined,
  issues: readonly ActionableIssueV1[],
  repairTasks: readonly RepairAidInputV1[] = []
): UserTaskSummaryV1 {
  const qualityRows: ActionableIssueV1[] = [];
  const others: RepairAidInputV1[] = [];
  for (const task of repairTasks) {
    const match = /^quality:([A-Z0-9_]+):/.exec(task.userTaskId ?? "");
    if (match) {
      qualityRows.push({
        issueId: `repair:${task.userTaskId}`,
        targetId: task.targetIds?.find((id) => id) ?? "document",
        severity: "blocker",
        code: match[1],
        userMessage: "",
        source: "gate",
        rootCause: match[1]
      });
    } else {
      others.push(task);
    }
  }
  const summary = buildUserTasks(ds, [...issues, ...qualityRows]);
  if (!ds || !summary.ready) return summary;

  // 已经有条目的题位：云端那一路不再为它另起一条（一个题位只出一条）。
  const covered = new Set<string>();
  for (const task of summary.tasks) {
    for (const action of task.actions) covered.add(action.targetId);
    for (const part of task.taskId.split(/[:+]/)) covered.add(part);
  }
  const tasks = [...summary.tasks];
  const seen = new Set<string>();
  const coverage = new Map<string, {
    questionNumbers: number[];
    pageNumber?: number;
    region?: string;
    messages: string[];
    covers: string[];
    targetId?: string;
  }>();
  for (const task of others) {
    const target = task.targetIds?.find((id) => id) ?? "";
    const id = task.userTaskId;
    if (seen.has(id)) continue;
    seen.add(id);
    if (id.startsWith("cloud-diff:")) {
      if (target && covered.has(target)) continue;
      const field = task.field ?? id.split(":").pop() ?? "";
      const questionNumbers = questionNumbersFor(ds, target, task.questionNumbers);
      const isPartBoundary = field === "part_boundary";
      const currentPart = isPartBoundary ? partRangeLabel(task.currentValue) : "";
      const cloudPart = isPartBoundary ? partRangeLabel(task.cloudValue) : "";
      const where = isPartBoundary
        ? currentPart !== "没有这一段" ? currentPart : cloudPart
        : questionRangeLabel(questionNumbers) || (target ? placeLabel(ds, target) : "这一处");
      if (where === "这一处") {
        logUnlocalized(id);
        continue;
      }
      if (sameRepairValue(ds, task, target, field)) continue;
      const label = FIELD_LABEL[field] ?? "内容";
      const isAnswer = field === "answer";
      const current = valueForReview(task.currentValue, ds, target);
      const cloud = valueForReview(task.cloudValue, ds, target);
      const cloudLabel = task.challengerLabel || "云端";
      tasks.push({
        taskId: id,
        kind: "cloud-difference",
        severity: "warning",
        title: isPartBoundary
          ? `听力分段对不上：现在是「${partRangeLabel(task.currentValue)}」，云端读到的是「${partRangeLabel(task.cloudValue)}」`
          : isAnswer
            ? `${where}的答案：现在是「${current}」，${cloudLabel}读到的是「${cloud}」`
            : `${where}的${label}有差异`,
        // 「材料没到手」不能说成「看过了但定不了」：后者会让人以为云端已经对照过原文。
        detail: task.contextInsufficient
          ? "云端没能拿到足够的原文来判断这一处，请对照原文确认。"
          : "本地与云端识别存在差异，云端对照原文件后没能定论。请查看下面标出的两侧内容，再对照原文决定是否调整。",
        comparison: isPartBoundary ? undefined : { current, cloud, cloudLabel },
        actions: isAnswer && target
          ? [{ id: "fill-answer", label: "去看看", targetId: target }, { id: "view-source", label: "查看原文", targetId: target }]
          : [{ id: "view-source", label: "查看原文", targetId: target || "document" }],
        covers: [id]
      });
      if (target) covered.add(target);
      continue;
    }
    if (id.startsWith("cloud-question:")) {
      if (target && covered.has(target)) continue;
      const questionNumbers = questionNumbersFor(ds, target, task.questionNumbers);
      const where = questionRangeLabel(questionNumbers) || (target ? placeLabel(ds, target) : "");
      if (!where || where === "这一处") {
        logUnlocalized(id);
        continue;
      }
      const answerTarget = target || Object.values(ds.answerSlots).find((slot) => questionNumbers.includes(slot.questionNumber))?.slotId || "";
      const text = (task.message ?? "").replace(/^云端未能确认[：:]\s*/, "").trim();
      tasks.push({
        taskId: id,
        kind: "cloud-note",
        severity: "warning",
        title: `${where}：云端没能确认`,
        detail: text || undefined,
        actions: answerTarget
          ? [{ id: "fill-answer", label: "去看看", targetId: answerTarget }]
          : [{ id: "view-source", label: "查看原文", targetId: "document" }],
        covers: [id]
      });
      continue;
    }
    const questionNumbers = questionNumbersFor(ds, target, task.questionNumbers);
    const pageNumber = sourcePageNumber(task);
    const region = sourceRegionLabel(task);
    const locationKey = pageNumber && region
      ? `page:${pageNumber}:${normalizedReviewText(region, "region")}`
      : questionNumbers.length
        ? `questions:${questionNumbers.join(",")}`
        : "";
    if (!locationKey) {
      logUnlocalized(id);
      continue;
    }
    const bucket = coverage.get(locationKey) ?? {
      questionNumbers: [],
      pageNumber,
      region,
      messages: [],
      covers: [],
      targetId: target || undefined
    };
    bucket.questionNumbers = [...new Set([...bucket.questionNumbers, ...questionNumbers])].sort((a, b) => a - b);
    bucket.messages.push((task.message ?? "").trim());
    bucket.covers.push(id);
    if (!bucket.targetId && target) bucket.targetId = target;
    coverage.set(locationKey, bucket);
  }

  for (const [locationKey, entry] of coverage) {
    const questionRange = questionRangeLabel(entry.questionNumbers);
    const sourceArea = entry.pageNumber && entry.region
      ? `原文件第 ${entry.pageNumber} 页「${entry.region}」区域`
      : undefined;
    const location = [questionRange, sourceArea].filter(Boolean).join("，");
    const where = questionRange
      ? `${questionRange}的原文件内容${sourceArea ? `（${sourceArea}）` : ""}`
      : sourceArea!;
    const target = entry.targetId
      || Object.values(ds.answerSlots).find((slot) => entry.questionNumbers.includes(slot.questionNumber))?.slotId
      || "document";
    const descriptions = [...new Set(entry.messages
      .map(safeCoverageDescription)
      .filter((message): message is string => Boolean(message)))];
    tasks.push({
      taskId: `cloud-coverage:${locationKey}`,
      kind: "cloud-note",
      severity: "warning",
      title: `${where}可能没有识别完整`,
      detail: [location, "请检查题面、选项或答案说明是否有遗漏。", ...descriptions].join("："),
      actions: [{ id: "view-source", label: "查看原文", targetId: target }],
      covers: entry.covers
    });
  }

  // 识别自己记下的「空位是推断出来的」：草稿的 `taskGroups[].recognitionWarnings`
  // 里带一条 `slot_order_inferred:<题号…>`（由 `completion.rs` 在按阅读顺序把
  // 未印题号的空位派给剩余题号时写入）。这是**推断来源**，不是缺内容，所以只给
  // 一条 warning，不参与 `blockerCount`，发布判据完全不动。
  const inferredNumbers: number[] = [];
  for (const task of ds.taskGroups) {
    for (const warning of task.recognitionWarnings ?? []) {
      if (!warning.startsWith(SLOT_ORDER_INFERRED_WARNING)) continue;
      const rest = warning.slice(SLOT_ORDER_INFERRED_WARNING.length);
      for (const part of rest.split(",")) {
        const number = Number(part.trim());
        if (Number.isInteger(number) && number > 0) inferredNumbers.push(number);
      }
    }
  }
  if (inferredNumbers.length) {
    const range = questionRangeLabel(inferredNumbers);
    tasks.push({
      taskId: "inferred-slot",
      kind: "inferred-slot",
      severity: "warning",
      title: `${range}的空位位置是按阅读顺序推断的`,
      detail: "原卷这几行没有印题号，识别按顺序把它们对到了剩余题号上；请对照原文件确认位置对不对。",
      actions: [{ id: "view-source", label: "查看原文", targetId: "document" }],
      covers: []
    });
  }
  return { ...summary, tasks, headline: headlineFor(tasks.length) };
}
