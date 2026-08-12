import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { applyAuthoringV2Patches, getAuthoringV2 } from "../api/tauriCommands";
import { go } from "../app/router";
import { answerValueForSelection, applyAuthoringV2Patches as applyLocalPatches, taskTypeLabel } from "../services/authoringV2Patches";
import type {
  AnswerSlotV2,
  AnswerValueV2,
  AuthoringEditorRecoveryV2,
  AuthoringEditorSessionV2,
  AuthoringPatchV2,
  ContentNodeV2,
  IeltsAuthoringIRV2,
  OptionV2,
  ResponseGroupV2,
  SourceAnchorV2,
  TaskGroupV2,
  TaskTypeV2
} from "../types";

const RECOVERY_KEY_PREFIX = "ielts-author-studio.phase5-recovery.";
const TASK_TYPES: TaskTypeV2[] = [
  "multiple_choice",
  "single_choice",
  "true_false_not_given",
  "yes_no_not_given",
  "matching_headings",
  "matching_information",
  "summary_completion",
  "note_completion",
  "table_completion",
  "diagram_label_completion",
  "short_answer"
];

function recoveryKey(jobId: string): string {
  return RECOVERY_KEY_PREFIX + jobId;
}

function nodeText(nodes: ContentNodeV2[] | undefined): string {
  if (!nodes) return "";
  return nodes.map((node) => {
    if (node.type === "text") return node.text;
    if ("children" in node) return nodeText(node.children);
    if (node.type === "answer_slot") return "[" + node.displayLabel + "]";
    return "";
  }).join("");
}

function findEntity(value: unknown, id: string): Record<string, unknown> | undefined {
  if (Array.isArray(value)) {
    for (const item of value) {
      const found = findEntity(item, id);
      if (found) return found;
    }
    return undefined;
  }
  if (!value || typeof value !== "object") return undefined;
  const object = value as Record<string, unknown>;
  if (object.id === id || object.taskId === id || object.responseGroupId === id || object.slotId === id || object.optionId === id) return object;
  for (const child of Object.values(object)) {
    const found = findEntity(child, id);
    if (found) return found;
  }
  return undefined;
}

function sourceAnchorsFor(authoring: IeltsAuthoringIRV2, id: string): SourceAnchorV2[] {
  const entity = findEntity(authoring, id);
  return Array.isArray(entity?.sourceAnchors) ? entity.sourceAnchors as SourceAnchorV2[] : [];
}

function sourceSummary(anchors: SourceAnchorV2[]): string {
  if (!anchors.length) return "暂无来源锚点";
  const pages = Array.from(new Set(anchors.map((anchor) => anchor.pageIndex + 1))).sort((a, b) => a - b);
  return "源文件页码：" + pages.join("、") + " · " + anchors.reduce((count, anchor) => count + anchor.nodeIds.length, 0) + " 个源节点";
}

function readRecovery(jobId: string): AuthoringEditorRecoveryV2 | undefined {
  try {
    const raw = window.localStorage.getItem(recoveryKey(jobId));
    if (!raw) return undefined;
    const value = JSON.parse(raw) as AuthoringEditorRecoveryV2;
    if (value.schemaVersion !== "AuthoringEditorRecoveryV1" || value.jobId !== jobId || !Array.isArray(value.patches)) return undefined;
    return value;
  } catch {
    return undefined;
  }
}

function selectedLabels(value: AnswerValueV2 | undefined): string[] {
  return value?.kind === "option" ? value.labels : [];
}

function answerText(value: AnswerValueV2 | undefined): string {
  return value?.kind === "text" ? value.values.join(", ") : "";
}

