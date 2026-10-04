// @vitest-environment jsdom
//
// 识别中的后台草稿刷新 vs 已完成的拖动手势（2026-09-26 验收缺陷）。
//
// 缺陷（真实应用复现）：导入后立刻打开工作区（顶部显示「正在本机识别…」），拖动选项行
// 手柄、松手后既不保存也不报错，顺序不变；识别结束后再拖都正常。识别进行中，后台草稿
// 刷新（processing 事件 → 编辑器重拉 → `editor.draft` 被替换成新对象）会打断一次已完成的
// 拖动手势：
//   - 画布上的拖动会话挂在每个手柄上，行被重建时手柄卸载、cleanup 直接取消会话——
//     松手静默无动作；
//   - 就算会话活着，ExamWorkspacePage 的 onStructureAction 用**渲染时闭包**里的
//     `editor.draft!` 编译动作——用旧选项列表生成补丁，会覆盖后台刚写入的内容。
//
// 两条用例的先红语义（对当前实现都是红的）：
//  1) 拖动中草稿以**新对象身份**刷新（同 id、新引用、页面重渲染后传下新的
//     onStructureAction）→ 松手必须把动作提交给**最新**的回调；
//  2) 拖动中草稿刷新把被拖选项删掉 → 工作区必须经现有错误提示通道（showError →
//     `.workspace-notice`）响亮失败，且不提交任何补丁。
//
// 证据层级：组件级。渲染真实 ExamCanvas / ExamWorkspacePage，驱动手柄的 pointer 事件，
// 用真实 compileStructureAction 校验顺序；不经过 Tauri/WebView2，桌面端行为由
// scripts/e2e/tauri-cdp-option-drag.mjs 在真实应用里验收。

import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ExamCanvas } from "./ExamCanvas";
import { compileStructureAction } from "./structureActions";
import { ExamWorkspacePage } from "../features/editor/ExamWorkspacePage";
import type { AnswerSlotV2, IeltsAuthoringIRV2, OptionV2, TaskGroupV2 } from "../types";

vi.mock("../api/tauriCommands", () => ({
  resolveAuthoringAssetPreview: vi.fn(async () => undefined),
  command: vi.fn(async () => ({})),
  getJob: vi.fn(async () => null),
}));

// ── ExamWorkspacePage 的依赖替身（与 ExamWorkspacePage.repairAids.test.tsx 同一套路）──

const editorRef = vi.hoisted(() => ({ current: null as unknown }));

vi.mock("../features/editor/useCanonicalEditor", () => ({
  useCanonicalEditor: vi.fn(() => editorRef.current),
}));

vi.mock("../api/recognitionClient", () => ({
  getRecognitionDecision: vi.fn(async () => ({ repair: undefined })),
}));

vi.mock("../api/processingClient", () => ({
  subscribeProcessing: vi.fn(async () => () => {}),
  describeRetryOutcome: vi.fn(() => ""),
  retryAnswerPageRecognition: vi.fn(),
  retryProcessing: vi.fn(),
  cancelProcessing: vi.fn(),
}));

vi.mock("../api/desktopDialogs", () => ({ chooseExportDirectory: vi.fn() }));

vi.mock("../api/publishClient", () => ({
  describePublishOutcome: vi.fn(() => ""),
  publishItem: vi.fn(),
  publishOutcomeKind: vi.fn(() => "success"),
}));

vi.mock("../api/workspaceClient", () => ({
  getWorkspaceItem: vi.fn(async () => ({ item: { sourcePurged: false } })),
  getPublishPreflight: vi.fn(async () => null),
  listLibraryItems: vi.fn(async () => []),
  getLibraryItemProcessing: vi.fn(async () => null),
  applyEditorCommands: vi.fn(),
}));

vi.mock("../features/settings/appSettings", () => ({
  readAppSettings: vi.fn(() => ({ nasDestination: "", developerMode: false })),
  writeAppSettings: vi.fn(),
}));

vi.mock("../app/router", () => ({ go: vi.fn(), libraryPath: vi.fn(() => "/library") }));

vi.mock("../features/editor/finalVersion", () => ({
  SOURCE_PURGED_EXPLANATION: "",
  saveToLibrary: vi.fn(),
  sourceActionsAvailable: vi.fn(() => true),
}));

vi.mock("../features/editor/SelectionInspector", () => ({ SelectionInspector: () => null }));
vi.mock("../features/editor/RecognitionPanel", () => ({ RecognitionPanel: () => null }));

