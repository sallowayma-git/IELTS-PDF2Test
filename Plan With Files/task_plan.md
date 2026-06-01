# Task Plan: Epic 8 Tauri Authoring App

## Goal
实现 `Epic8-Tauri作者端应用详细设计.md` 描述的本地 Tauri 作者端应用全部开发任务，并持续维护工程追踪记录表、发现记录和进度日志。

## Current Phase
Phase 5: Verification & Completion

## Engineering Tracking Table
| ID | Task | Source | Status | Dependencies | Acceptance |
|----|------|--------|--------|--------------|------------|
| E8-00 | 初始化 Plan With Files 工作区与工程追踪记录 | User request | complete | 无 | 根目录存在 `Plan With Files/task_plan.md`、`findings.md`、`progress.md` |
| E8-01 | 细读 Tauri 设计文档并用旧工程文档抽取输出契约 | Tauri + output-contract docs | complete | E8-00 | 任务表覆盖页面、Rust command、数据模型、解析、LLM、预览、导出、设置 |
| E8-02 | 选择并搭建 Tauri 本地应用与内嵌界面工程骨架 | Tauri doc: 开发顺序 | complete | E8-01 | 可启动桌面开发环境与内嵌 UI，基础页面路由存在 |
| E8-03 | 实现本地数据目录、配置与 Job 存储 | Tauri doc: 本地数据目录、Rust 模型 | complete | E8-02 | Rust `cargo check` 与 Tauri release build 已验证 app data/job 存储命令可编译 |
| E8-04 | 实现文件导入与 parser 接入骨架 | Tauri doc: Files/Parser commands | complete | E8-03 | TXT/MD/PDF/DOCX 已改为 Rust 主解析；Python sidecar 仅保留 legacy fallback 和 PDF 嵌入图片抽取；macOS 图片 PDF 可由 Rust 调 `sips` 渲染后交给视觉 LLM |
| E8-05 | 实现规则粗切、答案对齐与 Authoring IR 编辑 | Tauri + output-contract pipeline | complete | E8-04 | 可从 Document IR 生成题组草稿并在 UI 编辑；新增上传后自动流水线（解析、粗切、AuthoringIR、LLM批处理、校验/E2E） |
| E8-06 | 实现 LLM profile、密钥存储、调用与建议审阅 | Tauri doc: LLM 设置与安全、LLM Review | in_progress | E8-05 | LLM profile、跨平台 OS secure storage 默认存储（keyring native backends 已显式启用）、Rust OpenAI-compatible HTTP gateway、结构化建议与审阅 UI 已接入；高置信自动应用经 schema/evidence/sourceBlockIds/quotes 白名单校验；仍需真实 provider 覆盖与 Windows 实机凭据 smoke |
| E8-07 | 实现模板渲染、统一阅读页预览与校验 | Tauri + output-contract docs | in_progress | E8-05 | JS/manifest 生成、Rust ReadingExamSourceV1/DOM 静态合同校验已接入；真实 runtime E2E 保留为开发/CI/诊断命令，不再作为普通生产导出硬依赖；仍需更多复杂题型与完整 UI E2E |
| E8-08 | 实现 PackBuilder、JS 导出和组 Pack 发布 | Tauri doc: Pack 发布 | in_progress | E8-07 | 单题 JS/manifest、Pack 目录/标准 `.zip` 已实现；导出/Pack 已强制 SourceReview + AuthoringReview + Rust 静态合同门禁；生产不打包 Node/Python/OCR，真实 runtime E2E 为诊断项 |
| E8-09 | 完成 Dashboard、ImportWizard、DocumentReview、SplitAndAnswers、GroupEditor、LlmReview、UnifiedPreview、PackBuilder、Settings 全页面 | Tauri doc: 页面详细设计 | complete | E8-02..E8-08 | 页面流程闭环且状态可持久化 |
| E8-10 | 验收测试、错误修复与文档同步 | Tauri doc: 最小 MVP 范围、关键验收用例 | in_progress | E8-03..E8-09 | 已完成构建/语法/冒烟验证、no-text/image PDF、复杂 TXT/MD/PDF/DOCX、Rust 静态 export/Pack 回归；扫描 PDF 策略已定为视觉 LLM + SourceReview + 手工兜底，仍需更广 UI E2E 和真实服务覆盖 |
| E8-11 | 审计后发布硬门禁补齐 | Deep audit 2026-05-31 | complete | E8-06..E8-10 | 已新增 `publish_readiness_gate`、SourceReviewV1、no-text/image PDF fixture、Rust 静态合同 gate、命令级导出/Pack static fixture；OCR 策略定为视觉 LLM + 人工 SourceReview，不打包本地 OCR |
| E8-12 | 审计后架构拆分与测试基线 | Deep audit 2026-05-31 | in_progress | E8-03..E8-10 | 已新增 parser no-text/image PDF、complex TXT/MD/PDF/DOCX、source metadata、export/Pack static core、路径段安全、Rust LLM gateway 测试；已拆出 parser/source review/auto pipeline/export-pack/LLM/preview-authoring/job command modules，并落地首个 typed SourceReview seam，仍需扩展 typed-domain refactor 与更多 UI E2E |
| E8-13 | 生产化交付、安全与依赖自包含审计 | Deep audit 2026-05-31 15:40 | in_progress | E8-06..E8-12 | 已加固明文密钥兜底默认禁用、路径段校验、sandbox iframe 预览和显式 CSP；最新策略是不打包 Node/Python/OCR，剩余 Python/pypdf 仅为 legacy/嵌入图片可选能力，Node 仅诊断 |

## Phases

### Phase 1: Requirements & Discovery
- [x] 创建 Plan With Files 文件夹与三份文档
- [x] 阅读并摘要两份设计文档
- [x] 拆分本地端全部开发任务
- [x] 识别当前仓库状态、工具链和约束
- **Status:** complete

### Phase 2: Architecture & Scaffold
- [x] 决定 Tauri/Rust/内嵌界面技术栈与目录结构
- [x] 初始化 package、src、src-tauri 等工程文件
- [x] 建立共享类型、API 调用封装与基础路由
- **Status:** complete

### Phase 3: Core Local Backend
- [x] 实现 Rust 数据模型、存储、命令 API
- [x] 实现导入、解析骨架、粗切、校验、导出服务
- [x] 将规则粗切与 Authoring IR 生成改为优先从 `DocumentIRV1` 动态推导
- [x] 将 TXT/MD Python parser sidecar 与 Node validator sidecar 接入 Rust command 优先路径
- [x] 建立 Rust 工具链后的命令级验证
- [x] 建立桌面文件选择器与导出目录选择 UI
- [x] 添加 txt/md parser sidecar 与 Node validator sidecar 入口
- [x] 添加 PDF/DOCX deterministic parser adapter：PDF 先走 Rust `pdf-extract`，DOCX 先走 Rust OOXML 解析，Python sidecar 仅作为 legacy fallback/图片链路依赖
- **Status:** complete

### Phase 4: Authoring UI
- [x] 实现 9 个页面的主要交互骨架
- [x] 接通页面到 Tauri command/dev fallback 的主流程
- [x] 添加本地 RuntimePreview contract simulator，执行生成的 manifest/wrapper 并校验自动填正确答案 100%
- [x] 完成真实/模拟 runtime 状态语义修正、导入页/LLM 审阅页/DocumentReview/JobList/PackBuilder 的风险可见性修订
- [ ] 完成系统 Stronghold/安全 hardening、手工修订审计、复杂题型 UI E2E 等最终闭环
- **Status:** complete

### Phase 5: Verification & Completion
- [x] 跑通 MVP 导题链路
- [x] 实现上传后自动流水线：自动解析、粗切、生成 AuthoringIR、批量 LLM 建议、高置信落库、低置信待审、校验/E2E
- [x] 修复关键门禁策略：发布链路默认 Rust 静态合同 gate + SourceReview/AuthoringReview，真实 runtime E2E 退为诊断/CI 项
- [x] 修复配置移植性风险：移除代码内本机绝对路径默认值，仅保留 `EPIC8_UNIFIED_HTML_PATH`/`EPIC8_UNIFIED_PYTHON` 注入
- [x] 完成当前实现深度审计，识别发布硬门禁与复杂 PDF/OCR 缺口
- [ ] 完成最终验收清单执行（Rust 静态 gate、no-text/image PDF、复杂 TXT/MD/PDF/DOCX、export/Pack 命令级证据、视觉 LLM/人工转录策略、路径/CSP/密钥 hardening 已补；模块拆分、真实 provider 覆盖和更广 UI E2E 仍未完成）
- [ ] 完成最终交付说明
- **Status:** in_progress

## Key Questions
1. 当前机器是否具备 Rust、Node、Tauri 构建工具链？
2. 设计文档要求的 Python parser sidecar 是否已有代码，还是本轮需要实现最小替代解析器？
3. 最终导出 JS/manifest 需要兼容哪个现有运行时项目？当前工作区是否包含该项目代码？
4. LLM 调用应先实现真实 provider，还是先实现本地占位/可配置 gateway 以保证离线 MVP？

## Decisions Made
| Decision | Rationale |
|----------|-----------|
| 将计划文件放在根目录 `Plan With Files/` 而非根目录平铺 | 用户明确要求建立 `Plan With Files` 文件夹放置三份文档 |
| 先以 MVP 闭环为开发顺序，再扩展完整页面与 LLM | Tauri 设计文档明确给出最小 MVP 范围和开发顺序 |
| 旧 Web 设计只作为输出契约参考，不作为作者端产品形态 | 用户明确更正作者端没有独立 Web |
| 保留 `ReadingExamSourceV1.sourceRefs.primaryProvider = "author_web"` | 旧输出契约明确该字段，可能被学生端运行时兼容逻辑依赖；用 audit notes 标识本地 Tauri 来源 |

## Errors Encountered
| Error | Attempt | Resolution |
|-------|---------|------------|
| `/goal` create failed because thread already has active goal | 1 | 使用现有活动目标继续推进 |
| Rust dynamic helper 大块替换 patch 上下文不匹配 | 1 | 改为新增动态 helper 并切换 command 入口，保留静态样例兜底 |
| Browser smoke 暴露题干重复 | 1 | `blockText`/`dynamic_block_text` 改为优先使用 `text`，仅在缺失时 fallback 到 stripped HTML |
| `rustc` / `cargo` 不在 PATH | 多次 | 2026-05-30 已用官方 `rustup` 安装 Rust stable 1.96.0，并配置 `~/.zshrc`/`~/.zprofile` |
| `brew install rustup-init` 在 macOS 13 上转为源码编译 CMake，耗时过长 | 1 | 终止 Homebrew 安装树并清理缓存，改用官方 rustup 安装 |
| 首次 `cargo check` 发现 Rust 函数重名、类型转换、图标缺失等编译错误 | 1 | 修复 `render_group_html` 重名、`Value::String` 转换、`write_json` 泛型约束，添加 `src-tauri/icons/icon.png` |
| `cargo clippy --all-targets -D warnings` 发现 10 个惯用法 lint | 1 | 按 clippy 建议修复后通过 |
| Homebrew Python `pip install --user` 触发 PEP 668 externally managed environment | 1 | 不污染全局 Python；PDF 使用已存在 `pypdf`，DOCX 使用 Python 标准库 OOXML 解析 |
| `cargo tauri --version` 初次不可用 | 1 | 通过 `cargo install tauri-cli --version 2.11.2 --locked` 安装全局 `cargo-tauri`，现 `cargo tauri` 与 npm 版 `tauri` 均可用 |

## Notes
- 每完成一个阶段或发现关键事实，需要更新本文件。
- 所有错误需记录，避免重复失败。

## E8-10 Final Acceptance Checklist

### A. Runtime Gate and Publish Safety
- [x] 在未设置 `EPIC8_UNIFIED_HTML_PATH`/`EPIC8_UNIFIED_PYTHON` 时执行导出，预期：Rust 静态合同 gate 可独立通过（`runtime.mode == static-rust`），真实 E2E 不作为生产硬依赖。
- [x] 设置有效 unified runtime 环境后执行 RuntimePreview E2E，预期：诊断报告可达到 `runtime.mode == real`、正确答案 100%、错误样本降分，但不影响普通生产导出资格。
- [x] 组 Pack 发布在同样条件下验证 Rust 静态 gate，预期：无真实 runtime 环境仍可发布，前提是静态合同与人工审核门禁通过。
- [x] 导出/Pack 前执行发布 readiness gate，阻断 `NeedsReview`、未人工确认、空答案、低置信未确认和未解决 parser warning。

### B. Authoring to Output Contract
- [x] `ReadingAuthoringIRV1` -> `ReadingExamSourceV1` 字段完整性检查通过（`answerKey`/`questionOrder`/`questionDisplayMap`/`questionGroups`）。
- [x] DOM 协议校验通过（input/select/textarea/dropzone 可采集，题号映射正确）。
- [x] RuntimePreview 诊断命令可验证正确答案自动填充得分为 100%，错误答案样本得分低于正确答案；生产硬门禁使用 Rust 静态合同校验。

### C. Parser and Import Reliability
- [x] TXT/MD/PDF/DOCX 导入解析均可产出 `DocumentIRV1`。
- [x] PDF/DOCX 复杂格式样例至少各 1 个，parser -> split -> AuthoringIR -> answerKey fixture 回归通过。
- [x] no-text PDF fixture 经 Python parser 产出 parser warning + low-confidence block，并由 `SourceReviewV1` 阻断发布；仍需真实扫描图片 OCR adapter 策略。
- [x] parser sidecar 失败不再生成 sample 题内容，而是生成 failure Document IR 并强制人工审核。
- [x] 导入、导出路径权限边界符合设计：仅 app data + 用户显式选择路径。

### D. LLM Safety and Review
- [x] API Key 存储策略可验证：默认使用跨平台 OS secure storage（macOS Keychain、Windows Credential Manager、系统 keyring），明文文件兜底默认禁用且仅 dev/emergency 显式开启。
- [x] 低置信度建议不可直接 apply（需人工复核）；高置信度建议可按白名单 patch 应用。
- [x] LLM 输出为结构化 JSON 建议，不可绕过模板/校验/E2E 门禁。

