import { useCallback, useEffect, useState } from "react";

// 本地应用偏好（计划 §14）。
//
// 这些是「产品行为偏好」，不是模型连接（模型连接是后端 LlmProfile）。后端目前没有对应的设置命令，
// 因此先落在 localStorage；需要时把这里换成
// get_app_settings / save_app_settings 即可，读写点已经收敛到本文件。
//
// NAS 目录之前分散在 ExportPage / LibraryPage / ExamWorkspacePage 各自读同一个 localStorage key，
// 现在统一从这里读写，避免三处各自演化。
const STORAGE_KEY = "ielts-author-studio.app-settings.v1";
/** 与旧 ExportPage 共用的历史 key，迁移期继续兼容读取。 */
const LEGACY_NAS_KEY = "ielts-author-studio.confirmed-nas-export-dir.v1";

// 已移除（只被设置页自己读写、对产品没有任何作用）：`keepSourceFiles`（原文件发布后
// 由后端统一清理）、`localConcurrency` / `cloudConcurrency`（调度器并发由后端决定）。
export interface AppSettingsV1 {
  /** 发布目标目录，只选一次后记住（计划 §13.2）。 */
  nasDestination: string;
  /** 开发者模式：技术日志、完整环境诊断、过程文件保留开关才出现。 */
  developerMode: boolean;
}

export const DEFAULT_APP_SETTINGS: Readonly<AppSettingsV1> = Object.freeze({
  nasDestination: "",
  developerMode: false
});

export function readAppSettings(): AppSettingsV1 {
  let stored: Partial<AppSettingsV1> = {};
  try {
    stored = JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? "{}") as Partial<AppSettingsV1>;
  } catch {
    stored = {};
  }
  const legacyNas = window.localStorage.getItem(LEGACY_NAS_KEY)?.trim() ?? "";
  return {
    nasDestination: (stored.nasDestination ?? legacyNas).trim(),
    developerMode: stored.developerMode === true
  };
}

const listeners = new Set<(settings: AppSettingsV1) => void>();

export function writeAppSettings(patch: Partial<AppSettingsV1>): AppSettingsV1 {
  const next = { ...readAppSettings(), ...patch };
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
