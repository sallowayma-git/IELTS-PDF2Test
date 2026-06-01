import { useEffect, useState } from "react";
import { applyLlmSuggestion, getJob, listLlmProfiles, llmClassifyGroup } from "../api/tauriCommands";
import { go } from "../app/router";
import type { LlmProfilePublic, LlmSuggestion, ReadingAuthoringIr } from "../types";
import { jobStatusLabel, workflowStepLabel } from "../utils/displayLabels";

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

  function suggestionFromGroup(groupIdValue: string): LlmSuggestion | undefined {
    const group = ir?.groups.find((item) => item.groupId === groupIdValue);
    if (!group?.llmReview?.required) return undefined;
    return {
      suggestionId: group.llmReview.suggestionId ?? `${groupIdValue}-review`,
      jobId,
      groupId: groupIdValue,
      kind: group.llmReview.suggestedKind ?? group.kind,
      confidence: group.llmReview.confidence,
      patch: [],
      questions: [],
      evidence: group.llmReview.evidence ?? {},
      warnings: group.llmReview.warnings ?? [],
      createdAt: group.llmReview.recordedAt ?? new Date().toISOString()
    };
  }

  async function load() {
    const [detail, profileList] = await Promise.all([getJob(jobId), listLlmProfiles()]);
    setIr(detail.authoringIr);
    setProfiles(profileList);
    const authoredSuggestions = detail.authoringIr?.groups.flatMap((group) => suggestionFromGroup(group.groupId) ? [suggestionFromGroup(group.groupId)!] : []) ?? [];
    setSuggestions(detail.llmSuggestions?.length ? detail.llmSuggestions : authoredSuggestions);
    setPipelineReport(detail.pipelineReport as PipelineSummary | undefined);
    setGroupId((current) => current || detail.authoringIr?.groups[0]?.groupId || "");
    setSuggestion((current) => current ?? (detail.llmSuggestions?.[0] ?? authoredSuggestions[0]));
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

  const activeGroup = ir?.groups.find((group) => group.groupId === groupId);
  const evidence = (suggestion?.evidence ?? {}) as { sourceBlockIds?: string[]; blockIds?: string[]; quotes?: Array<{ blockId?: string; text?: string }>; source?: string };
  const evidenceBlocks = evidence.sourceBlockIds ?? evidence.blockIds ?? [];

  return (
    <section className="page-enter">
      <div className="section-heading spread">
        <div><p className="eyebrow">模型建议审阅</p><h2>模型建议审阅</h2></div>
        <div className="button-row"><button className="ghost" onClick={run}>调用分类/抽取</button><button className="primary" disabled={!suggestion || suggestion.confidence < 0.85} onClick={apply}>应用建议</button></div>
      </div>
      <div className="llm-grid">
        <section className="form-section">
          <h3>当前题组结构</h3>
          <label>题组<select value={groupId} onChange={(event) => {
            const nextGroupId = event.target.value;
            setGroupId(nextGroupId);
            const nextSuggestion = suggestions.find((item) => item.groupId === nextGroupId);
            setSuggestion(nextSuggestion ?? suggestionFromGroup(nextGroupId));
          }}>{ir?.groups.map((group) => <option key={group.groupId}>{group.groupId}</option>)}</select></label>
          {activeGroup ? (
            <dl>
              <dt>题型</dt><dd>{activeGroup.kind}</dd>
              <dt>题号范围</dt><dd>Q{activeGroup.questionRange?.[0]}-{activeGroup.questionRange?.[1]}</dd>
              <dt>题目数量</dt><dd>{activeGroup.questions.length}</dd>
              <dt>置信度</dt><dd>{Math.round(activeGroup.confidence * 100)}%</dd>
              <dt>状态</dt><dd>{activeGroup.verified ? "已人工确认" : "待人工确认"}</dd>
            </dl>
          ) : <p className="empty">请选择一个题组。</p>}
        </section>
        <section className="form-section contrast">
          <h3>建议内容</h3>
          {suggestion ? (
            <>
              <p className="eyebrow">题组建议</p>
              <div className="score-ring">{Math.round(suggestion.confidence * 100)}%</div>
              {suggestion.confidence < 0.85 ? <p className="empty">低置信度建议只能人工参考，不能自动应用。</p> : null}
              {pipelineReport?.llm?.blockedAutoApplyGroups?.includes(suggestion.groupId) ? <p className="empty">该建议虽然达到置信度阈值，但缺少可追溯来源，或格式不符合要求，已被自动应用门禁拦截。</p> : null}
              <h4>拟修改内容</h4>
              <pre>{JSON.stringify(suggestion.patch, null, 2)}</pre>
              <h4>题目建议</h4>
              <pre>{JSON.stringify(suggestion.questions ?? [], null, 2)}</pre>
              <h4>来源依据</h4>
              {evidenceBlocks.length || evidence.quotes?.length ? (
                <dl>
                  <dt>来源段落</dt><dd>{evidenceBlocks.length ? evidenceBlocks.join(", ") : "未提供"}</dd>
                  <dt>来源摘录</dt><dd>{evidence.quotes?.map((quote) => quote.text).filter(Boolean).join(" / ") || "未提供"}</dd>
                </dl>
              ) : <p className="empty">该建议未提供可展示的来源依据。</p>}
            </>
          ) : <p className="empty">当前题组没有建议。自动流水线的低置信建议会在这里按题组显示；也可以手动调用一次分类/抽取。</p>}
        </section>
        <aside className="inspector">
          <p className="eyebrow">模型调用信息</p>
          <h3>{profiles[0]?.name ?? "No profile"}</h3>
          <dl><dt>模型</dt><dd>{profiles[0]?.model}</dd><dt>强制结构化输出</dt><dd>{profiles[0]?.forceJson ? "是" : "否"}</dd><dt>密钥状态</dt><dd>{profiles[0]?.hasApiKey ? "已配置" : "本地兜底"}</dd><dt>自动应用规则</dt><dd>必须同时满足高置信、格式合法，并能追溯到当前题组来源段落；模型不会直接生成最终 JS。</dd></dl>
          <h4>历史建议</h4>
          <div className="layer-list">{suggestions.map((item) => <button className="ghost small" key={item.suggestionId} onClick={() => { setGroupId(item.groupId); setSuggestion(item); }}>题组 {Math.round(item.confidence * 100)}%</button>)}</div>
          {suggestion?.warnings?.length ? <><h4>提醒</h4><pre>{JSON.stringify(suggestion.warnings, null, 2)}</pre></> : null}
          {pipelineReport ? <><h4>自动处理报告</h4><dl><dt>任务状态</dt><dd>{jobStatusLabel((pipelineReport as { status?: string }).status)}</dd><dt>当前步骤</dt><dd>{workflowStepLabel((pipelineReport as { currentStep?: string }).currentStep)}</dd></dl></> : null}
        </aside>
      </div>
    </section>
  );
}
