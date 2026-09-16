import { useCallback, useEffect, useRef, useState } from "react";
import { applyEditorCommands, getWorkspaceItem } from "../../api/workspaceClient";
import { applyAuthoringV2Patches as applyLocalPatches, inverseAuthoringPatch } from "../../services/authoringV2Patches";
import { conflictRecoveryNotice, rebasePendingPatches } from "./conflictRecovery";
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
  /** 已保存的权威稿版本号（用于学生预览显示 revision 状态）。 */
  version: number;
  canUndo: boolean;
  canRedo: boolean;
  title?: string;
  setTitle: (title: string) => void;
  applyCommand: (command: EditorCommandV1) => void;
  applyPatch: (patch: AuthoringPatchV2) => void;
  undo: () => void;
  redo: () => void;
  reload: () => void;
  /** 版本冲突后：以服务端最新版本为基线重放未保存修改并保存。 */
  recoverFromConflict: () => Promise<void>;
  /** 放弃本地未保存的修改，以服务端最新版本重新加载。 */
  discardLocalChanges: () => void;
  conflictRecovering: boolean;
  /**
   * 冲突恢复的结果提示（例如「另有 N 项未能应用」）。
   *
   * 与 `saveMessage` 分开：`saveMessage` 是保存状态机的瞬态文案，每次保存循环
   * 都会被清空；「有改动没能应用」是**必须被看到**的信息，不能被随后的
   * 「已保存」覆盖掉，否则用户会以为全部改动都落盘了。
   */
  saveNotice?: string;
  dismissSaveNotice: () => void;
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
  const [conflictRecovering, setConflictRecovering] = useState(false);
  const [saveNotice, setSaveNotice] = useState<string>();
  const draftRef = useRef<IeltsAuthoringIRV2 | undefined>(undefined);
  const titleRef = useRef<string | undefined>(undefined);
  const versionRef = useRef(0);
  // 已保存的权威稿版本号。用 state 暴露给界面（预览要显示当前版本），
  // ref 仍是保存事务的读取源，二者始终同步写入。
  const [editVersion, setEditVersion] = useState(0);
  const setVersion = useCallback((next: number) => { versionRef.current = next; setEditVersion(next); }, []);
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
          setVersion(result.editVersion);
          batchRef.current = undefined;
          checkpoint();
        }
        setSaveState("saved");
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        const conflict = message.includes("EDIT_VERSION_CONFLICT");
        setSaveState(conflict ? "conflict" : "failed");
        setSaveMessage(conflict
          ? "这道题在别处也被改过。本地修改仍保留，可「重试保存」重新应用，或「放弃本地修改」以最新版本重新加载。"
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
      setVersion(workspace.editVersion);
      try {
        const saved = localStorage.getItem(recoveryKey);
        const recovery: RecoveryDraft | undefined = saved ? JSON.parse(saved) : undefined;
        if (recovery?.draft && Array.isArray(recovery.pending)) {
          loaded = recovery.draft;
          loadedTitle = recovery.title ?? loadedTitle;
          setVersion(recovery.version);
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
    // 这里**不能**清 `saveNotice`。工作区会在识别事件到来时自动 reload（`pendingCount` 归零后），
    // 而冲突恢复保存成功后恰好使 `pendingCount` 归零——若在这里清提示，
    // 「有改动未能应用」就会被一条后台事件抹掉，又回到静默丢失。
    // 提示只在用户主动放弃本地修改或手动关闭时失效。
    void persist().then(() => setReloadTick((value) => value + 1)).catch((error) => {
      // 保存失败/冲突时不能静默什么都不做：用户点了「刷新」必须看到原因与出路，
      // 否则编辑器会停在一个既存不上、也刷不掉的死路上。
      const message = error instanceof Error ? error.message : String(error);
      if (!message.includes("EDIT_VERSION_CONFLICT")) {
        setSaveMessage(toUserFacingError(error, "刷新前保存失败，请重试。").userMessage);
      }
    });
  }, [persist]);

  /** 仍未被服务端接受的命令，保持原始顺序（先已提交失败的批次，再待发送队列）。 */
  const outstandingCommands = useCallback((): AuthoringPatchV2[] => {
    const batched = batchRef.current ? batchRef.current.commands : [];
    return [...batched, ...pendingRef.current];
  }, []);

  /** 放弃本地未保存的修改，以服务端最新版本重新加载。 */
  const discardLocalChanges = useCallback(() => {
    try { localStorage.removeItem(recoveryKey); } catch { /* Ignore storage failures. */ }
    batchRef.current = undefined;
    pendingRef.current = [];
    pendingTitleRef.current = undefined;
    undoStack.current = [];
    redoStack.current = [];
    setVersion(0);
    setHistoryDepth({ undo: 0, redo: 0 });
    setPendingCount(0);
    setSaveState("idle");
    setSaveMessage(undefined);
    setSaveNotice(undefined);
    setReloadTick((value) => value + 1);
  }, [recoveryKey]);

  const dismissSaveNotice = useCallback(() => { setSaveNotice(undefined); }, []);

  /** 版本冲突后的出路：以服务端最新版本为基线重放本地未保存修改，再保存。 */
  const recoverFromConflict = useCallback(async (): Promise<void> => {
    if (conflictRecovering) return;
    setConflictRecovering(true);
    try {
      const outstanding = outstandingCommands();
      const localTitle = batchRef.current?.title ?? pendingTitleRef.current;
      const workspace = await getWorkspaceItem(itemId);
      if (!workspace.ds) throw new Error("ITEM_DS_NOT_SEEDED");
      const base = workspace.ds as unknown as IeltsAuthoringIRV2;
      // 逐条重放：某条补丁因原文已改动而无法应用时停下，已应用的部分照常保存，
      // 未应用的部分明确告知用户——绝不静默丢弃本地修改。
      const { rebased, applied, dropped } = rebasePendingPatches(base, outstanding);
      draftRef.current = rebased;
      setDraft(rebased);
      setVersion(workspace.editVersion);
      batchRef.current = undefined;
      pendingRef.current = applied;
      pendingTitleRef.current = localTitle && localTitle !== workspace.item.title ? localTitle : undefined;
      titleRef.current = workspace.item.title;
      setTitleState(workspace.item.title);
      // 撤销栈建立在旧基线上，重放后不再可靠。
      undoStack.current = [];
      redoStack.current = [];
      setHistoryDepth({ undo: 0, redo: 0 });
      checkpoint();
      setSaveState("idle");
      // 只在确有补丁未能应用时写入，且**绝不自动清除**。
      // 这条提示说的是「有修改永久没有保存」——用户没点关闭之前，任何自动清除
      // （包括后续一次 dropped === 0 的恢复、以及后台识别事件触发的 reload）
      // 都可能把尚未看到的数据丢失信息抹掉。清除只发生在用户主动关闭或放弃本地修改时。
      if (dropped > 0) {
        setSaveNotice(conflictRecoveryNotice(applied.length, dropped));
      }
      await persist();
    } catch (error) {
      setSaveState("failed");
      setSaveMessage(toUserFacingError(error, "重试保存失败，可放弃本地修改后重新加载。").userMessage);
    } finally {
      setConflictRecovering(false);
    }
  }, [checkpoint, conflictRecovering, itemId, outstandingCommands, persist]);

  return {
    loading, loadError, draft, saveState, saveMessage, pendingCount, title, setTitle,
    version: editVersion,
    canUndo: historyDepth.undo > 0, canRedo: historyDepth.redo > 0,
    applyCommand, applyPatch: (patch) => enqueue(patch, true), undo, redo,
    reload, recoverFromConflict, discardLocalChanges, conflictRecovering,
    saveNotice, dismissSaveNotice,
    flush: persist
  };
}