vi.mock("../features/editor/studentPreview", () => ({
  compilePreviewSource: vi.fn(() => ({ ok: true, summary: { answerKeyIssues: [] } })),
  describePreviewPublishLimitation: vi.fn(() => ({ message: "", level: "info" })),
}));

// ── 草稿构造（与 optionReorder.test.tsx 同一份构造方式）──

function option(optionId: string, label: string, text: string): OptionV2 {
  return { optionId, label, content: [{ id: `t-${optionId}`, type: "text", text, sourceAnchors: [], provenanceStatus: "source" }], sourceAnchors: [] };
}

const slot: AnswerSlotV2 = {
  slotId: "q1", questionNumber: 1, displayLabel: "1", hostType: "prompt", interaction: "radio",
  participation: "scoring", sourceAnchors: [], confidence: 1
};

function taskWithOptions(options: OptionV2[]): TaskGroupV2 {
  return {
    taskId: "task-1",
    taskType: "multiple_choice",
    displayRange: { kind: "range", start: 1, end: 1 },
    instructions: [],
    instructionSignature: { normalizedText: "", taskType: "multiple_choice", expectedQuestionNumbers: [], expectedSlotCount: 0, evidenceAnchors: [], confidence: 1 },
    responseGroups: [{
      responseGroupId: "rg-1", kind: "choice", slotIds: ["q1"], optionBankRef: "bank-1",
      cardinality: { min: 1, max: 1 }, assignment: "per_slot", scoringPolicy: "per_slot_binary",
      duplicatePolicy: "reject_submission", allowOptionReuse: false, sourceAnchors: []
    }],
    optionBank: { optionBankId: "bank-1", scope: "task_group", sourceAnchors: [], allowReuse: false, options },
    sourceAnchors: [],
    quality: { score: 1, sourceCoverage: 1, hardFailures: [] },
    reviewState: "unreviewed"
  };
}

function makeDraft(): IeltsAuthoringIRV2 {
  return {
    schemaVersion: "IeltsAuthoringIRV2",
    jobId: "job-1",
    exam: { examId: "exam-1", title: "T", language: "en", tags: [], sourceFiles: [] },
    modality: "reading",
    taskGroups: [taskWithOptions([option("o-a", "A", "alpha"), option("o-b", "B", "beta"), option("o-c", "C", "gamma")])],
    answerSlots: { q1: slot },
    answerKey: { q1: { kind: "option", labels: ["B"], assignment: "per_slot" } },
    assets: [],
    sourceDocumentId: "doc-1",
    quality: {
      schemaVersion: "QualityReportV2", state: "review_required", documentScore: 0, sourceCoverage: 0, coverageLedger: [],
      coverageStatus: { physicalShadow: "missing", complete: false, significantSourceNodeCount: 0, explainedSourceNodeCount: 0, unassignedSourceNodeIds: [] },
      compilerProbes: {
        v2Runtime: { status: "passed", schemaVersion: "", issueCodes: [], details: [] },
        v1Compatibility: { status: "passed", schemaVersion: "", issueCodes: [], details: [] }
      },
      taskScores: {}, hardFailures: [], issues: [], metrics: {}, evaluatedAt: "", evaluatorVersion: ""
    },
    audit: { revision: 1, source: "auto_extract", humanVerified: false, llmUsed: false, updatedAt: "", notes: [] }
  };
}

/** 后台刷新后的新草稿：所有对象都是新身份（与编辑器重拉 get_workspace_item 一致），
 *  选项 id 不变——这是刷新最常见的一档：内容相同/推进，但对象引用全新。 */
function makeRefreshedDraft(): IeltsAuthoringIRV2 {
  return makeDraft();
}

/** 后台刷新把被拖选项删掉的新草稿（识别改写/云端修复都会换掉整个选项库）。 */
function makeDraftWithoutC(): IeltsAuthoringIRV2 {
  return {
    ...makeDraft(),
    taskGroups: [taskWithOptions([option("o-a", "A", "alpha"), option("o-b", "B", "beta")])]
  };
}

/** jsdom 没有布局：给每个选项行一个 40px 高的假盒子，拖动落点才可计算。 */
function layOutRows(container: HTMLElement) {
  container.querySelectorAll<HTMLElement>("[data-option-row]").forEach((row, index) => {
    row.getBoundingClientRect = () => ({ top: index * 40, height: 40, bottom: index * 40 + 40, left: 0, right: 100, width: 100, x: 0, y: index * 40, toJSON: () => ({}) });
  });
}

