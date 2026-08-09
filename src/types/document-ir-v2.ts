import type {
  AssetDescriptorV2,
  ExtractionMode,
  QuadV2,
  RectV2,
  SourceAnchorV2,
  SourceFileRecordV2,
  TextStyleV2
} from "./schema-common-v2";

export type {
  AssetDescriptorV2,
  ExtractionMode,
  QuadV2,
  RectV2,
  SourceAnchorV2,
  SourceFileRecordV2,
  TextStyleV2
} from "./schema-common-v2";

export interface GlyphNodeV2 {
  id: string;
  text: string;
  bbox: RectV2;
  quad?: QuadV2;
  origin: { x: number; y: number };
  baseline?: number;
  angleRad?: number;
  style: TextStyleV2;
  unicodeMapError: boolean;
  hidden: boolean;
  confidence: number;
  source: "native" | "ocr";
  sourceAnchor: SourceAnchorV2;
}

export interface SpanNodeV2 {
  id: string;
  glyphIds: string[];
  text: string;
  bbox: RectV2;
  style: TextStyleV2;
  whitespaceBefore: "none" | "source" | "synthetic";
  whitespaceAfter: "none" | "source" | "synthetic";
  confidence: number;
  sourceAnchors: SourceAnchorV2[];
}

export interface LineNodeV2 {
  id: string;
  spanIds: string[];
  text: string;
  bbox: RectV2;
  baseline?: number;
  writingMode: "horizontal-tb" | "vertical-rl" | "vertical-lr";
  indentationPt: number;
  hangingIndentPt?: number;
  lineHeightPt?: number;
  hardBreakAfter: boolean;
  sourceOrder: number;
  confidence: number;
  sourceAnchors: SourceAnchorV2[];
}

export type PhysicalRegionKind =
  | "text"
  | "title"
  | "list"
  | "table"
  | "figure"
  | "diagram"
  | "form"
  | "header"
  | "footer"
  | "page_number"
  | "rule"
  | "unknown";

export interface RegionNodeV2 {
  id: string;
  kind: PhysicalRegionKind;
  bbox: RectV2;
  childLineIds: string[];
  childObjectIds: string[];
  columnIndex?: number;
  sectionIndex?: number;
  zIndex?: number;
  readingOrderRank?: number;
  readingOrderAlternatives?: string[][];
  confidence: number;
  sourceAnchors: SourceAnchorV2[];
}

export type VectorPathCommandV2 =
  | { op: "move"; x: number; y: number }
  | { op: "line"; x: number; y: number }
  | { op: "curve"; points: number[] }
  | { op: "close" };

export interface VectorPathV2 {
  id: string;
  bbox: RectV2;
  commands?: VectorPathCommandV2[];
  strokeWidth?: number;
  strokeColor?: string;
  fillColor?: string;
  isAxisAlignedRule?: boolean;
  sourceAnchor: SourceAnchorV2;
}

export interface TableCellV2 {
  cellId: string;
  row: number;
  col: number;
  rowSpan: number;
  colSpan: number;
  bbox: RectV2;
  contentRegionIds: string[];
  headerScope?: "row" | "column" | "both" | "none";
  borderEvidence: string[];
  confidence: number;
  sourceAnchors: SourceAnchorV2[];
}

export interface TableNodeV2 {
  id: string;
  bbox: RectV2;
  rows: number;
  cols: number;
  cells: TableCellV2[];
  detectionMode: "ooxml" | "ruling_lines" | "text_alignment" | "vision_model" | "manual";
  captionRegionId?: string;
  visualFallbackAssetId?: string;
  topologyConfidence: number;
  contentConfidence: number;
  sourceAnchors: SourceAnchorV2[];
}

export interface PageQualityV2 {
  classification: "born_digital" | "mixed" | "scanned" | "garbled" | "empty";
  nativeCharacterCount: number;
  unicodeErrorRatio: number;
  duplicateTextRatio: number;
  imageCoverageRatio: number;
  textCoverageRatio: number;
  rotationConfidence: number;
  requiresOcrRegions: RectV2[];
  warnings: string[];
}

export interface PageNodeV2 {
  pageIndex: number;
  widthPt: number;
  heightPt: number;
  rotation: 0 | 90 | 180 | 270;
  mediaBox?: RectV2;
  cropBox?: RectV2;
  glyphs: GlyphNodeV2[];
  spans: SpanNodeV2[];
  lines: LineNodeV2[];
  regions: RegionNodeV2[];
  vectorPaths: VectorPathV2[];
  tables: TableNodeV2[];
  assetIds: string[];
  readingOrder: string[];
  quality: PageQualityV2;
}

export interface CoverageEntryV2 {
  sourceNodeId: string;
  disposition:
    | "passage"
    | "question"
    | "instruction"
    | "option"
    | "answer"
    | "explanation"
    | "header_footer"
    | "decorative"
    | "ignored_with_reason"
    | "unassigned";
  targetIds: string[];
  reason?: string;
}

export interface DocumentIRV2 {
  schemaVersion: "DocumentIRV2";
  documentId: string;
  jobId: string;
  sourceFiles: SourceFileRecordV2[];
  pages: PageNodeV2[];
  assets: AssetDescriptorV2[];
  coverageLedger: CoverageEntryV2[];
  parser: {
    provider: string;
    providerVersion: string;
    extractionStartedAt: string;
    extractionCompletedAt: string;
    options: Record<string, unknown>;
    warnings: string[];
  };
}

export interface DocumentIrV2OverlayResult {
  shadowPath: string;
  overlayPath: string;
  pageCount: number;
  glyphCount: number;
}

export const DOCUMENT_IR_V2_SCHEMA_VERSION = "DocumentIRV2" as const;
