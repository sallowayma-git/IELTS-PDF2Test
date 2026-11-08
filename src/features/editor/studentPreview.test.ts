import { describe, expect, it, vi } from "vitest";
import type {
  AnswerSlotV2,
  AnswerValueV2,
  AssetDescriptorV2,
  IeltsAuthoringIRV2,
  ListeningPartV2,
  ListeningStructureV2,
  OptionV2,
  ResponseGroupV2,
  TaskGroupV2
} from "../../types";
import type { TextNodeV2 } from "../../types/content-doc-v2";
import { compilePreviewSource, describePreviewPublishLimitation } from "./studentPreview";
import { buildReadingInteractionModelV2, buildRuntimeViewModelV2 } from "../../services/runtimeViewModelV2";

// 证据层级：pure unit。断言学生预览的编译闸门——它决定「能不能显示预览」以及
// 「编译失败时用户能不能定位到具体题目」，而不是题面排版。

function textNode(text: string): TextNodeV2 {
  return { id: `n-${text}`, type: "text", text, sourceAnchors: [], provenanceStatus: "source" };
}

function option(optionId: string, label: string, text: string): OptionV2 {
  return { optionId, label, content: text ? [textNode(text)] : [], sourceAnchors: [] };
}

function slot(slotId: string, questionNumber: number): AnswerSlotV2 {
  return {
    slotId,
    questionNumber,
    displayLabel: String(questionNumber),
    hostType: "prompt",
    interaction: "radio",
    participation: "scoring",
    sourceAnchors: [],
    confidence: 1
  };
}

function group(
  partial: Partial<ResponseGroupV2> & Pick<ResponseGroupV2, "responseGroupId" | "kind" | "slotIds">
): ResponseGroupV2 {
  return {
    cardinality: { min: 1, max: 1 },
    assignment: "per_slot",
    scoringPolicy: "per_slot_binary",
    duplicatePolicy: "reject_submission",
    allowOptionReuse: false,
    sourceAnchors: [],
    ...partial
  };
}

function task(
  partial: Partial<TaskGroupV2> & Pick<TaskGroupV2, "taskId" | "taskType" | "responseGroups">
): TaskGroupV2 {
  return {
    displayRange: { kind: "range", start: 1, end: 1 },
    instructions: [],
    instructionSignature: {
      normalizedText: "",
      taskType: partial.taskType,
      expectedQuestionNumbers: [],
      expectedSlotCount: 0,
      evidenceAnchors: [],
      confidence: 1
    },
    sourceAnchors: [],
    quality: { score: 1, sourceCoverage: 1, hardFailures: [] },
    reviewState: "unreviewed",
    ...partial
  };
}

function makeDs(input: {
  taskGroups: TaskGroupV2[];
  answerSlots?: Record<string, AnswerSlotV2>;
  answerKey?: Record<string, AnswerValueV2>;
  modality?: "reading" | "listening";
  listening?: ListeningStructureV2;
  assets?: AssetDescriptorV2[];
}): IeltsAuthoringIRV2 {
  return {
    schemaVersion: "IeltsAuthoringIRV2",
    jobId: "job-1",
    exam: { examId: "exam-1", title: "T", language: "en", tags: [], sourceFiles: [] },
    modality: input.modality ?? "reading",
    listening: input.listening,
    taskGroups: input.taskGroups,
    answerSlots: input.answerSlots ?? {},
    answerKey: input.answerKey ?? {},
    assets: input.assets ?? [],
    sourceDocumentId: "doc-1",
    quality: {
      schemaVersion: "QualityReportV2",
      state: "review_required",
      documentScore: 0,
      sourceCoverage: 0,
      coverageLedger: [],
      coverageStatus: {
        physicalShadow: "missing",
        complete: false,
        significantSourceNodeCount: 0,
        explainedSourceNodeCount: 0,
        unassignedSourceNodeIds: []
      },
      compilerProbes: {
        v2Runtime: { status: "passed", schemaVersion: "", issueCodes: [], details: [] },
        v1Compatibility: { status: "passed", schemaVersion: "", issueCodes: [], details: [] }
      },
      taskScores: {},
      hardFailures: [],
      issues: [],
      metrics: {},
      evaluatedAt: "",
      evaluatorVersion: ""
    },
    audit: { revision: 1, source: "auto_extract", humanVerified: false, llmUsed: false, updatedAt: "", notes: [] }
  };
}

