import { useEffect, useMemo, useRef, useState } from "react";
import { go, workspacePath, type LibraryIntent } from "../../app/router";
import { getCurrentPaper, type CurrentPaperV1 } from "../../api/paperClient";
import { describePublishResult, publishItems } from "../../api/publishClient";
import { renameLibraryItem } from "../../api/workspaceClient";
import { readAppSettings, writeAppSettings } from "../settings/appSettings";
import { chooseExportDirectory } from "../../api/desktopDialogs";
import { toUserFacingError } from "../../utils/userFacingError";
import { CurrentPaperCard } from "./CurrentPaperCard";
import { LibraryItemList } from "./LibraryItemList";
import { LibraryPaperTable } from "./LibraryPaperTable";
import { PaperComposerBar, type PaperConfirmation } from "./PaperComposerBar";
import { useLibraryStore } from "./libraryStore";
import { libraryRows, matchesSearch } from "./libraryTypes";
import {
  buildPaperSpec,
  EMPTY_PAPER_SELECTION,
  isReadingPart,
  reconcilePaperSelection,
  selectedPaperItemIds,
  toggleListeningItem,
  toggleReadingItem,
  type PaperSelection,
  type ReadingPart
} from "./paperSelection";
import { WritingPanel } from "./WritingPanel";

// 题库一级页：只显示「可发布」与「已发布」。待检查、识别中、失败的题在「导入」页。
// 唯一的发布动作是「发布为当前试卷」（替换语义）。回收站是右上角的次级入口。

/** 失败提示经用户文案层收敛；机器码只进日志。 */
function describeLibraryActionError(error: unknown, fallback: string): string {
  const facing = toUserFacingError(error, fallback);
  console.error("[library]", facing.internalDetail);
  return facing.userMessage;
}

/** 确认框里列出的「将被替换」的当前试卷题名（阅读 + 听力）。 */
export function replacedPaperTitles(paper: CurrentPaperV1 | null | undefined): string[] {
  if (!paper) return [];
  return [...paper.reading, ...paper.listening].map((entry) => entry.title);
}

