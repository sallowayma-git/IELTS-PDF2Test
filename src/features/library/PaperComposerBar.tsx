import { isPaperReady, missingReadingParts, READING_PARTS, type PaperSelection } from "./paperSelection";

// 组卷栏（固定在底部）：显示 P1/P2/P3 已选的题名与听力篇数；三篇齐了才能发布为当前试卷。
export function PaperComposerBar({
  selection,
  titleOf,
  publishing,
  message,
  onClear,
  onPublish
}: {
  selection: PaperSelection;
  titleOf: (itemId: string) => string;
  publishing: boolean;
  message?: string;
  onClear: () => void;
  onPublish: () => void;
}) {
  const ready = isPaperReady(selection);
  const picked = READING_PARTS.map((part) => selection.reading[part]);
  const anyPicked = picked.some(Boolean) || selection.listening.length > 0;
  const missing = missingReadingParts(selection);
  return (
    <div className="paper-composer" role="region" aria-label="组卷" data-testid="paper-composer">
      <div className="paper-composer-slots">
        {READING_PARTS.map((part) => {
          const id = selection.reading[part];
          return (
            <span key={part} className={`paper-slot${id ? " is-filled" : ""}`} data-testid={`paper-slot-${part}`}>
              <b>{part}</b>
              <span>{id ? titleOf(id) : "未选"}</span>
            </span>
          );
        })}
        <span className="paper-slot" data-testid="paper-slot-listening">
          <b>听力</b>
          <span>{selection.listening.length ? `${selection.listening.length} 篇` : "未选（可选）"}</span>
        </span>
      </div>
      {message ? <span className="library-batch-message" role="status">{message}</span> : null}
      <div className="button-row">
        {anyPicked ? (
          <button className="ghost small" onClick={onClear} disabled={publishing}>清空选择</button>
        ) : null}
        <span className="muted" data-testid="paper-composer-hint">
          {ready ? "三篇阅读已齐" : `还差 ${missing.join("、")}`}
        </span>
        <button
          className="primary small"
          data-testid="paper-publish"
          onClick={onPublish}
          disabled={!ready || publishing}
        >
          {publishing ? "正在发布…" : "发布为当前试卷"}
        </button>
      </div>
    </div>
  );
}