function contentNodeEditor(
  nodes: ContentNodeV2[] | undefined,
  selectedId: string | undefined,
  onSelect: (id: string) => void,
  onTextChange: (node: Extract<ContentNodeV2, { type: "text" }>) => void,
  readOnly = false
): ReactNode {
  if (!nodes?.length) return <span className="structured-empty-inline">暂无内容</span>;
  return nodes.map((node) => {
    const selected = node.id === selectedId;
    const className = ["structured-node", "structured-node-" + node.type, selected ? "selected" : ""].join(" ");
    switch (node.type) {
      case "text":
        return readOnly
          ? <span key={node.id} data-editor-id={node.id} className={className} onClick={() => onSelect(node.id)}>{node.text}</span>
          : <textarea key={node.id} data-editor-id={node.id} className={className} aria-label={"编辑文本 " + node.id} value={node.text} rows={Math.max(1, Math.min(4, Math.ceil(node.text.length / 70)))} onClick={() => onSelect(node.id)} onChange={(event) => onTextChange({ ...node, text: event.target.value })} />;
      case "paragraph":
        return <p key={node.id} data-editor-id={node.id} className={className} onClick={() => onSelect(node.id)}>{contentNodeEditor(node.children, selectedId, onSelect, onTextChange, readOnly)}</p>;
      case "heading":
        return <div key={node.id} data-editor-id={node.id} className={className} onClick={() => onSelect(node.id)}><strong>{contentNodeEditor(node.children, selectedId, onSelect, onTextChange, readOnly)}</strong></div>;
      case "hard_break":
        return <br key={node.id} />;
      case "answer_slot":
        return <span key={node.id} data-editor-id={node.id} className={className + " structured-slot-chip"} onClick={() => onSelect(node.id)}>{node.displayLabel}</span>;
      case "bullet_list":
      case "ordered_list":
        return <ul key={node.id} data-editor-id={node.id} className={className} onClick={() => onSelect(node.id)}>{node.items.map((item) => <li key={item.id}>{contentNodeEditor(item.children, selectedId, onSelect, onTextChange, readOnly)}</li>)}</ul>;
      case "table":
        return <table key={node.id} data-editor-id={node.id} className={className} onClick={() => onSelect(node.id)}><tbody>{node.rows.map((row) => <tr key={row.id}>{row.cells.map((cell) => <td key={cell.id} colSpan={cell.colSpan} rowSpan={cell.rowSpan}>{contentNodeEditor(cell.children, selectedId, onSelect, onTextChange, readOnly)}</td>)}</tr>)}</tbody></table>;
      case "figure":
      case "image":
      case "diagram":
        return <div key={node.id} data-editor-id={node.id} className={className + " structured-asset-placeholder"} onClick={() => onSelect(node.id)}><span>视觉资源</span><small>{node.assetId}</small></div>;
      case "flowchart":
        return <div key={node.id} data-editor-id={node.id} className={className + " structured-flowchart"} onClick={() => onSelect(node.id)}>{node.steps.map((step) => <div key={step.id} className="structured-flow-step">{step.label ? <strong>{step.label}</strong> : null}{contentNodeEditor(step.children, selectedId, onSelect, onTextChange, readOnly)}</div>)}</div>;
      case "flow_step":
      case "list_item":
      case "figcaption":
      case "doc":
        return <div key={node.id} data-editor-id={node.id} className={className} onClick={() => onSelect(node.id)}>{"children" in node ? contentNodeEditor(node.children, selectedId, onSelect, onTextChange, readOnly) : null}</div>;
      case "option_bank":
      case "horizontal_rule":
        return <div key={node.id} data-editor-id={node.id} className={className} onClick={() => onSelect(node.id)} />;
    }
  });
}

function optionText(option: OptionV2): string {
  return nodeText(option.content);
}

function StudentPreview({ authoring }: { authoring: IeltsAuthoringIRV2 }) {
  return <div className="student-parity-grid">
    <article className="student-sheet"><p className="student-sheet-label">Passage</p><h3>{authoring.passage?.title}</h3>{contentNodeEditor(authoring.passage?.content, undefined, () => undefined, () => undefined, true)}</article>
    <article className="student-sheet"><p className="student-sheet-label">Questions</p>{authoring.taskGroups.map((task) => <section key={task.taskId} className="student-task">{contentNodeEditor(task.instructions, undefined, () => undefined, () => undefined, true)}{task.responseGroups.map((group) => <div key={group.responseGroupId} className="student-response">{contentNodeEditor(group.prompt, undefined, () => undefined, () => undefined, true)}{task.optionBank?.options.map((option) => <div key={option.optionId} className="student-option"><span>{option.label}</span>{optionText(option)}</div>)}{group.slotIds.map((slotId) => <div key={slotId} className="student-answer-row"><strong>{authoring.answerSlots[slotId]?.displayLabel}</strong><span className="student-control">选择答案</span></div>)}</div>)}</section>)}</article>
  </div>;
}

