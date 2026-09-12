import { useCallback, useEffect, useRef, useState } from "react";
import { applyEditorCommands, getWorkspaceItem } from "../../api/workspaceClient";
import { applyAuthoringV2Patches as applyLocalPatches, inverseAuthoringPatch } from "../../services/authoringV2Patches";
import { EditorCommandConflictError, compileEditorCommand, type EditorCommandV1 } from "../../exam-canvas/editorCommands";
import { toUserFacingError } from "../../utils/userFacingError";
import type { AuthoringPatchV2, IeltsAuthoringIRV2 } from "../../types";

const SAVE_DEBOUNCE_MS = 450;
const RECOVERY_KEY_PREFIX = "ielts-author-studio.workspace-recovery.v1:";
export type SaveState = "idle" | "saving" | "saved" | "failed" | "conflict";

/** 文案分层（计划 §9.10）：机器码不进入正文，普通用户只看到可操作的人话。
 *  结构操作自身抛出的中文提示（如「这个选项已用作本题答案」）原样透传。 */
function describeEditError(error: unknown): string {
  if (error instanceof EditorCommandConflictError) return "这段内容已被改动过，请刷新后重新编辑。";
  return toUserFacingError(error, "这次修改没有生效。请刷新后重试，或先处理已提示的问题。").userMessage;
}

interface HistoryEntry { patch: AuthoringPatchV2; inverse: AuthoringPatchV2 }
interface SaveBatch {
  itemId: string;
  baseVersion: number;
  requestId: string;
  commands: AuthoringPatchV2[];
  title?: string;
}
interface RecoveryDraft {
  draft: IeltsAuthoringIRV2;
  version: number;
  title?: string;
  pending: AuthoringPatchV2[];
  pendingTitle?: string;
  batch?: SaveBatch;
}

export interface CanonicalEditor {
  loading: boolean;
  loadError?: string;
  draft?: IeltsAuthoringIRV2;
  saveState: SaveState;
  saveMessage?: string;
  pendingCount: number;
  canUndo: boolean;
  canRedo: boolean;
  title?: string;
  setTitle: (title: string) => void;
  applyCommand: (command: EditorCommandV1) => void;
  applyPatch: (patch: AuthoringPatchV2) => void;
  undo: () => void;
  redo: () => void;
  reload: () => void;
  flush: () => Promise<void>;
}

