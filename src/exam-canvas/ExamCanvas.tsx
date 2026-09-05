import { useEffect, useMemo, useState, type CSSProperties, type ReactNode } from "react";
import { InlineTextEditor } from "./editors/InlineTextEditor";
import { MatchingMatrix, matchingRowsFor } from "./renderers/MatchingMatrix";
import { resolveAuthoringAssetPreview, type AuthoringAssetPreview } from "../api/tauriCommands";
import { buildReadingInteractionModelV2, buildRuntimeViewModelV2 } from "../services/runtimeViewModelV2";
import type { AnswerValueV2, ContentNodeV2, IeltsAuthoringIRV2, OptionV2, ResponseGroupV2, TaskGroupV2 } from "../types";

export type ExamCanvasStructureAction =
  | { type: "option.add"; taskId: string; responseGroupId: string; afterOptionId?: string }
  | { type: "option.move"; taskId: string; responseGroupId: string; optionId: string; direction: "up" | "down" }
  | { type: "option.delete"; taskId: string; responseGroupId: string; optionId: string }
  | { type: "table.row.add"; tableId: string; afterRowId?: string }
  | { type: "table.row.delete"; tableId: string; rowId: string }
  | { type: "table.column.add"; tableId: string; afterColumnIndex?: number }
  | { type: "table.column.delete"; tableId: string; columnIndex: number }
  | { type: "answer-slot.insert"; afterNodeId: string }
  | { type: "answer-slot.delete"; nodeId: string; slotId: string };

export interface ExamCanvasProps {
  authoring: IeltsAuthoringIRV2;
  mode: "student" | "author";
  selectedId?: string;
  onSelect?: (id: string) => void;
  onTextChange?: (node: Extract<ContentNodeV2, { type: "text" }>) => void;
  /** 优先于 onTextChange。`expectedText` 是进入编辑那一刻的文本，用于乐观并发校验：
   *  如果编辑过程中草稿被重新加载或被云端结果合并过，这次提交会被拒绝而不是静默覆盖。 */
  onTextCommand?: (command: { nodeId: string; expectedText: string; text: string }) => void;
  onAnswerChange?: (slotId: string, value: AnswerValueV2) => void;
  onStructureAction?: (action: ExamCanvasStructureAction) => void;
}

type VisualNodeV2 = Extract<ContentNodeV2, { type: "figure" | "image" | "diagram" }>;

const assetPreviewCache = new Map<string, Promise<AuthoringAssetPreview | undefined>>();

function previewFor(jobId: string, assetId: string): Promise<AuthoringAssetPreview | undefined> {
  const key = `${jobId}:${assetId}`;
  const cached = assetPreviewCache.get(key);
  if (cached) return cached;
  const pending = resolveAuthoringAssetPreview(jobId, assetId);
  assetPreviewCache.set(key, pending);
  return pending;
}

function AuthorTools({
  canvas,
  label,
  children,
  compact = false
}: {
  canvas: ExamCanvasProps;
  label: string;
  children: ReactNode;
  compact?: boolean;
}) {
  if (canvas.mode !== "author" || !canvas.onStructureAction) return null;
  return <div
    className={`v2-author-tools${compact ? " is-compact" : ""}`}
    role="toolbar"
    aria-label={label}
    onClick={(event) => event.stopPropagation()}
  >{children}</div>;
}

