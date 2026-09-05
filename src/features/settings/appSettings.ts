import { useCallback, useEffect, useState } from "react";

// 本地应用偏好（计划 §14）。
//
// 这些是「产品行为偏好」，不是模型连接（模型连接是后端 LlmProfile）。后端目前没有对应的设置命令，
// 因此先落在 localStorage；P9 把源文件保留策略和并发上限交给 Rust 后，把这里换成
// get_app_settings / save_app_settings 即可，读写点已经收敛到本文件。
//
// NAS 目录之前分散在 ExportPage / LibraryPage / ExamWorkspacePage 各自读同一个 localStorage key，
// 现在统一从这里读写，避免三处各自演化。
const STORAGE_KEY = "ielts-author-studio.app-settings.v1";
/** 与旧 ExportPage 共用的历史 key，迁移期继续兼容读取。 */
const LEGACY_NAS_KEY = "ielts-author-studio.confirmed-nas-export-dir.v1";

export interface AppSettingsV1 {
  /** 默认开启：一旦立刻只保留 DS，用户之后发现题干错漏就失去本地来源对照（计划 §4.5）。 */
  keepSourceFiles: boolean;
  /** 发布目标目录，只选一次后记住（计划 §13.2）。 */
  nasDestination: string;
  localConcurrency: number;
  cloudConcurrency: number;
  /** 开发者模式：技术日志、完整环境诊断、过程文件保留开关才出现。 */
  developerMode: boolean;
}

export const DEFAULT_APP_SETTINGS: Readonly<AppSettingsV1> = Object.freeze({
  keepSourceFiles: true,
  nasDestination: "",
  localConcurrency: 2,
  cloudConcurrency: 2,
  developerMode: false
});

function clampConcurrency(value: unknown, fallback: number): number {
  const parsed = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(parsed)) return fallback;
  return Math.min(4, Math.max(1, Math.round(parsed)));
}

export function readAppSettings(): AppSettingsV1 {
  let stored: Partial<AppSettingsV1> = {};
  try {
    stored = JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? "{}") as Partial<AppSettingsV1>;
  } catch {
    stored = {};
  }
  const legacyNas = window.localStorage.getItem(LEGACY_NAS_KEY)?.trim() ?? "";
  return {
    keepSourceFiles: typeof stored.keepSourceFiles === "boolean" ? stored.keepSourceFiles : DEFAULT_APP_SETTINGS.keepSourceFiles,
    nasDestination: (stored.nasDestination ?? legacyNas).trim(),
    localConcurrency: clampConcurrency(stored.localConcurrency, DEFAULT_APP_SETTINGS.localConcurrency),
    cloudConcurrency: clampConcurrency(stored.cloudConcurrency, DEFAULT_APP_SETTINGS.cloudConcurrency),
    developerMode: stored.developerMode === true
  };
}

const listeners = new Set<(settings: AppSettingsV1) => void>();

export function writeAppSettings(patch: Partial<AppSettingsV1>): AppSettingsV1 {
  const next = { ...readAppSettings(), ...patch };
  next.localConcurrency = clampConcurrency(next.localConcurrency, DEFAULT_APP_SETTINGS.localConcurrency);
  next.cloudConcurrency = clampConcurrency(next.cloudConcurrency, DEFAULT_APP_SETTINGS.cloudConcurrency);
  next.nasDestination = next.nasDestination.trim();
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
    // 迁移期同时写回旧 key，兼容仍在使用它的旧导出页。
    if (next.nasDestination) window.localStorage.setItem(LEGACY_NAS_KEY, next.nasDestination);
  } catch {
    // localStorage 不可用时只影响持久化，不影响当前会话。
  }
  for (const listener of listeners) listener(next);
  return next;
}

/** 订阅式读取，让设置页改完之后题库与工作区不用刷新也能拿到新值。 */
export function useAppSettings(): [AppSettingsV1, (patch: Partial<AppSettingsV1>) => void] {
  const [settings, setSettings] = useState<AppSettingsV1>(() => readAppSettings());
  useEffect(() => {
    const listener = (next: AppSettingsV1) => setSettings(next);
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  }, []);
  const update = useCallback((patch: Partial<AppSettingsV1>) => {
    setSettings(writeAppSettings(patch));
  }, []);
  return [settings, update];
}
