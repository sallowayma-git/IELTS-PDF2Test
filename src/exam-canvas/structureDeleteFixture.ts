// 结构删除/添加测试共用的合成题稿：一道大题（headings）两个作答组，共三题（q14、q15 在 rg-a；q16 在 rg-b）。
// 选项库共享、拖放作答，与选项池测试的版式一致。仅供测试引用。
import type { AnswerSlotV2, IeltsAuthoringIRV2, OptionV2, TaskGroupV2 } from "../types";

function option(optionId: string, label: string, text = label): OptionV2 {
  return { optionId, label, content: [{ id: `${optionId}-text`, type: "text", text, sourceAnchors: [], provenanceStatus: "source" }], sourceAnchors: [] };
}

function slot(questionNumber: number): AnswerSlotV2 {
  return { slotId: `q${questionNumber}`, questionNumber, displayLabel: String(questionNumber), hostType: "prompt", interaction: "dragdrop", participation: "scoring", sourceAnchors: [], confidence: 1 };
}

export function structureDraft(): IeltsAuthoringIRV2 {
  const task: TaskGroupV2 = {
    taskId: "headings", taskType: "matching_features", displayRange: { kind: "range", start: 14, end: 16 }, instructions: [],
    instructionSignature: { normalizedText: "", taskType: "matching_features", expectedQuestionNumbers: [14, 15, 16], expectedSlotCount: 3, evidenceAnchors: [], confidence: 1 },
    responseGroups: [
      { responseGroupId: "rg-a", kind: "matching", slotIds: ["q14", "q15"], optionBankRef: "headings-bank", cardinality: { min: 1, max: 1, exact: 1 }, assignment: "per_slot", scoringPolicy: "per_slot_binary", duplicatePolicy: "reject_submission", allowOptionReuse: false, sourceAnchors: [] },
      { responseGroupId: "rg-b", kind: "matching", slotIds: ["q16"], optionBankRef: "headings-bank", cardinality: { min: 1, max: 1, exact: 1 }, assignment: "per_slot", scoringPolicy: "per_slot_binary", duplicatePolicy: "reject_submission", allowOptionReuse: false, sourceAnchors: [] }
    ],
    optionBank: { optionBankId: "headings-bank", scope: "task_group", options: [option("heading-i", "i"), option("heading-iv", "iv")], allowReuse: false, sourceAnchors: [] },
    sourceAnchors: [], quality: { score: 1, sourceCoverage: 1, hardFailures: [] }, reviewState: "unreviewed"
  };
  const quality = {
    schemaVersion: "QualityReportV2" as const, state: "review_required" as const,
    documentScore: 0, sourceCoverage: 0, coverageLedger: [],
    coverageStatus: { physicalShadow: "missing" as const, complete: false, significantSourceNodeCount: 0, explainedSourceNodeCount: 0, unassignedSourceNodeIds: [] },
    compilerProbes: {
      v2Runtime: { status: "passed" as const, schemaVersion: "", issueCodes: [], details: [] },
      v1Compatibility: { status: "passed" as const, schemaVersion: "", issueCodes: [], details: [] }
    },
    taskScores: {}, hardFailures: [], issues: [], metrics: {}, evaluatedAt: "", evaluatorVersion: ""
  };
  return {
    schemaVersion: "IeltsAuthoringIRV2", jobId: "job-structure", exam: { examId: "exam", title: "Structure", language: "en", tags: [], sourceFiles: [] }, modality: "reading",
    passage: { content: [], paragraphMap: {}, sourceAnchors: [] },
    taskGroups: [task],
    answerSlots: { q14: slot(14), q15: slot(15), q16: slot(16) },
    answerKey: { q14: { kind: "unresolved" }, q15: { kind: "unresolved" }, q16: { kind: "unresolved" } },
    assets: [], sourceDocumentId: "source", quality,
    audit: { revision: 1, source: "auto_extract", humanVerified: false, llmUsed: false, updatedAt: "", notes: [] }
  } as unknown as IeltsAuthoringIRV2;
}
