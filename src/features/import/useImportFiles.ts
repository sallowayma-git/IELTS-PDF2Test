import { useCallback, useRef, useState } from "react";
import { listLlmProfiles } from "../../api/tauriCommands";
import { importFiles as enqueueFiles, type ImportModality } from "../../api/processingClient";
import { bindListeningAudio } from "../../api/listeningAudioClient";
import type { PickedPath } from "../../api/desktopDialogs";
import { toUserFacingError } from "../../utils/userFacingError";
import { buildRow, type LibraryRowV1 } from "../library/libraryTypes";
import { splitImportPlan, type ListeningImportDecision } from "./listeningAudioPlan";

export function formatImportError(error: unknown): string {
  return toUserFacingError(error, "文件未能导入，请稍后重试。").userMessage;
}
export interface ImportRejection { name: string; reason: string }
export interface ImportBatchResult { rows: LibraryRowV1[]; rejected: ImportRejection[] }

export interface ImportOptions {
  cloudEnabled?: boolean;
  modality?: ImportModality;
  /** Per source path, from the listening dialog. Paths without a decision import as reading. */
  decisions?: Record<string, ListeningImportDecision>;
}

export function useImportFiles(onRowsChanged: () => void) {
  const [busy, setBusy] = useState(false);
  const [stageMessage, setStageMessage] = useState<string>();
  const [error, setError] = useState<string>();
  const changed = useRef(onRowsChanged);
  changed.current = onRowsChanged;
  const importFiles = useCallback(async (files: PickedPath[], options: ImportOptions = {}): Promise<ImportBatchResult> => {
    if (!files.length) return { rows: [], rejected: [] };
    setBusy(true);
    setError(undefined);
    setStageMessage(`正在导入 ${files.length} 份文件`);
    const rows: LibraryRowV1[] = [];
    const rejected: ImportRejection[] = [];
    try {
      const profiles = options.cloudEnabled === false ? [] : await listLlmProfiles().catch(() => []);
      const cloudProfileId = profiles.find((profile) => profile.enabled && profile.profileId !== "profile-local-placeholder")?.profileId;
      const cloudEnabled = options.cloudEnabled ?? Boolean(cloudProfileId);

      const enqueue = async (batch: PickedPath[], modality: ImportModality) => {
        const result = await enqueueFiles({ files: batch, cloudEnabled, cloudProfileId, modality });
        rejected.push(...result.rejected.map(({ name, reason }) => ({ name, reason: toUserFacingError(reason, "文件未能导入。").userMessage })));
        const created = result.created.map(({ itemId, title }) => ({ ...buildRow(itemId, undefined, undefined), title, modality }));
        rows.push(...created);
        return created;
      };

      if (!options.decisions) {
        await enqueue(files, options.modality ?? "reading");
      } else {
        const plan = splitImportPlan(files, options.decisions);
        if (plan.reading.length) await enqueue(plan.reading, "reading");
        for (const { file, audio } of plan.listening) {
          const [item] = await enqueue([file], "listening");
          if (!item) continue;
          for (const assignment of audio) {
            setStageMessage(`正在保存「${item.title}」的 Part ${assignment.partOrdinal} 音频`);
            try {
              await bindListeningAudio(item.id, assignment.partOrdinal, assignment.path);
            } catch (cause) {
              rejected.push({
                name: assignment.name,
                reason: `音频未能添加到「${item.title}」，可在工作区「添加音频」补充。${toUserFacingError(cause, "").userMessage}`
              });
            }
          }
        }
      }
      if (rejected.length) setError(`${rejected.length} 个文件未能导入或绑定。`);
      changed.current();
      return { rows, rejected };
    } catch (cause) {
      const reason = formatImportError(cause);
      setError(reason);
      const createdNames = new Set(rows.map((row) => row.title));
      return { rows, rejected: [...rejected, ...files.filter(({ name }) => !createdNames.has(name)).map(({ name }) => ({ name, reason }))] };
    } finally {
      setBusy(false);
      setStageMessage(undefined);
    }
  }, []);
  return { busy, stageMessage, error, importFiles, clearError: () => setError(undefined) };
}
