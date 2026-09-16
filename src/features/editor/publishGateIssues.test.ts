import { describe, expect, it } from "vitest";
import { blockerCount, factIdOf, mergePublishGateIssues, rootCauseOf, type ActionableIssueV1 } from "./actionableIssues";

// 证据层级：pure unit（计划 §19.1 层 1）。
// 断言「编辑器问题列表 = 发布门禁拦下的东西」，避免出现「界面 0 个问题、点发布却失败」。

function localIssue(code: string, targetId: string): ActionableIssueV1 {
  return {
    issueId: `${code}:${targetId}`,
    targetId,
    severity: "blocker",
    code,
    userMessage: `${code} 本地`,
    source: "local"
  };
}

describe("mergePublishGateIssues", () => {
  it("没有门禁结果时保持本地问题列表（只补上 source/rootCause 标记）", () => {
    const local = [localIssue("OPTION_TEXT_MISSING", "o1")];
    const merged = mergePublishGateIssues(local, undefined);

    expect(merged).toHaveLength(1);
    // 原有字段一个不改；只是多补了两个供界面/校验脚本用的标记。
    expect(merged[0]).toMatchObject(local[0]);
    expect(merged[0].rootCause).toBe("OPTION_TEXT_MISSING");
  });

  it("把门禁 blocker 并入列表并保留后端文案", () => {
    const merged = mergePublishGateIssues([], {
      blockers: [
        { code: "QUALITY_NOT_READY", targetId: null, userMessage: "这道题还有未确认的内容。" },
        { code: "ANSWER_MISSING", targetId: "q14", userMessage: "第 14 题还没有答案。" }
      ],
      warnings: [{ code: "BLOCKER_LIST_TRUNCATED", message: "问题较多，仅显示前 20 条。" }]
    });

    expect(merged.map((issue) => issue.code)).toEqual(["QUALITY_NOT_READY", "ANSWER_MISSING", "BLOCKER_LIST_TRUNCATED"]);
    expect(blockerCount(merged)).toBe(2);
    expect(merged.find((issue) => issue.code === "ANSWER_MISSING")?.targetId).toBe("q14");
    // 文案直接用后端准备好的 userMessage，不再被压成泛化提示。
    expect(merged.find((issue) => issue.code === "QUALITY_NOT_READY")?.userMessage).toBe("这道题还有未确认的内容。");
  });

  it("同一 code+target 的本地问题与门禁 blocker 不重复计数", () => {
    const merged = mergePublishGateIssues([localIssue("ANSWER_MISSING", "q14")], {
      blockers: [{ code: "ANSWER_MISSING", targetId: "q14", userMessage: "第 14 题还没有答案。" }]
    });

    expect(merged).toHaveLength(1);
    expect(merged[0].userMessage).toBe("ANSWER_MISSING 本地");
  });

  it("blocker 仍然排在 warning 之前", () => {
    const merged = mergePublishGateIssues([], {
      blockers: [{ code: "QUALITY_NOT_READY", targetId: null }],
      warnings: [{ code: "NOTE", message: "提示" }]
    });

    expect(merged.map((issue) => issue.severity)).toEqual(["blocker", "warning"]);
  });
});

// 证据层级：pure unit。夹具形状照抄真实运行产物
// （artifacts/e2e-cdp/run-publish-attribution-2026-09-15T23-00-06-246Z）：一份缺 14 个答案的题稿，
// 门禁对**同一个根因**同时给出 1 条泛化 QUALITY_HARD_FAILURE、14 条 ISSUE_UNRESOLVED、14 条 ANSWER_MISSING。
describe("rootCauseOf", () => {
  it("QUALITY_HARD_FAILURE 的根因是它 internal 里的质量码", () => {
    expect(rootCauseOf({ code: "QUALITY_HARD_FAILURE", internal: "ANSWER_KEY_MISSING_SLOT" })).toBe("ANSWER_KEY_MISSING_SLOT");
  });

  it("ISSUE_UNRESOLVED 的根因从 phase4-<码>-<目标> 里取码", () => {
    expect(rootCauseOf({ code: "ISSUE_UNRESOLVED", internal: "phase4-ANSWER_KEY_MISSING_SLOT-q27" })).toBe("ANSWER_KEY_MISSING_SLOT");
    expect(rootCauseOf({ code: "ISSUE_UNRESOLVED", internal: "phase4-RUNTIME_COMPILER_FAILED-document" })).toBe("RUNTIME_COMPILER_FAILED");
    expect(rootCauseOf({ code: "ISSUE_UNRESOLVED", internal: "phase4-SLOT_HOST_MISSING-group-2" })).toBe("SLOT_HOST_MISSING");
  });

  it("没有 internal 时退回 code 本身，不抛错", () => {
    expect(rootCauseOf({ code: "ANSWER_MISSING" })).toBe("ANSWER_MISSING");
    expect(rootCauseOf({ code: "ISSUE_UNRESOLVED", internal: "" })).toBe("ISSUE_UNRESOLVED");
    expect(rootCauseOf({ code: "QUALITY_HARD_FAILURE" })).toBe("QUALITY_HARD_FAILURE");
  });
});

