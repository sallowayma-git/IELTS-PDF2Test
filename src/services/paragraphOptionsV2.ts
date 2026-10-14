import type { ContentNodeV2 } from "../types/content-doc-v2";
import type { ResponseGroupV2, TaskGroupV2 } from "../types/ielts-authoring-v2";

function paragraphIds(nodes: ContentNodeV2[], result = new Set<string>()): Set<string> {
  for (const node of nodes) {
    if (node.type === "paragraph") result.add(node.id);
    for (const key of ["children", "items", "rows", "cells", "steps", "caption"]) {
      const children = (node as unknown as Record<string, unknown>)[key];
      if (Array.isArray(children)) paragraphIds(children as ContentNodeV2[], result);
    }
  }
  return result;
}

export function paragraphMapIssues(content: ContentNodeV2[], map?: Record<string, string>) {
  const ids = paragraphIds(content);
  const seen = new Set<string>();
  const issues: Array<{ code: string; targetId: string; message: string }> = [];
  for (const [label, id] of Object.entries(map ?? {})) {
    if (!label.trim() || !id.trim() || !ids.has(id)) {
      issues.push({ code: "RUNTIME_PARAGRAPH_MAP_TARGET_INVALID", targetId: label, message: "Every paragraphMap entry must target a real paragraph node in passage.content." });
    }
    if (seen.has(id)) {
      issues.push({ code: "RUNTIME_PARAGRAPH_MAP_TARGET_DUPLICATE", targetId: label, message: "A passage paragraph may have only one paragraph label." });
    }
    seen.add(id);
  }
  return issues;
}

/** Mirror the Rust compiler's source-backed choices without mutating the draft. */
export function taskGroupsWithParagraphOptions<T extends Pick<TaskGroupV2, "taskType" | "responseGroups">>(
  tasks: T[], content: ContentNodeV2[], map?: Record<string, string>
): T[] {
  const labels = Object.keys(map ?? {}).sort();
  if (!labels.length || paragraphMapIssues(content, map).length) return tasks;
  return tasks.map((task) => task.taskType !== "matching_information" ? task : {
    ...task,
    responseGroups: task.responseGroups.map((response): ResponseGroupV2 => {
      if (response.kind !== "matching" || response.options !== undefined || response.optionBankRef !== undefined) return response;
      return { ...response, options: labels.map((label) => {
        const optionId = `${response.responseGroupId}-paragraph-${label}`;
        return { optionId, label, sourceAnchors: [], provenanceStatus: "derived", content: [{
          type: "text", id: `${optionId}-text`, text: label, sourceAnchors: [], provenanceStatus: "derived"
        }] };
      }) };
    })
  });
}
