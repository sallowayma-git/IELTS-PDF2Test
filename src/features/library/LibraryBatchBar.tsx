export function LibraryBatchBar({
  selectedCount,
  publishing,
  publishMessage,
  onClear,
  onPublish
}: {
  selectedCount: number;
  publishing: boolean;
  publishMessage?: string;
  onClear: () => void;
  onPublish: () => void;
}) {
  if (!selectedCount) return null;
  return (
    <div className="library-batch-bar" role="region" aria-label="批量操作" data-testid="library-batch-bar">
      <span>已选择 {selectedCount} 个题目</span>
      {publishMessage ? <span className="library-batch-message">{publishMessage}</span> : null}
      <div className="button-row">
        <button className="ghost small" onClick={onClear} disabled={publishing}>取消选择</button>
        <button className="primary small" data-testid="library-publish-selected" onClick={onPublish} disabled={publishing}>
          {publishing ? "正在发布…" : "发布到 NAS"}
        </button>
      </div>
    </div>
  );
}