function ToolButton({ label, disabled, onClick, children }: {
  label: string;
  disabled?: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return <button type="button" title={label} aria-label={label} disabled={disabled} onClick={onClick}>{children}</button>;
}

function OptionBankTools({ canvas, taskId, responseGroupId, options }: {
  canvas: ExamCanvasProps;
  taskId: string;
  responseGroupId: string;
  options: OptionV2[];
}) {
  return <AuthorTools canvas={canvas} label="编辑选项库">
    <ToolButton label="在末尾添加选项" onClick={() => canvas.onStructureAction?.({
      type: "option.add",
      taskId,
      responseGroupId,
      afterOptionId: options.at(-1)?.optionId
    })}>＋选项</ToolButton>
    {options.map((option, index) => <span key={option.optionId} className="v2-author-option-tools">
      <b>{option.label}</b>
      <ToolButton label={`上移选项 ${option.label}`} disabled={index === 0} onClick={() => canvas.onStructureAction?.({ type: "option.move", taskId, responseGroupId, optionId: option.optionId, direction: "up" })}>↑</ToolButton>
      <ToolButton label={`下移选项 ${option.label}`} disabled={index === options.length - 1} onClick={() => canvas.onStructureAction?.({ type: "option.move", taskId, responseGroupId, optionId: option.optionId, direction: "down" })}>↓</ToolButton>
      <ToolButton label={`删除选项 ${option.label}`} disabled={options.length <= 1} onClick={() => canvas.onStructureAction?.({ type: "option.delete", taskId, responseGroupId, optionId: option.optionId })}>×</ToolButton>
    </span>)}
  </AuthorTools>;
}

function contentText(nodes: ContentNodeV2[] | undefined): string {
  if (!nodes) return "";
  return nodes.map((node) => {
    if (node.type === "text") return node.text;
    if ("children" in node) return contentText(node.children);
    return "";
  }).join("");
}

function selectedValues(value: AnswerValueV2 | undefined): string[] {
  if (value?.kind === "option") return value.labels;
  if (value?.kind === "text") return value.values;
  return [];
}

/** 文本节点：作者模式下双击或点击进入原位编辑，学生模式下就是一个普通 span。
 *  两种模式渲染同一个 `span.v2-text`，编辑器只是聚焦时替换其内容，
 *  这样 author/student 的语义 DOM 保持一致（计划 §19.6 parity）。 */
function EditableTextNode({
  node,
  canvas
}: {
  node: Extract<ContentNodeV2, { type: "text" }>;
  canvas: ExamCanvasProps;
}) {
  const [editing, setEditing] = useState(false);
  // 进入编辑时快照当前文本，作为提交时的 expectedText。
  const [expectedText, setExpectedText] = useState(node.text);
  const author = canvas.mode === "author";
  const editable = Boolean(canvas.onTextCommand ?? canvas.onTextChange);
  const beginEditing = () => {
    setExpectedText(node.text);
    setEditing(true);
  };
  const commitText = (text: string) => {
    setEditing(false);
    if (text === node.text) return;
    if (canvas.onTextCommand) canvas.onTextCommand({ nodeId: node.id, expectedText, text });
    else canvas.onTextChange?.({ ...node, text });
  };
  const selected = canvas.selectedId === node.id;
  const marks = (node.marks ?? []).map((mark) => (typeof mark === "string" ? mark : ""));
  const className = [
    "v2-text",
    marks.includes("bold") ? "is-bold" : "",
    marks.includes("italic") ? "is-italic" : "",
    marks.includes("underline") ? "is-underlined" : "",
    author ? "v2-author-editable" : "",
    selected ? "is-selected" : "",
    editing ? "is-editing" : ""
  ].filter(Boolean).join(" ");

  if (author && editing) {
    return (
      <span key={node.id} className={className} data-editor-id={node.id}>
        <InlineTextEditor
          value={expectedText}
          ariaLabel="编辑题目文字"
          onCommit={commitText}
          onCancel={() => setEditing(false)}
        />
      </span>
    );
  }

  return (
    <span
      key={node.id}
      className={className}
      data-editor-id={node.id}
      // 作者模式下文本要能被键盘聚焦并进入编辑，否则原位编辑对键盘用户不可达。
      tabIndex={author ? 0 : undefined}
      role={author ? "button" : undefined}
      aria-label={author ? `编辑文字：${node.text.slice(0, 40)}` : undefined}
      onClick={(event) => {
        if (!author) return;
        event.stopPropagation();
        canvas.onSelect?.(node.id);
        if (editable) beginEditing();
      }}
      onKeyDown={(event) => {
        if (!author || !editable) return;
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          event.stopPropagation();
          beginEditing();
        }
      }}
    >
      {node.text}
    </span>
  );
}

function nodeStyle(node: ContentNodeV2): CSSProperties | undefined {
  if (node.type === "paragraph") return node.align ? { textAlign: node.align } : undefined;
  if (node.type === "heading") return undefined;
  return undefined;
}

