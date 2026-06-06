import { useEffect, useMemo, useState } from "react";
import { getJob, updateAuthoringIr, validateAuthoringIr } from "../api/tauriCommands";
import { go } from "../app/router";
import type { GroupKind, QuestionGroupDraft, ReadingAuthoringIr } from "../types";
import { renderGroupBodyHtml } from "../services/templateRenderer";

const groupKinds: GroupKind[] = ["true_false_not_given", "yes_no_not_given", "single_choice", "multi_choice", "short_answer", "sentence_completion", "summary_completion", "table_completion", "matching", "heading_matching", "matching_information", "classification", "diagram_completion"];

const groupKindLabels: Record<GroupKind, string> = {
  single_choice: "单选",
  multi_choice: "多选",
  true_false_not_given: "判断题 TRUE/FALSE/NOT GIVEN",
  yes_no_not_given: "判断题 YES/NO/NOT GIVEN",
  matching: "匹配题",
  heading_matching: "标题匹配",
  matching_information: "信息匹配",
  classification: "分类题",
  summary_completion: "摘要填空",
  table_completion: "表格填空",
  diagram_completion: "图表填空",
  short_answer: "简答题",
  sentence_completion: "句子填空"
};

function auditIssueText(issue: string | { message?: string; [key: string]: unknown }): string {
  if (typeof issue === "string") return issue;
  return typeof issue.message === "string" ? issue.message : "";
}

function auditIssueKind(issue: string | { message?: string; [key: string]: unknown }): string {
  if (typeof issue === "string") return "";
  return typeof issue.kind === "string" ? issue.kind : "";
}

function auditIssuePath(issue: string | { message?: string; [key: string]: unknown }): string {
  if (typeof issue === "string") return "";
  return typeof issue.path === "string" ? issue.path : "";
}

function issueList(issue: string | { message?: string; [key: string]: unknown }, key: string): Array<{ message?: string; [key: string]: unknown }> {
  if (typeof issue === "string") return [];
  const value = issue[key];
  return Array.isArray(value) ? value.filter((item): item is { message?: string; [key: string]: unknown } => Boolean(item && typeof item === "object")) : [];
}

