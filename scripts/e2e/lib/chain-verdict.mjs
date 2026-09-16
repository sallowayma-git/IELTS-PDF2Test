// 完整链的判定：把「这次运行到底算不算通过」变成可测的纯函数。
//
// 为什么必须单独抽出来（本文件存在的唯一理由）：
// 旧版判定是
//
//     const failed = report.steps.filter((s) => s.status === "failed");
//     report.verdict = failed.length || report.steps.length === 0 ? "failed" : "passed";
//
// `blocked` 步骤只被写进 `summary.blocked`，**完全不参与 verdict**。于是
// 「发布被质量门禁拦下、manifest.js 根本没生成、学生端没有任何产物」的运行，
// 照样 `verdict=passed`、`process.exit(0)`。这就是**假绿**：报告看起来通过，
// 而完整链一步都没走完。
//
// 判定规则（与任务书「本轮退出标准」一致）：
//   1. 必需步骤缺失（未执行）→ incomplete，不是通过。
//   2. 必需步骤 failed        → failed。
//   3. 必需步骤 blocked       → blocked，**不得**计为通过（退出码非 0）。
//   4. 全部 passed 但发布产物不完整（manifest / 题目 JS / 资源清单缺失）→ failed。
//   5. 门禁正确拦下坏题，可以作为**独立负例**通过（`passed-negative-case`），
//      但它不是成功发布链，名字必须区分开。
//   6. 编辑/预览专项通过 → `passed-specialty`，**不得**沿用完整链的成功名。
//
// 判定必须建立在「recorder.steps 已经赋值给 report.steps 之后」的完整数组上；
// 在 `try` 块里提前算会把空数组算成通过。

/** 发布步骤名。完整链里它是唯一能证明「产物真的落盘」的步骤。 */
export const PUBLISH_STEP = "publish-via-workspace-button";

/** 完整链必需步骤：缺任何一步都不算「完整链通过」。 */
export const FULL_CHAIN_REQUIRED_STEPS = [
  "library-page-loads",
  "import-pdf-via-folder-hook",
  "background-pipeline-reaches-stable-stage",
  "workspace-opens",
  "edit-body-text-and-save",
  "edit-survives-reopen",
  "student-preview-renders",
  "student-preview-answering-isolated",
  "edit-after-preview-survives-reopen",
  PUBLISH_STEP,
  "recognition-panel-reads-real-backend",
  "preview-and-gate-agree"
];

/**
 * 编辑/预览专项必需步骤：**不含发布**。
 * 用途是「我只想验编辑与预览」时能有一条诚实的绿色，而这条绿色不许叫「完整链通过」。
 */
export const EDIT_PREVIEW_REQUIRED_STEPS = [
  "library-page-loads",
  "import-pdf-via-folder-hook",
  "background-pipeline-reaches-stable-stage",
  "workspace-opens",
  "edit-body-text-and-save",
  "edit-survives-reopen",
  "student-preview-renders",
  "student-preview-answering-isolated",
  "edit-after-preview-survives-reopen",
  "preview-and-gate-agree"
];

/**
 * 退出码。任何非「通过」的判定都必须非 0，CI 才能拦住假绿。
 * 3 是既有约定（CANNOT-RUN，环境/构建不满足），沿用。
 */
export const EXIT_CODES = {
  "passed": 0,
  "passed-negative-case": 0,
  "passed-specialty": 0,
  "failed": 1,
  "incomplete": 2,
  "cannot-run": 3,
  "blocked": 4,
  "not-executable": 5
};

/** 场景状态。`not-executable` 是**独立的第三态**，不是失败，也绝不是通过。 */
export const SCENARIO_STATUS = {
  PASSED: "passed",
  FAILED: "failed",
  NOT_EXECUTABLE: "not-executable"
};

/**
 * 步骤状态词表。**只有 `passed` 表示「这一步做成了」**。
 *
 * 任务书要求：「完整链每个必需步骤必须明确 passed；未知、skipped、not-executable
 * 均不得通过」。所以判定不能只查 `failed`/`blocked`/缺失三样 —— 一个状态是
 * `skipped`、`not-executable`、`pending`，或者干脆拼错的状态，既不算缺失、也不算失败，
 * 旧写法会**直接漏到最后的 `passed`**。这里显式列出「有结论」的三个状态，
 * 其余一律按「没有明确结论」处理（不做白名单枚举，免得将来冒出新状态又漏过去）。
 */
export const STEP_STATUS = {
  PASSED: "passed",
  FAILED: "failed",
  BLOCKED: "blocked"
};