function VisualAssetNode({ node, canvas, select }: {
  node: VisualNodeV2;
  canvas: ExamCanvasProps;
  select: (event: React.MouseEvent) => void;
}) {
  const [preview, setPreview] = useState<AuthoringAssetPreview>();
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let active = true;
    setPreview(undefined);
    setFailed(false);
    previewFor(canvas.authoring.jobId, node.assetId)
      .then((resolved) => {
        if (!active) return;
        setPreview(resolved);
        setFailed(!resolved);
      })
      .catch(() => active && setFailed(true));
    return () => { active = false; };
  }, [canvas.authoring.jobId, node.assetId]);

  const selected = canvas.selectedId === node.id;
  const authorClass = canvas.mode === "author" ? " v2-author-node" : "";
  const selectedClass = selected ? " is-selected" : "";
  const crop = node.crop ?? [0, 0, 1, 1];
  const [cropX, cropY, cropWidth, cropHeight] = crop;
  const width = Math.max(0.01, cropWidth);
  const height = Math.max(0.01, cropHeight);
  const displayWidth = Math.min(100, Math.max(10, node.display?.widthPercent ?? 100));
  const align = node.display?.align ?? "center";
  const frameStyle: CSSProperties = {
    width: `${displayWidth}%`,
    maxWidth: node.display?.maxWidthPx ? `${node.display.maxWidthPx}px` : undefined,
    marginLeft: align === "center" || align === "right" ? "auto" : undefined,
    marginRight: align === "center" || align === "left" ? "auto" : undefined,
    aspectRatio: preview?.widthPx && preview.heightPx
      ? `${preview.widthPx * width} / ${preview.heightPx * height}`
      : undefined
  };
  const imageStyle: CSSProperties = {
    width: `${100 / width}%`,
    maxWidth: "none",
    left: `${-cropX * 100 / width}%`,
    top: `${-cropY * 100 / height}%`
  };
  const altText = node.type === "image"
    ? node.altText || canvas.authoring.assets.find((asset) => asset.assetId === node.assetId)?.altText || ""
    : canvas.authoring.assets.find((asset) => asset.assetId === node.assetId)?.altText || "";

  return <figure
    data-editor-id={node.id}
    className={`v2-figure-wrapper v2-author-asset${authorClass}${selectedClass}`}
    onClick={select}
  >
    <div className="v2-asset-frame" style={frameStyle}>
      {preview ? <img src={preview.resourceUri} alt={altText} style={imageStyle} draggable={false} /> : <div className="v2-asset-placeholder"><span>{failed ? "视觉资源无法读取" : "正在载入视觉资源…"}</span><small>{node.assetId}</small></div>}
      {node.type !== "image" ? (node.hotspots ?? []).map((hotspot) => {
        const left = (hotspot.normalizedRect[0] - cropX) / width;
        const top = (hotspot.normalizedRect[1] - cropY) / height;
        const hotspotWidth = hotspot.normalizedRect[2] / width;
        const hotspotHeight = hotspot.normalizedRect[3] / height;
        if (left + hotspotWidth <= 0 || top + hotspotHeight <= 0 || left >= 1 || top >= 1) return null;
        return <button
          key={hotspot.hotspotId}
          type="button"
          className="v2-canvas-hotspot"
          style={{ left: `${left * 100}%`, top: `${top * 100}%`, width: `${hotspotWidth * 100}%`, height: `${hotspotHeight * 100}%` }}
          onClick={(event) => {
            event.stopPropagation();
            if (canvas.mode === "author") canvas.onSelect?.(hotspot.slotId);
          }}
        >{canvas.authoring.answerSlots[hotspot.slotId]?.displayLabel ?? hotspot.slotId}</button>;
      }) : null}
    </div>
    {node.type === "figure" && node.caption?.length ? <figcaption><ContentNodes nodes={node.caption} canvas={canvas} /></figcaption> : null}
  </figure>;
}

