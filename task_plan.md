# Current Active Goal: 2026-09-15 三个缺口闭环（真实 Tauri 通道 / 学生预览 / 单一题稿与合并问题）

## Goal

闭环三件事：(1) 恢复当前源码的真实 Tauri PDF/DOCX 产品端到端验证通道；(2) 在现有编辑工作区接通学生预览并与真实学生端语义一致；(3) 前端保持**一份题稿**、只显示**合并后的待处理问题**，并把「后台运行状态 / 统一建议 / 决策」接口接到执行代理交付的真实后端。

明确**不在本轮范围**：云端识别调度、候选生成、裁定与 V2 应用后端由另一个执行代理负责，本代理不再扩展该部分；已改动部分只做清单交接，不回滚有效工作。

- [complete] R0 建立本轮可追溯基线（样本、期望答案、命令、产物目录）。
- [complete] R1 恢复真实 Tauri 验证通道（最小冒烟两次连续通过 + 真实编辑/保存/重开）。
- [complete] R2 工作区学生预览（官方编译闸门 + 共享语义渲染 + 学生作答状态隔离 + 真实链路断言）。
- [in_progress] R3 前端单一题稿 + 合并问题列表 + 识别建议/决策 UI（前端已实现并有单测；后端命令已注册，面板已对真实后端跑通只读路径；**写入路径的端到端验收未做**）。
- [incomplete] R4 真实 PDF/DOCX → 发布 → 真实 Electron 学生端加载/作答/提交/计分。
  - PDF 作者侧已通过；DOCX 作者侧已修导入入口，待重跑。
  - 发布被质量门禁拦下（4 项阻断），前端无清除入口 → 学生端链路**未完成**。

## R1/R2 验收证据（本轮）

| 项 | 命令 | 产物 |
| --- | --- | --- |
| 最小冒烟（两次连续） | `node scripts/e2e/tauri-cdp-smoke.mjs --keep --extra-args "--no-sandbox --disable-gpu"` | `artifacts/e2e-cdp/run-smoke-2026-09-15T20-58-35-007Z/report.json`、`...20-58-47-753Z/report.json`（均 passed） |
| 真实 PDF 全链 | `node scripts/e2e/tauri-cdp-product-chain.mjs --keep --tolerate-concurrent-edits` | `artifacts/e2e-cdp/run-chain-2026-09-15T21-08-49-769Z/report.json`（9 步 passed，发布 blocked_by_quality_gate） |
| 识别建议面板（只读路径） | `node tmp/read-recognition.mjs --run-dir <上面> --item import-20260915210859-b9dc26cf` | 面板挂载、状态文案正确、无 `undefined` |
| 前端回归 | `npx tsc --noEmit` / `npx vitest run` | tsc 干净；vitest **113 passed / 11 files** |

## R3 与后端对齐的待办（发给识别/云端 agent）

1. `get_recognition_decision` 实际返回 `chains.*.state` + `actionable`/`autoApplied`，契约 §2.3 写的是 `cloudStatus`/`localStatus`/`items`。前端已双向兼容，但**契约文档或实现需要收敛到一种**。
2. 契约 §4 的四个未决点中，本 agent 的选择：`reconciling` 用「正在核对识别结果」文案；`auto_fixed` 的撤销入口放在识别建议面板（默认折叠）；`unverifiable` 不做题库行徽标（先不加噪音）；`get_workspace_item` 不内联 `recognition`（面板自己按需拉取，避免工作区加载被识别链拖慢）。

## Current Decisions

- 两仓 HEAD（本轮起点）：PDF2Test `c8c3d3b`；学生端 `a9ea3c1`（仅 `.workbuddy/` 未跟踪，禁触）。
- **历史结论不算本轮验收**：Rust 627 / 前端 78 / 四条跨仓套件的旧结论只作 HISTORY 记录。
- **通道结论（R1）**：`tauri-driver` + `msedgedriver` 在本环境无法为被测 exe 建立稳定会话
  （`session not created / unable to connect to renderer`、`chrome not reachable`）。改为
  **WebView2 自带 CDP**：`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=<port> --remote-allow-origins=*"`，
  由 `scripts/e2e/lib/tauri-cdp-harness.mjs` 封装。驱动的是真实 exe、真实 Rust 后端、真实 SQLite、真实文件系统。
- **构建事实（R1，关键）**：`cargo build` 产出的是 **dev 模式二进制**（加载 `devUrl http://localhost:1420`，
  实测页面为 `ERR_CONNECTION_REFUSED`）。内嵌前端的可验收二进制必须来自
  `npx tauri build --debug --no-bundle`（本轮用 `--config '{"build":{"beforeBuildCommand":""}}'` 跳过
  `vite build` 的 `emptyOutDir` 批量删除守卫，dist 已先行单独构建）。
- **参数标注（R1）**：CDP 通道需要 `--no-sandbox --disable-gpu` 才能让 renderer 不中途崩溃。
  这两项放宽了渲染进程沙箱/GPU 路径，属**诊断参数**；所有以此运行得到的结论必须标注
  `diagnosticRun=true`，**不得**写成默认产品路径通过。
- **单一题稿**：前端只保留一份草稿（`useCanonicalEditor.draft`），问题列表 = 本地闭包检查
  ∪ 发布门禁 blocker/warning（`mergePublishGateIssues`），不引入「本地稿/云端稿/校验稿」三份版本。

## R0 本轮验收样本（真实导入样本，非 UI 隔离夹具、非故障注入夹具）

| 样本 | SHA256 | 覆盖题型 | 人工期望答案（来自 `fixtures/parser/complex-reading.md`） |
| --- | --- | --- | --- |
| `fixtures/parser/complex-reading.pdf` | `036307eb978bb0e3d4a0a5129b59eea43cbd71e3e66743db4ca1a18e4907eb66` | T/F/NG（radio）+ note completion（text） | 1 TRUE, 2 FALSE, 3 TRUE, 4 diaries, 5 diaries |
| `fixtures/parser/complex-reading.docx` | `717918f5bd0e640a40496d13b96242f4cead97ab1a2d24be40c5c0d780e32314` | 同上 | 同上 |
| `fixtures/parser/complex-reading.md` | `7d0d6ff71ad089cf34c46baed1b548dabad4d674a285c274eff4d432edf36582` | 人工答案表来源（非导入输入） | — |
| `fixtures/parser/demanding-reading-passage-1.docx` | `ab0ad67ee31789cf5e464ce43bea175a0475fbbab1d1a1ccbffb8c794a0e2e78` | 多题组长文（DOCX 富样本） | 待人工核对后填写 |
| `fixtures/parser/demanding-reading-passage-3.pdf` | `f13bd65cb5f5c76a178ff87fa212df30f79852bdb103249af9eb5804a4ccfe18` | 多题组长文（PDF 富样本） | 待人工核对后填写 |