/** 有明确结论的步骤状态。不在此列的一律不得当作通过。 */
const CONCLUSIVE_STEP_STATUSES = new Set(Object.values(STEP_STATUS));

/**
 * 场景级判定（用于「真实候选按钮流程」这类由多个独立场景组成的验收）。
 *
 * 为什么需要它：任务书明确要求「无候选时，场景记为无法执行，**不能跳过后判 passed**」。
 * 如果沿用「没有 failed 就算 passed」，一个根本没有候选可点的运行会得到绿色——
 * 那和 F-R8-1 的假绿是同一类错误，只是换了一层。
 *
 * 规则：
 *   - 有场景 failed → failed（1）
 *   - 有场景 not-executable → not-executable（5）：**本次验收没做成**，不是通过
 *   - 场景列表为空 → incomplete（2）
 *   - 全部 passed → passed（0）
 */
export function computeScenarioVerdict({ scenarios = [] } = {}) {
  const names = (status) => scenarios.filter((s) => s.status === status).map((s) => s.name);
  const passed = names(SCENARIO_STATUS.PASSED);
  const failed = names(SCENARIO_STATUS.FAILED);
  const notExecutable = names(SCENARIO_STATUS.NOT_EXECUTABLE);
  const unknown = scenarios.filter((s) => !Object.values(SCENARIO_STATUS).includes(s.status)).map((s) => s.name);
  const facts = { passed, failed, notExecutable, unknown, total: scenarios.length };
  const finish = (verdict, reason) => ({ ...facts, verdict, exitCode: EXIT_CODES[verdict], reason });

  if (scenarios.length === 0) return finish("incomplete", "没有任何场景被执行");
  if (unknown.length) return finish("failed", `场景状态非法（必须显式声明 passed/failed/not-executable）：${unknown.join(", ")}`);
  if (failed.length) return finish("failed", `场景失败：${failed.join(", ")}`);
  if (notExecutable.length) {
    return finish(
      "not-executable",
      `以下场景本次**无法执行**（前提不成立，例如没有真实候选项）：${notExecutable.join(", ")}。这不等于通过。`
    );
  }
  return finish("passed", `全部 ${passed.length} 个场景通过`);
}

/**
 * 发布产物完整性：步骤自称成功时，产物必须真的落盘。
 *
 * 「步骤 passed」与「产物存在」是两件事：前者只说明 UI 没报错。
 * 任务书要求 manifest / 题目 JS / 资源三者都要核对，所以这里逐项查。
 */
export function evaluatePublication({ publishDetail, publicationFacts } = {}) {
  const failures = [];
  if (!publishDetail) {
    failures.push("报告里没有发布步骤的详情，无法确认产物");
    return failures;
  }
  if (publishDetail.manifestExists !== true) {
    failures.push("manifest.js 未落盘（manifestExists !== true）");
  }
  const preflightPassed = publishDetail.preflight?.passed;
  if (preflightPassed !== true) {
    failures.push(`发布预检未通过（preflight.passed=${String(preflightPassed)}）`);
  }
  const facts = publicationFacts ?? {};
  if (!Array.isArray(facts.scriptFiles) || facts.scriptFiles.length === 0) {
    failures.push("发布目录里没有题目 JS（v2-p*.js）");
  }
  if (facts.resourceManifestExists !== true) {
    failures.push("发布目录里没有资源清单（resources/<examId>/asset-manifest.json）");
  }
  return failures;
}

/**
 * 计算完整链（或专项）判定。
 *
 * @param {object} input
 * @param {Array<{name: string, status: string}>} input.steps 已执行完的步骤
 * @param {"full-chain"|"edit-preview-specialty"} [input.scope]
 * @param {string[]} [input.requiredSteps] 默认由 scope 推导
 * @param {boolean} [input.expectBlocked] 负例模式：期望门禁正确拦下坏题
 * @param {boolean} [input.cannotRun] 环境/构建不满足，根本没跑起来
 * @param {string[]} [input.publicationFailures] evaluatePublication 的结果
 */
