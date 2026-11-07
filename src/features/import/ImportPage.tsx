import { useCallback, useState } from "react";
import { describeRetryOutcome, retryProcessing } from "../../api/processingClient";
import { go, workspacePath } from "../../app/router";
import { LibraryItemRow } from "../library/LibraryItemRow";
import { useLibraryStore } from "../library/libraryStore";
import { importQueueGroups, type LibraryRowV1 } from "../library/libraryTypes";
import { toUserFacingError } from "../../utils/userFacingError";
import { ImportDrawer } from "./ImportDrawer";
import { useImportFiles, type ImportRejection } from "./useImportFiles";

// 导入一级页：选择文件（与原导入抽屉同一套逻辑，常驻在页面里）+ 处理队列。
// 队列分三组：识别中 / 待检查 / 失败。检查通过变成「可发布」后离开本页，进入题库。

function describeError(error: unknown, fallback: string): string {
  const facing = toUserFacingError(error, fallback);
  console.error("[import]", facing.internalDetail);
  return facing.userMessage;
}

function QueueGroup({
  title,
  testid,
  rows,
  emptyText,
  onOpen,
  onTrash,
  onRetry,
  onSetPart
}: {
  title: string;
  testid: string;
  rows: LibraryRowV1[];
  emptyText: string;
  onOpen: (id: string) => void;
  onTrash: (id: string) => void;
  onRetry: (id: string) => void;
  onSetPart: (id: string, label: string | null) => void;
}) {
  return (
    <section className="import-queue-group" data-testid={testid}>
      <header className="paper-group-head">
        <h3>{title}</h3>
        <span className="paper-group-count">{rows.length} 项</span>
      </header>
      {rows.length ? (
        <div className="library-list">
          {rows.map((row) => (
            <LibraryItemRow
              key={row.id}
              row={row}
              selected={false}
              selectable={false}
              onToggleSelect={() => undefined}
              onOpen={onOpen}
              onTrash={onTrash}
              onRestore={() => undefined}
              onPermanentDelete={() => undefined}
              onSetPart={onSetPart}
              onRetry={onRetry}
            />
          ))}
        </div>
      ) : (
        <p className="empty">{emptyText}</p>
      )}
    </section>
  );
}

export function ImportPage() {
  const store = useLibraryStore();
  const importer = useImportFiles(store.refresh);
  const [rejected, setRejected] = useState<ImportRejection[]>([]);
  const [notice, setNotice] = useState<string | undefined>();
  // 导入成功后递增，让选择面板重新挂载（清空已选文件，避免重复导入）。
  const [panelVersion, setPanelVersion] = useState(0);

  const startImport = useCallback(async (
    files: Parameters<typeof importer.importFiles>[0],
    decisions?: NonNullable<Parameters<typeof importer.importFiles>[1]>["decisions"]
  ) => {
    const result = await importer.importFiles(files, decisions ? { decisions } : undefined);
    setRejected(result.rejected);
    if (result.rows.length) {
      store.prependOptimistic(result.rows);
      setPanelVersion((version) => version + 1);
      setNotice(result.rejected.length
        ? `已建立 ${result.rows.length} 个识别任务，但另有 ${result.rejected.length} 项没有成功（见上方清单）。`
        : `已建立 ${result.rows.length} 个识别任务，识别在后台继续，完成后在下方「待检查」里处理。`);
    }
  }, [importer, store]);

  async function retry(id: string) {
    try {
      const queued = await retryProcessing(id);
      setNotice(describeRetryOutcome(queued));
      store.refresh();
    } catch (error) {
      setNotice(describeError(error, "重试没有成功，请稍后再试。"));
    }
  }

  async function trash(id: string) {
    try {
      await store.moveToTrash(id);
      setNotice("已移入回收站，可在题库的回收站里恢复。");
    } catch (error) {
      setNotice(describeError(error, "删除失败，请稍后重试。"));
    }
  }

  async function setPart(id: string, label: string | null) {
    try {
      await store.setPart(id, label);
    } catch (error) {
      setNotice(describeError(error, "设置 Part 标签失败，请稍后重试。"));
    }
  }

  const groups = importQueueGroups(store.rows);
  const open = (id: string) => go(workspacePath(id));

  return (
    <section className="import-page" data-testid="import-page">
      <header className="library-header">
        <div className="library-header-top">
          <div>
            <h1>导入</h1>
            <p className="library-background-note">拖入或选择 PDF / DOCX，识别在后台进行。</p>
          </div>
        </div>
      </header>

      {notice ? (
        <p className="library-notice" data-testid="import-notice" role="status">
          {notice}
          <button className="ghost small" onClick={() => setNotice(undefined)} aria-label="关闭提示">×</button>
        </p>
      ) : null}

      <ImportDrawer
        key={panelVersion}
        mode="page"
        open
        busy={importer.busy}
        stageMessage={importer.stageMessage}
        error={importer.error}
        rejected={rejected}
        onClose={() => undefined}
        onImport={startImport}
      />

      {store.error ? <p className="error-text">{toUserFacingError(store.error, "队列读取失败，请稍后重试。").userMessage}</p> : null}
      <QueueGroup title="识别中" testid="import-group-working" rows={groups.working} emptyText="没有正在识别的题目。" onOpen={open} onTrash={trash} onRetry={retry} onSetPart={setPart} />
      <QueueGroup title="待检查" testid="import-group-review" rows={groups.review} emptyText="没有待检查的题目。检查通过后会进入题库。" onOpen={open} onTrash={trash} onRetry={retry} onSetPart={setPart} />
      <QueueGroup title="失败" testid="import-group-failed" rows={groups.failed} emptyText="没有失败的导入。" onOpen={open} onTrash={trash} onRetry={retry} onSetPart={setPart} />
    </section>
  );
}
