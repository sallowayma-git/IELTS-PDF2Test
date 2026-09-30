// @vitest-environment jsdom
import { describe,it,expect } from "vitest";
import { findTargetElement } from "./locate";
describe("comparison navigation",()=>{
  it("locates the editable current question instead of an earlier read-only alternative",()=>{
    document.body.innerHTML='<div class="cloud-comparison-preview"><div data-question-id="q1" id="alternative"></div></div><div data-question-id="q1" id="current"></div>';
    expect(findTargetElement("q1",undefined)?.id).toBe("current");
  });
});
