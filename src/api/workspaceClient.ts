import { command } from "./tauriCommands";

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
}

export interface ApplyEditorCommandsResultV1 {
  schemaVersion: string;
  itemId: string;
  editVersion: number;
  appliedCount: number;
  replayed: boolean;
  recoverySnapshotSaved: boolean;
  status: string;
}

export async function getWorkspaceItem(itemId: string): Promise<WorkspaceItemV1> {
  return command("get_workspace_item", { itemId });
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
