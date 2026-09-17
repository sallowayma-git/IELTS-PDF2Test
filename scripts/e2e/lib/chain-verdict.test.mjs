import { describe, expect, it } from "vitest";
import {
  computeChainVerdict,
  computeScenarioVerdict,
  describePreflightDisagreement,
  EDIT_PREVIEW_REQUIRED_STEPS,
  evaluatePublication,
  EXIT_CODES,
  FULL_CHAIN_REQUIRED_STEPS,
  PUBLISH_STEP,
  SCENARIO_STATUS
} from "./chain-verdict.mjs";

// 证据层级：pure unit。断言的是**验收判定本身**，不是产品行为。
//
// 背景：旧判定只看 `failed`，`blocked` 步骤不参与 verdict，
// 于是「发布被门禁拦下、manifest 没生成」照样 verdict=passed、退出码 0（假绿）。
// 下面每条用例都钉住「假绿不再可能」的一个入口。

/** 所有必需步骤 passed，可对个别步骤覆盖状态。 */
function stepsWith(overrides = {}) {
  return FULL_CHAIN_REQUIRED_STEPS.map((name) => ({ name, status: overrides[name] ?? "passed" }));
}

/** 产物齐全的发布详情（用于「步骤通过」路径）。 */
function completePublication() {
  return {
    publishDetail: { manifestExists: true, preflight: { passed: true } },
    publicationFacts: { runtimeScripts: ["releases/cb5e39b4/early-approaches.js"], resourceManifestExists: true }
  };
}

function publicationFailuresFor(input) {
  return evaluatePublication(input);
}

describe("完整链判定：发布未发生不得报通过", () => {
  it("发布被门禁 blocked → 不是 passed，退出码非 0", () => {
    const r = computeChainVerdict({ steps: stepsWith({ [PUBLISH_STEP]: "blocked" }) });
    expect(r.verdict).toBe("blocked");
    expect(r.verdict).not.toBe("passed");
    expect(r.exitCode).not.toBe(0);
    expect(r.blocked).toEqual([PUBLISH_STEP]);
  });

  it("发布步骤整条缺失（未执行）→ incomplete，不是通过", () => {
    const steps = stepsWith().filter((s) => s.name !== PUBLISH_STEP);
    const r = computeChainVerdict({ steps });
    expect(r.verdict).toBe("incomplete");
    expect(r.verdict).not.toBe("passed");
    expect(r.missing).toEqual([PUBLISH_STEP]);
    expect(r.exitCode).not.toBe(0);
  });

  it("manifest 缺失 → 即使步骤自称成功也不通过", () => {
    const failures = publicationFailuresFor({
      publishDetail: { manifestExists: false, preflight: { passed: true } },
      publicationFacts: { runtimeScripts: ["releases/cb5e39b4/early-approaches.js"], resourceManifestExists: true }
    });
    expect(failures.join()).toContain("manifest.js 未落盘");
    const r = computeChainVerdict({ steps: stepsWith(), publicationFailures: failures });
    expect(r.verdict).toBe("failed");
    expect(r.exitCode).not.toBe(0);
  });

  it("题目运行时脚本与资源清单缺失同样不通过", () => {
    const failures = publicationFailuresFor({
      publishDetail: { manifestExists: true, preflight: { passed: true } },
      publicationFacts: { runtimeScripts: [], resourceManifestExists: false }
    });
    expect(failures).toHaveLength(2);
    const r = computeChainVerdict({ steps: stepsWith(), publicationFailures: failures });
    expect(r.verdict).toBe("failed");
  });

  it("题目脚本的判定必须按 manifest 的 entry.script，而不是包根 v2-p*.js", () => {
    // R14 实测（F-R14-6）：真实成功的发布在包根只有 manifest.js，
    // 运行时脚本在 releases/<batchId>/<examId>.js。旧规则会把它判成「没有题目 JS」。
    const failures = publicationFailuresFor({
      publishDetail: { manifestExists: true, preflight: { passed: true } },
      publicationFacts: {
        runtimeScripts: ["releases/1237a710/early-approaches.js"],
        resourceManifestExists: true,
        // 包根没有任何 v2-p*.js —— 旧规则看的就是这个字段
        scriptFiles: []
      }
    });
    expect(failures).toEqual([]);
  });

  it("预检与发布不一致：**作为独立观察上报**，不用「产物缺失」来描述成功发布", () => {
    const publishDetail = {
      manifestExists: true,
      preflight: { passed: false, blockers: [{ code: "QUALITY_NOT_READY" }, { code: "QUALITY_NOT_READY" }] }
    };
    // 产物齐全 → failures 为空：发布**确实发生了**，不能因为预检口径不一致就说没产物。
    expect(publicationFailuresFor({ publishDetail, publicationFacts: completePublication().publicationFacts })).toEqual([]);
    const note = describePreflightDisagreement(publishDetail);
    expect(note).toContain("passed=false");
    expect(note).toContain("QUALITY_NOT_READY");
    // 去重后只报一次 code。
    expect(note.match(/QUALITY_NOT_READY/g)).toHaveLength(1);
    // 预检通过、或根本没有预检读数时，不产生这条观察。
    expect(describePreflightDisagreement({ manifestExists: true, preflight: { passed: true } })).toBeNull();
    expect(describePreflightDisagreement({ manifestExists: true })).toBeNull();
    // 发布没成功时也不该报「发布已成功」。
    expect(describePreflightDisagreement({ manifestExists: false, preflight: { passed: false } })).toBeNull();
  });

  it("没有发布步骤详情时不当作产物齐全", () => {
    expect(publicationFailuresFor({ publishDetail: null }).join()).toContain("没有发布步骤的详情");
  });

  it("全部通过且产物齐全 → passed，退出码 0", () => {
    const r = computeChainVerdict({ steps: stepsWith(), publicationFailures: publicationFailuresFor(completePublication()) });
    expect(r.verdict).toBe("passed");
    expect(r.exitCode).toBe(0);
  });
});

