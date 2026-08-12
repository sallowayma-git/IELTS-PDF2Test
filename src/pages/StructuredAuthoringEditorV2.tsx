import { useCallback, useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { applyAuthoringV2Patches, exportAuthoringV2, exportNasPackageV2, getAuthoringV2 } from "../api/tauriCommands";
import { go } from "../app/router";
import { chooseExportDirectory } from "../api/desktopDialogs";
import { answerValueForSelection, applyAuthoringV2Patches as applyLocalPatches, inverseAuthoringPatch, locateContentNode, taskTypeLabel } from "../services/authoringV2Patches";
import { AuthoringTiptapEditor } from "../editor/authoringTiptap";
import { buildReadingInteractionModelV2, buildRuntimeViewModelV2 } from "../services/runtimeViewModelV2";
import type {
  AnswerSlotV2,
  AnswerValueV2,
  AuthoringContentTargetV2,
  AuthoringEditorRecoveryV2,
  AuthoringEditorSessionV2,
  AuthoringPatchV2,
  ContentNodeV2,
  IeltsAuthoringIRV2,
  OptionV2,
  QuestionNumberExpressionV2,
  ResponseGroupV2,
  SourceAnchorV2,
  TaskGroupV2,
  TaskTypeV2
} from "../types";

const RECOVERY_KEY_PREFIX = "ielts-author-studio.phase5-recovery.";
const NAS_PACKAGE_V2_ENABLED = import.meta.env.VITE_IELTS_NAS_PACKAGE_V2 === "1";
const TASK_TYPES: TaskTypeV2[] = [
  "multiple_choice",
  "single_choice",
  "true_false_not_given",
  "yes_no_not_given",
  "matching_headings",
  "matching_information",
  "matching_features",
  "matching_sentence_endings",
  "classification",
  "sentence_completion",
  "summary_completion",
  "note_completion",
  "table_completion",
  "form_completion",
  "flowchart_completion",
  "diagram_label_completion",
  "plan_map_label_completion",
  "short_answer"
];

function recoveryKey(jobId: string): string {
  return RECOVERY_KEY_PREFIX + jobId;
}

function nodeText(nodes: ContentNodeV2[] | undefined): string {
  if (!nodes) return "";
  return nodes.map((node) => {
    if (node.type === "text") return node.text;
    if ("children" in node) return nodeText(node.children);
    if (node.type === "answer_slot") return "[" + node.displayLabel + "]";
    return "";
  }).join("");
}

function findEntity(value: unknown, id: string): Record<string, unknown> | undefined {
  if (Array.isArray(value)) {
    for (const item of value) {
      const found = findEntity(item, id);
      if (found) return found;
    }
    return undefined;
  }
  if (!value || typeof value !== "object") return undefined;
  const object = value as Record<string, unknown>;
  if (object.id === id || object.taskId === id || object.responseGroupId === id || object.slotId === id || object.optionId === id) return object;
  for (const child of Object.values(object)) {
    const found = findEntity(child, id);
    if (found) return found;
  }
  return undefined;
}

function sourceAnchorsFor(authoring: IeltsAuthoringIRV2, id: string): SourceAnchorV2[] {
  const entity = findEntity(authoring, id);
  return Array.isArray(entity?.sourceAnchors) ? entity.sourceAnchors as SourceAnchorV2[] : [];
}

function sourceSummary(anchors: SourceAnchorV2[]): string {
  if (!anchors.length) return "暂无来源锚点";
  const pages = Array.from(new Set(anchors.map((anchor) => anchor.pageIndex + 1))).sort((a, b) => a - b);
  return "源文件页码：" + pages.join("、") + " · " + anchors.reduce((count, anchor) => count + anchor.nodeIds.length, 0) + " 个源节点";
}

function sourceAnchorStyle(anchor: SourceAnchorV2): CSSProperties | undefined {
  const rect = anchor.displayBBox ?? anchor.bbox ?? anchor.nativeBBox;
  if (!rect?.normalized) return undefined;
  const [x, y, width, height] = rect.normalized;
  return {
    left: `${x * 100}%`,
    top: `${y * 100}%`,
    width: `${width * 100}%`,
    height: `${height * 100}%`
  };
}

function sourceAnchorGeometry(anchor: SourceAnchorV2): string {
  const rect = anchor.displayBBox ?? anchor.bbox ?? anchor.nativeBBox;
  if (!rect) return "坐标未提供";
  const values = [rect.x, rect.y, rect.width, rect.height].map((value) => Number(value.toFixed(2))).join(", ");
  const normalized = rect.normalized ? ` · normalized ${rect.normalized.map((value) => Number(value.toFixed(3))).join(", ")}` : "";
  return `${rect.unit}/${rect.origin} [${values}]${normalized}`;
}

function readRecovery(jobId: string): AuthoringEditorRecoveryV2 | undefined {
  try {
    const raw = window.localStorage.getItem(recoveryKey(jobId));
    if (!raw) return undefined;
    const value = JSON.parse(raw) as AuthoringEditorRecoveryV2;
    if (value.schemaVersion !== "AuthoringEditorRecoveryV1" || value.jobId !== jobId || !Array.isArray(value.patches)) return undefined;
    return value;
  } catch {
    return undefined;
  }
}

function selectedLabels(value: AnswerValueV2 | undefined): string[] {
  return value?.kind === "option" ? value.labels : [];
}

function answerText(value: AnswerValueV2 | undefined): string {
  return value?.kind === "text" ? value.values.join(", ") : "";
}

function contentNodeEditor(
  nodes: ContentNodeV2[] | undefined,
  selectedId: string | undefined,
  onSelect: (id: string) => void,
  onTextChange: (node: Extract<ContentNodeV2, { type: "text" }>) => void,
  readOnly = false
): ReactNode {
  if (!nodes?.length) return <span className="structured-empty-inline">暂无内容</span>;
  return nodes.map((node) => {
    const selected = node.id === selectedId;
    const className = ["structured-node", "structured-node-" + node.type, selected ? "selected" : ""].join(" ");
    switch (node.type) {
      case "text":
        return readOnly
          ? <span key={node.id} data-editor-id={node.id} className={className} onClick={() => onSelect(node.id)}>{node.text}</span>
          : <textarea key={node.id} data-editor-id={node.id} className={className} aria-label={"编辑文本 " + node.id} value={node.text} rows={Math.max(1, Math.min(4, Math.ceil(node.text.length / 70)))} onClick={() => onSelect(node.id)} onChange={(event) => onTextChange({ ...node, text: event.target.value })} />;
      case "paragraph":
        return <p key={node.id} data-editor-id={node.id} className={className} onClick={() => onSelect(node.id)}>{contentNodeEditor(node.children, selectedId, onSelect, onTextChange, readOnly)}</p>;
      case "heading":
        return <div key={node.id} data-editor-id={node.id} className={className} onClick={() => onSelect(node.id)}><strong>{contentNodeEditor(node.children, selectedId, onSelect, onTextChange, readOnly)}</strong></div>;
      case "hard_break":
        return <br key={node.id} />;
      case "answer_slot":
        return <span key={node.id} data-editor-id={node.id} className={className + " structured-slot-chip"} onClick={() => onSelect(node.id)}>{node.displayLabel}</span>;
      case "bullet_list":
      case "ordered_list":
        return <ul key={node.id} data-editor-id={node.id} className={className} onClick={() => onSelect(node.id)}>{node.items.map((item) => <li key={item.id}>{contentNodeEditor(item.children, selectedId, onSelect, onTextChange, readOnly)}</li>)}</ul>;
      case "table":
        return <table key={node.id} data-editor-id={node.id} className={className} onClick={() => onSelect(node.id)}><tbody>{node.rows.map((row) => <tr key={row.id}>{row.cells.map((cell) => { const Cell = cell.headerScope && cell.headerScope !== "none" ? "th" : "td"; return <Cell key={cell.id} scope={Cell === "th" ? cell.headerScope : undefined} colSpan={cell.colSpan} rowSpan={cell.rowSpan}>{contentNodeEditor(cell.children, selectedId, onSelect, onTextChange, readOnly)}</Cell>; })}</tr>)}</tbody></table>;
      case "figure":
      case "image":
      case "diagram":
        return <div key={node.id} data-editor-id={node.id} className={className + " structured-asset-placeholder"} onClick={() => onSelect(node.id)}><span>视觉资源</span><small>{node.assetId}</small></div>;
      case "flowchart":
        return <div key={node.id} data-editor-id={node.id} className={className + " structured-flowchart"} onClick={() => onSelect(node.id)}>{node.steps.map((step) => <div key={step.id} className="structured-flow-step">{step.label ? <strong>{step.label}</strong> : null}{contentNodeEditor(step.children, selectedId, onSelect, onTextChange, readOnly)}</div>)}</div>;
      case "flow_step":
      case "list_item":
      case "figcaption":
      case "doc":
        return <div key={node.id} data-editor-id={node.id} className={className} onClick={() => onSelect(node.id)}>{"children" in node ? contentNodeEditor(node.children, selectedId, onSelect, onTextChange, readOnly) : null}</div>;
      case "option_bank":
      case "horizontal_rule":
        return <div key={node.id} data-editor-id={node.id} className={className} onClick={() => onSelect(node.id)} />;
    }
  });
}

function optionText(option: OptionV2): string {
  return nodeText(option.content);
}

function RuntimeAnswerControl({ interaction, name, options }: { interaction: AnswerSlotV2["interaction"]; name: string; options: string[] }) {
  if (interaction === "text" || interaction === "select") return <input className="student-control" type="text" name={name} placeholder="填写答案" />;
  const inputType = interaction === "checkbox" || interaction === "dragdrop" ? "checkbox" : "radio";
  return <span className="student-control-group">{options.map((option) => <label key={option}><input type={inputType} name={name} value={option} />{option}</label>)}</span>;
}

function StudentPreview({ authoring }: { authoring: IeltsAuthoringIRV2 }) {
  const runtime = buildRuntimeViewModelV2(authoring);
  const interactionModel = buildReadingInteractionModelV2(runtime);
  return <div className="student-parity-grid">
    <article className="student-sheet"><p className="student-sheet-label">Passage · RuntimeViewModelV2</p><h3>{runtime.title}</h3>{contentNodeEditor(runtime.passage, undefined, () => undefined, () => undefined, true)}</article>
    <article className="student-sheet"><p className="student-sheet-label">Questions · ReadingInteractionModelV2</p>{runtime.taskGroups.map((task) => <section key={task.taskId} className="student-task">{contentNodeEditor(task.instructions, undefined, () => undefined, () => undefined, true)}{task.stimulus?.length ? <div className="student-stimulus">{contentNodeEditor(task.stimulus, undefined, () => undefined, () => undefined, true)}</div> : null}{task.responseGroups.map((group) => { const interaction = interactionModel.responseGroups[group.responseGroupId]; if (!interaction) return null; const options = interaction.options.map((option) => option.label); return <div key={group.responseGroupId} className="student-response">{contentNodeEditor(group.prompt, undefined, () => undefined, () => undefined, true)}{group.slotIds.map((slotId) => { const slot = interactionModel.slots[slotId]; if (!slot) return null; return <div key={slotId} className="student-answer-row"><strong>{runtime.questionDisplayMap[slotId]}</strong><RuntimeAnswerControl interaction={slot.slot.interaction} name={slotId} options={options.length ? options : slot.slot.constraints?.acceptedOptionLabels ?? []} /></div>; })}</div>; })}</section>)}</article>
  </div>;
}

function SourceOverlay({ authoring, selectedId, anchorsOverride }: { authoring: IeltsAuthoringIRV2; selectedId?: string; anchorsOverride?: SourceAnchorV2[] }) {
  const anchors = anchorsOverride ?? (selectedId ? sourceAnchorsFor(authoring, selectedId) : []);
  const pages = Array.from(new Set(anchors.map((anchor) => anchor.pageIndex))).sort((a, b) => a - b);
  return <section className="source-overlay-panel">
    <div className="inspector-section-heading"><span>源定位</span><small>{selectedId ?? "未选择节点"}</small></div>
    <p className="source-summary">{sourceSummary(anchors)}</p>
    {pages.length ? pages.map((page) => <div className="source-page-mini" key={page}><span>源页面 {page + 1}</span><div className="source-page-grid source-coordinate-overlay">{anchors.filter((anchor) => anchor.pageIndex === page).map((anchor, index) => { const style = sourceAnchorStyle(anchor); return <div className={"source-anchor-card" + (style ? " has-geometry" : "")} key={`${anchor.sourceFileId}-${index}`} style={style} title={anchor.nodeIds.join("、")}><span>{anchor.nodeIds.join("、")}</span><small>{sourceAnchorGeometry(anchor)}</small></div>; })}</div></div>) : <div className="source-page-mini source-page-empty">点击题干、选项或问题即可查看源锚点</div>}
  </section>;
}

export function StructuredAuthoringEditorV2({ jobId, refresh }: { jobId: string; refresh?: () => void }) {
  const [session, setSession] = useState<AuthoringEditorSessionV2>();
  const [draft, setDraft] = useState<IeltsAuthoringIRV2>();
  const draftRef = useRef<IeltsAuthoringIRV2 | undefined>(undefined);
  const [pendingPatches, setPendingPatches] = useState<AuthoringPatchV2[]>([]);
  const pendingRef = useRef<AuthoringPatchV2[]>([]);
  const [selectedId, setSelectedId] = useState<string>();
  const [viewMode, setViewMode] = useState<"edit" | "preview">("edit");
  const [status, setStatus] = useState<"loading" | "clean" | "dirty" | "saving" | "saved" | "conflict" | "error">("loading");
  const [error, setError] = useState<string>();
  const [recoveryCandidate, setRecoveryCandidate] = useState<AuthoringEditorRecoveryV2>();
  const [saving, setSaving] = useState(false);
  const [undoDepth, setUndoDepth] = useState(0);
  const [redoDepth, setRedoDepth] = useState(0);
  const [exporting, setExporting] = useState(false);
  const [exportResult, setExportResult] = useState<{
    outputDir: string;
    revision: number;
    package?: { manifestPath: string; reportPath: string; assetCount: number; probePassed: boolean };
  }>();
  const undoRef = useRef<Array<{ patch: AuthoringPatchV2; inverse: AuthoringPatchV2 }>>([]);
  const redoRef = useRef<Array<{ patch: AuthoringPatchV2; inverse: AuthoringPatchV2 }>>([]);

  const load = useCallback(async () => {
    setStatus("loading");
    setError(undefined);
    try {
      const next = await getAuthoringV2(jobId);
      setSession(next);
      draftRef.current = next.authoring;
      setDraft(next.authoring);
      setPendingPatches([]);
      pendingRef.current = [];
      undoRef.current = [];
      redoRef.current = [];
      setUndoDepth(0);
      setRedoDepth(0);
      const recovery = readRecovery(jobId);
      if (recovery?.patches.length) setRecoveryCandidate(recovery);
      setStatus("clean");
    } catch (caught) {
      setStatus("error");
      setError(caught instanceof Error ? caught.message : String(caught));
    }
  }, [jobId]);

  useEffect(() => { void load(); }, [load]);

  const queuePatch = useCallback((patch: AuthoringPatchV2, recordHistory = true) => {
    const currentDraft = draftRef.current;
    const inverse = currentDraft ? inverseAuthoringPatch(currentDraft, patch) : undefined;
    let nextDraft: IeltsAuthoringIRV2 | undefined;
    try {
      nextDraft = currentDraft ? applyLocalPatches(currentDraft, [patch]) : undefined;
    } catch (caught) {
      setStatus("error");
      setError(caught instanceof Error ? caught.message : String(caught));
      return;
    }
    draftRef.current = nextDraft;
    setDraft(nextDraft);
    pendingRef.current = pendingRef.current.concat(patch);
    setPendingPatches(pendingRef.current);
    if (recordHistory && inverse) {
      undoRef.current = undoRef.current.concat({ patch, inverse });
      redoRef.current = [];
      setUndoDepth(undoRef.current.length);
      setRedoDepth(0);
    }
    setStatus("dirty");
  }, []);

  const undo = useCallback(() => {
    const entry = undoRef.current.pop();
    if (!entry) return;
    queuePatch(entry.inverse, false);
    redoRef.current = redoRef.current.concat(entry);
    setUndoDepth(undoRef.current.length);
    setRedoDepth(redoRef.current.length);
  }, [queuePatch]);

  const redo = useCallback(() => {
    const entry = redoRef.current.pop();
    if (!entry) return;
    queuePatch(entry.patch, false);
    undoRef.current = undoRef.current.concat(entry);
    setUndoDepth(undoRef.current.length);
    setRedoDepth(redoRef.current.length);
  }, [queuePatch]);

  const exportV2 = useCallback(async () => {
    if (!session || pendingRef.current.length) {
      setError("请等待自动保存完成后再导出。");
      return;
    }
    setExporting(true);
    setError(undefined);
    try {
      if (!NAS_PACKAGE_V2_ENABLED) {
        setError("NAS V2 发布功能当前未启用，请使用 V1 发布入口。");
        return;
      }
      const exportDir = await chooseExportDirectory();
      if (!exportDir) return;
      const result = await exportAuthoringV2({ jobId, exportDir, revision: session.revision });
      // The materialized runtime source is the only input to the NAS V2
      // publisher. It performs the lock/CAS/staging/probe/manifest-last
      // transaction and returns a durable report, so the visible editor
      // action now exercises the same production path as the student loader.
      const packageResult = await exportNasPackageV2({
        libraryRoot: exportDir,
        sourcePath: result.receipt.runtimePath,
        assetRoot: result.receipt.outputDir,
        examId: result.examId,
        minimumRuntimeVersion: "0.2.0"
      });
      setExportResult({
        outputDir: result.outputDir,
        revision: result.revision,
        package: {
          manifestPath: String(packageResult.manifestPath),
          reportPath: String(packageResult.reportPath),
          assetCount: Number(packageResult.assetCount || 0),
          probePassed: packageResult.probe?.passed === true
        }
      });
    } catch (caught) {
      setStatus("error");
      setError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setExporting(false);
    }
  }, [jobId, session]);

  const flush = useCallback(async (patches: AuthoringPatchV2[]) => {
    if (!session || !patches.length || saving) return;
    setSaving(true);
    setStatus("saving");
    try {
      const result = await applyAuthoringV2Patches({ jobId, baseRevision: session.revision, patches });
      const remaining = pendingRef.current.slice(patches.length);
      setSession(result);
      const nextDraft = remaining.length ? applyLocalPatches(result.authoring, remaining) : result.authoring;
      draftRef.current = nextDraft;
      setDraft(nextDraft);
      pendingRef.current = remaining;
      setPendingPatches(remaining);
      if (!remaining.length) window.localStorage.removeItem(recoveryKey(jobId));
      setStatus(remaining.length ? "dirty" : "saved");
      refresh?.();
    } catch (caught) {
      const message = caught instanceof Error ? caught.message : String(caught);
      setStatus(message.includes("revision_conflict") ? "conflict" : "error");
      setError(message);
    } finally {
      setSaving(false);
    }
  }, [jobId, refresh, saving, session]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey)) return;
      if (event.key.toLowerCase() === "z" && !event.shiftKey) {
        event.preventDefault();
        undo();
      } else if (event.key.toLowerCase() === "y" || (event.key.toLowerCase() === "z" && event.shiftKey)) {
        event.preventDefault();
        redo();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [redo, undo]);

  useEffect(() => {
    if (!session || !pendingPatches.length) return;
    const recovery: AuthoringEditorRecoveryV2 = { schemaVersion: "AuthoringEditorRecoveryV1", jobId, baseRevision: session.revision, updatedAt: new Date().toISOString(), patches: pendingPatches };
    window.localStorage.setItem(recoveryKey(jobId), JSON.stringify(recovery));
    const timer = window.setTimeout(() => { void flush(pendingPatches); }, 650);
    return () => window.clearTimeout(timer);
  }, [flush, jobId, pendingPatches, session]);

  useEffect(() => {
    if (!selectedId) return;
    const timer = window.setTimeout(() => {
      document.querySelector("[data-editor-id=\"" + selectedId + "\"]")?.scrollIntoView({ behavior: "smooth", block: "center" });
    }, 0);
    return () => window.clearTimeout(timer);
  }, [selectedId]);

  const selectIssue = useCallback((issue: IeltsAuthoringIRV2["quality"]["issues"][number]) => {
    const targetId = issue.targetId || issue.issueId;
    setSelectedId(targetId);
    window.setTimeout(() => {
      const target = Array.from(document.querySelectorAll<HTMLElement>("[data-editor-id]"))
        .find((element) => element.dataset.editorId === targetId);
      target?.scrollIntoView({ behavior: "smooth", block: "center" });
    }, 0);
  }, []);

  const activeTasks = draft?.taskGroups ?? [];
  const issues = draft?.quality.issues ?? [];
  const selectedIssue = selectedId ? issues.find((issue) => issue.targetId === selectedId || issue.issueId === selectedId) : undefined;
  const selectedAnchors = selectedIssue?.sourceAnchors?.length ? selectedIssue.sourceAnchors : draft && selectedId ? sourceAnchorsFor(draft, selectedId) : [];
  const staleDerivedQualityCodes = new Set(["ANSWER_KEY_MISSING_SLOT", "RUNTIME_COMPILER_FAILED"]);
  const unresolvedAnswerIds = draft ? Object.entries(draft.answerKey).filter(([, value]) => value.kind === "unresolved").map(([slotId]) => slotId) : [];
  const hardFailureCodes = draft?.quality.hardFailures.filter((code) => !staleDerivedQualityCodes.has(code)) ?? [];
  const blockingIssueIds = issues.filter((issue) => issue.severity === "blocking" && !staleDerivedQualityCodes.has(issue.code) && issue.details?.resolution !== "resolved" && issue.details?.resolution !== "ignored").map((issue) => issue.issueId);
  const exportBlockers = [
    ...unresolvedAnswerIds.map((slotId) => "答案位 " + slotId + " 仍未解析"),
    ...hardFailureCodes.map((code) => "质量硬失败：" + code),
    ...blockingIssueIds.map((issueId) => "未处理阻断 issue：" + issueId)
  ];
  const exportBlocked = exportBlockers.length > 0;
  const selectedContent = draft && selectedId ? locateContentNode(draft, selectedId)?.node : undefined;

  const runExport = useCallback(() => {
    if (exportBlocked) {
      setError("导出已阻断：请先处理下方列出的答案位、质量硬失败或阻断 issue。");
      return;
    }
    void exportV2();
  }, [exportBlocked, exportV2]);

  function editText(node: Extract<ContentNodeV2, { type: "text" }>) {
    const currentText = findEntity(draftRef.current, node.id)?.text;
    const fromLength = typeof currentText === "string" ? Array.from(currentText).length : 0;
    queuePatch({ op: "replaceText", nodeId: node.id, from: 0, to: fromLength, text: node.text });
  }

  function replaceContent(target: AuthoringContentTargetV2, content: ContentNodeV2[]) {
    queuePatch({ op: "replaceContent", target, content });
  }

  function newParagraph(): ContentNodeV2 {
    const stamp = Date.now().toString(36);
    return {
      type: "paragraph",
      id: `manual-paragraph-${stamp}`,
      sourceAnchors: [],
      provenanceStatus: "manual",
      children: [{
        type: "text",
        id: `manual-text-${stamp}`,
        sourceAnchors: [],
        provenanceStatus: "manual",
        text: "新段落"
      }]
    };
  }

  function insertAfterSelected() {
    if (!draft || !selectedId) return;
    const location = locateContentNode(draft, selectedId);
    if (!location) return;
    queuePatch({ op: "insertNode", target: location.target, parentId: location.parentId, index: location.index + 1, node: newParagraph() });
  }

  function deleteSelectedNode() {
    if (!selectedId || !draft || !locateContentNode(draft, selectedId)) return;
    queuePatch({ op: "deleteNode", nodeId: selectedId });
    setSelectedId(undefined);
  }

  function moveSelected(delta: number) {
    if (!selectedId || !draft) return;
    const location = locateContentNode(draft, selectedId);
    if (!location) return;
    const nextIndex = Math.max(0, location.index + delta);
    queuePatch({ op: "moveNode", nodeId: selectedId, target: location.target, parentId: location.parentId, index: nextIndex });
  }

  function updateCrop(nodeId: string, crop: [number, number, number, number] | null) {
    queuePatch({ op: "cropAsset", nodeId, crop });
  }

  function updateHotspot(nodeId: string, hotspot: NonNullable<Extract<ContentNodeV2, { type: "figure" | "diagram" }>['hotspots']>[number]) {
    queuePatch({ op: "setHotspot", nodeId, hotspot });
  }

  function expressionNumbers(expression: QuestionNumberExpressionV2): number[] {
    if (expression.kind === "range") {
      return Array.from({ length: expression.end - expression.start + 1 }, (_, index) => expression.start + index);
    }
    return expression.values.flatMap((value) => typeof value === "number"
      ? [value]
      : Array.from({ length: value.end - value.start + 1 }, (_, index) => value.start + index));
  }

  function expressionInput(expression: QuestionNumberExpressionV2): string {
    if (expression.kind === "range") return `${expression.start}-${expression.end}`;
    return expression.values.map((value) => typeof value === "number" ? String(value) : `${value.start}-${value.end}`).join(",");
  }

  function parseExpressionValues(raw: string, allowRanges: boolean): Array<number | { start: number; end: number }> | undefined {
    const tokens = raw.split(",").map((token) => token.trim()).filter(Boolean);
    if (!tokens.length) return undefined;
    const values: Array<number | { start: number; end: number }> = [];
    for (const token of tokens) {
      const range = token.match(/^(\d+)\s*-\s*(\d+)$/);
      if (range) {
        if (!allowRanges) return undefined;
        const start = Number(range[1]);
        const end = Number(range[2]);
        if (!Number.isInteger(start) || !Number.isInteger(end) || start < 1 || end < start || end - start > 200) return undefined;
        values.push({ start, end });
        continue;
      }
      if (!/^\d+$/.test(token)) return undefined;
      const number = Number(token);
      if (!Number.isInteger(number) || number < 1) return undefined;
      values.push(number);
    }
    const expanded = values.flatMap((value) => typeof value === "number"
      ? [value]
      : Array.from({ length: value.end - value.start + 1 }, (_, index) => value.start + index));
    return new Set(expanded).size === expanded.length ? values : undefined;
  }

  function updateQuestionExpression(task: TaskGroupV2, expression: QuestionNumberExpressionV2) {
    if (expression.kind === "range" && (!Number.isInteger(expression.start) || !Number.isInteger(expression.end) || expression.start < 1 || expression.end < expression.start || expression.end - expression.start > 200)) return;
    if (expression.kind === "set" && (!expression.values.length || expression.values.some((value) => !Number.isInteger(value) || value < 1) || new Set(expression.values).size !== expression.values.length)) return;
    if (expression.kind === "mixed" && (!expression.values.length || expression.values.some((value) => typeof value === "number" ? !Number.isInteger(value) || value < 1 : !Number.isInteger(value.start) || !Number.isInteger(value.end) || value.start < 1 || value.end < value.start || value.end - value.start > 200))) return;
    queuePatch({ op: "setQuestionExpression", taskId: task.taskId, expression });
  }

  function changeExpressionKind(task: TaskGroupV2, kind: QuestionNumberExpressionV2["kind"]) {
    if (kind === task.displayRange.kind) return;
    const values = expressionNumbers(task.displayRange);
    if (!values.length) return;
    if (kind === "range") {
      updateQuestionExpression(task, { kind, start: Math.min(...values), end: Math.max(...values) });
    } else if (kind === "set") {
      updateQuestionExpression(task, { kind, values });
    } else {
      updateQuestionExpression(task, { kind, values });
    }
  }

  function updateResponseCardinality(task: TaskGroupV2, group: ResponseGroupV2, field: "min" | "max" | "exact", raw: string) {
    if (field === "exact" && raw.trim() === "") {
      const { exact: _exact, ...withoutExact } = group.cardinality;
      queuePatch({ op: "setResponseCardinality", taskId: task.taskId, responseGroupId: group.responseGroupId, cardinality: withoutExact });
      return;
    }
    const value = Number(raw);
    if (!Number.isInteger(value) || value < 0) return;
    const next = { ...group.cardinality, [field]: value } as ResponseGroupV2["cardinality"];
    if (next.min > next.max) {
      if (field === "min") next.max = next.min;
      else next.min = next.max;
    }
    if (next.exact !== undefined) next.exact = Math.max(next.min, Math.min(next.max, next.exact));
    queuePatch({
      op: "setResponseCardinality",
      taskId: task.taskId,
      responseGroupId: group.responseGroupId,
      cardinality: next
    });
  }

  function updateResponseGroup(task: TaskGroupV2, group: ResponseGroupV2, changes: Partial<ResponseGroupV2>) {
    queuePatch({
      op: "setResponseGroup",
      taskId: task.taskId,
      responseGroup: { ...group, ...changes }
    });
  }

  function renderSelectedContentInspector() {
    if (!selectedContent) return null;
    const canMove = Boolean(locateContentNode(draft!, selectedContent.id));
    const media = selectedContent.type === "figure" || selectedContent.type === "image" || selectedContent.type === "diagram" ? selectedContent : undefined;
    return <>
      {canMove ? <div className="inspector-edit-actions"><div className="inspector-section-heading"><span>节点操作</span><small>结构化 patch</small></div><div className="button-row compact"><button className="ghost small" onClick={insertAfterSelected}>新增段落</button><button className="ghost small" onClick={() => moveSelected(-1)}>上移</button><button className="ghost small" onClick={() => moveSelected(1)}>下移</button><button className="danger small" onClick={deleteSelectedNode}>删除节点</button></div><p className="inspector-note">新增、删除和移动会写入同一棵 V2 文档树，并可用撤销/重做恢复。</p></div> : null}
      {media ? <div className="inspector-media-editor"><div className="inspector-section-heading"><span>资源 / 裁剪</span><small>{media.assetId}</small></div>{media.type === "image" ? <label className="inspector-field">替代文字<input value={media.altText ?? ""} onChange={(event) => queuePatch({ op: "setNodeAttrs", nodeId: media.id, attrs: { altText: event.target.value } })} /></label> : null}<div className="crop-grid">{(media.crop ?? [0, 0, 1, 1]).map((value, index) => <label key={index}>{["x", "y", "宽", "高"][index]}<input type="number" min={0} max={1} step={0.01} value={value} onChange={(event) => { const next = [...(media.crop ?? [0, 0, 1, 1])] as [number, number, number, number]; next[index] = Number(event.target.value); updateCrop(media.id, next); }} /></label>)}</div><button type="button" className="ghost small" onClick={() => updateCrop(media.id, null)}>恢复完整资源</button></div> : null}
      {media && (media.type === "figure" || media.type === "diagram") ? <div className="inspector-hotspot-editor"><div className="inspector-section-heading"><span>热点 / 答案位</span><small>{media.hotspots?.length ?? 0} 个</small></div><div className="hotspot-editor-board">{(media.hotspots ?? []).map((hotspot) => <button key={hotspot.hotspotId} type="button" draggable className="hotspot-editor-chip" style={{ left: `${hotspot.normalizedRect[0] * 100}%`, top: `${hotspot.normalizedRect[1] * 100}%`, width: `${hotspot.normalizedRect[2] * 100}%`, height: `${hotspot.normalizedRect[3] * 100}%` }} onDragEnd={(event) => { const board = event.currentTarget.parentElement?.getBoundingClientRect(); if (!board) return; const x = Math.max(0, Math.min(1 - hotspot.normalizedRect[2], (event.clientX - board.left) / board.width)); const y = Math.max(0, Math.min(1 - hotspot.normalizedRect[3], (event.clientY - board.top) / board.height)); updateHotspot(media.id, { ...hotspot, normalizedRect: [x, y, hotspot.normalizedRect[2], hotspot.normalizedRect[3]] }); }}>{hotspot.slotId}</button>)}</div>{(media.hotspots ?? []).map((hotspot) => <div className="hotspot-row" key={hotspot.hotspotId}><strong>{hotspot.slotId}</strong>{hotspot.normalizedRect.map((value, index) => <input key={index} type="number" min={0} max={1} step={0.01} value={value} aria-label={`${hotspot.slotId} ${["x", "y", "宽", "高"][index]}`} onChange={(event) => { const rect = [...hotspot.normalizedRect] as [number, number, number, number]; rect[index] = Number(event.target.value); updateHotspot(media.id, { ...hotspot, normalizedRect: rect }); }} />)}<button type="button" className="ghost small" onClick={() => queuePatch({ op: "removeHotspot", nodeId: media.id, hotspotId: hotspot.hotspotId })}>移除</button></div>)}<button type="button" className="ghost small" onClick={() => { const slot = Object.values(draft?.answerSlots ?? {})[0]; if (!slot) return; updateHotspot(media.id, { hotspotId: `${media.id}-hotspot-${(media.hotspots?.length ?? 0) + 1}`, slotId: slot.slotId, normalizedRect: [0.1, 0.1, 0.25, 0.15] }); }}>新增热点</button></div> : null}
    </>;
  }

  function toggleAnswer(slot: AnswerSlotV2, label: string) {
    const current = selectedLabels(draft?.answerKey[slot.slotId]);
    const next = current.includes(label) ? current.filter((item) => item !== label) : current.concat(label);
    queuePatch({ op: "setAnswer", slotId: slot.slotId, value: answerValueForSelection(next) });
  }

  function expressionLabel(task: TaskGroupV2): string {
    const expression = task.displayRange;
    if (expression.kind === "range") return "Questions " + expression.start + "–" + expression.end;
    if (expression.kind === "set") return "Questions " + expression.values.join(" and ");
    return "Questions " + expression.values.map((value) => typeof value === "number" ? String(value) : value.start + "–" + value.end).join(", ");
  }

  function renderResponseGroup(task: TaskGroupV2, group: ResponseGroupV2) {
    const bank = task.optionBank;
    return <section key={group.responseGroupId} className={"structured-response-card " + (selectedId === group.responseGroupId ? "selected" : "")} data-editor-id={group.responseGroupId} onClick={() => setSelectedId(group.responseGroupId)}>
      <div className="structured-card-heading"><div><span className="node-kicker">Response group</span><h4>{group.slotIds.map((slotId) => draft?.answerSlots[slotId]?.displayLabel).join("、")} · 共享题干</h4></div><small>{group.allowOptionReuse ? "选项可复用" : "选项不可复用"}</small></div>
      <div className="structured-response-controls" onClick={(event) => event.stopPropagation()}>
        <label>答案分配<select value={group.assignment} onChange={(event) => updateResponseGroup(task, group, { assignment: event.target.value as ResponseGroupV2["assignment"] })}>
          <option value="per_slot">每题独立</option>
          <option value="unordered_set">无序集合</option>
          <option value="ordered_slots">按序分配</option>
        </select></label>
        <label>评分策略<select value={group.scoringPolicy} onChange={(event) => updateResponseGroup(task, group, { scoringPolicy: event.target.value as ResponseGroupV2["scoringPolicy"] })}>
          <option value="per_slot_binary">每题二元</option>
          <option value="per_slot_ielts_normalized">IELTS 归一化</option>
          <option value="exact_set">集合完全匹配</option>
          <option value="all_or_nothing">全对才得分</option>
        </select></label>
        <label>重复策略<select value={group.duplicatePolicy} onChange={(event) => updateResponseGroup(task, group, { duplicatePolicy: event.target.value as ResponseGroupV2["duplicatePolicy"] })}>
          <option value="reject_submission">拒绝重复</option>
          <option value="ignore_duplicates">忽略重复</option>
        </select></label>
        <label className="checkbox-field"><input type="checkbox" checked={group.allowOptionReuse} onChange={(event) => updateResponseGroup(task, group, { allowOptionReuse: event.target.checked })} />选项可复用</label>
        <label>optionBankRef<input list={`option-bank-${task.taskId}`} value={group.optionBankRef ?? ""} placeholder={bank?.optionBankId ?? "可选"} onChange={(event) => updateResponseGroup(task, group, { optionBankRef: event.target.value.trim() || undefined })} /></label>
        <datalist id={`option-bank-${task.taskId}`}>{bank ? <option value={bank.optionBankId} /> : null}</datalist>
        <div className="cardinality-editor"><span>答案数量</span><label>min<input type="number" min={0} value={group.cardinality.min} onChange={(event) => updateResponseCardinality(task, group, "min", event.target.value)} /></label><label>max<input type="number" min={0} value={group.cardinality.max} onChange={(event) => updateResponseCardinality(task, group, "max", event.target.value)} /></label><label>exact<input type="number" min={0} value={group.cardinality.exact ?? ""} placeholder="—" onChange={(event) => updateResponseCardinality(task, group, "exact", event.target.value)} /></label></div>
      </div>
      <div className="shared-prompt"><span className="node-kicker">Prompt</span><AuthoringTiptapEditor nodes={group.prompt ?? []} onSelect={setSelectedId} onChange={(nodes) => replaceContent({ kind: "responsePrompt", responseGroupId: group.responseGroupId }, nodes)} ariaLabel="共享题干编辑器" /></div>
      {bank ? <div className="option-bank-editor" data-editor-id={bank.optionBankId}><div className="option-bank-heading"><strong>{nodeText(bank.title)}</strong><small>{bank.options.length} 个公共选项</small></div>{bank.options.map((option) => <div className="option-editor-row" key={option.optionId} data-editor-id={option.optionId}><b>{option.label}</b><AuthoringTiptapEditor nodes={option.content} onSelect={setSelectedId} onChange={(nodes) => replaceContent({ kind: "option", optionId: option.optionId }, nodes)} ariaLabel={`选项 ${option.label} 编辑器`} /></div>)}</div> : null}
      <div className="slot-editor-grid">{group.slotIds.map((slotId) => {
        const slot = draft?.answerSlots[slotId];
        if (!slot) return null;
        const labels = selectedLabels(draft?.answerKey[slotId]);
        return <article key={slotId} className={"slot-editor-card " + (selectedId === slotId ? "selected" : "")} data-editor-id={slotId} onClick={() => setSelectedId(slotId)}>
          <div className="slot-editor-heading"><strong>{slot.displayLabel}</strong><small>{slot.interaction} · {Math.round(slot.confidence * 100)}%</small></div>
          <div className="choice-chip-row">{(bank?.options ?? []).map((option) => <button key={option.optionId} type="button" className={"choice-chip " + (labels.includes(option.label) ? "active" : "")} onClick={(event) => { event.stopPropagation(); toggleAnswer(slot, option.label); }}>{option.label}</button>)}</div>
          <label className="slot-text-fallback">手工文本<input value={answerText(draft?.answerKey[slotId])} onChange={(event) => queuePatch({ op: "setAnswer", slotId, value: { kind: "text", values: [event.target.value], normalization: "ielts_default" } })} placeholder="仅适用于填空题" /></label>
        </article>;
      })}</div>
    </section>;
  }

  function renderTaskEditor(task: TaskGroupV2) {
    const responseRules = task.responseGroups.map((group) => `${group.cardinality.exact ?? group.cardinality.max ?? 1} 个答案 · ${group.assignment === "unordered_set" ? "无序集合" : "按序"}`).join("；");
    const expression = task.displayRange;
    return <section key={task.taskId} className="structured-task-card" data-editor-id={task.taskId}>
      <div className="structured-card-heading"><div><span className="node-kicker">Task group</span><h3>{expressionLabel(task)}</h3></div><span className={"editor-review-state " + task.reviewState}>{task.reviewState === "confirmed" ? "已确认" : "待确认"}</span></div>
      <div className="structured-task-controls"><label>题型<select value={task.taskType} onChange={(event) => queuePatch({ op: "setTaskType", taskId: task.taskId, taskType: event.target.value as TaskTypeV2 })}>{TASK_TYPES.map((type) => <option key={type} value={type}>{taskTypeLabel(type)}</option>)}</select></label><div className="semantic-rule"><span>选择规则</span><strong>{responseRules || "暂无 response group"}</strong></div></div>
      <div className="structured-expression-editor" onClick={(event) => event.stopPropagation()}>
        <label>题号表达式<select value={expression.kind} onChange={(event) => changeExpressionKind(task, event.target.value as QuestionNumberExpressionV2["kind"])}>
          <option value="range">连续范围</option>
          <option value="set">题号集合</option>
          <option value="mixed">混合范围/集合</option>
        </select></label>
        {expression.kind === "range" ? <><label>start<input type="number" min={1} value={expression.start} onChange={(event) => updateQuestionExpression(task, { kind: "range", start: Number(event.target.value), end: expression.end })} /></label><label>end<input type="number" min={1} value={expression.end} onChange={(event) => updateQuestionExpression(task, { kind: "range", start: expression.start, end: Number(event.target.value) })} /></label></> : <label>题号（逗号分隔；mixed 支持 14-16）<input value={expressionInput(expression)} onChange={(event) => { const values = parseExpressionValues(event.target.value, expression.kind === "mixed"); if (!values) return; updateQuestionExpression(task, expression.kind === "set" ? { kind: "set", values: values as number[] } : { kind: "mixed", values }); }} /></label>}
      </div>
      <div className="structured-instruction"><span className="node-kicker">Instruction</span><AuthoringTiptapEditor nodes={task.instructions} onSelect={setSelectedId} onChange={(nodes) => replaceContent({ kind: "taskInstructions", taskId: task.taskId }, nodes)} ariaLabel="题组说明编辑器" /></div>
      {task.stimulus?.length ? <div className="structured-stimulus"><span className="node-kicker">Stimulus</span><AuthoringTiptapEditor nodes={task.stimulus} onSelect={setSelectedId} onChange={(nodes) => replaceContent({ kind: "taskStimulus", taskId: task.taskId }, nodes)} ariaLabel="题组刺激材料编辑器" /></div> : null}
      {task.responseGroups.length ? task.responseGroups.map((group) => renderResponseGroup(task, group)) : <div className="empty">当前题组没有 response group。</div>}
    </section>;
  }

  if (status === "loading") return <section className="page-enter"><p className="eyebrow">Phase 5</p><h2>正在打开结构化编辑器…</h2></section>;
  if (!draft || !session) return <section className="page-enter phase5-empty"><p className="eyebrow">Phase 5 · 结构化编辑器</p><h2>当前任务还没有 V2 编辑稿</h2><p>{error ?? "需要先生成 authoring-ir-v2.shadow.json，或从 Phase 5 架构 fixture 开始。"}</p><div className="button-row"><button className="ghost" onClick={() => go("/jobs")}>返回任务列表</button><button className="primary" onClick={() => go("/phase5")}>打开 Phase 5 fixture</button></div></section>;

  return <section className="page-enter phase5-editor" data-testid="structured-authoring-editor-v2">
    <header className="phase5-editor-header"><div><p className="eyebrow">Phase 5 · Schema-driven authoring</p><h2>{draft.exam.title}</h2><p>编辑结构化题源；学生预览、答案位和发布输入都来自同一棵文档树。</p></div><div className="phase5-header-actions"><span className={"editor-save-state " + status}>{status === "saving" ? "自动保存中" : status === "dirty" ? "有未保存修改" : status === "saved" ? "已保存" : "revision " + session.revision}</span><button className="ghost small" disabled={!undoDepth} onClick={undo}>撤销 {undoDepth ? `(${undoDepth})` : ""}</button><button className="ghost small" disabled={!redoDepth} onClick={redo}>重做 {redoDepth ? `(${redoDepth})` : ""}</button><button className="ghost" disabled={exporting || Boolean(pendingPatches.length) || exportBlocked || !NAS_PACKAGE_V2_ENABLED} title={!NAS_PACKAGE_V2_ENABLED ? "NAS V2 rollout 未启用" : exportBlocked ? "请先处理导出阻断项" : undefined} onClick={runExport}>{exporting ? "导出中…" : "导出 V2 包"}</button><button className="ghost" onClick={() => go(jobId === "phase5-editor-fixture" ? "/phase5" : "/jobs/" + jobId + "/preview")}>退出编辑</button><button className="primary" onClick={() => setViewMode(viewMode === "edit" ? "preview" : "edit")}>{viewMode === "edit" ? "学生端预览" : "返回编辑"}</button></div></header>
    {exportResult ? <div className="info-box phase5-recovery-banner"><strong>V2 NAS 发布完成 · revision {exportResult.revision}</strong><p>学生预览与导出使用同一份 authoring V2：{exportResult.outputDir}</p><p>probe：{exportResult.package?.probePassed ? "通过" : "未通过"} · assets：{exportResult.package?.assetCount ?? 0}</p><small>manifest：{exportResult.package?.manifestPath}<br />report：{exportResult.package?.reportPath}</small><button className="ghost small" onClick={() => setExportResult(undefined)}>关闭</button></div> : null}
    {recoveryCandidate ? <div className={"info-box phase5-recovery-banner " + (recoveryCandidate.baseRevision === session.revision ? "" : "warning-box")}><strong>{recoveryCandidate.baseRevision === session.revision ? "发现未提交的本地修改" : "发现与服务器 revision 冲突的本地修改"}</strong><p>浏览器关闭前留下 {recoveryCandidate.patches.length} 个 patch，基于 revision {recoveryCandidate.baseRevision}；当前服务器 revision 为 {session.revision}。恢复会尝试在当前版本上重放，失败时不会覆盖服务器稿。</p><div className="button-row"><button className="primary small" onClick={() => { try { const recovered = applyLocalPatches(draft, recoveryCandidate.patches); draftRef.current = recovered; setDraft(recovered); pendingRef.current = recoveryCandidate.patches; setPendingPatches(recoveryCandidate.patches); setRecoveryCandidate(undefined); } catch (caught) { setError(caught instanceof Error ? caught.message : String(caught)); } }}>恢复并尝试重放</button><button className="ghost small" onClick={() => { window.localStorage.removeItem(recoveryKey(jobId)); setRecoveryCandidate(undefined); }}>放弃</button></div></div> : null}
    {status === "conflict" ? <div className="warning-box phase5-recovery-banner"><strong>保存冲突：服务器已有新 revision</strong><p>为避免覆盖另一窗口的修改，当前本地 patch 已保留。请重新加载后手工合并。</p><div className="button-row"><button className="primary small" onClick={() => void load()}>重新加载服务器版本</button></div></div> : null}
    {error && status !== "conflict" ? <div className="warning-box phase5-recovery-banner"><strong>编辑器提示</strong><p>{error}</p></div> : null}
    <div className={"phase5-export-blockers " + (exportBlocked ? "blocked" : "ready")} data-testid="phase5-export-blockers"><strong>{exportBlocked ? "导出已阻断" : "导出检查通过"}</strong>{exportBlocked ? <ul>{exportBlockers.map((blocker) => <li key={blocker}>{blocker}</li>)}</ul> : <p>答案位、质量硬失败和阻断 issue 均满足 V2 发布门槛。</p>}</div>
    <div className="phase5-editor-toolbar"><div className="phase5-stat"><span>Task groups</span><strong>{draft.taskGroups.length}</strong></div><div className="phase5-stat"><span>Answer slots</span><strong>{Object.keys(draft.answerSlots).length}</strong></div><div className="phase5-stat"><span>Issues</span><strong>{issues.length}</strong></div><div className="phase5-stat"><span>Source coverage</span><strong>{Math.round(draft.quality.sourceCoverage * 100)}%</strong></div><small>来源：{session.source} · V1 文件保持可读</small></div>
    {viewMode === "preview" ? <StudentPreview authoring={draft} /> : <div className="phase5-editor-grid">
      <aside className="phase5-outline"><div className="inspector-section-heading"><span>文档结构</span><small>点击定位</small></div><button className={"outline-row " + (selectedId === draft.passage?.content[0]?.id ? "active" : "")} onClick={() => setSelectedId(draft.passage?.content[0]?.id)}><strong>Passage</strong><small>{draft.passage?.content.length ?? 0} 个内容节点</small></button>{draft.taskGroups.map((task) => <div key={task.taskId} className="outline-group"><button className={"outline-row " + (selectedId === task.taskId ? "active" : "")} onClick={() => setSelectedId(task.taskId)}><strong>{expressionLabel(task)}</strong><small>{taskTypeLabel(task.taskType)}</small></button>{task.responseGroups.map((group) => <button key={group.responseGroupId} className={"outline-row nested " + (selectedId === group.responseGroupId ? "active" : "")} onClick={() => setSelectedId(group.responseGroupId)}>共享 response group <small>{group.slotIds.map((slotId) => draft.answerSlots[slotId]?.displayLabel).join("、")}</small></button>)}{task.optionBank ? <button className={"outline-row nested " + (selectedId === task.optionBank.optionBankId ? "active" : "")} onClick={() => setSelectedId(task.optionBank?.optionBankId)}>公共选项池 <small>{task.optionBank.options.length} 项</small></button> : null}</div>)}<div className="issue-rail"><div className="inspector-section-heading"><span>需要确认</span><small>{issues.length} 项</small></div>{issues.map((issue) => <button key={issue.issueId} className={"issue-rail-item " + issue.severity} onClick={() => selectIssue(issue)}><strong>{issue.code}</strong><span>{issue.message}</span></button>)}</div></aside>
      <main className="phase5-editor-canvas"><article className="structured-paper" data-editor-id={draft.passage?.content[0]?.id}><div className="structured-paper-label">PASSAGE · 可编辑节点树 · Tiptap schema</div><h3>{draft.passage?.title}</h3><AuthoringTiptapEditor nodes={draft.passage?.content ?? []} onSelect={setSelectedId} onChange={(nodes) => replaceContent({ kind: "passage" }, nodes)} ariaLabel="阅读文章编辑器" /></article>{activeTasks.length ? activeTasks.map((task) => renderTaskEditor(task)) : <div className="empty">暂无题组。</div>}</main>
      <aside className="phase5-inspector"><div className="inspector-section-heading"><span>节点检查器</span><small>{selectedId ?? "未选择"}</small></div>{selectedId ? <><p className="selected-node-name">{String(findEntity(draft, selectedId)?.type ?? "semantic entity")}</p><p className="source-summary">{sourceSummary(selectedAnchors)}</p><dl className="phase5-detail-list"><dt>节点 / 实体</dt><dd>{selectedId}</dd><dt>来源模式</dt><dd>{selectedAnchors[0]?.extractionMode ?? "unknown"}</dd><dt>来源节点</dt><dd>{selectedAnchors.flatMap((anchor) => anchor.nodeIds).join("、") || "—"}</dd></dl>{renderSelectedContentInspector()}</> : <p className="empty">从左侧结构、题干或选项开始。</p>}<SourceOverlay authoring={draft} selectedId={selectedId} anchorsOverride={selectedAnchors} /><div className="inspector-section-heading"><span>编辑协议</span><small>V2 patch</small></div><p className="inspector-note">Tiptap 事务会映射成 replaceContent；节点操作、表格、资源裁剪和热点均以 append-only revision 保存。服务器按 base revision 拒绝并发覆盖。</p></aside>
    </div>}
  </section>;
}
