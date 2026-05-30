export type BlockType = "paragraph" | "table" | "image" | "list" | "header" | "footer";
export type BlockRole = "passage" | "question" | "answer" | "ignore";

export interface TableCellIr {
  row: number;
  col: number;
  text: string;
  rowSpan?: number;
  colSpan?: number;
}

export interface TableIr {
  cells: TableCellIr[];
  rows: number;
  cols: number;
}

export interface DocumentBlock {
  blockId: string;
  blockType: BlockType;
  text?: string;
  html?: string;
  table?: TableIr;
  bbox?: [number, number, number, number];
  confidence: number;
  roleHint?: BlockRole;
}

export interface DocumentPage {
  pageIndex: number;
  width: number;
  height: number;
  blocks: DocumentBlock[];
}

export interface DocumentAsset {
  assetId: string;
  type: "image" | "thumbnail";
  path: string;
  bbox?: [number, number, number, number];
}

export interface ParserInfo {
  provider: string;
  version: string;
  mode: "auto" | "text" | "ocr";
  warnings: string[];
}

export interface DocumentIr {
  schemaVersion: "DocumentIRV1";
  jobId: string;
  pages: DocumentPage[];
  assets: DocumentAsset[];
  parser: ParserInfo;
}

export interface ParseOptions {
  mode: "auto" | "text" | "ocr";
}