### E. Build and Release Artifacts
- [x] `npm run check` 通过。
- [x] `cargo check`（`src-tauri`）通过。
- [x] `cargo clippy --all-targets -- -D warnings`（`src-tauri`）通过。
- [x] `npm run tauri build` 通过并产出 `.app`/`.dmg`。

### F. Tracking Consistency
- [x] `Plan With Files/task_plan.md` 状态与实际实现一致：主链路可运行，Rust 静态 gate、视觉 LLM 策略、复杂 TXT/MD/PDF/DOCX、命令级 export/Pack fixture 已补；模块拆分和更广 UI E2E 仍未完成。
- [x] `Plan With Files/findings.md` 包含关键设计决策与风险。
- [x] `Plan With Files/progress.md` 记录执行结果、错误与修复。

## Deep Audit Findings: 2026-05-31

| ID | Severity | Area | Finding | Required Follow-up |
|----|----------|------|---------|--------------------|
| AUD-01 | P0 | Publish gate | `run_auto_pipeline` 会把 parser warning / low confidence 路由到人工审核，但 `export_reading_assets` / `build_pack` 只看四层 runtime gate，不回查 parser warning、low-confidence blocks、`NeedsReview` 或人工确认字段 | 在导出/Pack 前统一执行 `publish_readiness_gate` |
| AUD-02 | P0 fixed | Parser fallback | Python parser 失败时 Rust 对 PDF/DOCX 曾退到高置信 sample Document IR | 已改为 failure Document IR，仍需 fixture 回归 |
| AUD-03 | P0 | Human verification | `verified` 和 `audit.humanVerified` 没有被导出门禁强制检查；`update_authoring_ir` 可把 needsReview 清零 | AuthoringIR 校验必须要求答案/低置信字段人工确认后才允许发布 |
| AUD-04 | P1 | OCR | `rerun_ocr` 只是以 `mode=ocr` 重新跑同一 pypdf 解析；没有真实 OCR adapter | 增加 OCR adapter 或明确 no-text PDF 只能人工录入并阻断发布 |
| AUD-05 | P1 | Architecture | 核心 backend 超过 3600 行集中在 `src-tauri/src/lib.rs`，状态机、parser、LLM、validator、export、pack 混在一起 | 拆分为 storage/parser/pipeline/llm/validator/exporter/pack modules |
| AUD-06 | P1 | Test coverage | 仓库没有业务自动化测试/fixture；当前验证主要是 build、syntax、browser smoke | 建立 Rust unit/integration tests、sidecar fixtures、真实 runtime smoke |

## Audit Update: 2026-05-31 13:38 CST

| ID | Severity | Status | Summary | Follow-up |
|----|----------|--------|---------|-----------|
| AUD-07 | P0 | open | Parser warnings / low-confidence blocks can be bypassed when authoring `humanVerified` becomes true and job status is advanced out of `NeedsReview`. | Implement source/parser review provenance independent of question verification. |
| AUD-08 | P0 | open | `update_job_meta` can mutate `status` and `currentStep` directly. | Remove or guard status transitions. |
| AUD-09 | P1 | open | Production commands still have sample Document IR fallback for missing/unsupported source states. | Replace with explicit failure/review IR or dev-only sample command. |
| AUD-10 | P1 | open | High-confidence LLM auto-apply marks fields as verified, conflating model confidence with human confirmation. | Add separate auto-applied vs human-verified fields. |
| AUD-11 | P1 | partially improved | OCR remains a placeholder rerun of parser; no-text PDF fixture now exists and proves manual-review hard stop. | Decide OCR adapter vs explicit manual transcription flow; add scanned-image OCR fixture if OCR is in scope. |
| AUD-12 | P1 | fixed | Output source metadata/audit now derives `pdfFilename`, `shuiPdf`, source id/hash and match status from AuthoringIR provenance. | Add command-level export fixture to inspect final files. |
| AUD-13 | P1 | open | Rust backend architecture is still a single large file. | Split storage/parser/pipeline/llm/validator/runtime/export/pack modules. |
| AUD-14 | P1 | partially improved | Fixture/integration coverage now includes no-text PDF and minimal real runtime E2E, but remains insufficient for complex PDF/DOCX and export/Pack. | Add parser/pipeline/runtime/export/Pack fixtures. |

### Immediate Next Implementation Order After Audit
1. Fix P0 readiness provenance: parser warnings and low-confidence blocks must remain blocking until an explicit source-review action records resolution.
2. Lock workflow transitions: remove public arbitrary `status`/`currentStep` patching or enforce a legal transition/revalidation guard.
3. Stop production sample fallback: no command should create demo content for a real job when uploaded source is missing or unsupported.
4. Split LLM auto-apply from human verification: high confidence may apply safe structural patch, but cannot set `humanVerified`.
5. Add scanned/no-text PDF and real unified runtime fixtures before claiming Epic 8 completion.

## Implementation Update: 2026-05-31 14:04 CST

| ID | Severity | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| AUD-07 | P0 | fixed | Added independent `SourceReviewV1`; parser warnings / low-confidence blocks now block publish until explicit `resolve_source_review`, regardless of `audit.humanVerified`. | Rust tests pass; `publish_readiness_gate` uses `source_review_issues`. |
| AUD-08 | P0 | fixed | Removed public `status` / `currentStep` mutation from metadata patch in Rust and TypeScript. | `JobMetaPatch` no longer exposes workflow state fields; `npm run check` passed. |
| AUD-09 | P1 | fixed for production commands | Missing/unsupported source now creates low-confidence source-missing IR instead of sample content. | `missing_source_document_ir_never_uses_sample_content` test passed. |
| AUD-10 | P1 | fixed | LLM high-confidence auto-apply no longer marks fields as human verified; it records `autoApplied`. | `auto_applied_llm_patch_does_not_create_human_verification` test passed. |
| AUD-11 | P1 | partially improved | Added real no-text PDF fixture proving parser warning + low-confidence source review; OCR remains manual-review fallback, not true OCR adapter. | Decide OCR adapter vs manual transcription path; add scanned-image fixture if OCR is in scope. |
| AUD-13 | P1 | still open | Rust backend remains monolithic. | Needs module split. |
| AUD-14 | P1 | partially improved | Rust tests increased to 13; added no-text PDF, complex PDF/DOCX fixtures, real unified runtime minimal E2E, and command-level export/Pack real-runtime core tests. | Broader end-to-end pipeline and OCR fixtures still needed. |


## Implementation Update: 2026-05-31 14:30 CST

| ID | Severity | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| AUD-11 | P1 | partially improved | Added `fixtures/parser/no-text.pdf`; Python parser returns no-text warning and `confidence=0.2`, and Rust source-review/publish-gate tests confirm unresolved review issues block publish. | `no_text_pdf_fixture_requires_source_review` and `publish_gate_blocks_no_text_pdf_until_source_review_resolved` passed; manual parser smoke produced `page 1 has no extractable text`. |
| AUD-12 | P1 | fixed | `ReadingExamSourceV1` now derives `meta.pdfFilename`, `sourceRefs.shuiPdf`, `audit.matchStatus`, source id and hash notes from AuthoringIR source provenance instead of hard-coded `source.pdf`/always-verified. | `reading_source_uses_real_source_metadata_and_review_status` passed; TS types updated. |
| Runtime E2E | P1 | improved | Fixed preview E2E so real-runtime structured failures are not hidden by simulator fallback; fixed radio wrong-answer sample. External unified runtime minimal fixture now passes with `runtime.mode=real`. | `/tmp/epic8-real-runtime-report.json`: passed true, mode real, correct score 100%, wrong sample 50%. |
| E8-12 | P1 | still open | Fixture baseline now covers no-text PDF, complex PDF/DOCX, source metadata, export/Pack real-runtime core; Rust backend is still monolithic. | `src-tauri/src/lib.rs` remains single large backend file. |


## Implementation Update: 2026-05-31 14:45 CST

| ID | Severity | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| Export/Pack gate | P1 | improved | Extracted `export_reading_assets_core` and `build_pack_core` so Tauri commands and Rust tests share the same publish path. Originally added real-runtime command-level tests; current tests now validate the Rust static gate export/Pack path. | `cargo test` now passes static gate command tests including `export_core_writes_assets_after_static_runtime_gate` and `build_pack_core_writes_zip_after_static_runtime_gate`. |
| Runtime report merge | P1 | fixed | `merge_sidecar_validation` now preserves `runtime` from preview E2E reports; strict gate no longer sees `runtime.mode=unknown` after real runtime passes. | Initial export/Pack tests failed with `runtime.mode=unknown`; after fix, both passed. |


## Implementation Update: 2026-05-31 15:00 CST

| ID | Severity | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| Complex PDF/DOCX fixtures | P1 | fixed for minimal clear-layout cases | Added `fixtures/parser/complex-reading.pdf` and `fixtures/parser/complex-reading.docx`. Both parse through the Python sidecar, produce no parser warnings/low-confidence blocks, split into at least two groups, and generate AuthoringIR with 5 questions and expected answers. | `complex_text_pdf_fixture_reaches_authoring_ir` and `complex_docx_fixture_reaches_authoring_ir` passed. |
| AUD-14 | P1 | improved | Rust tests increased to 13, covering parser failure, no-text PDF hard stop, complex PDF/DOCX parser pipeline, source metadata, real-runtime export, and Pack. | `cargo test` with external runtime env passed 13 tests. |

## Audit Update: 2026-05-31 15:40 CST

| ID | Severity | Status | Summary | Follow-up |
|----|----------|--------|---------|-----------|
| AUD-16 | P1 | open | Rust backend is still monolithic and core domain objects are mostly `serde_json::Value`, limiting compile-time protection for field contracts. | Split modules and introduce typed domain models. |
| AUD-17 | P1 | improved | Local OCR is not bundled; no-text/image PDFs use vision LLM transcription when images are extractable, with manual transcription fallback and source-review gate. | Future rendered-page adapter may be needed for scans without extractable images. |
| AUD-18 | P1 | improved | Environment preflight now exposes Node/Python/pypdf/sidecar/unified-runtime readiness, but packaged app still depends on host runtimes. | Bundle dependencies or add production setup/installer. |
| AUD-19 | P1 | fixed for default path | Plaintext API-key file fallback is disabled by default and requires `EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK=1`; cross-platform OS secure storage is the production default via `keyring`. | Smoke-test Windows Credential Manager on a Windows build machine before cross-platform release. |
| AUD-20 | P1 | open | Source-review unresolved jobs can still advance through split/edit intermediate states, though publish remains blocked. | Preserve clearer `NeedsReview` semantics or add explicit “continue despite unresolved source review” UX. |
| AUD-21 | P1 | improved | Generated HTML preview is isolated in sandbox iframes and Tauri CSP is explicit instead of `null`. | Recheck CSP when integrating future real visual runtime assets. |
| AUD-22 | P1 | fixed for backend auto-apply | Rust auto-apply now validates patch whitelist, kind/interaction/question schema, non-fallback evidence, source block IDs, and quotes. | Typed domain models still needed for maintainability. |
| AUD-23 | P2 | fixed | Backend validates external filesystem-affecting segments including `jobId`, `packId`, `profileId`, and `examId`; regression tests reject traversal/nested/empty/space IDs. | Keep using validators when new filesystem-backed IDs are introduced. |
| AUD-24 | P2 | open | Real-runtime Rust tests skip when env vars are missing, which can hide coverage gaps in CI. | Make skipped coverage explicit in CI/reporting. |
| AUD-25 | P2 | fixed | Sidecar README documents no-sample parser behavior, vision LLM transcription, preflight, and secret fallback policy. | Keep docs in sync with packaging changes. |

## Audit Update: 2026-05-31 15:18 CST

| ID | Severity | Status | Summary | Follow-up |
|----|----------|--------|---------|-----------|
| AUD-26 | P0 | fixed | LLM API keys are passed via `EPIC8_LLM_API_KEY` and redacted from cached gateway inputs. | Continue secret-scanning cache artifacts during release audits. |
| AUD-27 | P1 | improved | AnswerKey source files are parsed and merged into split answer candidates in manual and auto pipeline paths. | More answer-file layouts and OCR/image answer keys need future coverage. |
| AUD-28 | P1 | improved | Split/answer UI can edit group heading/range/kind/block IDs/instructions and answer values before AuthoringIR generation. | Not yet a drag/select visual PDF block editor. |
| AUD-29 | P1 | clarified | Visible preview is labeled as sandboxed local template preview; real runtime pass/fail is shown through `runtime.mode` and strict E2E gates. | Replace with actual unified-runtime visual rendering if visual parity is required. |
| AUD-30 | P1 | fixed | Pack status updates happen only after validation, zip creation, and artifact writes. | Future transaction/rollback would further improve atomicity. |
| AUD-31 | P1 | fixed | Failed runtime validation downgrades stale ready statuses and refreshes issue counts. | Keep broader UI E2E coverage. |
| AUD-32 | P1 | fixed | Source-review fingerprint includes parser/source and low-confidence block text hash inputs. | Extend if new source-review dimensions are added. |
| AUD-33 | P1 | fixed | High-confidence suggestions without source-block evidence are saved for review but cannot auto-apply. | Continue provider-specific schema hardening. |
| AUD-34 | P2 | fixed | Authoring validation checks numeric continuity and duplicate display numbers. | Broader IELTS numbering edge cases can be added as fixtures. |
| AUD-35 | P2 | fixed | Validator-unavailable issues are merged through layer/pass recomputation. | None known. |
| AUD-36 | P2 | open | Settings lists providers that gateway does not actually implement. | Implement adapters or mark unsupported providers. |
| AUD-37 | P2 | open | Low-confidence LLM suggestions have no explicit human-approved apply path. | Add manual apply with approval provenance. |
| AUD-38 | P2 | open | Sidecar README still describes stale sample fallback behavior. | Update documentation. |

