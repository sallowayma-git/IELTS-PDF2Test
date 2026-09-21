import { command } from "./tauriCommands";
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
  processing?: ProcessingState | null;
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