function orderAfter(action: Parameters<typeof compileStructureAction>[1]) {
  const patch = compileStructureAction(makeRefreshedDraft(), action);
  if (patch?.op !== "setOptionBank") throw new Error(`unexpected patch ${JSON.stringify(patch)}`);
  return patch.optionBank!.options.map((entry) => `${entry.label}:${entry.optionId}`);
}

afterEach(cleanup);

describe("识别中的后台草稿刷新 vs 已完成的拖动手势", () => {
  it("拖动中草稿以新对象身份刷新（同 id 新引用）：松手后动作提交给最新的 onStructureAction，顺序正确", () => {
    const staleAction = vi.fn();
    const latestAction = vi.fn();
    const { container, rerender } = render(
      <ExamCanvas authoring={makeDraft()} mode="author" onStructureAction={staleAction} />
    );
    layOutRows(container);
    fireEvent.pointerDown(screen.getByLabelText(/拖动调整选项 C/), { button: 0 });
    fireEvent.pointerMove(document.body, { clientY: 5 });

    // 后台刷新：编辑器重拉，draft 全新对象、页面重渲染并传下新的 onStructureAction。
    rerender(<ExamCanvas authoring={makeRefreshedDraft()} mode="author" onStructureAction={latestAction} />);
    layOutRows(container);
    fireEvent.pointerUp(document.body);

    expect(latestAction).toHaveBeenCalledTimes(1);
    const action = latestAction.mock.calls[0][0];
    expect(action).toEqual({ type: "option.move", taskId: "task-1", responseGroupId: "rg-1", optionId: "o-c", beforeOptionId: "o-a" });
    // 动作对**新草稿**仍然成立：C 连同字母移到最前。
    expect(orderAfter(action)).toEqual(["C:o-c", "A:o-a", "B:o-b"]);
    // 提交给旧闭包等于用旧草稿覆盖后台写入——不允许。
    expect(staleAction).not.toHaveBeenCalled();
  });

  it("拖动中草稿刷新把被拖选项删掉：工作区出现错误提示，不提交任何补丁", async () => {
    const applyPatch = vi.fn();
    const makeEditor = (draft: IeltsAuthoringIRV2, version: number) => ({
      draft,
      version,
      pendingCount: 0,
      saveState: "idle",
      loading: false,
      loadError: undefined,
      conflictRecovering: false,
      saveNotice: undefined,
      saveMessage: undefined,
      deferredRemoteRefresh: undefined,
      title: "T",
      canUndo: false,
      canRedo: false,
      flush: vi.fn(async () => {}),
      reload: vi.fn(),
      noteRemoteVersion: vi.fn(),
      applyPatch,
      applyCommand: vi.fn(),
      undo: vi.fn(),
      redo: vi.fn(),
      setTitle: vi.fn(),
      recoverFromConflict: vi.fn(),
      dismissSaveNotice: vi.fn(),
      discardLocalChanges: vi.fn(),
    });
    let editor = makeEditor(makeDraft(), 1);
    editorRef.current = editor;
    const { container, rerender } = render(<ExamWorkspacePage itemId="item-1" />);
    await act(async () => {});
    layOutRows(container);

    fireEvent.pointerDown(screen.getByLabelText(/拖动调整选项 C/), { button: 0 });
    fireEvent.pointerMove(document.body, { clientY: 5 });

    // 后台刷新：draft 换成没有 C 的新草稿（页面用 editor.draft 重渲染）。
    editor = makeEditor(makeDraftWithoutC(), 2);
    editorRef.current = editor;
    rerender(<ExamWorkspacePage itemId="item-1" />);
    await act(async () => {});
    layOutRows(container);
    fireEvent.pointerUp(document.body);
    await act(async () => {});

    // 必须经现有错误提示通道响亮失败，不允许静默。
    const notice = document.querySelector(".workspace-notice");
    expect(notice).not.toBeNull();
    expect(notice?.textContent ?? "").toContain("这次移动没有生效");
    // 也不允许把任何补丁提交出去（用旧草稿编译出的补丁会覆盖后台刚写入的内容）。
    expect(applyPatch).not.toHaveBeenCalled();
  });
});
