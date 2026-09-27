import { describe, expect, it } from "vitest";
import { hasTrailingBlankPageResidue } from "./student-real-provider.mjs";

describe("student provider blank-page footer guard", () => {
  it("detects the residual marker after local folio filtering and the previous combined form", () => {
    expect(hasTrailingBlankPageResidue("The prompt ends 14 BLANK PAGE")).toBe(true);
    expect(hasTrailingBlankPageResidue("The prompt ends BLANK PAGE")).toBe(true);
  });

  it("allows prompts without a trailing blank-page marker", () => {
    expect(hasTrailingBlankPageResidue("The prompt ends here.")).toBe(false);
    expect(hasTrailingBlankPageResidue("BLANK PAGE text is discussed in the passage.")).toBe(false);
  });
});
