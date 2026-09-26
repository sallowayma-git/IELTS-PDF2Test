import { useCallback, useEffect, useMemo, useState } from "react";
import { chooseExportDirectory } from "../../api/desktopDialogs";
import { describeBatchPublishOutcome, publishItems } from "../../api/publishClient";
import { describeRetryOutcome, retryProcessing } from "../../api/processingClient";
import { go, legacyPath, workspacePath, type LibraryIntent } from "../../app/router";
import { ImportDrawer } from "../import/ImportDrawer";
import { useImportFiles, type ImportRejection } from "../import/useImportFiles";
import { LibraryBatchBar } from "./LibraryBatchBar";
import { LibraryHeader } from "./LibraryHeader";
import { LibraryItemList } from "./LibraryItemList";
import { readAppSettings, writeAppSettings } from "../settings/appSettings";
import { toUserFacingError } from "../../utils/userFacingError";
import { useLibraryStore } from "./libraryStore";
import { matchesPart, matchesSearch, matchesTab, type LibraryFilterTab } from "./libraryTypes";

// 题库是产品中心（计划 §0.3 / §16.4）：导入、批量任务进度、搜索、打开、选择发布都在这一页完成。
// 已退休的独立页面：Dashboard、JobList、ImportWizard、ExportPage、LibraryExamDetail。
const ALL_TABS: readonly LibraryFilterTab[] = ["all", "processing", "action_required", "ready", "failed", "trash"];

/** 失败提示经用户文案层收敛；机器码/路径只进日志（audit A7-F04）。 */
function describeLibraryActionError(error: unknown, fallback: string): string {
  const facing = toUserFacingError(error, fallback);
  console.error("[library]", facing.internalDetail);
  return facing.userMessage;
}

export function LibraryPage({ intent }: { intent?: LibraryIntent }) {
  const store = useLibraryStore();
  const [tab, setTab] = useState<LibraryFilterTab>("all");
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

  const importer = useImportFiles(store.refresh);

  const counts = useMemo(() => {
    const result = Object.fromEntries(ALL_TABS.map((value) => [value, 0])) as Record<LibraryFilterTab, number>;
    for (const value of ALL_TABS) result[value] = store.rows.filter((row) => matchesTab(row, value)).length;
    return result;
  }, [store.rows]);

  const tabRows = useMemo(
    () => store.rows.filter((row) => matchesTab(row, tab) && matchesSearch(row, search)),
    [store.rows, tab, search]
  );

  // C4：当前题面里实际出现的 Part（P1/P2/P3 / Part 1–4 / Task 1/2），用于渲染筛选按钮。
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
        ? `已建立 ${result.rows.length} 个题目，但另有 ${result.rejected.length} 项没有成功（见下方清单）。`
        : `已建立 ${result.rows.length} 个题目，识别在后台继续。`);
    }
  }, [importer, store]);

  // 发布目录在设置页选一次后记住（计划 §13.2）；这里只在还没选过时才追问。
  async function resolveDestination(): Promise<string | undefined> {
    const stored = readAppSettings().nasDestination;
    if (stored) return stored;
    const picked = await chooseExportDirectory();
    if (!picked) return undefined;
    writeAppSettings({ nasDestination: picked });
    return picked;
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
      setNotice(
        result.skipped.length
          ? `已永久删除 ${result.deleted} 项；${result.skipped.length} 项仍在识别/排队中被跳过，请先取消或等待完成。`
          : `已清空回收站，永久删除 ${result.deleted} 项。`
      );
    } catch (error) {
      setNotice(describeLibraryActionError(error, "清空回收站失败，请稍后重试。"));
    }
  }

  const trashedCount = counts.trash;

  /** C4：手动设置/清除某条目的 Part 标签。 */
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
        search={search}
        backgroundCount={counts.processing}
        onTabChange={setTab}
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

      {tab === "trash" && trashedCount > 0 ? (
        <div className="library-trash-toolbar" data-testid="library-trash-toolbar">
          <button className="danger small" data-testid="library-empty-trash" onClick={emptyTrash}>
            清空回收站（{trashedCount}）
          </button>
        </div>
      ) : null}

      <LibraryItemList
        rows={visibleRows}
        loading={store.loading}
        tab={tab}
        selectedIds={selectedIds}
        onToggleSelect={toggleSelect}
        onOpen={(id) => go(store.rows.find((row) => row.id === id)?.modality === "writing" ? legacyPath("writing", id) : workspacePath(id))}
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
