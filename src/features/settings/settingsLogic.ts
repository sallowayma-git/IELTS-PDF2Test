import type { PipelineMode } from "../../types";

/**
 * 保存模型连接后，云端识别该不该处于启用状态。
 *
 * 用户填好密钥、点了「保存并测试」、测试也通过了——这就是「我要用云端」。以前还得再
 * 单独勾一次「启用云端识别」（而只勾开关本身什么都不保存），于是常见的结局是：连接
 * 配好了，导入仍然只跑本地。测试没通过时不擅自启用，沿用开关。
 */
export function cloudShouldBeEnabledAfterSave(input: {
  toggle: boolean;
  testOk: boolean;
  hasKey: boolean;
  provider: string;
}): boolean {
  if (input.toggle) return true;
  return input.testOk && (input.hasKey || input.provider === "Ollama");
}

/** 是否已连接云端：存在一个启用中的真实 profile（与导入时的判定一致）。 */
export function hasCloudConnection(profiles: ReadonlyArray<{ profileId: string; enabled?: boolean }>): boolean {
  return profiles.some((profile) => profile.enabled && profile.profileId !== "profile-local-placeholder");
}



/**
 * 流水线模式归一：无法识别的值一律回退 `direct`（默认链）。
 * 与后端 `normalize_import_pipeline_mode` 的拒绝策略不同——前端只是展示层，
 * 猜错模式的代价由后端在入队时拒绝兜底；这里保持宽容，避免设置页被脏值卡死。
 */
export function normalizePipelineMode(value: unknown): PipelineMode {
  return value === "legacy" ? "legacy" : "direct";
}

/** 流水线模式的人话说明（设置页选项副文案）。 */
export function describePipelineMode(mode: PipelineMode): string {
  return mode === "direct"
    ? "在线时云端一次直出整份题稿（含原文、题组、答案），不等本地首稿、不逐组合并；离线时走本地识别兜底。"
    : "本地先出首稿，云端候选项再逐组合并校对（现有行为，较慢但可对照）。";
}
