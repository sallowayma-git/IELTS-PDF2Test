# Progress

## 2026-06-06 Settings Preflight + 100-PDF Live Regression

- Started current task at 2026-06-06 17:43:02 CST.
- User provided a test-only OpenAI-compatible API key and endpoint `https://icoe.pp.ua/`; key will be used only as a transient environment variable and not written to repo files.
- Read `frontend-skill` and `planning-with-files` instructions for this UI + long regression task.
- Confirmed Settings preflight UI is currently bulky because every preflight check is rendered as a full `.layer-list` card.
- Confirmed `/Users/maziheng/Downloads/0.3.1 working/ReadingPractice/PDF` contains 262 PDFs.
- Confirmed `ReadingPractice` has no legacy JS oracle directory, so a new no-legacy 100-PDF Live pipeline diagnostic script is needed.

## 2026-06-06 Windows Package and Compatibility

- Started Windows package and compatibility goal from existing active `/goal`.
- Used `planning-with-files` workflow because the task requires a persistent, trackable task record across many implementation steps.
- Session catchup skipped because Codex native session parsing is not implemented by the helper script.
- Read `Files/Windows包体与兼容规划.md` and split the work into W0-W10.
- Detected dirty worktree before edits; no existing changes were reverted.
- Spawned three concurrent Subagents:
  - release/audit worker for `package.json`, package audit, macOS path compatibility, offline WebView2 config.
  - runtime worker for Rust Python resolver, PDF renderer adapter, environment preflight.
  - dev/e2e worker for Windows-compatible UI flow, preview, and PDF regression scripts.
- Created `Files/Windows包体与兼容任务追踪.md`.
- Updated `Files/Windows包体与兼容规划.md` with an execution tracking index.
- Added the Windows compatibility active goal to the top of `task_plan.md`, `findings.md`, and `progress.md` while preserving prior task history.
- W6 dev/e2e Subagent completed: `ui-flow-e2e` now uses `fileURLToPath`, Windows `npm.cmd`, and Windows Chrome/Edge paths; `preview-e2e` Python resolver supports `EPIC8_PYTHON`/Windows `py -3`; `pdf-regression-sample` no longer has a local macOS corpus default and supports Windows `.exe`.
- W6 validation reported passed: `node --check` for the three scripts, `node scripts/pdf-regression-sample.mjs --help`, expected exit 2 without explicit corpus dirs, and `npm run check`.
- Sent W6 integration finding to release/audit Subagent: `package.json` must update `test:pdf-regression` because the script now requires explicit `--pdf-dir` and `--legacy-dir`.
- W1/W2 release/audit Subagent completed: split macOS/Windows release scripts, platformized package audit, added Windows artifact metadata/sha256/WebView2/lockfile reporting, fixed `repack-macos-dmg.mjs` path handling, and added `src-tauri/tauri.windows.offline.conf.json`.
- W1/W2 validation reported passed: `npm run check`, `node scripts/package-audit.mjs --help`, `npm run audit:package`, expected Windows missing-artifact failure, fake Windows artifact success branch, expected `npm run test:pdf-regression` exit 2 without corpus dirs, and `git diff --check`.
- W8 remains open because Authenticode/PowerShell signature audit was not added in the release/audit worker pass.
- W3-W5 runtime Subagent completed and parent fixed one Rust borrow-check issue in `merge_rendered_page_images`.
- Runtime validation passed: `cargo check`, `cargo test environment_preflight_reports_required_dependency_names`, and `cargo test pdf_render_adapter`.
- Added `scripts/audit-windows-signatures.ps1`, `scripts/windows-install-instructions.txt`, and `.github/workflows/windows-smoke.yml` to close W7-W9.
- Final local verification passed: `npm run check`, `cargo fmt --check`, `cargo check`, `cargo test --manifest-path src-tauri/Cargo.toml` (115 passed, 2 ignored), `git diff --check`, `node --check` for the changed JS scripts, `node scripts/package-audit.mjs --help`, and `npm run audit:package`.
- Expected local limitations recorded: `npm run audit:package:windows` fails because this macOS workspace has no NSIS/MSI artifact; `pwsh` is unavailable locally for the Windows signature script; `npm run test:pdf-regression` now intentionally requires explicit `--pdf-dir` and `--legacy-dir`.

