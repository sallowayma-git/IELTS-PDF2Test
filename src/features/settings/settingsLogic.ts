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
