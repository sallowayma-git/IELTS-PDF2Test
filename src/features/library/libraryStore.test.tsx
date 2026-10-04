// @vitest-environment jsdom
// Actual React hook, mocked IPC: request counts and event/snapshot ordering, not native timing.
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ProcessingItemUpdate } from "../../api/processingClient";
const mocks = vi.hoisted(() => ({
  listJobs: vi.fn(), listLibraryExams: vi.fn(), listTrashedExams: vi.fn(), listLibraryItems: vi.fn(),
  getLibraryRow: vi.fn(), subscribe: vi.fn(), unlisten: vi.fn(),
  listener: undefined as ((update: ProcessingItemUpdate) => void) | undefined,
}));
vi.mock("../../api/tauriCommands", () => ({
  listJobs: mocks.listJobs, listLibraryExams: mocks.listLibraryExams, listTrashedExams: mocks.listTrashedExams,
  deleteLibraryExam: vi.fn(), emptyRecycleBin: vi.fn(), permanentlyDeleteExam: vi.fn(), restoreLibraryExam: vi.fn(), setLibraryItemPart: vi.fn(),
}));
vi.mock("../../api/workspaceClient", () => ({ listLibraryItems: mocks.listLibraryItems, getLibraryRow: mocks.getLibraryRow }));
vi.mock("../../api/processingClient", () => ({ subscribeProcessing: mocks.subscribe }));
import { useLibraryStore } from "./libraryStore";
const item = (id = "item", status = "processing") => ({ id, title: `Title ${id}`, modality: "reading", status,
  updatedAt: "2026-10-04", partLabel: "P3", hasCanonicalDs: true, currentEditVersion: 1 });