- 失败注入夹具（如 `fixtures/golden/synthetic/docx/bad-*`）**只用于故障路径**，不得当作导入验收样本。
- 历史产物目录 `artifacts/e2e-tauri/`（含 `run-*` 与 `FINAL-HEAD-ACCEPTANCE-91828ea.json`）**无一条钉在 `c8c3d3b`**，
  全部记为 HISTORY；本轮产物目录为 `artifacts/e2e-cdp/`。
- 默认路径 vs flag 路径：`QLG_DIRECT_CANONICAL` 默认关闭，本轮默认路径结论一律在 flag 关闭下取得；
  若需 flag-on 补充结论，单独列出，不得混入默认路径。

---

# Previous Active Goal: 跨仓库产品闭环（PDF2Test ↔ IELTS-NASfor-WenDao）

## Goal

把 PDF2Test 打磨成普通用户可用的 IELTS 转换工具：PDF/DOCX 上传 → 识别 → IeltsAuthoringIRV2 → 同一界面所见即所得编辑 → 本地规则+云端 LLM 校验 → 用户确认 → ReadingExamSourceV2 → NAS/GS 题目包 → 学生端（Electron+Vue）正确发现/加载/渲染/作答/提交/计分。核心验收是真实产品链条可被学生端消费，不是 schema 校验通过。

阶段（详见用户任务书）：
- [in_progress] P0 保护现场、建立可复现基线、field-by-field 兼容矩阵。
- [pending] P1 冻结并统一共享契约（ReadingExamSourceV2 / ContentDocV2 / IeltsAuthoringIRV2 三仓内两仓逐字节一致）。
- [pending] P2 关闭学生端运行时高风险（Electron 图片 URL、hotspot 提交契约、resource layout、answer leakage）。
- [pending] P3 统一 PDF2Test WYSIWYG 与学生端语义（author/preview 双模式、事务式结构编辑）。
- [pending] P4 云端 LLM 校验体验（candidate diff、stale revision、逐项接受/拒绝）。
- [pending] P5 识别与发布门完善（direct canonical 保持默认关闭直到质量门）。
- [pending] P6 真实跨仓产品 E2E（PDF+DOCX → Tauri 全链 → Electron 作答/提交/计分）。
- [pending] P7 回归与兼容性（flag-off、legacy/V1、导出失败回滚、CAS、路径穿越等）。
- [pending] V2 六 verifier 独立验证 → 修复 P0/P1 → 第三轮确认。

## Current Decisions

- 第一轮 6 个侦察子代理（A-F）并发只读派发，返回前不改产品代码。
- 两仓 HEAD：PDF2Test f119b2a；学生端 ceee9c8。学生端仅 .workbuddy/ 未跟踪（禁触）。
- PDF2Test 未提交改动定性：flag-on Tauri E2E 支撑（automation_source_files 钩子、harness appEnv、importSourceViaFileHook、data-editor-id/issue 定位修复、tauri-direct-canonical.mjs 新脚本）＝本任务有效在途工作，保留并继续。
- IeltsAuthoringIRV2 漂移：PDF2Test 多出 recognitionBlockers/recognitionBlockerTargets/recognitionBlockerTarget；先查契约所有权/同步脚本/消费者，再选最小修复。
- 学生端 resolver 硬编码 resources/<examId>：保持 PDF2Test 固定目录，不做资源路径抽象。
- answerKey 绝不发送学生前端；hotspot 提交值必须能通过服务端验证与计分。
- direct canonical 默认关闭；synthetic 指标（empty prompt 55/57、visual 5/5 失败、statement 0 覆盖）如实记录，private-real corpus 缺失不宣布达标。

# Previous Active Goal: 2026-09-13 产品可用性诊断 + WYSIWYG/云端校验下一步定义

## Goal

围绕 `Plan With Files/IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md` 的产品目标，先做一次面向真实用户路径的证据化诊断：核对当前题库导入、PDF/DOCX 识别、Canonical DS、ExamCanvas 所见即所得编辑、云端 LLM 校验/候选呈现、发布与学生运行时一致性；结合外部同类产品/交互模式研究，形成“问题地图 + 优先级假设 + 苏格拉底式产品澄清问题”。本轮不先修改产品代码。

- [in_progress] R1 完整阅读产品简化/双路识别/WYSIWYG 规划并提取不可妥协验收标准。
- [pending] R2 并行核查当前真实产品路径：Library/Import、Workspace/ExamCanvas、processing/cloud/reconcile、publish/student runtime。
- [pending] R3 检索同类文档转题、assessment authoring、WYSIWYG/AI review 产品与设计模式。
- [pending] R4 汇总当前实现与目标的差距，区分“产品行为证据 / 后端证据 / 仅测试或文档证据”。
- [pending] R5 给出苏格拉底式问题，确认下一步改进范围、默认自动化程度、校验策略和验收标准。

## Current Decisions

- 当前阶段只诊断与定义，不直接改 UI 或识别逻辑。
- 奠基性规划文档由主线程完整阅读；跨目录/大范围检索用只读子代理压缩结果，并按 `file:line` 抽查关键出处。
- 判断优先看真实 Tauri 用户路径和最终学生 runtime，而不是 CLI/schema 单测。

# Previous Active Goal: IELTS 文档识别重构 Phase 5 全量交付

## Goal

依据 `Files/IELTS_Document_Recognition_Overhaul_Plan_CN.md`（该旧世代文档已于 2026-09-12 清理，可用 `git show 87b7747:Files/IELTS_Document_Recognition_Overhaul_Plan_CN.md` 取回）第 16.6、20.4、24.10 和 24.11 节，完成结构化编辑器 Phase 5 全部范围：普通用户可直接编辑 V2 内容节点、题组、共享答案位、选项库、表格、资源、热点和答案；问题可定位到源锚点；同一份草稿可切换学生预览；编辑具备防抖 patch、immutable revision、版本冲突、本地崩溃恢复、undo/redo 和 V2 bundle export；真实 PDF 从导入到导出通过端到端验收。

