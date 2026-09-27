import {
  FILTER_TAB_LABEL,
  MODALITY_TABS,
  MODALITY_TAB_LABEL,
  type LibraryFilterTab,
  type LibraryModalityTab
} from "./libraryTypes";

const TABS: readonly LibraryFilterTab[] = ["all", "processing", "action_required", "ready", "failed", "trash"];

export function LibraryHeader({
  tab,
  counts,
  modalityTab,
  modalityCounts,
  search,
  backgroundCount,
  onTabChange,
  onModalityChange,
  onSearchChange,
  onImport
}: {
  tab: LibraryFilterTab;
  counts: Record<LibraryFilterTab, number>;
  modalityTab: LibraryModalityTab;
  modalityCounts: Record<LibraryModalityTab, number>;
  search: string;
  backgroundCount: number;
  onTabChange: (tab: LibraryFilterTab) => void;
  onModalityChange: (tab: LibraryModalityTab) => void;
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

      <div className="library-modality-tabs" role="tablist" aria-label="题型">
        {MODALITY_TABS.map((value) => (
          <button
            key={value}
            role="tab"
            aria-selected={modalityTab === value}
            className={modalityTab === value ? "active" : ""}
            data-testid={`library-modality-${value}`}
            onClick={() => onModalityChange(value)}
          >
            {MODALITY_TAB_LABEL[value]}
            <span className="tab-count">{modalityCounts[value]}</span>
          </button>
        ))}
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
