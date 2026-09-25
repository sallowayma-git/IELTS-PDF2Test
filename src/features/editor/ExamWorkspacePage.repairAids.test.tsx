// @vitest-environment jsdom
//
// 云端修复剩余条目（`repairAids`）的刷新行为（2026-09-25 W2 返工）。
//
// 这是本文件里第一条**组件级**用例：被测对象是 ExamWorkspacePage 里「读取云端修复
// 剩余条目」的那段 effect 与它渲染出来的清单 DOM，mock 的只是它依赖的命令客户端
// （getRecognitionDecision / 命令层 / 编辑器 hook），「读取 → 并入清单」的真实链路
// （含 `buildEditingAids` 与 `data-task-id` 渲染契约）保持真实——那正是 W2 缺条目
// 事故的发生地。
//
// 三条用例的红绿语义（与质量方返工单一一对应）：
//  1. `running` 之后不再有事件 ⇒ 1.5 秒后的重读必须拿到终态并把 cloud-question 条目
//     送进清单。回退修复（回到「读到 running 永久空、无人重读」的实现）后此条变红。
//  2. 清单已有条目时一次读取失败 ⇒ 条目必须保留（旧实现 `.catch` 直接置空）。
//     回退修复后此条变红。
//  3. 新一轮修复开跑（running）⇒ 上一轮留下的条目必须立刻清掉（「修复进行中不挂
//     任何云端条目」）。这条对着 984612f 提交时的实现就是红的（running 分支漏了
//     `setRepairAids([])`，旧条目挂着可点），是本次返工的先红测试。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { RecognitionDecisionViewV1 } from "../../api/recognitionClient";
import { getRecognitionDecision } from "../../api/recognitionClient";
import { ExamWorkspacePage } from "./ExamWorkspacePage";
import { useCanonicalEditor } from "./useCanonicalEditor";

const editorRef = vi.hoisted(() => ({ current: null as unknown }));

vi.mock("./useCanonicalEditor", () => ({
  useCanonicalEditor: vi.fn(() => editorRef.current),
}));

vi.mock("../../api/recognitionClient", () => ({
  getRecognitionDecision: vi.fn(),
}));

vi.mock("../../api/tauriCommands", () => ({
  command: vi.fn(async () => ({})),
  getJob: vi.fn(async () => null),
}));

vi.mock("../../api/processingClient", () => ({
  subscribeProcessing: vi.fn(async () => () => {}),
  describeRetryOutcome: vi.fn(() => ""),
  retryAnswerPageRecognition: vi.fn(),
  retryProcessing: vi.fn(),
  cancelProcessing: vi.fn(),
}));

vi.mock("../../api/desktopDialogs", () => ({
  chooseExportDirectory: vi.fn(),
}));

vi.mock("../../api/publishClient", () => ({
  describePublishOutcome: vi.fn(() => ""),
  publishItem: vi.fn(),
  publishOutcomeKind: vi.fn(() => "success"),
}));

vi.mock("../../api/workspaceClient", () => ({
  getWorkspaceItem: vi.fn(async () => null),
  getPublishPreflight: vi.fn(async () => null),
  listLibraryItems: vi.fn(async () => []),
}));

vi.mock("../settings/appSettings", () => ({
  readAppSettings: vi.fn(async () => ({})),
  writeAppSettings: vi.fn(),
}));

vi.mock("../../app/router", () => ({
  go: vi.fn(),
  libraryPath: vi.fn(() => "/library"),
}));

vi.mock("./finalVersion", () => ({
  SOURCE_PURGED_EXPLANATION: "",
  saveToLibrary: vi.fn(),
  sourceActionsAvailable: vi.fn(() => true),
}));

vi.mock("../../exam-canvas/ExamCanvas", () => ({ ExamCanvas: () => null }));
vi.mock("./SelectionInspector", () => ({ SelectionInspector: () => null }));
vi.mock("./RecognitionPanel", () => ({ RecognitionPanel: () => null }));

vi.mock("./studentPreview", () => ({
  compilePreviewSource: vi.fn(() => ({ ok: true, summary: { answerKeyIssues: [] } })),
  describePreviewPublishLimitation: vi.fn(() => null),
}));

const ITEM_ID = "item-1";

/** 编辑器 hook 的最小替身：字段面 = ExamWorkspacePage 实际读到的那些。 */
function makeEditor(version: number) {
  return {
    draft: { exam: { title: "受控卷" }, taskGroups: [], answerSlots: {}, answerKey: {} },
    version,
    pendingCount: 0,
    saveState: "idle",
    loading: false,
    loadError: undefined,
    conflictRecovering: false,
    saveNotice: undefined,
    saveMessage: "",
    title: "",
    canUndo: false,
    canRedo: false,
    deferredRemoteRefresh: false,
    flush: vi.fn(async () => {}),
    reload: vi.fn(),
    noteRemoteVersion: vi.fn(),
    applyPatch: vi.fn(),
    applyCommand: vi.fn(),
    undo: vi.fn(),
    redo: vi.fn(),
    setTitle: vi.fn(),
    recoverFromConflict: vi.fn(),
    dismissSaveNotice: vi.fn(),
    discardLocalChanges: vi.fn(),
  };
}