function SourceOverlay({ authoring, selectedId }: { authoring: IeltsAuthoringIRV2; selectedId?: string }) {
  const anchors = selectedId ? sourceAnchorsFor(authoring, selectedId) : [];
  const pages = Array.from(new Set(anchors.map((anchor) => anchor.pageIndex))).sort((a, b) => a - b);
  return <section className="source-overlay-panel">
    <div className="inspector-section-heading"><span>源定位</span><small>{selectedId ?? "未选择节点"}</small></div>
    <p className="source-summary">{sourceSummary(anchors)}</p>
    {pages.length ? pages.map((page) => <div className="source-page-mini" key={page}><span>源页面 {page + 1}</span><div className="source-page-grid">{anchors.filter((anchor) => anchor.pageIndex === page).flatMap((anchor) => anchor.nodeIds).map((nodeId) => <i key={nodeId} title={nodeId}>{nodeId}</i>)}</div></div>) : <div className="source-page-mini source-page-empty">点击题干、选项或问题即可查看源锚点</div>}
  </section>;
}

export function StructuredAuthoringEditorV2({ jobId, refresh }: { jobId: string; refresh?: () => void }) {
  const [session, setSession] = useState<AuthoringEditorSessionV2>();
  const [draft, setDraft] = useState<IeltsAuthoringIRV2>();
  const draftRef = useRef<IeltsAuthoringIRV2 | undefined>(undefined);
  const [pendingPatches, setPendingPatches] = useState<AuthoringPatchV2[]>([]);
  const pendingRef = useRef<AuthoringPatchV2[]>([]);
  const [selectedId, setSelectedId] = useState<string>();
  const [viewMode, setViewMode] = useState<"edit" | "preview">("edit");
  const [status, setStatus] = useState<"loading" | "clean" | "dirty" | "saving" | "saved" | "conflict" | "error">("loading");
  const [error, setError] = useState<string>();
  const [recoveryCandidate, setRecoveryCandidate] = useState<AuthoringEditorRecoveryV2>();
  const [saving, setSaving] = useState(false);

  const load = useCallback(async () => {
    setStatus("loading");
    setError(undefined);
    try {
      const next = await getAuthoringV2(jobId);
      setSession(next);
      draftRef.current = next.authoring;
      setDraft(next.authoring);
      setPendingPatches([]);
      pendingRef.current = [];
      const recovery = readRecovery(jobId);
      if (recovery && recovery.baseRevision === next.revision && recovery.patches.length) setRecoveryCandidate(recovery);
      else if (recovery) window.localStorage.removeItem(recoveryKey(jobId));
      setStatus("clean");
    } catch (caught) {
      setStatus("error");
      setError(caught instanceof Error ? caught.message : String(caught));
    }
  }, [jobId]);

  useEffect(() => { void load(); }, [load]);

  const queuePatch = useCallback((patch: AuthoringPatchV2) => {
    const nextDraft = draftRef.current ? applyLocalPatches(draftRef.current, [patch]) : undefined;
    draftRef.current = nextDraft;
    setDraft(nextDraft);
    pendingRef.current = pendingRef.current.concat(patch);
    setPendingPatches(pendingRef.current);
    setStatus("dirty");
  }, []);

  const flush = useCallback(async (patches: AuthoringPatchV2[]) => {
    if (!session || !patches.length || saving) return;
    setSaving(true);
    setStatus("saving");
    try {
      const result = await applyAuthoringV2Patches({ jobId, baseRevision: session.revision, patches });
      const remaining = pendingRef.current.slice(patches.length);
      setSession(result);
      const nextDraft = remaining.length ? applyLocalPatches(result.authoring, remaining) : result.authoring;
      draftRef.current = nextDraft;
      setDraft(nextDraft);
      pendingRef.current = remaining;
      setPendingPatches(remaining);
      if (!remaining.length) window.localStorage.removeItem(recoveryKey(jobId));
      setStatus(remaining.length ? "dirty" : "saved");
      refresh?.();
    } catch (caught) {
      const message = caught instanceof Error ? caught.message : String(caught);
      setStatus(message.includes("revision_conflict") ? "conflict" : "error");
      setError(message);
    } finally {
      setSaving(false);
    }
  }, [jobId, refresh, saving, session]);

  useEffect(() => {
    if (!session || !pendingPatches.length) return;
    const recovery: AuthoringEditorRecoveryV2 = { schemaVersion: "AuthoringEditorRecoveryV1", jobId, baseRevision: session.revision, updatedAt: new Date().toISOString(), patches: pendingPatches };
    window.localStorage.setItem(recoveryKey(jobId), JSON.stringify(recovery));
    const timer = window.setTimeout(() => { void flush(pendingPatches); }, 650);
    return () => window.clearTimeout(timer);
  }, [flush, jobId, pendingPatches, session]);

  useEffect(() => {
    if (!selectedId) return;
    const timer = window.setTimeout(() => {
      document.querySelector("[data-editor-id=\"" + selectedId + "\"]")?.scrollIntoView({ behavior: "smooth", block: "center" });
    }, 0);
    return () => window.clearTimeout(timer);
  }, [selectedId]);

  const activeTask = useMemo(() => draft?.taskGroups[0], [draft]);
  const issues = draft?.quality.issues ?? [];
  const selectedAnchors = draft && selectedId ? sourceAnchorsFor(draft, selectedId) : [];

  function editText(node: Extract<ContentNodeV2, { type: "text" }>) {
    const currentText = findEntity(draftRef.current, node.id)?.text;
    const fromLength = typeof currentText === "string" ? Array.from(currentText).length : 0;
    queuePatch({ op: "replaceText", nodeId: node.id, from: 0, to: fromLength, text: node.text });
  }

  function toggleAnswer(slot: AnswerSlotV2, label: string) {
    const current = selectedLabels(draft?.answerKey[slot.slotId]);
    const next = current.includes(label) ? current.filter((item) => item !== label) : current.concat(label);
    queuePatch({ op: "setAnswer", slotId: slot.slotId, value: answerValueForSelection(next) });
  }

  function expressionLabel(task: TaskGroupV2): string {
    const expression = task.displayRange;
    if (expression.kind === "range") return "Questions " + expression.start + "–" + expression.end;
    if (expression.kind === "set") return "Questions " + expression.values.join(" and ");
    return "Questions " + expression.values.map((value) => typeof value === "number" ? String(value) : value.start + "–" + value.end).join(", ");
  }

  function renderResponseGroup(task: TaskGroupV2, group: ResponseGroupV2) {
    const bank = task.optionBank;
    return <section className={"structured-response-card " + (selectedId === group.responseGroupId ? "selected" : "")} data-editor-id={group.responseGroupId} onClick={() => setSelectedId(group.responseGroupId)}>
      <div className="structured-card-heading"><div><span className="node-kicker">Response group</span><h4>{group.slotIds.map((slotId) => draft?.answerSlots[slotId]?.displayLabel).join("、")} · 共享题干</h4></div><small>{group.allowOptionReuse ? "选项可复用" : "选项不可复用"}</small></div>
      <div className="shared-prompt"><span className="node-kicker">Prompt</span>{contentNodeEditor(group.prompt, selectedId, setSelectedId, editText)}</div>
      {bank ? <div className="option-bank-editor" data-editor-id={bank.optionBankId}><div className="option-bank-heading"><strong>{nodeText(bank.title)}</strong><small>{bank.options.length} 个公共选项</small></div>{bank.options.map((option) => <div className="option-editor-row" key={option.optionId} data-editor-id={option.optionId}><b>{option.label}</b><div>{contentNodeEditor(option.content, selectedId, setSelectedId, editText)}</div></div>)}</div> : null}
      <div className="slot-editor-grid">{group.slotIds.map((slotId) => {
        const slot = draft?.answerSlots[slotId];
        if (!slot) return null;
        const labels = selectedLabels(draft?.answerKey[slotId]);
        return <article key={slotId} className={"slot-editor-card " + (selectedId === slotId ? "selected" : "")} data-editor-id={slotId} onClick={() => setSelectedId(slotId)}>
          <div className="slot-editor-heading"><strong>{slot.displayLabel}</strong><small>{slot.interaction} · {Math.round(slot.confidence * 100)}%</small></div>
          <div className="choice-chip-row">{(bank?.options ?? []).map((option) => <button key={option.optionId} type="button" className={"choice-chip " + (labels.includes(option.label) ? "active" : "")} onClick={(event) => { event.stopPropagation(); toggleAnswer(slot, option.label); }}>{option.label}</button>)}</div>
          <label className="slot-text-fallback">手工文本<input value={answerText(draft?.answerKey[slotId])} onChange={(event) => queuePatch({ op: "setAnswer", slotId, value: { kind: "text", values: [event.target.value], normalization: "ielts_default" } })} placeholder="仅适用于填空题" /></label>
        </article>;
      })}</div>
    </section>;
  }

  function renderTaskEditor(task: TaskGroupV2) {
    const group = task.responseGroups[0];
    return <section className="structured-task-card" data-editor-id={task.taskId}>
      <div className="structured-card-heading"><div><span className="node-kicker">Task group</span><h3>{expressionLabel(task)}</h3></div><span className={"editor-review-state " + task.reviewState}>{task.reviewState === "confirmed" ? "已确认" : "待确认"}</span></div>
      <div className="structured-task-controls"><label>题型<select value={task.taskType} onChange={(event) => queuePatch({ op: "setTaskType", taskId: task.taskId, taskType: event.target.value as TaskTypeV2 })}>{TASK_TYPES.map((type) => <option key={type} value={type}>{taskTypeLabel(type)}</option>)}</select></label><div className="semantic-rule"><span>选择规则</span><strong>{group?.cardinality.exact ?? group?.cardinality.max ?? 1} 个答案 · {group?.assignment === "unordered_set" ? "无序集合" : "按序"}</strong></div></div>
      <div className="structured-instruction"><span className="node-kicker">Instruction</span>{contentNodeEditor(task.instructions, selectedId, setSelectedId, editText)}</div>
      {group ? renderResponseGroup(task, group) : <div className="empty">当前题组没有 response group。</div>}
    </section>;
  }

  if (status === "loading") return <section className="page-enter"><p className="eyebrow">Phase 5</p><h2>正在打开结构化编辑器…</h2></section>;
  if (!draft || !session) return <section className="page-enter phase5-empty"><p className="eyebrow">Phase 5 · 结构化编辑器</p><h2>当前任务还没有 V2 编辑稿</h2><p>{error ?? "需要先生成 authoring-ir-v2.shadow.json，或从 Phase 5 架构 fixture 开始。"}</p><div className="button-row"><button className="ghost" onClick={() => go("/jobs")}>返回任务列表</button><button className="primary" onClick={() => go("/phase5")}>打开 Phase 5 fixture</button></div></section>;

  return <section className="page-enter phase5-editor" data-testid="structured-authoring-editor-v2">
    <header className="phase5-editor-header"><div><p className="eyebrow">Phase 5 · Schema-driven authoring</p><h2>{draft.exam.title}</h2><p>编辑结构化题源；学生预览、答案位和发布输入都来自同一棵文档树。</p></div><div className="phase5-header-actions"><span className={"editor-save-state " + status}>{status === "saving" ? "自动保存中" : status === "dirty" ? "有未保存修改" : status === "saved" ? "已保存" : "revision " + session.revision}</span><button className="ghost" onClick={() => go(jobId === "phase5-editor-fixture" ? "/phase5" : "/jobs/" + jobId + "/preview")}>退出编辑</button><button className="primary" onClick={() => setViewMode(viewMode === "edit" ? "preview" : "edit")}>{viewMode === "edit" ? "学生端预览" : "返回编辑"}</button></div></header>
    {recoveryCandidate ? <div className="info-box phase5-recovery-banner"><strong>发现未提交的本地修改</strong><p>浏览器关闭前留下 {recoveryCandidate.patches.length} 个 patch，基于 revision {recoveryCandidate.baseRevision}。</p><div className="button-row"><button className="primary small" onClick={() => { const recovered = applyLocalPatches(draft, recoveryCandidate.patches); draftRef.current = recovered; setDraft(recovered); pendingRef.current = recoveryCandidate.patches; setPendingPatches(recoveryCandidate.patches); setRecoveryCandidate(undefined); }}>恢复修改</button><button className="ghost small" onClick={() => { window.localStorage.removeItem(recoveryKey(jobId)); setRecoveryCandidate(undefined); }}>放弃</button></div></div> : null}
    {status === "conflict" ? <div className="warning-box phase5-recovery-banner"><strong>保存冲突：服务器已有新 revision</strong><p>为避免覆盖另一窗口的修改，当前本地 patch 已保留。请重新加载后手工合并。</p><div className="button-row"><button className="primary small" onClick={() => void load()}>重新加载服务器版本</button></div></div> : null}
    {error && status !== "conflict" ? <div className="warning-box phase5-recovery-banner"><strong>编辑器提示</strong><p>{error}</p></div> : null}
    <div className="phase5-editor-toolbar"><div className="phase5-stat"><span>Task groups</span><strong>{draft.taskGroups.length}</strong></div><div className="phase5-stat"><span>Answer slots</span><strong>{Object.keys(draft.answerSlots).length}</strong></div><div className="phase5-stat"><span>Issues</span><strong>{issues.length}</strong></div><div className="phase5-stat"><span>Source coverage</span><strong>{Math.round(draft.quality.sourceCoverage * 100)}%</strong></div><small>来源：{session.source} · V1 文件保持可读</small></div>
    {viewMode === "preview" ? <StudentPreview authoring={draft} /> : <div className="phase5-editor-grid">
      <aside className="phase5-outline"><div className="inspector-section-heading"><span>文档结构</span><small>点击定位</small></div><button className={"outline-row " + (selectedId === draft.passage?.content[0]?.id ? "active" : "")} onClick={() => setSelectedId(draft.passage?.content[0]?.id)}><strong>Passage</strong><small>{draft.passage?.content.length ?? 0} 个内容节点</small></button>{draft.taskGroups.map((task) => <div key={task.taskId} className="outline-group"><button className={"outline-row " + (selectedId === task.taskId ? "active" : "")} onClick={() => setSelectedId(task.taskId)}><strong>{expressionLabel(task)}</strong><small>{taskTypeLabel(task.taskType)}</small></button>{task.responseGroups.map((group) => <button key={group.responseGroupId} className={"outline-row nested " + (selectedId === group.responseGroupId ? "active" : "")} onClick={() => setSelectedId(group.responseGroupId)}>共享 response group <small>{group.slotIds.map((slotId) => draft.answerSlots[slotId]?.displayLabel).join("、")}</small></button>)}{task.optionBank ? <button className={"outline-row nested " + (selectedId === task.optionBank.optionBankId ? "active" : "")} onClick={() => setSelectedId(task.optionBank?.optionBankId)}>公共选项池 <small>{task.optionBank.options.length} 项</small></button> : null}</div>)}<div className="issue-rail"><div className="inspector-section-heading"><span>需要确认</span><small>{issues.length} 项</small></div>{issues.map((issue) => <button key={issue.issueId} className={"issue-rail-item " + issue.severity} onClick={() => setSelectedId(issue.targetId)}><strong>{issue.code}</strong><span>{issue.message}</span></button>)}</div></aside>
      <main className="phase5-editor-canvas"><article className="structured-paper" data-editor-id={draft.passage?.content[0]?.id}><div className="structured-paper-label">PASSAGE · 可编辑节点树</div><h3>{draft.passage?.title}</h3>{contentNodeEditor(draft.passage?.content, selectedId, setSelectedId, editText)}</article>{activeTask ? renderTaskEditor(activeTask) : <div className="empty">暂无题组。</div>}</main>
      <aside className="phase5-inspector"><div className="inspector-section-heading"><span>节点检查器</span><small>{selectedId ?? "未选择"}</small></div>{selectedId ? <><p className="selected-node-name">{String(findEntity(draft, selectedId)?.type ?? "semantic entity")}</p><p className="source-summary">{sourceSummary(selectedAnchors)}</p><dl className="phase5-detail-list"><dt>节点 / 实体</dt><dd>{selectedId}</dd><dt>来源模式</dt><dd>{selectedAnchors[0]?.extractionMode ?? "unknown"}</dd><dt>来源节点</dt><dd>{selectedAnchors.flatMap((anchor) => anchor.nodeIds).join("、") || "—"}</dd></dl></> : <p className="empty">从左侧结构、题干或选项开始。</p>}<SourceOverlay authoring={draft} selectedId={selectedId} /><div className="inspector-section-heading"><span>编辑协议</span><small>V2 patch</small></div><p className="inspector-note">文字输入会在 650ms 后以 replaceText patch 保存；服务器按 base revision 拒绝覆盖并发修改。</p></aside>
    </div>}
  </section>;
}
