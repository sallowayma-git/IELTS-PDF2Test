import type { ContentNodeV2, IeltsAuthoringIRV2, ResponseGroupV2, TaskGroupV2 } from "../../types";

// ActionableIssue（计划 §8.5）：只呈现「第 12 题题干可能缺少下一行」这类可执行问题，
// 不把置信度、hash、schema 名当成用户任务。
//
// 后端的 typed ActionableIssue 与 reconciliation 是 P8 的工作。在它落地之前，这里对
// 当前 Canonical DS 做计划 §6.8 的硬闭包检查 —— 检查项与将来后端一致，
// 因此 P8 迁到后端时可以直接对比两边结果。
export type IssueSeverity = "blocker" | "warning";

export type IssueCode =
  | "QUESTION_PROMPT_MISSING"
  | "OPTION_TEXT_MISSING"
  | "OPTION_RUN_INCOMPLETE"
  | "SHARED_OPTION_BANK_MISSING"
  | "ANSWER_MISSING"
  | "ANSWER_UNRESOLVED";

export interface ActionableIssueV1 {
  issueId: string;
  /** 点击问题时要定位到的 DS 节点或实体 id。 */
  targetId: string;
  severity: IssueSeverity;
  /** 本地闭包检查码，或发布门禁的 blocker/warning 码（如 QUALITY_NOT_READY）。 */
  code: IssueCode | string;
  /** 用户可读的一句话，例如「第 18 题 B 选项未识别」。 */
  userMessage: string;
  /**
   * 这一行是谁产生的：`local` 是编辑器本地闭包检查（`deriveActionableIssues`），
   * `gate` 是发布门禁的 blocker / warning。合并时补上。
   *
   * 存在的理由：**光看 code 分不出来源** —— 本地与门禁都可能用 `ANSWER_MISSING`。
   * 校验脚本要能分别断言「门禁那半渲染对了」和「本地那半没被吃掉」，就需要这个标记。
   */
  source?: "local" | "gate";
  /**
   * 去重用的**根因码**。门禁行取自 `internal`（`phase4-<质量码>-<目标>` 里的质量码），
   * 本地行取自本地码（过别名表）。合并时补上。
   *
   * 为什么必须单独存：门禁用 `ISSUE_UNRESOLVED` 一个 code 承载所有质量码，
   * **光看 code 分不出两个不同的阻断问题**（实测 `complex-reading.pdf` 上
   * `SIGNIFICANT_REGION_UNASSIGNED` 与 `RUNTIME_COMPILER_FAILED` 都写 `ISSUE_UNRESOLVED`、
   * targetId 都是 `document`）。界面把它输出成 `data-issue-root-cause`，
   * 校验脚本据此断言「门禁里每个根因都在界面上出现了」。
   */
  rootCause?: string;
  /**
   * **后端给出的稳定事实 id**（门禁 blocker 的 `internal`，形如
   * `phase4-{质量码}-{目标}-{slug}`，`slug` 是判别性载荷的确定性哈希）。
   *
   * 这是任务书里「等后端稳定事实 id」那一条的落点。后端把 `issueId` 从
   * `phase4-{code}-{target}` 改成带 slug 之后，**同一目标上的两个不同事实终于有了不同的 id**
   * （此前 `complex-reading.pdf` 的 group-2 上两条不同事实撞成同一个 `phase4-SLOT_HOST_MISSING-group-2`）。
   *
   * 只在**同来源**比较时作为「更强的不同判据」使用，见 `sameFact()`：
   * id 不同 ⇒ 一定是两条事实。id 相同或缺失时**仍退回文案比较**，
   * 因为旧载荷的 id 会撞键、而直接 blocker（如 `ANSWER_MISSING`）根本没有 id。
   */
  factId?: string;
}

/** 发布门禁（`check_publish_preflight`）返回的结构化 blocker。 */
export interface PublishGateBlocker {
  code: string;
  targetId?: string | null;
  userMessage?: string;
  /** 门禁在 `internal` 里放了根因码；类型上没声明，但去重必须用到它。 */
  internal?: string;
}

export interface PublishGateResult {
  blockers?: PublishGateBlocker[];
  warnings?: Array<{ code: string, message?: string }>;
}

