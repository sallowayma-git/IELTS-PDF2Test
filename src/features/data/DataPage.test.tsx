// @vitest-environment jsdom
//
// 「数据」模块组件级测试：渲染真实 DataPage，分析接口用固定夹具替身。
// 证据层级：组件级（不经过 Tauri/SQLite）；产品级证据见 scripts/e2e/tauri-cdp-data-module.mjs。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import type { ExamView, Overview, QuestionView, StudentView } from "../../api/analyticsClient";

const exam: ExamView = {
  batchId: "B-2026-11",
  paperId: "paper-A",
  n: 4,
  excluded: [{ attemptId: "A9", candidateName: "半套", reason: "篇数不足（1/3）" }],
  hardConclusionMinN: 30,
  disclaimer: "机构模拟测试原始分，非 IELTS 官方成绩。",
  score: { mean: 7, median: 7.5, max: 12, min: 1, meanAccuracy: 0.583, maxTotal: 12 },
  histogram: [
    { from: 0, to: 1, count: 0 },
    { from: 1, to: 2, count: 1 },
    { from: 7, to: 8, count: 1 },
    { from: 8, to: 9, count: 1 },
    { from: 12, to: 13, count: 1 }
  ],
  parts: [
    { passageIndex: 0, label: "P1", n: 4, correct: 8, total: 16, accuracy: 0.5 },
    { passageIndex: 1, label: "P2", n: 4, correct: 11, total: 16, accuracy: 0.6875 }
  ],
  kinds: [{ kind: "true_false_not_given", n: 4, points: 16, correctPoints: 8, accuracy: 0.5 }],
  hardQuestions: [
    { passageIndex: 0, passageLabel: "P1", questionId: "p0q3", displayLabel: "Q3", questionKind: "true_false_not_given", n: 4, correctCount: 1, accuracy: 0.25, hardConclusion: false }
  ],
  questions: [
    { passageIndex: 0, passageLabel: "P1", questionId: "p0q1", displayLabel: "Q1", questionKind: "true_false_not_given", n: 4, correctCount: 3, accuracy: 0.75, hardConclusion: false },
    { passageIndex: 0, passageLabel: "P1", questionId: "p0q3", displayLabel: "Q3", questionKind: "true_false_not_given", n: 4, correctCount: 1, accuracy: 0.25, hardConclusion: false }
  ],
  matrix: [
    { attemptId: "A1", candidateName: "张三", cells: [true, false] },
    { attemptId: "A2", candidateName: "李四", cells: [true, null] }
  ],
  students: [
    { attemptId: "A1", candidateId: "C-001", candidateName: "张三", rawScore: 8, rawTotal: 12, accuracy: 0.667, partCorrect: [3, 4, 1], submittedAt: "2026-11-07T11:10:00+08:00", totalDurationSec: 3600 }
  ]
};

const student: StudentView = {
  attemptId: "A1",
  batchId: "B-2026-11",
  paperId: "paper-A",
  candidateId: "C-001",
  candidateName: "张三",
  submittedAt: "2026-11-07T11:10:00+08:00",
  valid: true,
  invalidReason: null,
  rawScore: 8,
  rawTotal: 12,
  accuracy: 0.667,
  passages: [{ passageIndex: 0, label: "P1", title: null, correct: 3, total: 4, accuracy: 0.75, durationSec: 1200, startedAt: null, endedAt: null, overlap: true }],
  overlapNote: "部分篇章的作答时间区间重叠，用时可能重叠，仅供参考。",
  wrongAnswers: [
    { passageIndex: 0, passageLabel: "P1", questionId: "p0q3", displayLabel: "Q3", questionKind: "true_false_not_given", userAnswer: "FALSE", correctAnswer: "TRUE", weight: 1, elapsedMs: 3000, changeCount: 1, visitCount: 2 }
  ],
  writings: [{ taskType: "task2", wordCount: 260, durationSec: 2400, submittedAt: "2026-11-07T11:10:00+08:00" }],
  disclaimer: "机构模拟测试原始分，非 IELTS 官方成绩。"
};