## 2026-06-04
- Started validation from existing dirty worktree.
- Confirmed key files are modified and `scripts/pdf-regression-sample.mjs` exists.
- `npm run check` passed.
- `cargo test --manifest-path src-tauri/Cargo.toml` passed: 94 passed, 2 ignored.
- `jq` is not installed locally; switched JSON inspection to Node one-liners.
- Fixed dev fallback routing so clear-text PDF lands directly on the editable draft page.
- `npm run e2e:ui-flow` passed with real fixture PDFs; scanned/manual transcription flow now creates a new draft revision instead of being blocked by overwrite protection.
- Added Rust regression coverage for manual transcription replacing the source document, archiving the previous draft, and regenerating a new editable draft.
- Tightened classifier rules after inspecting the 30-PDF regression report: explicit A-D evidence is required for single choice, explicit `Choose TWO/THREE letters` is required for multi choice, `according to` no longer triggers classification, `Questions X and Y` now preserves both question ids, and `match each statement/opinion/person` is ordinary matching rather than paragraph information matching.
- Updated PDF regression normalization so legacy JS table-rendered paragraph matching and old completion display variants compare by semantic kind instead of raw legacy field names.
- Changed auto-pipeline routing so generated drafts open the editable draft page by default; only source-document review keeps the user on document confirmation.
- Fixed remaining random-regression structure issues: sentence-ending matching now wins over single-choice detection, overlapping ranges are normalized, flow/diagram completion uses inline completion layout, and explicit completion groups can extend to numbered blanks present in the same source span.
- Final pre-build checks passed: `npm run check`, `cargo test --manifest-path src-tauri/Cargo.toml` (98 passed, 2 ignored), `npm run e2e:ui-flow`, and `npm run test:pdf-regression` with 30/30 structure pass on seed `1780508509492`.
- Random regression answer comparison still reports missing answers when answer pages have no extractable text; this is tracked as parser/scan limitation and is routed to user confirmation rather than production overwrite.
- Built fresh DMG: `/Users/maziheng/Downloads/Desktop/copy/PDF2Test/src-tauri/target/release/bundle/dmg/IELTS Author Studio_0.1.0_aarch64.dmg`, size 5.7 MB, SHA-256 `278d89b830ff5021f25bfd6918f9aa2358a77ce6456ec49606f01a28fa615a7d`.
- Re-ran the real OpenAI-compatible API chain with the new test profile on the Margaret Preston PDF; the CLI completed successfully and wrote `tmp/live-auto-pipeline-margaret-new-key.json`.
- Real vision answer extraction succeeded with 13 answers and populated `q8`-`q13` as `symbols`, `titles`, `stencilling`, `books`, `travel`, and `400`.
- Real cloud whole-paper comparison was attempted. Direct PDF upload was rejected by the provider as unsupported, the image fallback returned JSON, and the local draft remained authoritative because the cloud comparison disagreed with the local `8-13` layout/kind.
- Verified the resulting local draft status is `Authoring`, `nextRoute=groups`, `Questions 8-13` is one `sentence_completion` group with `layoutHint=inline_completion`, and `q8`-`q13` remain in the same group.
- `npm run check` passed after updating the routing tests.
- `cargo test --manifest-path src-tauri/Cargo.toml` passed: 98 passed, 2 ignored.
- `npm run e2e:ui-flow` failed because the script still expects the old source-review route during the OCR flow; product behavior now routes generated drafts to `groups`.
- `npm run test:pdf-regression` completed on seed `1780555370790`: 27/30 structure pass, 3 structure failures, 30/30 answer failures due to parser limitations on answer pages.