export function LibraryPage({ intent, pickItemId }: { intent?: LibraryIntent; pickItemId?: string }) {
  const store = useLibraryStore();
  // 编辑器「去组卷发布」带来的预选题：每个 id 只应用一次，避免刷新时反复勾选。
  const appliedPick = useRef<string | undefined>(undefined);
  // 组卷发布的防重入：在 await 之前就占住，双击或快速重复点击只会发出一次请求。
  const paperBusy = useRef(false);
  const [search, setSearch] = useState("");
  const [showTrash, setShowTrash] = useState(false);
  const [notice, setNotice] = useState<string | undefined>(
    intent === "publish" ? "在下面勾选三篇阅读（每个 Part 一篇），然后发布为当前试卷。" : undefined
  );
  const [paperSelection, setPaperSelection] = useState<PaperSelection>(EMPTY_PAPER_SELECTION);
  const [paperPublishing, setPaperPublishing] = useState(false);
  const [paperMessage, setPaperMessage] = useState<string | undefined>();
  const [confirmation, setConfirmation] = useState<PaperConfirmation | undefined>();
  const [destination, setDestination] = useState<string | undefined>(() => readAppSettings().nasDestination);
  // undefined = 还在读取；null = NAS 上还没有当前试卷。
  const [currentPaper, setCurrentPaper] = useState<CurrentPaperV1 | null | undefined>(undefined);
  const [currentPaperError, setCurrentPaperError] = useState<string | undefined>();
  const [paperVersion, setPaperVersion] = useState(0);

  // 当前试卷（NAS 上的指针）：打开页面时读取，发布后重读。
  useEffect(() => {
    if (!destination) {
      setCurrentPaper(undefined);
      return;
    }
    let cancelled = false;
    getCurrentPaper(destination)
      .then((paper) => {
        if (cancelled) return;
        setCurrentPaperError(undefined);
        setCurrentPaper(paper);
      })
      .catch((error) => {
        if (cancelled) return;
        console.error("[library] current paper read failed", error);
        setCurrentPaperError(describeLibraryActionError(error, "读取当前试卷失败，请稍后再试。"));
        setCurrentPaper(undefined);
      });
    return () => {
      cancelled = true;
    };
  }, [destination, paperVersion]);

  // 编辑器「去组卷发布」：把这道题预选进它所在 Part 的组卷栏；没有 Part 就提示先设 Part。
  useEffect(() => {
    if (!pickItemId || appliedPick.current === pickItemId) return;
    if (!store.rows.length) return;
    appliedPick.current = pickItemId;
    const row = store.rows.find((candidate) => candidate.id === pickItemId && !candidate.inTrash);
    if (!row) {
      setNotice("这道题不在题库里，无法加入组卷。");
      return;
    }
    if (row.modality === "reading" && isReadingPart(row.part)) {
      const part = row.part;
      setPaperSelection((current) =>
        current.reading[part] === row.id ? current : toggleReadingItem(current, part, row.id)
      );
      setNotice(`已把「${row.title}」放进 ${part} 的组卷栏；选齐三篇阅读后即可发布为当前试卷。`);
      return;
    }
    if (row.modality === "listening" && row.part) {
      const part = row.part;
      setPaperSelection((current) =>
        current.listening.some((entry) => entry.itemId === row.id)
          ? current
          : toggleListeningItem(current, row.id, part)
      );
      setNotice(`已把「${row.title}」加入听力组卷（${part}）。`);
      return;
    }
    setNotice(`「${row.title}」还没有 Part，请先在下面的表格里为它设置 P1/P2/P3（阅读）或 Part 1–4（听力），再去组卷。`);
  }, [pickItemId, store.rows]);

  // 组卷选择与题库对齐：题被删除、或 Part 被改到别的槽位，就从选择里去掉。
  useEffect(() => {
    const candidates = store.rows
      .filter((row) => !row.inTrash)
      .map((row) => ({ id: row.id, modality: row.modality, part: row.part }));
    setPaperSelection((current) => {
      const next = reconcilePaperSelection(current, candidates);
      return JSON.stringify(next) === JSON.stringify(current) ? current : next;
    });
  }, [store.rows]);

  const currentExamIds = useMemo(
    () => new Set([...(currentPaper?.reading ?? []), ...(currentPaper?.listening ?? [])].map((entry) => entry.examId)),
    [currentPaper]
  );

  /** 题库可见行：可发布 + 已发布（待检查不在这里），按标题搜索。 */
  const visibleRows = useMemo(
    () => libraryRows(store.rows).filter((row) => matchesSearch(row, search)),
    [store.rows, search]
  );
  const trashRows = useMemo(() => store.rows.filter((row) => row.inTrash), [store.rows]);

  // 发布目录：设置里选过就记住，否则追问一次。
  async function resolveDestination(): Promise<string | undefined> {
    const stored = readAppSettings().nasDestination;
    if (stored) return stored;
    const picked = await chooseExportDirectory();
    if (!picked) return undefined;
    writeAppSettings({ nasDestination: picked });
    setDestination(picked);
    return picked;
  }

  function requestPublish() {
    if (!buildPaperSpec(paperSelection)) return;
    const titleOf = (id: string) => store.rows.find((row) => row.id === id)?.title ?? id;
    setConfirmation({
      incoming: selectedPaperItemIds(paperSelection).map(titleOf),
      replacing: replacedPaperTitles(currentPaper)
    });
  }

  /** 组卷发布（替换语义）。清单提交并写入当前试卷之后才算成功；缺题时旧试卷保持不变。 */
  async function publishPaper() {
    if (paperBusy.current) return;
    paperBusy.current = true;
    setPaperPublishing(true);
    setConfirmation(undefined);
    try {
      const spec = buildPaperSpec(paperSelection);
      if (!spec) return;
      const itemIds = selectedPaperItemIds(paperSelection);
      const target = await resolveDestination();
      if (!target) {
        setPaperMessage("请先选择 NAS 目录。");
        return;
      }
      setPaperMessage(`正在发布 0/${itemIds.length}`);
      const outcome = await publishItems(itemIds, target, (done, total) => {
        setPaperMessage(`正在发布 ${done}/${total}`);
      }, { paper: spec });
      const titleOf = (id: string) => store.rows.find((row) => row.id === id)?.title ?? id;
      // 一条结论只说一次：顶层提示与组卷区用同一句，由后端 status 决定（审计 A5）。
      const message = describePublishResult(outcome, titleOf);
      setNotice(message);
      setPaperMessage(message);
      if (outcome.paper?.ok) setPaperSelection(EMPTY_PAPER_SELECTION);
    } finally {
      // 不论成败都重读当前试卷：锁忙或指针失败时，页面要显示 NAS 上真实的当前试卷。
      setPaperVersion((version) => version + 1);
      store.refresh();
      paperBusy.current = false;
      setPaperPublishing(false);
    }
  }

  /** 组卷视图里改名：失败时抛出用户文案，由单元格回滚并提示。 */
  async function renameItem(id: string, title: string) {
    const row = store.rows.find((candidate) => candidate.id === id);
    if (row?.currentEditVersion === undefined) {
      throw new Error("这道题的数据还没准备好，请刷新后再改名。");
    }
    try {
      await renameLibraryItem(id, title, row.currentEditVersion);
      store.refresh();
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (message.includes("EDIT_VERSION_CONFLICT")) {
        throw new Error("已被其他编辑修改，请刷新。");
      }
      throw new Error(describeLibraryActionError(error, "名称保存失败，请稍后再试。"));
    }
  }

  async function trash(id: string) {
    try {
      await store.moveToTrash(id);
      setNotice("已移入回收站，可在回收站恢复。");
    } catch (error) {
      setNotice(describeLibraryActionError(error, "删除失败，请稍后重试。"));
    }
  }

  async function restore(id: string) {
    try {
      await store.restore(id);
      setNotice("已从回收站恢复。");
    } catch (error) {
      setNotice(describeLibraryActionError(error, "恢复失败，请稍后重试。"));
    }
  }

  /** 永久删除单个回收站条目（不可恢复），二次确认后执行。 */
  async function permanentlyDelete(id: string) {
    if (!window.confirm("永久删除后无法恢复，确定要彻底删除这道题吗？")) return;
    try {
      await store.permanentlyDelete(id);
      setNotice("已永久删除。");
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (message.includes("ITEM_STILL_PROCESSING")) {
        setNotice("这道题仍在识别/排队中，请先取消或等待完成，再永久删除。");
        return;
      }
      setNotice(describeLibraryActionError(error, "永久删除失败，请稍后重试。"));
    }
  }

  /** 清空回收站（不可恢复），二次确认后执行；仍在处理中的条目会被跳过并如实说明。 */
  async function emptyTrash() {
    if (!window.confirm("清空回收站会永久删除其中所有题目，无法恢复。确定继续吗？")) return;
    try {
      const result = await store.emptyTrash();
      if (!result.skipped.length) {
        setNotice(`已清空回收站，永久删除 ${result.deleted} 项。`);
        return;
      }
      const stillProcessing = result.skipped.filter((s) => s.reason.includes("ITEM_STILL_PROCESSING")).length;
      const otherFailed = result.skipped.length - stillProcessing;
      const parts = [`已永久删除 ${result.deleted} 项`];
      if (stillProcessing) parts.push(`${stillProcessing} 项仍在识别/排队中被跳过（请先取消或等待完成）`);
      if (otherFailed) parts.push(`${otherFailed} 项删除失败（请稍后重试）`);
      setNotice(parts.join("；") + "。");
    } catch (error) {
      setNotice(describeLibraryActionError(error, "清空回收站失败，请稍后重试。"));
    }
  }

  /** 手动设置/清除某条目的 Part 标签。 */
  async function setPart(id: string, label: string | null) {
    try {
      await store.setPart(id, label);
    } catch (error) {
      setNotice(describeLibraryActionError(error, "设置 Part 标签失败，请稍后重试。"));
    }
  }

  if (showTrash) {
    return (
      <section className="library-page" data-testid="library-page">
        <header className="library-header">
          <div className="library-header-top">
            <div>
              <h1>回收站</h1>
              <p className="library-background-note">回收站里的题不参与组卷。恢复后回到题库。</p>
            </div>
            <div className="button-row">
              <button className="ghost small" data-testid="library-trash-close" onClick={() => setShowTrash(false)}>返回题库</button>
              {trashRows.length ? (
                <button className="danger small" data-testid="library-empty-trash" onClick={emptyTrash}>
                  清空回收站（{trashRows.length}）
                </button>
              ) : null}
            </div>
          </div>
        </header>
        {notice ? (
          <p className="library-notice" data-testid="library-notice" role="status">
            {notice}
            <button className="ghost small" onClick={() => setNotice(undefined)} aria-label="关闭提示">×</button>
          </p>
        ) : null}
        <LibraryItemList
          rows={trashRows}
          loading={store.loading}
          tab="trash"
          selectedIds={new Set()}
          onToggleSelect={() => undefined}
          onOpen={() => undefined}
          onTrash={() => undefined}
          onRestore={restore}
          onPermanentDelete={permanentlyDelete}
          onSetPart={setPart}
          onRetry={() => undefined}
        />
      </section>
    );
  }

  return (
    <section className="library-page" data-testid="library-page">
      <header className="library-header">
        <div className="library-header-top">
          <div>
            <h1>题库</h1>
            <p className="library-background-note">这里只显示可发布与已发布的题。待检查的题在「导入」页处理。</p>
          </div>
          <div className="button-row">
            <input
              className="library-search"
              type="search"
              placeholder="按标题搜索…"
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              aria-label="按标题搜索题目"
            />
            <button className="ghost small" data-testid="library-import" onClick={() => go("/import")}>导入</button>
            <button className="ghost small" data-testid="library-trash-open" onClick={() => setShowTrash(true)}>
              回收站（{trashRows.length}）
            </button>
          </div>
        </div>
      </header>

      <CurrentPaperCard paper={currentPaper} destination={destination} error={currentPaperError} />
      {store.error ? <p className="error-text">{toUserFacingError(store.error, "题库读取失败，请稍后重试。").userMessage}</p> : null}
      {notice ? (
        <p className="library-notice" data-testid="library-notice" role="status">
          {notice}
          <button className="ghost small" onClick={() => setNotice(undefined)} aria-label="关闭提示">×</button>
        </p>
      ) : null}

      <LibraryPaperTable
        rows={visibleRows}
        selection={paperSelection}
        hasCurrentPaper={Boolean(currentPaper)}
        currentExamIds={currentExamIds}
        onToggleReading={(part: ReadingPart, id: string) => setPaperSelection((current) => toggleReadingItem(current, part, id))}
        onToggleListening={(id: string, part: string) => setPaperSelection((current) => toggleListeningItem(current, id, part))}
        onRename={renameItem}
        onOpen={(id) => go(workspacePath(id))}
        onSetPart={setPart}
        onTrash={trash}
      />
      <PaperComposerBar
        selection={paperSelection}
        titleOf={(id) => store.rows.find((row) => row.id === id)?.title ?? id}
        publishing={paperPublishing}
        message={paperMessage}
        confirmation={confirmation}
        onClear={() => setPaperSelection(EMPTY_PAPER_SELECTION)}
        onRequestPublish={requestPublish}
        onConfirm={publishPaper}
        onCancelConfirm={() => setConfirmation(undefined)}
      />

      <section className="library-writing-section" data-testid="library-writing-section">
        <header className="paper-group-head"><h3>写作</h3></header>
        <WritingPanel refresh={store.refresh} />
      </section>
    </section>
  );
}