export function computeChainVerdict({
  steps = [],
  scope = "full-chain",
  requiredSteps,
  expectBlocked = false,
  cannotRun = false,
  publicationFailures = []
} = {}) {
  const required = requiredSteps ?? (scope === "edit-preview-specialty" ? EDIT_PREVIEW_REQUIRED_STEPS : FULL_CHAIN_REQUIRED_STEPS);
  const byName = new Map();
  for (const step of steps) byName.set(step.name, step);

  const missing = required.filter((name) => !byName.has(name));
  // 失败**不**按 required 过滤：任何一步抛错都是真实错误，放过它就是把假绿的洞换个位置。
  const failed = steps.filter((step) => step.status === "failed").map((step) => step.name);
  // blocked 只在必需步骤上判定：专项范围（如编辑/预览）里「发布被门禁拦下」本来就在范围之外。
  const blocked = required.filter((name) => byName.get(name)?.status === "blocked");
  const passed = required.filter((name) => byName.get(name)?.status === "passed");
  // 必需步骤里状态「既不是 passed、也不是明确 failed/blocked」的那些 ——
  // 未知值 / `skipped` / `not-executable` / 拼错的状态。它们既不缺、也没失败，
  // 旧判定会直接漏到最后的 `passed`（任务书明确点出的那个洞）。
  const notOutcome = required
    .map((name) => ({ name, status: byName.get(name)?.status }))
    .filter((step) => !CONCLUSIVE_STEP_STATUSES.has(step.status));

  const facts = {
    scope,
    requiredSteps: [...required],
    passed,
    failed,
    blocked,
    missing,
    notOutcome,
    publicationFailures: [...publicationFailures]
  };
  const finish = (verdict, reason) => ({ ...facts, verdict, exitCode: EXIT_CODES[verdict], reason });

  if (cannotRun) return finish("cannot-run", "环境或构建不满足运行条件，见 fatal");
  if (steps.length === 0) return finish("incomplete", "没有任何步骤被执行");
  if (failed.length) return finish("failed", `必需步骤失败：${failed.join(", ")}`);
  // 缺步骤 = 没跑完。这一步必须排在 blocked 之前判，否则「漏跑发布」会被当成通过。
  if (missing.length) return finish("incomplete", `必需步骤缺失（未执行）：${missing.join(", ")}`);

  // 「每个必需步骤必须明确 passed」：状态没有明确结论的一律不得通过。
  // 必须排在 blocked / 负例之前判 —— 否则「发布被拦 + 另一步 skipped」会走
  // `passed-negative-case`，skipped 那步照样溜过去。
  if (notOutcome.length) {
    const described = notOutcome.map((step) => `${step.name}(${String(step.status)})`).join(", ");
    if (notOutcome.every((step) => step.status === "not-executable")) {
      return finish("not-executable", `必需步骤本次无法执行：${described}。这不是通过。`);
    }
    return finish(
      "incomplete",
      `必需步骤没有明确的执行结论（状态必须是 passed/failed/blocked）：${described}`
    );
  }

  if (blocked.length) {
    const publishBlocked = blocked.includes(PUBLISH_STEP);
    const otherBlocked = blocked.filter((name) => name !== PUBLISH_STEP);
    if (expectBlocked && publishBlocked && otherBlocked.length === 0) {
      return finish(
        "passed-negative-case",
        "负例通过：门禁正确拦下坏题（发布被阻断），其余必需步骤全部通过。**这不代表发布成功**。"
      );
    }
    if (expectBlocked && !publishBlocked) {
      return finish("failed", `负例模式期望发布被门禁拦下，但实际被阻断的是：${blocked.join(", ")}`);
    }
    return finish("blocked", `必需步骤被质量门禁阻断：${blocked.join(", ")}。发布未发生，不得计为通过。`);
  }

  // 走到这里说明没有任何必需步骤被阻断。负例模式的**前提**因此不成立：
  // 它本来是来证明「门禁会拦下坏题」的，结果题稿发布成功了。
  // 这种情况不能算负例通过，否则负例模式会变成一条永远绿色的通道。
  if (expectBlocked) {
    return finish("failed", "负例前提不成立：期望门禁拦下坏题，但没有任何必需步骤被阻断（发布成功了）");
  }

  if (publicationFailures.length) {
    return finish("failed", `步骤自称成功，但发布产物不完整：${publicationFailures.join("；")}`);
  }

  // 到这里还没返回，说明每个必需步骤都有明确结论且都不是 failed/blocked。
  // 最后再自检一次「通过数 = 必需数」：上面任何一条规则将来被改坏，这里都会拦住，
  // 而不是让一个没被覆盖的状态静默变成通过。
  if (passed.length !== required.length) {
    return finish(
      "incomplete",
      `内部不一致：必需步骤通过 ${passed.length} 个，但必需步骤共 ${required.length} 个`
    );
  }

  if (scope === "edit-preview-specialty") {
    return finish("passed-specialty", "编辑/预览专项通过（范围不含发布，不代表完整发布链通过）");
  }
  return finish("passed", "完整链通过：导入 → 编辑 → 预览 → 发布 → 产物落盘");
}