- [complete] S1 建立 Phase 5 V2 session、patch 协议与 immutable revision 保存；保留 V1 文件可读。
- [complete] S2 完成结构化编辑器页面、题组/共享答案位/选项库/答案编辑与 issue rail/source overlay。
- [complete] S3 接通 student parity preview、650ms autosave、乐观版本冲突提示和 localStorage recovery。
- [complete] S4 增加 `/phase5` fixture 路由、现有 job 的 `authoring-v2` 路由与生产默认关闭的 feature flag。
- [complete] S5 完成前端构建、Phase 1 schema 回归、Rust check/test 与 Phase 5 专用验证。
- [complete] S6 扩展完整节点增删移动、table/asset/hotspot 编辑、undo/redo、原生 Tiptap schema 适配与真实 PDF 端到端验收。
- [complete] S7 修复最终审计发现：答案位 undo 安全、issue target/bbox 定位、同源 runtime interaction model、导出失败回滚、真实流水线 V2 shadow 受控写入与显式 editor opt-in；独立只读复审已通过。

## Current Decisions

- Phase 5 编辑层采用 Tiptap 3 + custom node/schema，将编辑事务映射为 canonical V2 content nodes 和 append-only patch/revision。
- V2 导出与既有 V1 NAS/export 并行；本阶段不提前承诺 Phase 6 的 NAS V2 runtime/student 双读迁移。
- V2 编辑只写 `authoring-ir-v2.shadow.json` 对应的 revision 命名空间，不重写 V1 `authoring-ir.json`；逐题 PDF LLM repair 继续关闭。

## Tracking Files

> 下列旧世代追踪文档已于 2026-09-12 清理（`Files/` 下已不存在）。Phase 5 已 `[complete]`，当前追踪以 `Plan With Files/Dual_Recognition/` 为准。
> 取回方式：`git show 87b7747:Files/IELTS_Document_Recognition_Overhaul_Plan_CN.md` / `git show 36cd3f1:Files/IELTS_Document_Recognition_Phase_5_Progress_CN.md`

- `Files/IELTS_Document_Recognition_Overhaul_Plan_CN.md`（已清理）
- `Files/IELTS_Document_Recognition_Phase_5_Progress_CN.md`（已清理）

# Previous Active Goal: Settings Preflight Slimdown + 100-PDF Live LLM Regression

## Goal

精简设置页“运行环境预检/依赖排查”界面，避免卡片堆叠；使用用户提供的测试专用 OpenAI-compatible endpoint 临时运行 Live 云端 LLM 回归，并覆盖 `/Users/maziheng/Downloads/0.3.1 working/ReadingPractice/PDF` 中 100 篇 PDF，记录自动生成、视觉答案补全、云端对照和导出阻塞类别。

- [in_progress] S1 重构设置页依赖预检 UI 为摘要 + 筛选 + 紧凑列表。
- [pending] S2 补 100 篇 PDF Live pipeline 调度/报告脚本。
- [pending] S3 使用临时环境变量验证测试 Key 与模型连接，不写入仓库。
- [pending] S4 跑满 100 篇真实 PDF，并归类失败/卡顿/空答案/云端差异。
- [pending] S5 根据回归发现修复高频问题并复测关键路径。

## Current Decisions

- 测试 Key 只通过当前命令环境变量使用，不保存到 repo、计划文件或应用配置。
- 100 篇 PDF 目录没有 legacy JS 对照文件，因此本轮 Live 回归以真实 pipeline smoke/diagnostic 为主，不做旧题库结构对照。
- 依赖预检 UI 应优先显示用户需要处理的 warning/error，OK/info 收进紧凑视图。

# Previous Active Goal: Windows Package and Compatibility

## Goal

依据 `Files/Windows包体与兼容规划.md` 推进 Windows 版 EXE/NSIS/MSI 分发、运行时兼容、扫描 PDF 降级、开发脚本兼容、签名审计与 CI smoke，并持续维护 `Files/Windows包体与兼容任务追踪.md` 与原规划任务书，直到任务全部完成。

- [complete] W0 建立可追踪任务书记录，并同步原规划任务书索引。
- [complete] W1/W2 Release scripts、Windows package audit、offline WebView2 配置。
- [complete] W3/W4/W5 Rust Python resolver、PDF renderer adapter、环境预检平台化。
- [complete] W6 开发/E2E 脚本 Windows 兼容。
- [complete] W7/W8 macOS-only 脚本隔离、Windows 签名与分发预留。
- [complete] W9 Windows CI smoke 规划落地。
- [complete] W10 集成、验证、最终同步并将全部任务闭环。

本机验证完成；真实 NSIS/MSI 产物、签名状态和 offline WebView2 installer 仍需在 Windows runner 上执行新增 workflow 验证。

## Current Decisions

- 默认小包体边界不变：不打包 Node、Python、Tesseract、本地 OCR 或离线 WebView2 Runtime。
- Windows offline WebView2 作为独立发行配置，不污染默认小包体渠道。
- Python/PyMuPDF/Poppler 是 optional 诊断或用户自带环境能力，不作为生产主路径 blocker。
- 扫描 PDF 在 Windows renderer 未实现时必须进入结构化人工确认/云端 vision 提示，不因缺少 `python3` 或 macOS `sips` 硬失败。
- 使用 Subagent 并发开发，但按不重叠写入范围拆分并由父代理统一集成。

## Tracking Files

- [Files/Windows包体与兼容规划.md](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/Files/Windows包体与兼容规划.md)
- [Files/Windows包体与兼容任务追踪.md](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/Files/Windows包体与兼容任务追踪.md)
- [findings.md](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/findings.md)
- [progress.md](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/progress.md)

# Previous Goal: PDF2Test Import Automation + Silent LLM Repair Plan

## Goal
Make PDF/DOCX import usable as a one-click/batch flow that lands on editable drafts, silently extracts scanned answer images with a vision model, runs a cloud full-paper comparison in the background, warns only on user-actionable uncertainty, validates against real fixtures, and produces a fresh DMG.

