import { command } from "./tauriCommands";
import type { ImportJob, LibraryExamSummary } from "../types";
import type { ProcessingState } from "./processingClient";

// Workspace API client（M1 / 计划 §16.14 拆分的第一片）：
// 工作区与题库列表从 library_items_v2 读取，编辑保存走 apply_editor_commands 事务。
// 旧 artifact 会话链（getAuthoringV2/applyAuthoringV2Patches）保留给 legacy 页面与
// 未迁移条目的回退，见 useCanonicalEditor 的双轨逻辑。

export interface WorkspaceItemSummaryV1 {
  itemId: string;
  title: string;
  modality: string;
  status: string;
  editVersion: number;
  hasCanonicalDs: boolean;
  updatedAt: string;
  /** 题库保存：发布后原文件与过程文件已删除（需要原文件的操作不可用）。 */
  sourcePurged?: boolean;
}

export interface WorkspaceItemV1 {
  schemaVersion: string;
  item: WorkspaceItemSummaryV1;
  /** 首稿未生成时为 null（计划 §3 接口契约）。 */
  ds: Record<string, unknown> | null;
  editVersion: number;
  issues: unknown[];
  /**
   * 最近的编辑日志（基线版本 + 来源）。保存冲突时据此判断「是不是只被机器写入挤掉」，
   * 是就自动重放，不必让用户二选一。旧后端没有这个字段。
   */
  recentEdits?: Array<{ baseVersion: number; origin?: string | null }>;
}

export interface LibraryItemSummaryV2 {
  id: string;
  modality: string;
  title: string;
  status: string;
  currentEditVersion: number;
  hasCanonicalDs: boolean;
  sourceAssetId: string | null;
  createdAt: string;
  updatedAt: string;
  deletedAt: string | null;
  /** Part 标签（阅读 P1/P2/P3、听力 Part 1–4、写作 Task 1/2）；判不出为 null。 */
  partLabel?: string | null;
  /** 判定来源：manual | content | range | filename | none。 */
  partSource?: string | null;
  /** 题型（内部 taskType 标识，去重，取自权威稿题组）；展示层译成中文。 */
  questionTypes?: string[];
  /** 权威稿的 examId（与发布清单、当前试卷同一身份）；无权威稿时为 null。 */
  examId?: string | null;
  processing?: ProcessingState | null;
  /** 用户确认入库的时间；未确认为 null/undefined。后端命令上线前不返回该字段。 */
  confirmedAt?: string | null;
}

export interface ApplyEditorCommandsResultV1 {
  schemaVersion: string;
  itemId: string;
  editVersion: number;
  appliedCount: number;
  replayed: boolean;
  recoverySnapshotSaved: boolean;
  status?: string;
}

export async function getWorkspaceItem(itemId: string): Promise<WorkspaceItemV1> {
  return command("get_workspace_item", { itemId });
}

/** Lightweight single-item reads for progress events; no canonical document payload. */
export function getLibraryItemProcessing(itemId: string): Promise<ProcessingState | null> {
  return command("get_library_item_processing", { itemId });
}

export interface LibraryRowData {
  job: ImportJob | null;
  summary: LibraryExamSummary | null;
  item: LibraryItemSummaryV2 | null;
  inTrash: boolean;
}

export function getLibraryRow(itemId: string): Promise<LibraryRowData | null> {
  return command("get_library_row", { itemId });
}

/** 发布门禁结果（`check_publish_preflight`）：编辑器把它作为可操作问题直接呈现。 */
export interface PublishCheckResultV1 {
  schemaVersion: string;
  jobId: string;
  editVersion: number;
  passed: boolean;
  blockers: Array<{ code: string, targetId?: string | null, userMessage?: string, action?: string }>;
  warnings: Array<{ code: string, message?: string }>;
}

export async function getPublishPreflight(itemId: string): Promise<PublishCheckResultV1> {
  return command("get_publish_preflight", { jobId: itemId });
}

export async function applyEditorCommands(input: {
  itemId: string;
  baseVersion: number;
  requestId?: string;
  commands: unknown[];
  title?: string;
}): Promise<ApplyEditorCommandsResultV1> {
  return command("apply_editor_commands", { input });
}

export async function listLibraryItems(includeDeleted = false): Promise<LibraryItemSummaryV2[]> {
  return command("list_library_items", { includeDeleted });
}

/** 题库页改名：后端 trim 后非空、不超过 120 字；返回新的题库行。 */
/** 改名走权威稿的版本 CAS：`baseVersion` 过期时后端返回 EDIT_VERSION_CONFLICT。 */
export async function renameLibraryItem(itemId: string, title: string, baseVersion: number): Promise<LibraryItemSummaryV2> {
  return command("rename_library_item", { itemId, title, baseVersion });
}

/**
 * 用户确认入库 / 退回待确认（`set_library_item_confirmed`）。
 * 后端尚未注册该命令时调用会失败，由 confirmedStore 判断后退回本地兜底。
 */
export async function setLibraryItemConfirmed(itemId: string, confirmed: boolean): Promise<LibraryItemSummaryV2> {
  return command("set_library_item_confirmed", { itemId, confirmed });
}

/** 批量确认 / 退回的逐条结果：业务拒绝逐条返回（`error` 为机器码），数据库错误整批失败。 */
export interface LibraryItemConfirmResult {
  itemId: string;
  ok: boolean;
  row?: LibraryItemSummaryV2;
  /** 机器码，如 `LIBRARY_ITEM_NOT_CONFIRMABLE:processing`、`LIBRARY_ITEM_IN_USE`。 */
  error?: string;
}

/** 批量确认入库 / 退回待确认（`set_library_items_confirmed`）。 */
export async function setLibraryItemsConfirmed(itemIds: string[], confirmed: boolean): Promise<LibraryItemConfirmResult[]> {
  const response = await command<{ results: LibraryItemConfirmResult[] }>("set_library_items_confirmed", { itemIds, confirmed });
  return response.results;
}

/**
 * 把启动前写在 localStorage 的确认集合导入后端（`import_local_confirmed_ids`）。
 * 返回被后端接受（仍存在且可确认）的 id。
 */
export async function importLocalConfirmedIds(itemIds: string[]): Promise<string[]> {
  return command("import_local_confirmed_ids", { itemIds });
}
