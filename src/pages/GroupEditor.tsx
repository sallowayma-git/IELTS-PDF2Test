import { useEffect, useMemo, useState } from "react";
import { getJob, updateAuthoringIr, validateAuthoringIr } from "../api/tauriCommands";
import { go } from "../app/router";
import type { GroupKind, QuestionGroupDraft, ReadingAuthoringIr } from "../types";
import { renderGroupBodyHtml } from "../services/templateRenderer";

const groupKinds: GroupKind[] = ["true_false_not_given", "yes_no_not_given", "single_choice", "multi_choice", "short_answer", "sentence_completion", "summary_completion", "table_completion", "matching", "classification", "diagram_completion"];

function buildGroupPreviewSrcDoc(group: QuestionGroupDraft): string {
  return `<!doctype html><html><head><meta charset="utf-8"><style>
    body{font-family:Georgia,serif;margin:0;padding:16px;background:#fffaf0;color:#17211f;line-height:1.55}
    .reading-question-group{display:block}
    .choice-row{display:flex;gap:8px;flex-wrap:wrap}
    .completion-table{width:100%;border-collapse:collapse}
    .completion-table th,.completion-table td{border:1px solid #d8d0c0;padding:8px}
    input{font:inherit;padding:6px}
  </style></head><body>${renderGroupBodyHtml(group)}</body></html>`;
}

export function GroupEditor({ jobId, refresh }: { jobId: string; refresh: () => void }) {
  const [ir, setIr] = useState<ReadingAuthoringIr | undefined>();
  const [activeGroupId, setActiveGroupId] = useState<string | undefined>();
  const activeGroup = useMemo<QuestionGroupDraft | undefined>(() => ir?.groups.find((group) => group.groupId === activeGroupId) ?? ir?.groups[0], [ir, activeGroupId]);

  async function load() {
    const detail = await getJob(jobId);
    setIr(detail.authoringIr);
    setActiveGroupId((current) => current ?? detail.authoringIr?.groups[0]?.groupId);
  }

  useEffect(() => {
    load().catch(console.error);
  }, [jobId]);

  async function save(next: ReadingAuthoringIr) {
    const saved = await updateAuthoringIr(jobId, { ir: next });
    setIr(saved);
    refresh();
  }

  async function validate() {
    await validateAuthoringIr(jobId);
    refresh();
    go(`/jobs/${jobId}/preview`);
  }

  if (!ir || !activeGroup) {
    return <section className="page-enter"><p className="empty">Authoring IR 不存在。请先在粗切页生成。</p></section>;
  }

  const currentGroup = activeGroup;

  function updateGroup(mutator: (group: QuestionGroupDraft) => QuestionGroupDraft) {
    if (!ir || !currentGroup) return;
    const next: ReadingAuthoringIr = { ...ir, groups: ir.groups.map((group) => (group.groupId === currentGroup.groupId ? mutator(group) : group)) };
    void save(next);
  }

  return (
    <section className="group-editor page-enter" data-testid="group-editor">
      <div className="section-heading spread">
        <div><p className="eyebrow">Authoring IR</p><h2>题组结构化编辑器</h2></div>
        <div className="button-row"><button className="ghost" data-testid="go-llm-review" onClick={() => go(`/jobs/${jobId}/llm-review`)}>LLM 建议</button><button className="primary" data-testid="validate-and-preview" onClick={validate}>校验并预览</button></div>
      </div>
      <div className="editor-grid">
        <aside className="group-nav">
          {ir.groups.map((group) => (
            <button key={group.groupId} className={group.groupId === activeGroup.groupId ? "active" : ""} onClick={() => setActiveGroupId(group.groupId)}>
              <strong>{group.groupId}</strong><span>Q{group.questionRange?.[0]}-{group.questionRange?.[1]}</span><small>{group.requiresManualQuestionImport ? "manual question import" : group.kind}</small>
            </button>
          ))}
        </aside>
        <section className="form-section editor-form">
          {ir.passage.questionUmbrellaRanges?.length ? (
            <div className="info-box" data-testid="umbrella-ranges">
              <strong>开头总题组范围已纳入</strong>
              {ir.passage.questionUmbrellaRanges.map((range) => (
                <p key={`${range.blockId}-${range.questionRange.join("-")}`}>
                  {range.heading}: Q{range.questionRange[0]}-{range.questionRange[1]}
                </p>
              ))}
            </div>
          ) : null}
          {activeGroup.requiresManualQuestionImport ? (
            <div className="warning-box" data-testid="manual-question-import-warning">
              <strong>需要人工补题</strong>
              <p>当前只识别到开头总范围 Q{activeGroup.questionRange?.[0]}-{activeGroup.questionRange?.[1]}，尚未检测到每道题的题干。请粘贴或修订具体题干、答案并逐题确认。</p>
            </div>
          ) : null}
          <label>题型<select value={activeGroup.kind} onChange={(event) => updateGroup((group) => ({ ...group, kind: event.target.value as GroupKind }))}>{groupKinds.map((kind) => <option key={kind}>{kind}</option>)}</select></label>
          <label>说明<textarea value={activeGroup.instruction.join("\n")} onChange={(event) => updateGroup((group) => ({ ...group, instruction: event.target.value.split("\n") }))} /></label>
          <div className="question-stack">
            {activeGroup.questions.map((question, index) => (
              <div className="question-edit" key={question.id}>
                <label>题号<input value={question.displayNumber} onChange={(event) => updateGroup((group) => ({ ...group, questions: group.questions.map((item, i) => i === index ? { ...item, displayNumber: event.target.value } : item) }))} /></label>
                <label>题干<input value={question.prompt} onChange={(event) => updateGroup((group) => ({ ...group, questions: group.questions.map((item, i) => i === index ? { ...item, prompt: event.target.value } : item) }))} /></label>
                <label>答案<input value={Array.isArray(question.answer) ? question.answer.join(",") : question.answer ?? ""} onChange={(event) => updateGroup((group) => ({ ...group, questions: group.questions.map((item, i) => i === index ? { ...item, answer: event.target.value } : item) }))} /></label>
                <label className="inline-check"><input type="checkbox" checked={question.verified} onChange={(event) => updateGroup((group) => ({ ...group, questions: group.questions.map((item, i) => i === index ? { ...item, verified: event.target.checked } : item) }))} /> 人工确认</label>
              </div>
            ))}
          </div>
        </section>
        <aside className="live-preview">
          <p className="eyebrow">BodyHtml Preview</p>
          <iframe className="html-preview-frame" title="group-body-preview" sandbox="" srcDoc={buildGroupPreviewSrcDoc(activeGroup)} />
          <details><summary>DOM 输出</summary><pre>{renderGroupBodyHtml(activeGroup)}</pre></details>
        </aside>
      </div>
    </section>
  );
}