- [complete] Validate current changed code and test baseline.
- [complete] Fix remaining continuous completion, routing, parsing, or regression issues found by tests.
- [complete] Run real PDF regression sampling and record mismatches.
- [complete] Build a fresh DMG and report exact artifact path.
- [complete] Diagnose why auto-generation still lands on the import/recognition page.
- [complete] Add silent vision answer extraction JSON path for PDF image answer pages.
- [complete] Add background cloud whole-paper generation/comparison, with local output authoritative.
- [complete] Add or update tests for routing, LLM answer extraction, cloud comparison warning, and random PDF sampling.
- [complete] Run the full business chain with the provided OpenAI-compatible test profile.
- [pending] Build a fresh DMG after the new fixes.

## 2026-06-06 Update
- Added user-visible generation stage copy for long cloud-model waits during batch import.
- Persisted vision answer extraction and cloud-comparison summaries into `authoring-ir.audit.issues` so they survive process artifact minimization.
- Added editor UI for vision answer补全、云端整卷对照、本地/云端结构摘要, plus a current-group confirmation action.
- Improved export-page validation errors by parsing publish gate JSON and showing actionable categories instead of raw `js_export_validation_failed` payloads.
- Full Rust test suite now passes: 114 passed, 2 ignored.

## Decisions
- Production generation must depend on the uploaded source file, not legacy reading-exams JS.
- Legacy reading-exams JS is only a regression oracle with normalized fields.
- Existing editable drafts must not be overwritten unless explicitly requested.
- Normal users should see user-level text only; OCR/LLM/IR/runtime/rule split wording belongs only in advanced diagnostics.
- Local generated draft is authoritative; cloud model output is a background quality check and never overwrites the draft by default.
# Current Active Goal: AI 生成、核验、补全能力完整性审计与产品闭环

## Goal

审计并补齐真实 Tauri 文档导入、编辑、预览、导出链路中的 AI 能力：生成（从 PDF/DOCX 形成可编辑草稿）、核验（结构/答案/质量门禁/云端交叉检查）、补全（扫描答案、缺失字段和编辑器内内容），并确认模型配置、工具调用、权限/超时/重试、结构化输出校验、持久化证据、UI 状态和测试覆盖是否完整。最终交付可追踪的审计结论、优先级缺口，以及必要的产品代码和回归验证。

- [complete] A1 盘点 AI 入口、调用协议、产物、UI 和测试覆盖（6 路并发审计完成）
- [complete] A2 形成按风险排序的缺口矩阵和目标闭环设计
- [complete] A3 实现高优先级缺口：并发编排（LLM转化∥本地解析）、视觉答案候选闭环、云端对照 opt-in 可达、confidence/quote/base_url/provider/重试/可观测收紧
- [complete] A4 以底层测试验证（cargo 535 通过，新增 7 测试；tsc 绿）
- [complete] A5 对抗审计轮（2 轮红队）：修复 worker panic 挂死、reject-only 忽略失败、base_url 绕过、候选防覆盖、TOCTOU、quote 验真盲区
- [complete] A6 V2 导出门禁错误分类解析 + 题组「AI 题组建议」UI 入口
- [complete] A7 验证接手：确认现有改动质量（535 tests passed, tsc green），A7 后续项为增量体验改进非阻断

## Status

**AI 审计目标已达成**。核心 P0/P1 缺口（并发、视觉闭环、网关收紧、导出门禁、UI 入口）已修复并通过测试验证。A7 后续项（导入取消、队列持久化、preflight LLM 检查、V2 revision 分歧）为用户体验增强和架构统一，不影响当前功能正确性。

## Current Decisions

- 以 Tauri UI、Rust command handler、持久化 job/revision、真实 student runtime 和导出门禁为主证据；CLI 仅作辅助诊断。
- 本地生成结果仍是默认权威；AI 核验结果必须可追溯并能阻止高风险导出；AI 补全必须保留来源、置信度和人工确认状态。
- 不把密钥写入仓库或计划文件；工具调用必须有显式 schema、超时、重试边界、错误分类和 JSON 校验。

## 2026-09-03 用户确认的 AI 交互边界

- 可追踪审计按一次用户可理解的 AI 阶段/run 归并，不要求每次重试、轮询或纯预览调用都产生持久记录；仅对候选生成、接受/拒绝、权威稿写入、导出阻断和阶段失败保留 durable event。
- AI 输出缺少业务必需字段、类型错误、未知 patch 路径或证据不匹配时必须 fail-closed；仅允许明确安全的编号/格式归一化，不得用空数组、空对象或默认置信度掩盖协议错误。
- AI 文字/答案/结构变更先作为候选 diff 展示：新增绿色、删除红色；用户接受后才写入权威稿和 revision，拒绝不修改权威内容但保留决策摘要。
- 视觉答案候选与题组补全遵循同一接受/拒绝协议；本地确定性稿仍是初始权威，云端结果默认只读核验。

---

## 2026-09-15 续行 R5 状态（范围调整后）

### 本 agent 负责范围与完成度

| 项 | 状态 | 证据 |
| --- | --- | --- |
| 1. 恢复当前构建的真实 Tauri 产品验证通道 | **完成** | WebView2 CDP 通道；`scripts/e2e/lib/tauri-cdp-harness.mjs`；最小冒烟连续两次 passed；12 步产品链 passed |
| 2. 工作区学生预览与实际学生端语义对齐 | **完成** | `studentPreview.ts` 走官方编译器；`ExamCanvas mode="student"`；共享选择阈值改 `cardinality.max`；新增答案键类型判定 `validateReadingAnswerKeyKinds()`（对齐 `reading_runtime_v2.rs`） |
| 3. 前端保持一份题稿、只显示统一后的待确认问题 | **完成（只读路径已验，写路径待后端）** | `RecognitionPanel` + `recognitionDecisions.ts`；不展示本地/云端/校验三稿；`agreed`/`info`/`superseded` 不出现 |
| 4. 接入后台运行状态、统一建议及决策接口 | **部分完成** | 读路径已对真实后端验证；wire 形状转换已完成（契约漂移 0 破坏）；写路径无真实候选项可验 |
| 5. 真实 PDF/DOCX → 发布 → 学生端加载/作答/提交/计分 | **未完成** | 发布恒被 `blocked_by_quality_gate`，无 NAS 包（`manifestExists: false`），无产物可交学生端 |

### 交接出去的部分（按任务书停止扩展，已列出文件/完成度/接口）

