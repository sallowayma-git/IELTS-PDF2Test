import { describe, expect, it, beforeEach } from "vitest";
import { cloudShouldBeEnabledAfterSave, describePipelineMode, hasCloudConnection, normalizePipelineMode } from "./settingsLogic";
import { DEFAULT_APP_SETTINGS, readAppSettings } from "./appSettings";

describe("保存有效密钥即启用云端", () => {
  it("连接测试通过且有密钥：即使没勾选开关也启用", () => {
    expect(cloudShouldBeEnabledAfterSave({ toggle: false, testOk: true, hasKey: true, provider: "OpenAiCompatible" })).toBe(true);
  });
  it("测试没通过：不擅自启用（沿用开关）", () => {
    expect(cloudShouldBeEnabledAfterSave({ toggle: false, testOk: false, hasKey: true, provider: "OpenAiCompatible" })).toBe(false);
    expect(cloudShouldBeEnabledAfterSave({ toggle: true, testOk: false, hasKey: true, provider: "OpenAiCompatible" })).toBe(true);
  });
  it("本地 Ollama 不需要密钥", () => {
    expect(cloudShouldBeEnabledAfterSave({ toggle: false, testOk: true, hasKey: false, provider: "Ollama" })).toBe(true);
  });
});

describe("设置里没有摆设", () => {
  beforeEach(() => {
    const store = new Map<string, string>();
    (globalThis as { window?: unknown }).window = {
      localStorage: {
        getItem: (key: string) => store.get(key) ?? null,
        setItem: (key: string, value: string) => { store.set(key, value); },
        removeItem: (key: string) => { store.delete(key); }
      }
    };
  });
  it("不再有只被设置页读写、对产品毫无作用的字段", () => {
    const keys = Object.keys(DEFAULT_APP_SETTINGS);
    expect(keys).not.toContain("keepSourceFiles");
    expect(keys).not.toContain("localConcurrency");
    expect(keys).not.toContain("cloudConcurrency");
    expect(Object.keys(readAppSettings())).toEqual(keys);
  });
  it("云端 token 预算默认值与导入端一致", () => {
    expect(DEFAULT_APP_SETTINGS.cloudTokenBudget).toBe(300_000);
    expect(readAppSettings().cloudTokenBudget).toBe(300_000);
  });
});

describe("导入抽屉的云端提示", () => {
  it("只有启用中的真实连接才算已连接", () => {
    expect(hasCloudConnection([{ profileId: "profile-local-placeholder", enabled: true }])).toBe(false);
    expect(hasCloudConnection([{ profileId: "p1", enabled: false }])).toBe(false);
    expect(hasCloudConnection([{ profileId: "p1", enabled: true }])).toBe(true);
  });
});

describe("流水线模式", () => {
  it("默认 direct；无法识别的值不把界面卡死", () => {
    expect(normalizePipelineMode(undefined)).toBe("direct");
    expect(normalizePipelineMode("cloud")).toBe("direct");
    expect(normalizePipelineMode("")).toBe("direct");
    expect(normalizePipelineMode("legacy")).toBe("legacy");
    expect(normalizePipelineMode("direct")).toBe("direct");
  });
  it("两种模式都有可读说明，且互不相同", () => {
    expect(describePipelineMode("direct")).toContain("直出");
    expect(describePipelineMode("legacy")).toContain("本地");
    expect(describePipelineMode("direct")).not.toBe(describePipelineMode("legacy"));
  });
});
