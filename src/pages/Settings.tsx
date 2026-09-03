import { useEffect, useRef, useState } from "react";
import {
  deleteLlmProfile,
  getDiagnosticsSettings,
  listLlmProfiles,
  runEnvironmentPreflight,
  saveDiagnosticsSettings,
  saveLlmProfile,
  testLlmProfile
} from "../api/tauriCommands";
import type { DiagnosticsSettings, EnvironmentPreflightCheck, EnvironmentPreflightReport, LlmProfilePublic, LlmProvider, LlmTestResult } from "../types";

type ProfileForm = {
  profileId?: string;
  name: string;
  provider: LlmProvider;
  baseUrl: string;
  model: string;
  apiKey: string;
  temperature: number;
  timeoutMs: number;
  forceJson: boolean;
  enabled: boolean;
};

function blankProfileForm(): ProfileForm {
  return {
    name: "OpenAI Compatible",
    provider: "OpenAiCompatible",
    baseUrl: "https://api.openai.com/v1",
    model: "gpt-4.1",
    apiKey: "",
    temperature: 0,
    timeoutMs: 60000,
    forceJson: true,
    enabled: true
  };
}

function toProfileForm(profile?: LlmProfilePublic): ProfileForm {
  return profile ? {
    profileId: profile.profileId,
    name: profile.name,
    provider: profile.provider,
    baseUrl: profile.baseUrl,
    model: profile.model,
    apiKey: "",
    temperature: profile.temperature,
    timeoutMs: profile.timeoutMs,
    forceJson: profile.forceJson,
    enabled: profile.enabled
  } : blankProfileForm();
}

function secretBackendLabel(profile: LlmProfilePublic): string {
  if (!profile.hasApiKey) return "未保存 API Key";
  if (profile.secretStorageBackend === "os" || profile.secretStorageBackend === "keychain") return "API Key 已写入系统安全存储";
  if (profile.secretStorageBackend === "file") return "API Key 已写入文件兜底";
  return "API Key 状态未知";
}

function checkStateLabel(check: EnvironmentPreflightCheck): string {
  if (check.ok) return "正常";
  if (check.severity === "error") return "错误";
  if (check.severity === "warning") return "提醒";
  return "信息";
}

function SettingsPreflight({ preflight, rerun }: { preflight?: EnvironmentPreflightReport; rerun: () => void }) {
  const attention = preflight?.checks.filter((check) => !check.ok) ?? [];
  return (
    <details className="compact-panel">
      <summary>
        <span>
          <strong>环境预检</strong>
          <small>{preflight ? `${preflight.errors} 个错误，${preflight.warnings} 个提醒` : "尚未执行"}</small>
        </span>
        <button className="ghost small" onClick={(event) => { event.preventDefault(); rerun(); }}>重新检测</button>
      </summary>
      {preflight ? (
        <div className="compact-panel-body">
          <div className="preflight-summary mini">
            <div className={preflight.ok ? "good" : preflight.errors ? "bad" : "warn"}>
              <span>状态</span>
              <strong>{preflight.ok ? "可用" : preflight.errors ? "需处理" : "有提醒"}</strong>
            </div>
            <div>
              <span>Error</span>
              <strong>{preflight.errors}</strong>
            </div>
            <div>
              <span>Warning</span>
              <strong>{preflight.warnings}</strong>
            </div>
          </div>
          <div className="preflight-list compact">
            {(attention.length ? attention : preflight.checks.slice(0, 4)).map((check) => (
              <div key={check.name} className={`preflight-inline ${check.ok ? "ok" : check.severity}`}>
                <strong>{check.name}</strong>
                <small>{check.message}</small>
                <em>{checkStateLabel(check)}</em>
              </div>
            ))}
          </div>
        </div>
      ) : <p className="empty compact">尚未执行环境预检。</p>}
    </details>
  );
}

