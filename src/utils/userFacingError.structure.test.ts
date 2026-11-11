// 结构编辑与确认入库的机器码 → 人话（证据层级：单元）。
import { describe, expect, it } from "vitest";
import { toUserFacingError } from "./userFacingError";
import { describePublishFailure } from "../api/publishClient";

describe("结构编辑的错误码", () => {
  it("最后一题不能删：提示每组至少保留一题", () => {
    expect(toUserFacingError(new Error("AUTHORING_PATCH_LAST_ANSWER_SLOT"), "x").userMessage).toBe("每组至少保留一题，可删除整组。");
  });
  it("撤销冲突：内容已被修改，无法撤销这一步", () => {
    expect(toUserFacingError(new Error("AUTHORING_PATCH_RESTORE_CONFLICT"), "x").userMessage).toBe("内容已被修改，无法撤销这一步。");
  });
  it("云端校核中：提示等待校核结束", () => {
    expect(toUserFacingError(new Error("CLOUD_REVIEW_IN_PROGRESS:item-1"), "x").userMessage).toContain("云端校核进行中");
  });
});

describe("确认入库相关的错误码", () => {
  it("在用条目不能退回待确认", () => {
    expect(toUserFacingError(new Error("LIBRARY_ITEM_IN_USE"), "x").userMessage).toContain("不能退回待确认");
  });
});

describe("组卷发布：试卷里有未确认入库的题", () => {
  it("PAPER_ITEM_NOT_CONFIRMED:<id> 译成「试卷里有题目尚未确认入库：<题目名>」", () => {
    const titles: Record<string, string> = { "job-7": "阅读 · 第七卷" };
    expect(describePublishFailure(new Error("PAPER_ITEM_NOT_CONFIRMED:job-7"), (id) => titles[id] ?? id))
      .toBe("试卷里有题目尚未确认入库：阅读 · 第七卷");
  });
  it("取不到题目名时退回 id，不出现机器码前缀以外的乱码", () => {
    expect(describePublishFailure("PAPER_ITEM_NOT_CONFIRMED:job-9")).toBe("试卷里有题目尚未确认入库：job-9");
  });
});