### Revised Immediate Implementation Order
1. Fix AUD-26 secret persistence before using any real API key in normal development.
2. Fix AUD-31 and AUD-30 state consistency issues around failed E2E and Pack atomicity.
3. Implement AUD-28/AUD-27 manual correction and answer-file merge so PDF edge cases can be handled without raw JSON edits.
4. Decide OCR policy: real OCR/layout adapter vs explicit manual transcription workflow.
5. Continue E8-12 module split and typed domain models to reduce regression risk.

## Implementation Update: 2026-05-31 15:59 CST

| ID | Severity | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| AUD-26 | P0 | fixed | LLM gateway inputs cached under `jobs/<job>/cache/llm` are now redacted; API keys are passed to the Node sidecar via `EPIC8_LLM_API_KEY` rather than serialized JSON. | `llm_cache_input_redacts_api_key` and `make_llm_input_never_contains_api_key` passed. |
| AUD-30 | P1 | fixed | Pack build now validates all jobs, prepares entries, writes zip and Pack artifacts, then transitions jobs through `Exported` and cleanup; status update no longer happens before artifact creation. | `build_pack_core_writes_zip_after_static_runtime_gate` now asserts zip, pack.json, manifest.js, and exam JS exist before cleanup. |
| AUD-31 | P1 | fixed | Preview/E2E state application now updates failed reports to `NeedsReview` and refreshes issue counts, preventing stale `DraftSaved`/`ExportReady`. | `failed_runtime_validation_downgrades_stale_export_ready_status` passed. |
| AUD-32 | P1 | fixed | `SourceReviewV1` fingerprint now includes parser provider/mode/source identifiers and low-confidence block content hashes. | `source_review_fingerprint_changes_when_low_confidence_text_changes` passed. |
| AUD-34 | P2 | fixed | Authoring validation now flags duplicate question IDs, numeric question gaps, empty display numbers, and duplicate display numbers. | `validate_authoring_blocks_duplicate_display_numbers_and_gaps` passed. |
| AUD-35 | P2 | fixed | `validate_authoring_ir` now uses `merge_validation_issues` after adding Node-validator-unavailable warnings so `passed/layers/issues` stay consistent. | `cargo test`, `cargo clippy`, `npm run check` passed. |
| AUD-36 | P2 | fixed for UI | Settings no longer offers unsupported `AnthropicCompatible`/`Ollama`/`Custom` providers; UI states only OpenAI-compatible is implemented. | `npm run check` and `npm run build` passed. |
| AUD-38 | P2 | fixed | Sidecar README no longer claims production parser can fall back to review-required sample IR. | README updated; `git diff --check` passed. |

### Current Remaining Implementation Order
1. Implement editable split/answer repair and optional answer-file merge (`AUD-27`, `AUD-28`) so complex PDFs can be corrected without raw JSON edits.
2. Decide OCR policy: real OCR/layout adapter vs explicit manual transcription workflow (`AUD-17`).
3. Add LLM suggestion JSON schema/evidence checks before high-confidence auto-apply (`AUD-22`, `AUD-33`).
4. Harden packaged runtime dependencies and secret fallback storage (`AUD-18`, `AUD-19`).
5. Split Rust backend into typed modules and add safe path segment validators (`AUD-16`, `AUD-23`).

## Implementation Update: 2026-05-31 16:17 CST

| ID | Severity | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| AUD-27 | P1 | improved | `AnswerKey` source files now parse independently and merge into `answerKeyCandidates` during rule split and auto pipeline; missing answer key issue is removed when external answers are recovered. | `answer_key_source_candidates_merge_into_split_answers` passed; `cargo test` now passes 19 tests. |
| AUD-28 | P1 | improved | `SplitAndAnswers` now supports editing group heading/range/kind/block IDs/instruction and answer values, plus save through `save_split_adjustments` before AuthoringIR generation. | `npm run check` and `npm run build` passed. |

### Current Remaining Implementation Order
1. Decide OCR/manual-transcription policy and implement the chosen path (`AUD-17`).
2. Add LLM suggestion JSON schema/evidence checks before high-confidence auto-apply (`AUD-22`, `AUD-33`).
3. Harden packaged runtime dependencies and secret fallback storage (`AUD-18`, `AUD-19`).
4. Split Rust backend into typed modules and add safe path segment validators (`AUD-16`, `AUD-23`).
5. Upgrade visual preview from simplified template iframe to real unified-runtime preview or explicitly label the limitation (`AUD-29`).

## Implementation Update: 2026-05-31 16:37 CST

| ID | Severity | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| AUD-17 | P1 | improved via manual transcription | Added an explicit manual transcription path for scanned/no-text PDFs: users can paste verified text, backend creates `DocumentIRV1` with `parser.provider=manual-transcription`, and the normal split/answer pipeline can continue. This is not a true OCR adapter. | `manual_transcription_document_ir_reaches_split_answers` passed; `cargo test` now passes 20 tests. |

### Current Remaining Implementation Order
1. Add LLM suggestion JSON schema/evidence checks before high-confidence auto-apply (`AUD-22`, `AUD-33`).
2. Harden packaged runtime dependencies and secret fallback storage (`AUD-18`, `AUD-19`).
3. Split Rust backend into typed modules and add safe path segment validators (`AUD-16`, `AUD-23`).
4. Upgrade visual preview from simplified template iframe to real unified-runtime preview or explicitly label the limitation (`AUD-29`).
5. Decide whether a real OCR/layout adapter is still in scope beyond the implemented manual transcription fallback (`AUD-17`).

## Implementation Update: 2026-05-31 16:58 CST

| ID | Severity | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| AUD-17 | P1 | improved via vision LLM transcription | Image-only/no-text PDFs now have an automatic first-line path: Python extracts embedded PDF images, the LLM gateway sends them to an OpenAI-compatible vision model, and Rust converts non-empty transcription into `DocumentIRV1` with `parser.provider=vision-llm-transcription`. Publish remains blocked by `SourceReviewV1` until human verification. | `image_only_pdf_fixture_exposes_embedded_images_for_vision` and `vision_transcription_document_ir_requires_source_review_and_reaches_split` passed; full verification passed. |
| OCR policy | P1 | decided for current product direction | Do not bundle heavyweight local OCR by default. Use vision LLM transcription for image-only PDFs when page images can be extracted, with manual transcription fallback when model/image extraction fails. | Sidecar README updated; DocumentReview exposes both vision LLM and manual transcription actions. |

### Current Remaining Implementation Order
1. Add LLM suggestion JSON schema/evidence checks before high-confidence auto-apply (`AUD-22`, `AUD-33`).
2. Harden packaged runtime dependencies and secret fallback storage (`AUD-18`, `AUD-19`).
3. Split Rust backend into typed modules and add safe path segment validators (`AUD-16`, `AUD-23`).
4. Upgrade visual preview from simplified template iframe to real unified-runtime preview or explicitly label the limitation (`AUD-29`).
5. Improve vision/PDF coverage for rendered-page scans where pypdf cannot expose embedded images; current fallback is manual transcription.

## Implementation Update: 2026-05-31 17:16 CST

| ID | Severity | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| AUD-22 | P1 | fixed for backend auto-apply path | Added Rust-side high-confidence LLM auto-apply validation: allowed patch paths, valid kind/interaction/question IDs, non-fallback provider evidence, `sourceBlockIds`, and evidence quotes tied to the current group source blocks. Manual apply and auto pipeline now share the same gate. | `high_confidence_llm_without_evidence_cannot_auto_apply` and `high_confidence_llm_with_source_block_evidence_can_auto_apply` passed; full checks passed. |
| AUD-33 | P1 | fixed for high-confidence auto-apply | High-confidence suggestions with missing/invalid source-block evidence are saved for review but not auto-applied; auto pipeline reports `blockedAutoApplyGroups` and routes to LLM Review. | `cargo test` passed 24 tests; `LlmReview` displays blocked-auto-apply guidance. |

### Current Remaining Implementation Order
1. Harden packaged runtime dependencies and secret fallback storage (`AUD-18`, `AUD-19`).
2. Split Rust backend into typed modules and add safe path segment validators (`AUD-16`, `AUD-23`).
3. Upgrade visual preview from simplified template iframe to real unified-runtime preview or explicitly label the limitation (`AUD-29`).
4. Improve vision/PDF coverage for rendered-page scans where pypdf cannot expose embedded images; current fallback is manual transcription.

## Implementation Update: 2026-05-31 17:34 CST

| ID | Severity | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| AUD-18 | P1 | improved via preflight | Added `EnvironmentPreflightV1` backend report and Settings UI to expose host/runtime readiness for Node.js, python3, pypdf, sidecar scripts, strict runtime gate, and unified runtime env vars. This does not yet bundle runtimes, but makes missing dependencies visible before import/export. | `environment_preflight_reports_required_dependency_names` passed; `npm run check` passed. |

### Current Remaining Implementation Order
1. Harden file-secret fallback storage (`AUD-19`).
2. Split Rust backend into typed modules and add safe path segment validators (`AUD-16`, `AUD-23`).
3. Upgrade visual preview from simplified template iframe to real unified-runtime preview or explicitly label the limitation (`AUD-29`).
4. Improve vision/PDF coverage for rendered-page scans where pypdf cannot expose embedded images; current fallback is manual transcription.
5. Consider bundling Node/Python/pypdf or providing a signed installer/setup flow; preflight only diagnoses, it does not self-contain dependencies.

## Implementation Update: 2026-05-31 17:55 CST

| ID | Severity | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| AUD-19 | P1 | fixed for default production path | Plaintext API-key file fallback is now disabled by default. OS secure storage is the default required path; plaintext app-data fallback only works when `EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK=1` is explicitly set for dev/emergency use. Existing plaintext files are ignored unless the opt-in is set. | `plaintext_secret_fallback_is_disabled_by_default` and `plaintext_secret_fallback_requires_explicit_opt_in` passed; full checks passed. |

### Current Remaining Implementation Order
1. Split Rust backend into typed modules (`AUD-16`).
2. Upgrade visual preview from simplified template iframe to real unified-runtime preview or explicitly label the limitation (`AUD-29`).
3. Improve vision/PDF coverage for rendered-page scans where pypdf cannot expose embedded images; current fallback is manual transcription.
4. Consider bundling Node/Python/pypdf or providing a signed installer/setup flow; preflight only diagnoses, it does not self-contain dependencies.


## Implementation Update: 2026-05-31 18:24 CST

| ID | Severity | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| AUD-23 | P1 | fixed for path/id segment class | Added Rust path-segment validation for external filesystem-affecting ids (`jobId`, `packId`, `profileId`, `examId`) and regression tests rejecting traversal/nested/empty/space-containing values. | `cargo test` passed 32 tests including unsafe path segment cases; full checks and Tauri release build passed. |
| AUD-21 | P1 | improved | Removed privileged-webview `dangerouslySetInnerHTML` preview, switched authoring/preview iframes to sandboxed documents, and replaced `csp: null` with explicit Tauri CSP. | `rg` shows no `dangerouslySetInnerHTML`; `npm run check`, `npm run build`, and `npm run tauri build` passed. |

### Current Remaining Implementation Order
1. Split Rust backend into typed modules (`AUD-16`).
2. Replace visible template preview with actual unified-runtime visual preview if visual parity is required; current UI now clearly labels the limitation (`AUD-29`).
3. Improve vision/PDF coverage for rendered-page scans where pypdf cannot expose embedded images; current fallback is manual transcription.
4. Consider bundling Node/Python/pypdf or providing a signed installer/setup flow; preflight only diagnoses, it does not self-contain dependencies.


## Implementation Update: 2026-05-31 18:31 CST

| ID | Severity | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| AUD-29 | P1 | clarified | UnifiedPreview now labels the visible iframe as a sandboxed local template preview and displays `runtime.mode`; strict export/Pack remains gated by real unified runtime E2E evidence. | `npm run check`, `npm run build`, `cargo test`, `cargo clippy`, sidecar syntax checks, and `git diff --check` passed. |

### Current Remaining Implementation Order
1. Split Rust backend into typed modules (`AUD-16`).
2. Improve vision/PDF coverage for rendered-page scans where pypdf cannot expose embedded images; current fallback is manual transcription.
3. Consider bundling Node/Python/pypdf or providing a signed installer/setup flow; preflight only diagnoses, it does not self-contain dependencies.
4. If product requires visual parity, replace the template preview iframe with actual unified-runtime visual rendering.

## Latest Requirements Addendum: 2026-05-31 18:34 CST

This addendum is the latest execution guidance for future agents. It supersedes earlier assumptions about SQLite-first storage, indefinite job-folder retention, heavyweight local OCR, and immediate Rust module splitting.

### Product Decisions
| Decision | Status | Execution Guidance |
|----------|--------|--------------------|
| MVP does not introduce SQL | decided | Keep file-based app-data persistence. SQLite is reserved for future question-bank indexing/search. |
| Long-term retained artifact is the editable draft | decided | Preserve `authoring-project.json` or equivalent `ReadingAuthoringIRV1` plus minimal metadata/review/export summaries. |
| Original uploads are temporary | decided | Keep only while needed for Working/NeedsReview/DraftSaved; delete by default after successful export/Pack cleanup. |
| Export triggers automatic cleanup | decided | Do not expose cleanup as a primary user workflow. Show an informational notice after cleanup. |
| Developer debug retention is optional | decided | Add Settings -> Developer/Diagnostics entry. `keep full process artifacts` defaults off. |
| Local heavyweight OCR is not bundled by default | decided | Text PDFs use deterministic parser; image PDFs use vision LLM transcription plus human source review; manual transcription remains fallback. |
| Rust backend module split is deferred | decided | `src-tauri/src/lib.rs` may remain monolithic during MVP stabilization. Split after production-level flow and dependencies are settled. |

