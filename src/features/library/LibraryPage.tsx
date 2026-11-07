import { useCallback, useEffect, useMemo, useState } from "react";
import { chooseExportDirectory } from "../../api/desktopDialogs";
import { describeBatchPublishOutcome, describePaperPublishOutcome, publishItems } from "../../api/publishClient";
import { describeRetryOutcome, retryProcessing } from "../../api/processingClient";
import { getCurrentPaper, type CurrentPaperV1 } from "../../api/paperClient";
import { renameLibraryItem } from "../../api/workspaceClient";
import { go, workspacePath, type LibraryIntent } from "../../app/router";
import { ImportDrawer } from "../import/ImportDrawer";
import { useImportFiles, type ImportRejection } from "../import/useImportFiles";
import { CurrentPaperCard } from "./CurrentPaperCard";
import { LibraryBatchBar } from "./LibraryBatchBar";
import { LibraryHeader } from "./LibraryHeader";
import { LibraryItemList } from "./LibraryItemList";
import { LibraryPaperTable } from "./LibraryPaperTable";
import { PaperComposerBar } from "./PaperComposerBar";
import {
  buildPaperSpec,
  EMPTY_PAPER_SELECTION,
  reconcilePaperSelection,
  selectedPaperItemIds,
  toggleListeningItem,
  toggleReadingItem,
  type PaperSelection,
  type ReadingPart
} from "./paperSelection";
import { WritingPanel } from "./WritingPanel";
import { readAppSettings, writeAppSettings } from "../settings/appSettings";
import { toUserFacingError } from "../../utils/userFacingError";
import { useLibraryStore } from "./libraryStore";
import {
  isModalityTab,
  isProcessingStage,
  matchesModality,
  matchesPart,
  matchesSearch,
  matchesTab,
  MODALITY_TABS,
  type LibraryFilterTab,
  type LibraryModalityTab
} from "./libraryTypes";

// 题库是产品中心（计划 §0.3 / §16.4）：导入、批量任务进度、搜索、打开、选择发布都在这一页完成。
// 已退休的独立页面：Dashboard、JobList、ImportWizard、ExportPage、LibraryExamDetail。
const ALL_TABS: readonly LibraryFilterTab[] = ["all", "processing", "action_required", "ready", "failed", "trash"];

/** 失败提示经用户文案层收敛；机器码/路径只进日志（audit A7-F04）。 */
function describeLibraryActionError(error: unknown, fallback: string): string {
  const facing = toUserFacingError(error, fallback);
  console.error("[library]", facing.internalDetail);
  return facing.userMessage;
}

