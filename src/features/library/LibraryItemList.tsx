import { LibraryItemRow } from "./LibraryItemRow";
import type { LibraryFilterTab, LibraryRowV1 } from "./libraryTypes";

export function LibraryItemList({
  rows,
  loading,
  tab,
  selectedIds,
  onToggleSelect,
  onOpen,
  onTrash,
  onRestore
}: {
  rows: LibraryRowV1[];
  loading: boolean;
  tab: LibraryFilterTab;
  selectedIds: Set<string>;
  onToggleSelect: (id: string) => void;
  onOpen: (id: string) => void;
  onTrash: (id: string) => void;
  onRestore: (id: string) => void;
}) {
  if (loading && !rows.length) return <p className="empty">加载中…</p>;
  if (!rows.length) {
    return (
      <p className="empty">
        {tab === "trash" ? "回收站为空。" : "题库为空。点击右上角「导入」选择 PDF 或 DOCX 开始。"}
      </p>
    );
  }
  return (
    <div className="library-list" data-testid="library-list">
      {rows.map((row) => (
        <LibraryItemRow
          key={row.id}
          row={row}
          selected={selectedIds.has(row.id)}
          onToggleSelect={onToggleSelect}
          onOpen={onOpen}
          onTrash={onTrash}
          onRestore={onRestore}
        />
      ))}
    </div>
  );
}
