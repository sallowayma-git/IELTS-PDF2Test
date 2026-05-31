import { useEffect, useState } from "react";
import { applyLlmSuggestion, getJob, listLlmProfiles, llmClassifyGroup } from "../api/tauriCommands";
import { go } from "../app/router";
import type { LlmProfilePublic, LlmSuggestion, ReadingAuthoringIr } from "../types";

interface PipelineSummary {
  llm?: {
    blockedAutoApplyGroups?: string[];
  };
}

export function LlmReview({ jobId, refresh }: { jobId: string; refresh: () => void }) {
  const [ir, setIr] = useState<ReadingAuthoringIr | undefined>();
  const [profiles, setProfiles] = useState<LlmProfilePublic[]>([]);
  const [suggestion, setSuggestion] = useState<LlmSuggestion | undefined>();
  const [suggestions, setSuggestions] = useState<LlmSuggestion[]>([]);
  const [pipelineReport, setPipelineReport] = useState<PipelineSummary | undefined>();
  const [groupId, setGroupId] = useState<string>("");

  async function load() {
    const [detail, profileList] = await Promise.all([getJob(jobId), listLlmProfiles()]);
    setIr(detail.authoringIr);
    setProfiles(profileList);
    setSuggestions(detail.llmSuggestions ?? []);
    setPipelineReport(detail.pipelineReport as PipelineSummary | undefined);
    setGroupId((current) => current || detail.authoringIr?.groups[0]?.groupId || "");
    setSuggestion((current) => current ?? detail.llmSuggestions?.[0]);
  }

  useEffect(() => {
    load().catch(console.error);
  }, [jobId]);

  async function run() {
    const profileId = profiles[0]?.profileId ?? "profile-local-placeholder";
    const next = await llmClassifyGroup(jobId, groupId, profileId);
    setSuggestion(next);
    refresh();
    await load();
  }

  async function apply() {
    if (!suggestion) return;
    if (suggestion.confidence < 0.85) return;
    await applyLlmSuggestion(jobId, suggestion.suggestionId, ["kind", "questions"]);
    refresh();
    go(`/jobs/${jobId}/groups`);
  }

  return (
    <section className="page-enter">
      <div className="section-heading spread">
        <div><p className="eyebrow">LLM Diff Review</p><h2>LLM 建议审阅</h2></div>
        <div className="button-row"><button className="ghost" onClick={run}>调用分类/抽取</button><button className="primary" disabled={!suggestion || suggestion.confidence < 0.85} onClick={apply}>应用建议</button></div>
      </div>
      <div className="llm-grid">
        <section className="form-section">
          <h3>Current IR</h3>
          <label>题组<select value={groupId} onChange={(event) => {
            const nextGroupId = event.target.value;
            setGroupId(nextGroupId);
            setSuggestion(suggestions.find((item) => item.groupId === nextGroupId));
          }}>{ir?.groups.map((group) => <option key={group.groupId}>{group.groupId}</option>)}</select></label>
          <pre>{JSON.stringify(ir?.groups.find((group) => group.groupId === groupId), null, 2)}</pre>
        </section>
        <section className="form-section contrast">
          <h3>Suggestion</h3>
          {suggestion ? (
            <>
              <p className="eyebrow">{suggestion.groupId} / {suggestion.suggestionId}</p>
              <div className="score-ring">{Math.round(suggestion.confidence * 100)}%</div>
              {suggestion.confidence < 0.85 ? <p className="empty">低置信度建议只能人工参考，不能自动应用。</p> : null}
              {pipelineReport?.llm?.blockedAutoApplyGroups?.includes(suggestion.groupId) ? <p className="empty">该建议虽然达到置信度阈值，但缺少合格 evidence/sourceBlock 引用或 schema 不合格，已被后端自动应用门禁拦截。</p> : null}
              <h4>Patch</h4>
              <pre>{JSON.stringify(suggestion.patch, null, 2)}</pre>
              <h4>Questions</h4>
              <pre>{JSON.stringify(suggestion.questions ?? [], null, 2)}</pre>
              <h4>Evidence</h4>
              <pre>{JSON.stringify(suggestion.evidence ?? {}, null, 2)}</pre>
            </>
          ) : <p className="empty">当前题组没有建议。自动流水线的低置信建议会在这里按题组显示；也可以手动调用一次分类/抽取。</p>}
        </section>
        <aside className="inspector">
          <p className="eyebrow">Prompt Info</p>
          <h3>{profiles[0]?.name ?? "No profile"}</h3>
          <dl><dt>model</dt><dd>{profiles[0]?.model}</dd><dt>forceJson</dt><dd>{String(profiles[0]?.forceJson)}</dd><dt>key</dt><dd>{profiles[0]?.hasApiKey ? "secret ref present" : "local fallback"}</dd><dt>rule</dt><dd>自动应用必须同时满足高置信、schema 合法、evidence 引用当前 sourceBlock；LLM 不直接输出 JS</dd></dl>
          <h4>历史建议</h4>
          <div className="layer-list">{suggestions.map((item) => <button className="ghost small" key={item.suggestionId} onClick={() => { setGroupId(item.groupId); setSuggestion(item); }}>{item.groupId} {Math.round(item.confidence * 100)}%</button>)}</div>
          {suggestion?.warnings?.length ? <><h4>Warnings</h4><pre>{JSON.stringify(suggestion.warnings, null, 2)}</pre></> : null}
          {pipelineReport ? <><h4>Pipeline Report</h4><pre>{JSON.stringify(pipelineReport, null, 2)}</pre></> : null}
        </aside>
      </div>
    </section>
  );
}
