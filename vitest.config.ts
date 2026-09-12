import { defineConfig } from "vitest/config";

// 前端单元测试配置（audit A11-F06 / 计划 §19.5）。
//
// 说明：
//  - 这些测试覆盖的是**纯前端逻辑**（错误分类、问题优先级、路由解析、发布意图、展示文案），
//    属于计划 §19.1 层 1「Pure unit」。
//  - 它们**不是**产品验收证据：不驱动真实 Tauri/WebView2/SQLite/文件系统，
//    产品级证据只在 `scripts/e2e/tauri-*.mjs`（真实应用进程）里产生。
//  - 默认 node 环境；只有真正需要 DOM 的用例才自带 `// @vitest-environment jsdom`，
//    当前用例集全部为纯逻辑，因此不引入 jsdom 依赖。
export default defineConfig({
  test: {
    include: ["src/**/*.test.ts", "src/**/*.test.tsx"],
    environment: "node",
    reporters: "default"
  }
});
