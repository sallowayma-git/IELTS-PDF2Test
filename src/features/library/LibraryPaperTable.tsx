import { useRef, useState, type KeyboardEvent } from "react";
import { taskTypeLabel } from "../../utils/displayLabels";
import { STAGE_LABEL, type LibraryRowV1 } from "./libraryTypes";
import { isReadingPart, READING_PARTS, type PaperSelection, type ReadingPart } from "./paperSelection";

// 组卷视图：按 阅读 P1 / P2 / P3、听力 分组的表格。勾选即组卷选择（阅读每 Part 一篇，听力多选）。
// 名称列像表格单元格：单击进入编辑，Enter 或失焦保存，Esc 取消，失败回滚并提示。

export interface PaperGroup {
  key: string;
  label: string;
  rows: LibraryRowV1[];
}

const byTitle = (a: LibraryRowV1, b: LibraryRowV1) => a.title.localeCompare(b.title, "zh-CN");

/** 把题库行分成组卷视图的分组。写作不参与组卷，不出现在这里。 */
export function groupRowsForPaper(rows: LibraryRowV1[]): PaperGroup[] {
  const active = rows.filter((row) => !row.inTrash);
  const groups: PaperGroup[] = READING_PARTS.map((part) => ({
    key: part,
    label: `阅读 ${part}`,
    rows: active.filter((row) => row.modality === "reading" && row.part === part).sort(byTitle)
  }));
  const unlabeled = active.filter((row) => row.modality === "reading" && !isReadingPart(row.part)).sort(byTitle);
  if (unlabeled.length) groups.push({ key: "unlabeled", label: "阅读 · 未标 Part", rows: unlabeled });
  groups.push({
    key: "listening",
    label: "听力",
    rows: active
      .filter((row) => row.modality === "listening")
      .sort((a, b) => (a.part ?? "").localeCompare(b.part ?? "", "en") || byTitle(a, b))
  });
  return groups;
}

/**
 * 名称单元格。单击进入编辑；Enter 或失焦保存；Esc 取消（不保存）；
 * 空名不允许（停留在编辑态并提示）；保存失败则回到原名并提示原因。
 */
export function EditableTitle({ title, onSave }: { title: string; onSave: (next: string) => Promise<void> }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(title);
  const [error, setError] = useState<string | undefined>();
  // 已结束（提交中、已保存、或 Esc 取消）的编辑会话：之后的失焦不得再次提交。
  const settled = useRef(false);

  function begin() {
    settled.current = false;
    setDraft(title);
    setError(undefined);
    setEditing(true);
  }

  function cancel() {
    settled.current = true;
    setDraft(title);
    setError(undefined);
    setEditing(false);
  }

  async function commit() {
    if (settled.current) return;
    const next = draft.trim();
    if (!next) {
      setError("名称不能为空。");
      return;
    }
    if (next === title) {
      settled.current = true;
      setEditing(false);
      return;
    }
    settled.current = true;
    try {
      await onSave(next);
      setError(undefined);
      setEditing(false);
    } catch (cause) {
      // 回滚：显示回原名，错误留在单元格下方。
      setDraft(title);
      setError(cause instanceof Error ? cause.message : String(cause));
      setEditing(false);
    }
  }

  function onKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (event.key === "Enter") {
      event.preventDefault();
      void commit();
    } else if (event.key === "Escape") {
      event.preventDefault();
      cancel();
    }
  }

  if (!editing) {
    return (
      <>
        <button type="button" className="paper-title-cell" title="单击编辑名称" onClick={begin}>
          {title}
        </button>
        {error ? <span className="paper-title-error" role="alert">{error}</span> : null}
      </>
    );
  }
  return (
    <>
      <input
        className="paper-title-input"
        aria-label="题目名称"
        autoFocus
        maxLength={120}
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={onKeyDown}
        onBlur={() => void commit()}
      />
      {error ? <span className="paper-title-error" role="alert">{error}</span> : null}
    </>
  );
}

export function LibraryPaperTable({
  rows,
  selection,
  hasCurrentPaper,
  currentExamIds,
  onToggleReading,
  onToggleListening,
  onRename,
  onOpen
}: {
  rows: LibraryRowV1[];
  selection: PaperSelection;
  hasCurrentPaper: boolean;
  currentExamIds: Set<string>;
  onToggleReading: (part: ReadingPart, itemId: string) => void;
  onToggleListening: (itemId: string, part: string) => void;
  onRename: (itemId: string, title: string) => Promise<void>;
  onOpen: (itemId: string) => void;
}) {
  const groups = groupRowsForPaper(rows);
  return (
    <div className="paper-table" data-testid="paper-table">
      {groups.map((group) => (
        <section key={group.key} className="paper-group" data-testid={`paper-group-${group.key}`}>
          <header className="paper-group-head">
            <h3>{group.label}</h3>
            <span className="paper-group-count" data-testid={`paper-group-count-${group.key}`}>{group.rows.length} 篇</span>
          </header>
          {group.rows.length ? (
            <table>
              <thead>
                <tr>
                  <th scope="col">选为试卷</th>
                  <th scope="col">Part</th>
                  <th scope="col">题型</th>
                  <th scope="col">名称</th>
                  <th scope="col">状态</th>
                  <th scope="col">在当前试卷</th>
                  <th scope="col"><span className="visually-hidden">操作</span></th>
                </tr>
              </thead>
              <tbody>
                {group.rows.map((row) => {
                  const inPaper = Boolean(row.examId && currentExamIds.has(row.examId));
                  const readingChecked = isReadingPart(row.part) && selection.reading[row.part] === row.id;
                  const listeningChecked = selection.listening.some((entry) => entry.itemId === row.id);
                  const isListening = row.modality === "listening";
                  const checkable = isListening ? Boolean(row.part) : isReadingPart(row.part);
                  return (
                    <tr key={row.id} data-testid="paper-row" data-item-id={row.id}>
                      <td>
                        <input
                          type="checkbox"
                          data-testid="paper-row-check"
                          checked={isListening ? listeningChecked : readingChecked}
                          disabled={!checkable}
                          title={checkable ? undefined : "请先在题库里设置 Part，再选入试卷"}
                          aria-label={`选为试卷：${row.title}`}
                          onChange={() => {
                            if (isListening) onToggleListening(row.id, row.part ?? "");
                            else if (isReadingPart(row.part)) onToggleReading(row.part, row.id);
                          }}
                        />
                      </td>
                      <td>{row.part ? <span className="part-badge">{row.part}</span> : <span className="muted">未设置</span>}</td>
                      <td>
                        {row.questionTypes.length
                          ? row.questionTypes.map((type) => (
                              <span key={type} className="chip small paper-type-chip">{taskTypeLabel(type)}</span>
                            ))
                          : <span className="muted">—</span>}
                      </td>
                      <td className="paper-name-cell">
                        <EditableTitle title={row.title} onSave={(next) => onRename(row.id, next)} />
                      </td>
                      <td><span className={`stage-pill stage-${row.stage}`}>{STAGE_LABEL[row.stage]}</span></td>
                      <td data-testid="paper-row-in-paper">
                        {hasCurrentPaper ? (inPaper ? "是" : "否") : <span className="muted">—</span>}
                      </td>
                      <td>
                        <button type="button" className="ghost small" onClick={() => onOpen(row.id)}>打开</button>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          ) : (
            <p className="empty">暂无题目</p>
          )}
        </section>
      ))}
    </div>
  );
}