/**
 * 一个门禁 blocker 的「根因码」。
 *
 * 发布门禁对**同一个根因**会同时给出多条 code 不同的记录。实测 `demanding-reading-passage-3.pdf`
 * 缺 14 个答案时，一个根因产生了约 29 条阻断项：
 *   - `QUALITY_HARD_FAILURE`：`internal` 是质量码（`ANSWER_KEY_MISSING_SLOT`），targetId 为空，
 *     文案是泛化的「这道题存在必须修复的内容缺陷。」；
 *   - `ISSUE_UNRESOLVED`：`internal` 是 `phase4-<质量码>-<目标>`，targetId 是具体题位；
 *   - 该质量码自己的直接 blocker（如 `ANSWER_MISSING`），targetId 也是具体题位。
 * 归到同一个根因码上，才能判断哪条只是「泛化的重复」。
 */
export function rootCauseOf(blocker: PublishGateBlocker): string {
  const internal = blocker.internal ?? "";
  if (blocker.code === "QUALITY_HARD_FAILURE") return internal || blocker.code;
  // `phase4-<CODE>-<target>`。**不能**按最后一个 '-' 切：目标 id 自己也带 '-'（如 `group-2`），
  // 那样 `phase4-SLOT_HOST_MISSING-group-2` 会被切成 `SLOT_HOST_MISSING-group`。
  // 质量码是 SCREAMING_SNAKE_CASE，因此按「开头连续的大写/数字/下划线」取码。
  const phase4 = /^phase4-([A-Z0-9_]+)-/.exec(internal);
  if (blocker.code === "ISSUE_UNRESOLVED" && phase4) return phase4[1];
  return blocker.code;
}

/**
 * 一个门禁 blocker 携带的**后端稳定事实 id**，没有就返回 `undefined`。
 *
 * 只有 `ISSUE_UNRESOLVED` 的 `internal` 才是 `issueId`（见
 * `authoring_v2_commands.rs`：`"internal": issue.get("issueId")`）。
 * 别的 code 的 `internal` 是**另一个意思** —— `QUALITY_HARD_FAILURE` 放的是质量码本身 ——
 * 所以不能无条件取 `internal`，那会把质量码当成事实 id 用。
 */
export function factIdOf(blocker: PublishGateBlocker): string | undefined {
  if (blocker.code !== "ISSUE_UNRESOLVED") return undefined;
  const internal = (blocker.internal ?? "").trim();
  return internal || undefined;
}

/** 泛化 blocker：既不指向具体目标、文案也不带具体事实，只是「有硬失败」的汇总。 */
const GENERIC_GATE_CODES = new Set(["QUALITY_HARD_FAILURE"]);

/**
 * 同一根因在不同来源被写成了不同 code。
 *
 * 本地闭包看的是 `answerKey[slotId].kind`：`unresolved` 记成 `ANSWER_UNRESOLVED`；
 * 门禁不看 kind，一律记成 `ANSWER_MISSING`。对用户这是**同一件事**（这个空要填），
 * 但去重键 `code:targetId` 不同，于是**同一道题渲染两行**。
 *
 * 实测 `demanding-reading-passage-3.pdf`：14 个未解析的答案位 → 本地 14 条 warning
 * 加门禁 14 条 blocker，问题列表里 14 道题各占两行（46 行里 14 行是纯重复）。
 * 归一到同一个根因码才能去重。
 *
 * 只登记这一族：别名表越短越安全，宁可留两行也不要误合两条真正不同的问题。
 */
const ROOT_CAUSE_ALIASES: Record<string, string> = {
  ANSWER_UNRESOLVED: "ANSWER_MISSING"
};

/** 一条问题在界面上去重用的**根因码**。 */
export function issueRootCause(issue: ActionableIssueV1): string {
  return issue.rootCause ?? ROOT_CAUSE_ALIASES[issue.code] ?? issue.code;
}

