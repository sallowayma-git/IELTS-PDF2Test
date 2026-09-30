// @vitest-environment jsdom
import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { CloudComparison, chooseComparison } from "./CloudComparison";
import type { IeltsAuthoringIRV2 } from "../../types";
import { command } from "../../api/tauriCommands";
import { getWorkspaceItem } from "../../api/workspaceClient";
vi.mock("../../exam-canvas/ExamCanvas", () => ({ ExamCanvas: ({authoring, locked}: {authoring:IeltsAuthoringIRV2;locked:boolean}) => <div data-testid="candidate" data-locked={String(locked)}>{authoring.taskGroups.map(g => g.taskId).join(",")}</div> }));
vi.mock("../../api/tauriCommands", () => ({command:vi.fn(async()=>({}))}));
vi.mock("../../api/workspaceClient", () => ({getWorkspaceItem:vi.fn(async()=>({editVersion:12}))}));
const candidate = (taskId:string) => ({taskGroups:[{taskId}]} as IeltsAuthoringIRV2);
describe("cloud comparison",()=>{
  it("shows complete read-only alternatives and sends only the selected identity",()=>{
    const choose = vi.fn();
    render(<CloudComparison task={{userTaskId:"u",comparisonUnitId:"unit",localCandidate:candidate("local-group"),cloudCandidate:candidate("cloud-group")}} disabled={false} onChoose={choose}/>);
    expect(screen.getAllByTestId("candidate").every(el=>el.dataset.locked === "true")).toBe(true);
    fireEvent.click(screen.getByRole("button",{name:"采用本地"}));
    expect(choose).toHaveBeenCalledWith("local");
  });
  it("flushes before reading the current version and choosing",async()=>{
    const order:string[]=[];
    vi.mocked(getWorkspaceItem).mockImplementationOnce(async()=>{order.push("version");return {editVersion:19} as Awaited<ReturnType<typeof getWorkspaceItem>>;});
    vi.mocked(command).mockImplementationOnce(async()=>{order.push("choose");return {} as never;});
    await chooseComparison("item","unit","cloud",async()=>{order.push("flush");});
    expect(order).toEqual(["flush","version","choose"]);
    expect(command).toHaveBeenLastCalledWith("choose_cloud_comparison",{itemId:"item",unitId:"unit",choice:"cloud",baseVersion:19});
  });
  it("does not submit when saving pending edits fails",async()=>{
    vi.mocked(command).mockClear();
    await expect(chooseComparison("item","unit","local",async()=>{throw Error("unsaved");})).rejects.toThrow("unsaved");
    expect(command).not.toHaveBeenCalled();
  });
});