function ContentNodes({ nodes, canvas }: { nodes: ContentNodeV2[] | undefined; canvas: ExamCanvasProps }): ReactNode {
  if (!nodes?.length) return null;
  return nodes.map((node) => {
    const selected = canvas.selectedId === node.id;
    const authorClass = canvas.mode === "author" ? " v2-author-node" : "";
    const selectedClass = selected ? " is-selected" : "";
    const select = (event: React.MouseEvent) => {
      if (canvas.mode !== "author") return;
      event.stopPropagation();
      canvas.onSelect?.(node.id);
    };
    switch (node.type) {
      case "text":
        return <EditableTextNode key={node.id} node={node} canvas={canvas} />;
      case "hard_break":
        return <br key={node.id} />;
      case "paragraph":
        return <p key={node.id} data-editor-id={node.id} className={`v2-paragraph${authorClass}${selectedClass}`} style={nodeStyle(node)} onClick={select}><ContentNodes nodes={node.children} canvas={canvas} /></p>;
      case "heading": {
        const Heading = `h${Math.min(6, Math.max(1, node.level))}` as "h1" | "h2" | "h3" | "h4" | "h5" | "h6";
        return <Heading key={node.id} data-editor-id={node.id} className={`v2-heading${authorClass}${selectedClass}`} onClick={select}><ContentNodes nodes={node.children} canvas={canvas} /></Heading>;
      }
      case "bullet_list":
      case "ordered_list": {
        const List = node.type === "bullet_list" ? "ul" : "ol";
        return <List key={node.id} data-editor-id={node.id} className={`v2-list v2-list-${node.type === "bullet_list" ? "bullet" : "ordered"}${authorClass}${selectedClass}`} onClick={select}>{node.items.map((item) => <li key={item.id} data-editor-id={item.id}><ContentNodes nodes={item.children} canvas={canvas} /></li>)}</List>;
      }
      case "table": {
        const columnCount = Math.max(0, ...node.rows.map((row) => row.cells.reduce((count, cell) => count + Math.max(1, cell.colSpan), 0)));
        const table = <table key={node.id} data-editor-id={node.id} className={`v2-table${authorClass}${selectedClass}`} onClick={select}><tbody>{node.rows.map((row) => <tr key={row.id} data-editor-id={row.id}>{row.cells.map((cell) => { const Cell = cell.headerScope && cell.headerScope !== "none" ? "th" : "td"; return <Cell key={cell.id} data-editor-id={cell.id} rowSpan={cell.rowSpan || 1} colSpan={cell.colSpan || 1} scope={Cell === "th" ? cell.headerScope === "column" ? "col" : "row" : undefined}><ContentNodes nodes={cell.children} canvas={canvas} /></Cell>; })}</tr>)}</tbody></table>;
        if (canvas.mode !== "author" || !canvas.onStructureAction) return table;
        const lastRow = node.rows.at(-1);
        return <div key={node.id} className="v2-table-frame">
          <AuthorTools canvas={canvas} label="编辑表格">
            <ToolButton label="在末尾添加一行" onClick={() => canvas.onStructureAction?.({ type: "table.row.add", tableId: node.id, afterRowId: lastRow?.id })}>＋行</ToolButton>
            <ToolButton label="删除末行" disabled={!lastRow || node.rows.length <= 1} onClick={() => lastRow && canvas.onStructureAction?.({ type: "table.row.delete", tableId: node.id, rowId: lastRow.id })}>－行</ToolButton>
            <ToolButton label="在末尾添加一列" onClick={() => canvas.onStructureAction?.({ type: "table.column.add", tableId: node.id, afterColumnIndex: columnCount ? columnCount - 1 : undefined })}>＋列</ToolButton>
            <ToolButton label="删除末列" disabled={columnCount <= 1} onClick={() => canvas.onStructureAction?.({ type: "table.column.delete", tableId: node.id, columnIndex: columnCount - 1 })}>－列</ToolButton>
          </AuthorTools>
          {table}
        </div>;
      }
      case "figure":
      case "image":
      case "diagram":
        return <VisualAssetNode key={node.id} node={node} canvas={canvas} select={select} />;
      case "flowchart":
        return <section key={node.id} data-editor-id={node.id} className={`v2-flowchart${authorClass}${selectedClass}`} onClick={select}>{node.steps.map((step) => <div key={step.id} data-editor-id={step.id} className="v2-flow-step">{step.label ? <strong>{step.label}</strong> : null}<ContentNodes nodes={step.children} canvas={canvas} /></div>)}</section>;
      case "answer_slot": {
        const slot = canvas.authoring.answerSlots[node.slotId];
        if (!slot) return null;
        const values = selectedValues(canvas.authoring.answerKey[node.slotId]);
        const removeTools = <AuthorTools canvas={canvas} label={`编辑答案位 ${slot.displayLabel}`} compact>
          <ToolButton label={`在此答案位后插入答案位`} onClick={() => canvas.onStructureAction?.({ type: "answer-slot.insert", afterNodeId: node.id })}>＋</ToolButton>
          <ToolButton label={`删除答案位 ${slot.displayLabel}`} onClick={() => canvas.onStructureAction?.({ type: "answer-slot.delete", nodeId: node.id, slotId: slot.slotId })}>×</ToolButton>
        </AuthorTools>;
        const withTools = (control: ReactNode) => canvas.mode === "author" && canvas.onStructureAction
          ? <span key={node.id} className="v2-answer-slot-frame">{control}{removeTools}</span>
          : control;
        if (slot.interaction === "text") {
          return withTools(<label key={node.id} data-editor-id={node.id} className={`v2-answer-slot v2-answer-slot-text${authorClass}${selectedClass}`} onClick={select}><span className="v2-slot-label">{slot.displayLabel}</span><input type="text" name={slot.slotId} value={canvas.mode === "author" ? values[0] ?? "" : undefined} defaultValue={canvas.mode === "student" ? "" : undefined} placeholder={node.placeholder || "Answer"} onChange={(event) => canvas.mode === "author" && canvas.onAnswerChange?.(slot.slotId, { kind: "text", values: [event.target.value], normalization: "ielts_default" })} /></label>);
        }
        if (slot.interaction === "hotspot") {
          return withTools(<button key={node.id} type="button" data-editor-id={node.id} className={`v2-answer-slot v2-answer-slot-hotspot${authorClass}${selectedClass}`} onClick={select}>{slot.displayLabel}{values[0] ? `: ${values[0]}` : ""}</button>);
        }
        return withTools(<span key={node.id} data-editor-id={node.id} className={`v2-answer-slot v2-answer-slot-badge${authorClass}${selectedClass}`} onClick={select}>{slot.displayLabel}</span>);
      }
      case "option_bank":
        return <section key={node.id} data-editor-id={node.id} className={`v2-option-bank${authorClass}${selectedClass}`} onClick={select}><h4>Options</h4><ul>{node.options.map((option) => <li key={option.optionId} data-editor-id={option.optionId}><strong>{option.label}</strong> <ContentNodes nodes={option.children} canvas={canvas} /></li>)}</ul></section>;
      case "horizontal_rule":
        return <hr key={node.id} data-editor-id={node.id} className={`${authorClass}${selectedClass}`} onClick={select} />;
      case "doc":
      case "list_item":
      case "table_row":
      case "table_cell":
      case "flow_step":
      case "figcaption":
        return <span key={node.id} data-editor-id={node.id} className={`v2-node-fallback${authorClass}${selectedClass}`} onClick={select}>{"children" in node ? <ContentNodes nodes={node.children} canvas={canvas} /> : null}</span>;
    }
  });
}