/**
 * 两条行是不是**同一个事实**。
 *
 * 为什么不能只用 `根因 + 目标` 当唯一身份（这是本轮修正的一个真实错误）：
 * 实测 `complex-reading.pdf` 的 group-2 上，**两条不同事实**（「completion slot 没有可渲染的
 * 宿主节点。」与「table completion 没有可渲染的 table stimulus。」）**共用同一个 issueId**
 * （`phase4-SLOT_HOST_MISSING-group-2`）。根因相同、目标相同，但它们是两件事。
 * 只按 `根因 + 目标` 去重就会**整条吞掉**其中一个 —— 界面看起来「更干净了」，
 * 实际是**少了一条阻断问题**。任务书要求同时满足「同一问题不重复显示」与
 * 「不同问题不被隐藏」，所以必须把「事实」判得比「根因+目标」更细。
 *
 * 判据分两种情形，都是可解释的：
 *  - **跨来源**（本地闭包 vs 发布门禁）：同一个 `根因 + 目标` 就是同一件事。
 *    两个子系统各写一句文案是**设计如此**（本地带题号「第 27 题还没有答案。」，
 *    门禁写「这道题还有答案没有填写。」），文案不同**不能**当成两条事实，
 *    否则同一道题又会各占一行（实测 14 道题白多 14 行）。
 *    这一情形的事实等价性由谓词证明：两边都用 `answerKey[slot].kind === "unresolved"`。
 *  - **同来源**：还要 `userMessage` 也相同才算同一条。文案不同就是两条事实，
 *    两条都留着 —— 宁可多显示一条，也不能把一条阻断问题藏起来。
 *
 * 注意这**仍然是保守的**：真正的身份由后端给出（`factId`，见下）。
 * 后端把 `issueId` 改成带判别性 slug 之后，同一目标上的两个不同事实已有不同的 id，
 * 这里就把它用起来；但**只当作「更强的不同判据」**，见 `sameFact()` 的注释。
 */
function sameFact(a: ActionableIssueV1, b: ActionableIssueV1): boolean {
  if (issueRootCause(a) !== issueRootCause(b)) return false;
  if (a.targetId !== b.targetId) return false;
  if (a.source !== b.source) return true;
  // 后端稳定事实 id（`factId`）在这里是**单向**判据：
  //   两个 id 不同 ⇒ 一定是两条事实，哪怕文案逐字相同；
  //   id 相同或某一侧缺失 ⇒ **不作结论**，继续按文案比较。
  //
  // 为什么必须是单向的：`factId` 的可靠性依赖后端当前这套 `issueId` 编码。
  // 旧载荷里 `issueId = phase4-{code}-{target}` 会**撞键**（group-2 的两条事实同 id），
  // 若拿它去「断言同一」，就会把两条不同事实合成一条 —— 正是 F-R9-13 那个反方向错误。
  // 单向使用保证了：无论拿到的是新载荷、旧载荷、还是根本没有 id 的
  // 直接 blocker（如 `ANSWER_MISSING`），**合并都不会比上一轮更多**。
  if (a.factId && b.factId && a.factId !== b.factId) return false;
  return a.userMessage === b.userMessage;
}

function sortIssues(issues: ActionableIssueV1[]): ActionableIssueV1[] {
  return issues.sort((a, b) => (a.severity === b.severity ? 0 : a.severity === "blocker" ? -1 : 1));
}

/**
 * 把发布门禁的 blocker 合并进编辑器问题列表（计划 §8.5 / findings「问题列表 ≠ 发布门禁」）。
 *
 * 编辑器此前只有纯前端近似检查，于是会出现「界面显示 0 个问题、点发布却失败」且文案
 * 不可操作。这里把后端 `check_publish_preflight` 的 blocker 按同一形状并入，用户看到的
 * 就是发布真正会拦下的东西；文案直接用后端准备好的 `userMessage`。
 *
 * **两条硬要求同时成立**（任务书）：同一问题不重复显示，不同问题不被隐藏。
 * 所以去重不能用「门禁 code + 目标」（门禁用 `ISSUE_UNRESOLVED` 一个 code 承载所有质量码，
 * 会把 `SIGNIFICANT_REGION_UNASSIGNED` 和 `RUNTIME_COMPILER_FAILED` 合成一条），
 * 也不能只用「根因 + 目标」（group-2 的两条不同事实会撞），而是走 `sameFact()`。
 */
