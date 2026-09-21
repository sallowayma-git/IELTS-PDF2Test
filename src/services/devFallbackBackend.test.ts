import { describe, expect, it } from "vitest";

import { normalizeValidationPolicy } from "./devFallbackBackend";

// 这里只覆盖**旧导出命令**的 validationPolicy（无界面调用方，保持严格）。
// 产品发布入口 `publish_items` 的「点击即放行并记录」走 Tauri 后端，devFallback 不模拟它
// （调用会报 requires_tauri_runtime），见 src/api/publishClient.test.ts 与 Rust publish_final_tests。
describe("dev fallback validation policy (legacy export commands)", () => {
  it("rejects the removed force policy instead of treating it as an override", () => {
    expect(() => normalizeValidationPolicy("force")).toThrow("invalid_validation_policy:force");
  });
});
