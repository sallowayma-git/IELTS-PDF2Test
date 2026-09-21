import { useCallback, useEffect, useRef, useState } from "react";
import { applyEditorCommands, getWorkspaceItem } from "../../api/workspaceClient";
import { applyAuthoringV2Patches as applyLocalPatches, inverseAuthoringPatch } from "../../services/authoringV2Patches";
import {
  conflictRecoveryNotice,
  rebasePendingPatches,
  tryAutoRebase,
  type ConflictRebaseResult
} from "./conflictRecovery";
import {
  decideRemoteVersionAction,
  shouldApplyDeferredRemoteRefresh,
  type DeferredRemoteRefresh
} from "./remoteVersion";
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
  /**
   * 因本地有未保存修改而推迟读取的远端刷新；`undefined` = 没有推迟中的刷新。
   *
   * 界面据此如实告诉用户「云端已更新，保存后会加载最新版本」，而不是让他在保存时
   * 突然撞上冲突、以为是自己操作出错。
   */
  deferredRemoteRefresh?: DeferredRemoteRefresh;
  /**
   * 收到「权威稿版本推进」通知。`incoming` 是事件携带的远端版本号（后端读不到时为
   * `null`）。由工作区在 `processing://item-updated` 到达时调用——**不要**自己判断
   * 「有没有未保存修改再决定是否重拉」，那正是这个 hook 要收口的地方。
   */
  noteRemoteVersion: (incoming: number | null | undefined) => void;
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
  /**
   * 因本地有未保存修改而**推迟**的远端刷新（见 `remoteVersion.ts`）。
   *
   * 必须暴露给界面：用户看到「云端已更新到版本 N」才知道自己这份稿是旧的，
   * 否则保存时突然撞上冲突会显得毫无来由。`undefined` = 没有推迟中的刷新。
   */
  const [deferredRemoteRefresh, setDeferredRemoteRefresh] = useState<DeferredRemoteRefresh>();
  const deferredRemoteRef = useRef<DeferredRemoteRefresh | undefined>(undefined);
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
  /** 本轮冲突是否已经自动重放过一次（成功保存后复位）。只自动一次，避免与后台写入无限追逐。 */
  const autoRebaseTried = useRef(false);
  const recoveryKey = `${RECOVERY_KEY_PREFIX}${itemId}`;

  /**
   * 尚未被服务端接受的改动数量（待发送队列 + 已提交但失败的批次）。
   *
   * 与 `checkpoint` 共用同一份计数：判断「现在能不能安全重拉」必须和界面上
   * `pendingCount` 说的是同一件事，否则会出现「界面显示还有 2 项没保存，但刷新逻辑
   * 认为可以覆盖」这种自相矛盾的状态。
   */
  const countUnsaved = useCallback(() => {
    const batched = batchRef.current
      ? batchRef.current.commands.length + (batchRef.current.title === undefined ? 0 : 1)
      : 0;
    return batched + pendingRef.current.length + (pendingTitleRef.current === undefined ? 0 : 1);
  }, []);

  const checkpoint = useCallback(() => {
    const count = countUnsaved();
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
  }, [recoveryKey, countUnsaved]);

  /** 作废推迟记录（本次读取已经会拿到最新版本，或本地修改被放弃）。 */
  const clearDeferredRemoteRefresh = useCallback(() => {
    deferredRemoteRef.current = undefined;
    setDeferredRemoteRefresh(undefined);
  }, []);

  /**
   * 主动重拉：作废推迟记录并触发一次加载。
   *
   * 顺序不能反——先作废再触发，否则加载完成后那条推迟记录还在，下一次保存又会
   * 触发一次多余的重拉。
   */
  const requestReload = useCallback(() => {
    clearDeferredRemoteRefresh();
    setReloadTick((value) => value + 1);
  }, [clearDeferredRemoteRefresh]);

  /**
   * 保存循环排空后的收尾：若有一次远端变更因为「本地有未保存修改」被推迟，现在补读。
   *
   * 这是「推迟」策略的另一半。只推迟不补读，等于把一次真实的云端修改永久忘掉：
   * 界面会一直停在改动前的结论，直到用户下次手动刷新。
   */
  const applyDeferredRemoteRefreshAfterSave = useCallback(() => {
    if (shouldApplyDeferredRemoteRefresh(deferredRemoteRef.current, versionRef.current)) {
      requestReload();
    }
  }, [requestReload]);

  /**
   * 把一次冲突重放的结果收进编辑器状态（自动重放与「重试保存」共用）。
   * 未能重放的补丁**丢弃而不强写**，并写入需要用户手动关闭的提示。
   */
  const adoptRebase = useCallback((
    latestDs: IeltsAuthoringIRV2,
    latestVersion: number,
    latestTitle: string | undefined,
    rebase: ConflictRebaseResult,
    localTitle: string | undefined
  ) => {
    draftRef.current = rebase.rebased;
    setDraft(rebase.rebased);
    setVersion(latestVersion);
    // 这次读取已经拿到最新版本，推迟记录不再有意义。
    clearDeferredRemoteRefresh();
    batchRef.current = undefined;
    pendingRef.current = rebase.applied;
    pendingTitleRef.current = localTitle && localTitle !== latestTitle ? localTitle : undefined;
    if (latestTitle !== undefined) {
      titleRef.current = latestTitle;
      setTitleState(latestTitle);
    }
    // 撤销栈建立在旧基线上，重放后不再可靠。
    undoStack.current = [];
    redoStack.current = [];
    setHistoryDepth({ undo: 0, redo: 0 });
    checkpoint();
    // 只在确有补丁未能应用时写入，且**绝不自动清除**（见 `conflictRecoveryNotice`）。
    if (rebase.dropped > 0) {
      setSaveNotice(conflictRecoveryNotice(rebase.applied.length, rebase.dropped));
    }
    void latestDs;
  }, [checkpoint, clearDeferredRemoteRefresh, setVersion]);

  const persist = useCallback((): Promise<void> => {
    if (timer.current !== undefined) window.clearTimeout(timer.current);
    timer.current = undefined;
    if (inFlight.current) return inFlight.current;
    const run = async (): Promise<void> => {
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
        autoRebaseTried.current = false;
        setSaveState("saved");
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        const conflict = message.includes("EDIT_VERSION_CONFLICT");
        // 撞上的是云端修复 / 答案页识别自己的写入：自动重放**一次**，不逼用户二选一。
        // 只有冲突里有人工写入、或重放本身失败时，才落到下面的按钮。
        if (conflict && !autoRebaseTried.current) {
          autoRebaseTried.current = true;
          const localTitle = batchRef.current?.title ?? pendingTitleRef.current;
          let latestTitle: string | undefined;
          const outcome = await tryAutoRebase({
            localBase: batchRef.current?.baseVersion ?? versionRef.current,
            outstanding: [...(batchRef.current?.commands ?? []), ...pendingRef.current],
            fetchLatest: async () => {
              const workspace = await getWorkspaceItem(itemId);
              latestTitle = workspace.item.title;
              return {
                ds: workspace.ds as unknown as IeltsAuthoringIRV2,
                editVersion: workspace.editVersion,
                recentEdits: workspace.recentEdits
              };
            }
          });
          if (outcome.kind === "rebased") {
            adoptRebase(outcome.latest.ds, outcome.latest.editVersion, latestTitle, outcome.rebase, localTitle);
            return run();
          }
        }
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
  }, [itemId, checkpoint, adoptRebase]);

  /**
   * 收到「权威稿版本推进」通知时的处置。
   *
   * 判定规则本身在 `remoteVersion.ts`（纯函数、有单测）。这里只负责执行：
   *   - `ignore` → 自己保存引起的回声，什么都不做（重拉会覆盖正在编辑的内容，
   *     并让撤销栈的基线错位）；
   *   - `defer`  → 记录，**不**覆盖用户正在编辑的稿；等保存排空后补读；
   *   - `reload` → 本地干净，直接读最新版本。
   *
   * `incoming` 允许为 `undefined`：后端读不到版本号时会发 `null`，那种情况下
   * 按「可能有变更」保守处理，绝不能因为拿不到版本号就把通知丢掉。
   */
  const noteRemoteVersion = useCallback((incoming: number | null | undefined) => {
    const action = decideRemoteVersionAction({
      incoming,
      current: versionRef.current,
      unsavedCount: countUnsaved()
    });
    if (action === "ignore") return;
    if (action === "reload") {
      requestReload();
      return;
    }
    const version = typeof incoming === "number" && Number.isFinite(incoming) ? incoming : undefined;
    const previous = deferredRemoteRef.current;
    // 连续推迟时保留**最大**版本号：后处理的一条通知版本可能更小，不能让它把
    // 「更靠后的那次变更」这件事冲掉。
    const merged: DeferredRemoteRefresh = {
      version: version === undefined ? previous?.version : Math.max(version, previous?.version ?? version)
    };
    deferredRemoteRef.current = merged;
    setDeferredRemoteRefresh(merged);
  }, [countUnsaved, requestReload]);

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
    timer.current = window.setTimeout(() => {
      // 保存排空后补读被推迟的远端变更（见 `noteRemoteVersion`）。放在这里而不是
      // `persist` 内部：`persist` 也被「刷新」「冲突恢复」调用，那些路径自己会读最新
      // 版本，再补一次就是多余往返。
      void persist().then(applyDeferredRemoteRefreshAfterSave).catch(() => {});
    }, SAVE_DEBOUNCE_MS);
  }, [persist, checkpoint, applyDeferredRemoteRefreshAfterSave]);

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
    // 用户主动刷新：本次读取必然拿到最新版本，推迟记录一并作废（`requestReload`
    // 会清），否则下一次保存后还会再补读一次。
    void persist().then(requestReload).catch((error) => {
      // 保存失败/冲突时不能静默什么都不做：用户点了「刷新」必须看到原因与出路，
      // 否则编辑器会停在一个既存不上、也刷不掉的死路上。
      const message = error instanceof Error ? error.message : String(error);
      if (!message.includes("EDIT_VERSION_CONFLICT")) {
        setSaveMessage(toUserFacingError(error, "刷新前保存失败，请重试。").userMessage);
      }
    });
  }, [persist, requestReload]);

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
    // 本地修改被放弃，这次加载本身就是「读最新版本」：推迟记录一并作废。
    clearDeferredRemoteRefresh();
    setReloadTick((value) => value + 1);
  }, [recoveryKey, clearDeferredRemoteRefresh]);

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
      adoptRebase(base, workspace.editVersion, workspace.item.title, rebasePendingPatches(base, outstanding), localTitle);
      setSaveState("idle");
      await persist();
    } catch (error) {
      setSaveState("failed");
      setSaveMessage(toUserFacingError(error, "重试保存失败，可放弃本地修改后重新加载。").userMessage);
    } finally {
      setConflictRecovering(false);
    }
  }, [adoptRebase, conflictRecovering, itemId, outstandingCommands, persist]);

  return {
    loading, loadError, draft, saveState, saveMessage, pendingCount, title, setTitle,
    version: editVersion,
    canUndo: historyDepth.undo > 0, canRedo: historyDepth.redo > 0,
    applyCommand, applyPatch: (patch) => enqueue(patch, true), undo, redo,
    reload, recoverFromConflict, discardLocalChanges, conflictRecovering,
    saveNotice, dismissSaveNotice,
    deferredRemoteRefresh, noteRemoteVersion,
    flush: persist
  };
}