### New/Adjusted Work Items
| ID | Task | Priority | Status | Acceptance |
|----|------|----------|--------|------------|
| E8-14 | Implement task lifecycle states `Working -> NeedsReview -> DraftSaved -> ExportReady -> Exported -> Cleaned` in product semantics | P0 | complete | Rust/TS status model now serializes lifecycle states; old statuses deserialize via aliases; UI labels/dashboard/Pack filters use lifecycle semantics. |
| E8-15 | Add post-export automatic cleanup | P0 | complete | Export/Pack write `authoring-project.json`, export/cleanup summaries, delete transient artifacts by default, and keep editable `authoring-ir.json`. |
| E8-16 | Add developer diagnostics retention option | P1 | complete | Settings exposes default-off `keepFullProcessArtifacts`; when enabled cleanup retains process artifacts and reports retention. |
| E8-17 | Evaluate replacing host Python PDF text extraction with lightweight Rust parser | P1 | complete | `pdf-extract` is now the primary clear-text PDF parser; complex text PDF and no-text PDF fixtures pass through `rust-parser:pdf:pdf-extract`; Python/pypdf remains only for DOCX sidecar fallback, legacy parser fallback, and embedded image extraction for vision transcription. |
| E8-18 | Add optional rendered-page adapter for scanned PDFs if vision coverage requires it | P2 | complete for macOS fallback | Python parser sidecar now falls back from embedded PDF image extraction to a macOS `sips` rendered PNG when no embedded images are exposed; the image is only input for vision LLM and still requires SourceReview before publish. Full cross-platform PDFium adapter remains a future enhancement if needed. |
| E8-19 | Document SQL as future index layer only | P1 | complete in docs | SQL scope excludes raw uploads, caches, raw LLM logs, and large process JSON. |

### Current Execution Order
1. Keep vision LLM + source review as the scanned/image PDF strategy.
2. Do not bundle Node/Python/OCR into production by default; keep Node for diagnostics and Python/pypdf for legacy/embedded-image optional paths only.
3. Evaluate whether a full cross-platform PDFium rendered-page adapter is needed beyond the current macOS `sips` fallback.
4. Defer Rust module split until product flow and dependency decisions are stable.


## Implementation Update: 2026-05-31 19:06 CST

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-14 | P0 | complete | Replaced externally visible job statuses with product lifecycle states `Working`, `NeedsReview`, `DraftSaved`, `ExportReady`, `Exported`, `Cleaned`; legacy status JSON aliases are accepted for existing jobs. | `npm run check`, Rust tests, and Tauri build passed. |
| E8-15 | P0 | complete | Successful JS export and Pack build now write `authoring-project.json`, export summary, cleanup summary, retain editable `authoring-ir.json`, and delete transient uploads/cache/preview/DocumentIR/Split/pipeline/LLM/transcription artifacts by default. | `export_core_writes_assets_after_static_runtime_gate` and `build_pack_core_writes_zip_after_static_runtime_gate` assert `Cleaned` state and cleanup artifacts; Rust tests pass 51 tests. |
| E8-16 | P1 | complete | Added default-off Developer/Diagnostics setting `keepFullProcessArtifacts`; when enabled, cleanup reports retention and leaves process files intact. | `cleanup_respects_diagnostics_artifact_retention` passed; Settings UI and dev fallback support the setting. |

### Current Remaining Implementation Order
1. Do not bundle Node/Python/OCR into production by default; keep Node for diagnostics and Python/pypdf for legacy/embedded-image optional paths only.
2. Evaluate whether a full cross-platform PDFium rendered-page adapter is needed beyond the current macOS `sips` fallback.
3. Defer Rust module split until lifecycle, cleanup, and dependency strategy stabilize.

## Implementation Update: 2026-05-31 19:52 CST

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-17 | P1 | complete | Added `pdf-extract = "0.10"` and made Rust PDF text-layer extraction the primary parser for clear text PDFs. `parse_source_document` now uses provider `rust-parser:pdf:pdf-extract` for PDF text extraction and falls back to the Python parser only if the Rust extractor errors. No-text PDFs still produce low-confidence blocks and parser warnings, preserving the vision LLM/manual review flow. | `complex_text_pdf_fixture_reaches_authoring_ir` and `no_text_pdf_fixture_requires_source_review` pass with Rust provider; `cargo test` passed 33 tests; `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, sidecar syntax checks, `git diff --check`, `npm run check`, `npm run build`, and `npm run tauri build` passed. |

### Current Remaining Implementation Order
1. Keep production dependency strategy Rust-first: no bundled Node/Python/OCR; Node remains diagnostic, Python/pypdf remains legacy/embedded-image optional, macOS `sips` handles rendered-page fallback.
2. Evaluate whether a full cross-platform PDFium rendered-page adapter is needed beyond the current macOS `sips` fallback.
3. If visual parity becomes a product requirement, replace the current local-template preview iframe with actual unified-runtime rendering or embedded WebView/JS diagnostics without requiring host Node.
4. Defer Rust module split until dependency and flow decisions stabilize.

## Implementation Update: 2026-05-31 20:11 CST

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-18 | P2 | complete for macOS fallback | Added a lightweight rendered-page fallback without bundling local OCR. `extract_pdf_images` still prefers embedded images via `pypdf`; if none are exposed, it uses macOS `sips` to render a PNG for vision LLM transcription and returns `renderedFallback: true` plus warnings. SourceReview remains mandatory because this is only a page-image input, not OCR or human verification. | `no_text_pdf_fixture_renders_page_fallback_for_vision` and `image_only_pdf_fixture_exposes_embedded_images_for_vision` passed; `cargo test` passed 34 tests; `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, sidecar syntax checks, `git diff --check`, `npm run check`, `npm run build`, and `npm run tauri build` passed. |

### Current Remaining Implementation Order
1. Keep production dependency strategy Rust-first: no bundled Node/Python/OCR; Node remains diagnostic, Python/pypdf remains legacy/embedded-image optional, macOS `sips` handles rendered-page fallback.
2. If cross-platform scan rendering is required, replace or supplement the macOS `sips` fallback with a PDFium rendered-page adapter.
3. If visual parity becomes a product requirement, replace the current local-template preview iframe with actual unified-runtime rendering or embedded WebView/JS diagnostics without requiring host Node.
4. Defer Rust module split until dependency and flow decisions stabilize.

## Implementation Update: 2026-05-31 20:46 CST

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-20 | P1 | complete | Added a Rust DOCX OOXML primary parser using `zip` + `quick-xml`. `parse_source_document` now parses DOCX locally first with provider `rust-parser:docx:ooxml`; Python sidecar is only a fallback for DOCX parser errors and remains required for TXT/MD sidecar flow, PDF image extraction, macOS `sips` orchestration, and legacy parser fallback. | `complex_docx_fixture_reaches_authoring_ir` passed with Rust provider; full `cargo test` passed 34 tests; `cargo clippy --all-targets -- -D warnings`, `npm run check`, `npm run build`, sidecar syntax checks, `git diff --check`, and `npm run tauri build` passed. `cargo tree -p zip --depth 2` shows `flate2`/`miniz_oxide` without `zopfli`. |

### Current Remaining Implementation Order
1. Keep production dependency strategy Rust-first: no bundled Node/Python/OCR; Node remains diagnostic, Python/pypdf remains legacy/embedded-image optional, macOS `sips` handles rendered-page fallback.
2. If cross-platform scan rendering is required, replace or supplement the macOS `sips` fallback with a PDFium rendered-page adapter.
3. If visual parity becomes a product requirement, replace the current local-template preview iframe with actual unified-runtime rendering or embedded WebView/JS diagnostics without requiring host Node.
4. Defer Rust module split until dependency and flow decisions stabilize.

## Implementation Update: 2026-05-31 21:02 CST

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-21 | P1 | complete | Moved the rendered-page fallback orchestration for image/no-text PDFs into Rust. Vision transcription now tries Python/pypdf embedded-image extraction first, but if that fails or returns no images, Rust directly invokes macOS `sips` and emits the same `PdfImageExtractionV1` contract for the LLM gateway. This reduces Python/pypdf from a hard dependency for scanned-PDF vision input on macOS while preserving SourceReview as the publish gate. | `no_text_pdf_fixture_renders_page_fallback_for_vision` passed through the unified Rust entrypoint; `rust_sips_fallback_renders_pdf_without_python_extraction` passed without using Python extraction; full `cargo test` passed 35 tests; `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `npm run check`, `npm run build`, sidecar syntax checks, `git diff --check`, and `npm run tauri build` passed. |

### Current Remaining Implementation Order
1. Keep production dependency strategy Rust-first: no bundled Node/Python/OCR; Node remains diagnostic, Python/pypdf remains legacy/embedded-image optional, macOS `sips` handles rendered-page fallback.
2. If cross-platform scan rendering is required, replace or supplement the macOS `sips` fallback with a PDFium rendered-page adapter.
3. If visual parity becomes a product requirement, replace the current local-template preview iframe with actual unified-runtime rendering or embedded WebView/JS diagnostics without requiring host Node.
4. Defer Rust module split until dependency and flow decisions stabilize.

## Implementation Update: 2026-05-31 21:19 CST

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-22 | P1 | complete | Made TXT/MD parsing Rust-primary. `parse_source_document` now routes TXT and Markdown through `rust-parser:text:plain` / `rust-parser:text:markdown` before any Python sidecar fallback, and preflight exposes `rust:text-parser`. Added TXT/MD fixtures proving both formats reach AuthoringIR and answer extraction through the Rust path. | `complex_txt_fixture_reaches_authoring_ir` and `complex_markdown_fixture_reaches_authoring_ir` passed; full `cargo test` passed 37 tests; `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `npm run check`, `npm run build`, sidecar syntax checks, `git diff --check`, and `npm run tauri build` passed. |

### Current Remaining Implementation Order
1. Keep production dependency strategy Rust-first: no bundled Node/Python/OCR; Node remains diagnostic, Python/pypdf remains legacy/embedded-image optional, macOS `sips` handles rendered-page fallback.
2. If cross-platform scan rendering is required, replace or supplement the macOS `sips` fallback with a PDFium rendered-page adapter.
3. If visual parity becomes a product requirement, replace the current local-template preview iframe with actual unified-runtime rendering or embedded WebView/JS diagnostics without requiring host Node.
4. Defer Rust module split until dependency and flow decisions stabilize.


## Implementation Update: 2026-05-31 22:49 CST

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-23 | P0 | complete | Added Rust built-in `ReadingExamSourceV1` and DOM protocol validation as the authoritative production contract gate. Warning-only reports remain passable; `severity=error` blocks. | `cargo test` passed 50 tests before the latest full rerun; validation tests cover answer key/order/display map/DOM controls/dropzones and warning semantics. |
| E8-24 | P0 | complete | Migrated production LLM gateway behavior to Rust OpenAI-compatible HTTP calls with JSON output validation for group classification/extraction and vision transcription. Node LLM sidecar is no longer on the production path. | `rust_llm_fallback_remains_low_confidence_and_non_auto_applicable` and `rust_openai_compatible_output_validation_adds_evidence_metadata` pass; `reqwest`/`base64` are in Cargo dependencies. |
| E8-25 | P1 | complete | Node validator is development parity diagnostics only, controlled by `EPIC8_NODE_VALIDATOR_DIAGNOSTICS=1`; missing Node cannot weaken production validation. | `validate_authoring_ir` uses Rust validation by default and only calls `run_node_validator_diagnostic` when the env flag is set. |
| E8-26 | P1 | complete | Preview E2E is explicit development/CI/diagnostic behavior only. Export/Pack use Rust static contract gate plus SourceReview/AuthoringReview. Diagnostic E2E failure is visible but no longer demotes static `ExportReady` status. | `export_core_writes_assets_after_static_runtime_gate`, `build_pack_core_writes_zip_after_static_runtime_gate`, and `preview_e2e_diagnostic_failure_does_not_block_static_export_ready` pass. |
| E8-27 | P1 | complete | Started backend module decomposition by extracting common IO/path/zip helpers into `src-tauri/src/util.rs`. | `cargo fmt --check`, `cargo test` 51 tests, `cargo clippy --all-targets -- -D warnings`, `npm run check`, and `git diff --check` passed. |
| E8-28 | P1 | complete | Continued backend module decomposition by extracting ReadingExamSourceV1/DOM contract validation into `src-tauri/src/validator.rs`. | `cargo fmt --check`, `cargo test` 51 tests, `cargo clippy --all-targets -- -D warnings`, `npm run check`, and `git diff --check` passed. |

### Current Remaining Implementation Order
1. Continue backend module decomposition beyond `util.rs` and `validator.rs`: parser, LLM, export/pack, and storage are the next logical seams.
2. Add broader UI E2E and real provider coverage for the Rust LLM gateway.
3. If cross-platform scan rendering is required, add an optional PDFium page-render adapter that only renders pages for vision LLM, not local OCR.

## Implementation Update: 2026-05-31 23:58 CST

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-29 | P1 | complete | Continued backend decomposition by extracting Rust LLM gateway transport/validation logic into `src-tauri/src/llm_gateway.rs`. Kept authoring orchestration and deterministic fallback in `lib.rs` to preserve business boundaries. | `cargo fmt --check`, `cargo test` 51 tests, `cargo clippy --all-targets -- -D warnings`, `npm run check`, and `git diff --check` passed. `lib.rs` reduced to about 7885 lines. |

### Current Remaining Implementation Order
1. Continue backend module decomposition with parser seams next: clear-text parser/DOCX/PDF extraction first, then source review and vision-image extraction.
2. Keep production dependency strategy aligned with the latest 2026-05-31 CST requirement: no bundled Node/Python/OCR hard dependency; Rust main path plus vision LLM for image PDFs.
3. Add live OpenAI-compatible provider coverage for the Rust LLM gateway when credentials/test endpoint are available.
4. If cross-platform scan rendering is required, add an optional PDFium page-render adapter that only renders pages for vision LLM, not local OCR.

## Implementation Update: 2026-06-01 00:20 CST

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-30 | P1 | complete | Extracted the parser/import stack into `src-tauri/src/parser.rs`, including Rust TXT/MD/PDF/DOCX parsing, Python legacy fallback, PDF image extraction, macOS `sips` rendered-page fallback, and DocumentIR failure/transcription builders. | `cargo fmt --check`, `cargo test` 51 tests, `cargo clippy --all-targets -- -D warnings`, `npm run check`, and `git diff --check` passed. `lib.rs` reduced to about 6922 lines. |

