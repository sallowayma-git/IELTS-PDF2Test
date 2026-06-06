# Windows 包体与兼容任务追踪

## 目标

依据 [Windows包体与兼容规划.md](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/Files/Windows包体与兼容规划.md) 推进 Windows 版 EXE/NSIS/MSI 分发、运行时兼容、扫描 PDF 降级、开发脚本兼容、签名审计与 CI smoke。保持默认小包体边界：不打包 Node、Python、Tesseract、本地 OCR 或离线 WebView2 Runtime；扫描 PDF 优先走页面图片/云端 vision/人工确认路径。

## 当前状态

| 字段 | 内容 |
|------|------|
| 创建时间 | 2026-06-06 17:24:53 CST |
| 当前阶段 | 实现与本机验证完成 |
| 总体状态 | complete |
| 并发执行 | Subagent: release/audit、runtime compatibility、dev/e2e compatibility 均已完成 |
| 原规划任务书 | [Files/Windows包体与兼容规划.md](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/Files/Windows包体与兼容规划.md) |

## 任务追踪表

| ID | 任务 | 来源章节 | 状态 | 负责人 | 验收标准 |
|----|------|----------|------|--------|----------|
| W0 | 建立可追踪任务书记录，并同步原规划任务书索引 | 推荐下一步 + 用户要求 | complete | parent | 存在本文件；原规划任务书包含追踪索引和状态摘要；根目录 planning 文件指向当前目标 |
| W1 | Release scripts 拆分与 Windows package audit | Phase 1 / 4 / 5 | complete | subagent-release | `package.json` 有 macOS/Windows release 和 audit 入口；`package-audit.mjs` 支持 macOS/Windows；Windows audit 输出 artifact size、WebView2 模式、SHA-256、依赖边界 |
| W2 | Windows offline WebView2 配置 | Phase 1 / WebView2 | complete | subagent-release | 新增 `src-tauri/tauri.windows.offline.conf.json`；默认主配置不引入 offline 大包 |
| W3 | Rust Python command resolver | 平台兼容点 2 | complete | subagent-runtime | `EPIC8_PYTHON` 优先；Windows 支持 `py -3`/`python`；macOS/Linux 支持 `python3`/`python`；Python 仍为 optional |
| W4 | PDF renderer adapter 平台化和扫描 PDF 降级 | 平台兼容点 1 / Phase 3 / 4 | complete | subagent-runtime | macOS 继续 `sips`；Windows 未实现 renderer 返回结构化 manual-review/unsupported 状态；不因缺 Python/sips 硬失败 |
| W5 | 环境预检平台化 | 平台兼容点 3 / Phase 2 | complete | subagent-runtime | Windows 不显示 macOS sips unavailable；显示平台 renderer、local OCR disabled、cloud vision/profile capability 状态 |
| W6 | 开发/E2E 脚本 Windows 兼容 | 平台兼容点 7 | complete | subagent-e2e | `ui-flow-e2e` 支持 Windows npm/Chrome/Edge；`preview-e2e` 支持 Windows Python resolver；`pdf-regression-sample` 不绑定本机 macOS 默认数据路径并支持 `.exe` |
| W7 | macOS-only 脚本隔离 | 平台兼容点 6 | complete | parent | Windows release 不引用 DMG repack 或 macOS unsigned helper；如有 Windows 说明单独成文 |
| W8 | 签名与分发预留 | Phase 5 / SmartScreen | complete | parent/subagent-release | 记录内测未签名策略；预留 Authenticode audit/signing env；不把签名与业务构建揉死 |
| W9 | Windows CI smoke 规划落地 | Phase 6 | complete | parent | 规划任务书和追踪表包含 CI smoke 清单；代码支持在 `windows-latest` 执行基础 check/build/audit |
| W10 | 验证、集成与最终同步 | 全部 | complete | parent | 运行可行的本机验证；更新本追踪、原规划、`findings.md`、`progress.md`；全部任务状态 complete 或明确记录平台限制 |

## 验证记录

