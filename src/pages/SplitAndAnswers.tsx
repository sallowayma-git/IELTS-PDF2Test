import { useEffect, useState } from "react";
import { buildAuthoringIr, getJob, runRuleSplit } from "../api/tauriCommands";
import { go } from "../app/router";
import type { SplitCandidates } from "../types";

export function SplitAndAnswers({ jobId, refresh }: { jobId: string; refresh: () => void }) {
  const [split, setSplit] = useState<SplitCandidates | undefined>();

  async function load() {
    const detail = await getJob(jobId);
    setSplit(detail.splitCandidates);
  }

  useEffect(() => {
    load().catch(console.error);
  }, [jobId]);

  async function run() {
    const result = await runRuleSplit(jobId);
    setSplit(result);
    refresh();
  }

  async function build() {
    await buildAuthoringIr(jobId);
    refresh();
    go(`/jobs/${jobId}/groups`);
  }

  const answers = split?.answerKeyCandidates[0]?.answers ?? {};

  return (
    <section className="page-enter">
      <div className="section-heading spread">
        <div><p className="eyebrow">Rule Split</p><h2>粗切与答案对齐</h2></div>
        <div className="button-row"><button className="ghost" onClick={run}>运行规则粗切</button><button className="primary" disabled={!split} onClick={build}>生成 Authoring IR</button></div>
      </div>
      <div className="split-grid">
        <section className="form-section">
          <h3>Passage 区</h3>
          {split?.passageCandidates.map((candidate) => (
            <div className="candidate" key={candidate.title}><strong>{candidate.title}</strong><span>{candidate.range.join(" - ")}</span><small>{candidate.categoryHint}</small></div>
          )) ?? <p className="empty">尚未生成 passage candidate。</p>}
        </section>
        <section className="form-section">
          <h3>题组区</h3>
          {split?.questionGroupCandidates.map((group) => (
            <div className="candidate" key={group.groupId}>
              <strong>{group.heading}</strong>
              <span>{group.kindHint} · Q{group.questionRange[0]}-{group.questionRange[1]}</span>
              <p>{group.instructionText}</p>
            </div>
          )) ?? <p className="empty">尚未生成 question groups。</p>}
        </section>
        <section className="form-section contrast">
          <h3>答案区</h3>
          <div className="answer-grid">
            {Object.entries(answers).map(([number, answer]) => (
              <div key={number}><span>{number}</span><strong>{Array.isArray(answer) ? answer.join(", ") : answer}</strong></div>
            ))}
          </div>
        </section>
      </div>
    </section>
  );
}
