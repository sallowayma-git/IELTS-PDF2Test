import { useEffect, useState } from "react";
import { subscribeProcessing } from "../../api/processingClient";
import { listLibraryItems } from "../../api/workspaceClient";
import { buildRow, importBadgeCount } from "../library/libraryTypes";

/**
 * 导入入口角标：识别中 + 待检查 + 失败的条数。
 * `undefined` 表示还没读到第一次结果（启动时据此决定默认落点，不猜）。
 * 只读题库行模型，与导入页、题库页同源；处理事件与窗口聚焦时重算。
 */
export function useImportBadge(): number | undefined {
  const [count, setCount] = useState<number | undefined>(undefined);
  useEffect(() => {
    let alive = true;
    const load = () => {
      listLibraryItems()
        .then((items) => {
          if (!alive) return;
          const rows = items.map((item) => buildRow(item.id, undefined, undefined, {}, item));
          setCount(importBadgeCount(rows));
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
  return count;
}
