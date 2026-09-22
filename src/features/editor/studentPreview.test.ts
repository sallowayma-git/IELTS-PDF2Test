import { describe, expect, it } from "vitest";
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

describe("compilePreviewSource — 编译闸门", () => {
  it("没有草稿时不给预览", () => {
    expect(compilePreviewSource(undefined)).toBeUndefined();
  });

  it("空题组草稿可以编译（预览显示 0 个题组，而不是报错）", () => {
    const result = compilePreviewSource(makeDs({ taskGroups: [] }));
    expect(result?.ok).toBe(true);
    if (result?.ok) {
      expect(result.summary).toEqual({
        modality: "reading",
        taskGroups: 0,
        slots: 0,
        assets: 0,
        answeredSlots: 0,
        listeningParts: 0,
        answerKeyIssues: []
      });
    }
  });

  it("选择题没有可解析的选项来源时编译失败，并给出可定位的 responseGroupId", () => {
    const ds = makeDs({
      taskGroups: [
        task({
          taskId: "task-1",
          taskType: "single_choice",
          responseGroups: [group({ responseGroupId: "rg-1", kind: "choice", slotIds: ["q1"] })]
        })
      ],
      answerSlots: { q1: slot("q1", 1) },
      answerKey: { q1: { kind: "option", labels: ["A"], assignment: "per_slot" } }
    });
    const result = compilePreviewSource(ds);
    expect(result?.ok).toBe(false);
    if (result && !result.ok) {
      expect(result.issue.code).toBe("RUNTIME_OPTION_BANK_MISSING");
      expect(result.issue.targetId).toBe("rg-1");
    }
  });

  it("答案位没有被任何题组引用时编译失败，并定位到该答案位", () => {
    const ds = makeDs({
      taskGroups: [],
      answerSlots: { q9: slot("q9", 9) },
      answerKey: { q9: { kind: "option", labels: ["A"], assignment: "per_slot" } }
    });
    const result = compilePreviewSource(ds);
    expect(result?.ok).toBe(false);
    if (result && !result.ok) {
      expect(result.issue.code).toBe("RUNTIME_SLOT_UNASSIGNED");
      expect(result.issue.targetId).toBe("q9");
    }
  });

  it("合法草稿编译通过，并统计题组/答案位/已作答数", () => {
    const ds = makeDs({
      taskGroups: [
        task({
          taskId: "task-1",
          taskType: "single_choice",
          responseGroups: [group({ responseGroupId: "rg-1", kind: "choice", slotIds: ["q1"], optionBankRef: "bank-1" })],
          optionBank: { optionBankId: "bank-1", allowReuse: false, options: [option("o-a", "A", "alpha"), option("o-b", "B", "beta")] }
        } as Partial<TaskGroupV2> & Pick<TaskGroupV2, "taskId" | "taskType" | "responseGroups">)
      ],
      answerSlots: { q1: slot("q1", 1) },
      answerKey: { q1: { kind: "option", labels: ["A"], assignment: "per_slot" } }
    });
    const result = compilePreviewSource(ds);
    expect(result?.ok, JSON.stringify(result)).toBe(true);
    if (result?.ok) {
      expect(result.summary.taskGroups).toBe(1);
      expect(result.summary.slots).toBe(1);
      expect(result.summary.answeredSlots).toBe(1);
      expect(result.compiled.modality).toBe("reading");
      if (result.compiled.modality === "reading") expect(result.compiled.reading.answerKey.q1).toBeTruthy();
    }
  });
});