见 `Plan With Files/Dual_Recognition/HANDOFF_2026-09-15_gate_resolution_blindness.md`：
- 质量门禁无视 `resolution`（`quality.rs:211` + `authoring_v2_commands.rs:430` + `refresh_quality_report` 顺序），
  导致任何出现过硬失败的题稿永久无法发布 → 交给识别/校验后端 agent。
- `RUNTIME_COMPILER_FAILED` 真实原因：选项型槽位的答案键被写成 `kind:"text"`（应为 `kind:"option"`）→ 识别侧产出问题。
- `recognitionBlockers: QUESTION_NUMBER_MISSING` 与 `answerSlots[].questionNumber` 已存在的矛盾 → 待后端解释。

本 agent **未修改** `src-tauri/src/ielts_grammar/**` 与 `src-tauri/src/authoring_v2_commands.rs` 的任何逻辑。

### 需要后端回复的 4 个问题

1. `resolution` 语义最终由谁落地？（若前端提供「确认/忽略」入口，需先修门禁，否则按钮无效）
2. `hardFailures` 能否改为携带 issueId（便于按 resolution 精确过滤）？
3. `get_recognition_decision` 按契约补 `cloudStatus`/`localStatus`/`items`，还是双方确认以 `chains`/`actionable` 为准并改契约文档？
4. `QUESTION_NUMBER_MISSING` 与已存在的 `questionNumber` 矛盾如何解释？

### 未完成项（不得当作已交付）

- 学生端（Electron）加载/作答/提交/计分验收：**未完成**（无可用发布产物）。
- 识别建议面板写路径（采用修正/保持现状）：**未完成**（无真实候选项）。
- `resolution` 落地 UI 入口：**未完成且刻意不做**（门禁未修前是假按钮）。
- 与真实云端服务的一次成功调用：**未完成**（无凭据，`cloudEnabled=false`）。

---

## R7 轮更新（2026-09-15）：撤销语义修正 + 构建新鲜度恢复 + 交接

### R7 状态表

| 项 | 状态 | 证据 |
| --- | --- | --- |
| 1. 恢复真实 Tauri 产品验证通道 | **完成** | 重建内嵌前端二进制（`tauri build --debug --no-bundle`，exe 23:09:40）；`buildFresh.ok=true, toleratedConcurrentEdits=0`；PDF 与 DOCX 两条 12 步链 passed（publish 预期 blocked） |
| 2. 学生预览与学生端语义对齐 | **完成** | 同上两条链的 `student-preview-renders` / `student-preview-answering-isolated` / `preview-and-gate-agree` 全 passed |
| 3. 单一题稿 + 只显示统一待确认问题 | **完成（本轮逐条核实，不需改动）** | 全仓仅一处「本地稿/云端稿」字样且是注释；工作区只有一条版本化保存链 + 一份预览；面板只消费一份统一建议集合 |
| 4. 接入后台状态/统一建议/决策接口 | **接线完成，用户流程未验收** | 读路径对真实后端已验证；写入面**两层**漂移均已修（字段名 + `input` 包装）；新增真实 IPC 第 9 步验证撤销通道写入权威稿；但真实候选项为 0，按钮级流程无法验收 |
| 5. 真实 PDF/DOCX → 发布 → 学生端 | **未完成** | 发布仍全部 `blocked_by_quality_gate`（门禁对 `resolution` 无视，属后端） |

### R7 本轮修掉的缺陷（本 agent 自身文件）

**P0 假完成：「撤销自动修正」根本不撤销。** 面板把撤销实现成 `reject` 决策，
而后端 reject 的语义是「只改状态，不碰权威稿」，于是界面说「已保持现状」而自动修正
仍留在权威稿里。契约 §4.4/§6.1 要求的是「提交 `undo` 作为 editor 命令」。

修复：`parseUndoPatch`（严格校验形状，认不出来**不给按钮**）→ `RecognitionPanel`
经 `onUndoAutoFix` → `ExamWorkspacePage` 用 `applyAuthoringV2Patches` 离线试算 +
`editor.applyPatch` + `await editor.flush()` 走版本化事务。

**同类自检**：新增的 E2E 第 9 步第一版是**空断言**（探针值与原值相同 → 恒真），
已改为「写入值必须与原值不同，否则拒绝执行」。这是同类错误第三次（F-R5-3 / F-R5-8 / F-R7-2）。

### R7 新增交接

`Plan With Files/Dual_Recognition/HANDOFF_2026-09-15_frontend_write_path_and_undo_fixed.md`：
- 对方清单 §3.1 / §4 已全部落地；§3.2「本组件无需改动」就契约漂移而言正确，
  但漏了本轮这个**独立语义缺陷**；
- 对方 §1 表与契约 §10.2 表已过期（仍写「写入面至今未修」「2 处破坏性不一致」，
  实际 0 破坏 / 1 留意）——两文件属对方独占写入区，**未改动**；
- 重申检查器结构性盲区（只比对结构体字段名，看不到 Tauri 命令包装层）；
- 回复契约 §9 三个待确认点；
- 新增一问：撤销后该 `auto_fixed` 是否应离开 `autoApplied`？（`build_view` 目前不看 `status`）

### R7 未完成项（不得当作已交付）

- **「本轮真实导入 → 发布 → 学生端」的耦合验收：未完成。** 学生侧本身已通过（真 Electron
  加载/作答/提交/服务端计分 `is_correct=1`、HTTP 载荷无 `answerKey`），但它消费的是
  **预置 ready 夹具**，不是本轮导入发布的产物——因为发布被门禁拦下。
- **面板「采用修正 / 保持现状 / 撤销」的用户流程：未完成**（真实候选项为 0，只能验接线）。
- **`resolution` 落地 UI 入口：未完成且刻意不做**（门禁未修前是假按钮）。
- **与真实云端服务的一次成功调用：未完成**（无凭据，`cloudEnabled=false`）。

---

## R8 轮（2026-09-15）新任务书：先修验收判定

优先级：**准确的验收报告 → 真实候选操作 → 真实导入发布 → 实际学生端提交计分**。
不继续增加预览提示。我（前端 agent）独占 `src/**` 与 `scripts/e2e/**`，不自行修改后端门禁。

### 任务一：修正完整链报告假绿 —— **完成**