export function mergePublishGateIssues(
  local: readonly ActionableIssueV1[],
  gate: PublishGateResult | undefined
): ActionableIssueV1[] {
  // 浅拷贝一层再改：下面会把「与门禁同一事实」的本地行升级成 blocker，
  // 不能就地改调用方传进来的对象（`localIssues` 是 useMemo 的产物，改了会串状态）。
  const merged: ActionableIssueV1[] = local.map((issue) => ({
    ...issue,
    source: "local" as const,
    rootCause: issueRootCause(issue)
  }));
  if (!gate) return sortIssues(merged);
  const blockers = gate.blockers ?? [];
  // 已经由「带具体目标」的记录表达过的根因。泛化的 `QUALITY_HARD_FAILURE` 若命中这里就是纯重复：
  // 它只说「有硬失败」，而用户要处理的东西已经逐条列在下面了。
  const rootCausesWithTarget = new Set(
    blockers
      .filter((blocker) => !GENERIC_GATE_CODES.has(blocker.code) && (blocker.targetId ?? "") !== "")
      .map((blocker) => rootCauseOf(blocker))
  );
  for (const blocker of blockers) {
    const rootCause = rootCauseOf(blocker);
    if (GENERIC_GATE_CODES.has(blocker.code) && rootCausesWithTarget.has(rootCause)) continue;
    const targetId = blocker.targetId ?? "";
    const factId = factIdOf(blocker);
    const row: ActionableIssueV1 = {
      // 行 id 必须**逐行唯一** —— 它就是 React 的 `key`（`ExamWorkspacePage` 的 `<li key={issue.issueId}>`）。
      // 上一轮拼的 `gate:{根因}:{目标}:{code}` 在「同 code + 同目标的两条不同事实」上会撞键
      // （group-2 那两条正是这样），两条事实拿到同一个 key 会让 React 复用/丢弃 DOM。
      // 后端现在给了稳定事实 id，就用它当 key；没有 id 时退回文案，仍然逐行唯一。
      issueId: factId
        ? `gate:${factId}`
        : `gate:${rootCause}:${targetId}:${blocker.code}:${blocker.userMessage ?? ""}`,
      targetId: targetId || blocker.code,
      severity: "blocker",
      code: blocker.code,
      userMessage: blocker.userMessage || "发布前还有必须处理的问题。",
      source: "gate",
      rootCause,
      factId
    };
    const existing = merged.find((issue) => sameFact(issue, row));
    if (existing) {
      // 本地已经有同一事实的行。门禁的**级别**更高（发布确实被它拦下），
      // 但本地的**文案**更具体（带题号：`第 27 题还没有答案。` 对 `这道题还有答案没有填写。`）。
      // 所以保留本地文案、把级别提到 blocker，而不是再插一条泛化行。
      if (existing.severity !== "blocker") {
        merged[merged.indexOf(existing)] = { ...existing, severity: "blocker" };
      }
      continue;
    }
    merged.push(row);
  }
  const seenWarnings = new Set(merged.map((issue) => `${issue.code}:`));
  for (const warning of gate.warnings ?? []) {
    const key = `${warning.code}:`;
    if (seenWarnings.has(key)) continue;
    seenWarnings.add(key);
    merged.push({
      issueId: `gate-warning:${key}`,
      targetId: warning.code,
      severity: "warning",
      code: warning.code,
      userMessage: warning.message || "发布前有一项提示需要确认。",
      source: "gate",
      rootCause: warning.code
    });
  }
  return sortIssues(merged);
}

const CHOICE_TASK_TYPES = new Set(["single_choice", "multiple_choice", "true_false_not_given", "yes_no_not_given"]);
const MATCHING_TASK_TYPES = new Set([
  "matching_information",
  "matching_headings",
  "matching_features",
  "matching_sentence_endings",
  "classification"
]);

function textOf(nodes: ContentNodeV2[] | undefined): string {
  if (!nodes?.length) return "";
  return nodes
    .map((node) => {
      if (node.type === "text") return node.text;
      if ("children" in node) return textOf(node.children);
      if ("items" in node) return node.items.map((item) => textOf(item.children)).join(" ");
      if ("rows" in node) return node.rows.map((row) => row.cells.map((cell) => textOf(cell.children)).join(" ")).join(" ");
      if ("steps" in node) return node.steps.map((step) => textOf(step.children)).join(" ");
      return "";
    })
    .join("");
}

/** 一个答案位在题面上的显示编号，用于「第 N 题」文案。 */
function displayLabel(ds: IeltsAuthoringIRV2, slotId: string): string {
  const slot = ds.answerSlots[slotId];
  if (!slot) return slotId;
  return slot.displayLabel || String(slot.questionNumber);
}

