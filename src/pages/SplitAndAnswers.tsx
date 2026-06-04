import { useEffect, useState } from "react";
import { buildAuthoringIr, getJob, runRuleSplit, saveSplitAdjustments } from "../api/tauriCommands";
import { go } from "../app/router";
import type { AnswerValue, GroupKind, SplitCandidates, SplitGroupCandidate } from "../types";

const groupKinds: GroupKind[] = [
  "single_choice",
  "multi_choice",
  "true_false_not_given",
  "yes_no_not_given",
  "matching",
  "classification",
  "summary_completion",
  "table_completion",
  "diagram_completion",
  "short_answer",
  "sentence_completion"
];

function parseRange(value: string): [number, number] {
  const [start, end] = value.split(/[-–—,~]/).map((item) => Number(item.trim())).filter(Number.isFinite);
  const safeStart = Math.max(1, Math.floor(start || 1));
  const safeEnd = Math.max(safeStart, Math.floor(end || safeStart));
  return [safeStart, safeEnd];
}

function answerToText(value: AnswerValue): string {
  return Array.isArray(value) ? value.join(", ") : value;
}

function textToAnswer(value: string): AnswerValue {
  const parts = value.split(",").map((item) => item.trim()).filter(Boolean);
  return parts.length > 1 ? parts : value.trim();
}