| 项 | 状态 | 证据 |
| --- | --- | --- |
| 明确完整链必需步骤 | **完成** | `FULL_CHAIN_REQUIRED_STEPS`（12 步含发布）/ `EDIT_PREVIEW_REQUIRED_STEPS`（11 步不含发布） |
| 必需步骤 blocked/未执行/缺失 → 整体不得 passed、退出码非 0 | **完成** | `blocked`→4、`incomplete`→2；实测同一次运行由 `passed/0` 变为 `blocked/4` |
| 编辑/预览专项不得沿用完整链成功名 | **完成** | `--scope=edit-preview-specialty` → `passed-specialty`；且**不注册**发布步骤（提前 return 会被记成 passed，不能用） |
| 门禁拦坏题可作独立负例通过，不得冒充发布成功 | **完成** | `--expect-blocked` → `passed-negative-case`；负例前提不成立（发布成功）→ `failed` |
| 不覆盖旧报告，保留历史证据并说明判定缺陷 | **完成** | 15 份受影响报告未删；`artifacts/e2e-cdp/VERDICT_DEFECT_NOTE.md` 列清单 + 复核命令 |
| 判定回归测试 | **完成** | `scripts/e2e/lib/chain-verdict.test.mjs` 19 条（发布 blocked / 步骤缺失 / manifest 缺失 / 题目 JS 缺失 / 资源缺失 / 预检不一致 / 负例 / 专项 / CANNOT-RUN） |
| 保留 `diagnosticRun`，诊断运行与默认运行分开记录 | **完成** | `runProfile` = `cdp-default` / `cdp-diagnostic` + `securityArgs`；默认不再带测试专用安全参数 |

**顺带修掉的第二个判定洞**：判定原在 `if (recorder)` 内，CANNOT-RUN 时 `recorder` 为 `null`
→ 判定根本不执行、`exitCode` 为 `undefined`。已改为无条件判定。

### 任务二/三/四：状态

| 任务 | 状态 | 阻塞点 |
| --- | --- | --- |
| 任务二 用户知道如何解决问题 | **未开始** | 待后端定义「可人工确认 vs 必须修数据」的问题处理能力 |
| 任务三 真实候选按钮流程 | **断言框架已就绪，场景无法执行** | 脚本已建（5 场景全走真实点击），但本仓无真实候选项 → 判 `not-executable`（退出码 5），**不报 passed** |
| 任务四 本轮发布产物 → 学生端 | **未完成** | 发布仍被质量门禁拦下（后端在修） |

### 任务三 交付的断言框架

`scripts/e2e/tauri-cdp-recognition-buttons.mjs`（真实界面点击，非 IPC 探针）：

| 场景 | 断言 |
| --- | --- |
| `accept-suggestion` | 点击接受 → 权威稿内容改变 + 版本递增 + 重开后状态仍为 accepted |
| `reject-suggestion` | 点击保持现状 → 权威稿**一字未改** + 重开后状态仍为 rejected |
| `undo-auto-fix` | 点击撤销 → 槽位值等于 `undo.value` + 版本递增 + 重开后不再提供「撤销」 |
| `stale-suggestion-protected` | 先经编辑器写入用户值 → 再接受旧建议 → 用户值必须保留 |
| `idempotent-retry` | 同一次事件循环点两次接受 → 版本只递增 **1** 次 |
| （无候选） | 全部记为 `not-executable`，判定 `not-executable`（5），**绝不 passed** |

判定用 `computeScenarioVerdict`（纯函数 + 7 条回归）：三态 `passed` / `failed` /
`not-executable`，后者是**独立的第三态**，不是失败也不是通过。

**实测**（`run-recog-buttons-2026-09-15T22-46-09-621Z`）：

```text
candidates: actionableCount=0, needsReviewCount=0, autoFixedCount=0, batchId=null, chains.*=not_run
5 个场景全部 not-executable
verdict=not-executable  exit=5
reason=以下场景本次无法执行（前提不成立，例如没有真实候选项）：… 这不等于通过。
```

### R8 未完成项（不得当作已交付）

- **`cdp-default`（无测试专用安全参数）的通过证据**：本沙箱不可得——受控对照显示
  默认档案下 renderer 在第一步之前就死（`CDP 连接已关闭`），诊断档案正常。
  故本环境所有证据均为 `runProfile=cdp-diagnostic`。
- 任务二、任务三、任务四：见上表。
- 与真实云端服务的一次成功调用：未完成（无凭据）。

---

# R9：发布阻塞归因（本轮第一优先级）

本轮任务书把顺序改成「**先解除真实发布阻塞，再继续模型链路**」。要解除它，先得知道它到底是什么。

## R9 任务表

| 项 | 状态 | 证据 / 说明 |
| --- | --- | --- |
| 发布阻塞**归因** | **完成** | 新探针 `scripts/e2e/tauri-cdp-publish-unblock-probe.mjs`，三次运行（两个夹具） |
| 任务一（验收报告假绿） | **完成**（上一轮） | 见 R8；本轮未回退 |
| 任务二（用户如何解决问题） | **部分完成** | 已修泛化重复 + 诚实的定位反馈；**「确认」动作被门禁挡住，不能做** |
| 任务三（真实候选按钮） | **框架就绪 / 场景无法执行** | 无真实候选项；`verdict=not-executable exit=5` |
| 任务四（发布产物 → 学生端） | **未完成** | 发布仍被 `WORD_LIMIT_UNPARSED` 误判拦下，`manifestExists=false` |

## R9 归因结论（一句话）

发布阻塞**不是**「数据没改对」：走真实界面把 14 个答案填上之后，
`hardFailures` 从 `["WORD_LIMIT_UNPARSED","ANSWER_KEY_MISSING_SLOT","RUNTIME_COMPILER_FAILED"]`
降到 `["WORD_LIMIT_UNPARSED"]` —— **其余阻塞都被真实数据修复清掉了**。

剩下的 `WORD_LIMIT_UNPARSED` 是**门禁误判**：它出现在一个按字母选的 summary completion
（"Complete the summary using the list of words and phrases, A-H"）上，该题型本就没有 word limit。
而它给的两个补救动作 `edit_text` / `confirm_table` **都是死路**
（前者：`instructionSignature` 只被 `set_task_type`/`set_question_expression` 改写，`replace_text` 不重算；
后者：门禁无视 `resolution`）。**用户看得到错误、永远修不掉 → 发布不可能发生。**

## R9 已交付

