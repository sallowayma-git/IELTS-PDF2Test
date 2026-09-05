import { useCallback, useEffect, useMemo, useState } from "react";
import { chooseExportDirectory } from "../../api/desktopDialogs";
import { publishItems } from "../../api/publishClient";
import { go, workspacePath, type LibraryIntent } from "../../app/router";
import { ImportDrawer } from "../import/ImportDrawer";
import { useImportFiles, type ImportRejection } from "../import/useImportFiles";
import { LibraryBatchBar } from "./LibraryBatchBar";
import { LibraryHeader } from "./LibraryHeader";
import { LibraryItemList } from "./LibraryItemList";
import { readAppSettings, writeAppSettings } from "../settings/appSettings";
import { useLibraryStore } from "./libraryStore";
import { matchesSearch, matchesTab, type LibraryFilterTab } from "./libraryTypes";

// 题库是产品中心（计划 §0.3 / §16.4）：导入、批量任务进度、搜索、打开、选择发布都在这一页完成。
// 已退休的独立页面：Dashboard、JobList、ImportWizard、ExportPage、LibraryExamDetail。
const ALL_TABS: readonly LibraryFilterTab[] = ["all", "processing", "action_required", "ready", "failed", "trash"];

export function LibraryPage({ intent }: { intent?: LibraryIntent }) {
  const store = useLibraryStore();
  const [tab, setTab] = useState<LibraryFilterTab>("all");
  const [search, setSearch] = useState("");
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

  const visibleRows = useMemo(
    () => store.rows.filter((row) => matchesTab(row, tab) && matchesSearch(row, search)),
    [store.rows, tab, search]
  );

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

  const startImport = useCallback(async (files: Parameters<typeof importer.importFiles>[0]) => {
    const result = await importer.importFiles(files);
    setRejected(result.rejected);
    if (result.rows.length) {
      store.prependOptimistic(result.rows);
      setDrawerOpen(false);
      setTab("all");
      setNotice(`已建立 ${result.rows.length} 个题目，识别在后台继续。`);
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
      const parts = [`发布完成：${outcome.succeeded.length} 题`];
      if (outcome.failed.length) parts.push(`${outcome.failed.length} 题未发布`);
      setNotice(parts.join(" · "));
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
      setNotice(`删除失败：${error instanceof Error ? error.message : String(error)}`);
    }
  }

  async function restore(id: string) {
    try {
      await store.restore(id);
      setNotice("已从回收站恢复。");
    } catch (error) {
      setNotice(`恢复失败：${error instanceof Error ? error.message : String(error)}`);
    }
  }

  return (
    <section className="library-page" data-testid="library-page">
      <LibraryHeader
        tab={tab}
        counts={counts}
        search={search}
        backgroundCount={importer.backgroundCount}
        onTabChange={setTab}
        onSearchChange={setSearch}
        onImport={() => {
          setRejected([]);
          importer.clearError();
          setDrawerOpen(true);
        }}
      />

      {store.error ? <p className="error-text">题库读取失败：{store.error}</p> : null}
      {notice ? (
        <p className="library-notice" data-testid="library-notice" role="status">
          {notice}
          <button className="ghost small" onClick={() => setNotice(undefined)} aria-label="关闭提示">×</button>
        </p>
      ) : null}

      <LibraryItemList
        rows={visibleRows}
        loading={store.loading}
        tab={tab}
        selectedIds={selectedIds}
        onToggleSelect={toggleSelect}
        onOpen={(id) => go(workspacePath(id))}
        onTrash={trash}
        onRestore={restore}
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
