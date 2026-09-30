import { createContext, Fragment, useCallback, useContext, useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { InlineTextEditor } from "./editors/InlineTextEditor";
import { MatchingMatrix, matchingRowsFor } from "./renderers/MatchingMatrix";
import { resolveAuthoringAssetPreview, type AuthoringAssetPreview } from "../api/tauriCommands";
import { getListeningAudio, type ListeningAudioStatus } from "../api/listeningAudioClient";
import { buildReadingInteractionModelV2, buildRuntimeViewModelV2 } from "../services/runtimeViewModelV2";
import { taskTypeLabel } from "../utils/displayLabels";
import { ListeningHeader } from "./ListeningHeader";
import { isListening, listeningParts, listeningStructureMissing, visibleTaskIds } from "./listeningWorkspace";
import { QuestionNavBar } from "./QuestionNavBar";
import { buildQuestionNavModel } from "./questionNavModel";
import { isTfngOptionSet, optionContentDuplicatesLabel } from "./tfngOptions";
import { perSlotPrompts } from "./perSlotPrompts";
import { usePaneDivider } from "../features/editor/usePaneDivider";
import type { AnswerValueV2, ContentNodeV2, IeltsAuthoringIRV2, OptionV2, ResponseGroupV2, TaskGroupV2 } from "../types";

export type ExamCanvasStructureAction =
  | { type: "option.add"; taskId: string; responseGroupId: string; afterOptionId?: string }
  /** 把选项移到 `beforeOptionId` 之前；缺省表示移到末尾。用选项 id 而不是下标定位，
   *  因为画布上显示的选项列表和存储里的选项库不一定一一对应下标。 */
  | { type: "option.move"; taskId: string; responseGroupId: string; optionId: string; beforeOptionId?: string }
  | { type: "option.delete"; taskId: string; responseGroupId: string; optionId: string }
  | { type: "table.row.add"; tableId: string; afterRowId?: string }
  | { type: "table.row.delete"; tableId: string; rowId: string }
  | { type: "table.column.add"; tableId: string; afterColumnIndex?: number }
  | { type: "table.column.delete"; tableId: string; columnIndex: number }
  | { type: "answer-slot.insert"; afterNodeId: string }
  | { type: "answer-slot.delete"; nodeId: string; slotId: string }
  | { type: "answer-slot.host.set"; slotId: string; hostNodeId: string };

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
  /** 只读锁（云端校核进行中）：对画布根节点加 inert，挡住文本/答案/结构/拖动等一切交互。 */
  locked?: boolean;
  /** Embedded recognition alternatives show only questions, without global navigation IDs. */
  comparisonPreview?: boolean;
  /** Author review controls anchored beside a complete question group. */
  taskAdornment?: (taskId: string) => ReactNode;
  unanchoredTaskAdornment?: ReactNode;
}

type VisualNodeV2 = Extract<ContentNodeV2, { type: "figure" | "image" | "diagram" }>;

/** author 模式读取 answerKey 投影，student 预览读取本地作答状态；
 *  两种模式用同一份 setter 语义，保证预览交互与学生端一致。 */
const CanvasAnswersContext = createContext<{
  answers: Record<string, string[]>;
  setText: (slotId: string, value: string) => void;
  setOption: (slotId: string, label: string, checked: boolean, multiple: boolean, assignment?: "per_slot" | "unordered_set") => void;
}>({ answers: {}, setText: () => {}, setOption: () => {} });

interface AnswerDragSession {
  taskId: string;
  responseGroupId: string;
  label: string;
  sourceSlotId?: string;
  source: HTMLElement;
  target: HTMLElement | null;
  startX?: number;
  startY?: number;
}

interface AnswerDragController {
  begin: (init: Omit<AnswerDragSession, "target">) => void;
}

const AnswerDragContext = createContext<AnswerDragController | null>(null);

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

/** 作者模式的选项增删：不再用悬浮工具条，而是贴在选项本身上、悬停才出现——
 *  删除是选项行末尾的 ×，添加是选项列表下方的一行轻量入口。排序见 {@link OptionDragHandle}。 */
function OptionDeleteButton({ canvas, taskId, responseGroupId, option, count }: {
  canvas: ExamCanvasProps;
  taskId: string;
  responseGroupId: string;
  option: OptionV2;
  count: number;
}) {
  if (canvas.mode !== "author" || !canvas.onStructureAction || count <= 1) return null;
  return <button
    type="button"
    className="v2-option-delete"
    title="删除这个选项"
    aria-label={`删除选项 ${option.label}`}
    // 按钮在 <label> 里：阻止点击冒泡成“选中这个选项”。
    onClick={(event) => {
      event.preventDefault();
      event.stopPropagation();
      canvas.onStructureAction?.({ type: "option.delete", taskId, responseGroupId, optionId: option.optionId });
    }}
  >×</button>;
}

function OptionAddButton({ canvas, taskId, responseGroupId, options }: {
  canvas: ExamCanvasProps;
  taskId: string;
  responseGroupId: string;
  options: OptionV2[];
}) {
  if (canvas.mode !== "author" || !canvas.onStructureAction) return null;
  return <button
    type="button"
    className={`v2-option-add${options.length ? "" : " is-empty"}`}
    aria-label="添加选项"
    onClick={(event) => {
      event.stopPropagation();
      canvas.onStructureAction?.({ type: "option.add", taskId, responseGroupId, afterOptionId: options.at(-1)?.optionId });
    }}
  >＋ 添加选项</button>;
}

const dropClasses = ["is-drop-before", "is-drop-after"];

/** 一次进行中的选项拖动。只存 id 与最近一次接触到的 DOM：识别中的后台草稿刷新会
 *  重拉草稿、重渲染画布（行甚至可能换节点），会话必须越过这次刷新活着，
 *  松手时再按**最新**的 DOM 与回调决定提交——不再在旧闭包里做任何判断。 */
