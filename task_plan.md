# Current Active Goal: IELTS 文档识别重构 Phase 5 全量交付

## Goal

依据 [Files/IELTS_Document_Recognition_Overhaul_Plan_CN.md](Files/IELTS_Document_Recognition_Overhaul_Plan_CN.md) 第 16.6、20.4、24.10 和 24.11 节，完成结构化编辑器 Phase 5 全部范围：普通用户可直接编辑 V2 内容节点、题组、共享答案位、选项库、表格、资源、热点和答案；问题可定位到源锚点；同一份草稿可切换学生预览；编辑具备防抖 patch、immutable revision、版本冲突、本地崩溃恢复、undo/redo 和 V2 bundle export；真实 PDF 从导入到导出通过端到端验收。

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

- [Files/IELTS_Document_Recognition_Overhaul_Plan_CN.md](Files/IELTS_Document_Recognition_Overhaul_Plan_CN.md)
- [Files/IELTS_Document_Recognition_Phase_5_Progress_CN.md](Files/IELTS_Document_Recognition_Phase_5_Progress_CN.md)

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
