import { describe, expect, it } from "vitest";
import type { CloudRepairSummaryV1, CloudRepairTaskV1, RecognitionDecisionViewV1 } from "../../api/recognitionClient";
import { describeRepairStatus } from "../../api/recognitionClient";
import {
  isRepairPanelQuiet,
  repairBlockerCount,
  repairHeadline,
  repairInFlight,
  repairTaskCount,
  repairTasks,
  toRepairTaskView,
  usesRepairTaskList
} from "./repairTasks";

// 剩余任务的呈现规则。断言的是**用户看到什么**，不是实现细节。

function view(repair: CloudRepairSummaryV1 | null): RecognitionDecisionViewV1 {
  return {
    schemaVersion: "RecognitionDecisionViewV1",
    itemId: "item-1",
    batchId: "batch-1",
    baseEditVersion: 1,
    currentEditVersion: 2,
    stale: false,
    localStatus: "succeeded",
    cloudStatus: "succeeded",
    sourceStatus: "succeeded",
    adjudicationStatus: "succeeded",
    summary: { agreed: 0, autoFixed: 0, needsReview: 0, unverifiable: 0 },
    items: [],
    repair
  } as unknown as RecognitionDecisionViewV1;
}

function task(overrides: Partial<CloudRepairTaskV1> = {}): CloudRepairTaskV1 {
  return { userTaskId: "cloud-diff:slot:q14:answer", ...overrides };
}

describe("剩余任务清单只在有修复记录时才接管界面", () => {
  it("没有修复记录（旧批次 / 无云导入）时不接管", () => {
    expect(usesRepairTaskList(view(null))).toBe(false);
    expect(usesRepairTaskList(undefined)).toBe(false);
    expect(repairTasks(view(null))).toEqual([]);
    // 不接管时面板不能因为「没有剩余任务」就报安静——旧路径有自己的判据。
    expect(isRepairPanelQuiet(view(null))).toBe(false);
  });

  it("有修复记录时接管，并按后端顺序原样呈现", () => {
    const first = task({ userTaskId: "quality:ANSWER_MISSING:q14", blocking: true, targetIds: ["q14"] });
    const second = task({ userTaskId: "cloud-question:q15:0", message: "字形模糊", targetIds: ["q15"] });
    const next = view({ status: "needs_attention", remainingTasks: [first, second] });
    expect(usesRepairTaskList(next)).toBe(true);
    expect(repairTasks(next).map((entry) => entry.taskId)).toEqual([
      "quality:ANSWER_MISSING:q14",
      "cloud-question:q15:0"
    ]);
    expect(repairTaskCount(next)).toBe(2);
    expect(repairBlockerCount(next)).toBe(1);
  });
});

describe("每条剩余任务都有真能做完的动作", () => {
  it("有目标就定位到题面；阻断项额外给「去题面修改」", () => {
    const entry = toRepairTaskView(task({ targetIds: ["q14"], blocking: true, action: "fix_blocking_issue" }));
    expect(entry.actions.map((action) => action.id)).toEqual(["locate", "fix-on-page"]);
    expect(entry.actions[0].targetId).toBe("q14");
  });

  it("非阻断的差异只给定位，不冒充「必须修改」", () => {
    const entry = toRepairTaskView(task({ targetIds: ["q14"], action: "review_difference" }));
    expect(entry.actions.map((action) => action.id)).toEqual(["locate"]);
  });

  it("文档级任务没有题面目标时给「打开原文件核对」", () => {
    const entry = toRepairTaskView(task({ targetIds: [], action: "review_source" }));
    expect(entry.actions.map((action) => action.id)).toEqual(["open-source"]);
  });

  it("没有目标也不看原文件时**不给按钮**——点了不动比没有按钮更糟", () => {
    const entry = toRepairTaskView(task({ targetIds: [], action: "review_difference" }));
    expect(entry.actions).toEqual([]);
  });

  it("空 targetIds 里的空串不算目标", () => {
    const entry = toRepairTaskView(task({ targetIds: [""], action: "review_source" }));
    expect(entry.actions.map((action) => action.id)).toEqual(["open-source"]);
  });

  it("message 缺失时也要给一句能读懂的话，不能是空白", () => {
    const entry = toRepairTaskView(task({ message: "   " }));
    expect(entry.message.length).toBeGreaterThan(0);
    expect(entry.message).not.toContain("undefined");
  });
});

describe("修复进行中不把空清单说成「没问题」", () => {
  it("running 时不安静、且 headline 说明还要等", () => {
    const next = view({ status: "running", appliedCount: 2, remainingTasks: [] });
    expect(repairInFlight(next)).toBe(true);
    expect(isRepairPanelQuiet(next)).toBe(false);
    expect(repairHeadline(next)).toContain("正在自动修复");
    // 进行中**不报**剩余条数：此刻还没有可信的剩余清单。
    expect(describeRepairStatus(next.repair)).not.toMatch(/还有\s*0/);
  });

  it("完成且没有剩余时安静，headline 明确说不用做事", () => {
    const next = view({ status: "completed", appliedCount: 3, remainingTasks: [] });
    expect(isRepairPanelQuiet(next)).toBe(true);
    expect(repairHeadline(next)).toContain("没有需要你处理");
  });
});

describe("修复状态行要说出云端替用户做了什么", () => {
  it("已修 + 已了结的条数都要出现", () => {
    const text = describeRepairStatus({
      status: "completed",
      appliedCount: 2,
      adjudicatedCount: 3,
      remainingTasks: []
    });
    expect(text).toContain("已自动修正 2 处");
    expect(text).toContain("已了结 3 处差异");
  });

  it("还有剩余时必须把剩余条数说出来", () => {
    const text = describeRepairStatus({
      status: "needs_attention",
      appliedCount: 1,
      adjudicatedCount: 0,
      remainingTasks: [task(), task({ userTaskId: "b" })]
    });
    expect(text).toContain("2 处");
  });

  it("没有修复记录说成「未进行云端修复」，绝不说成完成", () => {
    expect(describeRepairStatus(null)).toBe("未进行云端修复");
    expect(describeRepairStatus(undefined)).toBe("未进行云端修复");
  });
});