### Current Remaining Implementation Order
1. Continue architecture decomposition with export/Pack and storage/settings seams; avoid changing PDF production semantics unless tests and SourceReview gates are updated together.
2. Add live OpenAI-compatible provider coverage for the Rust LLM gateway when credentials/test endpoint are available.
3. Add broader UI E2E or embedded WebView diagnostic coverage without making host Node a production dependency.
4. If cross-platform scan rendering is required, add optional PDFium page-render adapter only for rendering page images for vision LLM, not OCR.

## Implementation Update: 2026-06-01 00:44 CST

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-31 | P1 | complete | Extracted LLM profile and API-key secret storage into `src-tauri/src/llm_profiles.rs`, preserving Keychain-first storage and opt-in plaintext fallback semantics. | `cargo fmt --check`, `cargo test` 51 tests, `cargo clippy --all-targets -- -D warnings`, `npm run check`, and `git diff --check` passed. `lib.rs` reduced to about 6673 lines. |

### Current Remaining Implementation Order
1. Split export/Pack cautiously by first extracting pure JS/manifest/pack artifact builders from side-effecting job-state and cleanup orchestration.
2. Keep production Rust-first dependency boundaries unchanged: no bundled Node/Python/OCR hard dependency; vision LLM remains the image-PDF OCR substitute plus SourceReview.
3. Add live OpenAI-compatible provider coverage for the Rust LLM gateway when credentials/test endpoint are available.
4. Add broader UI E2E or embedded WebView diagnostic coverage without making host Node a production dependency.

## Implementation Update: 2026-06-01 01:08 CST

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-32 | P1 | complete | Extracted pure ReadingExam/Pack artifact builders into `src-tauri/src/export_artifacts.rs` and reused a `ReadingAssetBundle` for preview/export asset generation. Side-effecting export, Pack, gate, job-state, and cleanup orchestration remains in `lib.rs`. | `cargo fmt --check`, `cargo test` 51 tests, `cargo clippy --all-targets -- -D warnings`, `npm run check`, and `git diff --check` passed. `lib.rs` reduced to about 6620 lines. |

### Current Remaining Implementation Order
1. Continue export/Pack split by extracting pure pack-entry assembly before moving side-effecting orchestration.
2. Preserve production gate semantics: Rust static contract gate + SourceReview/AuthoringReview; preview E2E remains diagnostic only.
3. Keep no-Node/no-Python/no-OCR production dependency boundary intact.
4. Add live OpenAI-compatible provider coverage for the Rust LLM gateway when credentials/test endpoint are available.

## Implementation Update: 2026-06-01

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-33 | P1 | complete | Extracted pure Pack entry assembly into `src-tauri/src/export_artifacts.rs`. `build_pack_core` now reads jobs, runs static/publish gates, writes ZIP/files, updates job state, and cleans up, while `build_pack_entry_bundle` owns Pack ZIP entries, `pack.json`, manifest JS, and per-exam wrapper assembly. Also normalized missing `examId` fallback so script filename, wrapper registration key, and Pack manifest stay consistent. | `cargo fmt --check`, `cargo test` 52 tests, `cargo clippy --all-targets -- -D warnings`, `npm run check`, and `git diff --check` passed. Added `pack_entry_bundle_normalizes_missing_exam_id_to_fallback`. `lib.rs` reduced to about 6603 lines. |

### Current Remaining Implementation Order
1. Continue backend architecture decomposition with side-effecting export/Pack orchestration only after the pure helpers remain covered by tests.
2. Keep production gates unchanged: Rust static contract validation plus SourceReview/AuthoringReview; preview E2E remains diagnostic/dev only.
3. Keep Node/Python/OCR outside production hard dependencies; vision LLM remains the image-PDF OCR substitute, with SourceReview mandatory.
4. Next safe seams: storage/settings command split, cleanup/export orchestration split, and typed `DocumentIRV1`/AuthoringIR structs to reduce `serde_json::Value` coupling.

## Implementation Update: 2026-06-01 / E8-34

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-34 | P2 | complete | Extracted diagnostics settings persistence into `src-tauri/src/diagnostics.rs`. `lib.rs` still owns cleanup behavior and command orchestration, while diagnostics storage now owns `DiagnosticsSettings`, default loading, and `config/diagnostics-settings.json` writes. | `cargo fmt --check`, `cargo test` 52 tests, `cargo clippy --all-targets -- -D warnings`, `npm run check`, and `git diff --check` passed. `lib.rs` reduced to about 6579 lines. |

### Current Remaining Implementation Order
1. Continue reducing `lib.rs` through low-risk orchestration seams: environment preflight, cleanup/export workflow, and job repository helpers.
2. Do not change PDF upload semantics unless adding tests for clear-text PDF, image/no-text PDF, vision transcription, SourceReview, and publish gate behavior in the same change.
3. Keep production package strategy unchanged: no bundled Node/Python/OCR hard dependency; Rust main path plus vision LLM for image PDFs.

## Implementation Update: 2026-06-01 / E8-35

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-35 | P2 | complete | Extracted environment/preflight infrastructure into `src-tauri/src/environment.rs`. The module now owns sidecar discovery, external command failure formatting, command probes, `EnvironmentPreflightV1`, unified runtime env resolution, static runtime strict-mode parsing, and optional Node validator diagnostic flag parsing. | `cargo fmt --check`, `cargo test` 52 tests, `cargo clippy --all-targets -- -D warnings`, `npm run check`, and `git diff --check` passed. `lib.rs` reduced to about 6210 lines. |

### Current Remaining Implementation Order
1. Continue reducing `lib.rs` through business workflow seams only when tests can cover state transitions: source review, job repository, cleanup/export lifecycle, and LLM orchestration.
2. Keep environment/preflight as diagnostics only; it must not reintroduce Node/Python/OCR as production hard dependencies.
3. Preserve PDF upload chain invariants: Rust clear-text parsing, vision LLM for image/no-text PDFs, SourceReview mandatory before publish, and no automatic human verification from LLM output.

## Implementation Update: 2026-06-01 / E8-36

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-36 | P0 | complete | Added real `Files/*.pdf` sample regression coverage for the upload -> Rust PDF parse -> split -> AuthoringIR review path. The opening P2 `Questions 14-26` instruction is now preserved as `umbrellaQuestionRanges`; it is not treated as a duplicate concrete group when later `14-19` / `20-23` / `24-26` groups exist. If only the umbrella range exists, the app creates a low-confidence manual-question-import scaffold instead of fabricating concrete prompts. Mixed text/image PDFs remain eligible for vision transcription and SourceReview, while fully text-layer-readable PDFs skip that source gate. | `files_pdf_samples_reach_expected_review_paths` passed against the four user-provided PDFs; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test)` passed, 53 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. |

### Current Remaining Implementation Order
1. Continue validating real PDF edge cases, especially answer keys or question pages embedded as images; keep vision LLM transcription mandatory for missing/image-only pages and keep SourceReview mandatory before publish.
2. Split stateful source-review or job-repository workflows only after adding tests around lifecycle transitions; `lib.rs` still owns these core business chains.
3. Preserve production dependency boundaries: no bundled Node/Python/OCR hard dependency; Rust clear-text parsing plus vision LLM for image PDFs.
4. Add live provider/diagnostic coverage for the Rust vision LLM path when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-37

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-37 | P1 | complete | Extracted SourceReview business logic into `src-tauri/src/source_review.rs` without changing the JSON contracts or review gates. The module now owns parser warning extraction, low-confidence block detection, SourceReview fingerprinting, `source-review.json` status persistence, and SourceReview publish-blocking issues. `lib.rs` keeps workflow orchestration and calls the module boundary. | `(cd src-tauri && cargo test source_review -- --nocapture)` passed, 5 targeted tests; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test)` passed, 53 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. `src-tauri/src/lib.rs` is about 6482 lines and `src-tauri/src/source_review.rs` is 184 lines. |

### Current Remaining Implementation Order
1. Continue splitting stateful workflow carefully: job repository helpers, cleanup/export lifecycle, and LLM orchestration are next candidates, but each needs lifecycle tests.
2. Keep SourceReview semantics stable: parser warnings and low-confidence blocks require manual review, vision transcription does not imply human verification, and publish readiness must include SourceReview issues.
3. Preserve no bundled Node/Python/OCR production dependency boundary; this split is architecture-only and does not change import or packaging behavior.

## Implementation Update: 2026-06-01 / E8-38

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-38 | P1 | complete | Extracted job persistence helpers into `src-tauri/src/job_store.rs`. The new module owns `make_job`, `load_job`, `save_job`, `update_job`, and filtered/sorted `list_saved_jobs`; Tauri commands keep only app-root resolution, directory setup, and workflow orchestration. No job status semantics, source review behavior, export lifecycle, or parser flow changed. | `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test)` passed, 53 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. `src-tauri/src/lib.rs` is about 6419 lines and `src-tauri/src/job_store.rs` is 76 lines. |

### Current Remaining Implementation Order
1. Continue splitting orchestration only where tests cover lifecycle transitions: cleanup/export lifecycle and LLM orchestration are next likely seams.
2. Keep job status aliases and state transitions stable while module boundaries improve.
3. Preserve PDF upload and review invariants: Rust clear-text parsing, vision LLM for image/no-text/mixed PDFs, SourceReview mandatory for low-confidence/missing pages, and no automatic human verification from LLM output.

## Implementation Update: 2026-06-01 / E8-39

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-39 | P1 | complete | Synced browser dev fallback with the production Rust semantics for opening `Questions 14-26` umbrella ranges. The fallback now preserves `umbrellaQuestionRanges`, avoids duplicate concrete Q14-Q26 groups when later concrete groups exist, creates low-confidence `requiresManualQuestionImport` scaffolds when only the umbrella is present, and blocks validation/readiness until manually completed. | `npm run check` passed; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` passed; `git diff --check` passed. |

### Current Remaining Implementation Order
1. Continue splitting orchestration only where tests cover lifecycle transitions: cleanup/export lifecycle and LLM orchestration remain next likely seams.
2. Preserve the umbrella range invariant: opening `Questions 14-26` is valid Passage-level range metadata, but publishable concrete interactions must come from later concrete groups or manual import.
3. Keep PDF upload and review invariants: Rust clear-text parsing, vision LLM for image/no-text/mixed PDFs, SourceReview mandatory for low-confidence/missing pages, and no automatic human verification from LLM output.
4. Add live provider/diagnostic coverage for the Rust vision LLM path when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-40

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-40 | P1 | complete | Extracted transient artifact cleanup mechanics into `src-tauri/src/cleanup.rs`. The new module handles diagnostics retention, transient directory/file deletion, and cleanup summary writing. `lib.rs` keeps AuthoringProject writing and job status transitions through closure injection, preserving export/Pack lifecycle behavior. | `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test cleanup -- --nocapture)` passed; export and Pack cleanup target tests passed; `(cd src-tauri && cargo test)` passed, 53 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. `src-tauri/src/lib.rs` is about 6381 lines and `src-tauri/src/cleanup.rs` is 65 lines. |

### Current Remaining Implementation Order
1. Continue splitting orchestration only where tests cover lifecycle transitions: LLM orchestration and export/Pack workflow are the next likely seams, but they are stateful and require stricter regression coverage.
2. Preserve the cleanup invariant: export/Pack should write AuthoringProject, remove only transient process files unless diagnostics retention is enabled, and keep editable `authoring-ir.json`.
3. Preserve PDF upload and review invariants: Rust clear-text parsing, vision LLM for image/no-text/mixed PDFs, SourceReview mandatory for low-confidence/missing pages, and no automatic human verification from LLM output.
4. Add live provider/diagnostic coverage for the Rust vision LLM path when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-41

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-41 | P1 | complete | Extracted LLM suggestion helpers into `src-tauri/src/llm_suggestions.rs`. The new module owns LLM input/output shaping, deterministic low-confidence fallback suggestions, suggestion persistence/loading, auto-apply safety checks, and selected patch application. `lib.rs` keeps profile/secret loading, provider invocation, audit/job state updates, and auto-pipeline orchestration. | `(cd src-tauri && cargo test llm -- --nocapture)` passed, 6 tests; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test)` passed, 53 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. `src-tauri/src/lib.rs` is about 5860 lines and `src-tauri/src/llm_suggestions.rs` is 546 lines. |

### Current Remaining Implementation Order
1. Continue splitting stateful orchestration only with broader tests: auto-pipeline and export/Pack workflows remain likely seams but require lifecycle regression coverage.
2. Preserve LLM safety invariants: no fallback auto-apply, no provider suggestion without source evidence/quotes, and no LLM-created human verification.
3. Preserve PDF upload and review invariants: Rust clear-text parsing, vision LLM for image/no-text/mixed PDFs, SourceReview mandatory for low-confidence/missing pages, and manual AuthoringReview before publish.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-42

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-42 | P1 | complete | Extracted pure AuthoringReview rules into `src-tauri/src/authoring_review.rs`. The module now owns empty-answer detection, low-confidence review counting, `audit.humanVerified` derivation, group verification refresh, and publish-blocking AuthoringIR issues. `lib.rs` keeps publish readiness orchestration with job status, SourceReview, and runtime/static validation. | Targeted AuthoringReview tests passed; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test)` passed, 53 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. `src-tauri/src/lib.rs` is about 5713 lines and `src-tauri/src/authoring_review.rs` is 152 lines. |

### Current Remaining Implementation Order
1. Continue extracting contract/export helpers only with matching runtime/export/Pack regression tests.
2. Preserve AuthoringReview invariants: low-confidence fields, empty answers, manual-question-import scaffolds, and missing human verification block publish.
3. Preserve PDF upload and review invariants: Rust clear-text parsing, vision LLM for image/no-text/mixed PDFs, SourceReview mandatory for low-confidence/missing pages, and manual AuthoringReview before publish.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-43

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-43 | P1 | complete | Extracted pure ReadingExamSource contract construction into `src-tauri/src/reading_source.rs`. The module now owns HTML escaping for question bodies, answer key projection, question order/display map derivation, and `ReadingExamSourceV1` assembly. `lib.rs` keeps validate/publish orchestration with SourceReview, AuthoringReview, runtime validation, and command handlers. | `(cd src-tauri && cargo test reading_source -- --nocapture)` passed; `(cd src-tauri && cargo test rust_contract_validator -- --nocapture)` passed, 10 tests; `(cd src-tauri && cargo test export_core_writes_assets_after_static_runtime_gate -- --nocapture)` passed; `(cd src-tauri && cargo test build_pack_core_writes_zip_after_static_runtime_gate -- --nocapture)` passed; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test)` passed, 53 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. `src-tauri/src/lib.rs` is about 5514 lines and `src-tauri/src/reading_source.rs` is 270 lines. |

