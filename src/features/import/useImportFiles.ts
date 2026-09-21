import { useCallback, useRef, useState } from "react";
import { listLlmProfiles } from "../../api/tauriCommands";
import { importFiles as enqueueFiles, type ImportModality } from "../../api/processingClient";
import type { PickedPath } from "../../api/desktopDialogs";
import { toUserFacingError } from "../../utils/userFacingError";
import { buildRow, type LibraryRowV1 } from "../library/libraryTypes";

export function formatImportError(error: unknown): string {
  return toUserFacingError(error, "文件未能导入，请稍后重试。").userMessage;
}
export interface ImportRejection { name: string; reason: string }
export interface ImportBatchResult { rows: LibraryRowV1[]; rejected: ImportRejection[] }

export function useImportFiles(onRowsChanged: () => void) {
  const [busy, setBusy] = useState(false);
  const [stageMessage, setStageMessage] = useState<string>();
  const [error, setError] = useState<string>();
  const changed = useRef(onRowsChanged);
  changed.current = onRowsChanged;
  const importFiles = useCallback(async (files: PickedPath[], options: { cloudEnabled?: boolean; modality?: ImportModality } = {}): Promise<ImportBatchResult> => {
    if (!files.length) return { rows: [], rejected: [] };
    setBusy(true);
    setError(undefined);
    setStageMessage(`正在导入 ${files.length} 份文件`);
    try {
      const profiles = options.cloudEnabled === false ? [] : await listLlmProfiles().catch(() => []);
      const cloudProfileId = profiles.find((profile) => profile.enabled && profile.profileId !== "profile-local-placeholder")?.profileId;
      const cloudEnabled = options.cloudEnabled ?? Boolean(cloudProfileId);
      const result = await enqueueFiles({ files, cloudEnabled, cloudProfileId, modality: options.modality });
      const rows = result.created.map(({ itemId, title }) => ({
        ...buildRow(itemId, undefined, undefined),
        title,
        modality: options.modality === "listening" ? ("listening" as const) : ("reading" as const)
      }));
      if (result.rejected.length) setError(`${result.rejected.length} 份文件未能导入。`);
      changed.current();
      return { rows, rejected: result.rejected.map(({ name, reason }) => ({ name, reason: toUserFacingError(reason, "文件未能导入。").userMessage })) };
    } catch (cause) {
      const reason = formatImportError(cause);
      setError(reason);
      return { rows: [], rejected: files.map(({ name }) => ({ name, reason })) };
    } finally {
      setBusy(false);
      setStageMessage(undefined);
    }
  }, []);
  return { busy, stageMessage, error, importFiles, clearError: () => setError(undefined) };
}
