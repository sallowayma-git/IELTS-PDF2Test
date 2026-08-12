import type {
  AnswerSlotV2,
  AnswerValueV2,
  AuthoringAuditV2,
  IeltsAuthoringIRV2,
  OptionBankV2,
  OptionV2,
  ResponseGroupV2,
  SourceAnchorV2,
  TaskGroupV2
} from "../types";

const SOURCE_HASH = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SOURCE_FILE_ID = "phase5-editor-pdf";

function anchor(nodeIds: string[], pageIndex = 1): SourceAnchorV2 {
  return {
    sourceFileId: SOURCE_FILE_ID,
    pageIndex,
    nodeIds,
    extractionMode: "pdf_native",
    sourceHash: SOURCE_HASH
  };
}

function textNode(id: string, text: string, pageIndex = 1) {
  return {
    type: "text" as const,
    id,
    sourceAnchors: [anchor([id], pageIndex)],
    provenanceStatus: "source" as const,
    text
  };
}

function paragraph(id: string, text: string, pageIndex = 1) {
  return {
    type: "paragraph" as const,
    id,
    sourceAnchors: [anchor([id], pageIndex)],
    provenanceStatus: "source" as const,
    children: [textNode(`${id}-text`, text, pageIndex)]
  };
}

function option(optionId: string, label: string, value: string): OptionV2 {
  return {
    optionId,
    label,
    content: [textNode(`${optionId}-text`, value)],
    sourceAnchors: [anchor([`${optionId}-line`])],
    provenanceStatus: "source"
  };
}

function answerSlot(slotId: string, questionNumber: number, promptId: string): AnswerSlotV2 {
  return {
    slotId,
    questionNumber,
    displayLabel: String(questionNumber),
    hostNodeId: promptId,
    hostType: "prompt",
    interaction: "checkbox",
    participation: "scoring",
    constraints: { acceptedOptionLabels: ["A", "B", "C", "D", "E"] },
    sourceAnchors: [anchor([`slot-${slotId}`, promptId])],
    provenanceStatus: "derived",
    confidence: 1
  };
}

function audit(): AuthoringAuditV2 {
  return {
    revision: 0,
    source: "auto_extract",
    humanVerified: false,
    llmUsed: false,
    updatedAt: "2026-08-12T00:00:00.000Z",
    notes: ["Phase 5 structured editor fixture; source remains authoritative."]
  };
}