function questionsLabel(ds: IeltsAuthoringIRV2, group: ResponseGroupV2): string {
  const labels = group.slotIds.map((slotId) => displayLabel(ds, slotId)).filter(Boolean);
  if (!labels.length) return "这一题";
  if (labels.length === 1) return `第 ${labels[0]} 题`;
  return `第 ${labels.join("、")} 题`;
}

function checkChoiceGroup(ds: IeltsAuthoringIRV2, task: TaskGroupV2, group: ResponseGroupV2, issues: ActionableIssueV1[]): void {
  const options = group.options?.length ? group.options : task.optionBank?.options ?? [];
  const label = questionsLabel(ds, group);
  if (!options.length) {
    issues.push({
      issueId: `${group.responseGroupId}:options-missing`,
      targetId: group.responseGroupId,
      severity: "blocker",
      code: "OPTION_RUN_INCOMPLETE",
      userMessage: `${label}没有识别到选项。`
    });
    return;
  }
  for (const option of options) {
    if (textOf(option.content).trim()) continue;
    issues.push({
      issueId: `${option.optionId}:text-missing`,
      targetId: option.optionId,
      severity: "blocker",
      code: "OPTION_TEXT_MISSING",
      userMessage: `${label} ${option.label} 选项没有正文。`
    });
  }
}

function checkMatchingTask(ds: IeltsAuthoringIRV2, task: TaskGroupV2, issues: ActionableIssueV1[]): void {
  const bank = task.optionBank?.options ?? [];
  if (!bank.length) {
    issues.push({
      issueId: `${task.taskId}:bank-missing`,
      targetId: task.taskId,
      severity: "blocker",
      code: "SHARED_OPTION_BANK_MISSING",
      userMessage: "这组匹配题没有识别到共享选项列表。"
    });
  }
}

/** 对当前 Canonical DS 做硬闭包检查。返回顺序按题号，blocker 在前。 */
export function deriveActionableIssues(ds: IeltsAuthoringIRV2 | undefined): ActionableIssueV1[] {
  if (!ds) return [];
  const issues: ActionableIssueV1[] = [];

  for (const task of ds.taskGroups) {
    if (MATCHING_TASK_TYPES.has(task.taskType)) checkMatchingTask(ds, task, issues);

    for (const group of task.responseGroups) {
      const label = questionsLabel(ds, group);

      // 题干闭包：选择题与匹配题的每一项都必须有非空题干（计划 §2.2「任一简单题 prompt == empty 必须产生 blocker」）。
      const needsPrompt = CHOICE_TASK_TYPES.has(task.taskType) || MATCHING_TASK_TYPES.has(task.taskType);
      if (needsPrompt && !textOf(group.prompt).trim()) {
        issues.push({
          issueId: `${group.responseGroupId}:prompt-missing`,
          targetId: group.responseGroupId,
          severity: "blocker",
          code: "QUESTION_PROMPT_MISSING",
          userMessage: `${label}的题干是空的，需要补上。`
        });
      }

      if (CHOICE_TASK_TYPES.has(task.taskType) || group.kind === "choice") {
        checkChoiceGroup(ds, task, group, issues);
      }

      for (const slotId of group.slotIds) {
        const slot = ds.answerSlots[slotId];
        if (slot && slot.participation !== "scoring") continue;
        const answer = ds.answerKey[slotId];
        if (!answer || answer.kind === "unresolved") {
          issues.push({
            issueId: `${slotId}:answer-missing`,
            targetId: slotId,
            severity: "warning",
            code: answer?.kind === "unresolved" ? "ANSWER_UNRESOLVED" : "ANSWER_MISSING",
            userMessage: `第 ${displayLabel(ds, slotId)} 题还没有答案。`
          });
          continue;
        }
        const empty = answer.kind === "text"
          ? !answer.values.some((value) => value.trim())
          : answer.kind === "option" && !answer.labels.length;
        if (empty) {
          issues.push({
            issueId: `${slotId}:answer-empty`,
            targetId: slotId,
            severity: "warning",
            code: "ANSWER_MISSING",
            userMessage: `第 ${displayLabel(ds, slotId)} 题还没有答案。`
          });
        }
      }
    }
  }

  return sortIssues(issues);
}

export function blockerCount(issues: readonly ActionableIssueV1[]): number {
  return issues.filter((issue) => issue.severity === "blocker").length;
}