| 时间 | 命令/检查 | 结果 | 备注 |
|------|-----------|------|------|
| 2026-06-06 17:24 CST | session catchup | passed | Codex native session parsing未实现，跳过无历史恢复 |
| 2026-06-06 17:24 CST | 初始仓库扫描 | passed | 发现已有未提交改动；本轮需避免 revert 用户改动 |
| 2026-06-06 17:30 CST | W6 subagent validation | passed | `node --check` 三个脚本通过；`npm run check` 通过；`pdf-regression-sample` 无显式 corpus 时按预期 exit 2 |
| 2026-06-06 17:39 CST | W1/W2 subagent validation | passed | `npm run check`、`package-audit --help`、macOS package audit、Windows 缺产物失败路径、临时 fake Windows artifact 成功分支、`git diff --check` |
| 2026-06-06 17:47 CST | W3-W5 Rust validation | passed | `cargo check`、`cargo test environment_preflight_reports_required_dependency_names`、`cargo test pdf_render_adapter` 通过 |
| 2026-06-06 17:52 CST | Full Rust regression | passed | `cargo test --manifest-path src-tauri/Cargo.toml`：115 passed, 2 ignored |
| 2026-06-06 17:52 CST | Formatting and script syntax | passed | `cargo fmt --check`、`git diff --check`、主要 `.mjs` `node --check` 通过 |
| 2026-06-06 17:52 CST | macOS package audit | passed | `npm run audit:package` 通过，输出 `.app`/`.dmg` size 与 SHA-256 |
| 2026-06-06 17:52 CST | Windows package audit without artifact | expected_fail | 本机无 NSIS/MSI，`npm run audit:package:windows` 清晰提示缺 Windows installer |
| 2026-06-06 17:52 CST | Windows signature audit locally | not_run | 本机缺 `pwsh`；脚本已加入 Windows workflow，由 Windows runner 执行 |
| 2026-06-06 17:52 CST | PDF regression without corpus | expected_fail | `npm run test:pdf-regression` 清晰提示必须显式传 `--pdf-dir` / `--legacy-dir` |

## 风险与决策

| ID | 类型 | 状态 | 内容 | 处理 |
|----|------|------|------|------|
| R1 | dirty worktree | open | 仓库已有多处未提交改动，包含 Rust、前端、脚本和原 Windows 规划文件 | 所有修改都按文件范围隔离；不 revert 既有改动 |
| R2 | Windows artifact unavailable locally | external_verification_pending | 当前机器是 macOS，无法真实生成/签名 Windows NSIS/MSI | 已新增 Windows workflow 与审计脚本；真实产物待 `windows-latest` runner 验证 |
| R3 | Tauri WebView2 config schema | external_verification_pending | offline WebView2 配置需要与 Tauri 2 Windows bundler 产物共同验证 | 使用独立 config 文件；默认小包体配置不受影响 |
| R4 | real Windows smoke | external_verification_pending | W6 已完成静态/类型验证，但未在真实 Windows 上运行 UI E2E | 已新增 Windows CI smoke；UI 交互 smoke 可在后续 runner 扩展 |
| R5 | Windows signing audit | mitigated | Authenticode 审计需要 Windows PowerShell 环境 | 已新增 `scripts/audit-windows-signatures.ps1` 与 npm/script/workflow 入口 |

## 更新日志

| 时间 | 更新 |
|------|------|
| 2026-06-06 17:24 CST | 建立 Windows 包体与兼容任务追踪，按规划任务书拆出 W0-W10，并发派发 release/audit、runtime、dev/e2e 三条 worker 任务 |
| 2026-06-06 17:30 CST | W6 开发/E2E 脚本 Windows 兼容完成；发现 `package.json` 的 `test:pdf-regression` 需随 package 脚本拆分同步调整 |
| 2026-06-06 17:39 CST | W1/W2 release/audit 和 offline WebView2 配置完成；Windows 真实 artifact/签名验证保留到 Windows runner |
| 2026-06-06 17:52 CST | W3-W5 runtime compatibility 完成并通过 Rust 检查；补齐 Windows 签名审计脚本、安装说明和 Windows smoke workflow；所有实现任务收口完成 |
