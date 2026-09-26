import { describe, expect, it } from "vitest";
import type { OptionV2 } from "../types";
import { isTfngLabel, isTfngOptionSet } from "./tfngOptions";

// Evidence level: pure unit（判断题短标签版式判定）。

const option = (label: string): OptionV2 =>
  ({ optionId: `opt-${label}`, label, content: [], sourceAnchors: [] }) as OptionV2;

describe("tfng option detection", () => {
  it("matches TRUE/FALSE/NOT GIVEN and YES/NO/NOT GIVEN regardless of case and padding", () => {
    expect(isTfngOptionSet([option("TRUE"), option("FALSE"), option("NOT GIVEN")])).toBe(true);
    expect(isTfngOptionSet([option("Yes"), option(" No "), option("Not Given")])).toBe(true);
    expect(isTfngLabel(" not given ")).toBe(true);
  });

  it("rejects regular option banks and mixed sets", () => {
    expect(isTfngOptionSet([option("A"), option("B"), option("C")])).toBe(false);
    expect(isTfngOptionSet([option("TRUE"), option("B")])).toBe(false);
    expect(isTfngOptionSet([])).toBe(false);
  });

  it("never treats a lone NO-style label set as regular choices", () => {
    expect(isTfngOptionSet([option("NO"), option("NOT GIVEN")])).toBe(true);
  });
});