describe("compilePreviewSource — 答案键类型与槽位交互的一致性（与真实学生端同一判定）", () => {
  // 这是本轮真实夹具暴露出的问题：T/F/NG 这类选项型槽位，答案键却被写成 text。
  // 真实学生端（reading_runtime_v2.rs）会判 RUNTIME_CHOICE_SLOT_ANSWER_NOT_OPTION
  // 并让整份提交失败，发布门禁也记成 RUNTIME_COMPILER_FAILED。
  // 而题面照样渲染 —— 预览必须把「能看」和「能提交」分开说，不能显示假完成。
  it("选项型槽位配文本型答案键：预览仍可渲染，但必须报出该答案位", () => {
    const ds = makeDs({
      taskGroups: [
        task({
          taskId: "task-1",
          taskType: "true_false_not_given",
          responseGroups: [group({ responseGroupId: "rg-1", kind: "choice", slotIds: ["q1"], optionBankRef: "bank-1" })],
          optionBank: {
            optionBankId: "bank-1",
            allowReuse: false,
            options: [option("o-t", "TRUE", "TRUE"), option("o-f", "FALSE", "FALSE")]
          }
        } as Partial<TaskGroupV2> & Pick<TaskGroupV2, "taskId" | "taskType" | "responseGroups">)
      ],
      answerSlots: { q1: slot("q1", 1) },
      answerKey: { q1: { kind: "text", values: ["TRUE"], normalization: "ielts_default" } }
    });
    const result = compilePreviewSource(ds);
    // 关键：不是编译失败——题面照样能画出来。
    expect(result?.ok, JSON.stringify(result)).toBe(true);
    if (result?.ok) {
      expect(result.summary.answerKeyIssues).toHaveLength(1);
      expect(result.summary.answerKeyIssues[0].code).toBe("RUNTIME_CHOICE_SLOT_ANSWER_NOT_OPTION");
      expect(result.summary.answerKeyIssues[0].targetId).toBe("q1");
    }
  });

  it("选项型槽位配选项型答案键：不报问题", () => {
    const ds = makeDs({
      taskGroups: [
        task({
          taskId: "task-1",
          taskType: "true_false_not_given",
          responseGroups: [group({ responseGroupId: "rg-1", kind: "choice", slotIds: ["q1"], optionBankRef: "bank-1" })],
          optionBank: {
            optionBankId: "bank-1",
            allowReuse: false,
            options: [option("o-t", "TRUE", "TRUE"), option("o-f", "FALSE", "FALSE")]
          }
        } as Partial<TaskGroupV2> & Pick<TaskGroupV2, "taskId" | "taskType" | "responseGroups">)
      ],
      answerSlots: { q1: slot("q1", 1) },
      answerKey: { q1: { kind: "option", labels: ["TRUE"], assignment: "per_slot" } }
    });
    const result = compilePreviewSource(ds);
    expect(result?.ok, JSON.stringify(result)).toBe(true);
    if (result?.ok) expect(result.summary.answerKeyIssues).toEqual([]);
  });

  it("答案未填（unresolved）不算类型不匹配——那属于另一类问题", () => {
    const ds = makeDs({
      taskGroups: [
        task({
          taskId: "task-1",
          taskType: "true_false_not_given",
          responseGroups: [group({ responseGroupId: "rg-1", kind: "choice", slotIds: ["q1"], optionBankRef: "bank-1" })],
          optionBank: {
            optionBankId: "bank-1",
            allowReuse: false,
            options: [option("o-t", "TRUE", "TRUE"), option("o-f", "FALSE", "FALSE")]
          }
        } as Partial<TaskGroupV2> & Pick<TaskGroupV2, "taskId" | "taskType" | "responseGroups">)
      ],
      answerSlots: { q1: slot("q1", 1) },
      answerKey: { q1: { kind: "unresolved" } }
    });
    const result = compilePreviewSource(ds);
    expect(result?.ok, JSON.stringify(result)).toBe(true);
    if (result?.ok) expect(result.summary.answerKeyIssues).toEqual([]);
  });

  it("文本型槽位配文本型答案键：不报问题", () => {
    const textSlot: AnswerSlotV2 = { ...slot("q1", 1), interaction: "text" };
    const ds = makeDs({
      taskGroups: [
        task({
          taskId: "task-1",
          taskType: "table_completion",
          responseGroups: [group({ responseGroupId: "rg-1", kind: "text_entry", slotIds: ["q1"] })]
        })
      ],
      answerSlots: { q1: textSlot },
      answerKey: { q1: { kind: "text", values: ["maps"], normalization: "ielts_default" } }
    });
    const result = compilePreviewSource(ds);
    expect(result?.ok, JSON.stringify(result)).toBe(true);
    if (result?.ok) expect(result.summary.answerKeyIssues).toEqual([]);
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

describe("compilePreviewSource — 听力稿按听力契约编译", () => {
  it("四个 Section 各带音频的听力稿可以预览，并如实汇报 Section 数与音频数", () => {
    const result = compilePreviewSource(listeningDs([1, 2, 3, 4]));
    expect(result?.ok, JSON.stringify(result)).toBe(true);
    if (result?.ok) {
      expect(result.summary.modality).toBe("listening");
      expect(result.summary.listeningParts).toBe(4);
      expect(result.summary.assets).toBe(4);
    }
  });

  it("某个 Section 没有音频时预览编译失败，并定位到那个 Section", () => {
    const ds = listeningDs([1, 2, 3, 4]);
    ds.listening!.parts[2].media = undefined;
    const result = compilePreviewSource(ds);
    expect(result?.ok).toBe(false);
    if (result && !result.ok) {
      expect(result.issue.code).toBe("LISTENING_MEDIA_MISSING");
      expect(result.issue.targetId).toBe("part-3");
    }
  });

  it("Section 指向的音频不在资源清单里时预览编译失败，并定位到那个音频", () => {
    const ds = listeningDs([1, 2]);
    ds.assets = ds.assets.slice(0, 1);
    const result = compilePreviewSource(ds);
    expect(result?.ok).toBe(false);
    if (result && !result.ok) {
      expect(result.issue.code).toBe("ASSET_REFERENCE_MISSING");
      expect(result.issue.targetId).toBe(audioAsset(2).assetId);
    }
  });

  it("答案形式与题目不匹配的听力稿也如实报出来（和学生端提交阶段一致）", () => {
    const ds = listeningDs([1]);
    ds.answerKey.q1 = { kind: "option", labels: ["A"], assignment: "per_slot" };
    const result = compilePreviewSource(ds);
    expect(result?.ok, JSON.stringify(result)).toBe(true);
    if (result?.ok) {
      expect(result.summary.answerKeyIssues.map((item) => item.code)).toEqual([
        "RUNTIME_TEXT_SLOT_ANSWER_NOT_TEXT"
      ]);
      expect(result.summary.answerKeyIssues[0].targetId).toBe("q1");
    }
  });
});
