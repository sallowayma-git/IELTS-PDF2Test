import { FILTER_TAB_LABEL, type LibraryFilterTab } from "./libraryTypes";

const TABS: readonly LibraryFilterTab[] = ["all", "processing", "action_required", "ready", "failed", "trash"];

export function LibraryHeader({
  tab,
  counts,
  search,
  backgroundCount,
  onTabChange,
  onSearchChange,
  onImport
}: {
  tab: LibraryFilterTab;
  counts: Record<LibraryFilterTab, number>;
  search: string;
  backgroundCount: number;
  onTabChange: (tab: LibraryFilterTab) => void;
  onSearchChange: (value: string) => void;
  onImport: () => void;
}) {
  return (
    <header className="library-header">
      <div className="library-header-top">
        <div>
          <h1>题库</h1>
          {backgroundCount > 0 ? (
            <p className="library-background-note" data-testid="library-background-note">
              {backgroundCount} 个题目正在后台识别，可以先打开已完成的题目。
            </p>
          ) : null}
        </div>
        <button className="primary" data-testid="library-import" onClick={onImport}>导入</button>
      </div>

      <div className="library-controls">
        <div className="library-tabs" role="tablist">
          {TABS.map((value) => (
            <button
              key={value}
              role="tab"
              aria-selected={tab === value}
              className={tab === value ? "active" : ""}
              data-testid={`library-tab-${value}`}
              onClick={() => onTabChange(value)}
            >
              {FILTER_TAB_LABEL[value]}
              <span className="tab-count">{counts[value]}</span>
            </button>
          ))}
        </div>
        <input
          className="library-search"
          type="search"
          placeholder="按标题搜索…"
          value={search}
          onChange={(event) => onSearchChange(event.target.value)}
          aria-label="按标题搜索题目"
        />
      </div>
    </header>
  );
}
