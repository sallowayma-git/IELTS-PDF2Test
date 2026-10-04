// @vitest-environment jsdom
// Actual workspace and canonical-editor hook; mocked IPC is frequency/race evidence only.
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { ProcessingItemUpdate } from "../../api/processingClient";
const mocks = vi.hoisted(() => ({
  workspace: vi.fn(), processing: vi.fn(), fullLibrary: vi.fn(),
  listener: undefined as ((update: ProcessingItemUpdate) => void) | undefined,
}));
vi.mock("../../api/workspaceClient", async () => {
  const { default: ds } = await import("../../../fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json");
  mocks.workspace.mockResolvedValue({ ds, item: { title: "Read audit", sourcePurged: false }, editVersion: 1 });
  return { getWorkspaceItem: mocks.workspace, getLibraryItemProcessing: mocks.processing, listLibraryItems: mocks.fullLibrary,
    getPublishPreflight: vi.fn(async () => ({ passed: true, blockers: [], warnings: [] })), applyEditorCommands: vi.fn() };
});
vi.mock("../../api/tauriCommands", () => ({ command: vi.fn(async () => ({})), getJob: vi.fn(async () => null) }));
vi.mock("../../api/recognitionClient", () => ({ getRecognitionDecision: vi.fn(async () => ({})) }));
vi.mock("../../api/processingClient", () => ({
  subscribeProcessing: vi.fn(async (callback) => { mocks.listener = callback; return () => {}; }),
  describeRetryOutcome: () => "", retryProcessing: vi.fn(), retryAnswerPageRecognition: vi.fn(), cancelProcessing: vi.fn(),
}));
import { ExamWorkspacePage } from "./ExamWorkspacePage";
const state = (stage: string) => ({ stage, localStatus: "succeeded", cloudStatus: stage === "cloud_recognition" ? "running" : "succeeded", actionableCount: 0, eventSeq: 1 });
beforeEach(() => {
  vi.clearAllMocks(); localStorage.clear();
  globalThis.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} } as never;
  mocks.processing.mockResolvedValue(state("local_recognition"));
});
afterEach(cleanup);
it("mount plus five same-edit-version processing events use only six current-item reads", async () => {
  render(<ExamWorkspacePage itemId="item" />);
  await waitFor(() => expect(mocks.processing).toHaveBeenCalledTimes(1));
  for (let version = 1; version <= 5; version++) {
    mocks.processing.mockResolvedValue(state(version === 5 ? "cancelled" : "cloud_recognition"));
    await act(async () => { mocks.listener?.({ itemId: "item", stateVersion: version, editVersion: 1 }); });
  }
  expect(mocks.processing).toHaveBeenCalledTimes(6);
  expect(mocks.processing.mock.calls.every(([itemId]) => itemId === "item")).toBe(true);
  expect(mocks.fullLibrary).not.toHaveBeenCalled();
  expect(screen.queryByTestId("workspace-cloud-review-banner")).toBeNull();
  const reads = mocks.processing.mock.calls.length;
  await act(async () => { mocks.listener?.({ itemId: "another", stateVersion: 8, editVersion: 1 }); });
  expect(mocks.processing).toHaveBeenCalledTimes(reads);
});
it("a late initial cloud-state response cannot restore the lock after the cancel event", async () => {
  let resolve!: (value: ReturnType<typeof state>) => void;
  mocks.processing.mockReturnValueOnce(new Promise((done) => { resolve = done; }));
  render(<ExamWorkspacePage itemId="item" />);
  await waitFor(() => expect(mocks.processing).toHaveBeenCalledTimes(1));
  mocks.processing.mockResolvedValue(state("cancelled"));
  await act(async () => { mocks.listener?.({ itemId: "item", stateVersion: 2, editVersion: 1 }); });
  expect(mocks.processing).toHaveBeenCalledTimes(2);
  await act(async () => { resolve(state("cloud_recognition")); });
  expect(screen.queryByTestId("workspace-cloud-review-banner")).toBeNull();
  expect(mocks.fullLibrary).not.toHaveBeenCalled();
});