describe("负例：门禁正确拦坏题是独立通过，不得冒充发布成功", () => {
  it("expectBlocked + 只有发布被拦 + 其余全过 → passed-negative-case，且不等于 passed", () => {
    const r = computeChainVerdict({ steps: stepsWith({ [PUBLISH_STEP]: "blocked" }), expectBlocked: true });
    expect(r.verdict).toBe("passed-negative-case");
    expect(r.verdict).not.toBe("passed");
    expect(r.exitCode).toBe(0);
    expect(r.reason).toContain("不代表发布成功");
  });

  it("expectBlocked 但发布成功 → 负例前提不成立，判 failed", () => {
    const r = computeChainVerdict({ steps: stepsWith(), expectBlocked: true, publicationFailures: publicationFailuresFor(completePublication()) });
    expect(r.verdict).toBe("failed");
    expect(r.reason).toContain("负例前提不成立");
  });

  it("expectBlocked 但被拦的是别的必需步骤 → failed（拦错了地方）", () => {
    const r = computeChainVerdict({ steps: stepsWith({ "workspace-opens": "blocked" }), expectBlocked: true });
    expect(r.verdict).toBe("failed");
    expect(r.reason).toContain("负例模式期望发布被门禁拦下");
  });
});

describe("编辑/预览专项：不得沿用完整链的成功名", () => {
  it("专项范围不含发布 → passed-specialty，且不是 passed", () => {
    const steps = EDIT_PREVIEW_REQUIRED_STEPS.map((name) => ({ name, status: "passed" }));
    const r = computeChainVerdict({ steps, scope: "edit-preview-specialty" });
    expect(r.verdict).toBe("passed-specialty");
    expect(r.verdict).not.toBe("passed");
    expect(r.exitCode).toBe(0);
  });

  it("专项范围里发布被门禁拦下不算失败（发布本就不在范围内）", () => {
    const steps = [
      ...EDIT_PREVIEW_REQUIRED_STEPS.map((name) => ({ name, status: "passed" })),
      { name: PUBLISH_STEP, status: "blocked" }
    ];
    const r = computeChainVerdict({ steps, scope: "edit-preview-specialty" });
    expect(r.verdict).toBe("passed-specialty");
  });

  it("专项范围内必需步骤缺失仍然 incomplete", () => {
    const steps = EDIT_PREVIEW_REQUIRED_STEPS.filter((n) => n !== "student-preview-renders").map((name) => ({ name, status: "passed" }));
    const r = computeChainVerdict({ steps, scope: "edit-preview-specialty" });
    expect(r.verdict).toBe("incomplete");
  });
});