/** 一条云端交还用户的疑问（cloud-question）剩余任务——W2 事故里丢掉的就是它。 */
const claimTask = {
  userTaskId: "cloud-question:q27:1",
  action: "answer",
  blocking: false,
  targetIds: ["q27"],
  message:
    "云端未能确认：第 27 题的答案无法核实：原文件里没有答案页（抓取核对过），云端不能编造答案。请对照原文件或自行填写。",
};

function decision(status: string, remainingTasks: unknown[] = []): RecognitionDecisionViewV1 {
  return {
    schemaVersion: "RecognitionDecisionViewV1",
    itemId: ITEM_ID,
    batchId: "batch-1",
    baseEditVersion: 1,
    currentEditVersion: 1,
    stale: false,
    localStatus: status,
    cloudStatus: status,
    sourceStatus: "succeeded",
    adjudicationStatus: "succeeded",
    repair: { status, appliedCount: 0, rounds: 1, adjudicatedCount: 0, remainingTasks },
  } as unknown as RecognitionDecisionViewV1;
}

/** 清单里当前渲染出来的条目 id（组件与 CDP 链共用的 `data-task-id` 契约）。 */
function renderedTaskIds(): string[] {
  return [...document.querySelectorAll("[data-task-id]")].map(
    (element) => element.getAttribute("data-task-id") ?? "",
  );
}

/** 打开「待补充」侧栏（清单收起时条目不在 DOM 里，与产品行为一致）。
 *  页面里有两个同 testid 的开关（桌面/移动两处布局），点第一个即可。 */
async function openIssueList() {
  const toggles = screen.getAllByTestId("workspace-issues");
  fireEvent.click(toggles[0]);
  await act(async () => {});
}

beforeEach(() => {
  vi.useFakeTimers();
  editorRef.current = makeEditor(1);
});

afterEach(() => {
  // vitest 没开 globals ⇒ RTL 的自动清理没有注册，必须显式 cleanup：
  // 否则上一个用例的组件实例还挂在 document 里，清单断言会串到旧实例。
  cleanup();
  vi.useRealTimers();
  vi.clearAllMocks();
});

describe("ExamWorkspacePage 的云端修复剩余条目刷新", () => {
  it("修复进行中读到 running 时安排重读：1.5 秒后的终态把 cloud-question 条目送进清单（期间不触发任何版本或处理事件）", async () => {
    vi.mocked(getRecognitionDecision)
      .mockResolvedValueOnce(decision("running"))
      .mockResolvedValue(decision("completed", [claimTask]));

    const { container } = render(<ExamWorkspacePage itemId={ITEM_ID} />);
    await act(async () => {});
    await openIssueList();

    expect(renderedTaskIds()).not.toContain("cloud-question:q27:1");

    // 假时钟推进 1.5 秒 = 产品里那次定时重读；没有版本 / 处理事件，重读是唯一机会。
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1500);
    });

    expect(renderedTaskIds()).toContain("cloud-question:q27:1");
    expect(vi.mocked(getRecognitionDecision)).toHaveBeenCalledTimes(2);
  });

  it("清单已有条目时一次读取失败：条目保留，随后重读成功仍在前", async () => {
    vi.mocked(getRecognitionDecision)
      .mockResolvedValueOnce(decision("completed", [claimTask]))
      .mockRejectedValueOnce(new Error("ipc hiccup"))
      .mockResolvedValue(decision("completed", [claimTask]));

    const { container, rerender } = render(<ExamWorkspacePage itemId={ITEM_ID} />);
    await act(async () => {});
    await openIssueList();
    expect(renderedTaskIds()).toContain("cloud-question:q27:1");

    // 版本推进触发一次重读，这次读取失败——旧实现会把清单洗成空。
    editorRef.current = makeEditor(2);
    rerender(<ExamWorkspacePage itemId={ITEM_ID} />);
    await act(async () => {});

    expect(renderedTaskIds()).toContain("cloud-question:q27:1");

    // 失败安排的重读到点执行，条目依旧在。
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1500);
    });
    expect(renderedTaskIds()).toContain("cloud-question:q27:1");
  });

  it("新一轮修复开跑（running）：上一轮留下的条目立刻清掉，收尾后随新终态回来", async () => {
    vi.mocked(getRecognitionDecision)
      .mockResolvedValueOnce(decision("completed", [claimTask]))
      .mockResolvedValueOnce(decision("running"))
      .mockResolvedValue(decision("completed", [claimTask]));

    const { container, rerender } = render(<ExamWorkspacePage itemId={ITEM_ID} />);
    await act(async () => {});
    await openIssueList();
    expect(renderedTaskIds()).toContain("cloud-question:q27:1");

    // 新一轮修复开跑：处理事件推进版本，effect 重读读到 running。
    editorRef.current = makeEditor(2);
    rerender(<ExamWorkspacePage itemId={ITEM_ID} />);
    await act(async () => {});

    // 「修复进行中不挂任何云端条目」：上一轮的旧条目不许再挂着让人点。
    expect(renderedTaskIds()).not.toContain("cloud-question:q27:1");

    // 修复收尾：重读拿到新终态，条目按新一轮的剩余任务回来。
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1500);
    });
    expect(renderedTaskIds()).toContain("cloud-question:q27:1");
  });
});
