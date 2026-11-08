import { go } from "../../app/router";
import { useAppSettings } from "../settings/appSettings";
import { BatchOverviewPanel } from "./BatchOverviewPanel";
import { dataViewPath, type DataView } from "./dataRoute";
import { ExamPanel } from "./ExamPanel";
import { paperText } from "./dataFormat";
import { QuestionPanel } from "./QuestionPanel";
import { StudentPanel } from "./StudentPanel";

// 「数据」一级入口：批次总览 → 单次考试 → 学生详情 / 题目详情（子视图由 #/data/... 决定）。
export function DataPage({ view }: { view: DataView }) {
  const [settings] = useAppSettings();
  const root = settings.submissionsRoot;
  const key = dataViewPath(view);

  return (
    <div className="data-page" data-testid="data-page">
      <Crumbs view={view} />
      {view.kind === "batches" ? <BatchOverviewPanel key={`${key}|${root}`} root={root} /> : null}
      {view.kind === "exam" ? (
        <ExamPanel key={key} batchId={view.batchId} paperId={view.paperId} />
      ) : null}
      {view.kind === "student" ? <StudentPanel key={key} attemptId={view.attemptId} /> : null}
      {view.kind === "question" ? (
        <QuestionPanel
          key={key}
          batchId={view.batchId}
          paperId={view.paperId}
          passageIndex={view.passageIndex}
          questionId={view.questionId}
        />
      ) : null}
    </div>
  );
}

function Crumbs({ view }: { view: DataView }) {
  if (view.kind === "batches") return null;
  const items: Array<{ label: string; to?: DataView }> = [{ label: "数据", to: { kind: "batches" } }];
  if (view.kind === "exam") {
    items.push({ label: `${view.batchId} · ${paperText(view.paperId)}` });
  }
  if (view.kind === "student") {
    items.push({ label: "学生", to: undefined });
  }
  if (view.kind === "question") {
    items.push({
      label: `${view.batchId} · ${paperText(view.paperId)}`,
      to: { kind: "exam", batchId: view.batchId, paperId: view.paperId }
    });
    items.push({ label: "题目" });
  }
  return (
    <nav className="data-crumbs" aria-label="数据导航">
      {items.map((item, index) => (
        <span key={`${item.label}-${index}`}>
          {index > 0 ? <span className="data-crumb-sep" aria-hidden="true">/</span> : null}
          {item.to ? (
            <button className="link-button" type="button" onClick={() => go(dataViewPath(item.to as DataView))}>
              {item.label}
            </button>
          ) : (
            <span>{item.label}</span>
          )}
        </span>
      ))}
    </nav>
  );
}
