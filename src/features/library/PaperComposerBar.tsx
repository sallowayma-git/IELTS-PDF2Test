import { isPaperReady, missingReadingParts, READING_PARTS, type PaperSelection } from "./paperSelection";

/** 确认框的内容：本次要发布的题名，以及将被替换（撤下）的当前试卷题名。 */
export interface PaperConfirmation {
  incoming: string[];
  replacing: string[];
}

// 组卷栏（固定在底部）：显示 P1/P2/P3 已选的题名与听力篇数；三篇齐了才能发布为当前试卷。
// 发布是「替换」：学生端只会看到这一套，上一套被撤下（正在考试的学生不受影响）。
export function PaperComposerBar({
  selection,
  titleOf,
  publishing,
  message,
  confirmation,
  onClear,
  onRequestPublish,
  onConfirm,
  onCancelConfirm
}: {
  selection: PaperSelection;
  titleOf: (itemId: string) => string;
  publishing: boolean;
  message?: string;
  /** 有值时显示确认框（发布前列出将被替换的试卷）。 */
  confirmation?: PaperConfirmation;
  onClear: () => void;
  onRequestPublish: () => void;
  onConfirm: () => void;
  onCancelConfirm: () => void;
}) {
  const ready = isPaperReady(selection);
  const picked = READING_PARTS.map((part) => selection.reading[part]);
  const anyPicked = picked.some(Boolean) || selection.listening.length > 0;
  const missing = missingReadingParts(selection);
  return (
    <div className="paper-composer" role="region" aria-label="组卷" data-testid="paper-composer">
      {confirmation ? (
        <div className="paper-confirm" role="dialog" aria-modal="false" aria-label="确认发布为当前试卷" data-testid="paper-confirm">
          <p><strong>发布为当前试卷？</strong></p>
          <p>将发布：{confirmation.incoming.join("、")}</p>
          {confirmation.replacing.length ? (
            <p data-testid="paper-confirm-replacing">将替换掉当前试卷：{confirmation.replacing.join("、")}</p>
          ) : (
            <p>NAS 上还没有当前试卷，这是第一次发布。</p>
          )}
          <p className="muted">发布后学生端只会看到这一套；上一套会被撤下（正在考试的学生不受影响）。</p>
          <div className="button-row">
            <button className="ghost small" onClick={onCancelConfirm} disabled={publishing}>取消</button>
            <button className="primary small" data-testid="paper-confirm-ok" onClick={onConfirm} disabled={publishing}>确认发布</button>
          </div>
        </div>
      ) : null}
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
        <span className="muted paper-replace-note">发布会替换当前试卷，上一套被撤下。</span>
        <button
          className="primary small"
          data-testid="paper-publish"
          onClick={onRequestPublish}
          disabled={!ready || publishing || Boolean(confirmation)}
        >
          {publishing ? "正在发布…" : "发布为当前试卷"}
        </button>
      </div>
    </div>
  );
}