function issueStringList(issue: string | { message?: string; [key: string]: unknown }, key: string): string[] {
  if (typeof issue === "string") return [];
  const value = issue[key];
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

function issueSummary(issue: string | { message?: string; [key: string]: unknown }, key: string): Array<{ range?: unknown; kind?: unknown; layoutHint?: unknown; questionIds?: unknown }> {
  if (typeof issue === "string") return [];
  const value = issue[key];
  return Array.isArray(value) ? value.filter((item): item is { range?: unknown; kind?: unknown; layoutHint?: unknown; questionIds?: unknown } => Boolean(item && typeof item === "object")) : [];
}

function formatSummaryRow(item: { range?: unknown; kind?: unknown; layoutHint?: unknown; questionIds?: unknown }): string {
  const range = Array.isArray(item.range) && item.range.length >= 2 ? `Q${item.range[0]}-${item.range[1]}` : "范围未知";
  const ids = Array.isArray(item.questionIds) ? item.questionIds.join(", ") : "";
  return `${range} · ${String(item.kind ?? "题型未知")} · ${String(item.layoutHint ?? "布局未知")}${ids ? ` · ${ids}` : ""}`;
}

function buildGroupPreviewSrcDoc(group: QuestionGroupDraft): string {
  return `<!doctype html><html><head><meta charset="utf-8"><style>
    body{font-family:Georgia,serif;margin:0;padding:16px;background:#fffaf0;color:#17211f;line-height:1.55}
    .reading-question-group{display:block}
    .choice-row{display:flex;gap:8px;flex-wrap:wrap}
    .completion-table{width:100%;border-collapse:collapse}
    .completion-table th,.completion-table td{border:1px solid #d8d0c0;padding:8px}
    input{font:inherit;padding:6px}
    .notes-completion{white-space:pre-wrap}
    .inline-completion input{width:8em;margin:0 4px}
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
    return <section className="page-enter"><p className="empty">题稿尚未生成。请先开始生成题稿，或在题组确认页保存后进入题稿编辑。</p></section>;
  }

  const currentGroup = activeGroup;
  const rawAuditIssues = ir.audit.issues ?? [];
  const auditIssues = rawAuditIssues.map(auditIssueText).filter(Boolean);
  const cloudIssue = rawAuditIssues.find((issue) => auditIssueKind(issue) === "cloud_comparison_summary" || auditIssuePath(issue).includes("cloudComparison"));
  const visionAnswerIssue = rawAuditIssues.find((issue) => auditIssueKind(issue) === "vision_answer_extraction_summary");
  const visibleAuditIssues = auditIssues.filter((issue) => issue !== auditIssueText(cloudIssue ?? "") && issue !== auditIssueText(visionAnswerIssue ?? ""));
  const emptyAnswerCount = currentGroup.questions.filter((question) => {
    const answer = question.answer;
    return Array.isArray(answer) ? answer.length === 0 || answer.every((item) => !String(item).trim()) : !String(answer ?? "").trim();
  }).length;

  function updateGroup(mutator: (group: QuestionGroupDraft) => QuestionGroupDraft) {
    if (!ir || !currentGroup) return;
    const next: ReadingAuthoringIr = { ...ir, groups: ir.groups.map((group) => (group.groupId === currentGroup.groupId ? mutator(group) : group)) };
    void save(next);
  }

  function verifyCurrentGroup() {
    updateGroup((group) => ({
      ...group,
      verified: true,
      questions: group.questions.map((question) => ({ ...question, verified: true }))
    }));
  }

  return (
    <section className="group-editor page-enter" data-testid="group-editor">
      <div className="section-heading spread">
        <div><p className="eyebrow">题稿编辑</p><h2>题稿编辑</h2></div>
        <div className="button-row"><button className="ghost" data-testid="go-llm-review" onClick={() => go(`/jobs/${jobId}/llm-review`)}>需要确认的识别结果</button><button className="ghost" data-testid="verify-current-group" onClick={verifyCurrentGroup}>确认当前题组</button><button className="primary" data-testid="validate-and-preview" onClick={validate}>检查并预览</button></div>
      </div>
      <div className="editor-grid">
        <aside className="group-nav">
          {ir.groups.map((group) => (
            <button key={group.groupId} className={group.groupId === activeGroup.groupId ? "active" : ""} onClick={() => setActiveGroupId(group.groupId)}>
              <strong>{group.groupId}</strong><span>Q{group.questionRange?.[0]}-{group.questionRange?.[1]}</span><small>{group.requiresManualQuestionImport ? "需要补题干" : groupKindLabels[group.kind]}</small>
            </button>
          ))}
        </aside>
        <section className="form-section editor-form">
          {visionAnswerIssue ? (
            <div className="info-box" data-testid="vision-answer-summary">
              <strong>视觉答案补全</strong>
              <p>{auditIssueText(visionAnswerIssue)}</p>
              {issueStringList(visionAnswerIssue, "filledQuestionIds").length ? <small>已写入：{issueStringList(visionAnswerIssue, "filledQuestionIds").join("、")}</small> : null}
              {issueStringList(visionAnswerIssue, "missingQuestionIds").length ? <small>仍缺少：{issueStringList(visionAnswerIssue, "missingQuestionIds").join("、")}</small> : null}
            </div>
          ) : null}
          {cloudIssue ? (
            <div className="warning-box" data-testid="cloud-comparison-summary">
              <strong>云端整卷对照</strong>
              <p>{auditIssueText(cloudIssue)}</p>
              {issueList(cloudIssue, "issues").slice(0, 4).map((issue, index) => <p key={`${index}-${issue.message}`}>{issue.message ?? "云端对照发现差异。"}</p>)}
              {issueList(cloudIssue, "observations").slice(0, 2).map((issue, index) => <small key={`${index}-${issue.message}`}>{issue.message ?? "云端对照备注。"}</small>)}
              <div className="compare-columns">
                <div>
                  <span>本地题稿</span>
                  {issueSummary(cloudIssue, "localSummary").slice(0, 6).map((item, index) => <small key={`local-${index}`}>{formatSummaryRow(item)}</small>)}
                </div>
                <div>
                  <span>云端对照</span>
                  {issueSummary(cloudIssue, "cloudSummary").slice(0, 6).map((item, index) => <small key={`cloud-${index}`}>{formatSummaryRow(item)}</small>)}
                </div>
              </div>
            </div>
          ) : null}
          {visibleAuditIssues.length ? (
            <div className="warning-box" data-testid="authoring-audit-warnings">
              <strong>需要确认</strong>
              {visibleAuditIssues.slice(0, 6).map((issue, index) => <p key={`${index}-${issue}`}>{issue}</p>)}
            </div>
          ) : null}
          {emptyAnswerCount ? (
            <div className="warning-box" data-testid="empty-answer-warning">
              <strong>当前题组仍有 {emptyAnswerCount} 题缺少答案</strong>
              <p>视觉模型可能未能从图片答案页安全提取这些题。请补齐答案后再确认导出。</p>
            </div>
          ) : null}
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
          {activeGroup.reviewWarnings?.length ? (
            <div className="warning-box" data-testid="classification-review-warning">
              <strong>题型/选项规则需要确认</strong>
              {activeGroup.reviewWarnings.map((warning) => <p key={warning}>{warning}</p>)}
              {activeGroup.classificationEvidence?.length ? <small>依据段落：{activeGroup.classificationEvidence.join(", ")}</small> : null}
            </div>
          ) : null}
          {activeGroup.sectionEvidence?.length ? (
            <div className="info-box" data-testid="section-evidence">
              <strong>切分证据</strong>
              <small>
                {activeGroup.sectionEvidence.map((item) => `${item.blockId}@p${item.pageIndex}/c${item.column}${item.tableRows ? `/table:${item.tableRows}x${item.tableCols ?? "?"}` : ""}${item.headingLevel ? `/h${item.headingLevel}` : ""}${item.numberingLevel !== undefined ? `/num:${item.numberingLevel}` : ""}`).join(" -> ")}
              </small>
              {activeGroup.continuationEdges?.length ? (
                <small>
                  连续关系：{activeGroup.continuationEdges.map((edge) => `${edge.fromBlockId} 延续到 ${edge.toBlockId}（${edge.reason}）`).join("；")}
                </small>
              ) : null}
            </div>
          ) : null}
          {(activeGroup.kind === "matching" || activeGroup.kind === "heading_matching" || activeGroup.kind === "matching_information" || activeGroup.kind === "classification") ? (
            <label className="inline-check"><input type="checkbox" checked={activeGroup.allowOptionReuse === true} onChange={(event) => updateGroup((group) => ({ ...group, allowOptionReuse: event.target.checked }))} /> 允许选项重复使用</label>
          ) : null}
          <label>题型<select value={activeGroup.kind} onChange={(event) => updateGroup((group) => ({ ...group, kind: event.target.value as GroupKind }))}>{groupKinds.map((kind) => <option key={kind} value={kind}>{groupKindLabels[kind]}</option>)}</select></label>
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
          <p className="eyebrow">题组预览</p>
          <iframe className="html-preview-frame" title="group-body-preview" sandbox="" srcDoc={buildGroupPreviewSrcDoc(activeGroup)} />
          <details><summary>生成内容（调试）</summary><pre>{renderGroupBodyHtml(activeGroup)}</pre></details>
        </aside>
      </div>
    </section>
  );
}
