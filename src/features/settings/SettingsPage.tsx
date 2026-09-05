import { useEffect, useMemo, useState } from "react";
import {
  getDiagnosticsSettings,
  listLlmProfiles,
  runEnvironmentPreflight,
  saveDiagnosticsSettings,
  saveLlmProfile,
  testLlmProfile
} from "../../api/tauriCommands";
import { chooseExportDirectory } from "../../api/desktopDialogs";
import type { DiagnosticsSettings, EnvironmentPreflightReport, LlmProfilePublic, LlmTestResult } from "../../types";
import { useAppSettings } from "./appSettings";

// 设置页（计划 §14 / §16.13）：默认只有模型连接、文件与发布目标，其他收进「高级设置」。
//
// 相对旧页面移除的东西：
//  - 多 Profile 管理器（普通用户只维护一个 active profile，§14.3）
//  - `forceJson` 复选框（产品逻辑固定开启，§14.2）
//  - `temperature` 输入（固定 0，除开发者模式）
//  - `enabled` 复选框（由「启用云端识别」总开关表达）
//  - 不受 gateway 支持的 Provider 选项（AnthropicCompatible / Custom，见 findings F18）
//  - 常驻的环境预检卡片堆叠（只在有错误时出一条摘要）
//  - API Key 安全存储 backend 技术文案
//
// 保存策略（§14.4）：不再每输入一个字符 700ms 自动保存并立即生效，改为显式「保存并测试」。
const LOCAL_PLACEHOLDER_PROFILE = "profile-local-placeholder";

/** gateway 只路由这两种协议（llm_gateway.rs:146），所以 UI 只给这两种。 */
const SUPPORTED_PROVIDERS = [
  { value: "OpenAiCompatible", label: "OpenAI 兼容" },
  { value: "Ollama", label: "Ollama（本地）" }
] as const;

type ProviderValue = (typeof SUPPORTED_PROVIDERS)[number]["value"];

interface ModelForm {
  profileId?: string;
  name: string;
  provider: ProviderValue;
  baseUrl: string;
  model: string;
  apiKey: string;
  timeoutMs: number;
  hasApiKey: boolean;
}

function blankForm(): ModelForm {
  return {
    name: "默认模型",
    provider: "OpenAiCompatible",
    baseUrl: "https://api.openai.com/v1",
    model: "gpt-4.1",
    apiKey: "",
    timeoutMs: 60000,
    hasApiKey: false
  };
}

function toForm(profile: LlmProfilePublic): ModelForm {
  return {
    profileId: profile.profileId,
    name: profile.name,
    provider: profile.provider === "Ollama" ? "Ollama" : "OpenAiCompatible",
    baseUrl: profile.baseUrl,
    model: profile.model,
    apiKey: "",
    timeoutMs: profile.timeoutMs,
    hasApiKey: profile.hasApiKey
  };
}

/** 普通用户面前的「云端识别」= 存在一个启用中的真实 profile。 */
function activeProfileOf(profiles: LlmProfilePublic[]): LlmProfilePublic | undefined {
  const real = profiles.filter((profile) => profile.profileId !== LOCAL_PLACEHOLDER_PROFILE);
  return real.find((profile) => profile.enabled) ?? real[0];
}

