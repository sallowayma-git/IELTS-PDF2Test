// @vitest-environment jsdom
//
// 云端校核进行中：工作区显示锁定横幅并禁用人工写入入口，结束后横幅消失。
// 证据层级：组件级（mock 命令客户端与编辑器 hook，横幅/禁用的真实渲染保持真实）；
// 画布只读与"结束后可保存"由真实应用 e2e（cloud-repair-chain）验收。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { getRecognitionDecision } from "../../api/recognitionClient";
import { cancelProcessing } from "../../api/processingClient";
import { listLibraryItems } from "../../api/workspaceClient";
import { ExamWorkspacePage } from "./ExamWorkspacePage";

const editorRef = vi.hoisted(() => ({ current: null as unknown }));

vi.mock("./useCanonicalEditor", () => ({ useCanonicalEditor: vi.fn(() => editorRef.current) }));
vi.mock("../../api/recognitionClient", () => ({ getRecognitionDecision: vi.fn() }));
vi.mock("../../api/tauriCommands", () => ({ command: vi.fn(async () => ({})), getJob: vi.fn(async () => null) }));
vi.mock("../../api/processingClient", () => ({
  subscribeProcessing: vi.fn(async () => () => {}),
  describeRetryOutcome: vi.fn(() => ""),
  retryAnswerPageRecognition: vi.fn(),
  retryProcessing: vi.fn(),
  cancelProcessing: vi.fn(async () => {}),
}));
vi.mock("../../api/desktopDialogs", () => ({ chooseExportDirectory: vi.fn() }));
vi.mock("../../api/publishClient", () => ({ describePublishOutcome: vi.fn(() => ""), publishItem: vi.fn(), publishOutcomeKind: vi.fn(() => "success") }));
vi.mock("../../api/workspaceClient", () => ({
  getWorkspaceItem: vi.fn(async () => null),
  getPublishPreflight: vi.fn(async () => null),
  listLibraryItems: vi.fn(async () => []),
}));
vi.mock("../settings/appSettings", () => ({ readAppSettings: vi.fn(() => ({})), writeAppSettings: vi.fn() }));
vi.mock("../../app/router", () => ({ go: vi.fn(), libraryPath: vi.fn(() => "/library") }));
vi.mock("./finalVersion", () => ({ SOURCE_PURGED_EXPLANATION: "", saveToLibrary: vi.fn(), sourceActionsAvailable: vi.fn(() => true) }));
vi.mock("../../exam-canvas/ExamCanvas", () => ({ ExamCanvas: () => null }));
vi.mock("./SelectionInspector", () => ({ SelectionInspector: () => null }));
vi.mock("./RecognitionPanel", () => ({ RecognitionPanel: () => null }));
vi.mock("./studentPreview", () => ({
  compilePreviewSource: vi.fn(() => ({ ok: true, summary: { answerKeyIssues: [] } })),
  describePreviewPublishLimitation: vi.fn(() => null),
}));

const ITEM_ID = "item-1";

function makeEditor() {
  return {
    draft: { exam: { title: "受控卷" }, taskGroups: [], answerSlots: {}, answerKey: {} },
    version: 1, pendingCount: 0, saveState: "idle", loading: false, loadError: undefined,
    conflictRecovering: false, saveNotice: undefined, saveMessage: "", title: "", canUndo: true, canRedo: true,
    deferredRemoteRefresh: false,
    flush: vi.fn(async () => {}), reload: vi.fn(), noteRemoteVersion: vi.fn(),
    applyPatch: vi.fn(), applyCommand: vi.fn(), undo: vi.fn(), redo: vi.fn(), setTitle: vi.fn(),
    recoverFromConflict: vi.fn(), dismissSaveNotice: vi.fn(), discardLocalChanges: vi.fn(),
  };
}

function withProcessing(stage: string, cloudStatus: string) {
  vi.mocked(listLibraryItems).mockResolvedValue([{ id: ITEM_ID, processing: { stage, localStatus: "succeeded", cloudStatus, actionableCount: 0, eventSeq: 1 } }] as never);
}

beforeEach(() => {
  editorRef.current = makeEditor();
  vi.mocked(getRecognitionDecision).mockResolvedValue({ repair: null } as never);
});
afterEach(() => { cleanup(); vi.clearAllMocks(); });

describe("ExamWorkspacePage 云端校核锁横幅", () => {
  it("云端校核进行中：横幅可见、停止按钮调用取消、保存禁用", async () => {
    withProcessing("cloud_recognition", "running");
    render(<ExamWorkspacePage itemId={ITEM_ID} />);
    await act(async () => {});

    expect(screen.getByTestId("workspace-cloud-review-banner").textContent).toContain("云端正在校核");
    expect((screen.getByTestId("workspace-save") as HTMLButtonElement).disabled).toBe(true);

    fireEvent.click(screen.getByTestId("workspace-cloud-review-stop"));
    expect(vi.mocked(cancelProcessing)).toHaveBeenCalledWith(ITEM_ID);
  });

  it("非校核阶段：不显示锁定横幅、保存可用", async () => {
    withProcessing("ready_for_review", "succeeded");
    render(<ExamWorkspacePage itemId={ITEM_ID} />);
    await act(async () => {});

    expect(screen.queryByTestId("workspace-cloud-review-banner")).toBeNull();
    expect((screen.getByTestId("workspace-save") as HTMLButtonElement).disabled).toBe(false);
  });
});