export function Settings({ refresh }: { refresh: () => void }) {
  const [profiles, setProfiles] = useState<LlmProfilePublic[]>([]);
  const [selectedProfileId, setSelectedProfileId] = useState<string | undefined>();
  const [form, setForm] = useState<ProfileForm>(blankProfileForm());
  const [preflight, setPreflight] = useState<EnvironmentPreflightReport | undefined>();
  const [diagnostics, setDiagnostics] = useState<DiagnosticsSettings>({ keepFullProcessArtifacts: false });
  const [saveState, setSaveState] = useState<"idle" | "saving" | "saved" | "error">("idle");
  const [testingProfile, setTestingProfile] = useState(false);
  const [testResult, setTestResult] = useState<LlmTestResult | undefined>();
  const [error, setError] = useState<string | undefined>();
  const [dirtyVersion, setDirtyVersion] = useState(0);
  const suppressAutosave = useRef(true);

  async function load() {
    const [profileList, preflightReport, diagnosticsSettings] = await Promise.all([
      listLlmProfiles(),
      runEnvironmentPreflight(),
      getDiagnosticsSettings()
    ]);
    setProfiles(profileList);
    setPreflight(preflightReport);
    setDiagnostics(diagnosticsSettings);
    setSelectedProfileId((current) => current && profileList.some((profile) => profile.profileId === current) ? current : profileList[0]?.profileId);
    suppressAutosave.current = true;
    setForm((current) => {
      if (current.profileId && profileList.some((profile) => profile.profileId === current.profileId)) return current;
      return toProfileForm(profileList[0]);
    });
  }

  useEffect(() => {
    load().catch((caught) => setError(caught instanceof Error ? caught.message : String(caught)));
  }, []);

  useEffect(() => {
    if (dirtyVersion === 0 || suppressAutosave.current) {
      suppressAutosave.current = false;
      return;
    }
    if (!form.name.trim() || !form.baseUrl.trim() || !form.model.trim()) return;
    const timer = window.setTimeout(async () => {
      setSaveState("saving");
      setError(undefined);
      try {
        const saved = await saveLlmProfile({
          profileId: form.profileId,
          name: form.name.trim(),
          provider: form.provider,
          baseUrl: form.baseUrl.trim(),
          model: form.model.trim(),
          ...(form.apiKey ? { apiKey: form.apiKey } : {}),
          temperature: form.temperature,
          timeoutMs: form.timeoutMs,
          forceJson: form.forceJson,
          enabled: form.enabled
        });
        const nextProfiles = await listLlmProfiles();
        setProfiles(nextProfiles);
        setSelectedProfileId(saved.profileId);
        suppressAutosave.current = true;
        setForm((current) => ({ ...current, profileId: saved.profileId, apiKey: "" }));
        setSaveState("saved");
        refresh();
      } catch (caught) {
        setSaveState("error");
        setError(caught instanceof Error ? caught.message : String(caught));
      }
    }, 700);
    return () => window.clearTimeout(timer);
  }, [dirtyVersion, form, refresh]);

  function selectProfile(profile?: LlmProfilePublic) {
    suppressAutosave.current = true;
    setSelectedProfileId(profile?.profileId);
    setForm(toProfileForm(profile));
    setTestResult(undefined);
    setError(undefined);
    setSaveState("idle");
  }

  async function removeProfile(profileId: string) {
    setError(undefined);
    try {
      const nextProfiles = await deleteLlmProfile(profileId);
      setProfiles(nextProfiles);
      refresh();
      selectProfile(nextProfiles[0]);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    }
  }

  async function runProfileTest() {
    if (!form.profileId) {
      setTestResult(undefined);
      setError("请先保存配置后再测试连接；未保存的新配置不会参与测试。");
      return;
    }
    setError(undefined);
    setTestingProfile(true);
    try {
      setTestResult(await testLlmProfile(form.profileId));
    } catch (caught) {
      setTestResult(undefined);
      setError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setTestingProfile(false);
    }
  }

  async function rerunPreflight() {
    setError(undefined);
    try {
      setPreflight(await runEnvironmentPreflight());
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    }
  }

  async function toggleArtifactRetention(enabled: boolean) {
    const next = await saveDiagnosticsSettings({ keepFullProcessArtifacts: enabled });
    setDiagnostics(next);
    refresh();
  }

  const update = <K extends keyof ProfileForm>(key: K, value: ProfileForm[K]) => {
    setForm((current) => ({ ...current, [key]: value }));
    setDirtyVersion((current) => current + 1);
  };

  return (
    <section className="page-enter">
      <div className="section-heading">
        <p className="eyebrow">Settings</p>
        <h2>模型与环境设置</h2>
      </div>

      <div className="settings-studio">
        <aside className="settings-profile-list">
          <div className="spread">
            <h3>模型配置</h3>
            <button className="ghost small" onClick={() => selectProfile(undefined)}>新建配置</button>
          </div>
          <div className="profile-stack">
            {profiles.map((profile) => (
              <article key={profile.profileId} className={`profile-card ${selectedProfileId === profile.profileId ? "active" : ""}`}>
                <button className="profile-card-main" onClick={() => selectProfile(profile)}>
                  <strong>{profile.name}</strong>
                  <small>{profile.model} · {profile.baseUrl}</small>
                  <small>{secretBackendLabel(profile)}</small>
                </button>
                <div className="profile-card-actions">
                  <button className="ghost small" onClick={() => selectProfile(profile)}>编辑</button>
                  <button className="ghost small" onClick={() => void removeProfile(profile.profileId)}>删除</button>
                </div>
              </article>
            ))}
            {!profiles.length ? <p className="empty compact">还没有模型配置。右上角可以直接新建。</p> : null}
          </div>
        </aside>

        <section className="settings-editor">
          <div className="spread">
            <div>
              <h3>{form.profileId ? "编辑配置" : "新建配置"}</h3>
              <p className="settings-save-hint">
                {saveState === "saving" ? "正在自动保存..." : saveState === "saved" ? "已自动保存" : "修改后会自动保存到当前配置"}
              </p>
            </div>
            {form.profileId ? <button className="ghost small" data-testid="test-llm-profile" onClick={() => void runProfileTest()} disabled={testingProfile}>{testingProfile ? "测试中..." : "测试连接"}</button> : null}
          </div>

          <div className="settings-editor-grid">
            <label>名称
              <input value={form.name} onChange={(event) => update("name", event.target.value)} />
            </label>
            <label>Provider
              <select value={form.provider} onChange={(event) => update("provider", event.target.value as LlmProvider)}>
                <option value="OpenAiCompatible">OpenAiCompatible</option>
                <option value="AnthropicCompatible">AnthropicCompatible</option>
                <option value="Ollama">Ollama</option>
                <option value="Custom">Custom</option>
              </select>
            </label>
            <label>Base URL
              <input value={form.baseUrl} onChange={(event) => update("baseUrl", event.target.value)} placeholder="https://api.openai.com/v1" />
            </label>
            <label>Model
              <input value={form.model} onChange={(event) => update("model", event.target.value)} />
            </label>
            <label>API Key
              <input type="password" value={form.apiKey} onChange={(event) => update("apiKey", event.target.value)} placeholder={form.profileId ? "留空则保留已保存密钥" : "填写后自动保存"} />
            </label>
            <label>Timeout ms
              <input type="number" min="1000" step="1000" value={form.timeoutMs} onChange={(event) => update("timeoutMs", Number(event.target.value))} />
            </label>
            <label>Temperature
              <input type="number" min="0" max="1" step="0.1" value={form.temperature} onChange={(event) => update("temperature", Number(event.target.value))} />
            </label>
            <div className="settings-checks">
              <label className="inline-check">
                <input type="checkbox" checked={form.forceJson} onChange={(event) => update("forceJson", event.target.checked)} />
                强制 JSON 输出
              </label>
              <label className="inline-check">
                <input type="checkbox" checked={form.enabled} onChange={(event) => update("enabled", event.target.checked)} />
                启用当前配置
              </label>
            </div>
          </div>

          {testResult ? (
            <div className={testResult.ok ? "info-box" : "warning-box"}>
              <strong>{testResult.ok ? "连接成功" : "连接失败"}</strong>
              <p>{testResult.message}</p>
              <small>{testResult.latencyMs} ms</small>
            </div>
          ) : null}
          {error ? <p className="error-text">{error}</p> : null}

          <div className="settings-secondary-panels">
            <SettingsPreflight preflight={preflight} rerun={() => void rerunPreflight()} />
            <details className="compact-panel">
              <summary>
                <span>
                  <strong>过程文件保留</strong>
                  <small>{diagnostics.keepFullProcessArtifacts ? "当前保留完整中间文件" : "导出后自动清理中间文件"}</small>
                </span>
              </summary>
              <div className="compact-panel-body">
                <label className="inline-check">
                  <input type="checkbox" checked={diagnostics.keepFullProcessArtifacts} onChange={(event) => void toggleArtifactRetention(event.target.checked)} />
                  保留完整过程文件
                </label>
              </div>
            </details>
          </div>
        </section>
      </div>
    </section>
  );
}
