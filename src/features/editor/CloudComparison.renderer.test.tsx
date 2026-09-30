// @vitest-environment jsdom
import { render, screen } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { CloudComparison } from "./CloudComparison";
import type { IeltsAuthoringIRV2 } from "../../types";
import fixture from "../../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json";
vi.mock("../../api/tauriCommands",()=>({command:vi.fn(),resolveAuthoringAssetPreview:vi.fn(async()=>undefined)}));
vi.mock("../../api/listeningAudioClient",()=>({getListeningAudio:vi.fn(async()=>undefined)}));
describe("real question renderer comparison",()=>{
  it("renders both complete alternatives without author edit tools or duplicate page navigation",()=>{
    const document = fixture as unknown as IeltsAuthoringIRV2;
    const {container}=render(<CloudComparison task={{userTaskId:"t",comparisonUnitId:"unit",localCandidate:document,cloudCandidate:document}} disabled={false} onChoose={()=>{}}/>);
    expect(screen.getAllByTestId("exam-canvas-v2-author")).toHaveLength(2);
    expect(container.querySelectorAll('.cloud-comparison-preview .v2-task-group')).toHaveLength(document.taskGroups.length * 2);
    expect(container.querySelector('[contenteditable="true"]')).toBeNull();
    expect(container.querySelector('.v2-author-tools')).toBeNull();
    expect(container.querySelector('#left, #right, #question-groups')).toBeNull();
    expect(container.querySelector('[data-testid="question-nav"]')).toBeNull();
    expect(container.querySelectorAll('.cloud-comparison-preview [inert]')).toHaveLength(2);
  });
});