describe("同一根因的泛化重复不再重复显示", () => {
  it("已经有具体目标的记录时，泛化的 QUALITY_HARD_FAILURE 被去掉", () => {
    const merged = mergePublishGateIssues([], {
      blockers: [
        { code: "QUALITY_NOT_READY", targetId: null, internal: "quality_state=blocked" },
        { code: "QUALITY_HARD_FAILURE", targetId: null, internal: "ANSWER_KEY_MISSING_SLOT" },
        { code: "ISSUE_UNRESOLVED", targetId: "q27", internal: "phase4-ANSWER_KEY_MISSING_SLOT-q27" },
        { code: "ANSWER_MISSING", targetId: "q27" }
      ]
    });

    expect(merged.map((issue) => issue.code)).toEqual(["QUALITY_NOT_READY", "ISSUE_UNRESOLVED", "ANSWER_MISSING"]);
    expect(merged.some((issue) => issue.code === "QUALITY_HARD_FAILURE")).toBe(false);
  });

  it("没有具体记录时保留泛化记录，不能把问题列表清空", () => {
    const merged = mergePublishGateIssues([], {
      blockers: [{ code: "QUALITY_HARD_FAILURE", targetId: null, internal: "RUNTIME_COMPILER_FAILED" }]
    });

    expect(merged.map((issue) => issue.code)).toEqual(["QUALITY_HARD_FAILURE"]);
    expect(blockerCount(merged)).toBe(1);
  });

  it("根因不同就不去重：两个码各自保留", () => {
    const merged = mergePublishGateIssues([], {
      blockers: [
        { code: "QUALITY_HARD_FAILURE", targetId: null, internal: "RUNTIME_COMPILER_FAILED" },
        { code: "ISSUE_UNRESOLVED", targetId: "document", internal: "phase4-SLOT_HOST_MISSING-document" }
      ]
    });

    expect(merged.map((issue) => issue.code)).toEqual(["QUALITY_HARD_FAILURE", "ISSUE_UNRESOLVED"]);
  });

  it("同一个 code 落在两个目标上仍然分别显示（不得合并成一条）", () => {
    const merged = mergePublishGateIssues([], {
      blockers: [
        { code: "ISSUE_UNRESOLVED", targetId: "group-1", internal: "phase4-SLOT_HOST_MISSING-group-1" },
        { code: "ISSUE_UNRESOLVED", targetId: "group-2", internal: "phase4-SLOT_HOST_MISSING-group-2" }
      ]
    });

    expect(merged).toHaveLength(2);
    expect(merged.map((issue) => issue.targetId).sort()).toEqual(["group-1", "group-2"]);
  });
});