interface OptionDragSession {
  taskId: string;
  responseGroupId: string;
  optionId: string;
  /** null = 指针还没移动过；undefined = 放到末尾。 */
  beforeOptionId?: string | null;
  row: HTMLElement | null;
  list: HTMLElement | null;
  /** 按下时的指针坐标：松手时判断“是否真的拖动过”。 */
  startX?: number;
  startY?: number;
}

/** 拖动会话由 ExamCanvas 持有并通过 context 下发：手柄中途被刷新卸载不再取消会话
 *  （那正是验收缺陷的静默丢弃点），只有 ExamCanvas 本身卸载（离开工作区/切预览）才取消。 */
interface OptionDragController {
  begin: (init: { taskId: string; responseGroupId: string; optionId: string; row: HTMLElement; list: HTMLElement; startX?: number; startY?: number }) => void;
}

const OptionDragContext = createContext<OptionDragController | null>(null);

/** 作者模式下选项行左侧的拖动手柄。选项顺序只通过拖动（或聚焦手柄后按 ↑/↓）调整，
 *  不再在工具条里给每个选项放上移/下移按钮。
 *
 *  用 pointer 事件而不是 HTML5 拖放：Tauri 在 Windows 上默认接管 WebView 的拖放
 *  （用于把文件拖进窗口），HTML5 `draggable` 在桌面端会失效。
 *  移动/松开挂在 window 上而不依赖 pointer capture：捕获会因视口变化、失焦等原因
 *  中途丢失，那时拖动会被静默取消（真实 WebView2 里复现过）。
 *  会话本身挂在 ExamCanvas 上（见 {@link OptionDragContext}）：识别进行中的后台草稿刷新
 *  会重渲染画布、可能换掉行节点甚至卸载本手柄——那不取消拖动，松手时按最新草稿提交。
 *  行需要带 `data-option-row` / `data-option-id`，并且是 `data-option-list` 容器的直接子元素。 */
function OptionDragHandle({ canvas, taskId, responseGroupId, options, index }: {
  canvas: ExamCanvasProps;
  taskId: string;
  responseGroupId: string;
  options: OptionV2[];
  index: number;
}) {
  const beginDrag = useContext(OptionDragContext);
  if (canvas.mode !== "author" || !canvas.onStructureAction || !beginDrag) return null;
  const option = options[index];
  const move = (beforeOptionId: string | undefined) => canvas.onStructureAction?.({ type: "option.move", taskId, responseGroupId, optionId: option.optionId, beforeOptionId });
  return <span
    className="v2-option-drag-handle"
    role="button"
    tabIndex={0}
    title="拖动调整选项顺序"
    aria-label={`拖动调整选项 ${option.label} 的顺序（聚焦后也可按上下方向键）`}
    // 手柄在 <label> 里：阻止点击冒泡成“选中这个选项”。
    onClick={(event) => { event.preventDefault(); event.stopPropagation(); }}
    onPointerDown={(event) => {
      if (event.button > 0) return;
      const row = event.currentTarget.closest<HTMLElement>("[data-option-row]");
      const list = row?.parentElement?.closest<HTMLElement>("[data-option-list]");
      if (!row || !list) return;
      event.preventDefault();
      event.stopPropagation();
      beginDrag.begin({ taskId, responseGroupId, optionId: option.optionId, row, list, startX: event.clientX, startY: event.clientY });
    }}
    onKeyDown={(event) => {
      if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
      event.preventDefault();
      event.stopPropagation();
      if (event.key === "ArrowUp" && index > 0) move(options[index - 1].optionId);
      if (event.key === "ArrowDown" && index < options.length - 1) move(options[index + 2]?.optionId);
    }}
  >⋮⋮</span>;
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
  const { answers, setOption } = useContext(CanvasAnswersContext);
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
        // 学生端（FigureNode.vue）点击热点把 hotspotId 写入该题答案；作者模式点击仅选中。
        const pressed = (answers[hotspot.slotId] ?? []).includes(hotspot.hotspotId);
        return <button
          key={hotspot.hotspotId}
          type="button"
          className={`v2-canvas-hotspot${pressed ? " is-pressed" : ""}`}
          aria-pressed={canvas.mode === "student" ? pressed : undefined}
          style={{ left: `${left * 100}%`, top: `${top * 100}%`, width: `${hotspotWidth * 100}%`, height: `${hotspotHeight * 100}%` }}
          onClick={(event) => {
            event.stopPropagation();
            if (canvas.mode === "author") canvas.onSelect?.(hotspot.slotId);
            else setOption(hotspot.slotId, hotspot.hotspotId, true, false);
          }}
        >{canvas.authoring.answerSlots[hotspot.slotId]?.displayLabel ?? hotspot.slotId}</button>;
      }) : null}
    </div>
    {node.type === "figure" && node.caption?.length ? <figcaption><ContentNodes nodes={node.caption} canvas={canvas} /></figcaption> : null}
  </figure>;
}

