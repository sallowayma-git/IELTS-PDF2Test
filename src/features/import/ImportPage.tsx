import { useCallback, useState } from "react";
import { describeRetryOutcome, retryProcessing } from "../../api/processingClient";
import { go, workspacePath } from "../../app/router";
import { LibraryItemRow } from "../library/LibraryItemRow";
import { confirmItems, useConfirmedIds } from "../library/confirmedStore";
import { useLibraryStore } from "../library/libraryStore";
import { importQueueGroups, type LibraryRowV1 } from "../library/libraryTypes";
import { toUserFacingError } from "../../utils/userFacingError";
import { ImportDrawer } from "./ImportDrawer";
import { useImportFiles, type ImportRejection } from "./useImportFiles";

// 导入一级页：选择文件（与原导入抽屉同一套逻辑，常驻在页面里）+ 处理队列。
// 队列分三组：识别中 / 待确认 / 失败。识别完成的题在这里检查后「确认入库」才进入题库。

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
  onSetPart,
  onConfirm,
  onConfirmAll
}: {
  title: string;
  testid: string;
  rows: LibraryRowV1[];
  emptyText: string;
  onOpen: (id: string) => void;
  onTrash: (id: string) => void;
  onRetry: (id: string) => void;
  onSetPart: (id: string, label: string | null) => void;
  /** 待确认组才提供：单行确认入库。 */
  onConfirm?: (id: string) => void;
  /** 待确认组才提供：全部确认入库（调用方负责二次确认）。 */
  onConfirmAll?: () => void;
}) {
  return (
    <section className={`import-queue-group${rows.length ? " has-items" : " is-empty"}`} data-testid={testid}>
      <header className="paper-group-head">
        <h3>{title}</h3>
        <span className="paper-group-count">{rows.length} 项</span>
        {onConfirmAll && rows.length ? (
          <button className="primary small" data-testid="import-confirm-all" onClick={onConfirmAll}>
            全部确认入库（{rows.length}）
          </button>
        ) : null}
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
              onConfirm={onConfirm}
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

  /** 确认入库：未设置 Part 的题照常入库，但提示用户入库后在题库里设置 Part 才能组卷。 */
  async function confirmRows(ids: string[]) {
    try {
      await confirmItems(ids);
      store.refresh();
      const single = ids.length === 1 ? store.rows.find((row) => row.id === ids[0]) : undefined;
      const needsPart = single !== undefined && (single.modality === "reading" || single.modality === "listening") && !single.part;
      setNotice(needsPart
        ? "已进入题库，可在题库里组卷发布。还没有设置 Part，入库后在题库里设置才能组卷。"
        : `已进入题库${ids.length > 1 ? `（${ids.length} 项）` : ""}，可在题库里组卷发布。`);
    } catch (error) {
      store.refresh();
      setNotice(describeError(error, "确认入库没有成功，请稍后重试。"));
    }
  }

  function confirmAll(rows: LibraryRowV1[]) {
    if (!window.confirm(`确认 ${rows.length} 项进入题库？确认后仍可在题库里「退回待确认」。`)) return;
    void confirmRows(rows.map((row) => row.id));
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

  const confirmedIds = useConfirmedIds();
  const groups = importQueueGroups(store.rows, confirmedIds);
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
      <section className="import-queue" aria-label="导入进度">
        <h2>导入进度</h2>
        <QueueGroup title="识别中" testid="import-group-working" rows={groups.working} emptyText="没有正在识别的题目。" onOpen={open} onTrash={trash} onRetry={retry} onSetPart={setPart} />
        <QueueGroup
          title="待确认"
          testid="import-group-review"
          rows={groups.review}
          emptyText="没有待确认的题目。识别完成的题在这里确认后进入题库。"
          onOpen={open}
          onTrash={trash}
          onRetry={retry}
          onSetPart={setPart}
          onConfirm={(id) => void confirmRows([id])}
          onConfirmAll={() => confirmAll(groups.review)}
        />
        <QueueGroup title="失败" testid="import-group-failed" rows={groups.failed} emptyText="没有失败的导入。" onOpen={open} onTrash={trash} onRetry={retry} onSetPart={setPart} />
      </section>
    </section>
  );
}