## 2026-06-06
- Read the pasted export failure: publish gate was blocked by `NeedsReview`, source-review parser warnings for page 5/6 no extractable text, low-confidence groups/questions, and empty answers.
- Added import UI stage tracking. During `runAutoPipeline`, the user now sees a cloud model wait message rather than only "正在生成".
- Added pipeline report fields and persisted audit summaries for vision answer extraction: attempted/applied state, answer count, filled question ids, and missing question ids.
- Added cloud comparison report fields and persisted audit summaries: issues, observations, local outline summary, and cloud outline summary. Local authoring remains authoritative.
- Added editor UI blocks for "视觉答案补全" and "云端整卷对照", including local-vs-cloud outline summaries and missing-answer warnings.
- Added "确认当前题组" action in the group editor to mark all questions in the active group verified after the user checks them.
- Reworked export-page error handling to parse `*_validation_failed:{...}` JSON payloads and show actionable guidance for empty answers, source review, verification, and cloud comparison.
- Extended the Node LLM sidecar command whitelist with `extract_pdf_image_answers` and `generate_pdf_reading_outline` for parity with the Rust gateway.
- Updated Rust cloud/vision regression coverage: cloud comparison summary persists after minimization, cloud mismatch does not overwrite local answers, and vision answer extraction exposes filled/missing question ids.
- Updated PDF image extraction tests to match full-page vision rendering behavior rather than the older single `sips` preview assumption.
- Verification passed: `npm run check`; `node --check sidecars/llm-gateway/gateway.mjs`; `cargo check --manifest-path src-tauri/Cargo.toml`; `cargo test --manifest-path src-tauri/Cargo.toml` (114 passed, 2 ignored).
- Started Vite at `http://127.0.0.1:1420/` after approval and checked the import wizard with the in-app browser.
# 2026-09-01 AI 能力完整性审计

- 已读取仓库级 AGENTS.md、现有 task_plan.md/findings.md/progress.md 与 package.json。
- 已确认仓库不是干净工作树；现有 `.workbuddy/` 未追踪改动保留不动。
- 当前阶段：等待并行只读勘察结果，尚未修改产品代码。
- 6 个并行只读探针均在约 10 分钟内未返回可用消息；按仓库规则停止等待，改由主线程分层核验，避免重复派发相同探索。

## 2026-09-02 AI 能力完整性审计

- 重新派发 6 个窄范围只读探针，分别覆盖前端入口、Rust 网关、自动流水线/落盘、视觉答案补全、测试契约、导出与学生运行时；全部运行超过约 10 分钟仍无可用消息，已按约束关闭，未将其作为证据。
- 主线程第一轮地图确认：AI 能力涉及 `src/pages/ImportWizard.tsx`、`src/pages/UnifiedPreview.tsx`、`src/pages/StructuredAuthoringEditorV2.tsx`、`src-tauri/src/auto_pipeline.rs`、`src-tauri/src/llm_gateway.rs`、`src-tauri/src/llm_commands.rs`、`src-tauri/src/cleanup.rs` 以及 V2 export/runtime 模块；需要重点审计 V1/V2 双路径和前端背景云端调度。
- 尚未修改产品代码；下一步读取上述关键实现的完整函数和测试，再形成 A1 缺口矩阵。

## 2026-09-03 AI 能力完整性审计续行

- 用户确认审计粒度：不要求每次 AI 网络调用产生持久审计；按 AI 阶段/run 归并重试，只有候选、用户接受/拒绝、权威写入、导出阻断和失败需要 durable evidence。
- 用户确认交互：AI 删除以红色删除线、AI 新增/补全以绿色显示，逐条或整组接受/拒绝；接受才创建 revision，拒绝不改权威稿。
- 工作区已有一批与本审计目标相关的未提交产品改动（自动流水线并发尝试、云端 opt-in、视觉答案候选文件/命令/UI、quote 检查、配置错误态）；这些不是本线程最初写入的改动，已保留并作为当前基线审计。
- 已获得两份有效探针结论：V2 `task.reviewState` 未纳入导出阻断；现有 AI 测试缺 provider/HTTP 负例、重启恢复、真实 Tauri command/UI 和并发时序覆盖。第三个探针因上游 503 无结果。
- 当前最高风险：检查现有中间态改动是否可编译、并发是否真实并行、视觉候选是否可拒绝/持久化、V2/NAS 导出门禁是否 fail-closed，再继续修改。