describe("判定优先级与边界", () => {
  it("failed 优先于 blocked", () => {
    const r = computeChainVerdict({ steps: stepsWith({ "workspace-opens": "failed", [PUBLISH_STEP]: "blocked" }) });
    expect(r.verdict).toBe("failed");
  });

  it("非必需步骤失败也不能被忽略", () => {
    const steps = [...stepsWith(), { name: "some-optional-step", status: "failed" }];
    const r = computeChainVerdict({ steps, publicationFailures: publicationFailuresFor(completePublication()) });
    expect(r.verdict).toBe("failed");
    expect(r.failed).toContain("some-optional-step");
  });

  it("没有任何步骤执行 → incomplete", () => {
    expect(computeChainVerdict({ steps: [] }).verdict).toBe("incomplete");
  });

  it("cannotRun 优先于一切，退出码 3", () => {
    const r = computeChainVerdict({ steps: [], cannotRun: true });
    expect(r.verdict).toBe("cannot-run");
    expect(r.exitCode).toBe(3);
  });

  it("只有通过类判定的退出码是 0", () => {
    const passing = ["passed", "passed-negative-case", "passed-specialty"];
    for (const [verdict, code] of Object.entries(EXIT_CODES)) {
      if (passing.includes(verdict)) expect(code, verdict).toBe(0);
      else expect(code, verdict).not.toBe(0);
    }
  });

  it("必需步骤清单本身包含发布步骤（防止清单被误删）", () => {
    expect(FULL_CHAIN_REQUIRED_STEPS).toContain(PUBLISH_STEP);
    expect(EDIT_PREVIEW_REQUIRED_STEPS).not.toContain(PUBLISH_STEP);
  });
});

// 任务书原文：「完整链每个必需步骤必须明确 passed；未知、skipped、not-executable
// 均不得通过。」下面每条钉住一个会**漏到 passed** 的状态入口。
//
// 漏洞形状：`missing` 只查「步骤名不在数组里」，`failed` 只查 `status === "failed"`，
// `blocked` 只查 `status === "blocked"`。于是状态是 `skipped` / `not-executable` /
// 未知字符串的必需步骤，三样都不占，**直接走到最后的 `passed`**。
describe("必需步骤必须明确 passed：skipped / not-executable / 未知状态都不得通过", () => {
  const complete = () => publicationFailuresFor(completePublication());

  it("必需步骤是 skipped → 不通过（incomplete），退出码非 0", () => {
    const r = computeChainVerdict({
      steps: stepsWith({ "edit-survives-reopen": "skipped" }),
      publicationFailures: complete()
    });

    expect(r.verdict).not.toBe("passed");
    expect(r.verdict).toBe("incomplete");
    expect(r.exitCode).not.toBe(0);
    expect(r.notOutcome.map((s) => s.name)).toContain("edit-survives-reopen");
  });

  it("必需步骤是 not-executable → not-executable，不是通过", () => {
    const r = computeChainVerdict({
      steps: stepsWith({ [PUBLISH_STEP]: "not-executable" }),
      publicationFailures: complete()
    });

    expect(r.verdict).toBe("not-executable");
    expect(r.exitCode).toBe(5);
  });

  it("必需步骤状态是未知字符串（拼错）→ 不通过", () => {
    const r = computeChainVerdict({
      steps: stepsWith({ "workspace-opens": "suceeded" }),
      publicationFailures: complete()
    });

    expect(r.verdict).not.toBe("passed");
    expect(r.verdict).toBe("incomplete");
  });

  it("必需步骤缺 status 字段（undefined）→ 不通过", () => {
    const steps = FULL_CHAIN_REQUIRED_STEPS.map((name) => (name === "workspace-opens" ? { name } : { name, status: "passed" }));
    const r = computeChainVerdict({ steps, publicationFailures: complete() });

    expect(r.verdict).not.toBe("passed");
    expect(r.verdict).toBe("incomplete");
  });

  it("发布被拦 + 另一步 skipped → **不得**走负例通过（负例也不许漏状态）", () => {
    const r = computeChainVerdict({
      steps: stepsWith({ [PUBLISH_STEP]: "blocked", "edit-survives-reopen": "skipped" }),
      expectBlocked: true
    });

    expect(r.verdict).not.toBe("passed-negative-case");
    expect(r.verdict).toBe("incomplete");
    expect(r.exitCode).not.toBe(0);
  });

  it("全部明确 passed 且产物齐全 → 仍然是 passed（没有把正常路径改坏）", () => {
    const r = computeChainVerdict({ steps: stepsWith(), publicationFailures: complete() });

    expect(r.verdict).toBe("passed");
    expect(r.notOutcome).toEqual([]);
    expect(r.passed).toHaveLength(FULL_CHAIN_REQUIRED_STEPS.length);
  });

  it("只有通过类判定的退出码是 0（not-executable=5 也在非 0 之列）", () => {
    expect(EXIT_CODES["not-executable"]).not.toBe(0);
    expect(EXIT_CODES.incomplete).not.toBe(0);
  });
});