// 证据层级：pure unit + IPC 委托。P2 起预览不再有 TS 民办编译器：
// `compilePreviewSource` 只负责把当前草稿交给 Rust `compile_authoring_preview`
// 并翻译其结果——「能不能预览」的判定完全来自产品发布时用的那个编译器。
vi.mock("../../api/tauriCommands", () => ({ command: vi.fn() }));

import { command } from "../../api/tauriCommands";

const commandMock = vi.mocked(command);

function rustOk(summary: Record<string, unknown> = {}) {
  return {
    schemaVersion: "PreviewCompileV1",
    ok: true,
    issues: [],
    summary: {
      modality: "reading",
      taskGroups: 1,
      slots: 1,
      assets: 0,
      answeredSlots: 1,
      listeningParts: 0,
      answerKeyIssues: [],
      ...summary
    }
  };
}

describe("compilePreviewSource — Rust 编译闸门（P2 单一编译器）", () => {
  it("没有草稿时不调用编译、不给预览", async () => {
    commandMock.mockReset();
    await expect(compilePreviewSource(undefined)).resolves.toBeUndefined();
    expect(commandMock).not.toHaveBeenCalled();
  });

  it("把当前草稿原样交给 Rust 编译命令", async () => {
    commandMock.mockReset();
    commandMock.mockResolvedValue(rustOk());
    const ds = makeDs({ taskGroups: [] });
    const result = await compilePreviewSource(ds);
    expect(commandMock).toHaveBeenCalledWith("compile_authoring_preview", { authoring: ds });
    expect(result?.ok).toBe(true);
  });

  it("编译通过：概要来自 Rust（题组/答案位/资源/已答）", async () => {
    commandMock.mockReset();
    commandMock.mockResolvedValue(rustOk({ taskGroups: 3, slots: 13, assets: 2, answeredSlots: 11 }));
    const result = await compilePreviewSource(makeDs({ taskGroups: [] }));
    expect(result?.ok).toBe(true);
    if (!result?.ok) throw new Error("expected ok");
    expect(result.summary).toMatchObject({
      modality: "reading",
      taskGroups: 3,
      slots: 13,
      assets: 2,
      answeredSlots: 11,
      listeningParts: 0
    });
    expect(result.summary.answerKeyIssues).toEqual([]);
  });

  it("编译失败：不渲染预览，给出第一条可定位问题", async () => {
    commandMock.mockReset();
    commandMock.mockResolvedValue({
      schemaVersion: "PreviewCompileV1",
      ok: false,
      issues: [
        { code: "RUNTIME_PASSAGE_MISSING", targetId: "exam", message: "passage missing" },
        { code: "RUNTIME_OPTION_BANK_MISSING", targetId: "group-2", message: "bank missing" }
      ],
      summary: null
    });
    const result = await compilePreviewSource(makeDs({ taskGroups: [] }));
    expect(result?.ok).toBe(false);
    if (result?.ok !== false) throw new Error("expected failure");
    expect(result.issue.code).toBe("RUNTIME_PASSAGE_MISSING");
    expect(result.issue.targetId).toBe("exam");
  });

  it("命令抛错（非编译问题）也收敛成可定位失败，不把用户挂在等待里", async () => {
    commandMock.mockReset();
    commandMock.mockRejectedValue(new Error("ipc down"));
    const result = await compilePreviewSource(makeDs({ taskGroups: [] }));
    expect(result?.ok).toBe(false);
    if (result?.ok !== false) throw new Error("expected failure");
    expect(result.issue.code).toBe("PREVIEW_COMPILE_FAILED");
    expect(result.issue.message).toContain("ipc down");
  });

  it("听力稿按稿件自身 modality 编译（由 Rust 分派），Section 数进概要", async () => {
    commandMock.mockReset();
    commandMock.mockResolvedValue(rustOk({ modality: "listening", listeningParts: 4, assets: 4 }));
    const result = await compilePreviewSource(makeDs({ taskGroups: [], modality: "listening" }));
    expect(result?.ok).toBe(true);
    if (!result?.ok) throw new Error("expected ok");
    expect(result.summary.modality).toBe("listening");
    expect(result.summary.listeningParts).toBe(4);
  });
});