### Current Remaining Implementation Order
1. Continue extracting contract-layer validation only with matching runtime/export/Pack regression tests.
2. Preserve the ReadingExamSource invariant: contract shape, field names, and author_verified vs needs_review audit semantics must remain stable.
3. Preserve PDF upload and review invariants: Rust clear-text parsing, vision LLM for image/no-text/mixed PDFs, SourceReview mandatory for low-confidence/missing pages, and manual AuthoringReview before publish.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-44

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-44 | P0 | complete | Hardened the `Questions 14-26` umbrella-range semantics after product clarification. Rust production split and browser dev fallback now recognize a broader set of opening Passage-level instructions, preserve them in `umbrellaQuestionRanges`, and still avoid misclassifying concrete question headings such as `Questions 14-19 Do the following statements agree with the information given in Reading Passage 2?`. | `(cd src-tauri && cargo test umbrella_question_range_detection_keeps_opening_instructions_distinct -- --nocapture)` passed; `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` passed; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test)` passed, 54 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. |

### Current Remaining Implementation Order
1. Preserve the umbrella range invariant: opening `Questions 14-26` is valid Passage-level range metadata and must be included, but concrete publishable interactions still come from later concrete groups or manual import.
2. Continue extracting validation/workflow seams only with runtime/export/Pack regression tests.
3. Preserve PDF upload and review invariants: Rust clear-text parsing, vision LLM for image/no-text/mixed PDFs, SourceReview mandatory for low-confidence/missing pages, and manual AuthoringReview before publish.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-45

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-45 | P1 | complete | Extracted pure AuthoringIR validation/report merging into `src-tauri/src/authoring_validation.rs`. The new module owns `validate_authoring`, sidecar validation report merging, and warning/error issue merging. `lib.rs` keeps runtime gate orchestration, SourceReview/AuthoringReview publish readiness, export/Pack state transitions, and command handlers. | `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test validate_authoring_blocks_duplicate_display_numbers_and_gaps -- --nocapture)` passed; `(cd src-tauri && cargo test validation_warning_does_not_block_runtime_gate_progress -- --nocapture)` passed; `(cd src-tauri && cargo test rust_contract_validator -- --nocapture)` passed, 10 tests; `(cd src-tauri && cargo test)` passed, 54 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. `src-tauri/src/lib.rs` is about 5355 lines and `src-tauri/src/authoring_validation.rs` is 206 lines. |

### Current Remaining Implementation Order
1. Keep publish readiness and export/Pack orchestration in `lib.rs` until there are broader command-level lifecycle tests for every transition.
2. Preserve AuthoringIR validation invariants: missing exam/group, contract validation, duplicate qid/display number, numeric continuity, warning-only diagnostics not blocking static gate progress.
3. Preserve PDF upload and review invariants: Rust clear-text parsing, vision LLM for image/no-text/mixed PDFs, SourceReview mandatory for low-confidence/missing pages, and manual AuthoringReview before publish.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-46

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-46 | P1 | complete | Added negative lifecycle regression coverage for export and Pack publish gates. If static contract validation passes but publish readiness fails because human verification is missing, export/Pack now have tests proving they return validation errors without writing final artifacts, without creating cleanup summaries/authoring projects, and without advancing jobs to `Exported`/`Cleaned`. | `(cd src-tauri && cargo test export_core_publish_gate_failure_writes_no_export_or_cleanup -- --nocapture)` passed; `(cd src-tauri && cargo test build_pack_publish_gate_failure_writes_no_pack_or_cleanup -- --nocapture)` passed; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test)` passed, 56 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. |

### Current Remaining Implementation Order
1. With positive and negative export/Pack lifecycle coverage in place, evaluate whether a small workflow module can own shared publish-gate helpers without moving command state transitions prematurely.
2. Preserve publish blocking invariants: missing SourceReview resolution, missing human verification, low-confidence/manual-import issues, and runtime contract errors must prevent export/Pack artifacts and cleanup.
3. Preserve PDF upload and review invariants: Rust clear-text parsing, vision LLM for image/no-text/mixed PDFs, SourceReview mandatory for low-confidence/missing pages, and manual AuthoringReview before publish.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-47

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-47 | P1 | complete | Fixed two command-level audit gaps. `validate_authoring_ir` now overwrites stale workflow steps based on validation and SourceReview outcome, keeping SourceReview issues in `DocumentReview`, validation failures in `Authoring`, and passing validation at `DraftSaved`/`Authoring`. `choose_export_dir` now opens a native Tauri folder picker instead of returning `None`. Browser dev fallback mirrors the validation step update. | `(cd src-tauri && cargo test validation_job_state_routes_review_and_authoring_steps -- --nocapture)` passed; `(cd src-tauri && cargo test validate_authoring_state_update_overwrites_stale_current_step -- --nocapture)` passed; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test)` passed, 58 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. |

### Current Remaining Implementation Order
1. Continue tightening command/UI lifecycle coverage around validation, preview, export, and Pack before moving stateful workflow code out of `lib.rs`.
2. Preserve SourceReview and AuthoringReview invariants: parser/vision uncertainty routes to DocumentReview, low-confidence/manual-import questions route to human AuthoringReview, and no LLM output creates human verification.
3. Preserve production dependency strategy: Rust-first parser/export/validation, optional Node/Python diagnostics/legacy fallback only, and vision LLM rather than bundled OCR for image PDFs.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-48

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-48 | P1 | complete | Extracted workflow lifecycle state transitions into `src-tauri/src/workflow_state.rs`. The module now owns preview-E2E job status updates, AuthoringIR validation job status updates, issue-count projection, and lifecycle regression tests. `lib.rs` keeps command orchestration and calls the module helpers. | `(cd src-tauri && cargo test workflow_state -- --nocapture)` passed, 4 tests; `(cd src-tauri && cargo test failed_runtime_validation_downgrades_stale_export_ready_status -- --nocapture)` passed; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test)` passed, 60 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. `src-tauri/src/lib.rs` is now 5375 lines and `src-tauri/src/workflow_state.rs` is 224 lines. |

### Current Remaining Implementation Order
1. Continue reducing `lib.rs` only along tested seams; export/Pack command orchestration remains high-risk and should move only after command-boundary lifecycle tests exist.
2. Preserve workflow-state invariants: preview diagnostics cannot alone publish, SourceReview issues route to DocumentReview, AuthoringIR failures route to Authoring, and export readiness requires both static validation and publish readiness.
3. Preserve production dependency strategy: Rust-first parser/export/validation, optional Node/Python diagnostics/legacy fallback only, and vision LLM rather than bundled OCR for image PDFs.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-49

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-49 | P1 | complete | Extracted runtime/static validation and publish-readiness helpers into `src-tauri/src/runtime_validation.rs`. The module now owns native preview asset generation, Rust static runtime gate, optional Node validator diagnostics, preview E2E sidecar execution, and SourceReview/AuthoringReview publish readiness merging. `lib.rs` keeps Tauri commands, export/Pack side effects, and job lifecycle orchestration. | `(cd src-tauri && cargo test rust_contract_validator -- --nocapture)` passed, 10 tests; `(cd src-tauri && cargo test publish_gate_blocks_no_text_pdf_until_source_review_resolved -- --nocapture)` passed; `(cd src-tauri && cargo test preview_e2e_diagnostic_failure_does_not_block_static_export_ready -- --nocapture)` passed; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test)` passed, 60 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. `src-tauri/src/lib.rs` is now 5185 lines and `src-tauri/src/runtime_validation.rs` is 223 lines. |

### Current Remaining Implementation Order
1. Continue reducing `lib.rs` only along tested seams; export/Pack command orchestration remains high-risk because it coordinates validation, artifact writes, cleanup, and job state.
2. Preserve runtime/publish invariants: static Rust contract gate is the production default, real runtime E2E is diagnostic, unresolved SourceReview/AuthoringReview issues block publish, and failed gates cannot write export/Pack artifacts.
3. Preserve production dependency strategy: Rust-first parser/export/validation, optional Node/Python diagnostics/legacy fallback only, and vision LLM rather than bundled OCR for image PDFs.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-50

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-50 | P0 | complete | Revalidated the product clarification that opening `Questions 14-26` / `Questions 14\u{2013}26` instructions are valid Passage-level question-group ranges and must be included. Production behavior remains the two-level model: preserve the opening range in `umbrellaQuestionRanges`, use later concrete headings as publishable `questionGroupCandidates`, and create a low-confidence `requiresManualQuestionImport` scaffold only when no concrete group exists. Added explicit Rust regression coverage for the en-dash spelling without changing runtime logic. | `(cd src-tauri && cargo test umbrella_question_range_detection_keeps_opening_instructions_distinct -- --nocapture)` passed; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` passed; `npm run check` passed. |

### Current Remaining Implementation Order
1. Preserve the umbrella range invariant across future parser/module refactors: opening Passage-level `Questions 14-26` style ranges are metadata/review context, not duplicate concrete interaction groups.
2. Continue reducing `lib.rs` only along tested seams; export/Pack command orchestration remains high-risk because it coordinates validation, artifact writes, cleanup, and job state.
3. Preserve runtime/publish invariants: static Rust contract gate is the production default, real runtime E2E is diagnostic, unresolved SourceReview/AuthoringReview issues block publish, and failed gates cannot write export/Pack artifacts.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-51

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-51 | P1 | complete | Extracted the pure dynamic split and AuthoringIR construction rules into `src-tauri/src/authoring_pipeline.rs`. The new module owns DocumentIR block text helpers, question-range/umbrella detection, answer text parsing, split candidate generation, split answer-source merging, prompt extraction, and initial ReadingAuthoringIRV1 construction. `lib.rs` keeps Tauri commands, file IO, parser side effects, source review, LLM orchestration, export/Pack side effects, cleanup, and job lifecycle transitions. | `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test umbrella_question_range_detection_keeps_opening_instructions_distinct -- --nocapture)` passed; `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` passed; `(cd src-tauri && cargo test)` passed, 60 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. `src-tauri/src/lib.rs` is now 4250 lines and `src-tauri/src/authoring_pipeline.rs` is 956 lines. |

### Current Remaining Implementation Order
1. Preserve the authoring pipeline invariants during future refactors: umbrella ranges are retained as metadata, concrete groups come from concrete headings or manual import, answer parsing remains source-backed, and generated AuthoringIR is never human-verified by default.
2. Continue reducing `lib.rs` only along tested seams; export/Pack command orchestration remains high-risk because it coordinates validation, artifact writes, cleanup, and job state.
3. Preserve runtime/publish invariants: static Rust contract gate is the production default, real runtime E2E is diagnostic, unresolved SourceReview/AuthoringReview issues block publish, and failed gates cannot write export/Pack artifacts.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-52

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-52 | P1 | complete | Consolidated successful-export project archival and transient cleanup into `src-tauri/src/cleanup.rs`. The cleanup module now owns `AuthoringProjectV1` writing, source/review/validation/export summary assembly, diagnostics-retention behavior, transient artifact removal, and the final `Cleaned` job-state transition. `lib.rs` still owns export/Pack artifact writing and publish gate orchestration. | `(cd src-tauri && cargo fmt --check)` passed; targeted cleanup/export/Pack positive and negative tests passed; `(cd src-tauri && cargo test)` passed, 60 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. `src-tauri/src/lib.rs` is now 4165 lines and `src-tauri/src/cleanup.rs` is 135 lines. |

### Current Remaining Implementation Order
1. Preserve cleanup invariants: successful export/Pack writes `authoring-project.json`, cleans transient process files unless diagnostics retention is enabled, and failed publish gates must not write final artifacts or cleanup summaries.
2. Continue reducing `lib.rs` only along tested seams; export/Pack artifact-writing orchestration and auto-pipeline command flow remain high-risk because they coordinate validation, file writes, cleanup, LLM calls, and job state.
3. Preserve runtime/publish invariants: static Rust contract gate is the production default, real runtime E2E is diagnostic, unresolved SourceReview/AuthoringReview issues block publish, and failed gates cannot write export/Pack artifacts.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-53

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-53 | P0 | complete | Added direct command-core regression coverage for the automatic upload pipeline. `run_auto_pipeline` now delegates to testable `run_auto_pipeline_core`, and tests prove two critical safety paths: clear text imports with unavailable LLM stay in `NeedsReview`/`LlmReview` without cleanup/export, and no-text/image PDF imports attempt vision transcription but remain blocked in `NeedsReview`/`DocumentReview` with unresolved SourceReview. | `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test auto_pipeline_llm_failure_keeps_text_import_in_llm_review -- --nocapture)` passed; `(cd src-tauri && cargo test auto_pipeline_keeps_no_text_pdf_blocked_for_source_review -- --nocapture)` passed; `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` passed; `(cd src-tauri && cargo test)` passed, 62 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. |