// 任务书原文：「无候选时，场景记为无法执行，不能跳过后判 passed」。
describe("场景级判定：无法执行不是通过", () => {
  const s = (name, status) => ({ name, status });

  it("全部通过 → passed，退出码 0", () => {
    const r = computeScenarioVerdict({ scenarios: [s("accept", "passed"), s("reject", "passed")] });
    expect(r.verdict).toBe("passed");
    expect(r.exitCode).toBe(0);
  });

  it("无候选项导致场景无法执行 → not-executable，**不是 passed**，退出码非 0", () => {
    const r = computeScenarioVerdict({
      scenarios: [
        s("accept-suggestion", SCENARIO_STATUS.NOT_EXECUTABLE),
        s("reject-suggestion", SCENARIO_STATUS.NOT_EXECUTABLE)
      ]
    });
    expect(r.verdict).toBe("not-executable");
    expect(r.verdict).not.toBe("passed");
    expect(r.exitCode).not.toBe(0);
    expect(r.notExecutable).toHaveLength(2);
    expect(r.reason).toContain("这不等于通过");
  });

  it("部分无法执行也整体不通过", () => {
    const r = computeScenarioVerdict({
      scenarios: [s("accept", "passed"), s("undo-auto-fix", SCENARIO_STATUS.NOT_EXECUTABLE)]
    });
    expect(r.verdict).toBe("not-executable");
  });

  it("失败优先于无法执行", () => {
    const r = computeScenarioVerdict({
      scenarios: [s("accept", "failed"), s("undo", SCENARIO_STATUS.NOT_EXECUTABLE)]
    });
    expect(r.verdict).toBe("failed");
  });

  it("场景列表为空 → incomplete", () => {
    expect(computeScenarioVerdict({ scenarios: [] }).verdict).toBe("incomplete");
  });

  it("状态非法的场景必须判失败，不能被当成通过", () => {
    const r = computeScenarioVerdict({ scenarios: [s("accept", "skipped")] });
    expect(r.verdict).toBe("failed");
    expect(r.unknown).toContain("accept");
  });

  it("not-executable 的退出码与 passed 不同且非 0", () => {
    expect(EXIT_CODES["not-executable"]).not.toBe(0);
    expect(EXIT_CODES["not-executable"]).not.toBe(EXIT_CODES.passed);
  });
});
