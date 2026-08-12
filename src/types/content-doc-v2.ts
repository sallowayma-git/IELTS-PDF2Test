import type { AssetDescriptorV2, ProvenanceStatus, SourceAnchorV2 } from "./schema-common-v2";

export type { ProvenanceStatus } from "./schema-common-v2";

export type ContentNodeV2 =
  | DocNodeV2
  | ParagraphNodeV2
  | HeadingNodeV2
  | TextNodeV2
  | HardBreakNodeV2
  | BulletListNodeV2
  | OrderedListNodeV2
  | ListItemNodeV2
  | TableContentNodeV2
  | TableRowContentNodeV2
  | TableCellContentNodeV2
  | FigureNodeV2
  | ImageNodeV2
  | FigcaptionNodeV2
  | FlowchartNodeV2
  | FlowStepNodeV2
  | DiagramNodeV2
  | AnswerSlotNodeV2
  | OptionBankNodeV2
  | HorizontalRuleNodeV2;

export interface BaseContentNodeV2 {
  id: string;
  sourceAnchors: SourceAnchorV2[];
  provenanceStatus: ProvenanceStatus;
}

export interface DocNodeV2 extends BaseContentNodeV2 {
  type: "doc";
  children: ContentNodeV2[];
}

export interface ParagraphNodeV2 extends BaseContentNodeV2 {
  type: "paragraph";
  children: ContentNodeV2[];
  align?: "left" | "center" | "right" | "justify";
  indentLevel?: number;
}

export interface HeadingNodeV2 extends BaseContentNodeV2 {
  type: "heading";
  level: number;
  children: ContentNodeV2[];
}

export interface TextNodeV2 extends BaseContentNodeV2 {
  type: "text";
  text: string;
  marks?: Array<"bold" | "italic" | "underline" | { link: string }>;
}

export interface HardBreakNodeV2 extends BaseContentNodeV2 {
  type: "hard_break";
}

export interface BulletListNodeV2 extends BaseContentNodeV2 {
  type: "bullet_list";
  items: ListItemNodeV2[];
}

export interface OrderedListNodeV2 extends BaseContentNodeV2 {
  type: "ordered_list";
  items: ListItemNodeV2[];
}

export interface ListItemNodeV2 extends BaseContentNodeV2 {
  type: "list_item";
  children: ContentNodeV2[];
}

export interface TableContentNodeV2 extends BaseContentNodeV2 {
  type: "table";
  rows: TableRowContentNodeV2[];
  caption?: ContentNodeV2[];
  sourceTableId?: string;
  visualFallbackAssetId?: string;
}

export interface TableRowContentNodeV2 extends BaseContentNodeV2 {
  type: "table_row";
  cells: TableCellContentNodeV2[];
}

export interface TableCellContentNodeV2 extends BaseContentNodeV2 {
  type: "table_cell";
  rowSpan: number;
  colSpan: number;
  headerScope?: "row" | "column" | "both" | "none";
  children: ContentNodeV2[];
}

export interface ContentDisplayV2 {
  widthPercent?: number;
  maxWidthPx?: number;
  align?: "left" | "center" | "right";
}

export interface DiagramHotspotV2 {
  hotspotId: string;
  slotId: string;
  normalizedRect: [number, number, number, number];
  labelAnchor?: [number, number];
}

export interface FigureNodeV2 extends BaseContentNodeV2 {
  type: "figure";
  assetId: string;
  caption?: ContentNodeV2[];
  hotspots?: DiagramHotspotV2[];
  /** Normalized [x, y, width, height] crop in asset coordinates. */
  crop?: [number, number, number, number];
  display: ContentDisplayV2;
}

export interface ImageNodeV2 extends BaseContentNodeV2 {
  type: "image";
  assetId: string;
  altText?: string;
  /** Normalized [x, y, width, height] crop in asset coordinates. */
  crop?: [number, number, number, number];
  display: ContentDisplayV2;
}

export interface FigcaptionNodeV2 extends BaseContentNodeV2 {
  type: "figcaption";
  children: ContentNodeV2[];
}

export interface FlowchartNodeV2 extends BaseContentNodeV2 {
  type: "flowchart";
  steps: FlowStepNodeV2[];
  display?: ContentDisplayV2;
}

export interface FlowStepNodeV2 extends BaseContentNodeV2 {
  type: "flow_step";
  label?: string;
  children: ContentNodeV2[];
  slotIds?: string[];
}

export interface DiagramNodeV2 extends BaseContentNodeV2 {
  type: "diagram";
  assetId: string;
  hotspots?: DiagramHotspotV2[];
  /** Normalized [x, y, width, height] crop in asset coordinates. */
  crop?: [number, number, number, number];
  display: ContentDisplayV2;
}

export interface AnswerSlotNodeV2 extends BaseContentNodeV2 {
  type: "answer_slot";
  slotId: string;
  displayLabel: string;
  inline: boolean;
  placeholder?: string;
}

export interface OptionContentNodeV2 {
  optionId: string;
  label: string;
  children: ContentNodeV2[];
  sourceAnchors: SourceAnchorV2[];
}

export interface OptionBankNodeV2 extends BaseContentNodeV2 {
  type: "option_bank";
  optionBankId: string;
  options: OptionContentNodeV2[];
  allowReuse: boolean;
}

export interface HorizontalRuleNodeV2 extends BaseContentNodeV2 {
  type: "horizontal_rule";
}

export interface ContentDocV2 {
  schemaVersion: "ContentDocV2";
  documentId: string;
  sourceDocumentId?: string;
  root: ContentNodeV2[];
}

export const CONTENT_DOC_V2_SCHEMA_VERSION = "ContentDocV2" as const;

// Keep the shared primitives visible to generated-type consumers that build a single contract index.
export type { AssetDescriptorV2, SourceAnchorV2 };
