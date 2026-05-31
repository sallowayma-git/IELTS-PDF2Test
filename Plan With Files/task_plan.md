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
| E8-04 | 实现文件导入与 sidecar/parser 接入骨架 | Tauri doc: Files/Parser commands | complete | E8-03 | TXT/MD/PDF/DOCX Python parser sidecar 已接入 Rust 调度；已补复杂 PDF/DOCX 与 no-text PDF fixture 回归 |
| E8-05 | 实现规则粗切、答案对齐与 Authoring IR 编辑 | Tauri + output-contract pipeline | complete | E8-04 | 可从 Document IR 生成题组草稿并在 UI 编辑；新增上传后自动流水线（解析、粗切、AuthoringIR、LLM批处理、校验/E2E） |
| E8-06 | 实现 LLM profile、密钥存储、调用与建议审阅 | Tauri doc: LLM 设置与安全、LLM Review | in_progress | E8-05 | 本地 LLM gateway sidecar、profile 密钥文件引用、结构化建议与审阅 UI 已接入；自动流水线高置信自动应用、低置信进入人工审阅；仍需 Stronghold/加密兜底和更严格 suggestion schema/evidence 校验 |
| E8-07 | 实现模板渲染、统一阅读页预览与校验 | Tauri + output-contract docs | in_progress | E8-05 | JS/manifest 生成、增强 DOM validator、RuntimePreview gate 已接入；外部统一阅读页最小 real-runtime E2E 和导出/Pack 命令级 real-runtime fixture 已通过；仍需更多复杂题型与完整 UI E2E |
| E8-08 | 实现 PackBuilder、JS 导出和组 Pack 发布 | Tauri doc: Pack 发布 | in_progress | E8-07 | 单题 JS/manifest、Pack 目录/标准 `.zip` 已实现；导出/Pack 已强制 AuthoringIR + ReadingExamSourceV1 + DOM + RuntimePreview 四层门禁；仍需交付包自包含依赖策略 |
| E8-09 | 完成 Dashboard、ImportWizard、DocumentReview、SplitAndAnswers、GroupEditor、LlmReview、UnifiedPreview、PackBuilder、Settings 全页面 | Tauri doc: 页面详细设计 | complete | E8-02..E8-08 | 页面流程闭环且状态可持久化 |
| E8-10 | 验收测试、错误修复与文档同步 | Tauri doc: 最小 MVP 范围、关键验收用例 | in_progress | E8-03..E8-09 | 已完成构建/语法/冒烟验证、真实 unified runtime、no-text PDF、复杂 PDF/DOCX、命令级 export/Pack 回归；仍需 OCR/扫描 PDF 策略和更广 UI E2E |
| E8-11 | 审计后发布硬门禁补齐 | Deep audit 2026-05-31 | in_progress | E8-06..E8-10 | 已新增 `publish_readiness_gate`、SourceReviewV1、no-text PDF fixture、真实 runtime E2E、命令级导出/Pack real-runtime fixture；仍需 OCR 策略定型 |
| E8-12 | 审计后架构拆分与测试基线 | Deep audit 2026-05-31 | in_progress | E8-03..E8-10 | 已新增 parser no-text PDF、complex PDF/DOCX、source metadata、export/Pack real-runtime core 测试；Rust 单文件业务逻辑拆分仍未开始 |
| E8-13 | 生产化交付、安全与依赖自包含审计 | Deep audit 2026-05-31 15:40 | in_progress | E8-06..E8-12 | 已识别 sidecar 依赖系统 Node/Python/pypdf/外部 runtime、CSP 为空、HTML 预览渲染、明文文件密钥兜底等风险；需实现打包/安全 hardening |

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
- [x] 添加 PDF/DOCX deterministic parser adapter：PDF 走 `pypdf`，DOCX 走 Python stdlib OOXML 解析
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
- [x] 修复关键门禁策略：发布链路默认 strict real-runtime gate，避免 fallback 通过掩盖真实兼容性问题
- [x] 修复配置移植性风险：移除代码内本机绝对路径默认值，仅保留 `EPIC8_UNIFIED_HTML_PATH`/`EPIC8_UNIFIED_PYTHON` 注入
- [x] 完成当前实现深度审计，识别发布硬门禁与复杂 PDF/OCR 缺口
- [ ] 完成最终验收清单执行（真实 runtime、no-text PDF、复杂 PDF/DOCX、export/Pack 命令级证据已补；OCR adapter/人工转录策略、模块拆分、依赖自包含和安全 hardening 仍未完成）
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
- [x] 在未设置 `EPIC8_UNIFIED_HTML_PATH`/`EPIC8_UNIFIED_PYTHON` 时执行导出，预期：strict gate 拒绝发布（`runtime.mode != real`）。
- [x] 设置有效 unified runtime 环境后执行 RuntimePreview E2E，预期：`runtime.mode == real`、正确答案 100%、错误样本降分。
- [x] 组 Pack 发布在同样条件下验证 strict gate，预期：与导出一致（无真实 runtime 不可发布）。
- [x] 导出/Pack 前执行发布 readiness gate，阻断 `NeedsHumanReview`、未人工确认、空答案、低置信未确认和未解决 parser warning。

