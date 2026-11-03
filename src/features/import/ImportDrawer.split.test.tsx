// @vitest-environment jsdom
// Real drawer component, mocked IPC: a whole-test PDF shows the passage split and sends `splitPassages`.
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ planImportSplit: vi.fn(), detectImportModality: vi.fn(), chooseSourceFiles: vi.fn() }));
vi.mock("../../api/tauriCommands", () => ({ listLlmProfiles: vi.fn().mockResolvedValue([]) }));
vi.mock("../../api/listeningAudioClient", () => ({ detectImportModality: mocks.detectImportModality }));
vi.mock("../../api/desktopDialogs", () => ({
  chooseSourceFiles: mocks.chooseSourceFiles, choosePdfFolderSources: vi.fn(), listenForFileDrops: vi.fn().mockResolvedValue(() => {})
}));
vi.mock("../../api/processingClient", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../api/processingClient")>()),
  planImportSplit: mocks.planImportSplit
}));
import { ImportDrawer } from "./ImportDrawer";

const whole = { path: "/papers/Test 1.pdf", name: "Test 1.pdf", sizeBytes: 10 };
const single = { path: "/papers/P2.pdf", name: "P2.pdf", sizeBytes: 10 };

beforeEach(() => {
  vi.clearAllMocks();
  mocks.chooseSourceFiles.mockResolvedValue([whole, single]);
  mocks.detectImportModality.mockResolvedValue([]);
  mocks.planImportSplit.mockResolvedValue([
    {
      path: whole.path,
      pageCount: 9,
      segments: [
        { label: "P1", passage: 1, pages: [1, 2, 3], sharesStartPage: false },
        { label: "P2", passage: 2, pages: [4, 5, 6], sharesStartPage: false },
        { label: "P3", passage: 3, pages: [7, 8], sharesStartPage: false }
      ],
      answerPages: [9],
      unassignedPages: []
    },
    { path: single.path, pageCount: 3, segments: [], answerPages: [], unassignedPages: [] }
  ]);
});
afterEach(cleanup);

async function pickAndStart(onImport: ReturnType<typeof vi.fn>) {
  render(<ImportDrawer open busy={false} rejected={[]} onClose={() => {}} onImport={onImport} />);
  fireEvent.click(screen.getByTestId("import-pick-files"));
  await screen.findByText("Test 1.pdf");
  fireEvent.click(screen.getByTestId("import-start"));
  await screen.findByTestId("import-split-plan");
}

describe("ImportDrawer whole-test split", () => {
  it("shows the detected passages and splits by default", async () => {
    const onImport = vi.fn();
    await pickAndStart(onImport);
    expect(onImport).not.toHaveBeenCalled();
    expect(screen.getAllByTestId("import-split-file")).toHaveLength(1);
    expect(screen.getByText(/识别到 3 篇，拆成 3 条/)).toBeTruthy();
    expect(screen.getByText("第 1–3 页")).toBeTruthy();
    expect(screen.getByText(/答案页（第 9 页）会附到每一篇/)).toBeTruthy();
    expect(screen.getByTestId("import-start").textContent).toBe("确认导入");
    expect(mocks.planImportSplit).toHaveBeenCalledWith([whole.path, single.path]);

    fireEvent.click(screen.getByTestId("import-start"));
    await waitFor(() => expect(onImport).toHaveBeenCalledTimes(1));
    const [files] = onImport.mock.calls[0];
    expect(files).toEqual([{ ...whole, splitPassages: true }, single]);
  });

  it("imports the file whole when the user unticks the split", async () => {
    const onImport = vi.fn();
    await pickAndStart(onImport);
    fireEvent.click(screen.getByTestId("import-split-toggle"));
    expect(screen.getByText(/不拆分，整份作为一条导入/)).toBeTruthy();
    fireEvent.click(screen.getByTestId("import-start"));
    await waitFor(() => expect(onImport).toHaveBeenCalledTimes(1));
    expect(onImport.mock.calls[0][0]).toEqual([whole, single]);
  });

  it("never plans a split for files the user confirms as listening", async () => {
    mocks.detectImportModality.mockResolvedValue([{ path: whole.path, modality: "listening", cues: ["listening:four_parts"] }]);
    mocks.planImportSplit.mockResolvedValue([]);
    const onImport = vi.fn();
    render(<ImportDrawer open busy={false} rejected={[]} onClose={() => {}} onImport={onImport} />);
    fireEvent.click(screen.getByTestId("import-pick-files"));
    await screen.findByText("Test 1.pdf");
    fireEvent.click(screen.getByTestId("import-start"));
    await waitFor(() => expect(mocks.planImportSplit).toHaveBeenCalledWith([single.path]));
  });
});
