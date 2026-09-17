import type { IeltsAuthoringIRV2, ResponseGroupV2 } from "../../types";
import { issueRootCause, type ActionableIssueV1, type IssueSeverity } from "./actionableIssues";

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
//      （定位答案控件 / 打开原文件 / 重新识别），不给「确认」「忽略」这类点了不改门禁的按钮。

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
  /** 其余处理失败：重试处理。 */
  | "processing-failed";

export type UserTaskActionId =
  /** 定位到答案控件（题面上的输入框）。 */
  | "fill-answer"
  /** 打开原文件并定位到题组。 */
  | "view-source"
  /** 重新识别这道题。 */
  | "retry-recognition";

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
  /** 顶部那一行：还有 N 处需要处理 / 可以导出。 */
  headline: string;
  /** 还有几处阻断（决定「能不能导出」）。 */
  blockerCount: number;
  /** 被合并掉的原始问题行数（验收报告用，普通界面不显示）。 */
  mergedRowCount: number;
}

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

/** 题号区间的用户话术：「第 11 题」「第 11–13 题」。 */
function questionRangeLabel(numbers: readonly number[]): string {
  const sorted = [...new Set(numbers)].sort((a, b) => a - b);
  if (!sorted.length) return "";
  if (sorted.length === 1) return `第 ${sorted[0]} 题`;
  return `第 ${sorted[0]}–${sorted[sorted.length - 1]} 题`;
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
  const mismatchCovers: string[] = [];
  let mismatchFirstSlot: string | undefined;
  for (const issue of take("answer-mismatch")) {
    const slotId = slotIdsOfTarget(ds, issue.targetId)[0];
    const number = ds && slotId ? numberOf(ds, slotId) : undefined;
    if (number !== undefined) mismatchNumbers.push(number);
    if (slotId && !mismatchFirstSlot) mismatchFirstSlot = slotId;
    mismatchCovers.push(issue.issueId);
  }
  if (mismatchCovers.length) {
    const range = questionRangeLabel(mismatchNumbers);
    tasks.push({
      taskId: "answer-mismatch",
      kind: "answer-mismatch",
      severity: "blocker",
      title: range ? `${range}的答案和题目形式对不上` : "有答案和题目形式对不上",
      detail: "这些答案填进去学生也提交不了，需要按题目要求改。",
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
    tasks.push({
      taskId: `incomplete-recognition:${groupKey}`,
      kind: "incomplete-recognition",
      severity: "blocker",
      title: range ? `${range}没有识别完整` : "这道题还有内容没有识别完整",
      detail: "请对照原文件检查题干和答案。",
      actions: [
        { id: "view-source", label: "查看原文", targetId: groupKey },
        { id: "retry-recognition", label: "重新识别", targetId: groupKey }
      ],
      covers: groupIssues.map((issue) => issue.issueId)
    });
  }

  // 3) 资源缺失。仓内暂无「重新选择图片」入口，所以给的是**真能按**的「重新识别」
  //    （任务书第三节允许「重新选择图片**或**重新识别」）。不给假按钮。
  const assetIssues = take("missing-asset");
  if (assetIssues.length) {
    tasks.push({
      taskId: "missing-asset",
      kind: "missing-asset",
      severity: "blocker",
      title: "这道题有图片没有识别到",
      detail: "重新识别一次；如果还是不行，请对照原文件确认图片是否清晰。",
      actions: [{ id: "retry-recognition", label: "重新识别", targetId: assetIssues[0].targetId }],
      covers: assetIssues.map((issue) => issue.issueId)
    });
  }

  // 4) 其余处理失败：一条任务，动作是重试处理。
  const failedIssues = take("processing-failed");
  if (failedIssues.length) {
    tasks.push({
      taskId: "processing-failed",
      kind: "processing-failed",
      severity: "blocker",
      title: "这道题有一处处理失败",
      detail: "重试一次通常可以恢复；重复失败请把原文件重新导入。",
      actions: [{ id: "retry-recognition", label: "重试处理", targetId: failedIssues[0].targetId }],
      covers: failedIssues.map((issue) => issue.issueId)
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

  if (tasks.length === 0 && (genericIssues.length || structureIssues.length)) {
    // 一条具体任务都没有：泛化/结构行是**唯一**的线索，必须显示（任务书第二节第 5 条）。
    tasks.push({
      taskId: "structure-incomplete",
      kind: "structure-incomplete",
      severity: "blocker",
      title: "这组题的结构还不完整，暂时不能导出。",
      detail: "重新识别一次；仍然不行请把原文件重新导入。",
      actions: [{ id: "retry-recognition", label: "重新识别", targetId: "document" }],
      covers: [...genericIssues, ...structureIssues].map((issue) => issue.issueId)
    });
  } else if (unexplained.length) {
    // 有具体任务，但仍有**没有被表达出来**的失败原因：如实再给一条，不让发布在界面上「看起来能过」。
    tasks.push({
      taskId: "structure-incomplete:unexplained",
      kind: "structure-incomplete",
      severity: "blocker",
      title: "这道题还有内容没有处理完，暂时不能导出。",
      detail: "重新识别一次；仍然不行请把原文件重新导入。",
      actions: [{ id: "retry-recognition", label: "重新识别", targetId: "document" }],
      covers: unexplained.map((issue) => issue.issueId)
    });
  }
  // 被隐藏的行数仍记入 `mergedRowCount`：验收脚本据此断言「泛化行确实被合并掉了」，
  // 而不是「渲染时把行吞掉了」。
  const hiddenGenerics = issues.length > 0 && tasks.length > 0
    ? genericIssues.length + structureIssues.length - unexplained.length
    : 0;
  // 5c) 落在没有题号的答案位上的缺答行：退化成一条不带题号的任务，
  //     而不是悄悄丢掉（丢一条阻断问题比多显示一条严重得多）。
  if (unnumberedAnswerIssues.length) {
    tasks.push({
      taskId: "missing-answer:unnumbered",
      kind: "missing-answer",
      severity: "blocker",
      title: "还有答案没有填写",
      actions: [
        { id: "fill-answer", label: "去填写", targetId: unnumberedAnswerIssues[0].targetId }
      ],
      covers: unnumberedAnswerIssues.map((issue) => issue.issueId)
    });
  }

  const order: Record<UserTaskKind, number> = {
    "missing-answer": 0,
    "answer-mismatch": 1,
    "incomplete-recognition": 2,
    "structure-incomplete": 3,
    "missing-asset": 4,
    "processing-failed": 5
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
    // 「没有问题」不生成卡片，只保留这一句（任务书第四节最后一条 + 第六节第 6 条）。
    headline: tasks.length === 0 ? "可以导出" : `还有 ${tasks.length} 处需要处理`,
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