// 证据层级：pure unit。夹具形状照抄真实运行产物
// `artifacts/e2e-cdp/run-issue-list-2026-09-15T23-26-03-228Z/report.json`：
// 14 个答案位的 `answerKey[slot].kind === "unresolved"`，于是
//   - 本地闭包 14 条 `ANSWER_UNRESOLVED`（warning，文案「第 27 题还没有答案。」）；
//   - 门禁 14 条 `ANSWER_MISSING`（blocker，文案「这道题还有答案没有填写。」）。
// 去重键按 code 拼，两个 code 不同 → **同一道题渲染两行**，14 道题白多 14 行。
describe("同一根因的不同 code 不再各占一行", () => {
  it("本地 ANSWER_UNRESOLVED 与门禁 ANSWER_MISSING 撞在同一目标上时只留一行", () => {
    const local: ActionableIssueV1[] = [
      { issueId: "q27:answer-missing", targetId: "q27", severity: "warning", code: "ANSWER_UNRESOLVED", userMessage: "第 27 题还没有答案。" }
    ];
    const merged = mergePublishGateIssues(local, {
      blockers: [{ code: "ANSWER_MISSING", targetId: "q27", userMessage: "这道题还有答案没有填写。" }]
    });

    expect(merged).toHaveLength(1);
    // 保留本地更具体的文案（带题号），而不是泛化的「这道题…」。
    expect(merged[0].userMessage).toBe("第 27 题还没有答案。");
    expect(merged[0].code).toBe("ANSWER_UNRESOLVED");
  });

  it("合并后的那一行要按门禁的级别显示为 blocker（发布确实被它拦下）", () => {
    const local: ActionableIssueV1[] = [
      { issueId: "q27:answer-missing", targetId: "q27", severity: "warning", code: "ANSWER_UNRESOLVED", userMessage: "第 27 题还没有答案。" }
    ];
    const merged = mergePublishGateIssues(local, {
      blockers: [{ code: "ANSWER_MISSING", targetId: "q27", userMessage: "这道题还有答案没有填写。" }]
    });

    expect(merged[0].severity).toBe("blocker");
    expect(blockerCount(merged)).toBe(1);
  });

  it("目标不同就不合并：两个答案位各留一行", () => {
    const local: ActionableIssueV1[] = [
      { issueId: "q27:answer-missing", targetId: "q27", severity: "warning", code: "ANSWER_UNRESOLVED", userMessage: "第 27 题还没有答案。" },
      { issueId: "q28:answer-missing", targetId: "q28", severity: "warning", code: "ANSWER_UNRESOLVED", userMessage: "第 28 题还没有答案。" }
    ];
    const merged = mergePublishGateIssues(local, {
      blockers: [
        { code: "ANSWER_MISSING", targetId: "q27", userMessage: "这道题还有答案没有填写。" },
        { code: "ANSWER_MISSING", targetId: "q28", userMessage: "这道题还有答案没有填写。" }
      ]
    });

    expect(merged).toHaveLength(2);
    expect(merged.map((issue) => issue.targetId).sort()).toEqual(["q27", "q28"]);
  });

  it("不就地修改传进来的本地问题对象（useMemo 的产物不能被串改）", () => {
    const local: ActionableIssueV1[] = [
      { issueId: "q27:answer-missing", targetId: "q27", severity: "warning", code: "ANSWER_UNRESOLVED", userMessage: "第 27 题还没有答案。" }
    ];
    const merged = mergePublishGateIssues(local, {
      blockers: [{ code: "ANSWER_MISSING", targetId: "q27", userMessage: "这道题还有答案没有填写。" }]
    });

    expect(merged[0].severity).toBe("blocker");
    // 升级的是合并结果里的副本，原来的 warning 不能被改掉。
    expect(local[0].severity).toBe("warning");
    expect(local[0]).not.toBe(merged[0]);
  });

  it("门禁那半标 source=gate、本地那半标 source=local（校验脚本要靠它分辨）", () => {
    const local: ActionableIssueV1[] = [
      { issueId: "q27:answer-missing", targetId: "q27", severity: "warning", code: "ANSWER_UNRESOLVED", userMessage: "第 27 题还没有答案。" }
    ];
    const merged = mergePublishGateIssues(local, {
      blockers: [
        { code: "ANSWER_MISSING", targetId: "q27", userMessage: "这道题还有答案没有填写。" },
        { code: "QUALITY_NOT_READY", targetId: null, userMessage: "这道题还有未确认的内容。" }
      ],
      warnings: [{ code: "BLOCKER_LIST_TRUNCATED", message: "问题较多，仅显示前 20 条。" }]
    });

    expect(merged.filter((issue) => issue.source === "gate").map((issue) => issue.code).sort())
      .toEqual(["BLOCKER_LIST_TRUNCATED", "QUALITY_NOT_READY"]);
    expect(merged.filter((issue) => issue.source === "local").map((issue) => issue.code))
      .toEqual(["ANSWER_UNRESOLVED"]);
  });
});