export function useCanonicalEditor(itemId: string): CanonicalEditor {
  const [draft, setDraft] = useState<IeltsAuthoringIRV2>();
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string>();
  const [saveState, setSaveState] = useState<SaveState>("idle");
  const [saveMessage, setSaveMessage] = useState<string>();
  const [pendingCount, setPendingCount] = useState(0);
  const [historyDepth, setHistoryDepth] = useState({ undo: 0, redo: 0 });
  const [reloadTick, setReloadTick] = useState(0);
  const [title, setTitleState] = useState<string>();
  const draftRef = useRef<IeltsAuthoringIRV2 | undefined>(undefined);
  const titleRef = useRef<string | undefined>(undefined);
  const versionRef = useRef(0);
  const pendingRef = useRef<AuthoringPatchV2[]>([]);
  const pendingTitleRef = useRef<string | undefined>(undefined);
  const batchRef = useRef<SaveBatch | undefined>(undefined);
  const undoStack = useRef<HistoryEntry[]>([]);
  const redoStack = useRef<HistoryEntry[]>([]);
  const timer = useRef<number | undefined>(undefined);
  const inFlight = useRef<Promise<void> | undefined>(undefined);
  const recoveryKey = `${RECOVERY_KEY_PREFIX}${itemId}`;

  const checkpoint = useCallback(() => {
    const count = pendingRef.current.length + (pendingTitleRef.current === undefined ? 0 : 1)
      + (batchRef.current ? batchRef.current.commands.length + (batchRef.current.title === undefined ? 0 : 1) : 0);
    setPendingCount(count);
    try {
      if (count && draftRef.current) {
        const recovery: RecoveryDraft = {
          draft: draftRef.current, version: versionRef.current, title: titleRef.current,
          pending: pendingRef.current, pendingTitle: pendingTitleRef.current, batch: batchRef.current
        };
        localStorage.setItem(recoveryKey, JSON.stringify(recovery));
      } else {
        localStorage.removeItem(recoveryKey);
      }
    } catch { /* A storage failure must not turn a committed save into a failed save. */ }
  }, [recoveryKey]);

  const persist = useCallback((): Promise<void> => {
    if (timer.current !== undefined) window.clearTimeout(timer.current);
    timer.current = undefined;
    if (inFlight.current) return inFlight.current;
    const run = async () => {
      try {
        while (batchRef.current || pendingRef.current.length || pendingTitleRef.current !== undefined) {
          if (!batchRef.current) {
            batchRef.current = {
              itemId, baseVersion: versionRef.current, requestId: crypto.randomUUID(),
              commands: pendingRef.current, title: pendingTitleRef.current
            };
            pendingRef.current = [];
            pendingTitleRef.current = undefined;
          }
          checkpoint();
          setSaveState("saving");
          setSaveMessage(undefined);
          const result = await applyEditorCommands(batchRef.current);
          versionRef.current = result.editVersion;
          batchRef.current = undefined;
          checkpoint();
        }
        setSaveState("saved");
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        const conflict = message.includes("EDIT_VERSION_CONFLICT");
        setSaveState(conflict ? "conflict" : "failed");
        setSaveMessage(conflict
          ? "这道题在别处也被改过。未保存的修改已保留，请先处理保存冲突。"
          : "保存失败，修改已保留。请稍后重试。");
        checkpoint();
        throw error;
      }
    };
    // Start in a microtask so even an immediate failure clears the assigned promise.
    inFlight.current = Promise.resolve().then(run).finally(() => { inFlight.current = undefined; });
    return inFlight.current;
  }, [itemId, checkpoint]);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setLoadError(undefined);
    getWorkspaceItem(itemId).then((workspace) => {
      if (cancelled) return;
      if (!workspace.ds) throw new Error("ITEM_DS_NOT_SEEDED");
      let loaded = workspace.ds as unknown as IeltsAuthoringIRV2;
      let loadedTitle = workspace.item.title;
      versionRef.current = workspace.editVersion;
      try {
        const saved = localStorage.getItem(recoveryKey);
        const recovery: RecoveryDraft | undefined = saved ? JSON.parse(saved) : undefined;
        if (recovery?.draft && Array.isArray(recovery.pending)) {
          loaded = recovery.draft;
          loadedTitle = recovery.title ?? loadedTitle;
          versionRef.current = recovery.version;
          pendingRef.current = recovery.pending;
          pendingTitleRef.current = recovery.pendingTitle;
          batchRef.current = recovery.batch;
          setSaveState("failed");
          setSaveMessage("已恢复未保存的修改，请重试保存。");
        }
      } catch { /* Ignore an unreadable local recovery record. */ }
      draftRef.current = loaded;
      titleRef.current = loadedTitle;
      setDraft(loaded);
      setTitleState(loadedTitle);
      checkpoint();
    }).catch((error) => {
      if (!cancelled) setLoadError(error instanceof Error ? error.message : String(error));
    }).finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [itemId, reloadTick, recoveryKey, checkpoint]);

  const schedule = useCallback(() => {
    checkpoint();
    setSaveState("saving");
    if (timer.current !== undefined) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => { void persist().catch(() => {}); }, SAVE_DEBOUNCE_MS);
  }, [persist, checkpoint]);

  const setTitle = useCallback((next: string) => {
    const trimmed = next.trim();
    if (!trimmed || trimmed === titleRef.current) return;
    titleRef.current = trimmed;
    setTitleState(trimmed);
    if (draftRef.current) {
      draftRef.current = { ...draftRef.current, exam: { ...draftRef.current.exam, title: trimmed } };
      setDraft(draftRef.current);
    }
    pendingTitleRef.current = trimmed;
    schedule();
  }, [schedule]);

  const enqueue = useCallback((patch: AuthoringPatchV2, recordHistory: boolean) => {
    const current = draftRef.current;
    if (!current) return;
    try {
      const inverse = recordHistory ? inverseAuthoringPatch(current, patch) : undefined;
      const next = applyLocalPatches(current, [patch]);
      draftRef.current = next;
      setDraft(next);
      if (inverse) {
        undoStack.current.push({ patch, inverse });
        redoStack.current = [];
      }
      pendingRef.current.push(patch);
      setHistoryDepth({ undo: undoStack.current.length, redo: redoStack.current.length });
      schedule();
    } catch (error) {
      setSaveState("failed");
      setSaveMessage(describeEditError(error));
    }
  }, [schedule]);

  const applyCommand = useCallback((command: EditorCommandV1) => {
    if (!draftRef.current) return;
    try { enqueue(compileEditorCommand(command, draftRef.current), true); }
    catch (error) {
      setSaveState(error instanceof EditorCommandConflictError ? "conflict" : "failed");
      setSaveMessage(describeEditError(error));
    }
  }, [enqueue]);

  const undo = useCallback(() => {
    const entry = undoStack.current.pop();
    if (!entry) return;
    enqueue(entry.inverse, false);
    redoStack.current.push(entry);
    setHistoryDepth({ undo: undoStack.current.length, redo: redoStack.current.length });
  }, [enqueue]);
  const redo = useCallback(() => {
    const entry = redoStack.current.pop();
    if (!entry) return;
    enqueue(entry.patch, false);
    undoStack.current.push(entry);
    setHistoryDepth({ undo: undoStack.current.length, redo: redoStack.current.length });
  }, [enqueue]);

  useEffect(() => {
    const flushNow = () => { if (draftRef.current) { checkpoint(); void persist().catch(() => {}); } };
    window.addEventListener("beforeunload", flushNow);
    return () => {
      window.removeEventListener("beforeunload", flushNow);
      flushNow();
    };
  }, [checkpoint, persist]);

  const reload = useCallback(() => {
    void persist().then(() => setReloadTick((value) => value + 1)).catch(() => {});
  }, [persist]);
  return {
    loading, loadError, draft, saveState, saveMessage, pendingCount, title, setTitle,
    canUndo: historyDepth.undo > 0, canRedo: historyDepth.redo > 0,
    applyCommand, applyPatch: (patch) => enqueue(patch, true), undo, redo,
    reload,
    flush: persist
  };
}