| 文件 | 内容 |
| --- | --- |
| `scripts/e2e/tauri-cdp-publish-unblock-probe.mjs` | 三阶段归因探针（诊断用，`isAcceptanceEvidence: false`） |
| `scripts/e2e/tauri-cdp-issue-list.mjs` | 问题列表**真实界面**校验（8 条断言；含「同一根因只占一行」的 DOM 不变量） |
| `Plan With Files/Dual_Recognition/HANDOFF_2026-09-15_publish_blocker_attribution.md` | 给后端的交接（含三条需要他们决定/修的事项） |
| `src/features/editor/actionableIssues.ts` | `rootCauseOf()`；去掉泛化 `QUALITY_HARD_FAILURE`；`ROOT_CAUSE_ALIASES` 去掉跨来源同根因重复 |
| `src/features/editor/ExamWorkspacePage.tsx` | 定位失败时如实说明；行加 `data-issue-code`/`data-issue-source`；`locateTarget` 兼按 `hostNodeId` 找 |
| `src/features/editor/publishGateIssues.test.ts` | 11 → 16 单测 |

真实产物回放校验：blockers **34 → 31**（两次独立运行一致）。
单测 **167 passed / 12 files**；`tsc --noEmit` 干净。

## R9-8 补做：问题列表的真实界面证据

R9 改的两处用户可见行为只有单测证据，本轮补了真实 DOM 校验
（`run-issue-list-2026-09-15T23-31-41-068Z`，**8/8 断言通过，exit=0**）：

```text
gate raw=34 warnings=1 expectedGate=18 expectedLocal=14
rendered=32                     # 修之前是 46
同一根因（归一后）在界面上只占一行 :: {"duplicated":[],"total":32}
本地来源的行数 = 独立算法算出的期望（没有被多删）:: {"rendered":14,"expected":14}
```

校验过程中查出**真实产品缺陷**：本地闭包记 `ANSWER_UNRESOLVED`、门禁记 `ANSWER_MISSING`，
同一道题渲染两行（14 个未解析答案位 → 白多 14 行）。别名**可证明**：
两边都用 `answerKey[slot].kind === "unresolved"` 这同一个谓词。已修（46 → 32）。

## R9-8 更正上一轮的 F-R9-4

上一轮写「`complex-reading.pdf` group-2 有两条 `SLOT_HOST_MISSING`（同码同目标、两条不同事实）」——
回原始产物核对后**不成立**：那两条逐字节相同（`internal` 也相同），是后端把同一个问题推了两遍。
前端收成一行是对的；真正缺陷是后端重复推送。**计数相同不等于内容不同。**

## R9 未完成项（不得当作已交付）

- **任务二的核心部分（可确认的疑点提供确认动作）**：**不能做**。后端已给出 `suggestedActions`
  含 `confirm_table`，但门禁无视 `resolution`（实测），现在加按钮就是**死按钮**，
  违反任务书「不提供能忽略…的通用按钮」。**依赖后端修门禁。**
- **任务三的 5 个场景本身**：无真实候选项，无法执行。
- **任务四**：发布未发生（`manifestExists=false`），因此**没有任何**完整链通过可报告。
- **探针阶段 C 的运行时补证**：**已完成** —— 说明文字改后保存（版本 16→17），
  但 `instructionSignature.normalizedText` 逐字节不变、`wordLimit` 仍为 `null`、
  `WORD_LIMIT_UNPARSED` 仍 blocking。`edit_text` 是死路，已成实测结论。
  同一次调查还抓到自己一个假阳性（探针点偏了，误以为说明文字不可编辑），已在工具内修正。
- `cdp-default` 通过证据、真实云服务调用：仍未完成。

---

## R10 任务表

| 项 | 状态 | 证据 / 说明 |
| --- | --- | --- |
| 1 完整链必需步骤必须明确 passed | **完成** | `chain-verdict.mjs` 加 `notOutcome`；单测 26 → 33；实跑 `blocked exit=4` / `passed-negative-case exit=0` |
| 2 过期候选验证实际 outcome | **完成（断言）/ 未执行（场景）** | 场景 4 重写为三条硬要求；实跑 5 场景 `not-executable exit=5` |
| 3 去重必须保留不同事实 | **完成** | `sameFact()`；逐键计数断言；PDF 7/7（32 行）、complex-reading 7/7（3 → 5 行） |
| 4 后端持久化撤销 | **完成** | 删会话内 `Set`，改 `isUndoAlreadyApplied`；单测 25 → 32 |
| 5 查明构建失败原因 | **未完成** | 处置+护栏已做；**原因未查明**（不编解释） |
| 6 真实候选 + 学生端 | **未完成** | 被后端阻塞；本轮复核后仍不可开始（见 R11） |

详见 `Plan With Files/Dual_Recognition/REPORT_2026-09-16_round10_frontend_agent2.md`。

---

## R11 任务表（后端交付已到，但交付物编译不过）

| 项 | 状态 | 证据 / 说明 |
| --- | --- | --- |
| 复核「后端是否已交付」 | **完成** | `reconcile/**` 已接线（`lib.rs:87/972/986`）；契约漂移 **0 处破坏性**（exit=0）；写入面已按交接单修好 |
| 重建可验收二进制 | **失败（后端编译错误）** | `VITE_EXIT=0` / `TAURI_EXIT=1`；4 个错误全在 `auto_pipeline.rs`（564/595/607/615） |
| 归属核实 | **完成** | `git diff --stat auto_pipeline.rs` = `446 insertions(+), 0 deletions(-)`，全为后端未提交新增 |
| 并发写入检测 | **完成** | 构建前后指纹对比：`scheduler.rs`、`instruction_signature.rs`、`quality.rs` 在构建期间被改 |
| 前端独立验证 | **完成** | `tsc` 干净；`vitest` **186 passed / 12 files**（不依赖 Rust 构建） |
| 6 真实候选 + 学生端 | **未开始** | 构建层阻塞：连可执行二进制都没有 |

## R11 结论（一句话）

**交付到了，但交付物不编译** —— 后端在 `auto_pipeline.rs` 新增的 DOCX 纯文本抽取
（446 行未提交）有 4 个借用错误，`ielts-author-studio` lib 编译失败，
完整链/问题列表/候选按钮流程**全部无法运行**。已按所有权约定**未改动后端文件**，
只出交接单。任务 6 **仍不可开始**，本轮**不宣称任何完整链通过**。

## R11 已交付

