import { describe, expect, it } from "vitest";
import {
  jobResumePath,
  legacyPath,
  legacyRedirect,
  libraryPath,
  parseRoute,
  workspacePath
} from "./router";

// 证据层级：pure unit（计划 §19.1 层 1 / §3.1 三路由收敛）。
// 只测纯函数（显式传入 hash），不触碰 window，因此不需要 DOM 环境。
// 若路由回归到「把旧页面当主路由渲染」或放行错误 legacy 路径，这些用例会失败。

describe("parseRoute — 三个主表面", () => {
  it("空 hash 落到题库", () => {
    expect(parseRoute("")).toEqual({ name: "library" });
    expect(parseRoute("#/")).toEqual({ name: "library" });
  });

  it("/items/:id 解析为工作区", () => {
    expect(parseRoute("#/items/import-1")).toMatchObject({ name: "workspace", itemId: "import-1" });
  });

  it("/settings 解析为设置页", () => {
    expect(parseRoute("#/settings")).toMatchObject({ name: "settings" });
  });

  it("/library 解析为题库并带上意图", () => {
    expect(parseRoute("#/library")).toEqual({ name: "library" });
    expect(parseRoute("#/library?import=1")).toEqual({ name: "library", intent: "import" });
    expect(parseRoute("#/library?publish=1")).toEqual({ name: "library", intent: "publish" });
  });

  it("工作区也可携带发布意图", () => {
    expect(parseRoute("#/items/x?publish=1")).toMatchObject({ name: "workspace", itemId: "x", intent: "publish" });
  });

  it("只放行 legacy/writing，其余 legacy 页面不再解析为 legacy 路由", () => {
    expect(parseRoute("#/legacy/writing")).toMatchObject({ name: "legacy", legacyPage: "writing" });
    expect(parseRoute("#/legacy/dashboard")).toMatchObject({ name: "legacy", legacyPage: "dashboard" });
    expect(parseRoute("#/legacy/not-a-page")).toMatchObject({ name: "library" });
  });

  it("无法识别的一级路径落回题库", () => {
    expect(parseRoute("#/jobs/abc")).toMatchObject({ name: "library" });
    expect(parseRoute("#/totally-unknown")).toMatchObject({ name: "library" });
  });
});

describe("legacyRedirect — 旧链接一次性重定向", () => {
  it("无路径回到题库", () => {
    expect(legacyRedirect("")).toBe("/library");
  });

  it("已退休的顶层页面重定向到题库", () => {
    expect(legacyRedirect("#/dashboard")).toBe("/library");
    expect(legacyRedirect("#/phase5")).toBe("/library");
  });

  it("/jobs 系列映射到题库 / 工作区", () => {
    expect(legacyRedirect("#/jobs")).toBe("/library");
    expect(legacyRedirect("#/jobs/new")).toBe("/library?import=1");
    expect(legacyRedirect("#/jobs/abc")).toBe("/items/abc");
    expect(legacyRedirect("#/jobs/abc/preview")).toBe("/items/abc");
    expect(legacyRedirect("#/jobs/abc/export")).toBe("/items/abc?publish=1");
  });

  it("/library/:id 与 /export、/packs 映射到新路由", () => {
    expect(legacyRedirect("#/library/abc")).toBe("/items/abc");
    expect(legacyRedirect("#/export")).toBe("/library?publish=1");
    expect(legacyRedirect("#/packs")).toBe("/library?publish=1");
  });

  it("显式 legacy 逃生通道：除 writing 外重定向", () => {
    expect(legacyRedirect("#/legacy/import")).toBe("/library?import=1");
    expect(legacyRedirect("#/legacy/dashboard")).toBe("/library");
    expect(legacyRedirect("#/legacy/dashboard/abc")).toBe("/items/abc");
  });

  it("新路由与 legacy/writing 不重定向", () => {
    expect(legacyRedirect("#/items/abc")).toBeUndefined();
    expect(legacyRedirect("#/settings")).toBeUndefined();
    expect(legacyRedirect("#/library")).toBeUndefined();
    expect(legacyRedirect("#/legacy/writing")).toBeUndefined();
    expect(legacyRedirect("#/legacy/writing/abc")).toBeUndefined();
  });
});

describe("路径构造", () => {
  it("libraryPath / workspacePath 带可选意图", () => {
    expect(libraryPath()).toBe("/library");
    expect(libraryPath("import")).toBe("/library?import=1");
    expect(workspacePath("x")).toBe("/items/x");
    expect(workspacePath("x", "publish")).toBe("/items/x?publish=1");
  });

  it("legacyPath 与 jobResumePath", () => {
    expect(legacyPath("writing")).toBe("/legacy/writing");
    expect(legacyPath("writing", "id-1")).toBe("/legacy/writing/id-1");
    expect(jobResumePath({ jobId: "job-9" })).toBe("/items/job-9");
  });
});
