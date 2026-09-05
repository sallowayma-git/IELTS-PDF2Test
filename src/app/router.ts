// 产品路由只有三个主表面（计划 §3.1）：题库 / 题目工作区 / 设置。
//
// 流水线内部阶段（source review、split、groups、llm-review、preview、export）不再是路由，
// 它们变成题库行上的阶段状态和工作区内的局部问题。旧链接通过 legacyRedirect 一次性重定向，
// 保留一个发行周期，只写日志，不出现在导航里。
export type RouteName = "library" | "workspace" | "settings" | "legacy";

/** 兼容期内仍可通过 URL 到达、但不出现在导航中的旧页面（计划 §16.2 的兼容周期）。
 *  收敛期不能在替代能力落地前静默移除用户已有能力，因此这些页面保留在 `#/legacy/...`，
 *  在 P10「旧链删除」阶段随 §20.2 清单一起删除。 */
export type LegacyPageName =
  | "dashboard"
  | "jobs"
  | "import"
  | "document"
  | "preview"
  | "authoring-v2"
  | "export"
  | "writing"
  | "library-exam";

const LEGACY_PAGES: readonly LegacyPageName[] = [
  "dashboard", "jobs", "import", "document", "preview", "authoring-v2", "export", "writing", "library-exam"
];

function asLegacyPage(value: string | undefined): LegacyPageName | undefined {
  return LEGACY_PAGES.find((page) => page === value);
}

/** 题库页可以由 URL 携带的一次性意图，用于承接被退休的 /jobs/new 与 /export 入口。 */
export type LibraryIntent = "import" | "publish";

export interface RouteState {
  name: RouteName;
  /** 题库条目 id。当前数据模型下 library item id 与 job id 相同（见 findings F12）。 */
  itemId?: string;
  intent?: LibraryIntent;
  legacyPage?: LegacyPageName;
}

/** 已退休的路由 -> 新路由。返回 undefined 表示这个 hash 不是旧链接。 */
export function legacyRedirect(hash: string): string | undefined {
  const value = hash.replace(/^#\/?/, "");
  const parts = value.split(/[/?]/).filter(Boolean);
  if (!parts.length) return "/library";
  const [head, second, third] = parts;
  if (head === "dashboard" || head === "phase5") return "/library";
  if (head === "jobs") {
    if (!second) return "/library";
    if (second === "new") return "/library?import=1";
    // /jobs/:id/(document|split|groups|llm-review|preview|authoring-v2|export) -> /items/:id
    return third === "export" ? `/items/${second}?publish=1` : `/items/${second}`;
  }
  if (head === "library" && second) return `/items/${second}`;
  if (head === "export" || head === "packs") return "/library?publish=1";
  return undefined;
}

function parseIntent(value: string): LibraryIntent | undefined {
  const query = value.includes("?") ? value.slice(value.indexOf("?") + 1) : "";
  if (!query) return undefined;
  const params = new URLSearchParams(query);
  if (params.get("import") === "1") return "import";
  if (params.get("publish") === "1") return "publish";
  return undefined;
}

export function parseRoute(hash = window.location.hash): RouteState {
  const value = hash.replace(/^#\/?/, "");
  const intent = parseIntent(value);
  const parts = value.split(/[/?]/).filter(Boolean);
  if (!parts.length) return { name: "library" };
  if (parts[0] === "items" && parts[1]) return { name: "workspace", itemId: parts[1], intent };
  if (parts[0] === "settings") return { name: "settings" };
  if (parts[0] === "legacy") {
    const legacyPage = asLegacyPage(parts[1]);
    if (legacyPage) return { name: "legacy", legacyPage, itemId: parts[2], intent };
    return { name: "library", intent };
  }
  if (parts[0] === "library") return { name: "library", intent };
  return { name: "library", intent };
}

export function go(path: string): void {
  window.location.hash = path;
}

export function libraryPath(intent?: LibraryIntent): string {
  return intent ? `/library?${intent}=1` : "/library";
}

export function workspacePath(itemId: string, intent?: LibraryIntent): string {
  return intent ? `/items/${itemId}?${intent}=1` : `/items/${itemId}`;
}

export function legacyPath(page: LegacyPageName, id?: string): string {
  return id ? `/legacy/${page}/${id}` : `/legacy/${page}`;
}

/** 「继续这个任务」现在等于「打开这道题的工作区」——流水线阶段不再是可导航的位置。
 *  兼容期旧页面（Dashboard / JobList）仍用它跳转。 */
export function jobResumePath(job: { jobId: string }): string {
  return workspacePath(job.jobId);
}

/** 在 hashchange 之前把旧链接换成新链接。返回 true 表示已触发一次重定向。 */
export function applyLegacyRedirect(hash = window.location.hash): boolean {
  const raw = hash.replace(/^#\/?/, "");
  const parts = raw.split(/[/?]/).filter(Boolean);
  // 新路由与显式 legacy 逃生通道都不重定向。
  if (parts[0] === "items" || parts[0] === "settings" || parts[0] === "legacy") return false;
  if (parts[0] === "library" && !parts[1]) return false;
  const target = legacyRedirect(hash);
  if (!target) return false;
  const current = `/${raw}`;
  if (current === target) return false;
  console.info(`[router] legacy route redirected: ${current || "/"} -> ${target}`);
  window.location.replace(`${window.location.pathname}${window.location.search}#${target}`);
  return true;
}
