// @vitest-environment jsdom
// Real drawer component, mocked IPC: a whole-test PDF shows the passage split and sends `splitPassages`.
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ planImportSplit: vi.fn(), detectImportModality: vi.fn(), chooseSourceFiles: vi.fn(), listLlmProfiles: vi.fn() }));
vi.mock("../../api/tauriCommands", () => ({ listLlmProfiles: mocks.listLlmProfiles, getPipelineSettings: vi.fn().mockResolvedValue({ pipelineMode: "direct" }) }));
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

const answers = (first: number, last: number) =>
  Array.from({ length: last - first + 1 }, (_, i) => ({ number: first + i, text: "TRUE" }));
const segment = (passage: number, pages: number[], range: [number, number], extra: object = {}) => ({
  label: `P${passage}`, passage, pages, sharesStartPage: false, questionRange: range,
  signals: ["heading", "instruction", "numbering", "band"], confidence: "high", answers: answers(...range),
  fallbackAnswerPages: [], ...extra
});
function plan(overrides: object = {}) {
  return {
    path: whole.path,
    pageCount: 9,
    segments: [segment(1, [1, 2, 3], [1, 13]), segment(2, [4, 5, 6], [14, 26]), segment(3, [7, 8], [27, 40])],
    answerPages: [9],
    unassignedPages: [],
    defaultSplit: true,
    warnings: [],
    ...overrides
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.listLlmProfiles.mockResolvedValue([]);
  mocks.chooseSourceFiles.mockResolvedValue([whole, single]);
  mocks.detectImportModality.mockResolvedValue([]);
  mocks.planImportSplit.mockResolvedValue([plan(), { ...plan(), path: single.path, pageCount: 3, segments: [] }]);
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
  it("lets cloud direct discover passages while keeping selected source PDFs intact", async () => {
    mocks.listLlmProfiles.mockResolvedValue([{ profileId:"cloud",enabled:true }]);
    const onImport = vi.fn();
    render(<ImportDrawer open busy={false} rejected={[]} onClose={() => {}} onImport={onImport} />);
    await screen.findByTestId("import-cloud-multi");
    fireEvent.click(screen.getByTestId("import-pick-files"));
    await screen.findByText("Test 1.pdf");
    fireEvent.click(screen.getByTestId("import-start"));
    await waitFor(() => expect(onImport).toHaveBeenCalledTimes(1));
    expect(mocks.planImportSplit).not.toHaveBeenCalled();
    expect(onImport.mock.calls[0][0]).toEqual([whole,single]);
  });
  it("shows the detected passages and splits by default", async () => {
    const onImport = vi.fn();
    await pickAndStart(onImport);
    expect(onImport).not.toHaveBeenCalled();
    expect(screen.getAllByTestId("import-split-file")).toHaveLength(1);
    expect(screen.getByText(/识别到 3 篇，拆成 3 条/)).toBeTruthy();
    expect(screen.getByText("第 1–3 页")).toBeTruthy();
    expect(screen.getAllByText("答案 13 条（已从答案页按题号分出）")).toHaveLength(2);
    expect(screen.getByText("题 14–26")).toBeTruthy();
    expect(screen.getAllByText("依据：篇章标题、说明行、题号接力、题号区间")).toHaveLength(3);
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

  it("defaults to whole import when the signals disagree, but lets the user opt in", async () => {
    mocks.planImportSplit.mockResolvedValue([plan({
      defaultSplit: false,
      warnings: ["P3：标题写的是 Passage 3，题号范围却像另一篇，请核对。"],
      segments: [
        segment(1, [1, 2], [1, 13]),
        segment(3, [3, 4], [14, 26], { confidence: "low", signals: ["heading"], answers: [], fallbackAnswerPages: [9] })
      ]
    })]);
    const onImport = vi.fn();
    await pickAndStart(onImport);
    expect(screen.getByTestId("import-split-weak")).toBeTruthy();
    expect(screen.getByText(/识别到 2 篇 · 不拆分/)).toBeTruthy();
    expect(screen.getByText("把握低")).toBeTruthy();
    expect(screen.getByText("答案未能分出，附第 9 页答案页")).toBeTruthy();
    expect(screen.getByText(/题号范围却像另一篇/)).toBeTruthy();
    fireEvent.click(screen.getByTestId("import-split-toggle"));
    fireEvent.click(screen.getByTestId("import-start"));
    await waitFor(() => expect(onImport).toHaveBeenCalledTimes(1));
    expect(onImport.mock.calls[0][0]).toEqual([{ ...whole, splitPassages: true }, single]);
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

  it("shows failed preflight instead of silently importing whole, and allows rechecking", async () => {
    mocks.planImportSplit.mockRejectedValueOnce(new Error("IPC failed"));
    const onImport = vi.fn();
    render(<ImportDrawer open busy={false} rejected={[]} onClose={() => {}} onImport={onImport} />);
    fireEvent.click(screen.getByTestId("import-pick-files"));
    await screen.findByText("Test 1.pdf");
    fireEvent.click(screen.getByTestId("import-start"));
    await screen.findByTestId("import-split-warning");
    expect(onImport).not.toHaveBeenCalled();
    expect(screen.getAllByText(/分篇检查未完成/)).toHaveLength(2);
    fireEvent.click(screen.getByText("重新检查"));
    await screen.findByTestId("import-split-plan");
    fireEvent.click(screen.getByTestId("import-start"));
    await waitFor(() => expect(onImport).toHaveBeenCalledTimes(1));
    expect(onImport.mock.calls[0][0][0].splitPassages).toBe(true);
  });

  it("keeps unreadable-page warnings visible even without detected segments", async () => {
    mocks.planImportSplit.mockResolvedValue([plan({ segments: [], defaultSplit: false, warnings: ["部分页面没有可读文字，需云端图像识别或人工核对分篇。"] })]);
    const onImport = vi.fn();
    render(<ImportDrawer open busy={false} rejected={[]} onClose={() => {}} onImport={onImport} />);
    fireEvent.click(screen.getByTestId("import-pick-files"));
    await screen.findByText("Test 1.pdf");
    fireEvent.click(screen.getByTestId("import-start"));
    await screen.findByTestId("import-split-warning");
    expect(onImport).not.toHaveBeenCalled();
    expect(screen.getByText(/部分页面没有可读文字/)).toBeTruthy();
    fireEvent.click(screen.getByTestId("import-start"));
    await waitFor(() => expect(onImport).toHaveBeenCalledTimes(1));
    expect(onImport.mock.calls[0][0]).toEqual([whole, single]);
  });

  it("checks passage splitting after a listening guess is corrected to reading", async () => {
    mocks.chooseSourceFiles.mockResolvedValue([whole]);
    mocks.detectImportModality.mockResolvedValue([{ path: whole.path, modality: "listening", cues: ["listening:four_parts"] }]);
    mocks.planImportSplit.mockImplementation(async (paths: string[]) => paths.length ? [plan()] : []);
    const onImport = vi.fn();
    render(<ImportDrawer open busy={false} rejected={[]} onClose={() => {}} onImport={onImport} />);
    fireEvent.click(screen.getByTestId("import-pick-files"));
    await screen.findByText("Test 1.pdf");
    fireEvent.click(screen.getByTestId("import-start"));
    await screen.findByTestId("listening-switch-reading");
    fireEvent.click(screen.getByTestId("listening-switch-reading"));
    await screen.findByTestId("import-split-plan");
    expect(onImport).not.toHaveBeenCalled();
    expect(mocks.planImportSplit).toHaveBeenCalledWith([whole.path]);
    fireEvent.click(screen.getByTestId("import-start"));
    await waitFor(() => expect(onImport).toHaveBeenCalledTimes(1));
    expect(onImport.mock.calls[0][0]).toEqual([{ ...whole, splitPassages: true }]);
    expect(onImport.mock.calls[0][1][whole.path].modality).toBe("reading");
  });

  it("reports an invalid preflight response and leaves checking state", async () => {
    mocks.planImportSplit.mockResolvedValue(null);
    const onImport = vi.fn();
    render(<ImportDrawer open busy={false} rejected={[]} onClose={() => {}} onImport={onImport} />);
    fireEvent.click(screen.getByTestId("import-pick-files"));
    await screen.findByText("Test 1.pdf");
    fireEvent.click(screen.getByTestId("import-start"));
    await screen.findByTestId("import-split-warning");
    expect(screen.getByTestId("import-start").textContent).toBe("确认导入");
    expect(onImport).not.toHaveBeenCalled();
  });

  it("rechecks a corrected reading file without losing an earlier split or listening choice", async () => {
    const listening = { path: "/papers/audio.pdf", name: "audio.pdf", sizeBytes: 10 };
    mocks.chooseSourceFiles.mockResolvedValue([whole, listening, single]);
    mocks.detectImportModality.mockResolvedValue([listening, single].map(file => ({ path: file.path, modality: "listening", cues: ["listening:four_parts"] })));
    mocks.planImportSplit.mockResolvedValueOnce([plan()]).mockRejectedValueOnce(new Error("temporary"))
      .mockResolvedValueOnce([plan({ path: single.path })]);
    const onImport = vi.fn();
    await pickAndStart(onImport);
    fireEvent.click(screen.getByTestId("import-start"));
    await screen.findByTestId("listening-audio-later");
    fireEvent.click(screen.getByTestId("listening-audio-later"));
    await screen.findByTestId("listening-switch-reading");
    fireEvent.click(screen.getByTestId("listening-switch-reading"));
    await screen.findByTestId("import-split-warning");
    fireEvent.click(screen.getByText("重新检查"));
    await screen.findByTestId("import-split-plan");
    expect(mocks.detectImportModality).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByTestId("import-start"));
    await waitFor(() => expect(onImport).toHaveBeenCalledTimes(1));
    expect(onImport.mock.calls[0][0]).toEqual([{ ...whole, splitPassages: true }, listening, { ...single, splitPassages: true }]);
    expect(onImport.mock.calls[0][1][listening.path].modality).toBe("listening");
  });

  it("prevents leaving for cloud settings while preflight is in progress", async () => {
    mocks.planImportSplit.mockReturnValue(new Promise(() => {}));
    const onClose = vi.fn();
    render(<ImportDrawer open busy={false} rejected={[]} onClose={onClose} onImport={vi.fn()} />);
    fireEvent.click(screen.getByTestId("import-pick-files"));
    await screen.findByText("Test 1.pdf");
    fireEvent.click(screen.getByTestId("import-start"));
    await waitFor(() => expect(mocks.planImportSplit).toHaveBeenCalled());
    const connect = screen.getByText("去连接") as HTMLButtonElement;
    expect(connect.disabled).toBe(true);
    fireEvent.click(connect);
    expect(onClose).not.toHaveBeenCalled();
  });
});
