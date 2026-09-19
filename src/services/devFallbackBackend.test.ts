import { describe, expect, it } from "vitest";

import { normalizeValidationPolicy } from "./devFallbackBackend";

describe("dev fallback validation policy", () => {
  it("rejects the removed force policy instead of treating it as an override", () => {
    expect(() => normalizeValidationPolicy("force")).toThrow("invalid_validation_policy:force");
  });
});
