import { ExamCanvas } from "../../exam-canvas/ExamCanvas";
import type { RepairAidInputV1 } from "./userTasks";
import { command } from "../../api/tauriCommands";
import { getWorkspaceItem } from "../../api/workspaceClient";

export type ComparisonChoice = "local" | "cloud";

/** Flush first, then fetch the authoritative version: flush can increment it. */
export async function chooseComparison(itemId: string, unitId: string, choice: ComparisonChoice, flush: () => Promise<void>) {
  await flush();
  const workspace = await getWorkspaceItem(itemId);
  return command("choose_cloud_comparison", { itemId, unitId, choice, baseVersion: workspace.editVersion });
}

export function CloudComparison({ task, disabled, onChoose, onOpenSource }: {
  task: RepairAidInputV1;
  disabled: boolean;
  onChoose: (choice: ComparisonChoice) => void;
  onOpenSource?: () => void;
}) {
  if (!task.comparisonUnitId || !task.localCandidate || !task.cloudCandidate) return null;
  return <section className="cloud-comparison" aria-label="比较本地与云端题组" data-comparison-unit={task.comparisonUnitId} onClick={(event) => event.stopPropagation()}>
    <p>{task.contextInsufficient ? "原文件证据不足，请比较后选择。" : "这组题仍有差异，请选择要保留的版本。"}</p>
    {onOpenSource ? <button className="ghost small" onClick={onOpenSource}>对照原文件{task.sourcePages?.length ? `（第 ${task.sourcePages.join("、")} 页）` : ""}</button> : null}
    <div className="cloud-comparison-grid">
      {(["local", "cloud"] as const).map((choice) => <div key={choice} className={`cloud-comparison-version is-${choice}`}>
        <header><strong>{choice === "local" ? "本地" : "云端"}</strong><button className="ghost small" disabled={disabled || (choice === "local" ? task.localSelectable : task.cloudSelectable) === false} onClick={() => onChoose(choice)}>采用{choice === "local" ? "本地" : "云端"}</button></header>
        {(choice === "local" ? task.localSelectable : task.cloudSelectable) === false ? <p role="status">此版本仍有结构问题，请修复后采用。</p> : null}
        <div className="cloud-comparison-preview">
          {(choice === "local" ? task.localCandidate : task.cloudCandidate)!.taskGroups.length ? <ExamCanvas authoring={choice === "local" ? task.localCandidate! : task.cloudCandidate!} mode="author" locked comparisonPreview /> : <p>此版本没有这组题。</p>}
        </div>
      </div>)}
    </div>
  </section>;
}