describe("describePreviewPublishLimitation — 预览与发布的差距", () => {
  it("有未保存修改时是 warning，并点明发布用的是已保存内容（**不带版本号**）", () => {
    const note = describePreviewPublishLimitation({ pendingCount: 3, blockerCount: 0 });
    expect(note.level).toBe("warning");
    expect(note.message).toContain("3");
    // 本轮任务书第一节：`v1/v2/v3` 这类内部版本计数不得进入普通界面。
    expect(note.message).not.toMatch(/\bv\d+\b/);
    expect(note.message).toContain("已保存");
  });

  it("不再说「发不出去」：预览里没有门槛话术（产品决定 2）", () => {
    const note = describePreviewPublishLimitation({ pendingCount: 0, blockerCount: 4 });
    expect(note.message).not.toMatch(/发不出去|不能导出|阻断/);
  });

  it("答案键类型不匹配时是 warning，并说明学生提交会被判无效", () => {
    const note = describePreviewPublishLimitation({ pendingCount: 0, blockerCount: 0, runtimeIssueCount: 3 });
    expect(note.level).toBe("warning");
    expect(note.message).toContain("3");
    expect(note.message).toContain("提交");
    expect(note.message).toContain("待补充");
  });

  it("干净草稿是 info，并说明预览与已保存内容一致（**不带版本号**）", () => {
    const note = describePreviewPublishLimitation({ pendingCount: 0, blockerCount: 0 });
    expect(note.level).toBe("info");
    expect(note.message).toContain("一致");
    expect(note.message).not.toMatch(/\bv\d+\b/);
  });
});

// ——— 听力稿的学生预览 ———
//
// 听力卷和阅读卷的学生端契约是两份不同的东西（`ListeningExamSourceV1` vs
// `ReadingExamSourceV2`），题面结构也不同（Section 各自一段音频 vs 一篇文章）。
// 预览如果只会按阅读编译，听力稿要么编译失败、要么被编译成一份没有文章的「阅读稿」
// 而看起来通过了——两者都是只在发布后才暴露的假象。

function audioAsset(ordinal: number): AssetDescriptorV2 {
  const sha = String(ordinal).repeat(64).slice(0, 64);
  return {
    assetId: `audio-${sha}`,
    kind: "audio",
    mime: "audio/wav",
    relativePath: `audio/${sha}.wav`,
    sha256: sha,
    byteLength: 1024,
    durationMs: 1000,
    extractionMode: "user_upload"
  };
}

function audioPart(ordinal: number, questionNumber: number): ListeningPartV2 {
  const asset = audioAsset(ordinal);
  return {
    partId: `part-${ordinal}`,
    displayLabel: `SECTION ${ordinal}`,
    expectedQuestionNumbers: [questionNumber],
    taskIds: [`task-${ordinal}`],
    sourceAnchors: [],
    media: {
      assetId: asset.assetId,
      mime: "audio/wav",
      durationMs: 1000,
      channels: 1,
      sampleRateHz: 16000,
      sha256: asset.sha256,
      probe: { status: "passed", provider: "symphonia", providerVersion: "0.6.0", probedAt: "", issueCodes: [] }
    }
  };
}

/** `partial_practice`：一份只有几个 Section 的练习卷，不受「四部分四十题」约束。 */
function listeningDs(ordinals: number[]): IeltsAuthoringIRV2 {
  const assets: AssetDescriptorV2[] = [];
  const parts: ListeningPartV2[] = [];
  const taskGroups: TaskGroupV2[] = [];
  const answerSlots: Record<string, AnswerSlotV2> = {};
  const answerKey: Record<string, AnswerValueV2> = {};
  ordinals.forEach((ordinal, index) => {
    const number = index + 1;
    const slotId = `q${number}`;
    assets.push(audioAsset(ordinal));
    parts.push(audioPart(ordinal, number));
    answerSlots[slotId] = { ...slot(slotId, number), hostType: "paragraph", interaction: "text" };
    answerKey[slotId] = { kind: "text", values: [`answer ${number}`], normalization: "ielts_default" };
    taskGroups.push(
      task({
        taskId: `task-${ordinal}`,
        taskType: "note_completion",
        displayRange: { kind: "range", start: number, end: number },
        instructionSignature: {
          normalizedText: "Write ONE WORD ONLY.",
          taskType: "note_completion",
          expectedQuestionNumbers: [number],
          expectedSlotCount: 1,
          evidenceAnchors: [],
          confidence: 1
        },
        responseGroups: [group({ responseGroupId: `rg-${ordinal}`, kind: "text_entry", slotIds: [slotId] })]
      })
    );
  });
  const listening: ListeningStructureV2 = {
    scope: "partial_practice",
    parts,
    playbackPolicy: {
      mode: "practice",
      allowPause: true,
      allowSeek: true,
      allowReplay: true,
      maxPlays: 2,
      refreshBehavior: "resume_from_snapshot",
      crashRecoveryBehavior: "resume_from_snapshot",
      showCurrentTime: true,
      showDuration: true
    }
  };
  return makeDs({ taskGroups, answerSlots, answerKey, modality: "listening", listening, assets });
}