const question: QuestionView = {
  batchId: "B-2026-11",
  paperId: "paper-A",
  passageIndex: 0,
  passageLabel: "P1",
  questionId: "p0q1",
  displayLabel: "Q1",
  questionKind: "true_false_not_given",
  n: 4,
  correctCount: 3,
  accuracy: 0.75,
  hardConclusion: false,
  correctAnswer: "TRUE",
  avgElapsedMs: 2000,
  avgChangeCount: 0.5,
  distribution: [
    { answer: "TRUE", count: 3, share: 0.75, isCorrect: true },
    { answer: "FALSE", count: 1, share: 0.25, isCorrect: false }
  ],
  disclaimer: "机构模拟测试原始分，非 IELTS 官方成绩。"
};

const overview: Overview = {
  batches: [
    { batchId: "B-2026-11", paperId: "paper-A", checkedIn: 1, inProgress: 1, submittedBeacon: 4, voided: 1, imported: 4, validScored: 4, importExceptions: 3, lastSubmittedAt: "2026-11-07T11:40:00+08:00" }
  ],
  exceptions: [
    { file: "snapshots/T3/A5.sqlite", status: "failed", terminalId: "T3", attemptId: "A5", batchId: "B-2026-11", paperId: "paper-A", exportedAt: null, importedAt: "2026-11-07T12:00:00+08:00", error: "sha256 不符" }
  ],
  identityIssues: [
    { kind: "same_id_diff_name", candidateId: "C-002", candidateName: "李四", attempts: [{ attemptId: "A2", candidateId: "C-002", candidateName: "李四", batchId: "B-2026-11", paperId: "paper-A" }, { attemptId: "A3", candidateId: "C-002", candidateName: "李四四", batchId: "B-2026-11", paperId: "paper-A" }] }
  ],
  beaconErrors: [],
  beaconCount: 7,
  lastImportAt: "2026-11-07T12:00:00+08:00",
  disclaimer: "机构模拟测试原始分，非 IELTS 官方成绩。"
};

const api = vi.hoisted(() => ({
  overview: vi.fn(),
  exam: vi.fn(),
  student: vi.fn(),
  question: vi.fn(),
  importSnapshots: vi.fn(),
  exportXlsx: vi.fn()
}));

vi.mock("../../api/analyticsClient", () => ({ analyticsApi: api }));

import { DataPage } from "./DataPage";
import { go } from "../../app/router";

const SETTINGS_KEY = "ielts-author-studio.app-settings.v1";

function setRoot(root: string) {
  window.localStorage.setItem(SETTINGS_KEY, JSON.stringify({ submissionsRoot: root }));
}

beforeEach(() => {
  api.overview.mockReset().mockResolvedValue(overview);
  api.exam.mockReset().mockResolvedValue(exam);
  api.student.mockReset().mockResolvedValue(student);
  api.question.mockReset().mockResolvedValue(question);
  api.importSnapshots.mockReset();
  api.exportXlsx.mockReset().mockResolvedValue(null);
  window.localStorage.clear();
  window.location.hash = "#/data";
});

afterEach(() => {
  cleanup();
});