// 证据层级：pure unit。夹具形状照抄真实运行产物
// `artifacts/e2e-cdp/run-issue-list-2026-09-15T23-36-44-422Z/report.json`（complex-reading.pdf）：
// 门禁 8 条 blocker，界面只剩 3 行 —— 因为**两个完全不同的阻断问题**共用了
// `ISSUE_UNRESOLVED:document`。门禁把**所有**质量码都写成 `ISSUE_UNRESOLVED`，
// 按 code 去重就会整条吞掉一个根因。根因在 `internal` 里（`phase4-<质量码>-<目标>`），能读出来。
describe("按 code 去重会吞掉不同根因（complex-reading 的真实形状）", () => {
  const complexReadingBlockers = [
    { code: "QUALITY_NOT_READY", targetId: null, internal: "quality_state=blocked" },
    { code: "QUALITY_HARD_FAILURE", targetId: null, internal: "SLOT_HOST_MISSING" },
    { code: "QUALITY_HARD_FAILURE", targetId: null, internal: "RUNTIME_COMPILER_FAILED" },
    { code: "ISSUE_UNRESOLVED", targetId: "group-2", internal: "phase4-SLOT_HOST_MISSING-group-2" },
    { code: "ISSUE_UNRESOLVED", targetId: "document", internal: "phase4-SIGNIFICANT_REGION_UNASSIGNED-document" },
    { code: "ISSUE_UNRESOLVED", targetId: "document", internal: "phase4-RUNTIME_COMPILER_FAILED-document" }
  ];

  it("同一个 ISSUE_UNRESOLVED:document 上的两个不同根因必须各自成行（不得吞掉一个）", () => {
    const merged = mergePublishGateIssues([], { blockers: complexReadingBlockers });

    const documentRows = merged.filter((issue) => issue.targetId === "document");
    expect(documentRows.map((issue) => issue.rootCause).sort())
      .toEqual(["RUNTIME_COMPILER_FAILED", "SIGNIFICANT_REGION_UNASSIGNED"]);
  });

  it("三个根因（含泛化）都指向同一个质量码时，泛化行被去掉、具体行都留下", () => {
    const merged = mergePublishGateIssues([], { blockers: complexReadingBlockers });

    expect(merged.some((issue) => issue.code === "QUALITY_HARD_FAILURE")).toBe(false);
    expect(merged.map((issue) => issue.rootCause).sort()).toEqual([
      "QUALITY_NOT_READY",
      "RUNTIME_COMPILER_FAILED",
      "SIGNIFICANT_REGION_UNASSIGNED",
      "SLOT_HOST_MISSING"
    ]);
  });

  it("根因相同、目标相同、文案不同 → **两条不同事实，两条都留**（不得吞掉一条）", () => {
    // 真实形状：group-2 上两条不同事实共用同一个 issueId（`phase4-SLOT_HOST_MISSING-group-2`），
    // preflight 只带 issueId，所以「根因 + 目标」分不开这两条。
    // 任务书要求「不同问题不被隐藏」——因此这里必须留两行，而不是收成一行。
    const merged = mergePublishGateIssues([], {
      blockers: [
        { code: "ISSUE_UNRESOLVED", targetId: "group-2", internal: "phase4-SLOT_HOST_MISSING-group-2", userMessage: "completion slot 没有可渲染的宿主节点。" },
        { code: "ISSUE_UNRESOLVED", targetId: "group-2", internal: "phase4-SLOT_HOST_MISSING-group-2", userMessage: "table completion 没有可渲染的 table stimulus。" }
      ]
    });

    expect(merged).toHaveLength(2);
    expect(merged.map((issue) => issue.rootCause)).toEqual(["SLOT_HOST_MISSING", "SLOT_HOST_MISSING"]);
    expect(merged.map((issue) => issue.userMessage).sort())
      .toEqual(["completion slot 没有可渲染的宿主节点。", "table completion 没有可渲染的 table stimulus。"]);
  });

  it("同来源、同根因、同目标、**同文案** → 仍然只留一行（真重复）", () => {
    const duplicated = { code: "ISSUE_UNRESOLVED", targetId: "group-2", internal: "phase4-SLOT_HOST_MISSING-group-2", userMessage: "同一条文案" };
    const merged = mergePublishGateIssues([], { blockers: [duplicated, { ...duplicated }] });

    expect(merged).toHaveLength(1);
  });

  it("跨来源、同根因、同目标、文案不同 → 收成一行（这是设计如此，不是重复）", () => {
    // 本地闭包写「第 27 题还没有答案。」，门禁写「这道题还有答案没有填写。」——
    // 两个子系统各写一句是**设计如此**，不能因此把同一道题显示两行（实测会白多 14 行）。
    const local: ActionableIssueV1[] = [
      { issueId: "q27:answer-missing", targetId: "q27", severity: "warning", code: "ANSWER_UNRESOLVED", userMessage: "第 27 题还没有答案。" }
    ];
    const merged = mergePublishGateIssues(local, {
      blockers: [{ code: "ANSWER_MISSING", targetId: "q27", userMessage: "这道题还有答案没有填写。" }]
    });

    expect(merged).toHaveLength(1);
    expect(merged[0].userMessage).toBe("第 27 题还没有答案。");
    expect(merged[0].severity).toBe("blocker");
  });
});