### Current Remaining Implementation Order
1. Preserve auto-pipeline invariants: parser/vision uncertainty routes to SourceReview, LLM failure/low-confidence routes to LlmReview, generated AuthoringIR is never human-verified by default, and no auto pipeline path writes cleanup/export artifacts before review gates pass.
2. Add more command-level coverage for positive high-confidence LLM auto-apply using a controlled provider/mock seam before moving auto-pipeline orchestration out of `lib.rs`.
3. Continue reducing `lib.rs` only along tested seams; export/Pack artifact-writing orchestration and auto-pipeline command flow remain high-risk because they coordinate validation, file writes, cleanup, LLM calls, and job state.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-54

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-54 | P0 | complete | Hardened the clarified business rule that opening `Questions 14-26` / `Questions 14–26` instructions are valid Passage-level question-group metadata and must be included. Added a minimal regression proving the opening total range is preserved in `umbrellaQuestionRanges`, later concrete groups remain `14-19` / `20-23` / `24-26`, no duplicate concrete Q14-Q26 group is created, and manual-import scaffolding is not triggered when concrete groups exist. | `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test opening_umbrella_range_is_included_without_replacing_concrete_groups -- --nocapture)` passed; `(cd src-tauri && cargo test umbrella_question_range_detection_keeps_opening_instructions_distinct -- --nocapture)` passed; `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` passed; `npm run check` passed; `(cd src-tauri && cargo test)` passed, 63 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed. |

### Current Remaining Implementation Order
1. Preserve the umbrella range invariant across future parser/module refactors: opening Passage-level `Questions 14-26` style ranges are included as metadata/review context, not duplicate concrete interaction groups.
2. Add more command-level coverage for positive high-confidence LLM auto-apply using a controlled provider/mock seam before moving auto-pipeline orchestration out of `lib.rs`.
3. Continue reducing `lib.rs` only along tested seams; export/Pack artifact-writing orchestration and auto-pipeline command flow remain high-risk because they coordinate validation, file writes, cleanup, LLM calls, and job state.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-55

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-55 | P0 | complete | Added command-core positive coverage for high-confidence LLM auto-apply in the automatic upload pipeline. Introduced `run_auto_pipeline_core_with_gateway(...)` as an internal test seam while keeping production `run_auto_pipeline_core(...)` on the real Rust gateway. The new regression proves source-evidenced high-confidence LLM suggestions can auto-apply structure and prompts, record `autoApplied`, preserve parsed answers, and still leave `audit.humanVerified=false` with the job in `NeedsReview`/`Authoring` until human verification. | `(cd src-tauri && cargo test auto_pipeline_high_confidence_llm_auto_applies_without_human_verification -- --nocapture)` passed; adjacent auto-pipeline failure/no-text PDF tests passed; `files_pdf_samples_reach_expected_review_paths` passed; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test)` passed, 64 tests; `npm run check` passed; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `git diff --check` passed. |

### Current Remaining Implementation Order
1. Extract auto-pipeline orchestration from `src-tauri/src/lib.rs` into a dedicated module now that parse failure, no-text PDF, LLM failure, high-confidence auto-apply, validation, and job-state side effects have command-core coverage.
2. Preserve auto-pipeline invariants during extraction: parser/vision uncertainty routes to SourceReview, LLM failure/low-confidence routes to LlmReview, high-confidence LLM auto-apply cannot create human verification, and no path writes cleanup/export artifacts before review gates pass.
3. Continue reducing `lib.rs` only along tested seams; export/Pack artifact-writing orchestration remains high-risk because it coordinates validation, file writes, cleanup, and job state.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-56

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-56 | P1 | complete | Extracted automatic upload pipeline orchestration from `src-tauri/src/lib.rs` into `src-tauri/src/auto_pipeline.rs`. The new module owns parse-if-needed, SourceReview initialization, vision transcription routing, split/answer merge, AuthoringIR generation, LLM suggestion/fallback, high-confidence auto-apply, static validation, and final pipeline/job-state projection. `lib.rs` keeps the Tauri command wrapper and delegates to the module. | Auto-pipeline high-confidence, LLM failure, no-text PDF SourceReview, and real PDF sample regressions passed; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test)` passed, 64 tests; `npm run check` passed; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `git diff --check` passed. `src-tauri/src/lib.rs` is now 3977 lines and `src-tauri/src/auto_pipeline.rs` is 632 lines. |

### Current Remaining Implementation Order
1. Continue reducing `lib.rs` along tested seams; export/Pack artifact-writing orchestration is the next large high-risk boundary and should only move with existing export/Pack positive and negative tests as guards.
2. Preserve auto-pipeline invariants in future typed refactors: parser/vision uncertainty routes to SourceReview, LLM failure/low-confidence routes to LlmReview, high-confidence LLM auto-apply cannot create human verification, and no auto pipeline path writes cleanup/export artifacts before review gates pass.
3. Keep production dependency strategy stable: Rust-first parser/export/validation, optional Node/Python diagnostics/legacy fallback only, and vision LLM rather than bundled OCR for image PDFs.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-57

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-57 | P1 | complete | Extracted export/Pack artifact-writing orchestration from `src-tauri/src/lib.rs` into `src-tauri/src/export_pack.rs`. The new module owns single asset export, Pack artifact writing, static runtime/publish gate enforcement, job status transition to `Exported`, zip writing, and successful-export cleanup invocation. Tauri command wrappers remain in `lib.rs` so handler macro generation stays correct. | Export positive/negative tests and Pack positive/negative tests passed; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test)` passed, 64 tests; `npm run check` passed; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `git diff --check` passed. `src-tauri/src/lib.rs` is now 3820 lines and `src-tauri/src/export_pack.rs` is 173 lines. |

### Current Remaining Implementation Order
1. Continue reducing `lib.rs` by extracting lower-risk command domains with existing tests, such as preview/runtime command wrappers or LLM command orchestration.
2. Preserve export/Pack invariants: static runtime gate plus SourceReview/AuthoringReview publish readiness are mandatory; failed gates write no final artifacts or cleanup summaries; successful exports/Pack invoke cleanup according to diagnostics retention.
3. Start typed-domain refactors where modules are now isolated (`auto_pipeline.rs`, `export_pack.rs`, `authoring_pipeline.rs`) to reduce long-term JSON-field drift.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-58

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-58 | P0 | complete | Hardened the opening Passage-level `Questions 14-26` / `Questions 14–26` rule for PDF extraction variants where the range appears as a standalone heading block. Rust production split and browser dev fallback now treat a bare opening full-passage range near `READING PASSAGE` as `umbrellaQuestionRanges`, preserve later concrete subgroups as publishable `questionGroupCandidates`, and avoid duplicate concrete Q14-Q26 groups. The rule remains conservative: short concrete subgroup ranges are not umbrella, and umbrella-only detections still require manual concrete question import before publish. | `(cd src-tauri && cargo test umbrella -- --nocapture)` passed, 3 tests; `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` passed; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test)` passed, 65 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. |

### Current Remaining Implementation Order
1. Preserve the two-level question-range model in future refactors: opening full-passage ranges are review metadata; concrete subgroups or manual import remain the source of publishable interactions.
2. Continue reducing `lib.rs` by extracting lower-risk command domains with existing tests, such as preview/runtime command wrappers or LLM command orchestration.
3. Start typed-domain refactors where modules are now isolated (`auto_pipeline.rs`, `export_pack.rs`, `authoring_pipeline.rs`) to reduce long-term JSON-field drift.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-59

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-59 | P1 | complete | Extracted LLM command-core orchestration from `src-tauri/src/lib.rs` into `src-tauri/src/llm_commands.rs`. The new module owns profile save/test core logic, classify/extract suggestion generation, suggestion persistence, high-confidence suggestion application, AuthoringReview refresh, SourceReview issue merging, answerKey/questionOrder/displayMap regeneration, and job-state updates. Tauri command wrappers remain in `lib.rs` so frontend command names and handler macro scope stay stable. | `(cd src-tauri && cargo test llm -- --nocapture)` passed, 8 tests; `(cd src-tauri && cargo test auto_pipeline_high_confidence_llm_auto_applies_without_human_verification -- --nocapture)` passed; `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` passed; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test)` passed, 65 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. `src-tauri/src/lib.rs` is now 3637 lines and `src-tauri/src/llm_commands.rs` is 241 lines. |

### Current Remaining Implementation Order
1. Continue reducing `lib.rs` by extracting remaining lower-risk command domains with existing tests, especially validation/preview command wrappers or job/file command orchestration.
2. Preserve LLM invariants: gateway fallback remains low-confidence, auto-apply requires source evidence and confidence, LLM output never creates human verification, and SourceReview/AuthoringReview still block publish.
3. Start typed-domain refactors where modules are now isolated (`auto_pipeline.rs`, `export_pack.rs`, `authoring_pipeline.rs`, `llm_commands.rs`) to reduce long-term JSON-field drift.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-60

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-60 | P1 | complete | Extracted validation/preview command-core orchestration from `src-tauri/src/lib.rs` into `src-tauri/src/preview_commands.rs`. The new module owns AuthoringIR validation command core, preview asset generation core, real-runtime E2E diagnostic orchestration, static runtime gate invocation, publish-readiness check projection, and job-state updates through workflow helpers. Tauri command wrappers remain in `lib.rs`, preserving frontend command names and handler macro scope. | `(cd src-tauri && cargo test preview -- --nocapture)` passed, 3 tests; `(cd src-tauri && cargo test runtime_gate -- --nocapture)` passed, 3 tests; `(cd src-tauri && cargo test validation_job_state -- --nocapture)` passed; `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` passed; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test)` passed, 65 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. `src-tauri/src/lib.rs` is now 3523 lines and `src-tauri/src/preview_commands.rs` is 146 lines. |

## Implementation Update: 2026-06-01 / E8-61

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-61 | P1 | complete | Extracted document/source-review/split/AuthoringIR command-core orchestration from `src-tauri/src/lib.rs` into `src-tauri/src/authoring_commands.rs`. The new module owns parse document, manual transcription, vision transcription, source review resolution, rule split, split adjustment save, AuthoringIR build/update, and group HTML render command cores. Tauri command wrappers remain in `lib.rs`. The opening `Questions 14-26` / `Questions 14–26` rule is preserved as valid umbrella range metadata: it is stored in `umbrellaQuestionRanges`, concrete subgroups remain the publishable question groups when present, and umbrella-only samples generate a low-confidence manual-import scaffold. | `(cd src-tauri && cargo check)` passed; `(cd src-tauri && cargo test umbrella -- --nocapture)` passed, 3 tests; `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` passed; `(cd src-tauri && cargo test complex_ -- --nocapture)` passed, 4 tests; `(cd src-tauri && cargo test transcription -- --nocapture)` passed, 2 tests; `(cd src-tauri && cargo test preview -- --nocapture)` passed, 3 tests; `(cd src-tauri && cargo test runtime_gate -- --nocapture)` passed, 3 tests; `(cd src-tauri && cargo test validation_job_state -- --nocapture)` passed; `(cd src-tauri && cargo test)` passed, 65 tests; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. `src-tauri/src/lib.rs` is now 3304 lines and `src-tauri/src/authoring_commands.rs` is 289 lines. |

## Implementation Update: 2026-06-01 / E8-62

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-62 | P1 | complete | Extracted job/import/settings/preflight command-core orchestration from `src-tauri/src/lib.rs` into `src-tauri/src/job_commands.rs`. The new module owns create/list/get/update/delete job, import source file, reveal job folder, choose export directory, list LLM profiles, environment preflight, and diagnostics settings command cores. App directory setup and file import helpers moved into `util.rs`; parser and LLM profile modules now reference `environment::{command_failure, find_sidecar}` directly instead of root-level aliases. Tauri wrappers remain in `lib.rs`, preserving frontend command names. | `(cd src-tauri && cargo check)` passed; `(cd src-tauri && cargo test job -- --nocapture)` passed, 3 tests; `(cd src-tauri && cargo test environment_preflight_reports_required_dependency_names -- --nocapture)` passed; `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` passed; `(cd src-tauri && cargo test complex_ -- --nocapture)` passed, 4 tests; `(cd src-tauri && cargo test)` passed, 65 tests; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. `src-tauri/src/lib.rs` is now 3146 lines and `src-tauri/src/job_commands.rs` is 186 lines. |

## Implementation Update: 2026-06-01 / E8-63

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-63 | P1 | complete | Introduced the first narrow typed-domain backend seam by adding `SourceReviewV1` in `src-tauri/src/source_review.rs`. `source_review_status` / `write_source_review_status` now round-trip a typed struct before returning the same JSON contract, while `source_review_issues` can read either typed or legacy JSON input. Added a Rust regression that locks `schemaVersion`, `jobId`, `required`, `resolved`, `stale`, `fingerprint`, `parserWarnings`, `lowConfidenceBlocks`, `resolvedAt`, and `note` semantics. | `(cd src-tauri && cargo test source_review_status_preserves_v1_json_contract -- --nocapture)` passed; `(cd src-tauri && cargo test source_review -- --nocapture)` passed, 7 tests; `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` passed; `(cd src-tauri && cargo test)` passed, 66 tests; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. `src-tauri/src/source_review.rs` is now 263 lines. |

### Current Remaining Implementation Order
1. Continue typed-domain refactors where modules are isolated, starting with `DocumentIR` / `SplitCandidates` / `ReadingAuthoringIR` validation and export DTOs, rather than more command wrapper extraction.
2. Preserve upload/PDF invariants: text-layer PDFs use Rust deterministic parsing; image/no-text PDFs route to vision LLM/manual transcription plus mandatory SourceReview; opening umbrella ranges remain metadata and do not create duplicate concrete groups.
3. Preserve preview/runtime invariants: Rust static validation is the production gate, real runtime E2E stays diagnostic, failed validation cannot mark jobs export-ready, and publish readiness still requires SourceReview/AuthoringReview/human verification.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-64

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-64 | P1 | complete | Introduced a typed-domain seam for split candidates by adding `SplitCandidatesV1` and child DTOs in `src-tauri/src/authoring_pipeline.rs`. The dynamic split path now builds typed structs and serializes to the existing camelCase JSON contract. The opening `Questions 14-26` / `Questions 14–26` rule is explicitly preserved: it is retained in `umbrellaQuestionRanges`, not duplicated as a concrete group when later concrete subgroups exist, and umbrella-only detections create low-confidence `requiresManualQuestionImport` scaffolds. | `(cd src-tauri && cargo test umbrella -- --nocapture)` passed, 4 tests; `(cd src-tauri && cargo test split_candidates_v1_preserves_umbrella_contract_and_manual_scaffold -- --nocapture)` passed; `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` passed; `(cd src-tauri && cargo test complex_ -- --nocapture)` passed, 4 tests; `(cd src-tauri && cargo test source_review -- --nocapture)` passed, 7 tests; `(cd src-tauri && cargo test)` passed, 67 tests; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. |