const row = (stage = "local_recognition", id = "item") => ({ job: null, summary: null, inTrash: false,
  item: { ...item(id, stage === "ready_for_review" ? "ready" : "processing"),
    processing: { stage, localStatus: "succeeded", cloudStatus: "succeeded", actionableCount: 0, eventSeq: 8 } } });
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>((done) => { resolve = done; }); return { promise, resolve }; }
async function emit(id = "item", stateVersion = 1) {
  await act(async () => { mocks.listener?.({ itemId: id, stateVersion, editVersion: 1 }); });
}
function expectFullCalls(times: number) { for (const call of [mocks.listJobs, mocks.listLibraryExams, mocks.listTrashedExams, mocks.listLibraryItems]) expect(call).toHaveBeenCalledTimes(times); }
beforeEach(() => {
  vi.clearAllMocks();
  mocks.listener = undefined;
  mocks.subscribe.mockImplementation(async (listener) => { mocks.listener = listener; return mocks.unlisten; });
  mocks.listJobs.mockResolvedValue([{ jobId: "legacy", title: "Legacy title", status: "DraftSaved", currentStep: "Authoring", issueCounts: { errors: 0, needsReview: 0 }, category: "P2", updatedAt: "2026-10-04" }]);
  mocks.listLibraryExams.mockResolvedValue([{ id: "writing", title: "Writing title", subject: "writing", status: "ready", updatedAt: "2026-10-04" }]);
  mocks.listTrashedExams.mockResolvedValue([{ id: "trash", title: "Trash title", subject: "reading", status: "ready", updatedAt: "2026-10-04" }]);
  mocks.listLibraryItems.mockResolvedValue([item()]);
  mocks.getLibraryRow.mockResolvedValue(row());
});
afterEach(cleanup);
describe("library event reads", () => {
  it("subscribes before one initial four-source load, retaining legacy/writing/trash rows", async () => {
    const subscription = deferred<() => void>();
    mocks.subscribe.mockImplementation((listener) => { mocks.listener = listener; return subscription.promise; });
    const { result } = renderHook(useLibraryStore);
    expectFullCalls(0);
    await act(async () => { subscription.resolve(mocks.unlisten); });
    await waitFor(() => expect(result.current.loading).toBe(false));
    expectFullCalls(1);
    expect(result.current.rows.find((r) => r.id === "legacy")?.category).toBe("P2");
    expect(result.current.rows.find((r) => r.id === "writing")?.modality).toBe("writing");
    expect(result.current.rows.find((r) => r.id === "trash")?.inTrash).toBe(true);
  });
  it("five same-edit-version stage events use five single-row reads and preserve unrelated rows", async () => {
    const { result } = renderHook(useLibraryStore);
    await waitFor(() => expect(result.current.loading).toBe(false));
    for (let index = 1; index <= 5; index++) await emit("item", index);
    expect(mocks.getLibraryRow).toHaveBeenCalledTimes(5);
    expectFullCalls(1);
    expect(result.current.rows.find((r) => r.id === "item")?.part).toBe("P3");
    expect(result.current.rows.find((r) => r.id === "writing")?.title).toBe("Writing title");
  });
  it("an event during initial snapshot is replayed after it, so its terminal row wins", async () => {
    const snapshot = deferred<unknown[]>(); mocks.listLibraryItems.mockReturnValue(snapshot.promise);
    mocks.getLibraryRow.mockResolvedValue(row("cancelled"));
    const { result } = renderHook(useLibraryStore);
    await waitFor(() => expect(mocks.listLibraryItems).toHaveBeenCalledTimes(1));
    await emit("item", 3);
    expect(mocks.getLibraryRow).not.toHaveBeenCalled();
    await act(async () => { snapshot.resolve([item()]); });
    await waitFor(() => expect(result.current.rows.find((r) => r.id === "item")?.detail).toBe("已取消"));
    expectFullCalls(1); expect(mocks.getLibraryRow).toHaveBeenCalledTimes(1);
  });
  it("a terminal event during an in-flight row read cannot be overwritten by the older result", async () => {
    const earlier = deferred<unknown>(); mocks.getLibraryRow.mockReturnValueOnce(earlier.promise).mockResolvedValue(row("ready_for_review"));
    const { result } = renderHook(useLibraryStore);
    await waitFor(() => expect(result.current.loading).toBe(false));
    await emit("item", 1); await emit("item", 2);
    expect(mocks.getLibraryRow).toHaveBeenCalledTimes(1);
    await act(async () => { earlier.resolve(row("local_recognition")); });
    await waitFor(() => expect(result.current.rows.find((r) => r.id === "item")?.stage).toBe("ready"));
    expect(mocks.getLibraryRow).toHaveBeenCalledTimes(2); expectFullCalls(1);
  });
  it("manual and focus refresh remain full snapshots; item events can add or remove rows", async () => {
    const { result } = renderHook(useLibraryStore);
    await waitFor(() => expect(result.current.loading).toBe(false));
    await act(async () => { result.current.refresh(); }); expectFullCalls(2);
    await act(async () => { window.dispatchEvent(new Event("focus")); }); expectFullCalls(3);
    mocks.getLibraryRow.mockResolvedValueOnce(row("ready_for_review", "new")); await emit("new", 1);
    expect(result.current.rows.some((r) => r.id === "new")).toBe(true);
    mocks.getLibraryRow.mockResolvedValueOnce(null); await emit("new", 2);
    expect(result.current.rows.some((r) => r.id === "new")).toBe(false);
  });
  it("a failed single-row read falls back to a full snapshot rather than hiding a terminal update", async () => {
    const { result } = renderHook(useLibraryStore);
    await waitFor(() => expect(result.current.loading).toBe(false));
    mocks.getLibraryRow.mockRejectedValueOnce(new Error("unavailable"));
    mocks.listLibraryItems.mockResolvedValue([{ ...item("item", "failed"), processing: row("cancelled").item.processing }]);
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    await emit("item", 9);
    await waitFor(() => expect(result.current.rows.find((r) => r.id === "item")?.detail).toBe("已取消"));
    expectFullCalls(2); consoleError.mockRestore();
  });
});
