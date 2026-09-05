import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { applyAuthoringV2Patches, getAuthoringV2 } from "../../api/tauriCommands";
import { applyAuthoringV2Patches as applyLocalPatches, inverseAuthoringPatch } from "../../services/authoringV2Patches";
import { EditorCommandConflictError, compileEditorCommand, type EditorCommandV1 } from "../../exam-canvas/editorCommands";
import type { AuthoringEditorSessionV2, AuthoringPatchV2, IeltsAuthoringIRV2 } from "../../types";

// 编辑保存链（计划 §9.5）：
//   用户输入 -> 前端内存 DS 立即更新 -> 450ms debounce -> apply_editor_commands
//   -> 后端事务加载当前 DS、验证 baseVersion、应用命令 -> 返回已保存版本 -> 显示「已保存」
//
// 用户只看到 正在保存 / 已保存 / 保存失败（计划 §2.4），不看到 revision、hash 或 schema 名。
// 内部仍然维护 revision 做乐观并发，只在真的冲突时才提示。
const SAVE_DEBOUNCE_MS = 450;
const RECOVERY_KEY_PREFIX = "ielts-author-studio.workspace-recovery.v1:";

export type SaveState = "idle" | "saving" | "saved" | "failed" | "conflict";

interface HistoryEntry {
  patch: AuthoringPatchV2;
  inverse: AuthoringPatchV2;
}

export interface CanonicalEditor {
  loading: boolean;
  session?: AuthoringEditorSessionV2;
  loadError?: string;
  draft?: IeltsAuthoringIRV2;
  saveState: SaveState;
  saveMessage?: string;
  /** 未保存的命令数，用于离开页面前 flush。 */
  pendingCount: number;
  canUndo: boolean;
  canRedo: boolean;
  applyCommand: (command: EditorCommandV1) => void;
  applyPatch: (patch: AuthoringPatchV2) => void;
  undo: () => void;
  redo: () => void;
  reload: () => void;
  flush: () => Promise<void>;
}