### B. Authoring to Output Contract
- [x] `ReadingAuthoringIRV1` -> `ReadingExamSourceV1` 字段完整性检查通过（`answerKey`/`questionOrder`/`questionDisplayMap`/`questionGroups`）。
- [x] DOM 协议校验通过（input/select/textarea/dropzone 可采集，题号映射正确）。
- [x] RuntimePreview 正确答案自动填充得分为 100%，错误答案样本得分低于正确答案。

### C. Parser and Import Reliability
- [x] TXT/MD/PDF/DOCX 导入解析均可产出 `DocumentIRV1`。
- [x] PDF/DOCX 复杂格式样例至少各 1 个，parser -> split -> AuthoringIR -> answerKey fixture 回归通过。
- [x] no-text PDF fixture 经 Python parser 产出 parser warning + low-confidence block，并由 `SourceReviewV1` 阻断发布；仍需真实扫描图片 OCR adapter 策略。
- [x] parser sidecar 失败不再生成 sample 题内容，而是生成 failure Document IR 并强制人工审核。
- [x] 导入、导出路径权限边界符合设计：仅 app data + 用户显式选择路径。

### D. LLM Safety and Review
- [x] API Key 存储策略可验证：macOS Keychain 可用时优先使用；不可用时进入本地文件兜底并在 UI 明示。
- [x] 低置信度建议不可直接 apply（需人工复核）；高置信度建议可按白名单 patch 应用。
- [x] LLM 输出为结构化 JSON 建议，不可绕过模板/校验/E2E 门禁。

### E. Build and Release Artifacts
- [x] `npm run check` 通过。
- [x] `cargo check`（`src-tauri`）通过。
- [x] `cargo clippy --all-targets -- -D warnings`（`src-tauri`）通过。
- [x] `npm run tauri build` 通过并产出 `.app`/`.dmg`。

### F. Tracking Consistency
- [x] `Plan With Files/task_plan.md` 状态与实际实现一致：主链路可运行，真实 runtime 最小证据已补；OCR adapter、复杂 PDF/DOCX、命令级 export/Pack fixture 和模块拆分仍未完成。
- [x] `Plan With Files/findings.md` 包含关键设计决策与风险。
- [x] `Plan With Files/progress.md` 记录执行结果、错误与修复。

## Deep Audit Findings: 2026-05-31

| ID | Severity | Area | Finding | Required Follow-up |
|----|----------|------|---------|--------------------|
| AUD-01 | P0 | Publish gate | `run_auto_pipeline` 会把 parser warning / low confidence 路由到人工审核，但 `export_reading_assets` / `build_pack` 只看四层 runtime gate，不回查 parser warning、low-confidence blocks、`NeedsHumanReview` 或人工确认字段 | 在导出/Pack 前统一执行 `publish_readiness_gate` |
| AUD-02 | P0 fixed | Parser fallback | Python parser 失败时 Rust 对 PDF/DOCX 曾退到高置信 sample Document IR | 已改为 failure Document IR，仍需 fixture 回归 |
| AUD-03 | P0 | Human verification | `verified` 和 `audit.humanVerified` 没有被导出门禁强制检查；`update_authoring_ir` 可把 needsReview 清零 | AuthoringIR 校验必须要求答案/低置信字段人工确认后才允许发布 |
| AUD-04 | P1 | OCR | `rerun_ocr` 只是以 `mode=ocr` 重新跑同一 pypdf 解析；没有真实 OCR adapter | 增加 OCR adapter 或明确 no-text PDF 只能人工录入并阻断发布 |
| AUD-05 | P1 | Architecture | 核心 backend 超过 3600 行集中在 `src-tauri/src/lib.rs`，状态机、parser、LLM、validator、export、pack 混在一起 | 拆分为 storage/parser/pipeline/llm/validator/exporter/pack modules |
| AUD-06 | P1 | Test coverage | 仓库没有业务自动化测试/fixture；当前验证主要是 build、syntax、browser smoke | 建立 Rust unit/integration tests、sidecar fixtures、真实 runtime smoke |

## Audit Update: 2026-05-31 13:38 CST