function ContentNodes({ nodes, canvas }: { nodes: ContentNodeV2[] | undefined; canvas: ExamCanvasProps }): ReactNode {
  const canvasState = useContext(CanvasAnswersContext);
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
      case "paragraph": {
        const targetSlots = Object.values(canvas.authoring.answerSlots).filter((slot) =>
          slot.interaction === "dragdrop" && slot.hostType === "passage_paragraph" && slot.hostNodeId === node.id,
        );
        const label = node.paragraphLabel ?? paragraphLabelFor(canvas.authoring, node.id);
        return <Fragment key={node.id}>
          {targetSlots.map((slot) => <AnswerDropTarget key={slot.slotId} canvas={canvas} slotId={slot.slotId} placement="passage" paragraphLabel={label} />)}
          <p data-editor-id={node.id} data-paragraph-label={node.paragraphLabel} className={`v2-paragraph${authorClass}${selectedClass}`} style={nodeStyle(node)} onClick={select}><ContentNodes nodes={node.children} canvas={canvas} /></p>
        </Fragment>;
      }
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
        const values = canvasState.answers[node.slotId] ?? [];
        const removeTools = <AuthorTools canvas={canvas} label={`编辑答案位 ${slot.displayLabel}`} compact>
          <ToolButton label={`在此答案位后插入答案位`} onClick={() => canvas.onStructureAction?.({ type: "answer-slot.insert", afterNodeId: node.id })}>＋</ToolButton>
          <ToolButton label={`删除答案位 ${slot.displayLabel}`} onClick={() => canvas.onStructureAction?.({ type: "answer-slot.delete", nodeId: node.id, slotId: slot.slotId })}>×</ToolButton>
        </AuthorTools>;
        const withTools = (control: ReactNode) => canvas.mode === "author" && canvas.onStructureAction
          ? <span key={node.id} className="v2-answer-slot-frame">{control}{removeTools}</span>
          : control;
        if (slot.interaction === "text") {
          // 学生预览也要能真实输入（与学生 AnswerSlotNode 行为一致），不再是无回显的 uncontrolled 输入框。
          return withTools(<label key={node.id} data-editor-id={node.id} className={`v2-answer-slot v2-answer-slot-text${authorClass}${selectedClass}`} onClick={select}><span className="v2-slot-label">{slot.displayLabel}</span><input type="text" name={slot.slotId} value={values[0] ?? ""} placeholder={node.placeholder || "Answer"} onChange={(event) => canvasState.setText(slot.slotId, event.target.value)} /></label>);
        }
        if (slot.interaction === "select") {
          const binding = slotPresentationFor(canvas.authoring, slot.slotId);
          if (!binding) return null;
          return withTools(<label key={node.id} data-editor-id={node.id} className={`v2-answer-slot v2-answer-slot-select${authorClass}${selectedClass}`} onClick={select}>
            <span className="v2-slot-label">{slot.displayLabel}</span>
            <select aria-label={`Answer ${slot.displayLabel}`} value={values[0] ?? ""} onChange={(event) => canvasState.setOption(slot.slotId, event.target.value, true, false)}>
              <option value="">Select…</option>
              {binding.options.map((option) => <option key={option.optionId} value={option.label}>{option.label}</option>)}
            </select>
          </label>);
        }
        if (slot.interaction === "dragdrop") return withTools(<AnswerDropTarget key={node.id} canvas={canvas} slotId={slot.slotId} placement="inline" />);
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

function slotPresentationFor(authoring: IeltsAuthoringIRV2, slotId: string) {
  for (const task of authoring.taskGroups) {
    for (const response of task.responseGroups) {
      if (!response.slotIds.includes(slotId)) continue;
      return { task, response, options: response.options?.length ? response.options : task.optionBank?.options ?? [] };
    }
  }
  return undefined;
}

function paragraphLabelFor(authoring: IeltsAuthoringIRV2, nodeId: string): string | undefined {
  return Object.entries(authoring.passage?.paragraphMap ?? {}).find(([, targetId]) => targetId === nodeId)?.[0]
    ?? authoring.passage?.content
      .flatMap((node) => node.type === "paragraph" && node.id === nodeId ? [node.paragraphLabel] : [])
      .find((label): label is string => Boolean(label));
}

function AnswerDropTarget({
  canvas,
  slotId,
  placement,
  paragraphLabel
}: {
  canvas: ExamCanvasProps;
  slotId: string;
  placement: "passage" | "row" | "inline";
  paragraphLabel?: string;
}) {
  const answers = useContext(CanvasAnswersContext);
  const drag = useContext(AnswerDragContext);
  const slot = canvas.authoring.answerSlots[slotId];
  const binding = slotPresentationFor(canvas.authoring, slotId);
  if (!slot || !binding) return null;
  const value = answers.answers[slotId]?.[0] ?? "";
  const selectedOption = binding.options.find((option) => option.label === value);
  const targetLabel = paragraphLabel ? `Paragraph ${paragraphLabel}` : `Response ${slot.displayLabel}`;
  const emptyLabel = /heading/iu.test(binding.task.taskType) ? "Drop heading here" : "Drop option here";
  const start = (event: React.PointerEvent<HTMLElement>) => {
    if (!value || event.button > 0) return;
    event.preventDefault();
    event.stopPropagation();
    drag?.begin({
      taskId: binding.task.taskId,
      responseGroupId: binding.response.responseGroupId,
      label: value,
      sourceSlotId: slotId,
      source: event.currentTarget,
      startX: event.clientX,
      startY: event.clientY
    });
  };
  const paragraphOptions = Object.entries(canvas.authoring.passage?.paragraphMap ?? {});
  return <span className={`v2-answer-dropzone-wrap is-${placement}`}>
    <button
      type="button"
      className={`v2-answer-dropzone${value ? " is-filled" : " is-empty"}`}
      data-answer-drop-slot={slotId}
      data-task-id={binding.task.taskId}
      data-response-group-id={binding.response.responseGroupId}
      aria-label={placement === "passage" ? `${targetLabel} (${slot.displayLabel})` : `Answer ${slot.displayLabel}`}
      aria-pressed={Boolean(value)}
      onPointerDown={start}
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        if (value) answers.setOption(slotId, value, false, false, "per_slot");
        if (canvas.mode === "author") canvas.onSelect?.(slotId);
      }}
    >
      {placement === "passage" ? <span className="v2-answer-dropzone-label">{targetLabel} ({slot.displayLabel})</span> : <span className="v2-answer-dropzone-label">{slot.displayLabel}</span>}
      <span className="v2-answer-dropzone-value">{(selectedOption?.label ?? value) || emptyLabel}</span>
      {selectedOption?.content.length ? <span className="v2-answer-dropzone-content"><ContentNodes nodes={selectedOption.content} canvas={canvas} /></span> : null}
    </button>
    {canvas.mode === "author" && canvas.onStructureAction && slot.hostType === "passage_paragraph" && paragraphOptions.length ? <label className="v2-answer-target-editor">
      <span>Target</span>
      <select
        aria-label={`Paragraph target for ${slot.displayLabel}`}
        value={slot.hostNodeId ?? ""}
        onChange={(event) => canvas.onStructureAction?.({ type: "answer-slot.host.set", slotId, hostNodeId: event.target.value })}
      >
        {paragraphOptions.map(([label, nodeId]) => <option key={nodeId} value={nodeId}>Paragraph {label}</option>)}
      </select>
    </label> : null}
  </span>;
}