export function SettingsPage() {
  const [settings, updateSettings] = useAppSettings();
  const [profiles, setProfiles] = useState<LlmProfilePublic[]>([]);
  const [form, setForm] = useState<ModelForm>(blankForm());
  const [cloudEnabled, setCloudEnabled] = useState(false);
  const [diagnostics, setDiagnostics] = useState<DiagnosticsSettings>({ keepFullProcessArtifacts: false });
  const [preflight, setPreflight] = useState<EnvironmentPreflightReport | undefined>();
  const [busy, setBusy] = useState<"idle" | "saving" | "testing">("idle");
  const [testResult, setTestResult] = useState<LlmTestResult | undefined>();
  const [notice, setNotice] = useState<string | undefined>();
  const [error, setError] = useState<string | undefined>();
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [reloadTick, setReloadTick] = useState(0);

  useEffect(() => {
    let cancelled = false;
    Promise.all([
      listLlmProfiles().catch(() => [] as LlmProfilePublic[]),
      getDiagnosticsSettings().catch(() => ({ keepFullProcessArtifacts: false })),
      runEnvironmentPreflight().catch(() => undefined)
    ]).then(([profileList, diagnosticsSettings, preflightReport]) => {
      if (cancelled) return;
      setProfiles(profileList);
      setDiagnostics(diagnosticsSettings);
      setPreflight(preflightReport);
      const active = activeProfileOf(profileList);
      setForm(active ? toForm(active) : blankForm());
      setCloudEnabled(Boolean(active?.enabled));
    });
    return () => {
      cancelled = true;
    };
  }, [reloadTick]);

  // 只有阻断项才需要出现在普通设置页（计划 §14.2 / §17.23）。
  const blockingChecks = useMemo(
    () => preflight?.checks.filter((check) => !check.ok && check.severity === "error") ?? [],
    [preflight]
  );

  function patchForm(patch: Partial<ModelForm>) {
    setForm((current) => ({ ...current, ...patch }));
    setTestResult(undefined);
    setNotice(undefined);
  }

  async function saveAndTest() {
    setBusy("saving");
    setError(undefined);
    setNotice(undefined);
    setTestResult(undefined);
    try {
      const saved = await saveLlmProfile({
        profileId: form.profileId,
        name: form.name.trim() || "默认模型",
        provider: form.provider,
        baseUrl: form.baseUrl.trim(),
        model: form.model.trim(),
        // 留空表示不改动已保存的 Key。
        apiKey: form.apiKey ? form.apiKey : undefined,
        // 产品逻辑固定：温度 0、强制 JSON。不给普通用户开关。
        temperature: 0,
        timeoutMs: form.timeoutMs,
        forceJson: true,
        enabled: cloudEnabled
      });
      setBusy("testing");
      const result = await testLlmProfile(saved.profileId);
      setTestResult(result);
      setNotice(result.ok ? "已保存，连接正常。" : "已保存，但连接测试未通过。");
      setReloadTick((value) => value + 1);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setBusy("idle");
    }
  }

  async function toggleCloud(next: boolean) {
    setCloudEnabled(next);
    setNotice(undefined);
    // 关闭是即时生效的安全动作；开启需要用户点「保存并测试」确认连接可用。
    if (next || !form.profileId) return;
    try {
      await saveLlmProfile({
        profileId: form.profileId,
        name: form.name,
        provider: form.provider,
        baseUrl: form.baseUrl,
        model: form.model,
        temperature: 0,
        timeoutMs: form.timeoutMs,
        forceJson: true,
        enabled: false
      });
      setNotice("已关闭云端识别，导入只跑本地识别。");
      setReloadTick((value) => value + 1);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    }
  }

  async function pickDestination() {
    const picked = await chooseExportDirectory();
    if (!picked) return;
    updateSettings({ nasDestination: picked });
    setNotice("已记住发布目录，之后发布不用再选。");
  }

  async function setKeepArtifacts(next: boolean) {
    try {
      setDiagnostics(await saveDiagnosticsSettings({ keepFullProcessArtifacts: next }));
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    }
  }

  return (
    <section className="settings-page" data-testid="settings-page">
      <header className="settings-header">
        <h1>设置</h1>
      </header>

      {blockingChecks.length ? (
        <p className="settings-preflight-alert" role="alert" data-testid="settings-preflight-alert">
          运行环境有 {blockingChecks.length} 项需要处理：{blockingChecks.map((check) => check.message).join("；")}
        </p>
      ) : null}
      {error ? <p className="error-text" data-testid="settings-error">{error}</p> : null}
      {notice ? <p className="settings-notice" role="status" data-testid="settings-notice">{notice}</p> : null}

      <section className="settings-group">
        <h2>模型识别</h2>
        <label className="settings-toggle">
          <input
            type="checkbox"
            checked={cloudEnabled}
            data-testid="settings-cloud-enabled"
            onChange={(event) => void toggleCloud(event.target.checked)}
          />
          <span>
            启用云端识别
            <small>开启后，导入时云端模型与本地识别并发运行，结果需要确认后才会进入题稿。</small>
          </span>
        </label>

        <label className="settings-field">
          服务地址
          <input
            value={form.baseUrl}
            data-testid="settings-base-url"
            placeholder="https://api.openai.com/v1"
            onChange={(event) => patchForm({ baseUrl: event.target.value })}
          />
        </label>

        <label className="settings-field">
          模型
          <input
            value={form.model}
            data-testid="settings-model"
            placeholder="gpt-4.1"
            onChange={(event) => patchForm({ model: event.target.value })}
          />
        </label>

        <label className="settings-field">
          API Key
          <input
            type="password"
            value={form.apiKey}
            data-testid="settings-api-key"
            placeholder={form.hasApiKey ? "已保存，留空表示不修改" : "粘贴 API Key"}
            onChange={(event) => patchForm({ apiKey: event.target.value })}
          />
        </label>

        <div className="button-row">
          <button
            className="primary"
            data-testid="settings-save-test"
            disabled={busy !== "idle"}
            onClick={() => void saveAndTest()}
          >
            {busy === "saving" ? "正在保存…" : busy === "testing" ? "正在测试连接…" : "保存并测试"}
          </button>
          {testResult ? (
            <span className={testResult.ok ? "settings-test-result ok" : "settings-test-result bad"} data-testid="settings-test-result">
              {testResult.ok ? "连接正常 · " + testResult.latencyMs + "ms" : testResult.message}
            </span>
          ) : null}
        </div>
      </section>

      <section className="settings-group">
        <h2>文件</h2>
        <label className="settings-toggle">
          <input
            type="checkbox"
            checked={settings.keepSourceFiles}
            data-testid="settings-keep-source"
            onChange={(event) => updateSettings({ keepSourceFiles: event.target.checked })}
          />
          <span>
            保留原始 PDF 以便后续核对
            <small>关闭后，题目首次确认或发布成功即删除原文件；发现题干错漏时将失去来源对照。</small>
          </span>
        </label>
      </section>

      <section className="settings-group">
        <h2>发布</h2>
        <div className="settings-field-row">
          <span className="settings-field-label">
            NAS 目录
            <small className="file-name">{settings.nasDestination || "尚未选择，首次发布时会提示选择"}</small>
          </span>
          <button className="ghost small" data-testid="settings-pick-nas" onClick={() => void pickDestination()}>
            {settings.nasDestination ? "更换目录" : "选择目录"}
          </button>
        </div>
      </section>

      <section className="settings-group">
        <button
          className="settings-advanced-toggle"
          aria-expanded={advancedOpen}
          data-testid="settings-advanced-toggle"
          onClick={() => setAdvancedOpen((open) => !open)}
        >
          高级设置 {advancedOpen ? "▾" : "▸"}
        </button>

        {advancedOpen ? (
          <div className="settings-advanced" data-testid="settings-advanced">
            <label className="settings-field">
              协议
              <select
                value={form.provider}
                data-testid="settings-provider"
                onChange={(event) => patchForm({ provider: event.target.value as ProviderValue })}
              >
                {SUPPORTED_PROVIDERS.map((provider) => (
                  <option key={provider.value} value={provider.value}>{provider.label}</option>
                ))}
              </select>
              <small>只列出当前真正支持的协议；新增协议需要先有对应的请求适配器和测试。</small>
            </label>

            <label className="settings-field">
              请求超时（毫秒）
              <input
                type="number"
                min={5000}
                max={600000}
                step={1000}
                value={form.timeoutMs}
                data-testid="settings-timeout"
                onChange={(event) => patchForm({ timeoutMs: Number(event.target.value) })}
              />
            </label>

            <label className="settings-field">
              本地并发数
              <input
                type="number"
                min={1}
                max={4}
                value={settings.localConcurrency}
                data-testid="settings-local-concurrency"
                onChange={(event) => updateSettings({ localConcurrency: Number(event.target.value) })}
              />
            </label>

            <label className="settings-field">
              云端并发数
              <input
                type="number"
                min={1}
                max={4}
                value={settings.cloudConcurrency}
                data-testid="settings-cloud-concurrency"
                onChange={(event) => updateSettings({ cloudConcurrency: Number(event.target.value) })}
              />
            </label>

            <label className="settings-toggle">
              <input
                type="checkbox"
                checked={settings.developerMode}
                data-testid="settings-developer-mode"
                onChange={(event) => updateSettings({ developerMode: event.target.checked })}
              />
              <span>开发者模式<small>显示技术日志入口、完整环境诊断和过程文件保留开关。</small></span>
            </label>

            {settings.developerMode ? (
              <>
                <label className="settings-toggle">
                  <input
                    type="checkbox"
                    checked={diagnostics.keepFullProcessArtifacts}
                    data-testid="settings-keep-artifacts"
                    onChange={(event) => void setKeepArtifacts(event.target.checked)}
                  />
                  <span>保留完整过程文件<small>用于排查识别问题；会显著增加磁盘占用。</small></span>
                </label>

                <details className="settings-diagnostics" data-testid="settings-diagnostics">
                  <summary>
                    环境诊断
                    {preflight ? "（" + preflight.errors + " 错误 · " + preflight.warnings + " 提醒）" : "（尚未执行）"}
                  </summary>
                  {preflight ? (
                    <ul>
                      {preflight.checks.map((check) => (
                        <li key={check.name} className={check.ok ? "ok" : check.severity}>
                          <strong>{check.name}</strong>
                          <span>{check.message}</span>
                        </li>
                      ))}
                    </ul>
                  ) : (
                    <p className="empty compact">环境预检未返回结果。</p>
                  )}
                  <button className="ghost small" onClick={() => setReloadTick((value) => value + 1)}>重新检测</button>
                </details>
              </>
            ) : null}
          </div>
        ) : null}
      </section>
    </section>
  );
}