export function LibraryPage({ intent, initialModality }: { intent?: LibraryIntent; initialModality?: string }) {
  const store = useLibraryStore();
  const [tab, setTab] = useState<LibraryFilterTab>("all");
  const [modalityTab, setModalityTab] = useState<LibraryModalityTab>(
    isModalityTab(initialModality) ? initialModality : "all"
  );
  const [search, setSearch] = useState("");
  const [partFilter, setPartFilter] = useState<string | undefined>(undefined);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [drawerOpen, setDrawerOpen] = useState(intent === "import");
  const [rejected, setRejected] = useState<ImportRejection[]>([]);
  const [publishing, setPublishing] = useState(false);
  const [publishMessage, setPublishMessage] = useState<string | undefined>();
  const [notice, setNotice] = useState<string | undefined>(
    intent === "publish" ? "在下面勾选要发布的题目，然后点击「发布到 NAS」。" : undefined
  );
  // 视图：列表（原有批量发布）或组卷视图（按 Part 分组、选三篇阅读发布为当前试卷）。
  const [viewMode, setViewMode] = useState<"list" | "paper">("list");
  const [paperSelection, setPaperSelection] = useState<PaperSelection>(EMPTY_PAPER_SELECTION);
  const [paperPublishing, setPaperPublishing] = useState(false);
  const [paperMessage, setPaperMessage] = useState<string | undefined>();
  const [destination, setDestination] = useState<string | undefined>(() => readAppSettings().nasDestination);
  // undefined = 还在读取；null = NAS 上还没有当前试卷。
  const [currentPaper, setCurrentPaper] = useState<CurrentPaperV1 | null | undefined>(undefined);
  const [currentPaperError, setCurrentPaperError] = useState<string | undefined>();
  const [paperVersion, setPaperVersion] = useState(0);

  const importer = useImportFiles(store.refresh);

  // 当前试卷（NAS 上的指针）：页面打开时读取，组卷发布后重读。
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

  // 路由携带 ?modality= 时（含 #/legacy/writing 重定向）切到对应题型子标签。
  useEffect(() => {
    if (isModalityTab(initialModality)) setModalityTab(initialModality);
  }, [initialModality]);

  // 题型子标签计数（活动条目，不含回收站）。
  const modalityCounts = useMemo(() => {
    const result = Object.fromEntries(MODALITY_TABS.map((value) => [value, 0])) as Record<LibraryModalityTab, number>;
    for (const value of MODALITY_TABS) {
      result[value] = store.rows.filter((row) => !row.inTrash && matchesModality(row, value)).length;
    }
    return result;
  }, [store.rows]);

  // 状态标签计数：限定在当前题型子标签内。
  const counts = useMemo(() => {
    const result = Object.fromEntries(ALL_TABS.map((value) => [value, 0])) as Record<LibraryFilterTab, number>;
    for (const value of ALL_TABS) {
      result[value] = store.rows.filter((row) => matchesModality(row, modalityTab) && matchesTab(row, value)).length;
    }
    return result;
  }, [store.rows, modalityTab]);

  const tabRows = useMemo(
    () => store.rows.filter((row) => matchesModality(row, modalityTab) && matchesTab(row, tab) && matchesSearch(row, search)),
    [store.rows, modalityTab, tab, search]
  );

  // 后台识别横幅与「清空回收站」都作用于全部题型，计数不能被题型子标签收敛。
  const totalProcessingCount = useMemo(
    () => store.rows.filter((row) => !row.inTrash && isProcessingStage(row.stage)).length,
    [store.rows]
  );
  const totalTrashedCount = useMemo(
    () => store.rows.filter((row) => row.inTrash).length,
    [store.rows]
  );

  // 当前题面里实际出现的 Part（P1/P2/P3 / Part 1–4 / Task 1/2），用于渲染筛选按钮。
  const availableParts = useMemo(() => {
    const set = new Set<string>();
    for (const row of tabRows) if (row.part) set.add(row.part);
    return [...set].sort();
  }, [tabRows]);

  const visibleRows = useMemo(
    () => tabRows.filter((row) => matchesPart(row, partFilter)),
    [tabRows, partFilter]
  );

  // 选中的 Part 不再出现在当前列表时（切了标签/搜索/删除），清掉筛选，避免空列表卡死。
  useEffect(() => {
    if (partFilter && !availableParts.includes(partFilter)) setPartFilter(undefined);
  }, [availableParts, partFilter]);

  // 行离开可见集合（被删除、被筛掉）后不应继续留在选择集中。
  useEffect(() => {
    const visibleIds = new Set(visibleRows.map((row) => row.id));
    setSelectedIds((current) => {
      const next = new Set([...current].filter((id) => visibleIds.has(id)));
      return next.size === current.size ? current : next;
    });
  }, [visibleRows]);

  const toggleSelect = useCallback((id: string) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const startImport = useCallback(async (
    files: Parameters<typeof importer.importFiles>[0],
    decisions?: NonNullable<Parameters<typeof importer.importFiles>[1]>["decisions"]
  ) => {
    const result = await importer.importFiles(files, decisions ? { decisions } : undefined);
    setRejected(result.rejected);
    if (result.rows.length) {
      store.prependOptimistic(result.rows);
      setDrawerOpen(false);
      setTab("all");
      // 失败必须跟「已建立 N 个题目」一起说出口。抽屉在这里被卸载，而 `rejected` 原本只在
      // 抽屉里渲染：导入期的音频绑定失败于是**静默消失**，用户只看到一句成功——
      // 而那段音频根本没绑上。所以提示词要说清失败数，清单要留在页面上（见下方 role="alert"）。
      setNotice(result.rejected.length
        ? `已建立 ${result.rows.length} 个识别任务，但另有 ${result.rejected.length} 项没有成功（见下方清单）。`
        : `已建立 ${result.rows.length} 个识别任务，多篇文章会分别保存，识别在后台继续。`);
    }
  }, [importer, store]);

  // 发布目录在设置页选一次后记住（计划 §13.2）；这里只在还没选过时才追问。
  async function resolveDestination(): Promise<string | undefined> {
    const stored = readAppSettings().nasDestination;
    if (stored) return stored;
    const picked = await chooseExportDirectory();
    if (!picked) return undefined;
    writeAppSettings({ nasDestination: picked });
    setDestination(picked);
    return picked;
  }

  /** 组卷发布：三篇阅读齐了才可点。清单提交之后才写当前试卷，失败时如实说明。 */
  async function publishPaper() {
    const spec = buildPaperSpec(paperSelection);
    if (!spec) return;
    const itemIds = selectedPaperItemIds(paperSelection);
    const target = await resolveDestination();
    if (!target) {
      setPaperMessage("请先选择 NAS 目录。");
      return;
    }
    setPaperPublishing(true);
    setPaperMessage(`正在发布 0/${itemIds.length}`);
    try {
      const outcome = await publishItems(itemIds, target, (done, total) => {
        setPaperMessage(`正在发布 ${done}/${total}`);
      }, { paper: spec });
      const titleOf = (id: string) => store.rows.find((row) => row.id === id)?.title ?? id;
      setNotice(describeBatchPublishOutcome(outcome));
      if (outcome.paper) {
        setPaperMessage(describePaperPublishOutcome(outcome, titleOf));
      } else {
        setPaperMessage(outcome.failed[0]?.message ?? describePaperPublishOutcome(outcome, titleOf));
      }
      if (outcome.paper?.ok) setPaperSelection(EMPTY_PAPER_SELECTION);
      setPaperVersion((version) => version + 1);
      store.refresh();
    } finally {
      setPaperPublishing(false);
    }
  }

  /** 组卷视图里改名：失败时抛出用户文案，由单元格回滚并提示。 */
  async function renameItem(id: string, title: string) {
    try {
      await renameLibraryItem(id, title);
      store.refresh();
    } catch (error) {
      throw new Error(describeLibraryActionError(error, "名称保存失败，请稍后再试。"));
    }
  }

  async function publishSelected() {
    const itemIds = [...selectedIds];
    if (!itemIds.length) return;
    const destination = await resolveDestination();
    if (!destination) {
      setPublishMessage("请先选择 NAS 目录。");
      return;
    }
    setPublishing(true);
    setPublishMessage(`正在发布 0/${itemIds.length}`);
    try {
      const outcome = await publishItems(itemIds, destination, (done, total) => {
        setPublishMessage(`正在发布 ${done}/${total}`);
      });
      // 一次点击即发布：没有预检、没有阻断清单。放行与否只在后端发布记录里。
      setNotice(describeBatchPublishOutcome(outcome));
      setPublishMessage(outcome.failed.length ? outcome.failed[0].message : undefined);
      if (!outcome.failed.length) setSelectedIds(new Set());
      store.refresh();
    } finally {
      setPublishing(false);
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

  /** 失败 / 已取消的行在题库里直接重试（按此刻的云端设置），并如实说有没有入队。 */
  async function retry(id: string) {
    try {
      const queued = await retryProcessing(id);
      setNotice(describeRetryOutcome(queued));
      store.refresh();
    } catch (error) {
      setNotice(describeLibraryActionError(error, "重试没有成功，请稍后再试。"));
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
      // 区分「仍在处理中」与「真的删除失败」——不要把后者也说成「请取消或等待」。
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

  return (
    <section className="library-page" data-testid="library-page">
      <LibraryHeader
        tab={tab}
        counts={counts}
        modalityTab={modalityTab}
        modalityCounts={modalityCounts}
        search={search}
        backgroundCount={totalProcessingCount}
        onTabChange={setTab}
        onModalityChange={setModalityTab}
        onSearchChange={setSearch}
        onImport={() => {
          setRejected([]);
          importer.clearError();
          setDrawerOpen(true);
        }}
      />

      {store.error ? <p className="error-text">{toUserFacingError(store.error, "题库读取失败，请稍后重试。").userMessage}</p> : null}
      {notice ? (
        <p className="library-notice" data-testid="library-notice" role="status">
          {notice}
          <button className="ghost small" onClick={() => setNotice(undefined)} aria-label="关闭提示">×</button>
        </p>
      ) : null}
      {modalityTab === "writing" ? (
        <WritingPanel refresh={store.refresh} />
      ) : (
        <>
          <CurrentPaperCard paper={currentPaper} destination={destination} error={currentPaperError} />
          {tab !== "trash" ? (
            <div className="library-view-toggle" role="group" aria-label="题库视图">
              <button className={`chip small${viewMode === "list" ? " is-active" : ""}`} onClick={() => setViewMode("list")}>列表</button>
              <button
                className={`chip small${viewMode === "paper" ? " is-active" : ""}`}
                data-testid="library-view-paper"
                onClick={() => setViewMode("paper")}
              >
                组卷视图
              </button>
            </div>
          ) : null}

          {/* 导入抽屉一旦关闭，它自己的 reject-list 就随之卸载。导入期的音频绑定失败不能跟着
              消失——「已建立 N 个题目」会和它同时出现，用户得能看见到底哪一段没绑上。 */}
          {!drawerOpen && rejected.length ? (
            <ul className="reject-list" data-testid="library-import-rejected" role="alert">
              {rejected.map((item) => (
                <li key={`${item.name}:${item.reason}`}>
                  <strong className="file-name">{item.name}</strong>
                  <span>{item.reason}</span>
                </li>
              ))}
            </ul>
          ) : null}

          {viewMode === "paper" && tab !== "trash" ? (
            <>
              <LibraryPaperTable
                rows={tabRows}
                selection={paperSelection}
                hasCurrentPaper={Boolean(currentPaper)}
                currentExamIds={currentExamIds}
                onToggleReading={(part: ReadingPart, id: string) => setPaperSelection((current) => toggleReadingItem(current, part, id))}
                onToggleListening={(id: string, part: string) => setPaperSelection((current) => toggleListeningItem(current, id, part))}
                onRename={renameItem}
                onOpen={(id) => go(workspacePath(id))}
              />
              <PaperComposerBar
                selection={paperSelection}
                titleOf={(id) => store.rows.find((row) => row.id === id)?.title ?? id}
                publishing={paperPublishing}
                message={paperMessage}
                onClear={() => setPaperSelection(EMPTY_PAPER_SELECTION)}
                onPublish={publishPaper}
              />
            </>
          ) : null}

          {viewMode === "list" || tab === "trash" ? (
            <>
          {tab !== "trash" && availableParts.length ? (
            <div className="library-part-filter" data-testid="library-part-filter" role="group" aria-label="按 Part 筛选">
              <button className={`chip small${!partFilter ? " is-active" : ""}`} onClick={() => setPartFilter(undefined)}>全部</button>
              {availableParts.map((part) => (
                <button
                  key={part}
                  className={`chip small${partFilter === part ? " is-active" : ""}`}
                  data-testid={`library-part-chip-${part}`}
                  onClick={() => setPartFilter(part)}
                >
                  {part}
                </button>
              ))}
            </div>
          ) : null}

          {/* 清空回收站作用于全部题型，只在「全部」子标签下出现，避免计数与实际删除范围不符。 */}
          {tab === "trash" && modalityTab === "all" && totalTrashedCount > 0 ? (
            <div className="library-trash-toolbar" data-testid="library-trash-toolbar">
              <button className="danger small" data-testid="library-empty-trash" onClick={emptyTrash}>
                清空回收站（{totalTrashedCount}）
              </button>
            </div>
          ) : null}

          <LibraryItemList
            rows={visibleRows}
            loading={store.loading}
            tab={tab}
            selectedIds={selectedIds}
            onToggleSelect={toggleSelect}
            onOpen={(id) =>
              store.rows.find((row) => row.id === id)?.modality === "writing"
                ? setModalityTab("writing")
                : go(workspacePath(id))
            }
            onTrash={trash}
            onRestore={restore}
            onPermanentDelete={permanentlyDelete}
            onSetPart={setPart}
            onRetry={retry}
          />

          <LibraryBatchBar
            selectedCount={selectedIds.size}
            publishing={publishing}
            publishMessage={publishMessage}
            onClear={() => setSelectedIds(new Set())}
            onPublish={publishSelected}
          />
            </>
          ) : null}
        </>
      )}

      <ImportDrawer
        open={drawerOpen}
        busy={importer.busy}
        stageMessage={importer.stageMessage}
        error={importer.error}
        rejected={rejected}
        onClose={() => setDrawerOpen(false)}
        onImport={startImport}
      />
    </section>
  );
}