function AnswerOptionPool({
  canvas,
  task,
  response,
  options
}: {
  canvas: ExamCanvasProps;
  task: TaskGroupV2;
  response: ResponseGroupV2;
  options: OptionV2[];
}) {
  const answers = useContext(CanvasAnswersContext);
  const drag = useContext(AnswerDragContext);
  const used = new Set(response.slotIds.flatMap((slotId) => answers.answers[slotId] ?? []));
  return <section className="v2-answer-option-pool" data-option-pool-task={task.taskId} data-answer-drop-pool={response.responseGroupId} data-task-id={task.taskId} data-response-group-id={response.responseGroupId}>
    <h3>Options</h3>
    <div className="v2-answer-option-list" data-option-list={canvas.mode === "author" ? "" : undefined}>
      {options.map((option, index) => {
        const disabled = !response.allowOptionReuse && used.has(option.label);
        return <div key={option.optionId} className={`v2-answer-option${disabled ? " is-consumed" : ""}`} {...(canvas.mode === "author" && canvas.onStructureAction ? { "data-option-row": "", "data-option-id": option.optionId } : {})}>
          <OptionDragHandle canvas={canvas} taskId={task.taskId} responseGroupId={response.responseGroupId} options={options} index={index} />
          <button
            type="button"
            className="v2-answer-option-token"
            data-option-label={option.label}
            aria-label={`Drag option ${option.label}`}
            disabled={disabled}
            onPointerDown={(event) => {
              if (event.button > 0 || disabled) return;
              event.preventDefault();
              event.stopPropagation();
              drag?.begin({
                taskId: task.taskId,
                responseGroupId: response.responseGroupId,
                label: option.label,
                source: event.currentTarget,
                startX: event.clientX,
                startY: event.clientY
              });
            }}
          >
            <strong>{option.label}</strong>{optionContentDuplicatesLabel(option) ? null : <ContentNodes nodes={option.content} canvas={canvas} />}
          </button>
          <OptionDeleteButton canvas={canvas} taskId={task.taskId} responseGroupId={response.responseGroupId} option={option} count={options.length} />
        </div>;
      })}
    </div>
  </section>;
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
  const setOption = (slotId: string, label: string, checked: boolean, multiple: boolean, assignment: "per_slot" | "unordered_set" = "per_slot") => {
    const current = canvasAnswers[slotId] ?? [];
    const next = multiple ? current.filter((value) => value !== label) : [];
    if (checked) next.push(label);
    if (props.mode === "author") props.onAnswerChange?.(slotId, { kind: "option", labels: next, assignment });
    else setStudentAnswers((answers) => ({ ...answers, [slotId]: next }));
  };
  // 听力：没有 passage 栏；头部只有音频行（Part 切换交给底部题号导航）。
  // Part 映射到题组时只显示该 Part 的题组；底部导航与这里共用同一个 selectedPart state。
  const listening = isListening(props.authoring);
  const [selectedPart, setSelectedPart] = useState<number>(1);
  // 音频绑定状态上提到画布：listeningPartViews 是底部导航（Part section 的音频标记）
  // 与 ListeningHeader（音频行）**同一份**数据源，两边不允许各拉各的。
  const [audioStatus, setAudioStatus] = useState<ListeningAudioStatus>();
  const jobId = props.authoring.jobId;
  // 依赖**只用** jobId + listening（听力/阅读身份布尔，每次渲染算一次），
  // 不用 props.authoring：每次编辑（set_text、set_answer…）都会生成新的 authoring
  // 对象引用，它一旦进依赖，重渲染就会换掉 reloadAudio 的身份、重触发拉取。
  // 换句话说：对听力稿执行 set_text 之后 getListeningAudio **不会**被再次调用。
  const reloadAudio = useCallback((verify: boolean) => {
    // 阅读稿没有音频绑定，不发这次 IPC（此前只有听力头部会拉，现在拉取上提了）。
    if (!listening) return;
    getListeningAudio(jobId, verify).then(setAudioStatus).catch(() => setAudioStatus(undefined));
  }, [jobId, listening]);
  // 校验拉取（verify=true）只在挂载 / jobId / 听力身份变化时发生一次。
  // cancelled 标记丢弃过期响应：依赖在响应回来前又变了，就不再写 state，
  // 避免旧题目的音频状态覆盖新题目。手动刷新（onAudioChanged）走 reloadAudio(false)。
  useEffect(() => {
    if (!listening) return;
    let cancelled = false;
    getListeningAudio(jobId, true)
      .then((status) => { if (!cancelled) setAudioStatus(status); })
      .catch(() => { if (!cancelled) setAudioStatus(undefined); });
    return () => { cancelled = true; };
  }, [jobId, listening]);
  const listeningPartViews = useMemo(
    () => (listening ? listeningParts(props.authoring, audioStatus?.bindings ?? []) : undefined),
    [listening, props.authoring, audioStatus]
  );
  const shownTaskIds = listening
    ? new Set(visibleTaskIds(runtime.taskGroups.map((task) => task.taskId), listeningPartViews ?? [], selectedPart))
    : undefined;
  // 底部题号导航的数据（author 看 answerKey，student 看预览本地作答；都不写回题稿）。
  const navModel = useMemo(() => buildQuestionNavModel({
    mode: props.mode,
    taskGroups: runtime.taskGroups,
    questionDisplayMap: runtime.questionDisplayMap,
    answerSlots: runtime.answerSlots,
    answerKey: props.authoring.answerKey,
    studentAnswers,
    listeningParts: listeningPartViews,
    selectedPart: listening ? selectedPart : undefined
  }), [props.mode, runtime, props.authoring.answerKey, studentAnswers, listeningPartViews, listening, selectedPart]);
  // 原文 | 题目 的可拖动分隔条（workspace.css 负责视觉，本组件只渲染元素）。
  const { dividerProps } = usePaneDivider(props.authoring.jobId);
  // ── 选项拖动会话（见 OptionDragSession）──
  // 会话挂在这里而不是每个手柄上：识别进行中的后台草稿刷新会重渲染画布、
  // 可能换掉行节点甚至卸载旧手柄——旧实现里那次卸载会取消会话，把一次已完成的
  // 拖动手势静默丢掉。现在只有 ExamCanvas 卸载（离开工作区/切学生预览）才取消；
  // 提交发生在 window 事件里，永远读**最新** props，不能用挂监听那一刻的闭包。
  const canvasPropsRef = useRef(props);
  canvasPropsRef.current = props;
  const dragRef = useRef<OptionDragSession | null>(null);
  const optionDrag = useMemo<OptionDragController>(() => ({
    begin: (init) => {
      if (dragRef.current) return;
      dragRef.current = { ...init, beforeOptionId: null };
      init.row.classList.add("is-dragging");
      init.list.classList.add("is-reordering");
    }
  }), []);
  useEffect(() => {
    const rowsOf = (list: HTMLElement) => Array.from(list.querySelectorAll<HTMLElement>(":scope > [data-option-row]"));
    const clearMarks = (list: HTMLElement | null) => {
      if (!list) return;
      rowsOf(list).forEach((row) => row.classList.remove(...dropClasses));
    };
    // 后台刷新可能重建了选项行：按 id 在最新 DOM 里重新定位被拖行与它的列表，
    // 找不到（选项已被删除）就保留最后的落点，松手时交给上层响亮失败。
    const resolveLive = (session: OptionDragSession) => {
      if (session.row?.isConnected && session.list?.isConnected && session.list.contains(session.row)) return;
      const section = Array.from(document.querySelectorAll<HTMLElement>("[data-response-group-id]"))
        .find((candidate) => candidate.dataset.responseGroupId === session.responseGroupId);
      const row = section
        ? Array.from(section.querySelectorAll<HTMLElement>("[data-option-row]"))
            .find((candidate) => candidate.dataset.optionId === session.optionId) ?? null
        : null;
      session.row = row;
      session.list = row?.parentElement?.closest<HTMLElement>("[data-option-list]") ?? null;
      if (session.row) {
        // 重建后的行不带拖动中的视觉状态，补上，避免提示线突然消失。
        session.row.classList.add("is-dragging");
        session.list?.classList.add("is-reordering");
      }
    };
    const track = (session: OptionDragSession, clientX: number, clientY: number) => {
      const list = session.list;
      if (!list) return;
      const rows = rowsOf(list);
      const boxes = rows.map((row) => row.getBoundingClientRect());
      // TFNG 短标签选项横排（可换行）：同一行内按水平中线判断，跨行按上下判断；
      // 竖排选项只看垂直中线。
      const horizontal = boxes.length > 1 && boxes[1].top < boxes[0].bottom && boxes[1].left > boxes[0].left;
      const target = boxes.findIndex((box) => horizontal
        ? clientY < box.top || (clientY < box.bottom && clientX < box.left + box.width / 2)
        : clientY < box.top + box.height / 2);
      clearMarks(list);
      if (target >= 0) {
        rows[target].classList.add("is-drop-before");
        session.beforeOptionId = rows[target].dataset.optionId;
      } else {
        rows.at(-1)?.classList.add("is-drop-after");
        session.beforeOptionId = undefined;
      }
    };
    const movedEnough = (session: OptionDragSession, release: { x: number; y: number }) => {
      const { startX, startY } = session;
      if (![startX, startY, release.x, release.y].every((value) => Number.isFinite(value))) return false;
      return Math.hypot(release.x - startX!, release.y - startY!) >= 4;
    };
    const detach = () => {
      const session = dragRef.current;
      if (!session) return;
      dragRef.current = null;
      clearMarks(session.list);
      session.row?.classList.remove("is-dragging");
      session.list?.classList.remove("is-reordering");
    };
    const finish = (commit: boolean, release?: { x: number; y: number }) => {
      const session = dragRef.current;
      if (!session) return;
      // 整个手势里一次落点都没算出来（按下瞬间恰好赶上后台刷新、没收到 pointermove），
      // 但指针确实移动过：用松手坐标补算一次落点，不能把这次拖动静默丢掉。
      // 原地按下松开（移动不超过几个像素）仍然不算拖动。
      if (commit && session.beforeOptionId === null && release && movedEnough(session, release)) {
        resolveLive(session);
        track(session, release.x, release.y);
      }
      detach();
      if (!commit || session.beforeOptionId === null) return;
      resolveLive(session);
      // 落在自己前后等于没动（用**最新** DOM 的行序判断；行已不在时跳过判断，
      // 动作照常上报，由工作区对最新草稿校验并响亮失败）。
      const rows = session.list ? rowsOf(session.list) : [];
      const ownIndex = rows.findIndex((row) => row.dataset.optionId === session.optionId);
      const beforeIndex = session.beforeOptionId === undefined
        ? rows.length
        : rows.findIndex((row) => row.dataset.optionId === session.beforeOptionId);
      if (ownIndex >= 0 && (beforeIndex === ownIndex || beforeIndex === ownIndex + 1)) return;
      canvasPropsRef.current.onStructureAction?.({
        type: "option.move", taskId: session.taskId, responseGroupId: session.responseGroupId,
        optionId: session.optionId, beforeOptionId: session.beforeOptionId ?? undefined
      });
    };
    const onMove = (moveEvent: PointerEvent) => {
      const session = dragRef.current;
      if (!session) return;
      resolveLive(session);
      track(session, moveEvent.clientX, moveEvent.clientY);
    };
    const onUp = (upEvent: PointerEvent) => finish(true, { x: upEvent.clientX, y: upEvent.clientY });
    const onCancel = () => finish(false);
    const onKey = (keyEvent: KeyboardEvent) => { if (keyEvent.key === "Escape") finish(false); };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onCancel);
    window.addEventListener("blur", onCancel);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onCancel);
      window.removeEventListener("blur", onCancel);
      window.removeEventListener("keydown", onKey);
      // 真正卸载（离开工作区/切学生预览）才取消会话：不提交。
      dragRef.current = null;
    };
  }, []);
  const answerValuesRef = useRef(canvasAnswers);
  answerValuesRef.current = canvasAnswers;
  const answerDragRef = useRef<AnswerDragSession | null>(null);
  const answerDrag = useMemo<AnswerDragController>(() => ({
    begin: (init) => {
      const current = canvasPropsRef.current;
      if (current.locked || answerDragRef.current) return;
      answerDragRef.current = { ...init, target: null };
      init.source.classList.add("is-answer-dragging");
    }
  }), []);
  useEffect(() => {
    const targetSelector = "[data-answer-drop-slot], [data-answer-drop-pool]";
    const targetFrom = (event: PointerEvent) => {
      const eventTarget = event.target instanceof Element ? event.target.closest<HTMLElement>(targetSelector) : null;
      if (eventTarget) return eventTarget;
      return typeof document.elementFromPoint === "function"
        ? document.elementFromPoint(event.clientX, event.clientY)?.closest<HTMLElement>(targetSelector) ?? null
        : null;
    };
    const clearTarget = (target: HTMLElement | null) => target?.classList.remove("is-drop-hover");
    const detach = () => {
      const session = answerDragRef.current;
      if (!session) return;
      answerDragRef.current = null;
      clearTarget(session.target);
      session.source.classList.remove("is-answer-dragging");
    };
    const onMove = (event: PointerEvent) => {
      const session = answerDragRef.current;
      if (!session) return;
      const next = targetFrom(event);
      const matches = next?.dataset.taskId === session.taskId
        && next?.dataset.responseGroupId === session.responseGroupId;
      const resolved = matches ? next : null;
      if (session.target === resolved) return;
      clearTarget(session.target);
      session.target = resolved;
      session.target?.classList.add("is-drop-hover");
    };
    const onUp = (event: PointerEvent) => {
      const session = answerDragRef.current;
      if (!session) return;
      const moved = Number.isFinite(session.startX) && Number.isFinite(session.startY)
        && Math.hypot(event.clientX - session.startX!, event.clientY - session.startY!) >= 4;
      const releaseTarget = targetFrom(event);
      const target = session.target ?? (moved ? releaseTarget : null);
      const validTarget = target?.dataset.taskId === session.taskId
        && target?.dataset.responseGroupId === session.responseGroupId
        ? target
        : null;
      detach();
      if (!moved || !validTarget) return;

      const currentProps = canvasPropsRef.current;
      const binding = slotPresentationFor(currentProps.authoring, session.sourceSlotId ?? "")
        ?? currentProps.authoring.taskGroups
          .flatMap((task) => task.responseGroups.map((response) => ({ task, response, options: response.options?.length ? response.options : task.optionBank?.options ?? [] })))
          .find(({ task, response }) => task.taskId === session.taskId && response.responseGroupId === session.responseGroupId);
      if (!binding) return;
      const assignment = binding.response.assignment === "unordered_set" ? "unordered_set" : "per_slot";
      const writeAnswer = (slotId: string, labels: string[]) => {
        answerValuesRef.current = { ...answerValuesRef.current, [slotId]: labels };
        if (currentProps.mode === "author") {
          currentProps.onAnswerChange?.(slotId, { kind: "option", labels, assignment });
        } else {
          setStudentAnswers((current) => ({ ...current, [slotId]: labels }));
        }
      };

      const targetSlotId = validTarget.dataset.answerDropSlot;
      if (targetSlotId) {
        if (session.sourceSlotId) {
          if (session.sourceSlotId === targetSlotId) return;
          const sourceValue = answerValuesRef.current[session.sourceSlotId] ?? [];
          const targetValue = answerValuesRef.current[targetSlotId] ?? [];
          const moving = sourceValue[0] ?? session.label;
          writeAnswer(session.sourceSlotId, targetValue.length ? [targetValue[0]] : []);
          writeAnswer(targetSlotId, [moving]);
          return;
        }
        if (!binding.response.allowOptionReuse) {
          const usedElsewhere = binding.response.slotIds.some((slotId) =>
            slotId !== targetSlotId && (answerValuesRef.current[slotId] ?? []).includes(session.label),
          );
          if (usedElsewhere) return;
        }
        writeAnswer(targetSlotId, [session.label]);
        return;
      }
      if (validTarget.dataset.answerDropPool && session.sourceSlotId) writeAnswer(session.sourceSlotId, []);
    };
    const onCancel = () => detach();
    const onKey = (event: KeyboardEvent) => { if (event.key === "Escape") detach(); };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onCancel);
    window.addEventListener("blur", onCancel);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onCancel);
      window.removeEventListener("blur", onCancel);
      window.removeEventListener("keydown", onKey);
      answerDragRef.current = null;
    };
  }, []);
  // 只在作者模式挂拖动排序需要的定位属性，学生预览的 DOM 保持不变。
  const optionRowProps = (option: OptionV2) => props.mode === "author" && props.onStructureAction
    ? { "data-option-row": "", "data-option-id": option.optionId }
    : {};
  const optionsFor = (task: TaskGroupV2, response: ResponseGroupV2) => interactionModel.responseGroups[response.responseGroupId]?.options ?? task.optionBank?.options ?? [];

  return <AnswerDragContext.Provider value={answerDrag}>
  <CanvasAnswersContext.Provider value={{ answers: canvasAnswers, setText, setOption }}>
    <OptionDragContext.Provider value={optionDrag}>
    <div className={`exam-canvas-v2 ${props.mode === "author" ? "is-author" : "is-student"}${listening ? " is-listening" : ""}`} data-testid={`exam-canvas-v2-${props.mode}`} inert={props.locked || undefined}>
    {!props.comparisonPreview && (listening ? (
      <ListeningHeader
        itemId={props.authoring.jobId}
        mode={props.mode}
        selectedPart={selectedPart}
        parts={listeningPartViews ?? []}
        onAudioChanged={() => reloadAudio(false)}
      />
    ) : (
      <>
        <main id="left" className="reading-pane passage-pane pane v2-passage-pane">
          <article className="reading-html passage-html v2-passage-content" aria-label={runtime.title}>
            <ContentNodes nodes={runtime.passage} canvas={props} />
          </article>
        </main>
        <div id="divider" {...dividerProps} />
      </>
    ))}
    <section id={props.comparisonPreview ? undefined : "right"} className="reading-pane question-pane pane v2-question-pane" aria-label={listening ? "Listening questions" : "Reading questions"}>
      <div id={props.comparisonPreview ? undefined : "question-groups"} className="question-groups v2-question-groups">
        {props.unanchoredTaskAdornment}
        {listening && listeningStructureMissing(props.authoring) ? (
          <p className="empty listening-structure-missing" data-testid="listening-structure-missing">听力结构尚未识别</p>
        ) : null}
        {runtime.taskGroups.filter((task) => !shownTaskIds || shownTaskIds.has(task.taskId)).map((task) => <article key={task.taskId} className={`question-group unified-group v2-task-group${props.selectedId === task.taskId ? " is-selected" : ""}`} data-group-id={task.taskId} data-editor-id={task.taskId} onClick={() => props.mode === "author" && props.onSelect?.(task.taskId)}>
          {props.taskAdornment?.(task.taskId)}
          <header className="v2-task-header"><h2>{taskTypeLabel(task.taskType)}</h2><div className="v2-instruction"><ContentNodes nodes={task.instructions} canvas={props} /></div></header>
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
          {task.responseGroups.map((response) => {
            const options = optionsFor(task as TaskGroupV2, response);
            const usesDragDrop = response.slotIds.some((slotId) => runtime.answerSlots[slotId]?.interaction === "dragdrop");
            return usesDragDrop && options.length
              ? <AnswerOptionPool key={`pool-${response.responseGroupId}`} canvas={props} task={task as TaskGroupV2} response={response} options={options} />
              : null;
          })}
          {matrixHandled(task as TaskGroupV2, runtime) ? null : task.responseGroups.map((response) => {
            const options = optionsFor(task as TaskGroupV2, response);
            const unordered = response.assignment === "unordered_set";
            // 学生端 ReadingExamV2Renderer 用 `cardinality.max` 作为共享选择的禁用阈值
            // （legend 才用 `exact`）。这里必须一致，否则作者预览比学生端更早锁住选项，
            // 作者会以为某些组合不可选。
            const unorderedLimit = unordered
              ? response.cardinality.max ?? Number.POSITIVE_INFINITY
              : Number.POSITIVE_INFINITY;
            const unorderedSelected = new Set(response.slotIds.flatMap((slotId) => canvasAnswers[slotId] ?? [])).size;
            const responseSlotIds = new Set(response.slotIds);
            // 判断题（TFNG/YNNG）逐题题干：response.prompt 的顶层段落能被每个 slot 的
            // hostNodeId 一一认领时（全有或全无），该题陈述渲染到题号后面，顶部只留公共段。
            // 缺宿主 / 宿主内嵌 answer_slot / unordered_set 共享多选一律不启用，保持旧版式。
            const perSlot = perSlotPrompts(response, runtime.answerSlots);
            const sharedPromptNodes = perSlot
              ? (response.prompt ?? []).filter((node) => !perSlot.claimedNodeIds.has(node.id))
              : response.prompt;
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
              {sharedPromptNodes?.length ? <div className="v2-response-prompt"><ContentNodes nodes={sharedPromptNodes} canvas={props} /></div> : null}
              {inlineStimulusComplete ? null : unordered ? <fieldset className="v2-shared-selection" data-option-list={props.mode === "author" ? "" : undefined}><legend>Select {response.cardinality.exact || response.slotIds.length} options for {response.slotIds.map((slotId) => runtime.questionDisplayMap[slotId]).join(", ")}</legend>{options.map((option, optionIndex) => { const checked = response.slotIds.some((slotId) => (canvasAnswers[slotId] ?? []).includes(option.label)); return <label key={option.optionId} className={`v2-choice-item${checked ? " is-checked" : ""}`} {...optionRowProps(option)}><OptionDragHandle canvas={props} taskId={task.taskId} responseGroupId={response.responseGroupId} options={options} index={optionIndex} /><input type="checkbox" value={option.label} checked={checked} disabled={!checked && unorderedSelected >= unorderedLimit} onChange={(event) => { const selected = Array.from(new Set(response.slotIds.flatMap((slotId) => canvasAnswers[slotId] ?? []).filter((value) => value !== option.label))).slice(0, response.slotIds.length); if (event.target.checked) selected.push(option.label); response.slotIds.forEach((slotId, index) => setOption(slotId, selected[index] ?? "", Boolean(selected[index]), false, "unordered_set")); }} /><span><strong>{option.label}</strong>{optionContentDuplicatesLabel(option) ? null : <> <ContentNodes nodes={option.content} canvas={props} /></>}</span><OptionDeleteButton canvas={props} taskId={task.taskId} responseGroupId={response.responseGroupId} option={option} count={options.length} /></label>; })}<div className="v2-slot-summary">{response.slotIds.map((slotId) => <span key={slotId} className="v2-slot-chip" data-question-id={slotId}>{runtime.questionDisplayMap[slotId]}: {(canvasAnswers[slotId] ?? []).join(", ") || "—"}</span>)}</div></fieldset> : <div className="v2-slot-list">{response.slotIds.map((slotId, index) => {
                const slot = runtime.answerSlots[slotId];
                if (!slot) return null;
                const values = canvasAnswers[slotId] ?? [];
                const textEntry = slot.interaction === "text" || response.kind === "text_entry";
                const hostPromptNode = perSlot?.hostNodeBySlotId.get(slotId);
                const paragraphLabel = slot.hostType === "passage_paragraph" && slot.hostNodeId
                  ? paragraphLabelFor(props.authoring, slot.hostNodeId)
                  : undefined;
                const answerControl = textEntry
                  ? <input className="v2-text-answer" type="text" name={slotId} value={values[0] ?? ""} maxLength={slot.constraints?.maxCharacters} aria-label={`Answer ${runtime.questionDisplayMap[slotId]}`} onChange={(event) => setText(slotId, event.target.value)} />
                  : slot.interaction === "select"
                    ? <select className="v2-select-answer" data-question-id={slotId} aria-label={`Answer ${runtime.questionDisplayMap[slotId]}`} value={values[0] ?? ""} onChange={(event) => setOption(slotId, event.target.value, true, false)}><option value="">Select…</option>{options.map((option) => <option key={option.optionId} value={option.label}>{option.label}</option>)}</select>
                    : slot.interaction === "dragdrop"
                      ? slot.hostType === "passage_paragraph"
                        ? <span className="v2-passage-target-reference">{paragraphLabel ? `Paragraph ${paragraphLabel}` : "Passage paragraph"}</span>
                        : <AnswerDropTarget canvas={props} slotId={slotId} placement="row" />
                      : options.length
                        ? <div className={`v2-choice-options${isTfngOptionSet(options) ? " v2-tfng-options" : ""}`} data-option-list={props.mode === "author" ? "" : undefined}>{options.map((option, optionIndex) => <label key={`${slotId}-${option.optionId}`} className={`v2-choice-item${values.includes(option.label) ? " is-checked" : ""}`} {...optionRowProps(option)}><OptionDragHandle canvas={props} taskId={task.taskId} responseGroupId={response.responseGroupId} options={options} index={optionIndex} /><input type={slot.interaction === "checkbox" ? "checkbox" : "radio"} name={slotId} value={option.label} checked={values.includes(option.label)} onChange={(event) => setOption(slotId, option.label, event.target.checked, slot.interaction === "checkbox")} /><span><strong>{option.label}</strong>{optionContentDuplicatesLabel(option) ? null : <> <ContentNodes nodes={option.content} canvas={props} /></>}</span><OptionDeleteButton canvas={props} taskId={task.taskId} responseGroupId={response.responseGroupId} option={option} count={options.length} /></label>)}</div>
                        : <input className="v2-text-answer" type="text" name={slotId} value={values[0] ?? ""} aria-label={`Answer ${runtime.questionDisplayMap[slotId]}`} onChange={(event) => setText(slotId, event.target.value)} />;
                return <div key={slotId} className={`v2-slot-question${props.selectedId === slotId ? " is-selected" : ""}`} data-question-id={slotId} onClick={(event) => { if (props.mode === "author") { event.stopPropagation(); props.onSelect?.(slotId); } }}><div className="v2-slot-question-label"><span className="v2-slot-number">{runtime.questionDisplayMap[slotId]}</span>{hostPromptNode ? <span className="v2-slot-prompt" style={{ flex: 1, minWidth: 0 }}><ContentNodes nodes={[hostPromptNode]} canvas={props} /></span> : null}{slot.hostType === "passage_paragraph" ? <span className="v2-passage-target-reference">{paragraphLabel ? `Paragraph ${paragraphLabel}` : "Passage paragraph"}</span> : response.kind === "text_entry" || response.kind === "matching" ? <span>Response {index + 1}</span> : null}</div>{answerControl}</div>;
              })}</div>}
              {options.length || (props.mode === "author" && (response.kind === "choice" || response.kind === "matching")) ? <OptionAddButton canvas={props} taskId={task.taskId} responseGroupId={response.responseGroupId} options={options} /> : null}
            </section>;
          })}
        </article>)}
      </div>
    </section>
    {!props.comparisonPreview && <QuestionNavBar
      model={navModel}
      mode={props.mode}
      authoring={props.authoring}
      activeSlotId={props.mode === "author" ? props.selectedId : undefined}
      onSelectSlot={props.onSelect}
      onSelectPart={setSelectedPart}
    />}
  </div>
    </OptionDragContext.Provider>
  </CanvasAnswersContext.Provider>
  </AnswerDragContext.Provider>;
}

/** 兼容期别名：`StructuredAuthoringEditorV2` 仍以旧名导入，P10 删除旧页面时一并移除。 */
export const ExamCanvasV2 = ExamCanvas;