### Current Remaining Implementation Order
1. Continue typed-domain refactors with `ReadingAuthoringIR` and validation/export DTO boundaries, using the same pattern: typed Rust structs internally, unchanged frontend JSON contract externally.
2. Preserve upload/PDF invariants: text-layer PDFs use Rust deterministic parsing; image/no-text PDFs route to vision LLM/manual transcription plus mandatory SourceReview; opening umbrella ranges remain metadata and do not create duplicate concrete groups.
3. Audit auto-pipeline persistence around split/manual review transitions so umbrella-only and low-confidence visual transcription outputs cannot accidentally advance to publish-ready states.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-65

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-65 | P1 | complete | Added a typed-domain seam for `ReadingAuthoringIRV1` generation in `src-tauri/src/authoring_pipeline.rs`. The dynamic AuthoringIR builder now constructs typed Rust DTOs for exam metadata, passage, groups, questions, audit, source files, `answerKey`, `questionOrder`, and `questionDisplayMap`, then serializes to the unchanged frontend JSON contract. The manual-import/umbrella-only path remains blocked by AuthoringReview/publish readiness even though its structural validation shape can be valid. | `(cd src-tauri && cargo test reading_authoring_ir_v1_preserves_manual_import_contract -- --nocapture)` passed; `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` passed; `(cd src-tauri && cargo test authoring -- --nocapture)` passed, 10 tests; `(cd src-tauri && cargo test)` passed, 68 tests; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. |

### Current Remaining Implementation Order
1. Continue typed-domain refactors at the export/runtime boundary, especially `ReadingExamSourceV1`, validation reports, and publish-readiness reports.
2. Preserve upload/PDF invariants: text-layer PDFs use Rust deterministic parsing; image/no-text PDFs route to vision LLM/manual transcription plus mandatory SourceReview; opening umbrella ranges remain metadata and do not create duplicate concrete groups.
3. Audit auto-pipeline persistence around split/manual review transitions so umbrella-only and low-confidence visual transcription outputs cannot accidentally advance to publish-ready states.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-66

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-66 | P1 | complete | Added a typed-domain seam for `ReadingExamSourceV1` export/runtime generation in `src-tauri/src/reading_source.rs`. The boundary now constructs typed Rust DTOs for source metadata, passage blocks, question groups, source refs, and audit before serializing to the unchanged preview/export/runtime JSON contract. Passage blocks are normalized to explicit `kind: "html"` entries, keeping the existing validator and runtime consumers satisfied. | `(cd src-tauri && cargo test reading_source_v1_preserves_export_contract -- --nocapture)` passed; `(cd src-tauri && cargo test reading_source_uses_real_source_metadata_and_review_status -- --nocapture)` passed; `(cd src-tauri && cargo test authoring -- --nocapture)` passed, 10 tests; `(cd src-tauri && cargo test)` passed, 69 tests; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. |

### Current Remaining Implementation Order
1. Continue typed-domain refactors on the validation/report layer, especially `validate_authoring`, runtime validation reports, and export/pack report DTOs.
2. Preserve upload/PDF invariants: text-layer PDFs use Rust deterministic parsing; image/no-text PDFs route to vision LLM/manual transcription plus mandatory SourceReview; opening umbrella ranges remain metadata and do not create duplicate concrete groups.
3. Audit auto-pipeline persistence around split/manual review transitions so umbrella-only and low-confidence visual transcription outputs cannot accidentally advance to publish-ready states.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-67

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-67 | P1 | complete | Added a typed-domain seam for `ValidationReportV1` and validation layer reports in `src-tauri/src/validator.rs` / `src-tauri/src/authoring_validation.rs`. Authoring validation now constructs typed report DTOs before serializing to the unchanged frontend JSON contract, while runtime diagnostics remain an optional JSON extension field. The static runtime gate contract is locked by a new regression covering top-level keys, layer counts, and `runtime.mode = static-rust`. | `(cd src-tauri && cargo test validation_report_v1_preserves_static_runtime_contract -- --nocapture)` passed; `(cd src-tauri && cargo test runtime_gate -- --nocapture)` passed, 3 tests; `(cd src-tauri && cargo test preview -- --nocapture)` passed, 3 tests; `(cd src-tauri && cargo test)` passed, 70 tests; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. |

### Current Remaining Implementation Order
1. Audit remaining non-typed report/export summaries only where they affect user-visible workflow or publish safety; avoid low-value mechanical typing of internal diagnostics.
2. Preserve upload/PDF invariants: text-layer PDFs use Rust deterministic parsing; image/no-text PDFs route to vision LLM/manual transcription plus mandatory SourceReview; opening umbrella ranges remain metadata and do not create duplicate concrete groups.
3. Audit auto-pipeline persistence around split/manual review transitions so umbrella-only and low-confidence visual transcription outputs cannot accidentally advance to publish-ready states.
4. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.

## Implementation Update: 2026-06-01 / E8-68

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-68 | P0 | complete | Audited and hardened auto-pipeline status projection for umbrella-only/manual-import drafts. Added a Rust regression proving that a structurally valid umbrella-only `Questions 14–26` draft with `staticRuntimePassed=true` still remains `NeedsReview` and cannot advance to `ExportReady` while `requiresManualQuestionImport` / AuthoringReview items remain. Synced browser dev fallback with Rust by including AuthoringReview in auto-pipeline status/currentStep/issueCount projection and exposing `authoring.remainingReviewItems` in `AutoPipelineReport`. | `(cd src-tauri && cargo test auto_pipeline_blocks_umbrella_only_manual_import_from_export_ready -- --nocapture)` passed; `(cd src-tauri && cargo test auto_pipeline -- --nocapture)` passed, 4 tests; `(cd src-tauri && cargo test)` passed, 71 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `(cd src-tauri && cargo fmt --check)` passed; `npm run check` passed; `git diff --check` passed. |

### Current Remaining Implementation Order
1. Continue end-to-end UI/runtime coverage for the full upload -> split -> SourceReview/AuthoringReview -> export workflow, especially real representative PDFs and manual intervention paths.
2. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.
3. Audit remaining user-visible report/export summaries only where they affect workflow safety; avoid low-value mechanical typing of internal diagnostics.
4. Preserve production dependency direction: Rust-first parsing/validation/export, no bundled Node/Python/OCR hard dependency, vision LLM for image/scanned PDFs with mandatory SourceReview.

## Implementation Update: 2026-06-01 / E8-69

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-69 | P0 | complete | Implemented the clarified product requirement that opening instructions such as `Questions 14-26` / `Questions 14–26` are valid question-group information and must be included beyond the split page. The app still avoids duplicating the opening full-passage range as a concrete Q14-Q26 interaction group when later subgroups exist, but now carries it forward as `passage.questionUmbrellaRanges` in `ReadingAuthoringIRV1`, `meta.questionUmbrellaRanges` plus `meta.questionIntroHtml` in `ReadingExamSourceV1`, and visible context in GroupEditor / UnifiedPreview / dev fallback preview. | `(cd src-tauri && cargo test opening_umbrella_range_is_included_without_replacing_concrete_groups -- --nocapture)` passed; `(cd src-tauri && cargo test umbrella -- --nocapture)` passed, 5 tests; `(cd src-tauri && cargo fmt --check && cargo test)` passed, 71 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `npm run check` passed; `git diff --check` passed. |

### Current Remaining Implementation Order
1. Continue end-to-end UI/runtime coverage for the full upload -> split -> SourceReview/AuthoringReview -> export workflow, especially representative PDFs and manual intervention paths.
2. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.
3. Preserve the two-level question-range model: opening full-passage ranges are included source/review metadata; concrete subgroups or manual import remain the source of publishable interactions.
4. Preserve production dependency direction: Rust-first parsing/validation/export, no bundled Node/Python/OCR hard dependency, vision LLM for image/scanned PDFs with mandatory SourceReview.

## Implementation Update: 2026-06-01 / E8-70

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-70 | P0 | complete | Revalidated the user clarification that `Questions 14-26` / `Questions 14–26` appearing in the opening instructions is a correct question-group range and must be included. The current implementation covers both full-sentence and standalone heading presentations: it stores the opening range as `umbrellaQuestionRanges`, propagates it to `ReadingAuthoringIRV1.passage.questionUmbrellaRanges` and `ReadingExamSourceV1.meta.questionUmbrellaRanges`, renders it through `questionIntroHtml`, and keeps concrete subgroups or manual import as the publishable question source. | `npm run check` passed; `(cd src-tauri && cargo test umbrella -- --nocapture)` passed, 5 tests; `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` passed; `(cd src-tauri && cargo fmt --check)` passed; `(cd src-tauri && cargo test)` passed, 72 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `git diff --check` passed. |

### Current Remaining Implementation Order
1. Preserve the two-level question-range invariant in future parser/refactor work: opening full-passage ranges are valid metadata, but not a substitute for concrete prompts unless the user manually imports and verifies them.
2. Continue end-to-end UI/runtime coverage for upload -> split -> review -> export using representative PDFs, especially manual-intervention and vision-transcription paths.
3. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.
4. Preserve production dependency direction: Rust-first parsing/validation/export, no bundled Node/Python/OCR hard dependency, vision LLM for image/scanned PDFs with mandatory SourceReview.

## Implementation Update: 2026-06-01 / E8-71

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-71 | P1 | complete | Added a development/CI browser UI-flow diagnostic without adding production dependencies. `npm run e2e:ui-flow` launches or reuses Vite, drives host Chrome/Chromium via DevTools Protocol, and verifies two critical app flows: clear-text upload -> auto-pipeline -> LLM review with static runtime evidence, and OCR/scanned upload -> vision transcription -> SourceReview-first routing. The run exposed and fixed a dev fallback drift where merged validation reports dropped `runtime.mode`; `mergeValidationReports` now preserves `sidecar.runtime`. | `npm run e2e:ui-flow` passed with `clear-text-auto-pipeline` status `NeedsReview`, currentStep `LlmReview`, runtimeMode `static-rust`, and `ocr-source-review-priority` status `NeedsReview`, currentStep `DocumentReview`, sourceReview `required`, `visionApplied=true`; `npm run check` passed; `(cd src-tauri && cargo fmt --check && cargo test)` passed, 72 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `git diff --check` passed. |

### Current Remaining Implementation Order
1. Extend browser UI-flow diagnostics from smoke coverage to full manual-intervention coverage: DocumentReview resolve/manual transcription, GroupEditor human verification, Preview, Export, and Pack.
2. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.
3. Preserve production dependency direction: `ui-flow-e2e` remains a development/CI diagnostic only; production app still does not bundle Node, Python, browser automation, or OCR runtimes.
4. Keep dev fallback behavior aligned with Rust production path for report fields, SourceReview/AuthoringReview routing, and runtime evidence.

## Implementation Update: 2026-06-01 / E8-72

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-72 | P1 | complete | Expanded `npm run e2e:ui-flow` from routing smoke coverage into a broader browser workflow diagnostic. The clear-text path now covers upload -> auto-pipeline -> low-confidence LLM review -> simulated human verification -> GroupEditor validation -> UnifiedPreview generation -> RuntimePreview diagnostic -> Export -> PackBuilder. The OCR/scanned path still verifies vision transcription plus SourceReview-first blocking. Added stable UI selectors for GroupEditor preview validation, UnifiedPreview generation/E2E/export navigation, and PackBuilder selection/build result. | `npm run e2e:ui-flow` passed with `clear-text-review-preview-export-pack` finalStatus `Cleaned`, runtimeMode `static-rust`, `exportedFileCount=4`, `packBuilt=true`, and `ocr-source-review-priority` currentStep `DocumentReview`, sourceReview `required`, `visionApplied=true`; `npm run check` passed; `(cd src-tauri && cargo fmt --check && cargo test)` passed, 72 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `git diff --check` passed. |

### Current Remaining Implementation Order
1. Extend UI-flow diagnostics to the scanned/manual-intervention path: SourceReview resolve, manual transcription text, concrete prompt verification, preview/export after source review.
2. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.
3. Preserve production dependency direction: UI E2E remains development/CI only and must not become a production runtime dependency.
4. Continue aligning dev fallback behavior with Rust production path when diagnostics expose report, review, or cleanup drift.

## Implementation Update: 2026-06-01 / E8-73

| ID | Priority | Status | Summary | Evidence |
|----|----------|--------|---------|----------|
| E8-73 | P1 | complete | Extended the browser UI-flow diagnostic to cover the scanned/image-PDF manual recovery path end-to-end. The OCR path now proves the intended product behavior: initial vision transcription remains blocked by SourceReview, the author can paste manual transcription in DocumentReview, SourceReview becomes resolved/not required, rule split and AuthoringIR generation work from the manual text, simulated human verification clears AuthoringReview, and the same preview/export/Pack closure succeeds with static runtime evidence. | `npm run e2e:ui-flow` passed with `ocr-manual-transcription-review-preview-export-pack`: initialStatus `NeedsReview`, initialStep `DocumentReview`, initialSourceReview `required`, `visionApplied=true`, `manualProvider=manual-transcription`, finalStatus `Cleaned`, runtimeMode `static-rust`, `exportedFileCount=4`, `packBuilt=true`; clear text path also passed with finalStatus `Cleaned`; `npm run check` passed; `(cd src-tauri && cargo fmt --check && cargo test)` passed, 72 tests; `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` passed; `git diff --check` passed. |

### Current Remaining Implementation Order
1. Add live provider/diagnostic coverage for Rust LLM gateway and vision transcription when credentials and representative scanned PDFs are available.
2. Consider adding packaged Tauri smoke coverage for the same flow, separate from the Vite/dev-fallback browser diagnostic.
3. Preserve production dependency direction: UI E2E remains development/CI only and must not become a production runtime dependency.
4. Continue aligning dev fallback behavior with Rust production path when diagnostics expose report, review, or cleanup drift.