| 文件 | 内容 |
| --- | --- |
| `Plan With Files/Dual_Recognition/HANDOFF_2026-09-16_build_broken_blocks_all_e2e.md` | 4 个错误的行号 + 最小修法 + 并发写入证据 + 「请留稳定点」 |
| `findings.md` | F-R11-1 / F-R11-2 / F-R11-3 |
| `progress.md` | R11 段 |
| `artifacts/tauri-build.log`、`tauri-build-retry.log`、`prebuild*/postbuild*-fingerprint.txt` | 构建与指纹证据 |

## R11 未完成（不得当作已交付）

- 任务 6 全部（真实候选按钮流程、PDF/DOCX 发布到学生端计分）—— **构建层阻塞**；
- 后端编译错误（4 处，`auto_pipeline.rs`）—— 不在我的写入面，已交接，**未修**；
- 任务 5 的根因（R10 遗留）；
- 后端待修（R10 遗留）：`issueId` 唯一性、门禁无视 `resolution`、
  `BLOCKER_LIST_TRUNCATED` 文案、无持久化「已撤销」状态；
- `cdp-default` 通过证据、真实云服务调用。

---

## R11 续：后端修好构建；稳定事实 id 采用并验证

| 项 | 状态 | 证据 / 说明 |
| --- | --- | --- |
| 构建解除 | **已解除** | 后端 `11:11:22` 修好 `auto_pipeline.rs`；`vite exit=0` / `tauri exit=0` / `error_count=0`；构建期间**无**并发写入 → 结果可归因 |
| 构建二元性的新坑 | **已记录** | `tauri build --no-bundle` + `beforeBuildCommand:""` **不重建前端**；必须 `vite build` → `tauri build` 两步，否则拿到「假 fresh」的 exe |
| 采用后端稳定事实 id | **完成** | `issueId` = `phase4-{code}-{target}-{slug}`；`factId` 单向判据；行 key 唯一化；`data-issue-fact-id` |
| 问题列表真实界面验证 | **完成** | PDF **8/8 passed exit=0**（`missing=[] duplicated=[]`，16/16）；complex-reading **7/7 passed exit=0**（5 行，`mismatches=[]`） |
| 预检尊重 `resolution` | **已解除** | 后端加了 `!matches!(details/resolution, resolved\|ignored)`（R10 交接单第 2 条） |
| 前端单测 | **完成** | `tsc` 干净；`vitest` **191 passed / 12 files**（186 → 191） |
| 6 真实候选按钮流程 | **未完成** | 5 场景 `not-executable exit=5`；`batchId=null`、四条链全 `not_run` |
| 6 的根因 | **已定位** | `launch_cloud = cloud_will_run && resolved_profile.is_some()`；无 profile → scheduler 提前 `return` → reconcile 从不运行 |

## R11 续结论（一句话）

**构建已解除、后端交付已到、任务书第 3 条的稳定事实 id 已采用并在真实界面上验证通过（8/8 + 7/7）；
但任务书第 6 条的候选按钮流程在无云 profile 的环境里结构上不可达** ——
不是夹具没有候选，而是 `launch_cloud=false` 时 `scheduler` 提前返回，reconcile 根本不运行。

## R11 续未完成（不得当作已交付）

- 任务 6 全部：真实候选按钮流程 5 场景、PDF/DOCX 完整链、学生端提交计分 —— **需要云 profile**；
- **默认路径（`cdp-default`）通过证据**：本轮全部真实界面运行都是 `cdp-diagnostic`
  （沙箱需要 `--no-sandbox --disable-gpu`），**不得报成默认路径通过**；
- 真实云服务调用、学生端计分证据；
- 任务 5 的根因（R10 遗留）；
- 后端待确认：契约 §3.2 宣称的 `source`（原文件核验）独立链，在无云时不可达；
  `BLOCKER_LIST_TRUNCATED` 文案与截断条件仍不符；无持久化「已撤销」状态。

## 2026-09-19 本轮收口记录

- 合并：`aa443d5`；合并后基线 Rust 850/0/11、Vitest 309/0；real-PDF harness 仍受缺失 private corpus 的环境条件影响。
- 产品修复：DOCX response prompt 投影、blocking 无目标动作、retry 新 batch/人工保护、force strict、CAS `Ok(0)`、PublishVerdict 等价性、readiness issue 落盘均已实现或验证。
- 待最终阶段：全量 Rust/Vitest/PDF harness 重跑，核对状态后清理 `feat-publish-chain` worktree/branch。

## 2026-09-19 本轮最终验收

- 合并提交 `aa443d5` 已落在 `main`；`feat-publish-chain` worktree 与分支已清理，恢复单一工作区。
- 合并后最终 Rust：`856 passed / 0 failed / 11 ignored`；Vitest：`19 files / 311 passed / 0 failed`；TypeScript `tsc --noEmit` 通过。
- `verify:phase5:real-pdf` 的 Rust exact test 明确 `SKIP`（缺 8 个 private-real PDF），外层 Node 因无 `report.json` 退出 1；与合并前环境红一致。
- 真实 `complex-reading.docx` 已覆盖导入→V2→canonical→批次→云端修复 HTTP 网关，题面无 placeholder，云端请求携带 DOCX `sourceText`。

## 2026-09-19 收口纠正：提交与指定 CDP 链

- 功能代码已按归属提交：`9838398`（DOCX）、`9945be5`（重新识别）、`59c1a78`（blocking 前端反例）、`9e83292`（force）、`557f24b`（CAS）、`d9081c7`（PublishVerdict/readiness 测试与持久化）。
- `node scripts/e2e/tauri-cdp-cloud-repair-chain.mjs` 使用仓库内 `fixtures/parser/demanding-reading-passage-3.pdf`，在刷新 dist 与 Tauri debug exe 后 **13/13 场景通过，退出码 0**。此前把该链报告成 private-real 缺失是错误归因；private-real 缺失属于另一条 `verify:phase5:real-pdf` 脚本。
- 用同一条 CDP 链指定 `fixtures/parser/complex-reading.docx`：真实 UI 导入通过，`placeholderPrompts=0`，但在 `derive-scenario-from-real-draft` 因 PDF golden hash（`f13bd65c...`）与 DOCX hash（`717918f5...`）不匹配而停止；该运行的真实 V2 质量报告同时为 `blocked`，含 `SLOT_HOST_MISSING` 与 `RUNTIME_COMPILER_FAILED`。因此不能把 DOCX 宣称为已经走完云端修复→预览→导出。
