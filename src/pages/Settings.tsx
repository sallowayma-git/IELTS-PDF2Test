import { useEffect, useState } from "react";
import { getDiagnosticsSettings, listLlmProfiles, runEnvironmentPreflight, saveDiagnosticsSettings, saveLlmProfile, testLlmProfile } from "../api/tauriCommands";
import type { DiagnosticsSettings, EnvironmentPreflightReport, LlmProfilePublic, LlmProvider, LlmTestResult } from "../types";

function secretBackendLabel(profile: LlmProfilePublic) {
  if (!profile.hasApiKey) return "无 API Key，将使用本地确定性建议";
  if (profile.secretStorageBackend === "os" || profile.secretStorageBackend === "keychain") return "API Key 已写入系统安全存储";
  if (profile.secretStorageBackend === "file") return "API Key 已写入明文文件兜底（仅 dev/emergency）";
  return "API Key 状态未知";
}

export function Settings({ refresh }: { refresh: () => void }) {
  const [profiles, setProfiles] = useState<LlmProfilePublic[]>([]);
  const [result, setResult] = useState<LlmTestResult | undefined>();
  const [preflight, setPreflight] = useState<EnvironmentPreflightReport | undefined>();
  const [diagnostics, setDiagnostics] = useState<DiagnosticsSettings>({ keepFullProcessArtifacts: false });
  const [form, setForm] = useState({ name: "OpenAI Compatible", provider: "OpenAiCompatible" as LlmProvider, baseUrl: "https://api.openai.com/v1", model: "gpt-4.1", apiKey: "", temperature: 0, timeoutMs: 60000, forceJson: true, enabled: true });

  async function load() {
    const [profileList, preflightReport, diagnosticsSettings] = await Promise.all([listLlmProfiles(), runEnvironmentPreflight(), getDiagnosticsSettings()]);
    setProfiles(profileList);
    setPreflight(preflightReport);
    setDiagnostics(diagnosticsSettings);
  }

  useEffect(() => {
    load().catch(console.error);
  }, []);

  async function save() {
    await saveLlmProfile(form);
    await load();
    refresh();
  }

  async function test(profileId: string) {
    setResult(await testLlmProfile(profileId));
  }

  async function rerunPreflight() {
    setPreflight(await runEnvironmentPreflight());
  }

  async function toggleArtifactRetention(enabled: boolean) {
    const next = await saveDiagnosticsSettings({ keepFullProcessArtifacts: enabled });
    setDiagnostics(next);
    refresh();
  }

  const update = <K extends keyof typeof form>(key: K, value: (typeof form)[K]) => setForm((current) => ({ ...current, [key]: value }));

  return (
    <section className="page-enter">
      <div className="section-heading"><p className="eyebrow">Settings</p><h2>设置</h2></div>
      <div className="settings-grid">
        <section className="form-section">
          <div className="spread">
            <h3>运行环境预检</h3>
            <button className="ghost small" onClick={rerunPreflight}>重新检测</button>
          </div>
          {preflight ? (
            <>
              <p className={preflight.ok ? "success-text" : "error-text"}>
                {preflight.ok ? "关键依赖可用" : `关键依赖缺失：${preflight.errors} error(s)`} · warnings {preflight.warnings}
              </p>
              <div className="layer-list">
                {preflight.checks.map((check) => (
                  <div key={check.name}>
                    <span>
                      <strong>{check.ok ? "OK" : check.severity.toUpperCase()} · {check.name}</strong>
                      <small>{check.message}</small>
                    </span>
                  </div>
                ))}
              </div>
            </>
          ) : <p className="empty">尚未执行环境预检。</p>}
        </section>
        <section className="form-section">
          <h3>大模型配置</h3>
          {profiles.map((profile) => (
            <div className="profile-row" key={profile.profileId}>
              <span>
                <strong>{profile.name}</strong>
                <small>{profile.provider} · {profile.model} · {profile.baseUrl}</small>
                <small>{secretBackendLabel(profile)}</small>
                {profile.apiKeySecretRef ? <small>密钥已保存</small> : null}
                {profile.secretStorageMessage ? <small>{profile.secretStorageMessage}</small> : null}
              </span>
              <button className="ghost small" onClick={() => test(profile.profileId)}>测试连接</button>
            </div>
          ))}
          <pre>{result ? JSON.stringify(result, null, 2) : "尚未测试连接。"}</pre>
        </section>
        <section className="form-section contrast">
          <h3>新建模型配置</h3>
          <label>名称<input value={form.name} onChange={(event) => update("name", event.target.value)} /></label>
          <label>Provider<select value={form.provider} onChange={(event) => update("provider", event.target.value as LlmProvider)}><option>OpenAiCompatible</option></select><small>当前 gateway 只实现 OpenAI-compatible chat completions；其他 provider 需要新增 adapter 后再开放。</small></label>
          <label>Base URL<input value={form.baseUrl} onChange={(event) => update("baseUrl", event.target.value)} /></label>
          <label>Model<input value={form.model} onChange={(event) => update("model", event.target.value)} /></label>
          <label>API Key<input type="password" value={form.apiKey} onChange={(event) => update("apiKey", event.target.value)} placeholder="写入系统安全存储；明文文件兜底默认禁用" /></label>
          <label>Temperature<input type="number" min="0" max="1" step="0.1" value={form.temperature} onChange={(event) => update("temperature", Number(event.target.value))} /></label>
          <label>Timeout ms<input type="number" min="1000" step="1000" value={form.timeoutMs} onChange={(event) => update("timeoutMs", Number(event.target.value))} /></label>
          <label className="inline-check"><input type="checkbox" checked={form.forceJson} onChange={(event) => update("forceJson", event.target.checked)} /> 强制 JSON 输出</label>
          <label className="inline-check"><input type="checkbox" checked={form.enabled} onChange={(event) => update("enabled", event.target.checked)} /> 启用</label>
          <button className="primary wide" onClick={save}>保存模型配置</button>
        </section>
        <aside className="inspector">
          <p className="eyebrow">Developer / Diagnostics</p>
          <h3>过程文件保留</h3>
          <p>默认关闭。导出成功后会自动清理 uploads/cache/preview/LLM raw log 等过程文件，只保留可编辑题目稿和摘要。</p>
          <label className="inline-check"><input type="checkbox" checked={diagnostics.keepFullProcessArtifacts} onChange={(event) => void toggleArtifactRetention(event.target.checked)} /> 保留完整过程文件</label>
          {diagnostics.keepFullProcessArtifacts ? <p className="warning-box">已开启调试保留：导出后不会自动删除完整过程文件。仅用于排查解析或模型问题。</p> : <p className="success-text">普通模式：导出后自动清理中间文件。</p>}
        </aside>
      </div>
    </section>
  );
}
