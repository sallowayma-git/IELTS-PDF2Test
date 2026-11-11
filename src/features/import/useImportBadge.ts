import { useEffect, useMemo, useState } from "react";
import { subscribeProcessing } from "../../api/processingClient";
import { listLibraryItems, type LibraryItemSummaryV2 } from "../../api/workspaceClient";
import { useConfirmedIds } from "../library/confirmedStore";
import { buildRow, importBadgeCount } from "../library/libraryTypes";

/**
 * 导入入口角标：识别中 + 待确认 + 失败的条数。
 * `undefined` 表示还没读到第一次结果（启动时据此决定默认落点，不猜）。
 * 只读题库行模型，与导入页、题库页同源；处理事件、窗口聚焦与确认集合变化时重算。
 */
export function useImportBadge(): number | undefined {
  const [items, setItems] = useState<LibraryItemSummaryV2[] | undefined>(undefined);
  const confirmedIds = useConfirmedIds();
  useEffect(() => {
    let alive = true;
    const load = () => {
      listLibraryItems()
        .then((next) => {
          if (alive) setItems(next);
        })
        .catch((error) => console.error("[import-badge] listLibraryItems failed", error));
    };
    load();
    let stop: (() => void) | undefined;
    subscribeProcessing(() => load())
      .then((unlisten) => {
        if (alive) stop = unlisten;
        else unlisten();
      })
      .catch((error) => console.error("[import-badge] subscription failed", error));
    window.addEventListener("focus", load);
    return () => {
      alive = false;
      stop?.();
      window.removeEventListener("focus", load);
    };
  }, []);
  return useMemo(() => {
    if (!items) return undefined;
    const rows = items.map((item) => buildRow(item.id, undefined, undefined, {}, item));
    return importBadgeCount(rows, confirmedIds);
  }, [items, confirmedIds]);
}