describe("数据总览", () => {
  it("未配置作答数据目录时引导去设置，不发请求", () => {
    render(<DataPage view={{ kind: "batches" }} />);
    expect(screen.getByTestId("data-not-configured")).toBeTruthy();
    expect(api.overview).not.toHaveBeenCalled();
  });

  it("批次卡片分开标注信标（实时）与快照（已导入），并标注 N", async () => {
    setRoot("F:/fixtures/terminals");
    render(<DataPage view={{ kind: "batches" }} />);
    await screen.findByTestId("data-batch-card");
    expect(screen.getByText(/信标（实时）/)).toBeTruthy();
    expect(screen.getByText(/有效计分 N=4/)).toBeTruthy();
    expect(screen.getByTestId("data-exceptions").textContent).toContain("sha256 不符");
    expect(screen.getByTestId("data-identity").textContent).toContain("卡号 C-002 对应不同姓名");
    expect(screen.getByText(/机构模拟测试原始分，非 IELTS 官方成绩。/)).toBeTruthy();
  });

  it("导入按钮调用导入命令并显示结果", async () => {
    setRoot("F:/fixtures/terminals");
    api.importSnapshots.mockResolvedValue({
      scanned: 6, ok: 2, skipped: 1, failed: 3, attemptsImported: 4, outcomes: [], finishedAt: "x", snapshotsDirFound: true
    });
    render(<DataPage view={{ kind: "batches" }} />);
    await screen.findByTestId("data-batch-card");
    fireEvent.click(screen.getByTestId("data-import"));
    await waitFor(() => expect(api.importSnapshots).toHaveBeenCalledWith("F:/fixtures/terminals"));
    expect((await screen.findByTestId("data-import-result")).textContent).toContain("新导入 2，跳过 1，失败 3");
  });
});

describe("单次考试与钻取", () => {
  it("分布、Part、题型等统计旁都标注 N，且不出现雅思 Band", async () => {
    render(<DataPage view={{ kind: "exam", batchId: "B-2026-11", paperId: "paper-A" }} />);
    await screen.findByTestId("data-exam");
    expect(screen.getByTestId("data-histogram")).toBeTruthy();
    expect(screen.getAllByText(/N=4/).length).toBeGreaterThanOrEqual(4);
    expect(screen.getByTestId("data-excluded").textContent).toContain("篇数不足");
    expect(document.body.textContent ?? "").not.toMatch(/Band/);
  });

  it("样本不足的难题只列出，不下难题结论", async () => {
    render(<DataPage view={{ kind: "exam", batchId: "B-2026-11", paperId: "paper-A" }} />);
    const hard = await screen.findByTestId("data-hard");
    expect(hard.textContent).toContain("样本不足（N=4");
    expect(hard.textContent).not.toContain("难题\n");
  });

  it("点击学生进入学生详情（写入 hash 路由）", async () => {
    render(<DataPage view={{ kind: "exam", batchId: "B-2026-11", paperId: "paper-A" }} />);
    await screen.findByTestId("data-students");
    fireEvent.click(within(screen.getByTestId("data-students")).getByRole("button", { name: "张三" }));
    expect(window.location.hash).toBe("#/data/student/A1");
  });

  it("导出按钮把结果路径告诉老师", async () => {
    api.exportXlsx.mockResolvedValue({ path: "F:/out/x.xlsx", sheets: ["成绩汇总", "学生明细"], n: 4 });
    render(<DataPage view={{ kind: "exam", batchId: "B-2026-11", paperId: "paper-A" }} />);
    await screen.findByTestId("data-exam");
    fireEvent.click(screen.getByTestId("data-export"));
    expect((await screen.findByTestId("data-export-notice")).textContent).toContain("F:/out/x.xlsx");
  });
});

describe("学生与题目详情", () => {
  it("学生页标出重叠用时，错题可进入题目页", async () => {
    render(<DataPage view={{ kind: "student", attemptId: "A1" }} />);
    await screen.findByTestId("data-student");
    expect(screen.getByText("用时可能重叠")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "看此题" }));
    expect(window.location.hash).toBe("#/data/question?batch=B-2026-11&paper=paper-A&passage=0&qid=p0q3");
  });

  it("题目页显示答案分布与 N", async () => {
    render(<DataPage view={{ kind: "question", batchId: "B-2026-11", paperId: "paper-A", passageIndex: 0, questionId: "p0q1" }} />);
    await screen.findByTestId("data-question");
    expect(screen.getAllByText("75%").length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText(/样本不足（N=4/)).toBeTruthy();
    expect(screen.getAllByText(/TRUE/).length).toBeGreaterThanOrEqual(1);
  });

  it("go 仍可用于数据导航（路由回归）", () => {
    go("/data");
    expect(window.location.hash).toBe("#/data");
  });
});
