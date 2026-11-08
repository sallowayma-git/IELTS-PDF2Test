// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { legacyRedirect, parseRoute } from "./router";

// 证据层级：pure unit。导入是一级路由；旧的 ?import=1 与 #/jobs/new 都落到导入页。

describe("导入一级路由", () => {
  it("#/import 解析为导入页", () => {
    expect(parseRoute("#/import").name).toBe("import");
  });

  it("旧的 ?import=1 意图也落到导入页，而不是题库抽屉", () => {
    expect(parseRoute("#/library?import=1").name).toBe("import");
  });

  it("题库与设置路由保持不变", () => {
    expect(parseRoute("#/library").name).toBe("library");
    expect(parseRoute("#/settings").name).toBe("settings");
    expect(parseRoute("#/items/abc").name).toBe("workspace");
  });

  it("退休链接改指导入页", () => {
    expect(legacyRedirect("#/jobs/new")).toBe("/import");
    expect(legacyRedirect("#/legacy/import")).toBe("/import");
  });
});