export function SplitAndAnswers({ jobId, refresh }: { jobId: string; refresh: () => void }) {
  const [split, setSplit] = useState<SplitCandidates | undefined>();
  const [saveMessage, setSaveMessage] = useState<string | undefined>();
  const [error, setError] = useState<string | undefined>();

  async function load() {
    const detail = await getJob(jobId);
    setSplit(detail.splitCandidates);
  }

  useEffect(() => {
    load().catch(console.error);
  }, [jobId]);

  function friendlyError(caught: unknown): string {
    const message = caught instanceof Error ? caught.message : String(caught);
    if (message.includes("editable_draft_exists")) {
      return "当前任务已有题稿。为了避免覆盖正在编辑的内容，默认不会重新识别题组；如确需重建，请在高级操作中选择覆盖。";
    }
    return message;
  }

  async function run(allowOverwrite = false) {
    try {
      const result = await runRuleSplit(jobId, allowOverwrite ? { allowOverwrite: true } : undefined);
      setSplit(result);
      setSaveMessage(undefined);
      setError(undefined);
      refresh();
    } catch (caught) {
      setError(friendlyError(caught));
    }
  }

  async function save(nextSplit = split) {
    if (!nextSplit) return;
    const saved = await saveSplitAdjustments(jobId, nextSplit);
    setSplit(saved);
    setSaveMessage("已保存人工修订，后续结构编辑会使用当前切分与答案。");
    setError(undefined);
    refresh();
  }

  async function build(allowOverwrite = false) {
    try {
      if (split) await save(split);
      await buildAuthoringIr(jobId, allowOverwrite ? { allowOverwrite: true } : undefined);
      setError(undefined);
      refresh();
      go(`/jobs/${jobId}/groups`);
    } catch (caught) {
      setError(friendlyError(caught));
    }
  }

  const answers = split?.answerKeyCandidates[0]?.answers ?? {};
  const setGroup = (index: number, updater: (group: SplitGroupCandidate) => SplitGroupCandidate) => {
    setSplit((current) => current ? { ...current, questionGroupCandidates: current.questionGroupCandidates.map((group, groupIndex) => groupIndex === index ? updater(group) : group) } : current);
  };
  const setAnswer = (number: string, value: string) => {
    setSplit((current) => {
      if (!current) return current;
      const candidates = current.answerKeyCandidates.length ? [...current.answerKeyCandidates] : [{ source: "manual", answers: {} }];
      const first = { ...candidates[0], answers: { ...candidates[0].answers, [number]: textToAnswer(value) } };
      candidates[0] = first;
      return { ...current, answerKeyCandidates: candidates, issues: current.issues.filter((issue) => !issue.includes("No answer key detected") && !issue.includes("未识别到答案")) };
    });
  };
  const removeAnswer = (number: string) => {
    setSplit((current) => {
      if (!current?.answerKeyCandidates.length) return current;
      const candidates = [...current.answerKeyCandidates];
      const answers = { ...candidates[0].answers };
      delete answers[number];
      candidates[0] = { ...candidates[0], answers };
      return { ...current, answerKeyCandidates: candidates };
    });
  };
  const addAnswer = () => {
    const existing = Object.keys(answers).map(Number).filter(Number.isFinite);
    const nextNumber = String((existing.length ? Math.max(...existing) : 0) + 1);
    setAnswer(nextNumber, "");
  };

  return (
    <section className="page-enter">
      <div className="section-heading spread">
        <div><p className="eyebrow">识别题组</p><h2>题组与答案确认</h2></div>
        <div className="button-row"><button className="ghost" data-testid="run-rule-split" onClick={() => run()}>识别题组</button><button className="ghost" data-testid="save-split-adjustments" disabled={!split} onClick={() => save()}>保存修订</button><button className="primary" data-testid="build-authoring-ir" disabled={!split} onClick={() => build()}>进入题稿编辑</button></div>
      </div>
      {error ? <p className="error-text" data-testid="split-error">{error}</p> : null}
      {saveMessage ? <p className="success-text">{saveMessage}</p> : null}
      {split?.issues.length ? <div className="warning-box"><strong>需要复核</strong>{split.issues.map((issue) => <p key={issue}>{issue}</p>)}</div> : null}
      <div className="split-grid">
        <section className="form-section">
          <h3>Passage 区</h3>
          {split?.passageCandidates.map((candidate) => (
            <div className="candidate" key={candidate.title}><strong>{candidate.title}</strong><span>{candidate.range.join(" - ")}</span><small>{candidate.categoryHint}</small></div>
          )) ?? <p className="empty">尚未生成文章范围候选。</p>}
          {split?.umbrellaQuestionRanges?.length ? (
            <>
              <h4>总题组范围</h4>
              {split.umbrellaQuestionRanges.map((range) => (
                <div className="candidate umbrella-candidate" key={`${range.blockId}-${range.questionRange.join("-")}`}>
                  <strong>{range.heading}</strong>
                  <span>Q{range.questionRange[0]}-{range.questionRange[1]}</span>
                  <small>来自开头说明，作为 Passage 2 总范围保留。</small>
                </div>
              ))}
            </>
          ) : null}
        </section>
        <section className="form-section">
          <h3>题组区</h3>
          {split?.questionGroupCandidates.map((group, index) => (
            <div className={`candidate ${group.isUmbrellaRange ? "umbrella-candidate" : ""}`} key={group.groupId}>
              {group.requiresManualQuestionImport ? <p className="error-text">仅检测到总题组范围，需要人工导入具体题干。</p> : null}
              <label>标题<input value={group.heading} onChange={(event) => setGroup(index, (item) => ({ ...item, heading: event.target.value }))} /></label>
              <label>题号范围<input value={`${group.questionRange[0]}-${group.questionRange[1]}`} onChange={(event) => setGroup(index, (item) => ({ ...item, questionRange: parseRange(event.target.value) }))} /></label>
              <label>题型<select value={group.kindHint ?? "short_answer"} onChange={(event) => setGroup(index, (item) => ({ ...item, kindHint: event.target.value as GroupKind }))}>{groupKinds.map((kind) => <option key={kind}>{kind}</option>)}</select></label>
              <label>来源段落<input value={group.blockIds.join(", ")} onChange={(event) => setGroup(index, (item) => ({ ...item, blockIds: event.target.value.split(",").map((id) => id.trim()).filter(Boolean) }))} /></label>
              <label>说明<textarea value={group.instructionText} onChange={(event) => setGroup(index, (item) => ({ ...item, instructionText: event.target.value }))} /></label>
            </div>
          )) ?? <p className="empty">尚未识别到题组。</p>}
          <details>
            <summary>高级操作</summary>
            <p className="empty">重新识别并覆盖当前题稿会丢弃已生成题稿中的编辑内容，只在确认需要重建时使用。</p>
            <div className="button-row">
              <button className="ghost small" onClick={() => run(true)}>覆盖并重新识别题组</button>
              <button className="ghost small" disabled={!split} onClick={() => build(true)}>覆盖并重新生成题稿</button>
            </div>
          </details>
        </section>
        <section className="form-section contrast">
          <div className="section-heading spread"><h3>答案区</h3><button className="ghost small" disabled={!split} onClick={addAnswer}>新增答案</button></div>
          <div className="answer-grid">
            {Object.entries(answers).map(([number, answer]) => (
              <div key={number} className="answer-edit-row"><span>{number}</span><input value={answerToText(answer)} onChange={(event) => setAnswer(number, event.target.value)} /><button className="ghost small" onClick={() => removeAnswer(number)}>删除</button></div>
            ))}
          </div>
          {split?.answerKeyCandidates.slice(1).map((candidate) => <details key={candidate.source}><summary>{candidate.source}</summary><pre>{JSON.stringify(candidate.answers, null, 2)}</pre></details>)}
        </section>
      </div>
    </section>
  );
}