export function createPhase5Fixture(jobId = "phase5-editor-fixture"): IeltsAuthoringIRV2 {
  const promptId = "phase5-shared-prompt";
  const options: OptionBankV2 = {
    optionBankId: "phase5-options",
    scope: "task_group",
    title: [paragraph("phase5-option-title", "List of factors")],
    options: [
      option("phase5-option-a", "A", "economic pressure"),
      option("phase5-option-b", "B", "new technology"),
      option("phase5-option-c", "C", "social change"),
      option("phase5-option-d", "D", "government policy"),
      option("phase5-option-e", "E", "population growth")
    ],
    allowReuse: false,
    sourceAnchors: [anchor(["phase5-option-a", "phase5-option-b", "phase5-option-c", "phase5-option-d", "phase5-option-e"])]
  };
  const responseGroup: ResponseGroupV2 = {
    responseGroupId: "phase5-shared-response",
    kind: "choice",
    prompt: [paragraph(promptId, "Which TWO factors influenced early organisational design?")],
    slotIds: ["q14", "q15"],
    optionBankRef: options.optionBankId,
    cardinality: { min: 2, max: 2, exact: 2 },
    assignment: "unordered_set",
    scoringPolicy: "per_slot_ielts_normalized",
    duplicatePolicy: "reject_submission",
    allowOptionReuse: false,
    sourceAnchors: [anchor(["phase5-shared-instruction", promptId, "slot-q14", "slot-q15"])]
  };
  const task: TaskGroupV2 = {
    taskId: "phase5-q14-15",
    displayRange: { kind: "set", values: [14, 15] },
    taskType: "multiple_choice",
    instructions: [paragraph("phase5-instruction", "Choose TWO letters, A-E.")],
    instructionSignature: {
      normalizedText: "Choose TWO letters, A-E.",
      taskType: "multiple_choice",
      expectedQuestionNumbers: [14, 15],
      expectedSlotCount: 2,
      optionAlphabet: "A-E",
      selectionCardinality: { min: 2, max: 2, exact: 2 },
      answerAssignment: "unordered_set",
      allowOptionReuse: false,
      evidenceAnchors: [anchor(["phase5-instruction"])],
      confidence: 1
    },
    stimulus: [paragraph("phase5-stimulus", "Several factors shaped early organisational design.", 0)],
    optionBank: options,
    responseGroups: [responseGroup],
    sourceAnchors: [anchor(["phase5-instruction", promptId, "phase5-option-a", "phase5-option-e"])],
    quality: { score: 0.92, sourceCoverage: 1, hardFailures: [] },
    reviewState: "unreviewed"
  };
  const answerKey: Record<string, AnswerValueV2> = {
    q14: { kind: "option", labels: ["B"], assignment: "unordered_set" },
    q15: { kind: "option", labels: ["D"], assignment: "unordered_set" }
  };
  const slotMap: Record<string, AnswerSlotV2> = {
    q14: answerSlot("q14", 14, promptId),
    q15: answerSlot("q15", 15, promptId)
  };
  return {
    schemaVersion: "IeltsAuthoringIRV2",
    jobId,
    exam: {
      examId: "phase5-editor",
      title: "Early Approaches to Organisational Design",
      category: "P3",
      frequency: "medium",
      language: "en",
      tags: ["phase5", "structured-editor", "shared-slots"],
      sourceFiles: [{ sourceFileId: SOURCE_FILE_ID, role: "question_paper" }]
    },
    modality: "reading",
    passage: {
      title: "Early Approaches to Organisational Design",
      content: [paragraph("phase5-passage", "Several factors shaped early organisational design.", 0)],
      paragraphMap: { A: "phase5-passage" },
      sourceAnchors: [anchor(["phase5-passage"], 0)]
    },
    taskGroups: [task],
    answerSlots: slotMap,
    answerKey,
    assets: [],
    sourceDocumentId: "phase5-editor-document",
    quality: {
      schemaVersion: "QualityReportV2",
      state: "review_required",
      documentScore: 0.92,
      sourceCoverage: 1,
      coverageLedger: [
        { sourceNodeId: "phase5-shared-prompt", significant: true, disposition: "assigned", targetIds: ["phase5-shared-response"] }
      ],
      coverageStatus: {
        physicalShadow: "available",
        complete: true,
        significantSourceNodeCount: 1,
        explainedSourceNodeCount: 1,
        unassignedSourceNodeIds: []
      },
      compilerProbes: {
        v2Runtime: { status: "passed", schemaVersion: "ReadingExamSourceV2", issueCodes: [], details: ["Phase 5 editor fixture"] },
        v1Compatibility: { status: "passed", schemaVersion: "ReadingExamSourceV1", issueCodes: [], details: ["V1 artifact remains readable"] }
      },
      taskScores: { "phase5-q14-15": 0.92 },
      hardFailures: [],
      issues: [
        {
          issueId: "phase5-prompt-review",
          code: "PROMPT_BOUNDARY_AMBIGUOUS",
          severity: "warning",
          message: "共享题干已识别，但仍建议核对源页面中的题干边界。",
          targetType: "response_group",
          targetId: "phase5-shared-response",
          sourceAnchors: [anchor([promptId])],
          suggestedActions: ["edit_text", "split_prompt"]
        }
      ],
      metrics: { taskCount: 1, slotCount: 2, optionBankCount: 1 },
      evaluatedAt: "2026-08-12T00:00:00.000Z",
      evaluatorVersion: "phase5-editor-fixture"
    },
    audit: audit()
  };
}