// 证据层级：pure unit。任务书第 3 条要求「等待后端稳定事实 ID」——
// 后端已把 `issueId` 从 `phase4-{code}-{target}` 改成 `phase4-{code}-{target}-{slug}`
// （`quality.rs` 的 `issue()`，slug 是判别性载荷的确定性哈希），
// preflight 把该 id 放进 `ISSUE_UNRESOLVED` 的 `internal`。于是同一目标上的两条不同事实
// **终于有了不同的 id**，不必再靠文案猜。
describe("后端稳定事实 id（factId）：同目标上的两条不同事实靠 id 区分", () => {
  const NEW_A = "phase4-SLOT_HOST_MISSING-group-2-9f1c2ab7";
  const NEW_B = "phase4-SLOT_HOST_MISSING-group-2-4d0e88c1";
  const OLD = "phase4-SLOT_HOST_MISSING-group-2"; // 旧编码：会撞键

  it("同 code + 同目标 + **同文案** + 不同 factId → 两条都留（上一轮会吞掉一条）", () => {
    // 这是本轮新加的保护：文案相同不再等于同一事实。
    // 旧规则只比文案，两条会被合成一条 —— 那正是「不同问题被隐藏」。
    const merged = mergePublishGateIssues([], {
      blockers: [
        { code: "ISSUE_UNRESOLVED", targetId: "group-2", internal: NEW_A, userMessage: "这一组无法渲染。" },
        { code: "ISSUE_UNRESOLVED", targetId: "group-2", internal: NEW_B, userMessage: "这一组无法渲染。" }
      ]
    });

    expect(merged).toHaveLength(2);
    expect(merged.map((issue) => issue.factId).sort()).toEqual([NEW_A, NEW_B].sort());
  });

  it("同 factId + 同文案 → 仍然只留一行（同一事实被推了两次）", () => {
    const duplicated = { code: "ISSUE_UNRESOLVED", targetId: "group-2", internal: NEW_A, userMessage: "同一条文案" };
    const merged = mergePublishGateIssues([], { blockers: [duplicated, { ...duplicated }] });

    expect(merged).toHaveLength(1);
  });

  it("旧编码（factId 撞键）+ 文案不同 → 两条都留（不得回退到 F-R9-13）", () => {
    // 单向使用 factId 的意义：id 相同**不作结论**，继续比文案。
    // 若拿 id 去「断言同一」，这两条会被合并 —— 就是上一轮的反方向错误。
    const merged = mergePublishGateIssues([], {
      blockers: [
        { code: "ISSUE_UNRESOLVED", targetId: "group-2", internal: OLD, userMessage: "completion slot 没有可渲染的宿主节点。" },
        { code: "ISSUE_UNRESOLVED", targetId: "group-2", internal: OLD, userMessage: "table completion 没有可渲染的 table stimulus。" }
      ]
    });

    expect(merged).toHaveLength(2);
  });

  it("行 id 逐行唯一（它是 React 的 key，撞键会让 React 复用/丢弃 DOM）", () => {
    const merged = mergePublishGateIssues([], {
      blockers: [
        { code: "ISSUE_UNRESOLVED", targetId: "group-2", internal: NEW_A, userMessage: "同一句文案" },
        { code: "ISSUE_UNRESOLVED", targetId: "group-2", internal: NEW_B, userMessage: "同一句文案" }
      ]
    });

    expect(new Set(merged.map((issue) => issue.issueId)).size).toBe(merged.length);
  });

  it("`internal` 只在 ISSUE_UNRESOLVED 上才是事实 id（QUALITY_HARD_FAILURE 放的是质量码）", () => {
    expect(factIdOf({ code: "ISSUE_UNRESOLVED", internal: NEW_A })).toBe(NEW_A);
    // 同一个 `internal` 值，换个 code 就不是 id —— 否则质量码会被当成事实 id。
    expect(factIdOf({ code: "QUALITY_HARD_FAILURE", internal: "SLOT_HOST_MISSING" })).toBeUndefined();
    expect(factIdOf({ code: "ISSUE_UNRESOLVED", internal: "   " })).toBeUndefined();
    expect(factIdOf({ code: "ISSUE_UNRESOLVED" })).toBeUndefined();
  });
});
