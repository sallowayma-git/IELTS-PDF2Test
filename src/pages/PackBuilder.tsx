import { useMemo, useState } from "react";
import { buildPack } from "../api/tauriCommands";
import { StatusPill } from "../components/StatusPill";
import type { ImportJob, PackBuildResult } from "../types";

export function PackBuilder({ jobs, refresh }: { jobs: ImportJob[]; refresh: () => void }) {
  const publishable = useMemo(() => jobs.filter((job) => ["ExportReady", "Exported", "Cleaned"].includes(job.status)), [jobs]);
  const [selected, setSelected] = useState<string[]>([]);
  const [packId, setPackId] = useState("pack-20260529-basic");
  const [version, setVersion] = useState("0.1.0");
  const [institution, setInstitution] = useState("internal");
  const [description, setDescription] = useState("Epic 8 generated pack");
  const [validFrom, setValidFrom] = useState("");
  const [validTo, setValidTo] = useState("");
  const [result, setResult] = useState<PackBuildResult | undefined>();
  const [error, setError] = useState<string | undefined>();

  async function run() {
    setError(undefined);
    try {
      setResult(await buildPack({ packId, version, institution, description, validFrom: validFrom || undefined, validTo: validTo || undefined, jobIds: selected }));
      refresh();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    }
  }

  return (
    <section className="page-enter" data-testid="pack-builder">
      <div className="section-heading spread"><div><p className="eyebrow">Pack Builder</p><h2>组卷与发布</h2></div><button className="primary" data-testid="build-pack" disabled={!selected.length} onClick={run}>生成 Pack</button></div>
      <div className="pack-grid">
        <section className="form-section"><h3>可发布题库</h3>{publishable.map((job) => <label className="pick-row" key={job.jobId}><input type="checkbox" data-testid="pack-job-checkbox" checked={selected.includes(job.jobId)} onChange={(event) => setSelected((current) => event.target.checked ? [...current, job.jobId] : current.filter((id) => id !== job.jobId))} /><span>{job.title}</span><StatusPill status={job.status} /></label>)}{!publishable.length ? <p className="empty">没有 ExportReady 的题目；DraftSaved 只代表可编辑稿已保存，不能进入 Pack 发布。</p> : null}</section>
        <section className="form-section contrast">
          <h3>发布设置</h3>
          <label>packId<input value={packId} onChange={(event) => setPackId(event.target.value)} /></label>
          <label>version<input value={version} onChange={(event) => setVersion(event.target.value)} /></label>
          <label>institution<input value={institution} onChange={(event) => setInstitution(event.target.value)} /></label>
          <label>validFrom<input type="date" value={validFrom} onChange={(event) => setValidFrom(event.target.value)} /></label>
          <label>validTo<input type="date" value={validTo} onChange={(event) => setValidTo(event.target.value)} /></label>
          <label>description<textarea value={description} onChange={(event) => setDescription(event.target.value)} /></label>
        </section>
        <aside className="inspector"><p className="eyebrow">Result</p><h3>{result?.packId ?? "未生成"}</h3>{error ? <p className="error-text" data-testid="pack-error">{error}</p> : null}<pre data-testid="pack-result">{JSON.stringify(result, null, 2)}</pre></aside>
      </div>
    </section>
  );
}
