import { STAGE_LABEL, isProcessingStage, type LibraryRowV1 } from "./libraryTypes";

// 普通行不显示 hash、source path、schema、revision 或错误技术码（计划 §10.6 / §3.4）。
const MODALITY_LABEL = { reading: "Reading", writing: "Writing" } as const;

function relativeTime(iso: string): string {
  if (!iso) return "";
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return "";
  const minutes = Math.round((Date.now() - then) / 60000);
  if (minutes < 1) return "刚刚更新";
  if (minutes < 60) return `${minutes} 分钟前`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} 小时前`;
  return new Date(then).toISOString().slice(0, 10);
}

export function LibraryItemRow({
  row,
  selected,
  onToggleSelect,
  onOpen,
  onTrash,
  onRestore
}: {
  row: LibraryRowV1;
  selected: boolean;
  onToggleSelect: (id: string) => void;
  onOpen: (id: string) => void;
  onTrash: (id: string) => void;
  onRestore: (id: string) => void;
}) {
  const processing = isProcessingStage(row.stage);
  const meta = [MODALITY_LABEL[row.modality], row.category, relativeTime(row.updatedAt)].filter(Boolean).join(" · ");

  return (
    <div className={`library-row stage-${row.stage}`} data-testid="library-row" data-item-id={row.id}>
      {row.inTrash ? (
        <span className="library-row-check" aria-hidden="true" />
      ) : (
        <label className="library-row-check">
          <input
            type="checkbox"
            checked={selected}
            onChange={() => onToggleSelect(row.id)}
            aria-label={`选择 ${row.title}`}
          />
        </label>
      )}

      <button className="library-row-main" onClick={() => (row.inTrash ? undefined : onOpen(row.id))} disabled={row.inTrash}>
        <strong className="file-name">{row.title}</strong>
        <small>{meta}</small>
      </button>

      <div className="library-row-state">
        <span className={`stage-pill stage-${row.stage}`}>{STAGE_LABEL[row.stage]}</span>
        {row.actionableCount > 0 && !row.inTrash ? (
          <span className="issue-count" data-testid="library-row-issues">需要检查 {row.actionableCount}</span>
        ) : null}
        {row.detail ? <small className="library-row-detail">{row.detail}</small> : null}
        {processing && row.progressPercent !== undefined ? (
          <span className="progress-track" role="progressbar" aria-valuenow={row.progressPercent} aria-valuemin={0} aria-valuemax={100}>
            <span className="progress-fill" style={{ width: `${row.progressPercent}%` }} />
          </span>
        ) : null}
      </div>

      <div className="library-row-actions">
        {row.inTrash ? (
          <button className="ghost small" onClick={() => onRestore(row.id)}>恢复</button>
        ) : (
          <>
            <button className="ghost small" onClick={() => onOpen(row.id)}>打开</button>
            <button className="danger small" onClick={() => onTrash(row.id)}>删除</button>
          </>
        )}
      </div>
    </div>
  );
}
