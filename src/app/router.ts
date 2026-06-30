export type RouteName =
  | "dashboard"
  | "jobs"
  | "new"
  | "document"
  | "split"
  | "groups"
  | "llm-review"
  | "preview"
  | "export"
  | "packs"
  | "writing"
  | "library"
  | "libraryExam"
  | "settings";

export interface RouteState {
  name: RouteName;
  jobId?: string;
  examId?: string;
}

export function parseRoute(hash = window.location.hash): RouteState {
  const value = hash.replace(/^#\/?/, "");
  const parts = value.split("/").filter(Boolean);
  if (!parts.length) return { name: "dashboard" };
  if (parts[0] === "jobs" && parts[1] === "new") return { name: "new" };
  if (parts[0] === "jobs" && parts[1]) {
    const jobId = parts[1];
    const step = parts[2] as RouteName | undefined;
    if (step && ["document", "split", "groups", "llm-review", "preview", "export"].includes(step)) return { name: step, jobId };
    return { name: "document", jobId };
  }
  if (parts[0] === "library" && parts[1]) return { name: "libraryExam", examId: parts[1] };
  if (parts[0] === "library") return { name: "library" };
  if (parts[0] === "packs") return { name: "packs" };
  if (parts[0] === "writing") return { name: "writing" };
  if (parts[0] === "settings") return { name: "settings" };
  if (parts[0] === "jobs") return { name: "jobs" };
  return { name: "dashboard" };
}

export function go(path: string): void {
  window.location.hash = path;
}
