import { useEffect, useState } from "react";
import { listLlmProfiles, saveLlmProfile, testLlmProfile } from "../api/tauriCommands";
import type { LlmProfilePublic, LlmProvider, LlmTestResult } from "../types";

export function Settings({ refresh }: { refresh: () => void }) {
  const [profiles, setProfiles] = useState<LlmProfilePublic[]>([]);
  const [result, setResult] = useState<LlmTestResult | undefined>();
  const [form, setForm] = useState({ name: "OpenAI Compatible", provider: "OpenAiCompatible" as LlmProvider, baseUrl: "https://api.openai.com/v1", model: "gpt-4.1", apiKey: "", temperature: 0, timeoutMs: 60000, forceJson: true, enabled: true });

  async function load() {
    setProfiles(await listLlmProfiles());
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

  const update = <K extends keyof typeof form>(key: K, value: (typeof form)[K]) => setForm((current) => ({ ...current, [key]: value }));

  return (
    <section className="page-enter">
      <div className="section-heading"><p className="eyebrow">Settings</p><h2>设置</h2></div>
      <div className="settings-grid">
        <section className="form-section">
          <h3>大模型 Profiles</h3>
          {profiles.map((profile) => (
            <div className="profile-row" key={profile.profileId}>
              <span>
                <strong>{profile.name}</strong>
                <small>{profile.provider} · {profile.model} · {profile.baseUrl}</small>
                <small>{profile.hasApiKey ? `API Key 已写入 ${profile.secretStorageBackend === "keychain" ? "macOS Keychain" : "本地文件兜底"}` : "无 API Key，将使用本地确定性建议"}</small>
                {profile.apiKeySecretRef ? <small>secretRef: {profile.apiKeySecretRef}</small> : null}
                {profile.secretStorageMessage ? <small>{profile.secretStorageMessage}</small> : null}
              </span>
              <button className="ghost small" onClick={() => test(profile.profileId)}>测试连接</button>
            </div>
          ))}
          <pre>{result ? JSON.stringify(result, null, 2) : "No test run."}</pre>
        </section>
        <section className="form-section contrast">
          <h3>新建 Profile</h3>
          <label>Name<input value={form.name} onChange={(event) => update("name", event.target.value)} /></label>
          <label>Provider<select value={form.provider} onChange={(event) => update("provider", event.target.value as LlmProvider)}><option>OpenAiCompatible</option></select><small>当前 gateway 只实现 OpenAI-compatible chat completions；其他 provider 需要新增 adapter 后再开放。</small></label>
          <label>Base URL<input value={form.baseUrl} onChange={(event) => update("baseUrl", event.target.value)} /></label>
          <label>Model<input value={form.model} onChange={(event) => update("model", event.target.value)} /></label>
          <label>API Key<input type="password" value={form.apiKey} onChange={(event) => update("apiKey", event.target.value)} placeholder="优先写入 macOS Keychain，不进入 profile JSON" /></label>
          <label>Temperature<input type="number" min="0" max="1" step="0.1" value={form.temperature} onChange={(event) => update("temperature", Number(event.target.value))} /></label>
          <label>Timeout ms<input type="number" min="1000" step="1000" value={form.timeoutMs} onChange={(event) => update("timeoutMs", Number(event.target.value))} /></label>
          <label className="inline-check"><input type="checkbox" checked={form.forceJson} onChange={(event) => update("forceJson", event.target.checked)} /> 强制 JSON 输出</label>
          <label className="inline-check"><input type="checkbox" checked={form.enabled} onChange={(event) => update("enabled", event.target.checked)} /> 启用</label>
          <button className="primary wide" onClick={save}>保存 Profile</button>
        </section>
        <aside className="inspector"><p className="eyebrow">Storage and parser</p><h3>本地权限边界</h3><p>允许 app data、用户显式选择输入文件、用户显式选择导出目录。API Key 优先写入 macOS Keychain；Keychain 不可用时才写入 app data 文件兜底；普通 profile JSON 只保存 secret ref 和后端状态。LLM 输出只允许结构化 JSON 建议，不能直接生成最终 JS。</p></aside>
      </div>
    </section>
  );
}