export function useCanonicalEditor(itemId: string): CanonicalEditor {
  const [session, setSession] = useState<AuthoringEditorSessionV2 | undefined>();
  const [draft, setDraft] = useState<IeltsAuthoringIRV2 | undefined>();
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | undefined>();
  const [saveState, setSaveState] = useState<SaveState>("idle");
  const [saveMessage, setSaveMessage] = useState<string | undefined>();
  const [pendingCount, setPendingCount] = useState(0);
  const [historyDepth, setHistoryDepth] = useState({ undo: 0, redo: 0 });
  const [reloadTick, setReloadTick] = useState(0);

  const draftRef = useRef<IeltsAuthoringIRV2 | undefined>(undefined);
  const revisionRef = useRef(0);
  const pendingRef = useRef<AuthoringPatchV2[]>([]);
  const undoStack = useRef<HistoryEntry[]>([]);
  const redoStack = useRef<HistoryEntry[]>([]);
  const timer = useRef<number | undefined>(undefined);
  const inFlight = useRef<Promise<void> | undefined>(undefined);

  const recoveryKey = useMemo(() => `${RECOVERY_KEY_PREFIX}${itemId}`, [itemId]);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setLoadError(undefined);
    getAuthoringV2(itemId)
      .then((loaded) => {
        if (cancelled) return;
        setSession(loaded);
        setDraft(loaded.authoring);
        draftRef.current = loaded.authoring;
        revisionRef.current = loaded.revision;
        pendingRef.current = [];
        undoStack.current = [];
        redoStack.current = [];
        setPendingCount(0);
        setHistoryDepth({ undo: 0, redo: 0 });
        setSaveState("idle");
        setSaveMessage(undefined);
      })
      .catch((error) => {
        if (!cancelled) setLoadError(error instanceof Error ? error.message : String(error));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [itemId, reloadTick]);

  const persist = useCallback(async () => {
    if (inFlight.current) {
      await inFlight.current;
      return;
    }
    const patches = pendingRef.current;
    if (!patches.length) return;
    pendingRef.current = [];
    setPendingCount(0);
    setSaveState("saving");
    setSaveMessage(undefined);
    const run = (async () => {
      try {
        // library item id 与 job id 相同（findings F12），因此可以直接透传。
        const result = await applyAuthoringV2Patches({ jobId: itemId, baseRevision: revisionRef.current, patches });
        revisionRef.current = result.revision;
        setSession(result);
        setSaveState("saved");
        window.localStorage.removeItem(recoveryKey);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        // 保存失败时把命令放回队列头部，避免用户的输入被静默丢弃。
        pendingRef.current = [...patches, ...pendingRef.current];
        setPendingCount(pendingRef.current.length);
        if (message.includes("revision_conflict")) {
          setSaveState("conflict");
          setSaveMessage("这道题在别处也被改过。重新加载后再修改，避免覆盖对方的修改。");
        } else {
          setSaveState("failed");
          setSaveMessage("保存失败，修改仍保留在本页面。可以稍后重试或重新加载。");
        }
        // 崩溃恢复：把未保存命令留在本地，重开工作区时可以重放。
        try {
          window.localStorage.setItem(recoveryKey, JSON.stringify({
            itemId,
            baseRevision: revisionRef.current,
            updatedAt: new Date().toISOString(),
            patches: pendingRef.current
          }));
        } catch {
          // localStorage 不可用时不影响当前会话，只是失去崩溃恢复。
        }
      } finally {
        inFlight.current = undefined;
      }
    })();
    inFlight.current = run;
    await run;
  }, [itemId, recoveryKey]);

  const schedule = useCallback(() => {
    if (timer.current !== undefined) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      timer.current = undefined;
      void persist();
    }, SAVE_DEBOUNCE_MS);
  }, [persist]);

  const enqueue = useCallback((patch: AuthoringPatchV2, recordHistory: boolean) => {
    const current = draftRef.current;
    if (!current) return;
    let next: IeltsAuthoringIRV2;
    let inverse: AuthoringPatchV2 | undefined;
    try {
      if (recordHistory) inverse = inverseAuthoringPatch(current, patch);
      next = applyLocalPatches(current, [patch]);
    } catch (error) {
      setSaveState("failed");
      setSaveMessage(error instanceof Error ? error.message : String(error));
      return;
    }
    draftRef.current = next;
    setDraft(next);
    if (recordHistory && inverse) {
      undoStack.current = [...undoStack.current, { patch, inverse }];
      redoStack.current = [];
    }
    pendingRef.current = [...pendingRef.current, patch];
    setPendingCount(pendingRef.current.length);
    setHistoryDepth({ undo: undoStack.current.length, redo: redoStack.current.length });
    schedule();
  }, [schedule]);

  const applyCommand = useCallback((command: EditorCommandV1) => {
    const current = draftRef.current;
    if (!current) return;
    try {
      enqueue(compileEditorCommand(command, current), true);
    } catch (error) {
      if (error instanceof EditorCommandConflictError) {
        setSaveState("conflict");
        setSaveMessage("这段文字已经变化过，请重新打开这道题再修改。");
        return;
      }
      setSaveState("failed");
      setSaveMessage(error instanceof Error ? error.message : String(error));
    }
  }, [enqueue]);

  const undo = useCallback(() => {
    const entry = undoStack.current.at(-1);
    if (!entry) return;
    undoStack.current = undoStack.current.slice(0, -1);
    enqueue(entry.inverse, false);
    redoStack.current = [...redoStack.current, entry];
    setHistoryDepth({ undo: undoStack.current.length, redo: redoStack.current.length });
  }, [enqueue]);

  const redo = useCallback(() => {
    const entry = redoStack.current.at(-1);
    if (!entry) return;
    redoStack.current = redoStack.current.slice(0, -1);
    enqueue(entry.patch, false);
    undoStack.current = [...undoStack.current, entry];
    setHistoryDepth({ undo: undoStack.current.length, redo: redoStack.current.length });
  }, [enqueue]);

  // 离开页面或关闭窗口前 flush，避免最后一次输入停在 debounce 里。
  useEffect(() => {
    const flushNow = () => {
      if (timer.current !== undefined) {
        window.clearTimeout(timer.current);
        timer.current = undefined;
      }
      void persist();
    };
    window.addEventListener("beforeunload", flushNow);
    window.addEventListener("hashchange", flushNow);
    return () => {
      window.removeEventListener("beforeunload", flushNow);
      window.removeEventListener("hashchange", flushNow);
      flushNow();
    };
  }, [persist]);

  return {
    loading,
    loadError,
    draft,
    saveState,
    saveMessage,
    pendingCount,
    canUndo: historyDepth.undo > 0,
    canRedo: historyDepth.redo > 0,
    applyCommand,
    applyPatch: (patch) => enqueue(patch, true),
    undo,
    redo,
    reload: () => setReloadTick((value) => value + 1),
    flush: persist,
    session
  };
}
