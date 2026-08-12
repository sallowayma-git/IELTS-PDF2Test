# Current Active Goal: IELTS 文档识别重构 Phase 5 结构化编辑器纵向切片

## Goal

依据 [Files/IELTS_Document_Recognition_Overhaul_Plan_CN.md](Files/IELTS_Document_Recognition_Overhaul_Plan_CN.md) 第 16.6 节，先交付一个可运行的结构化编辑器纵向切片：普通用户可以直接编辑 V2 内容节点、题组、共享答案位、选项库和答案；问题可以定位到源锚点；同一份草稿可以切换学生预览；编辑通过防抖 patch 保存，并具备版本冲突和本地崩溃恢复能力。

- [complete] S1 建立 Phase 5 V2 session、patch 协议与 immutable revision 保存；保留 V1 文件可读。
- [complete] S2 完成结构化编辑器页面、题组/共享答案位/选项库/答案编辑与 issue rail/source overlay。
- [complete] S3 接通 student parity preview、650ms autosave、乐观版本冲突提示和 localStorage recovery。
- [complete] S4 增加 `/phase5` fixture 路由、现有 job 的 `authoring-v2` 路由与生产默认关闭的 feature flag。
- [complete] S5 完成前端构建、Phase 1 schema 回归、Rust check/test 与 Phase 5 专用验证。
- [pending] S6 扩展完整节点增删移动、table/asset/hotspot 编辑、undo/redo、原生 Tiptap schema 适配与真实 PDF 端到端验收。

## Current Decisions

- 本轮以“可验证的第一条纵向切片”为 Phase 5 目标，不宣称已完成 4–6 周全部工作量。
- 编辑器当前采用 V2 schema-driven React 编辑器，先稳定 patch/revision/source/preview 契约；Tiptap 适配留在 S6。
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