| ID | Severity | Status | Summary | Follow-up |
|----|----------|--------|---------|-----------|
| AUD-07 | P0 | open | Parser warnings / low-confidence blocks can be bypassed when authoring `humanVerified` becomes true and job status is advanced out of `NeedsHumanReview`. | Implement source/parser review provenance independent of question verification. |
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
| Export/Pack gate | P1 | improved | Extracted `export_reading_assets_core` and `build_pack_core` so Tauri commands and Rust tests share the same publish path. Added real-runtime command-level tests for export asset files and Pack zip/manifest generation. | `cargo test` with external runtime env passed 11 tests, including `export_core_writes_assets_after_real_runtime_gate` and `build_pack_core_writes_zip_after_real_runtime_gate`. |
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
| AUD-17 | P1 | open | OCR remains a mode flag over pypdf text extraction, not a real OCR/layout adapter. | Implement OCR/layout adapter or define scanned PDF as explicit manual transcription flow. |
| AUD-18 | P1 | open | Packaged app includes sidecar scripts as resources but still depends on host `node`, `python3`, `pypdf`, and external unified runtime env. | Bundle dependencies or add production preflight/setup. |
| AUD-19 | P1 | open | API key file fallback is plaintext app-data storage; Keychain path is good, fallback is not production-grade. | Add Stronghold/encryption or make file fallback dev-only. |
| AUD-20 | P1 | open | Source-review unresolved jobs can still advance through split/edit intermediate states, though publish remains blocked. | Preserve clearer `NeedsHumanReview` semantics or add explicit “continue despite unresolved source review” UX. |
| AUD-21 | P1 | open | Generated HTML is rendered with `dangerouslySetInnerHTML` and Tauri CSP is null. | Add sanitizer/CSP and isolate preview rendering. |
| AUD-22 | P1 | open | LLM suggestion validation is shallow and does not require evidence/source-block provenance for high-confidence suggestions. | Add JSON Schema/patch whitelist/evidence checks. |
| AUD-23 | P2 | open | Command path segments such as `job_id` and `packId` should be validated even if normal UI-generated values are safe. | Add safe ID/path segment validators. |
| AUD-24 | P2 | open | Real-runtime Rust tests skip when env vars are missing, which can hide coverage gaps in CI. | Make skipped coverage explicit in CI/reporting. |
| AUD-25 | P2 | open | Sidecar README still contains stale wording about review-required sample IR. | Update docs to match no-sample production behavior. |

## Audit Update: 2026-05-31 15:18 CST

| ID | Severity | Status | Summary | Follow-up |
|----|----------|--------|---------|-----------|
| AUD-26 | P0 | open | LLM API keys are persisted into `jobs/<job>/cache/llm/*-input-*.json` because cached gateway input includes `apiKey`. | Redesign gateway invocation so secrets are passed without disk persistence; add redaction/no-secret regression tests. |
| AUD-27 | P1 | open | Optional answer-file import is stored but not used by parser/split/answer extraction. | Implement multi-source answer parsing/merge or remove the UI affordance. |
| AUD-28 | P1 | open | Split/answer manual correction UI is mostly read-only despite design requiring manual split and answer alignment. | Build editable split/answer repair workflow using `save_split_adjustments`. |
| AUD-29 | P1 | open | Visible preview iframe is simplified `srcDoc`, not the actual unified reading page. | Add real unified-runtime visual preview or label current view as template preview. |
| AUD-30 | P1 | open | Pack build marks jobs `Published` before all output artifacts are written. | Make Pack build atomic and update statuses only after successful zip/manifest writes. |
| AUD-31 | P1 | open | Failed `run_preview_e2e` does not update job status, allowing stale PreviewReady/ExportReady semantics. | Update status and issue counts on failed E2E. |
| AUD-32 | P1 | open | Source-review fingerprint is too narrow and omits source hash/parser mode/block content. | Expand fingerprint to source/parser/block hashes. |
| AUD-33 | P1 | open | High-confidence LLM suggestions do not require evidence/source-block provenance. | Add schema/evidence validation before auto-apply. |
| AUD-34 | P2 | open | Validator does not enforce numeric question continuity/display uniqueness. | Add continuity and duplicate display-number checks. |
| AUD-35 | P2 | open | UI validation report can have stale `passed/layers` if Node validator is unavailable. | Recompute layers after adding validator-unavailable warning. |
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
| AUD-30 | P1 | fixed | Pack build now validates all jobs, prepares entries, writes zip and Pack artifacts, then marks jobs `Published`; status update no longer happens before artifact creation. | `build_pack_core_writes_zip_after_real_runtime_gate` now asserts zip, pack.json, manifest.js, and exam JS exist before `Published`. |
| AUD-31 | P1 | fixed | Preview/E2E state application now updates failed reports to `ValidationFailed` and refreshes issue counts, preventing stale `PreviewReady`/`ExportReady`. | `failed_runtime_validation_downgrades_stale_export_ready_status` passed. |
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