function optionValue(option: OptionV2): string {
  return contentText(option.content);
}

function isInlineCompletionTask(task: TaskGroupV2): boolean {
  return ["sentence_completion", "summary_completion", "note_completion", "form_completion"].includes(task.taskType);
}

function containsAnswerSlot(nodes: ContentNodeV2[] | undefined, slotIds?: Set<string>): boolean {
  if (!nodes?.length) return false;
  return nodes.some((node) => {
    if (node.type === "answer_slot") return !slotIds || slotIds.has(node.slotId);
    if ("children" in node) return containsAnswerSlot(node.children, slotIds);
    if ("items" in node) return node.items.some((item) => containsAnswerSlot(item.children, slotIds));
    if ("rows" in node) return node.rows.some((row) => row.cells.some((cell) => containsAnswerSlot(cell.children, slotIds)));
    if ("steps" in node) return node.steps.some((step) => containsAnswerSlot(step.children, slotIds));
    return false;
  });
}

/** 矩阵已经完整呈现该 task 时，不再重复渲染逐组列表。 */
function matrixHandled(
  task: TaskGroupV2,
  runtime: { questionDisplayMap: Record<string, string>; answerSlots: Record<string, { interaction?: string }> }
): boolean {
  if (!task.optionBank?.options.length) return false;
  return matchingRowsFor(
    task.responseGroups,
    runtime.questionDisplayMap,
    (slotId) => (runtime.answerSlots[slotId]?.interaction === "checkbox" ? "checkbox" : "radio")
  ).length > 0;
}

