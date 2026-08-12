import { useEffect, useMemo, useRef } from "react";
import { EditorContent, useEditor } from "@tiptap/react";
import { Mark, Node, mergeAttributes, type JSONContent } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import { Table } from "@tiptap/extension-table";
import TableRow from "@tiptap/extension-table-row";
import TableHeader from "@tiptap/extension-table-header";
import TableCell from "@tiptap/extension-table-cell";
import TiptapImage from "@tiptap/extension-image";
import type { ContentNodeV2, DiagramHotspotV2 } from "../types/content-doc-v2";
import type { ProvenanceStatus, SourceAnchorV2 } from "../types";

type MetaAttrs = {
  nodeId?: string | null;
  sourceAnchors?: SourceAnchorV2[];
  provenanceStatus?: ProvenanceStatus;
};

const fallbackId = (type: string, path: number[]): string => `manual-${type}-${path.join("-") || "root"}`;

const baseAttrs = () => ({
  nodeId: { default: null },
  sourceAnchors: { default: [] },
  provenanceStatus: { default: "manual" }
});

const AuthoringBlock = Node.create({
  name: "authoringBlock",
  group: "block",
  content: "block*",
  defining: true,
  addAttributes() {
    return {
      ...baseAttrs(),
      nodeType: { default: "paragraph" }
    };
  },
  parseHTML() {
    return [{ tag: "div[data-authoring-block]" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["div", mergeAttributes(HTMLAttributes, { "data-authoring-block": "true", "data-authoring-node-id": HTMLAttributes.nodeId }), 0];
  }
});

const AuthoringText = Node.create({
  name: "authoringText",
  group: "inline",
  inline: true,
  content: "inline*",
  defining: true,
  addAttributes() {
    return baseAttrs();
  },
  parseHTML() {
    return [{ tag: "span[data-authoring-text]" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["span", mergeAttributes(HTMLAttributes, { "data-authoring-text": "true", "data-authoring-text-id": HTMLAttributes.nodeId }), 0];
  }
});

const AuthoringAnswerSlot = Node.create({
  name: "authoringAnswerSlot",
  group: "inline",
  inline: true,
  atom: true,
  selectable: true,
  addAttributes() {
    return {
      ...baseAttrs(),
      slotId: { default: "" },
      displayLabel: { default: "□" },
      inline: { default: true },
      placeholder: { default: null }
    };
  },
  parseHTML() {
    return [{ tag: "span[data-answer-slot]" }];
  },
  renderHTML({ HTMLAttributes }) {
    const label = String(HTMLAttributes.displayLabel ?? "□");
    return ["span", mergeAttributes(HTMLAttributes, {
      "data-answer-slot": "true",
      "data-authoring-node-id": HTMLAttributes.nodeId,
      class: "tiptap-answer-slot"
    }), label];
  }
});

const AuthoringMedia = Node.create({
  name: "authoringMedia",
  group: "block",
  content: "block*",
  defining: true,
  addAttributes() {
    return {
      ...baseAttrs(),
      nodeType: { default: "figure" },
      assetId: { default: "" },
      altText: { default: null },
      display: { default: {} },
      hotspots: { default: [] },
      crop: { default: null }
    };
  },
  parseHTML() {
    return [{ tag: "figure[data-authoring-media]" }];
  },
  renderHTML({ HTMLAttributes }) {
    const nodeType = String(HTMLAttributes.nodeType ?? "figure");
    return ["figure", mergeAttributes(HTMLAttributes, {
      "data-authoring-media": nodeType,
      "data-authoring-node-id": HTMLAttributes.nodeId,
      class: `tiptap-media tiptap-media-${nodeType}`
    }), ["div", { class: "tiptap-media-label" }, `${nodeType} · ${String(HTMLAttributes.assetId ?? "asset")}`], 0];
  }
});

const AuthoringImage = Node.create({
  name: "authoringImage",
  group: "block",
  atom: true,
  selectable: true,
  addAttributes() {
    return {
      ...baseAttrs(),
      assetId: { default: "" },
      altText: { default: "" },
      display: { default: {} },
      crop: { default: null }
    };
  },
  renderHTML({ HTMLAttributes }) {
    return ["div", mergeAttributes(HTMLAttributes, {
      "data-authoring-image": "true",
      "data-authoring-node-id": HTMLAttributes.nodeId,
      class: "tiptap-media tiptap-media-image"
    }), ["span", { class: "tiptap-media-label" }, `image · ${String(HTMLAttributes.assetId ?? "asset")}`]];
  }
});

const AuthoringFlowchart = Node.create({
  name: "authoringFlowchart",
  group: "block",
  atom: true,
  selectable: true,
  addAttributes() {
    return { ...baseAttrs(), payload: { default: null } };
  },
  parseHTML() {
    return [{ tag: "div[data-authoring-flowchart]" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["div", mergeAttributes(HTMLAttributes, {
      "data-authoring-flowchart": "true",
      "data-authoring-node-id": HTMLAttributes.nodeId,
      class: "tiptap-flowchart"
    }), "流程图 / flowchart"];
  }
});

const AuthoringLink = Mark.create({
  name: "authoringLink",
  inclusive: false,
  addAttributes() {
    return { href: { default: "" } };
  },
  parseHTML() {
    return [{ tag: "a[href]", getAttrs: (element) => ({ href: (element as HTMLElement).getAttribute("href") ?? "" }) }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["a", mergeAttributes(HTMLAttributes), 0];
  }
});

const AuthoringTableRow = TableRow.extend({
  addAttributes() {
    return { ...this.parent?.(), ...baseAttrs() };
  }
});

const AuthoringTableHeader = TableHeader.extend({
  addAttributes() {
    return { ...this.parent?.(), ...baseAttrs(), headerScope: { default: "column" } };
  }
});

const AuthoringTableCell = TableCell.extend({
  addAttributes() {
    return { ...this.parent?.(), ...baseAttrs(), headerScope: { default: "none" } };
  }
});

const AuthoringUnderline = Mark.create({
  name: "authoringUnderline",
  parseHTML() {
    return [{ tag: "u" }, { style: "text-decoration" }];
  },
  renderHTML() {
    return ["u", 0];
  }
});

function meta(node: ContentNodeV2, path: number[]): Record<string, unknown> {
  return {
    nodeId: node.id || fallbackId(node.type, path),
    sourceAnchors: node.sourceAnchors ?? [],
    provenanceStatus: node.provenanceStatus ?? "manual"
  };
}

function marksToTiptap(marks: Array<"bold" | "italic" | "underline" | { link: string }> | undefined): JSONContent["marks"] {
  return marks?.map((mark) => typeof mark === "string"
    ? { type: mark === "underline" ? "authoringUnderline" : mark }
    : { type: "authoringLink", attrs: { href: mark.link } });
}

function contentToTiptap(node: ContentNodeV2, path: number[]): JSONContent {
  const attrs = meta(node, path);
  switch (node.type) {
    case "text":
      return { type: "authoringText", attrs, content: [{ type: "text", text: node.text, marks: marksToTiptap(node.marks) }] };
    case "answer_slot":
      return { type: "authoringAnswerSlot", attrs: { ...attrs, slotId: node.slotId, displayLabel: node.displayLabel, inline: node.inline, placeholder: node.placeholder ?? null } };
    case "figure":
    case "diagram":
      return {
        type: "authoringBlock",
        attrs: { ...attrs, nodeType: node.type },
        content: [{ type: "authoringMedia", attrs: { ...attrs, nodeType: node.type, assetId: node.assetId, display: node.display, hotspots: node.hotspots ?? [], crop: node.crop ?? null }, content: node.type === "figure" && node.caption ? node.caption.map((child, index) => contentToTiptap(child, path.concat(index))) : undefined }]
      };
    case "image":
      return { type: "authoringBlock", attrs: { ...attrs, nodeType: node.type }, content: [{ type: "authoringImage", attrs: { ...attrs, assetId: node.assetId, altText: node.altText ?? "", display: node.display, crop: node.crop ?? null } }] };
    case "flowchart":
      return { type: "authoringBlock", attrs: { ...attrs, nodeType: node.type }, content: [{ type: "authoringFlowchart", attrs: { ...attrs, payload: node } }] };
    case "option_bank":
      return { type: "authoringBlock", attrs: { ...attrs, nodeType: node.type }, content: [{ type: "authoringFlowchart", attrs: { ...attrs, payload: node } }] };
    case "paragraph":
      return { type: "authoringBlock", attrs: { ...attrs, nodeType: node.type }, content: [{ type: "paragraph", content: node.children.map((child, index) => contentToTiptap(child, path.concat(index))) }] };
    case "heading":
      return { type: "authoringBlock", attrs: { ...attrs, nodeType: node.type }, content: [{ type: "heading", attrs: { level: node.level }, content: node.children.map((child, index) => contentToTiptap(child, path.concat(index))) }] };
    case "bullet_list":
      return { type: "authoringBlock", attrs: { ...attrs, nodeType: node.type }, content: [{ type: "bulletList", content: node.items.map((item, index) => ({ type: "listItem", content: [{ type: "paragraph", content: item.children.map((child, childIndex) => contentToTiptap(child, path.concat(index, childIndex))) }] })) }] };
    case "ordered_list":
      return { type: "authoringBlock", attrs: { ...attrs, nodeType: node.type }, content: [{ type: "orderedList", content: node.items.map((item, index) => ({ type: "listItem", content: [{ type: "paragraph", content: item.children.map((child, childIndex) => contentToTiptap(child, path.concat(index, childIndex))) }] })) }] };
    case "table":
      return { type: "authoringBlock", attrs: { ...attrs, nodeType: node.type }, content: [{ type: "table", attrs, content: node.rows.map((row, rowIndex) => ({ type: "tableRow", attrs: meta(row, path.concat(rowIndex)), content: row.cells.map((cell, cellIndex) => ({ type: cell.headerScope && cell.headerScope !== "none" ? "tableHeader" : "tableCell", attrs: { ...meta(cell, path.concat(rowIndex, cellIndex)), colspan: cell.colSpan, rowspan: cell.rowSpan, colwidth: null, headerScope: cell.headerScope ?? "none" }, content: [{ type: "paragraph", content: cell.children.map((child, childIndex) => contentToTiptap(child, path.concat(rowIndex, cellIndex, childIndex))) }] })) })) }] };
    case "hard_break":
      return { type: "hardBreak" };
    case "horizontal_rule":
      return { type: "horizontalRule" };
    case "figcaption":
    case "flow_step":
    case "list_item":
    case "doc":
      return { type: "authoringBlock", attrs: { ...attrs, nodeType: node.type }, content: [{ type: "paragraph", content: "children" in node ? node.children.map((child, index) => contentToTiptap(child, path.concat(index))) : [] }] };
    case "table_row":
    case "table_cell":
      return { type: "authoringBlock", attrs: { ...attrs, nodeType: node.type }, content: [{ type: "paragraph", content: "children" in node ? node.children.map((child, index) => contentToTiptap(child, path.concat(index))) : [] }] };
  }
}

export function contentNodesToTiptap(nodes: ContentNodeV2[]): JSONContent {
  return { type: "doc", content: nodes.map((node, index) => contentToTiptap(node, [index])) };
}

function canonicalMeta(attrs: Record<string, unknown> | undefined, type: string, path: number[]): { id: string; sourceAnchors: SourceAnchorV2[]; provenanceStatus: ProvenanceStatus } {
  const safe = attrs ?? {};
  return {
    id: typeof safe.nodeId === "string" && safe.nodeId ? safe.nodeId : fallbackId(type, path),
    sourceAnchors: Array.isArray(safe.sourceAnchors) ? safe.sourceAnchors as SourceAnchorV2[] : [],
    provenanceStatus: safe.provenanceStatus === "source" || safe.provenanceStatus === "derived" || safe.provenanceStatus === "user_edited" || safe.provenanceStatus === "manual" ? safe.provenanceStatus : "manual"
  };
}

function marksFromTiptap(value: JSONContent["marks"]): Array<"bold" | "italic" | "underline" | { link: string }> | undefined {
  const marks: Array<"bold" | "italic" | "underline" | { link: string }> = [];
  for (const mark of value ?? []) {
    if (mark.type === "bold" || mark.type === "italic") marks.push(mark.type);
    else if (mark.type === "authoringUnderline") marks.push("underline");
    else if (mark.type === "authoringLink") marks.push({ link: String(mark.attrs?.href ?? "") });
  }
  return marks?.length ? marks : undefined;
}

function childrenFromTiptap(value: JSONContent["content"], path: number[]): ContentNodeV2[] {
  return (value ?? []).flatMap((child, index) => {
    if (child.type === "authoringText") return textSegmentsFromTiptap(child, path.concat(index));
    const mapped = canonicalFromTiptap(child, path.concat(index));
    return mapped ? [mapped] : [];
  });
}

function textSegmentsFromTiptap(value: JSONContent, path: number[]): ContentNodeV2[] {
  const metaValue = canonicalMeta(value.attrs, "text", path);
  const textChildren = (value.content ?? []).filter((child) => child.type === "text");
  if (!textChildren.length) return [{ type: "text", ...metaValue, text: "" }];
  return textChildren.map((child, index) => ({
    type: "text" as const,
    ...metaValue,
    id: index === 0 ? metaValue.id : `${metaValue.id}-segment-${index}`,
    text: child.text ?? "",
    marks: marksFromTiptap(child.marks)
  }));
}

function canonicalFromTiptap(value: JSONContent, path: number[]): ContentNodeV2 | undefined {
  const type = value.type;
  if (!type) return undefined;
  if (type === "authoringText") {
    return textSegmentsFromTiptap(value, path)[0];
  }
  if (type === "authoringAnswerSlot") {
    const metaValue = canonicalMeta(value.attrs, "answer_slot", path);
    return { type: "answer_slot", ...metaValue, slotId: String(value.attrs?.slotId ?? ""), displayLabel: String(value.attrs?.displayLabel ?? "□"), inline: Boolean(value.attrs?.inline ?? true), placeholder: typeof value.attrs?.placeholder === "string" ? value.attrs.placeholder : undefined };
  }
  if (type === "authoringMedia") {
    const metaValue = canonicalMeta(value.attrs, String(value.attrs?.nodeType ?? "figure"), path);
    const nodeType = value.attrs?.nodeType === "diagram" ? "diagram" : "figure";
    const common = { ...metaValue, assetId: String(value.attrs?.assetId ?? ""), hotspots: Array.isArray(value.attrs?.hotspots) ? value.attrs.hotspots as DiagramHotspotV2[] : undefined, crop: Array.isArray(value.attrs?.crop) ? value.attrs.crop as [number, number, number, number] : undefined, display: (value.attrs?.display ?? {}) as { widthPercent?: number; maxWidthPx?: number; align?: "left" | "center" | "right" } };
    return nodeType === "diagram" ? { type: "diagram", ...common } : { type: "figure", ...common, caption: childrenFromTiptap(value.content, path) };
  }
  if (type === "authoringImage") {
    const metaValue = canonicalMeta(value.attrs, "image", path);
    return { type: "image", ...metaValue, assetId: String(value.attrs?.assetId ?? ""), altText: String(value.attrs?.altText ?? ""), crop: Array.isArray(value.attrs?.crop) ? value.attrs.crop as [number, number, number, number] : undefined, display: (value.attrs?.display ?? {}) as { widthPercent?: number; maxWidthPx?: number; align?: "left" | "center" | "right" } };
  }
  if (type === "authoringFlowchart") {
    const payload = value.attrs?.payload;
    if (payload && typeof payload === "object" && "type" in payload) return payload as ContentNodeV2;
    const metaValue = canonicalMeta(value.attrs, "flowchart", path);
    return { type: "flowchart", ...metaValue, steps: [] };
  }
  if (type === "authoringBlock") {
    const nodeType = String(value.attrs?.nodeType ?? "paragraph");
    const inner = (value.content ?? []).find((child) => child.type !== "authoringBlock");
    const mapped = inner ? canonicalFromTiptap(inner, path) : undefined;
    const metaValue = canonicalMeta(value.attrs, nodeType, path);
    if (nodeType === "paragraph") return { type: "paragraph", ...metaValue, children: mapped?.type === "paragraph" ? mapped.children : childrenFromTiptap(value.content, path) };
    if (nodeType === "heading") return { type: "heading", ...metaValue, level: mapped?.type === "heading" ? mapped.level : Number((inner?.attrs as Record<string, unknown> | undefined)?.level ?? 2), children: mapped?.type === "heading" ? mapped.children : childrenFromTiptap(value.content, path) };
    if (nodeType === "bullet_list" || nodeType === "ordered_list") {
      const list = mapped;
      if (list?.type === nodeType) return { ...list, ...metaValue };
      const items = (inner?.content ?? []).map((item, index) => ({ type: "list_item" as const, ...canonicalMeta(undefined, "list_item", path.concat(index)), children: childrenFromTiptap(item.content?.[0]?.content, path.concat(index)) }));
      return nodeType === "bullet_list" ? { type: "bullet_list", ...metaValue, items } : { type: "ordered_list", ...metaValue, items };
    }
    if (nodeType === "table" && mapped?.type === "table") return { ...mapped, ...metaValue };
    if (mapped && ["figure", "diagram", "image", "flowchart", "option_bank"].includes(mapped.type)) return { ...mapped, ...metaValue } as ContentNodeV2;
    if (nodeType === "horizontal_rule") return { type: "horizontal_rule", ...metaValue };
    return { type: "paragraph", ...metaValue, children: childrenFromTiptap(value.content, path) };
  }
  if (type === "paragraph") {
    const metaValue = canonicalMeta(value.attrs, "paragraph", path);
    return { type: "paragraph", ...metaValue, children: childrenFromTiptap(value.content, path) };
  }
  if (type === "heading") {
    const metaValue = canonicalMeta(value.attrs, "heading", path);
    return { type: "heading", ...metaValue, level: Number(value.attrs?.level ?? 2), children: childrenFromTiptap(value.content, path) };
  }
  if (type === "bulletList" || type === "orderedList") {
    const metaValue = canonicalMeta(value.attrs, type === "bulletList" ? "bullet_list" : "ordered_list", path);
    const items = (value.content ?? []).map((item, index) => ({ type: "list_item" as const, ...canonicalMeta(undefined, "list_item", path.concat(index)), children: childrenFromTiptap(item.content?.[0]?.content, path.concat(index)) }));
    return type === "bulletList" ? { type: "bullet_list", ...metaValue, items } : { type: "ordered_list", ...metaValue, items };
  }
  if (type === "table") {
    const metaValue = canonicalMeta(value.attrs, "table", path);
    const rows = (value.content ?? []).map((row, rowIndex) => ({ type: "table_row" as const, ...canonicalMeta(row.attrs, "table_row", path.concat(rowIndex)), cells: (row.content ?? []).map((cell, cellIndex) => {
      const rawHeaderScope = cell.attrs?.headerScope;
      const headerScope = rawHeaderScope === "row" || rawHeaderScope === "column" || rawHeaderScope === "both" || rawHeaderScope === "none"
        ? rawHeaderScope
        : cell.type === "tableHeader" ? "column" : "none";
      return { type: "table_cell" as const, ...canonicalMeta(cell.attrs, "table_cell", path.concat(rowIndex, cellIndex)), rowSpan: Number(cell.attrs?.rowspan ?? 1), colSpan: Number(cell.attrs?.colspan ?? 1), headerScope, children: childrenFromTiptap(cell.content?.flatMap((child) => child.type === "paragraph" ? child.content ?? [] : [child]), path.concat(rowIndex, cellIndex)) };
    }) }));
    return { type: "table", ...metaValue, rows };
  }
  if (type === "hardBreak") return { type: "hard_break", ...canonicalMeta(value.attrs, "hard_break", path) };
  if (type === "horizontalRule") return { type: "horizontal_rule", ...canonicalMeta(value.attrs, "horizontal_rule", path) };
  if (type === "text") return { type: "text", ...canonicalMeta(value.attrs, "text", path), text: value.text ?? "", marks: marksFromTiptap(value.marks) };
  return undefined;
}

export function tiptapToContentNodes(value: JSONContent): ContentNodeV2[] {
  return childrenFromTiptap(value.content, []);
}

const extensions = [
  StarterKit.configure({
    heading: { levels: [1, 2, 3, 4, 5, 6] },
    bulletList: { keepMarks: true },
    orderedList: { keepMarks: true }
  }),
  Table.configure({ resizable: true }),
  AuthoringTableRow,
  AuthoringTableHeader,
  AuthoringTableCell,
  TiptapImage.configure({ inline: false, allowBase64: true }),
  AuthoringBlock,
  AuthoringText,
  AuthoringAnswerSlot,
  AuthoringMedia,
  AuthoringImage,
  AuthoringFlowchart,
  AuthoringLink,
  AuthoringUnderline
];

export interface AuthoringTiptapEditorProps {
  nodes: ContentNodeV2[];
  onChange: (nodes: ContentNodeV2[]) => void;
  onSelect?: (nodeId: string) => void;
  readOnly?: boolean;
  ariaLabel?: string;
}

export function AuthoringTiptapEditor({ nodes, onChange, onSelect, readOnly = false, ariaLabel = "结构化内容编辑器" }: AuthoringTiptapEditorProps) {
  const inputSignature = useMemo(() => JSON.stringify(nodes), [nodes]);
  const lastEmittedSignature = useRef("");
  const onChangeRef = useRef(onChange);
  const onSelectRef = useRef(onSelect);
  onChangeRef.current = onChange;
  onSelectRef.current = onSelect;
  const editor = useEditor({
    extensions,
    content: contentNodesToTiptap(nodes),
    editable: !readOnly,
    immediatelyRender: false,
    onUpdate: ({ editor: current }) => {
      const next = tiptapToContentNodes(current.getJSON());
      const signature = JSON.stringify(next);
      lastEmittedSignature.current = signature;
      onChangeRef.current(next);
    }
  });

  useEffect(() => {
    if (!editor) return;
    editor.setEditable(!readOnly);
  }, [editor, readOnly]);

  useEffect(() => {
    if (!editor || inputSignature === lastEmittedSignature.current) return;
    const currentSignature = JSON.stringify(tiptapToContentNodes(editor.getJSON()));
    if (currentSignature !== inputSignature) editor.commands.setContent(contentNodesToTiptap(nodes), { emitUpdate: false });
  }, [editor, inputSignature, nodes]);

  if (!editor) return <div className="tiptap-authoring-shell is-loading">正在加载编辑器…</div>;
  return <div className="tiptap-authoring-shell" aria-label={ariaLabel} onClick={(event) => {
    const target = (event.target as HTMLElement).closest<HTMLElement>("[data-authoring-node-id], [data-authoring-text-id]");
    const id = target?.dataset.authoringNodeId ?? target?.dataset.authoringTextId;
    if (id) onSelectRef.current?.(id);
  }}>
    {!readOnly ? <div className="tiptap-authoring-toolbar" role="toolbar" aria-label="格式工具">
      <button type="button" onClick={() => editor.chain().focus().toggleBold().run()} className={editor.isActive("bold") ? "active" : ""}>粗体</button>
      <button type="button" onClick={() => editor.chain().focus().toggleItalic().run()} className={editor.isActive("italic") ? "active" : ""}>斜体</button>
      <button type="button" onClick={() => editor.chain().focus().toggleMark("authoringUnderline").run()} className={editor.isActive("authoringUnderline") ? "active" : ""}>下划线</button>
      <button type="button" onClick={() => editor.chain().focus().toggleBulletList().run()} className={editor.isActive("bulletList") ? "active" : ""}>项目符号</button>
      <button type="button" onClick={() => editor.chain().focus().toggleOrderedList().run()} className={editor.isActive("orderedList") ? "active" : ""}>编号列表</button>
      <button type="button" onClick={() => editor.chain().focus().insertTable({ rows: 2, cols: 2, withHeaderRow: true }).run()}>插入表格</button>
      <button type="button" onClick={() => editor.chain().focus().addRowAfter().run()}>加行</button>
      <button type="button" onClick={() => editor.chain().focus().addColumnAfter().run()}>加列</button>
      <button type="button" onClick={() => editor.chain().focus().deleteRow().run()}>删行</button>
      <button type="button" onClick={() => editor.chain().focus().deleteColumn().run()}>删列</button>
      <button type="button" onClick={() => editor.chain().focus().mergeCells().run()}>合并</button>
      <button type="button" onClick={() => editor.chain().focus().splitCell().run()}>拆分</button>
    </div> : null}
    <EditorContent editor={editor} />
  </div>;
}
