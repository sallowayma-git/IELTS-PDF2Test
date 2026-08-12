/** Shared primitives for the Phase 1 V2 contract bundle. */

export type ExtractionMode =
  | "pdf_native"
  | "pdf_ocr"
  | "pdf_rendered_crop"
  | "docx_ooxml"
  | "docx_rendered_fallback"
  | "manual";

export type PageRotation = 0 | 90 | 180 | 270;
export type CoordinateUnit = "pt" | "emu" | "px";
export type CoordinateOrigin = "top-left" | "bottom-left";

export interface RectV2 {
  x: number;
  y: number;
  width: number;
  height: number;
  unit: CoordinateUnit;
  origin: CoordinateOrigin;
  pageRotation: PageRotation;
  normalized?: [number, number, number, number];
}

export interface QuadV2 {
  points: [number, number, number, number, number, number, number, number];
  unit: CoordinateUnit;
  origin: CoordinateOrigin;
}

export interface SourceCharRangeV2 {
  start: number;
  end: number;
}

export interface SourceAnchorV2 {
  sourceFileId: string;
  pageIndex: number;
  nodeIds: string[];
  bbox?: RectV2;
  nativeBBox?: RectV2;
  displayBBox?: RectV2;
  pdfToDisplay?: [number, number, number, number, number, number];
  charRange?: SourceCharRangeV2;
  ooxmlPath?: string;
  relationshipId?: string;
  extractionMode: ExtractionMode;
  sourceHash: string;
  variants?: SourceVariantV2[];
}

export interface SourceVariantV2 {
  text?: string;
  extractionMode: ExtractionMode;
  bbox?: RectV2;
  confidence?: number;
  provider?: string;
  providerVersion?: string;
  language?: string;
  nodeIds?: string[];
}

export interface SourceFileRecordV2 {
  sourceFileId: string;
  originalName: string;
  mediaType: string;
  sha256: string;
  byteLength: number;
  role: "question_paper" | "answer_key" | "explanation" | "supplement" | "unknown";
}

export interface TextStyleV2 {
  fontName?: string;
  fontSizePt?: number;
  weight?: number;
  bold?: boolean;
  italic?: boolean;
  underline?: boolean;
  strike?: boolean;
  color?: string;
  backgroundColor?: string;
  superscript?: boolean;
  subscript?: boolean;
  language?: string;
}

export type ProvenanceStatus = "source" | "derived" | "user_edited" | "manual";

export type AssetKind =
  | "raster_image"
  | "vector_render"
  | "page_crop"
  | "diagram"
  | "chart"
  | "audio"
  | "thumbnail";

export interface AssetDescriptorV2 {
  assetId: string;
  kind: AssetKind;
  mime: string;
  relativePath: string;
  sha256: string;
  byteLength: number;
  widthPx?: number;
  heightPx?: number;
  durationMs?: number;
  extractionMode: "embedded" | "page_crop" | "rendered_vector" | "docx_media" | "user_upload";
  altText?: string;
  decorative?: boolean;
  sourceAnchor?: SourceAnchorV2;
}