export function ExamCanvas(props: ExamCanvasProps) {
  const runtime = useMemo(() => buildRuntimeViewModelV2(props.authoring), [props.authoring]);
  const interactionModel = useMemo(() => buildReadingInteractionModelV2(runtime), [runtime]);
  const [studentAnswers, setStudentAnswers] = useState<Record<string, string[]>>({});
  const canvasAnswers = props.mode === "author"
    ? Object.fromEntries(Object.entries(props.authoring.answerKey).map(([slotId, value]) => [slotId, selectedValues(value)]))
    : studentAnswers;

  const setText = (slotId: string, value: string) => {
    if (props.mode === "author") props.onAnswerChange?.(slotId, { kind: "text", values: [value], normalization: "ielts_default" });
    else setStudentAnswers((current) => ({ ...current, [slotId]: value ? [value] : [] }));
  };
  const setOption = (slotId: string, label: string, checked: boolean, multiple: boolean) => {
    const current = canvasAnswers[slotId] ?? [];
    const next = multiple ? current.filter((value) => value !== label) : [];
    if (checked) next.push(label);
    if (props.mode === "author") props.onAnswerChange?.(slotId, { kind: "option", labels: next, assignment: "per_slot" });
    else setStudentAnswers((answers) => ({ ...answers, [slotId]: next }));
  };
  const optionsFor = (task: TaskGroupV2, response: ResponseGroupV2) => interactionModel.responseGroups[response.responseGroupId]?.options ?? task.optionBank?.options ?? [];

  return <div className={`exam-canvas-v2 ${props.mode === "author" ? "is-author" : "is-student"}`} data-testid={`exam-canvas-v2-${props.mode}`}>
    <main id="left" className="reading-pane passage-pane pane v2-passage-pane">
      <article className="reading-html passage-html v2-passage-content" aria-label={runtime.title}>
        <ContentNodes nodes={runtime.passage} canvas={props} />
      </article>
    </main>
    <section id="right" className="reading-pane question-pane pane v2-question-pane" aria-label="Reading questions">
      <div id="question-groups" className="question-groups v2-question-groups">
        {runtime.taskGroups.map((task) => <article key={task.taskId} className={`question-group unified-group v2-task-group${props.selectedId === task.taskId ? " is-selected" : ""}`} data-group-id={task.taskId} onClick={() => props.mode === "author" && props.onSelect?.(task.taskId)}>
          <header className="v2-task-header"><h2>{task.taskType}</h2><div className="v2-instruction"><ContentNodes nodes={task.instructions} canvas={props} /></div></header>
          {task.stimulus?.length ? <div className="v2-stimulus"><ContentNodes nodes={task.stimulus} canvas={props} /></div> : null}
          {(() => {
            // Matching 走矩阵版式：共享选项库 + 每行一个答案位（计划 §9.8）。
            // 不符合矩阵前提的 matching（多答案位、unordered_set）继续走下面的逐组列表。
            const bankOptions = (task as TaskGroupV2).optionBank?.options ?? [];
            if (!bankOptions.length) return null;
            const rows = matchingRowsFor(
              task.responseGroups as ResponseGroupV2[],
              runtime.questionDisplayMap,
              (slotId) => runtime.answerSlots[slotId]?.interaction === "checkbox" ? "checkbox" : "radio"
            );
            if (!rows.length) return null;
            return <MatchingMatrix
              rows={rows}
              options={bankOptions}
              answers={canvasAnswers}
              selectedId={props.selectedId}
              interactive
              renderContent={(nodes) => nodes?.length ? <ContentNodes nodes={nodes} canvas={props} /> : null}
              onSelectOption={setOption}
              onSelectTarget={props.mode === "author" ? props.onSelect : undefined}
            />;
          })()}
          {matrixHandled(task as TaskGroupV2, runtime) ? null : task.responseGroups.map((response) => {
            const options = optionsFor(task as TaskGroupV2, response);
            const unordered = response.assignment === "unordered_set";
            const responseSlotIds = new Set(response.slotIds);
            // Text completion is rendered as one canonical stimulus document
            // with inline slots. Once every response slot is present there,
            // the detached response list would duplicate the student view;
            // keep it only for an incomplete/blocked group so the author can
            // still locate missing slots.
            const inlineStimulusComplete = isInlineCompletionTask(task as TaskGroupV2)
              && response.slotIds.length > 0
              && containsAnswerSlot(task.stimulus, responseSlotIds)
              && response.slotIds.every((slotId) => containsAnswerSlot(task.stimulus, new Set([slotId])));
            return <section key={response.responseGroupId} className={`v2-response-group${props.selectedId === response.responseGroupId ? " is-selected" : ""}`} data-response-group-id={response.responseGroupId} data-assignment={response.assignment} onClick={(event) => { if (props.mode === "author") { event.stopPropagation(); props.onSelect?.(response.responseGroupId); } }}>
              {response.prompt?.length ? <div className="v2-response-prompt"><ContentNodes nodes={response.prompt} canvas={props} /></div> : null}
              {options.length || (props.mode === "author" && (response.kind === "choice" || response.kind === "matching")) ? <OptionBankTools canvas={props} taskId={task.taskId} responseGroupId={response.responseGroupId} options={options} /> : null}
              {inlineStimulusComplete ? null : unordered ? <fieldset className="v2-shared-selection"><legend>Select {response.cardinality.exact || response.slotIds.length} options for {response.slotIds.map((slotId) => runtime.questionDisplayMap[slotId]).join(", ")}</legend>{options.map((option) => { const checked = response.slotIds.some((slotId) => (canvasAnswers[slotId] ?? []).includes(option.label)); return <label key={option.optionId} className="v2-choice-item"><input type="checkbox" value={option.label} checked={checked} onChange={(event) => { const selected = Array.from(new Set(response.slotIds.flatMap((slotId) => canvasAnswers[slotId] ?? []).filter((value) => value !== option.label))); if (event.target.checked) selected.push(option.label); response.slotIds.forEach((slotId, index) => setOption(slotId, selected[index] ?? "", Boolean(selected[index]), false)); }} /><span><strong>{option.label}</strong> <ContentNodes nodes={option.content} canvas={props} /></span></label>; })}<div className="v2-slot-summary">{response.slotIds.map((slotId) => <span key={slotId} className="v2-slot-chip" data-question-id={slotId}>{runtime.questionDisplayMap[slotId]}: {(canvasAnswers[slotId] ?? []).join(", ") || "—"}</span>)}</div></fieldset> : <div className="v2-slot-list">{response.slotIds.map((slotId, index) => {
                const slot = runtime.answerSlots[slotId];
                if (!slot) return null;
                const values = canvasAnswers[slotId] ?? [];
                const textEntry = slot.interaction === "text" || response.kind === "text_entry";
                return <div key={slotId} className={`v2-slot-question${props.selectedId === slotId ? " is-selected" : ""}`} data-question-id={slotId} onClick={(event) => { if (props.mode === "author") { event.stopPropagation(); props.onSelect?.(slotId); } }}><div className="v2-slot-question-label"><span className="v2-slot-number">{runtime.questionDisplayMap[slotId]}</span>{response.kind === "text_entry" || response.kind === "matching" ? <span>Response {index + 1}</span> : null}</div>{textEntry ? <input className="v2-text-answer" type="text" name={slotId} value={values[0] ?? ""} maxLength={slot.constraints?.maxCharacters} aria-label={`Answer ${runtime.questionDisplayMap[slotId]}`} onChange={(event) => setText(slotId, event.target.value)} /> : options.length ? <div className="v2-choice-options">{options.map((option) => <label key={`${slotId}-${option.optionId}`} className="v2-choice-item"><input type={slot.interaction === "checkbox" ? "checkbox" : "radio"} name={slotId} value={option.label} checked={values.includes(option.label)} onChange={(event) => setOption(slotId, option.label, event.target.checked, slot.interaction === "checkbox")} /><span><strong>{option.label}</strong> <ContentNodes nodes={option.content} canvas={props} /></span></label>)}</div> : <input className="v2-text-answer" type="text" name={slotId} value={values[0] ?? ""} aria-label={`Answer ${runtime.questionDisplayMap[slotId]}`} onChange={(event) => setText(slotId, event.target.value)} />}</div>;
              })}</div>}
            </section>;
          })}
        </article>)}
      </div>
    </section>
  </div>;
}

/** 兼容期别名：`StructuredAuthoringEditorV2` 仍以旧名导入，P10 删除旧页面时一并移除。 */
export const ExamCanvasV2 = ExamCanvas;