## 2026-09-03 对抗审计续行

- 按用户要求重启了 6 个窄范围只读探针，并将统一等待延长到 15 分钟；全部在阈值内未返回正文，已停止，结果不作为证据。
- 主线程重新接管关键文件阅读。当前实现已具备网关调用观测、429/408/部分 5xx 重试、视觉答案候选文件和 Tauri command，但工具调用审批/执行闭环仍未实现。
- 下一步先复核 `validate_*` 的 fail-closed 语义、视觉候选操作幂等、流水线并发是否被 `Mutex<FnMut>` 串行化，以及 V2/NAS 发布门禁，再补定向回归测试。

## 2026-09-02 AI 完整性审计-优化-对抗审计 全流程
- 解析 codex 会话 roll-out：上轮仅完成一半审计（findings 有初步缺口矩阵），修复从未开始、无代码改动。
- 6 路并发审计子代理（网关/并发编排/视觉落盘/前端/测试/产品要求）产出完整缺口矩阵；产品文档裁定并发目标的合规落法：LLM 转化与本地解析并发执行但结果只读、本地始终权威、云端需 opt-in。
- 第一轮优化：并发改造（thread::scope + Mutex 网关 + mpsc 汇合，抽图 3 次→1 次）、视觉答案候选闭环（落盘/apply 命令/UI 采用忽略/审计/清理策略）、云端对照 opt-in 可达（ImportWizard 开关 + profileId 传递 + 去重）、网关收紧（confidence fail-closed、quote 验真、base_url/provider 校验、429/408+Retry-After、llm_call 观测记录、enabled 检查）、Settings/预览静默失败补错误态。
- 验证：cargo test --lib 533 通过（新增 7 测试），10 个失败经 stash 对照确认为 Windows 本机预存环境问题；npm check 绿；e2e 基线同样失败（本机缺 Python pypdf 等）。
- 对抗审计 2 轮：红队确认无新阻断后，修复其发现的 worker panic 挂死（Sender 移交 + catch_unwind）、reject-only 忽略失败、base_url 前缀绕过（IP 字面量严格解析）、候选防覆盖（alreadyAnswered 防线）、TOCTOU（generatedAt 校验）、quote 验真盲区；补 reject-only/防覆盖测试。
- 第三轮：V2 导出门禁错误分类解析（ExportPage）+ 题组「AI 题组建议」面板（获取建议/应用/忽略/持久化 llmReview 横幅）+ 样式。
- 最终状态：533 测试通过、tsc 绿、AGENTS.md 要求的 CLI/产品证据分层已记录；遗留 A7（导入取消/进度事件、队列持久化、preflight LLM 检查、V2 revision 链分歧）已登记 task_plan。

## 2026-09-03 AI 审计接手验证与 A7 优先级确认

- 读取工作区现有改动：19 个文件已修改（+3473/-779 行），涵盖并发流水线、视觉候选闭环、网关收紧、UI 补全。
- 验证现有实现质量：cargo test --lib **535 通过**（超过基线 533），10 个失败为已知环境问题（缺私有 PDF 语料、pypdf 模块、reqwest 请求头断言）；npm run check 绿。
- 确认云端复核队列确实使用 sessionStorage（UnifiedPreview.tsx:80-99），重启后丢失。
- A7 遗留项评估：导入取消/进度事件、队列持久化为用户体验增强；preflight LLM 检查为设置体验增强；V2 revision 链分歧为系统性架构遗留，非本轮阻断问题。
- 决策：A1-A6 已完成并通过验证，A7 为后续增量改进，当前审计目标已达成。
