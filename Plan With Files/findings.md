# Findings & Decisions

## Requirements
- 基于 `Files/Epic8-Tauri作者端应用详细设计.md` 与 `Files/Epic8-作者端Web导题与组卷器工程设计.md` 开始开发。
- 最终目标是实现 Tauri 本地应用端的全部开发任务。
- 根目录必须建立 `Plan With Files` 文件夹，放置并持续维护 `task_plan.md`、`findings.md`、`progress.md`。
- 需要建立工程追踪记录表，追踪从设计文档拆分出的细分开发任务及实时状态。

## Current State Summary
- 本地 Tauri 作者端主链路已实现并可构建：Job、导入、解析、粗切、Authoring IR、LLM 建议、统一预览、导出、Pack 组卷。
- `Plan With Files/` 已存在并持续更新。
- 最新审计重点已从“是否能跑通”提升为“状态语义是否真实、低置信是否强制人工、发布门禁是否只认真实运行时”。

## High-Value Findings
- 上传/导入必须是用户通过系统对话框显式选择的真实文件；生产导入命令已改为“不可读即失败”，不再静默使用 demo 文件冒充真实上传。
- 自动流水线现在将 parser warnings、低置信块和 LLM 低置信建议统一路由到人工审阅，而不会把模拟 runtime 通过误标成可导出。
- `PreviewReady` 仅代表本地预览/E2E 通过，不再自动进入 Pack 发布候选；`ExportReady` 只在真实 runtime gate 通过时出现。
- LLM fallback 与 dev fallback 的建议置信度已降为 0.64，并显式带有 `fallback-output-never-auto-applies`，避免被自动落库。
- `JobDetail` 已扩展为返回 `pipelineReport` 与全部 `llmSuggestions`，便于审阅页面和任务详情追踪。
- `DocumentReview` 现在展示 parser warnings、低置信 block 与自动流水线摘要，便于人工确认。
- 真实 unified runtime 最小 E2E 已接入并通过；OCR/scanned-PDF 仍是人工审核边界，尚未实现自动 OCR adapter。

## Technical Decisions
| Decision | Rationale |
|----------|-----------|
| `ExportReady` 只允许真实 runtime gate 通过后出现 | 防止 simulator/fallback 误导发布状态 |
| `PreviewReady` 仅表示本地预览/E2E 通过 | 明确区分“可看”和“可发” |
| LLM fallback 一律低置信 | 防止离线/无 key 场景产生虚假的高置信自动落库 |
| 低置信建议按建议 ID 持久化 | 避免 `llm-last-suggestion.json` 的 last-write-wins 问题 |

## Issues Encountered
| Issue | Resolution |
|-------|------------|
| `cargo fmt --check` initially failed | Ran `cargo fmt` and reran Rust checks |
| `cargo clippy` flagged identical `if` branches | Simplified `next_step` branch to a single fallback |
| `git diff --check` required cleanup | Adjusted formatting before final verification |

## Verification Evidence
- `npm run check` passed.
- `npm run build` passed.
- `cargo fmt --check` passed.
- `cargo check` passed.
- `cargo clippy --all-targets -- -D warnings` passed.
- `npm run tauri build` passed and generated `.app` / `.dmg` artifacts.
- `node --check` passed for `sidecars/llm-gateway/gateway.mjs`, `sidecars/preview-e2e/preview-e2e.mjs`, and `sidecars/node-validator/validate-reading-source.mjs`.
- `python3 -m py_compile sidecars/python-parser/parser.py` passed.
- LLM gateway smoke with no API key returned `confidence: 0.64` and included `fallback-output-never-auto-applies`.
- Browser smoke against local Vite preview confirmed:
  - Import page submit disabled without a selected file.
  - Pack page only treats `ExportReady` as publishable.
  - LLM review page renders.
- 真实 unified runtime、复杂 PDF/DOCX 最小 fixture、no-text PDF hard-stop、命令级 export/Pack fixture 已通过；但 OCR/scanned-PDF 自动识别和更广泛端到端 pipeline 仍不足以证明最终生产闭环。

## Remaining Risk
- 真实 unified runtime 最小 E2E、复杂 PDF/DOCX fixture 和 no-text PDF hard-stop 已通过；OCR/scanned-PDF 自动识别仍未接入。
- 复杂扫描 PDF 的布局还不能视为最终完成，仍应保持强制人工审核边界。
- 因此，工程状态应表述为“主链路基本完成，外部统一阅读页最终接入待补齐”，而不是全量完成。

## Deep Audit: 2026-05-31

### Overall Product State
- 当前产品已经达到“可构建、可演示、主链路可串联”的 MVP 原型状态。
- 当前产品尚未达到“复杂 PDF/OCR 可生产发布”的完成状态。
- 最大差距不是页面数量，而是复杂 PDF/OCR 与真实 runtime 的端到端证据；本轮已补 `publish_readiness_gate`，但仍需真实运行时和扫描 PDF fixture 证明。

### Core Business Chain Audit
| Chain | Current Implementation | Audit Result |
|-------|------------------------|--------------|
| 用户选择文件 | `ImportWizard` 要求 `sourceFile`，Tauri dialog 限制 PDF/DOCX/TXT/MD；Rust `import_source_file` 复制到 app data 并计算 SHA256 | 基本符合权限边界 |
| PDF/DOCX 解析 | Python sidecar 使用 `pypdf` / OOXML；复杂清晰 PDF/DOCX fixture 可到 AuthoringIR；无文本 PDF 标 warning + confidence 0.2 | 正常清晰文本 PDF/DOCX 可用，扫描 PDF 只能低置信进入人工审核 |
| parser 失败 | Rust `parse_source_document` 对非 TXT/MD 生成 failure Document IR，不再 fallback 到 sample 内容 | P0 已修复；PDF/DOCX fixture 回归已补 |
| 自动切分 | `make_dynamic_split_candidates` 基于 role/text/Questions 范围推断 passage、题组、答案 | 可用但偏启发式；复杂 PDF/跨页表格/图片题风险高 |
| Authoring IR | `make_dynamic_authoring_ir` 生成 groups/questions/answerKey/order/displayMap，`refresh_authoring_review_state` 派生人工确认状态 | 字段齐全；发布前已接入 readiness gate |
| LLM 建议 | Gateway 输出结构化 JSON patch；fallback confidence=0.64；低置信不可 apply | 边界正确，但真实 provider 返回高 confidence 仍缺 evidence/字段来源强校验 |
| 高置信自动落库 | `run_auto_pipeline` 对 `confidence >= threshold` 自动 apply `kind/layout/questions` | 符合用户期望，但应只允许有 evidence 且不改答案；当前 apply 不改 answer，是正确边界 |
| 低置信人工审核 | `lowConfidenceGroups` 路由 `LlmReview`；parser warnings/low blocks 路由 `DocumentReview`；发布前检查 `humanVerified` | 状态闭环已加强，仍需真实 fixture 验证 |
| 校验与预览 | Rust 内置校验 + Node DOM validator + preview-e2e fallback/real runner | 四层框架存在；外部统一阅读页最小 real-runtime fixture 已通过，仍需更多题型/命令级集成 |
| 导出/Pack | 默认 Rust 静态合同 gate + SourceReview/AuthoringReview；Pack UI 只列 `ExportReady`；真实 runtime E2E 为诊断项 | P0 硬阻断已补齐，且不依赖 host Node/外部 runtime |

### Code Quality Findings
| ID | Severity | Finding | Evidence |
|----|----------|---------|----------|
| CQ-01 | P0 fixed | 发布门禁已统一检查 `NeedsHumanReview`、parser warnings、low-confidence blocks、未验证问题/答案 | `publish_readiness_gate` 已接入 `export_reading_assets` 和 `build_pack` |
| CQ-02 | P0 fixed | parser sidecar 失败时不再对 PDF/DOCX 生成 sample Document IR | `parser_failure_document_ir` 生成 failure IR |
| CQ-03 | P0 fixed | `generate_preview_assets` 已先校验，并在仍需人工审核时保持 `NeedsHumanReview` | 预览状态推进已修正 |
| CQ-04 | P1 fixed | `validate_authoring_ir` now updates `current_step` coherently: unresolved SourceReview issues route to `DocumentReview`, AuthoringIR validation failures stay in `Authoring`, and passing validation becomes `DraftSaved`/`Authoring` rather than implying Preview/runtime readiness. | `validation_job_state_routes_review_and_authoring_steps`, `validate_authoring_state_update_overwrites_stale_current_step`, full `cargo test` passed. |
| CQ-05 | P1 fixed | `choose_export_dir` Rust command now uses `tauri_plugin_dialog::DialogExt` to open a native folder picker and return the selected directory path; frontend plugin remains the primary UI path, but the backend command is no longer a stub. | `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `npm run check` passed. |
| CQ-06 | P1 | `src-tauri/src/lib.rs` 过大，业务边界混杂 | storage/parser/llm/validator/export/pack/pipeline 全部在单文件 |
| CQ-07 | P1 | 没有自动化业务测试/fixture | `find` 未发现 repo 自有 test/spec/fixture |
| CQ-08 | P2 | dev fallback 与真实 Rust 行为不完全一致，浏览器 smoke 不能证明发布门禁 | `src/services/devFallbackBackend.ts` 在浏览器 localStorage 内模拟命令 |

### Field and Contract Audit
- `ReadingExamSourceV1` 必要字段 `schemaVersion`、`examId`、`meta`、`passage.blocks`、`questionGroups`、`answerKey`、`questionOrder`、`questionDisplayMap` 均由确定性代码生成。
- `sourceRefs.primaryProvider` 仍保留 `author_web` 以兼容旧运行时契约；`audit.notes` 标注 `provider:author_tauri`。
- `QuestionDraft.verified`、`QuestionGroupDraft.verified`、`AuthoringAudit.humanVerified` 已接入发布 readiness gate。
- `DocumentBlock.confidence < 0.5` 会被自动流水线识别；导出/Pack 会在未完成人工确认时回查 document-ir 并阻断。
- `LlmSuggestion.confidence < 0.85` 会被后端 `apply_llm_suggestion` 阻断；这是当前较可靠的边界。

### Recommended Next Implementation Order
1. 补 OCR/no-text PDF fixture：验证 no-text PDF 进入 `NeedsHumanReview` 且导出/Pack 失败。
2. 补真实 unified runtime fixture：用环境变量注入外部 html/python，导出和 Pack 通过真实 mode=real 才允许。
3. 拆分 Rust backend 模块并增加自动化测试，避免继续在单文件里叠加业务。

## Implementation Findings: 2026-05-31 13:26 CST

### P0 Audit Fixes Implemented
- 新增 `parser_failure_document_ir`：PDF/DOCX parser sidecar 失败时不再 fallback 到 sample 题内容，而是生成低置信 failure Document IR，带 `no-sample-content-generated` warning。
- 新增 `refresh_authoring_review_state`：从题目/题组 `verified`、confidence、answer 自动派生 `audit.humanVerified` 和 `needsReview`。
- 新增 `publish_readiness_gate`：导出/Pack 前在四层 runtime gate 基础上继续阻断 `NeedsHumanReview`、未人工确认、空答案、低置信未确认、未复核 parser warning/low-confidence blocks。
- `run_preview_e2e` 现在只有真实 runtime 通过且 publish readiness 通过时才把任务置为 `ExportReady`。
- `generate_preview_assets` 不再无条件写 `PreviewReady`；仍需人工审核时保留 `NeedsHumanReview` 状态。
- `run_auto_pipeline` 高置信自动应用后会把对应题组/题目标记 verified，低置信仍进入 LLM Review。
- dev fallback 同步 strict publish readiness 语义，避免浏览器 smoke 继续给出假阳性。

### New Verification Evidence
- `npm run check` passed.
- `npm run build` passed.
- `cargo fmt --check` passed after formatting.
- `cargo test` passed with 3 Rust unit tests:
  - `parser_failure_document_ir_never_uses_sample_content`
  - `refresh_authoring_review_state_requires_low_confidence_verification`
  - `publish_review_issues_block_empty_answers`
- `cargo clippy --all-targets -- -D warnings` passed.
- Sidecar syntax checks passed: `node --check` for LLM/preview/validator and `python3 -m py_compile` for parser.

### Remaining After P0 Fixes
- 真实 unified runtime E2E 仍未完成，因此 `ExportReady` 最终闭环仍需真实外部运行时验证。
- OCR adapter 仍未真正实现；当前是 no-text/low-confidence/manual-review 边界，仍需 fixture 覆盖扫描 PDF。
- Rust 业务逻辑仍集中在 `src-tauri/src/lib.rs`，E8-12 架构拆分仍未开始。

## Deep Architecture and Business Audit: 2026-05-31 13:38 CST

### Scope
- Re-audited the local Tauri authoring app against the Tauri design document and the old Web output-contract document, treating the Web document only as a runtime contract reference.
- Focus areas: architecture boundaries, upload/parse/split/LLM/manual-review/publish chain, state machine, field contracts, parser/OCR complexity, runtime gate, Pack/export safety, dev fallback drift, and test coverage.

### Current Product State
- The app is a functional local MVP prototype: import UI, app-data storage, deterministic parser sidecar, rule split, Authoring IR, LLM suggestion gateway, preview assets, DOM validator, runtime E2E sidecar, export, and Pack builder are present.
- The app is not yet a production-grade complex PDF authoring tool. The remaining hard gaps are scanned/no-text PDF handling, real unified runtime E2E evidence, manual-review provenance, and modular/test architecture.

### New Audit Findings
| ID | Severity | Area | Finding | Evidence | Required Follow-up |
|----|----------|------|---------|----------|--------------------|
| AUD-07 | P0 | Publish gate/manual review | Parser warnings and low-confidence Document IR blocks are blocked only when `audit.humanVerified` is false or job status is `NeedsHumanReview`. If questions are marked verified and `validate_authoring_ir` advances the job to `PreviewReady`, parser/OCR warnings can stop blocking because `publish_readiness_gate` wraps parser warning checks inside `if !human_verified`. | `src-tauri/src/lib.rs` `refresh_authoring_review_state`, `validate_authoring_ir`, `publish_readiness_gate` | Add independent parserReviewResolved/sourceReviewVerified fields and always block unresolved parser warnings/low blocks regardless of authoring humanVerified/job.status. |
| AUD-08 | P0 | State machine API | `update_job_meta` accepts arbitrary `status` and `currentStep` from the frontend command surface. This can mutate workflow state without recomputing validation/readiness. | `src-tauri/src/lib.rs` `JobMetaPatch`, `update_job_meta` | Remove status/currentStep from public metadata patch or enforce legal transition guard with recomputed readiness. |
| AUD-09 | P1 | Parser fallback | `parse_document` and `run_auto_pipeline` still call `sample_document_ir` if there is no main source file, unsupported file type, or missing uploaded file. The import UI blocks common production entry, but backend commands can still synthesize demo content. | `src-tauri/src/lib.rs` `parse_document`, `run_auto_pipeline` | Replace sample fallback in production commands with explicit failure/review-required IR; move sample generation behind a dev-only command/feature flag. |
| AUD-10 | P1 | LLM trust boundary | High-confidence LLM suggestions automatically set group and question `verified=true`, even though design says LLM output must be diff-reviewed and cannot replace human confirmation. | `src-tauri/src/lib.rs` `run_auto_pipeline`, `apply_llm_suggestion` | Separate `autoApplied=true` from `humanVerified=true`; high confidence can auto-apply safe structural patches, but must not create human verification provenance. |
| AUD-11 | P1 | Complex PDF/OCR | OCR mode is only a flag that reruns the same parser path; scanned PDF produces low-confidence placeholder blocks but no OCR adapter or fixture coverage. | `src-tauri/src/lib.rs` `rerun_ocr`; `sidecars/python-parser/parser.py` `parse_pdf` | Add OCR adapter or explicit manual transcription workflow; add scanned/no-text PDF fixture proving Export/Pack block until source review is resolved. |
| AUD-12 | P1 | Output metadata | `reading_source` emits hard-coded `pdfFilename`, `shuiPdf`, and `audit.matchStatus=author_verified` independent of actual source and readiness. | `src-tauri/src/lib.rs` `reading_source` | Use real source metadata and derive output audit status from validated provenance; avoid unconditional author_verified in generated runtime source. |
| AUD-13 | P1 | Architecture | Backend remains a large single file mixing storage, parser, split, LLM, validator, runtime E2E, export, Pack, profile secrets, and tests. | `src-tauri/src/lib.rs` function map | Split modules before adding OCR/complex fixtures to reduce regression risk. |
| AUD-14 | P1 | Test coverage | Business tests cover only three unit cases. There are no repo fixtures for real PDF/DOCX/scanned PDF, no command-level integration tests, and no checked real unified runtime fixture. | `find` for test/spec/fixture; `cargo test` output | Add fixture-driven parser, pipeline, readiness, export, Pack, and real runtime E2E tests. |
| AUD-15 | P2 | Dev fallback drift | Browser dev fallback uses sample data/localStorage and can diverge from Rust readiness semantics. It is useful for UI smoke but not product acceptance evidence. | `src/services/devFallbackBackend.ts` | Keep fallback explicitly dev-only and avoid using it as acceptance evidence. |

### Business Chain Assessment
| Step | Current Behavior | Audit Assessment |
|------|------------------|------------------|
| Upload PDF/DOCX | UI requires system-selected file; Rust copies to app data and hashes content. | Good user-facing boundary; backend still has demo fallbacks if commands are called out of order. |
| Parse/OCR | PDF uses `pypdf`; DOCX uses OOXML; no-text PDF gets warnings and confidence 0.2. | Text PDFs can work; scanned PDFs require manual path or OCR implementation. |
| Rule split | Regex/roleHint based detection of passage, question ranges, answer blocks. | Acceptable MVP heuristic; brittle for multi-column PDFs, tables, images, answer sheets, and irregular numbering. |
| LLM | JSON-only gateway, fallback low confidence, suggestions persisted. | Good baseline; high-confidence auto-apply currently conflates model confidence with human verification. |
| Manual review | Question-level checkbox exists; parser warnings visible in DocumentReview; `SourceReviewV1` records explicit source-review resolution. | Parser/source-review provenance is now separated from authoring verification; still needs command-level fixture coverage. |
| Validation | Authoring, source schema, DOM, runtime preview layers exist. | Layering is correct; real runtime path still lacks completed fixture evidence. |
| Export/Pack | Strict runtime gate and publish readiness gate exist. | Parser warning bypass has been fixed with `SourceReviewV1`; command-level export/Pack real-runtime core tests now pass. |

### Verification During This Audit
- `npm run check` passed.
- `cargo test` passed with 3 Rust tests.
- Sidecar syntax checks passed for LLM gateway, preview E2E, node validator, and Python parser.
- `git diff --check` passed.
- Full `npm run build`, `cargo clippy`, `npm run tauri build`, real unified runtime E2E, and scanned PDF fixture were not rerun in this audit pass.

## Implementation Findings: 2026-05-31 14:04 CST

### P0/P1 Audit Fixes Implemented
- Added `SourceReviewV1` as an independent parser/source review gate. Parser warnings and low-confidence Document IR blocks now remain publish blockers until `resolve_source_review` records explicit resolution; this is no longer tied to `audit.humanVerified`.
- Added `resolve_source_review` Tauri command and DocumentReview UI action for manual source review completion.
- Removed public workflow mutation from `JobMetaPatch`; `update_job_meta` no longer accepts or applies `status` / `currentStep` from frontend metadata updates.
- Production parse and auto-pipeline paths no longer fall back to demo `sample_document_ir` for missing/unsupported main source states. They now emit a low-confidence `source-missing` Document IR with `no-sample-content-generated` warning.
- LLM high-confidence application no longer marks groups/questions as `verified=true`. It records `autoApplied` and the suggestion id, keeping human verification as a separate manual action.
- Dev fallback was updated to expose `sourceReview`, ignore status/currentStep metadata patches, require source review in publish readiness, and avoid treating LLM apply as human verification.

### New Verification Evidence
- `npm run check` passed.
- `npm run build` passed.
- `cargo fmt --check` passed.
- `cargo test` passed with 6 Rust tests:
  - `parser_failure_document_ir_never_uses_sample_content`
  - `missing_source_document_ir_never_uses_sample_content`
  - `source_review_issues_block_even_when_authoring_is_human_verified`
  - `auto_applied_llm_patch_does_not_create_human_verification`
  - `refresh_authoring_review_state_requires_low_confidence_verification`
  - `publish_review_issues_block_empty_answers`
- `cargo clippy --all-targets -- -D warnings` passed.
- Sidecar syntax checks passed for LLM gateway, preview E2E, node validator, and Python parser.
- `git diff --check` passed.

### Remaining Risk After This Fix
- Real unified runtime minimal E2E is now proven with external unified HTML/Python paths; more question-type fixtures are still needed.
- OCR is still not a true OCR adapter; no-text PDF now has fixture-backed manual-review blocking, but not automatic OCR recognition.
- Rust backend remains monolithic and still needs E8-12 module decomposition.


## Implementation Findings: 2026-05-31 14:30 CST

### Fixture and Runtime Evidence Added
- Added `fixtures/parser/no-text.pdf`, a real blank PDF fixture generated with `pypdf`; parser sidecar returns `python-parser-sidecar:pdf:pypdf`, warning `page 1 has no extractable text`, and block confidence `0.2`.
- Added Rust tests increasing backend tests from 6 to 9:
  - `no_text_pdf_fixture_requires_source_review`
  - `reading_source_uses_real_source_metadata_and_review_status`
- Updated output source metadata so `ReadingExamSourceV1.meta.pdfFilename`, `sourceRefs.shuiPdf`, `audit.matchStatus`, and `audit.notes` derive from imported source provenance and human verification state.
- Fixed `sidecars/preview-e2e/preview-e2e.mjs` to preserve structured real-runtime failures instead of hiding them with fallback simulator, and fixed radio wrong-answer generation.
- External unified runtime minimal E2E now passes: `runtime.mode=real`, correct-answer score `100%`, wrong-answer sample `50%`, report at `/tmp/epic8-real-runtime-report.json`.

### Current Remaining Risk
- no-text/scanned PDF is safe by hard-stop/manual review, not solved by OCR. If product scope requires automatic OCR recognition, an OCR adapter and scanned-image fixture remain required.
- Export/Pack command-level real-runtime core fixtures now pass; broader complex PDF/DOCX and full pipeline fixtures remain missing.
- Rust backend is still monolithic, so architecture work remains a P1 maintainability item.


## Implementation Findings: 2026-05-31 14:45 CST

### Command-Level Export/Pack Evidence
- Extracted `export_reading_assets_core` and `build_pack_core` to make publish commands testable without Tauri `AppHandle` while preserving command behavior.
- Added real-runtime tests proving export writes JSON/JS/manifest/report and marks job `ExportReady`; Pack writes zip/manifest and marks job `Published`.
- Fixed a strict gate bug: merged preview E2E reports were dropping the `runtime` object, causing strict gate to treat real runtime as `unknown`.

### Remaining Risk
- Complex PDF/DOCX minimal fixtures now pass; OCR adapter decision remains open.
- Rust backend is still monolithic; command core extraction is only a first seam, not full architecture decomposition.


## Implementation Findings: 2026-05-31 15:00 CST

### Complex Parser Fixtures
- Added `fixtures/parser/complex-reading.pdf`, a clear text PDF with passage, two question groups, a table-like section, and answers.
- Added `fixtures/parser/complex-reading.docx`, a minimal OOXML Word file with paragraphs, a table, and answers.
- Added Rust tests proving both fixtures parse without warnings/low-confidence blocks and reach split + AuthoringIR with 5 questions and expected answer key values.

### Parser Limitation Noted
- The PDF text extractor may merge visual table text into adjacent prose (`pypdf` behavior). Current deterministic parser still recovers answer blocks and question groups for the fixture, but complex layouts with images/flowcharts/cross-page tables remain manual-review territory unless OCR/layout tooling is added.

## Deep Implementation Audit: 2026-05-31 15:40 CST

### Audit Scope
- Re-audited the current local Tauri implementation against `Epic8-Tauri作者端应用详细设计.md`, using the old Web document only as the output/runtime contract reference.
- Focused on architecture, field contracts, source provenance, PDF upload edge cases, parser/OCR behavior, LLM trust boundaries, state machine semantics, export/Pack gates, packaging, and test evidence.

### Current Product State
- The local authoring app is a working MVP with a strong publish safety baseline: upload/import, parser sidecar, split, Authoring IR, LLM suggestions, validation, preview assets, real-runtime E2E gate, export, and Pack are implemented.
- It is not yet production-complete for the full Epic 8 local app target because scanned/OCR PDFs, modular backend architecture, bundled dependency/runtime independence, and fine-grained audit provenance remain incomplete.

### Detailed Findings
| ID | Severity | Area | Finding | Evidence | Recommendation |
|----|----------|------|---------|----------|----------------|
| AUD-16 | P1 | Architecture | Rust backend remains monolithic and untyped at core domain boundaries; `src-tauri/src/lib.rs` is ~4921 lines and passes most business records as `serde_json::Value`. | `src-tauri/src/lib.rs`; `JobDetail.document_ir/source_review/authoring_ir/validation_report` are `Value`. | Split modules and introduce typed structs for `DocumentIRV1`, `SourceReviewV1`, `ReadingAuthoringIRV1`, validation reports, pack input/output. |
| AUD-17 | P1 | OCR/PDF | `rerun_ocr` is still only `parse_document(mode=ocr)`; Python parser still uses pypdf text extraction and does not perform OCR or layout reconstruction. | `rerun_ocr` delegates to `parse_document`; `sidecars/python-parser/parser.py` `parse_pdf` calls `page.extract_text()`. | Decide product policy: either implement OCR/layout adapter, or rename UI to manual-source review and remove false OCR expectations. |
| AUD-18 | P1 | Packaging | Release bundle includes sidecar scripts as resources, but execution uses host `node`, `python3`, `pypdf`, and external Playwright runtime path. The packaged app is not self-contained. | `tauri.conf.json` has `resources: ["../sidecars"]`, `externalBin: []`; Rust uses `Command::new("node")` and `Command::new("python3")`. | Bundle sidecar runtimes or preflight required dependencies with actionable setup UI before claiming production installability. |
| AUD-19 | P1 | Security | API key fallback storage writes plaintext key files under app data when Keychain fails. UI discloses fallback, but there is no encryption/Stronghold equivalent. | `file_save_secret` writes `config/secrets/*.key`; design mentions Keychain/Credential Manager/Stronghold-style secure storage. | Treat file fallback as dev/emergency only or encrypt with Stronghold/OS credential manager before production. |
| AUD-20 | P1 | State semantics | `run_rule_split` and `build_authoring_ir` can continue even when source review is unresolved. Final publish gate blocks this, but intermediate status may become `SplitReady`/`AuthoringReady` briefly and can confuse operators. | `run_rule_split` unconditionally sets `SplitReady`; `build_authoring_ir` rechecks source review. | Allow continuation only with explicit “continue despite unresolved source review” UI state, or preserve `NeedsHumanReview` until source review is resolved. |
| AUD-21 | P1 | UI security | `GroupEditor` renders generated group HTML with `dangerouslySetInnerHTML`, and Tauri CSP is set to `null`. Current renderer escapes most user fields, but Authoring IR can be edited and stored as raw JSON. | `src/pages/GroupEditor.tsx`; `src-tauri/tauri.conf.json` `csp: null`. | Add sanitizer/CSP and restrict preview rendering surface; never render arbitrary imported/LLM HTML in privileged webview. |
| AUD-22 | P1 | LLM validation | LLM gateway validates top-level kind/confidence/array fields only; it does not schema-validate each patch path/value or enforce evidence/source-block provenance. | `sidecars/llm-gateway/gateway.mjs` `validateSuggestion`. | Add JSON Schema validation for suggestion shape, allowed patch whitelist, evidence requirements, and reject high-confidence suggestions without evidence. |
| AUD-23 | P2 | File/path safety | `job_dir(root, job_id)` and `packs/<packId>` use caller-controlled IDs/pack IDs as path segments. UUID-created jobs are safe in normal flow, but command surfaces should still sanitize. | `job_dir(root, job_id)`, `build_pack_core` pack path. | Use validated ID types or safe path segment checks for every command input. |
| AUD-24 | P2 | Test hygiene | Business tests are useful but live inside the 4921-line production file, and real-runtime tests silently skip if env vars are unavailable. | test module in `src-tauri/src/lib.rs`; `external_runtime_available()` returns early. | Move integration tests to dedicated files and make CI expose skipped real-runtime coverage explicitly. |
| AUD-25 | P2 | Docs drift | `sidecars/README.md` still says parser can fall back to a “review-required sample IR”, which conflicts with the current no-sample production behavior. | `sidecars/README.md` Rust command integration section. | Update sidecar docs to match current implementation. |

### Core Business Chain Assessment
| Step | Current Behavior | Audit Result |
|------|------------------|--------------|
| User uploads PDF/DOCX | UI requires explicit system file selection; Rust copies file bytes into app data and records hash/source metadata. | Good baseline. Backend command still trusts `file_path` from the front-end command surface, which is acceptable for a local trusted app but should be documented. |
| Automatic parse | TXT/MD/PDF/DOCX parse into Document IR; clear PDF/DOCX fixtures pass; no-text PDF becomes low-confidence + source review required. | Safe for clear text documents; not sufficient for scanned/image PDFs. |
| Automatic split | Heuristic rules infer passage, groups, answers from Document IR roles and regex. | Good MVP heuristic; brittle for multi-column, cross-page tables, diagrams, answer sheets, and image-heavy PDFs. |
| LLM recognition | Structured JSON suggestions only; fallback confidence is below auto-apply threshold; high-confidence apply records `autoApplied` not `verified`. | Trust boundary is much better than before; still needs stronger schema/evidence validation. |
| High-confidence auto apply | Applies only whitelisted structure fields (`kind`, layout template, prompts/interactions), not answer values. | Reasonable and aligned with “high confidence can auto-apply safe structure, not facts.” |
| Low-confidence/manual review | Low-confidence LLM suggestions and source parser issues route to review. Publish gate requires `SourceReviewV1.resolved` and `audit.humanVerified`. | Correct hard-gate behavior now exists. UX still needs clearer warnings if user continues editing before resolving source review. |
| Preview/runtime | Rust built-in validation is authoritative; Node validator and runtime E2E are explicit diagnostics only; export/Pack default to Rust static contract mode. | Strong safety baseline without production Node/external runtime dependency. |
| Export/Pack | Export and Pack call shared core functions and publish readiness gate. | Backend publish safety is currently the strongest part of the implementation. |

### Verification Refreshed This Audit
- `npm run check`: pass.
- `npm run build`: pass.
- `cargo fmt --check && cargo clippy --all-targets -- -D warnings`: pass.
- `cargo test` with `EPIC8_UNIFIED_HTML_PATH` and `EPIC8_UNIFIED_PYTHON`: pass, 13 tests.
- Sidecar syntax checks and `python3 -m py_compile`: pass.
- `git diff --check`: pass.

### Completion Judgment
- Status should remain `active/in_progress`.
- The current product is safe enough for MVP demonstrations on clear text PDF/DOCX and controlled local runtime environments.
- The current product is not yet safe to label as full Epic 8 local-app completion because OCR/scanned PDF, dependency bundling, backend modularization, and stricter schema/provenance enforcement are still open.

## Deep Detail Audit: 2026-05-31 15:18 CST

### Scope
- Re-audited implementation at function/page/field level after the previous 15:40 audit, with special focus on API-key handling, command surfaces, source-review provenance, manual correction UX, runtime preview fidelity, and Pack/export state consistency.
- This pass did not change product code; it updates audit records and verification evidence only.

### New Findings
| ID | Severity | Area | Finding | Evidence | Recommendation |
|----|----------|------|---------|----------|----------------|
| AUD-26 | P0 | Secret handling | LLM API keys are loaded from Keychain/file storage and then written into job cache input JSON before invoking the Node gateway. This means Keychain-protected secrets can still be persisted in plaintext under `jobs/<job>/cache/llm/*-input-*.json`; `test_llm_profile` can also create `jobs/profile-test/cache/llm` with an API key. | `make_llm_input` inserts `apiKey`; `run_llm_gateway` writes the full input to `input_path`; `gateway.mjs` reads `input.apiKey`. | Never write API keys to disk. Pass secrets via environment variable/stdin pipe or redact before cache write; add a regression test that no cache/log file contains the API key. |
| AUD-27 | P1 | Answer-file chain | Import UI accepts an optional answer file and Rust stores it as `AnswerKey`, but parser/split only reads the `MainQuestion` source. Separate answer PDFs/DOCX/TXT currently do not affect `answerKeyCandidates`. | `ImportWizard` imports `AnswerKey`; `main_source_file` filters `MainQuestion`; `parse_document` parses only main source; split derives answers only from one `DocumentIRV1`. | Add multi-source parsing/merge for AnswerKey sources or remove/disable the answer-file UI until implemented. |
| AUD-28 | P1 | Manual correction UX | The split/answer page is mostly read-only. It displays passage candidates, group candidates, and answer candidates, but does not expose editing/saving for ranges, answer key corrections, group add/remove, or low-confidence split repair. | `SplitAndAnswers.tsx` calls `runRuleSplit`/`buildAuthoringIr` and only renders candidates; `saveSplitAdjustments` exists in API but is not used by the page. | Implement editable split/answer UI as the required human intervention surface for parser/LLM failures. |
| AUD-29 | P1 | Runtime preview UX | The visible `UnifiedPreview` iframe is not the real unified reading page; it renders a simplified `srcDoc` built from passage blocks and group HTML. Real-runtime E2E runs in a sidecar, but the operator does not visually review that actual runtime. | `UnifiedPreview.tsx` `buildSrcDoc`; `preview_assets_for_source` has `tauri-local://preview/...` but UI does not load external unified HTML. | Add an actual unified-runtime preview surface or clearly label current iframe as template preview only. |
| AUD-30 | P1 | Pack atomicity | `build_pack_core` marks each job `Published` inside the loop before manifest/zip creation completes. If later file writes or zip creation fail, job state can say Published without a valid Pack artifact. | `build_pack_core` updates job status before `build_manifest`, `write_text(pack.json)`, and `write_zip`. | Make Pack build atomic: write to temp dir/zip, verify outputs, then update all job statuses in one final commit step. |
| AUD-31 | P1 | State semantics | `run_preview_e2e` updates status only when the report passes. If E2E fails after a previous PreviewReady state, job status can remain stale even though the latest validation report failed. | `run_preview_e2e` only calls `update_job` inside `if report.passed`. | On failed E2E, set `ValidationFailed` or `NeedsHumanReview` with current step `Preview` and update issue counts. |
| AUD-32 | P1 | Source review provenance | Source-review fingerprint includes only parser warnings and low-confidence block IDs. It does not include source file hash, parser provider/version/mode, page/block text hashes, or answer-source provenance. A resolved review can become stale too narrowly. | `source_review_fingerprint` hashes `{parserWarnings, lowConfidenceBlocks}` only. | Include source file hash, parser provider/version/mode, low-confidence block text hashes, and source stored name in the fingerprint. |
| AUD-33 | P1 | LLM schema/evidence | The gateway accepts high-confidence suggestions without requiring evidence quotes, source block IDs, or JSON-schema validation for each patch path/value. This weakens the high-confidence auto-apply boundary. | `gateway.mjs` `validateSuggestion` only checks object/kind/confidence/array fields; Rust applies whitelisted paths but does not require evidence for high confidence. | Add versioned JSON Schema and reject high-confidence output unless evidence references source blocks. |
| AUD-34 | P2 | Validator completeness | Reading source validator checks coverage and DOM controls, but does not enforce numeric question continuity/display uniqueness from the design contract. | `validate-reading-source.mjs` validates `questionOrder` length/coverage but not contiguous display numbers or duplicate display labels. | Add explicit checks for contiguous IELTS question numbering, duplicate `questionDisplayMap` values, and answer key continuity. |
| AUD-35 | P2 | Validation reporting | `validate_authoring_ir` treats Node validator unavailability as a warning pushed into `issues` after layers/passed are computed, so UI-level validation can report stale `passed/layers`. Publish path is stricter, but the review page can mislead. | `validate_authoring_ir` appends warning to `report.issues` without recomputing `passed`/`layers`. | Recompute validation layers after any issue mutation or reuse `merge_validation_issues`. |
| AUD-36 | P2 | Provider support | Settings offers `AnthropicCompatible`, `Ollama`, and `Custom`, but the sidecar gateway always calls an OpenAI-compatible `/chat/completions` endpoint. | `Settings.tsx` provider options; `gateway.mjs` `callOpenAiCompatible`. | Either implement provider adapters or label all non-OpenAI-compatible options as not yet supported. |
| AUD-37 | P2 | Low-confidence LLM UX | Low-confidence suggestions cannot be applied even after a human reviews the diff; the user must manually copy edits in GroupEditor. Safety is acceptable, but the diff-review workflow is incomplete. | `LlmReview.tsx` disables apply under 0.85; `apply_llm_suggestion` rejects confidence < 0.85. | Add an explicit human-reviewed apply path that records manual approval provenance without marking model output as verified automatically. |
| AUD-38 | P2 | Docs drift | Sidecar README still says parser may fall back to a review-required sample IR, but production Rust now emits failure/missing-source IR and no sample content. | `sidecars/README.md` line describing parser fallback. | Update README to avoid misleading future development and QA. |

### Updated Product Judgment
- The backend publish gate remains strong: unresolved source review, unverified authoring, non-real runtime, empty answers, parser failures, and low-confidence fields are blocked at export/Pack time.
- The biggest newly identified blocker is secret leakage into LLM cache files. This should be fixed before any real API-key usage beyond local testing.
- The PDF flow is safe for clear text PDFs and no-text PDFs are hard-stopped, but the product still lacks true OCR/layout reconstruction and lacks a complete manual correction surface for split/answer repair.
- The user-facing preview is not yet a true visual preview of the external unified runtime, even though real-runtime E2E validation can pass in tests.

### Verification Evidence Refreshed
- `npm run check`: pass.
- `npm run build`: pass.
- `cargo test` with external unified runtime env: pass, 13 tests.
- `cargo fmt --check && cargo clippy --all-targets -- -D warnings`: pass.
- Sidecar syntax checks and Python compile: pass.
- `git diff --check`: pass.

## Implementation Findings: 2026-05-31 15:59 CST

### Fixed in This Pass
- LLM secret persistence is fixed at the command boundary: `make_llm_input` no longer includes `apiKey`, `run_llm_gateway` writes only `redact_llm_input_for_cache(input)`, and the Node gateway reads `EPIC8_LLM_API_KEY` from the environment.
- Fallback LLM output remains deliberately low confidence (`0.64`) and carries `fallback-output-never-auto-applies`; this preserves the high-confidence auto-apply boundary.
- `run_preview_e2e` state semantics are no longer stale: failed validation now downgrades jobs to `ValidationFailed` and refreshes error/warning counts.
- Pack building is safer: job status changes to `Published` only after validation, zip creation, `pack.json`, `manifest.js`, and exam JS files have been written.
- `SourceReviewV1` stale detection is stronger: parser provider/mode/source identifiers and low-confidence block text hashes now participate in the fingerprint.
- Authoring validation now catches question numbering gaps and duplicate display numbers, reducing the risk of answerKey/DOM/display drift.
- Validation report consistency is improved: Node-validator-unavailable warnings are merged through the same issue/layer recomputation helper instead of mutating `issues` after `passed/layers` were computed.
- Settings no longer lists unsupported providers; sidecar documentation now matches the no-sample production behavior.

### Remaining Detailed Risks
- `AUD-27` and `AUD-28` remain the biggest product-flow gap for user-uploaded PDFs: optional answer files are stored but not merged into parsing/splitting, and the split/answer page is still mostly read-only.
- `AUD-17` remains open: no real OCR/layout adapter exists. No-text/scanned PDFs are safe by hard stop and manual review, not automatically solved.
- `AUD-22`/`AUD-33` remain open: high-confidence LLM suggestions still need schema/evidence/source-block provenance validation before the auto-apply path should be trusted for production.
- `AUD-18`/`AUD-19` remain open: packaged app is not fully self-contained and plaintext file secret fallback is still not production-grade.
- `AUD-16` remains open: `src-tauri/src/lib.rs` is still monolithic and JSON-heavy despite more tests.

### Verification Evidence
- `npm run check`: pass.
- `npm run build`: pass.
- `cargo fmt --check && cargo clippy --all-targets -- -D warnings`: pass.
- `cargo test`: pass, 18 tests.
- Sidecar checks: `node --check` for LLM gateway, preview E2E, node validator; `python3 -m py_compile` for parser all pass.
- `git diff --check`: pass.

## Implementation Findings: 2026-05-31 16:17 CST

### Fixed/Improved in This Pass
- Answer-key uploads are no longer dead metadata. Rust now detects `SourceFile.role == "AnswerKey"`, parses each answer source through the same parser sidecar path, extracts answer mappings, and appends them to `answerKeyCandidates`.
- `run_rule_split` and `run_auto_pipeline` both merge external answer candidates, so the automatic PDF flow and manual split flow share the same answer-file behavior.
- The split/answer repair UI now supports editing detected question group metadata and answer values, persists edits through `save_split_adjustments`, and saves before generating AuthoringIR.
- Dev fallback now mirrors answer-source merging enough for browser development, while the authoritative behavior remains the Rust backend.

### Remaining Detailed Risks
- `AUD-27` is improved but not fully complete for all answer-file formats/layouts: separate answer PDFs/DOCX still depend on deterministic text extraction, not OCR/layout semantics.
- `AUD-28` is improved but still not a full visual block-range editor: users can edit block ID lists and answer values, but not drag/select PDF blocks visually.
- OCR/scanned PDFs remain unresolved; no-text PDF is still hard-stopped/manual review.

### Verification Evidence
- `npm run check`: pass.
- `npm run build`: pass.
- `cargo fmt --check && cargo clippy --all-targets -- -D warnings`: pass.
- `cargo test`: pass, 19 tests.
- Sidecar checks: `node --check` for LLM gateway, preview E2E, node validator; `python3 -m py_compile` for parser all pass.
- `git diff --check`: pass.

## Implementation Findings: 2026-05-31 16:37 CST

### Fixed/Improved in This Pass
- Added a manual transcription fallback for scanned/no-text PDFs. This avoids a dead-end when `pypdf` cannot extract text: the operator can paste verified text and continue into split, answer alignment, LLM review, validation, and publish gates.
- The new Rust command writes `manual-transcription.txt`, generates `DocumentIRV1` through `manual_transcription_document_ir`, marks parser provider as `manual-transcription`, and resolves source review with an explicit note.
- `DocumentReview` now exposes the manual transcription UI with warning copy that current OCR is not an independent OCR engine.
- Dev fallback implements the same command for browser-mode testing.

### Remaining Detailed Risks
- This is a deliberate manual-transcription policy, not automatic OCR. If the product requires automatic scanned-PDF recognition, a real OCR/layout adapter remains open.
- Manual transcription still depends on operator accuracy; publish safety relies on later human verification and existing source-review/audit gates.

### Verification Evidence
- `npm run check`: pass.
- `npm run build`: pass.
- `cargo fmt --check && cargo clippy --all-targets -- -D warnings`: pass.
- `cargo test`: pass, 20 tests.
- Sidecar checks: `node --check` for LLM gateway, preview E2E, node validator; `python3 -m py_compile` for parser all pass.
- `git diff --check`: pass.

## Implementation Findings: 2026-05-31 16:58 CST

### Vision LLM Transcription Policy
- The product direction is now: text PDF/DOCX use deterministic local parsing; image-only/no-text PDFs first try visual LLM transcription; failed vision extraction/transcription falls back to operator-provided manual transcription.
- This avoids bundling a heavyweight local OCR engine while still allowing a mostly automatic upload flow for common scanned PDFs that contain extractable embedded page images.
- Vision transcription is treated as source-document extraction, not trusted authoring verification. The generated `DocumentIRV1` always carries parser warnings and unresolved `SourceReviewV1`, so export/Pack remain blocked until a human verifies the source.
- The LLM gateway returns empty transcription on provider failure or missing images. Rust rejects empty vision output, preventing fake reading content from entering production jobs.

### Implementation Notes
- `sidecars/python-parser/parser.py extract_pdf_images` extracts PDF page images via `pypdf` and writes `PdfImageExtractionV1` metadata plus image assets.
- `sidecars/llm-gateway/gateway.mjs transcribe_pdf_images` sends extracted images to an OpenAI-compatible vision chat completions endpoint and validates `{ text, confidence, warnings, evidence }` JSON.
- `apply_vision_transcription` lets the UI manually retry vision transcription; `run_auto_pipeline` invokes it automatically when the first parser pass detects no-text/low-confidence PDF output and an enabled LLM profile exists.
- `DocumentReview` now exposes both “视觉 LLM 转录” and manual transcription fallback.

### Remaining Risk
- Some scanned PDFs may render pages without exposing simple embedded images via `pypdf`. Those still require manual transcription or a future rendered-page adapter using a bundled renderer.
- Vision transcription quality depends on the configured model. Existing source-review and authoring-review gates are mandatory and should not be loosened.

## Implementation Findings: 2026-05-31 17:16 CST

### LLM Auto-Apply Hardening
- High-confidence no longer means auto-apply by itself. Rust now validates the suggestion against the current AuthoringIR before any automatic patch is applied.
- Required evidence for auto-apply: provider evidence must not be fallback/heuristic, `evidence.sourceBlockIds` must be present and all IDs must belong to the current group, and `evidence.quotes` must include non-empty excerpts tied to those group source blocks.
- Schema/patch constraints now block unsupported patch ops/paths, invalid group kinds, unknown question IDs, invalid interaction types, and missing options for radio/checkbox/select interactions.
- `run_auto_pipeline` saves blocked suggestions for human review, adds them to `blockedAutoApplyGroups`, and routes the job to `LlmReview` instead of silently applying or dropping them.
- `apply_llm_suggestion` uses the same backend gate, so manually applying a high-confidence but evidence-deficient suggestion fails with `llm_suggestion_auto_apply_blocked:...`.

### Remaining Risk
- This is still implemented with JSON `Value` in the monolithic Rust backend. Typed domain modules remain important for long-term maintainability.
- Provider-specific JSON Schema validation in the Node gateway is improved but not authoritative; Rust remains the source of truth for safety.

## Implementation Findings: 2026-05-31 17:34 CST

### Environment Preflight
- Added a production-readiness preflight layer because the packaged app currently depends on host `node`, `python3`, `pypdf`, bundled sidecar scripts, and external unified runtime env vars.
- The backend now reports `EnvironmentPreflightV1` with per-check `ok`, `severity`, `message`, and details for `node`, `python3`, `python:pypdf`, each sidecar, `EPIC8_UNIFIED_HTML_PATH`, `EPIC8_UNIFIED_PYTHON`, and strict runtime gate status.
- Settings now shows the preflight result and supports manual rerun, so users can diagnose missing dependencies before import/export/Pack.

### Remaining Risk
- This reduces support/debug risk but is not full dependency self-containment. A production installer should still bundle Node/Python/pypdf or run a signed setup step.
- Missing real unified runtime remains a warning in preflight and only affects explicit diagnostics; export/Pack use Rust static contract gates.

## Implementation Findings: 2026-05-31 17:55 CST

### Secret Fallback Hardening
- API keys no longer fall back to plaintext app-data files by default when Keychain fails.
- `save_profile_secret` now returns an error if Keychain/OS secure storage is unavailable and `EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK` is not explicitly enabled.
- `load_profile_secret` and profile redaction ignore legacy plaintext secret files unless the same opt-in environment variable is set.
- Environment preflight now includes `security:plaintext-secret-fallback`, warning when plaintext fallback is enabled.
- Settings copy now states that plaintext fallback is disabled by default and is only for dev/emergency use.

### Remaining Risk
- This is safer than plaintext fallback, but non-macOS secure storage is still not implemented. On non-macOS, users need a future OS credential manager/Stronghold adapter or the explicit dev fallback.

## Implementation Findings: 2026-05-31 18:24 CST

### Path and Webview Hardening
- Added backend path-segment validation for externally supplied identifiers that touch local filesystem paths: `jobId`, `packId`, `profileId`, and export `examId`.
- Unsafe identifiers such as `../evil`, nested paths, empty values, and whitespace-containing segments are now rejected at command/core boundaries instead of being sanitized into another real object id.
- `job_dir` and `secret_path` now also include defense-in-depth invalid-segment fallbacks so missed validation cannot become path traversal.
- Export wrapper/manifest generation now validates `examId` and JSON-escapes the registry key before inserting it into JavaScript, closing the edited-AuthoringIR asset-path/script-literal risk.
- Group body preview no longer uses `dangerouslySetInnerHTML` in the privileged React webview. It renders inside a sandboxed iframe; UnifiedPreview iframe is also sandboxed.
- Tauri CSP is no longer `null`; it now uses an explicit local-app policy with constrained script, style, font, image, frame, and IPC/connect sources.

### Remaining Risk
- `AUD-16` is still open: the Rust backend remains monolithic and JSON-heavy. The path helpers reduce filesystem risk but do not solve long-term maintainability.
- The visual preview iframe is safer, but it is still the simplified local template preview. Real unified runtime compatibility is enforced by the sidecar E2E gate, not by the visible iframe UI.
- CSP should be rechecked when integrating any future remote assets or a real embedded unified runtime UI.

## Implementation Findings: 2026-05-31 18:31 CST

### Preview Semantics Clarification
- UnifiedPreview now explicitly labels the visible iframe as an isolated local template preview, not proof that the production unified runtime UI has rendered.
- The page displays the latest `runtime.mode` from the RuntimePreview E2E report and states that export/Pack use Rust static contract gates while real runtime checks are diagnostic.
- This reduces operator confusion while preserving safety: export/Pack depend on Rust static contract evidence and human review gates; real runtime sidecar evidence is supplemental.

### Remaining Risk
- `AUD-29` is clarified, not fully solved. A future pass should embed or launch the actual unified runtime UI for visual inspection if product requirements demand visual parity, but the current publish safety gate already tests real runtime compatibility.

## Product Design Update: 2026-05-31 18:34 CST

### Latest Lifecycle and Storage Decision
- MVP remains a local Tauri conversion workbench, not a full SQL-backed question-bank manager.
- SQL is not needed for the current MVP because the primary user task is converting PDF/DOCX into editable structured drafts and exports, not long-term management of thousands of exams.
- The long-term retained artifact should be the editable structured draft (`authoring-project.json` or equivalent `ReadingAuthoringIRV1`) plus metadata/review/export summaries.
- Original PDF/DOCX copies and intermediate process files should be treated as transient working artifacts and deleted automatically after successful export/Pack.
- Developer/debug retention is still useful, but must be behind a default-off Settings -> Developer/Diagnostics option.

### Latest Lifecycle
| State | Meaning | Retention Policy |
|-------|---------|------------------|
| `Working` | Import, parse, split, LLM recognition, draft generation are in progress. | Process files may exist temporarily. |
| `NeedsReview` | Low confidence, image PDF transcription, blocked LLM suggestion, or validation issue requires user intervention. | Keep enough artifacts for review. |
| `DraftSaved` | Editable structured draft exists and can be reopened. | Keep minimal draft and summaries. |
| `ExportReady` | Validation, DOM, and runtime gates pass. | Ready to export. |
| `Exported` | JS/manifest or Pack successfully generated. | Record export summary. |
| `Cleaned` | Transient files have been automatically removed. | Keep only editable draft, metadata, source summary, review summary, export summary. |

### PDF Parser Dependency Research
- Tauri v2 supports bundling external binaries through sidecars and including additional files as resources, which is the correct packaging mechanism if future parser/validator binaries must be self-contained: https://v2.tauri.app/develop/sidecar/ and https://v2.tauri.app/develop/resources/.
- `pypdf` is suitable for extracting an existing PDF text layer, but it is not OCR and cannot extract text from images; scanned/image PDFs require OCR or a visual transcription path: https://pypdf.readthedocs.io/en/3.17.0/user/extract-text.html.
- `pdf-extract` is a lightweight Rust candidate for text-layer extraction and may reduce reliance on host Python/pypdf, but it does not solve image OCR by itself: https://docs.rs/crate/pdf-extract/latest.
- `pdfium-render` can render pages and extract text/images through PDFium bindings, making it a plausible future adapter for rendering scanned pages to images before sending them to a vision LLM: https://docs.rs/pdfium-render/latest/pdfium_render/.
- MuPDF/PyMuPDF is not a default MVP dependency because MuPDF is AGPL/commercial licensed, which creates distribution obligations for closed-source products: https://mupdf.readthedocs.io/en/1.26.9/license.html.

### Packaging Finding
- Current local macOS build output is small (`.app` about 11 MB, `.dmg` about 3.5 MB) because Node/Python/pypdf are still host dependencies.
- Bundling full Node, Python, local OCR engines, or document-intelligence frameworks would significantly increase package size and maintenance surface.
- Recommended direction: lightweight Rust text extraction for text PDFs; vision LLM for image PDFs; optional PDFium page-render adapter only if needed; no default heavyweight local OCR bundle.

### Rust Backend Structure Decision
- `src-tauri/src/lib.rs` being monolithic is acceptable for the current MVP stabilization phase.
- Module splitting remains a later engineering improvement after lifecycle, cleanup, dependency packaging, and production gates are stable.


## Implementation Findings: 2026-05-31 19:06 CST

### Lifecycle and Cleanup Implementation
- Product-facing job status now uses the latest lifecycle: `Working`, `NeedsReview`, `DraftSaved`, `ExportReady`, `Exported`, `Cleaned`. Internal workflow detail remains in `currentStep`.
- Rust keeps backward compatibility for existing local job JSON via serde aliases for old statuses such as `Parsed`, `SplitReady`, `AuthoringReady`, `ValidationFailed`, and `Published`.
- Successful single-exam export and Pack generation now transition through `Exported` and then automatically run cleanup unless diagnostics retention is enabled.
- Cleanup writes `authoring-project.json` as the long-term editable project container and keeps `authoring-ir.json` for existing editor compatibility.
- Cleanup removes transient working artifacts by default: uploads, per-job cache, preview assets, DocumentIR, split candidates, pipeline report, LLM suggestion files/logs, vision/manual transcription temp files, and intermediate validation/runtime reports.
- Settings now exposes Developer/Diagnostics -> `keepFullProcessArtifacts`, default off. This is not part of the ordinary workflow; it exists for parser/model debugging.

### Remaining Risk
- The cleanup policy currently keeps `exports/` under the job folder for local exports because generated output is user-visible. If later exports are always outside app data, this can be tightened.
- Reopening a `Cleaned` job relies on `authoring-ir.json`/`authoring-project.json`; source-document visual review cannot be repeated after cleanup unless diagnostics retention was enabled or the user reimports the source.

## Implementation Findings: 2026-05-31 19:52 CST

### Rust PDF Text Extraction
- Clear-text PDF parsing now uses the Rust `pdf-extract` crate as the primary path, with parser provider `rust-parser:pdf:pdf-extract`.
- The Rust path constructs `DocumentIRV1` directly in the Tauri backend, using the same low-confidence/no-text semantics as the prior Python `pypdf` path.
- If `pdf-extract` errors on a PDF, the backend falls back to the Python parser and records a parser warning that the Rust extractor failed.
- Existing fixture evidence is sufficient for the current MVP decision: complex text PDF reaches AuthoringIR, while no-text PDF still emits parser warning + low-confidence block and therefore triggers source review.
- `pdf-extract` pulled in roughly 32 transitive Cargo packages including `lopdf`; this is acceptable for the MVP because release `.app`/`.dmg` still build successfully and it removes host Python/pypdf from the clear-text PDF critical path.

### Remaining Risk
- `pdf-extract` is still text-layer extraction only. It does not OCR image/scanned PDF pages.
- `pypdf` and Python remain relevant for embedded PDF image extraction used by vision LLM transcription, DOCX parsing through the current sidecar, and legacy fallback.
- PDFs whose scanned pages do not expose embedded images still need manual transcription or a future rendered-page adapter such as PDFium feeding page images into the vision LLM.

## Implementation Findings: 2026-05-31 20:11 CST

### Rendered-Page Fallback for Vision LLM
- The Python parser sidecar now extends `extract_pdf_images`: it still extracts embedded images via `pypdf` first, then falls back to macOS `sips` to render a PNG when no embedded images are exposed.
- The fallback returns `PdfImageExtractionV1.renderedFallback=true` and image-level `renderedFallback=true`, so the backend and LLM gateway can distinguish rendered page previews from real embedded images.
- The fallback emits warnings that it is a rendered-page preview and not OCR or guaranteed multi-page coverage.
- Environment preflight now includes `renderer:macos-sips`, making this local capability visible in Settings.
- This improves the user-uploaded scanned/no-text PDF path: more files can reach vision LLM transcription instead of immediately requiring manual transcription.

### Remaining Risk
- This is macOS-specific and relies on the host `sips` tool. It is acceptable for the current macOS local app target but not a complete cross-platform rendering strategy.
- The `sips` fallback is a lightweight preview renderer, not a full layout/OCR engine. Human SourceReview remains mandatory before publish.
- If later production scope requires Windows/Linux or reliable multi-page scan rendering, a PDFium-based adapter should replace or supplement this fallback.

## Implementation Findings: 2026-05-31 20:46 CST

### Rust DOCX OOXML Primary Parser
- Clear-text DOCX parsing now uses a Rust OOXML path first, with provider `rust-parser:docx:ooxml`.
- The parser reads `word/document.xml` from the DOCX ZIP container, extracts paragraph text, converts table rows into tab-separated table blocks, and emits `DocumentIRV1` directly from the Tauri backend.
- The existing complex DOCX fixture reaches AuthoringIR through the Rust provider, so host Python is no longer on the clear-text DOCX critical path.
- Python sidecar remains as a fallback if Rust DOCX parsing fails, which preserves resiliency for unusual OOXML files while reducing the default dependency surface.
- The `zip` dependency is configured with `flate2` + `deflate-flate2`; `zopfli` is not present in the dependency tree, avoiding unnecessary compression-writing weight for DOCX reads.

### Updated Dependency Risk
- Clear text PDF and DOCX are now Rust-primary.
- Python/pypdf are still needed for PDF embedded image extraction, macOS rendered-page fallback orchestration, and legacy parser fallback.
- Node.js is no longer needed for production LLM, validation, export, or Pack; it remains useful only for optional validator/runtime diagnostics.
- The OCR strategy remains unchanged by this work: image/no-text PDFs use vision LLM transcription plus mandatory SourceReview, with manual transcription fallback. No heavyweight local OCR is bundled by default.

## Implementation Findings: 2026-05-31 21:02 CST

### Rust-Orchestrated Rendered-Page Fallback
- Vision transcription for no-text/image PDFs now uses a Rust wrapper for PDF image extraction.
- The wrapper preserves the preferred path: Python/pypdf extracts embedded images when available.
- If Python/pypdf is unavailable, fails, or returns zero images, Rust invokes macOS `sips` directly and writes a `PdfImageExtractionV1` result with one rendered PNG image.
- This reduces host Python/pypdf from a hard dependency for the macOS scanned-PDF vision path; users can still reach vision LLM transcription via rendered page images when only system `sips` is available.
- This does not add OCR. The generated page PNG is only input to the vision LLM, and the resulting `vision-llm-transcription` DocumentIR still requires SourceReview before export/Pack.

### Updated Dependency Risk
- Clear text PDF and DOCX are Rust-primary.
- macOS rendered-page fallback for scanned/no-text PDFs is now Rust-orchestrated through system `sips`.
- Python/pypdf remain relevant for extracting embedded PDF images and for legacy parser fallback.
- Node.js and the external unified runtime have been removed from the production hard path; they remain diagnostic dependencies only.

## Implementation Findings: 2026-05-31 21:19 CST

### Rust TXT/MD Primary Parser
- TXT and Markdown sources now use Rust parsing before Python sidecar fallback.
- The parser emits `rust-parser:text:plain` for `.txt` and `rust-parser:text:markdown` for `.md`, preserving the existing `DocumentIRV1` shape and downstream split/AuthoringIR behavior.
- Fixture evidence now covers TXT, Markdown, PDF, and DOCX clear-text parsing into AuthoringIR and answer keys through Rust-primary paths.
- Markdown answer-list detection was hardened so answer-only blocks such as `1 TRUE` / `2 FALSE` are recognized as answers instead of question prompts.

### Updated Dependency Risk
- Normal TXT/MD/PDF/DOCX clear-text imports are Rust-primary.
- Python/pypdf are now limited to embedded PDF image extraction and legacy parser fallback.
- macOS scanned/no-text rendered-page fallback is Rust-orchestrated through system `sips`.
- Node.js and the external unified runtime have been removed from the production hard path; they remain diagnostic dependencies only.

## Dependency Strategy Update: 2026-05-31 21:58 CST

### Production Dependency Direction
- Do not bundle Node, Python, or heavyweight OCR engines into the production app by default.
- Production main path should be Rust-first: TXT/MD, text-layer PDF, and DOCX are already Rust-primary.
- Image/no-text PDFs should use page/image rendering plus vision LLM transcription, not local OCR.
- macOS MVP can rely on system `sips` for rendered page images; future Windows/Linux scan support should evaluate an optional PDFium page-render adapter, not a full Python/OCR/Docling stack.
- Node has left the production main path: Rust owns OpenAI-compatible LLM HTTP calls and built-in ReadingExamSourceV1/DOM validation. Node validator remains a development parity check only.
- Preview E2E is now split into production static Rust contract validation and developer/CI/diagnostic real-runtime E2E. If local production E2E becomes mandatory later, prefer an embedded WebView/JS execution approach over a host Node requirement.


## Implementation Findings: 2026-05-31 22:49 CST

### Rust Production Path Consolidation
- Production LLM calls now run in Rust through OpenAI-compatible `/chat/completions` HTTP calls. The Rust gateway covers `classify_group`, `extract_group`, `test_profile`, and `transcribe_pdf_images`.
- Vision transcription encodes rendered/extracted page images as data URLs and validates returned JSON before converting transcription into `DocumentIRV1`.
- Rust built-in `ReadingExamSourceV1` and DOM protocol validation is now authoritative. Node validator is disabled unless `EPIC8_NODE_VALIDATOR_DIAGNOSTICS=1`.
- Export and Pack no longer require external unified runtime E2E. They use Rust static contract validation plus SourceReview/AuthoringReview readiness. The validation report records `runtime.mode=static-rust` for production gates.
- `run_preview_e2e` remains available as an explicit diagnostic command. It can return `runtime.mode=real` or fallback/error diagnostics, but diagnostic failure no longer demotes an otherwise static-gate-ready job from `ExportReady`.

### Updated Dependency Position
- Production MVP/macOS should not bundle Node, Python, or local OCR engines.
- Rust covers normal TXT/MD/PDF/DOCX clear-text imports, LLM HTTP calls, validation, export, and Pack.
- Python/pypdf remains only for legacy parser fallback and embedded PDF image extraction; macOS `sips` provides a Rust-orchestrated page render fallback for vision LLM input.
- For Windows/Linux image PDF support, evaluate a lightweight PDFium page-render adapter. It should render pages for vision LLM, not perform local OCR.

### Remaining Risks
- `src-tauri/src/lib.rs` remains very large and should be split after the current dependency and flow semantics stabilize.
- Rust LLM gateway still needs live-provider E2E coverage across representative OpenAI-compatible providers.
- Visible preview is still a sandboxed local template preview; real unified-runtime visual parity remains diagnostic/future work.


## Implementation Findings: 2026-05-31 23:16 CST

### Backend Module Decomposition First Cut
- Extracted common Rust utilities from `src-tauri/src/lib.rs` into `src-tauri/src/util.rs`.
- The extracted module owns path segment validation, safe job directory helpers, JSON/text/binary IO helpers, deletion helpers, append logging, and the minimal stored-ZIP writer.
- This reduces `lib.rs` from roughly 8886 lines to 8673 lines and establishes a low-risk pattern for later parser/LLM/validator/export module splits.
- No business behavior was intentionally changed; existing callers keep the same helper names through `use util::{...}` imports.

### Verification
- `cargo fmt --check`, `cargo test` (51 tests), `cargo clippy --all-targets -- -D warnings`, `npm run check`, and `git diff --check` passed after the split.
- A transient Cargo target cache issue initially reported a missing `libzip...rlib`; rebuilding after `cargo clean -p zip` restored the dependency artifact and tests passed. This was local build-cache state, not an application logic failure.


## Implementation Findings: 2026-05-31 23:34 CST

### Validator Module Split
- Extracted the pure ReadingExamSourceV1/DOM contract validator into `src-tauri/src/validator.rs`.
- Moved qid sorting, allowed question-kind checks, lightweight HTML tag/attribute parsing, collectible control/dropzone checks, `validate_reading_source_contract`, and validation issue/layer helpers.
- Kept `validate_authoring` in `lib.rs` for now because it still depends on AuthoringIR-to-ReadingExamSource generation and authoring-specific display-number checks.
- `src-tauri/src/lib.rs` is now about 8258 lines, down from about 8886 lines before the first utility split.

### Verification
- `cargo fmt --check`, `cargo test` (51 tests), `cargo clippy --all-targets -- -D warnings`, `npm run check`, and `git diff --check` passed after extracting `validator.rs`.

### Next Architecture Seams
- Parser is the next high-value split, but it touches PDF/DOCX/TXT/MD, source review, and vision image extraction. It should be done in smaller submodules or with typed `DocumentIRV1` structs to avoid increasing JSON-value coupling.
- LLM gateway is another good seam because it is now Rust-owned and has a clear API boundary around profile payloads, HTTP calls, output validation, and fallback generation.

## Implementation Findings: 2026-05-31 23:58 CST

### LLM Gateway Module Split
- Extracted the Rust production LLM gateway into `src-tauri/src/llm_gateway.rs`.
- The new module owns OpenAI-compatible `/chat/completions` calls, JSON-only response parsing, request-cache API-key redaction, vision image data URL encoding, and output normalization for both group suggestions and PDF-image transcription.
- `src-tauri/src/lib.rs` still owns profile CRUD, secret resolution, job orchestration, deterministic low-confidence fallback, suggestion persistence, source review, and auto-apply policy. This is intentional because those concerns are authoring workflow state, not gateway transport.
- The split preserves the latest product decision: Node LLM sidecar is not production-path code; Rust owns normal LLM and vision transcription HTTP calls.
- `lib.rs` is now about 7885 lines. It remains large, but the highest-risk pure infrastructure seams now have independent modules: `util.rs`, `validator.rs`, and `llm_gateway.rs`.

### Verification
- `cargo fmt --check`, `cargo test` (51 tests), `cargo clippy --all-targets -- -D warnings`, `npm run check`, and `git diff --check` passed after the split.

### Architecture Risk Update
- Parser code remains the next main concentration of business complexity. It mixes TXT/MD parsing, PDF text-layer extraction, DOCX OOXML parsing, low-confidence block handling, source review, and image-PDF vision preparation.
- Parser extraction should be incremental and avoid changing the user-facing PDF flow: clear-text inputs stay Rust-primary; image/no-text PDFs still become vision LLM transcription candidates and require SourceReview before publish.

## Implementation Findings: 2026-06-01 00:20 CST

### Parser Module Split
- Extracted the upload parsing stack into `src-tauri/src/parser.rs`.
- Parser module now owns deterministic DocumentIR generation for TXT/MD, text-layer PDFs via `pdf-extract`, DOCX via `zip + quick-xml`, parser failure/missing-source placeholders, manual transcription and vision transcription DocumentIR conversion, Python legacy parser fallback, embedded PDF image extraction, and macOS `sips` rendered-page fallback.
- `lib.rs` still owns source review, split generation, AuthoringIR generation, LLM orchestration, validation, export, and Pack because those are workflow/business-state boundaries rather than parser boundaries.
- The user-uploaded PDF chain remains consistent with the latest requirement: clear-text PDFs parse in Rust; no-text/low-confidence PDFs prepare page images for vision LLM; vision transcription is not treated as human verification; SourceReview and AuthoringReview remain required before publish.
- `src-tauri/src/lib.rs` is now about 6922 lines; backend architecture is materially better than the earlier 8300+ line state, but export/pack/storage remain concentrated.

### Verification
- `cargo fmt --check`, `cargo test` (51 tests), `cargo clippy --all-targets -- -D warnings`, `npm run check`, and `git diff --check` passed after the parser split.

### Residual Parser Risks
- The `sips` fallback still renders a preview image and is explicitly not guaranteed full multi-page coverage. This is acceptable for macOS MVP only because SourceReview blocks publish until the user verifies the source transcription.
- Python/pypdf still exists for embedded-image extraction and legacy fallback. It is not production-hard for clear-text TXT/MD/PDF/DOCX parsing, but the product should continue to show dependency/preflight status clearly for diagnostic/legacy paths.
- Parser output remains JSON `Value`-heavy. A future typed `DocumentIRV1` model would reduce schema drift and make parser/submodule splits safer.

## Implementation Findings: 2026-06-01 00:44 CST

### LLM Profile And Secret Storage Module Split
- Extracted LLM profile and secret storage into `src-tauri/src/llm_profiles.rs`.
- The module now owns profile persistence, UI redaction, Keychain references, plaintext file fallback checks, file secret helpers, and profile lookup.
- `lib.rs` still owns Tauri command handlers and LLM orchestration, which is the correct boundary for now because command handlers combine app state, user payload, profile storage, and gateway testing.
- The split preserves the latest security requirement: API keys are never cached in LLM request JSON, OS secure storage remains the normal storage backend, and plaintext fallback is opt-in only through `EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK`.
- `src-tauri/src/lib.rs` is now about 6673 lines. The remaining large concerns are source/authoring workflow, export/pack, cleanup, and command handlers.

### Verification
- `cargo fmt --check`, `cargo test` (51 tests), `cargo clippy --all-targets -- -D warnings`, `npm run check`, and `git diff --check` passed after the split.

### Architecture Risk Update
- Export/Pack should not be moved wholesale yet because it currently combines ReadingExamSource generation, static runtime gate, SourceReview/AuthoringReview gate, filesystem writes, job state updates, and cleanup. A safe next step is to extract pure packaging/string-generation helpers first, then later move side-effecting export orchestration.

## Implementation Findings: 2026-06-01 01:08 CST

### Export Artifact Builder Split
- Extracted pure ReadingExam output builders into `src-tauri/src/export_artifacts.rs`.
- The new module owns exam id validation, wrapper JS generation, manifest JS generation, pack manifest generation, and a small `ReadingAssetBundle` used by preview/export.
- Side-effecting workflow code remains in `lib.rs`: runtime gate, SourceReview/AuthoringReview publish gate, writing files, updating job status, Pack ZIP writing, and cleanup. This avoids changing production export semantics while still reducing coupling.
- Export and Pack tests continue to prove static Rust gate behavior, output file writing, Pack ZIP writing, and cleanup after successful export.

### Verification
- `cargo fmt --check`, `cargo test` (51 tests), `cargo clippy --all-targets -- -D warnings`, `npm run check`, and `git diff --check` passed after the split.

### Architecture Risk Update
- Export/Pack remains the next high-value area, but the safest path is staged: pure artifact builders first, then pure pack-entry assembly, then side-effecting filesystem/job orchestration only after tests cover each boundary.

## Implementation Findings: 2026-06-01 / E8-33

### Pack Entry Assembly Split
- Pack ZIP entry assembly is now isolated from side-effecting workflow orchestration.
- `src-tauri/src/export_artifacts.rs` owns pure Pack output construction: `pack.json`, manifest JS, and per-exam wrapper JS entries.
- `src-tauri/src/lib.rs` still owns the correct side effects: reading each job's `authoring-ir.json`, running static runtime validation and publish readiness gates, writing the ZIP and unpacked Pack files, updating job status, and cleanup.
- A latent fallback mismatch was addressed defensively: when a Pack source lacks `examId`, the fallback ID is injected into the normalized source before wrapper/manifest/Pack generation. That keeps file path, registry key, and Pack manifest aligned.

### Verification
- `cargo fmt --check`, `cargo test` (52 tests), `cargo clippy --all-targets -- -D warnings`, `npm run check`, and `git diff --check` passed after the split.

### Architecture Risk Update
- Export/Pack behavior remains production Rust-first and does not add Node, Python, or runtime E2E as a release dependency.
- The remaining backend concentration is mostly command/workflow orchestration, not pure artifact generation. Future extraction should focus on storage/settings commands or a dedicated export workflow module with tests around job-state transitions and cleanup.

## Implementation Findings: 2026-06-01 / E8-34

### Diagnostics Settings Boundary
- Diagnostics settings are now separated from workflow orchestration.
- `src-tauri/src/diagnostics.rs` owns only persistence and the public `DiagnosticsSettings` shape used by Tauri commands and the frontend.
- Cleanup remains in `lib.rs` because it changes job status, writes authoring-project/export summaries, and removes workflow artifacts. That is a separate export/lifecycle seam, not a settings seam.

### Verification
- `cargo fmt --check`, `cargo test` (52 tests), `cargo clippy --all-targets -- -D warnings`, `npm run check`, and `git diff --check` passed after extraction.

### Architecture Risk Update
- This split does not affect production parsing, vision LLM transcription, SourceReview, runtime validation, export, or Pack semantics.
- The remaining high-risk architecture work is reducing JSON `Value` coupling in DocumentIR/AuthoringIR and moving workflow orchestration only after explicit tests cover state transitions.

## Implementation Findings: 2026-06-01 / E8-35

### Environment And Preflight Boundary
- Environment/preflight logic is now isolated in `src-tauri/src/environment.rs`.
- The extracted module owns sidecar discovery, command probing, external runtime env path resolution, strict runtime gate parsing, optional Node-validator diagnostics flag parsing, and the complete `EnvironmentPreflightV1` report.
- Existing parser and profile modules continue to use shared `find_sidecar` and `command_failure` helpers through the crate boundary, so error formatting and packaged-resource lookup remain consistent.
- This split is diagnostic/infrastructure only. It does not change import parsing, vision LLM transcription, SourceReview, runtime validation, export, or Pack semantics.

### Verification
- `cargo fmt --check`, `cargo test` (52 tests), `cargo clippy --all-targets -- -D warnings`, `npm run check`, and `git diff --check` passed after extraction.

### Architecture Risk Update
- `lib.rs` is down to about 6210 lines, but still contains core workflow logic. The remaining high-value work is to split stateful business workflows carefully with tests around lifecycle transitions.
- Environment preflight continues to communicate Node/Python/pypdf as optional diagnostics/legacy capabilities, aligned with the Rust-first production path and no bundled local OCR strategy.

## Implementation Findings: 2026-06-01 / E8-36

### Real PDF Sample Audit And Umbrella Question Range Handling
- The four user-provided P2 PDF samples all contain a valid opening umbrella instruction equivalent to `Questions 14-26`. This is not a false positive; it is the overall Passage 2 question range.
- The opening umbrella range now flows into split output as `umbrellaQuestionRanges`, so the app preserves the business fact that Passage 2 covers Q14-Q26.
- The split builder now distinguishes concrete headings from inline references. Headings such as `Questions 14-19`, `Questions 20-23`, and Markdown `## Questions 1-5` are concrete groups; inline references such as `Look at the following statements (Questions 20-23)` are not treated as new group starts.
- When concrete groups exist, the umbrella range is not converted into a duplicate concrete Q14-Q26 group. This prevents duplicated questions and preserves the later specific interaction groups.
- When only the umbrella range exists, the app now creates a low-confidence `requiresManualQuestionImport` scaffold instead of pretending concrete prompts exist. AuthoringReview blocks publish until the user imports/edits and verifies the actual prompts and answers.
- Some samples are mixed text/image PDFs: Rust `pdf-extract` can parse the main text pages while later pages contain image-only or blank content. These correctly remain eligible for vision transcription and SourceReview, because missing image pages may contain answer keys or question content.
- The 120 sample has answer letters interleaved after earlier groups. The split logic no longer truncates question discovery at the first answer-like block, so later concrete groups `20-23` and `24-26` are retained.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 53 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Error Log Addition
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-06-01 | Initial real-PDF regression incorrectly asserted every text-layer PDF had no parser warnings. Mixed PDFs with image-only pages legitimately require vision transcription/SourceReview. | 1 | Changed the regression to separate fully text-layer-readable PDFs from mixed text/image PDFs and verify the correct review routing for each. |
| 2026-06-01 | New heading detection initially broke Markdown fixtures because `## Questions 1-5` was no longer considered a concrete heading. | 1 | Normalized leading Markdown `#` characters before checking heading starts. Full Rust tests passed afterward. |
| 2026-06-01 | A malformed targeted `cargo test` command passed two filter names; Cargo accepts only one test filter. | 1 | Ran the full `cargo test` suite instead and recorded the command mistake. |
| 2026-06-01 | `cargo clippy --all-targets -- -D warnings` flagged an unnecessary lazy default in `isUmbrellaRange` serialization. | 1 | Replaced `unwrap_or_else(|| json!(false))` with `unwrap_or(Value::Bool(false))` and reran clippy successfully. |

## Implementation Findings: 2026-06-01 / E8-37

### SourceReview Module Boundary
- `src-tauri/src/source_review.rs` now owns SourceReview-specific computation and persistence:
  - `parser_warnings`
  - `low_confidence_block_ids`
  - `source_review_fingerprint`
  - `source_review_status`
  - `write_source_review_status`
  - `source_review_issues`
- The module intentionally keeps JSON `Value` interfaces for now because callers still operate on `DocumentIRV1` and `AuthoringIR` as JSON. This keeps the extraction low risk and avoids changing frontend or persisted contracts.
- `lib.rs` still owns workflow state transitions after SourceReview results are computed. This is the correct boundary for now because status transitions combine SourceReview, AuthoringReview, LLM auto-apply, runtime validation, export, and cleanup.
- SourceReview publish semantics are unchanged: unresolved parser warnings or low-confidence blocks produce blocking AuthoringIR issues. Resolving review is fingerprint-aware and becomes stale when the underlying warnings/low-confidence source changes.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test source_review -- --nocapture)` | pass, 5 targeted tests |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 53 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Error Log Addition
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-06-01 | After extraction, `source_review_fingerprint` was imported for all builds but only used by tests, producing an unused import warning in normal lib builds. | 1 | Split it into a `#[cfg(test)]` import and reran targeted/full tests plus clippy successfully. |
| 2026-06-01 | Running `cargo test` and `cargo clippy` in parallel caused Cargo file-lock waiting messages. | 1 | Allowed Cargo to serialize naturally; both commands completed successfully. |

## Implementation Findings: 2026-06-01 / E8-38

### Job Store Module Boundary
- `src-tauri/src/job_store.rs` now owns job persistence and listing:
  - `make_job`
  - `load_job`
  - `save_job`
  - `update_job`
  - `list_saved_jobs`
- Tauri command handlers still own application root resolution, directory creation, delete-job filesystem removal, and business workflow orchestration. This keeps the module boundary narrow and avoids moving status transitions prematurely.
- `list_jobs` now delegates filtering/sorting to `list_saved_jobs`; the behavior remains the same: optional status filter, optional case-insensitive title/job-id search, and reverse `updated_at` ordering.
- The job path traversal checks remain covered through existing tests. `job_store` still uses `safe_job_dir` and `validate_path_segment` internally.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 53 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Error Log Addition
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-06-01 | A targeted `cargo test` command again passed two filter names, which Cargo rejects. | 1 | Switched to full `cargo test` to cover both intended areas and avoid false confidence. |
| 2026-06-01 | Moving job persistence helpers initially removed `read_json` from `lib.rs` imports, but workflow code still reads AuthoringIR/validation JSON directly. | 1 | Restored `read_json` import for workflow artifact reads; job-specific reads remain in `job_store`. |
| 2026-06-01 | `safe_job_dir` became unused in normal builds after moving `load_job`; clippy failed under `-D warnings`. | 1 | Moved `safe_job_dir` into a `#[cfg(test)]` import because tests still assert path-helper behavior directly. |

## Implementation Findings: 2026-06-01 / E8-39

### `Questions 14-26` Umbrella Range Product Semantics
- User clarified that opening instructions such as `Questions 14-26` are valid question-group information, even though they are presented differently from concrete subgroups.
- Current intended model is two-level:
  - `umbrellaQuestionRanges`: preserves the Passage 2 total range from the opening instruction.
  - `questionGroupCandidates`: contains concrete interactive groups such as `Questions 14-19`, `Questions 20-23`, and `Questions 24-26`, or a low-confidence manual-import scaffold when concrete prompts are not available.
- This avoids two bad outcomes: dropping a valid source range, or generating a duplicate concrete Q14-Q26 group that would duplicate later subgroups.
- Browser dev fallback now mirrors the Rust production behavior for this distinction, including `requiresManualQuestionImport` validation/readiness blocking.

### Verification
| Test | Status |
|------|--------|
| `npm run check` | pass |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `git diff --check` | pass |

### Remaining Risk
- There is still no dedicated frontend unit test runner for `devFallbackBackend.ts`; TypeScript checking verifies the contract shape, while Rust regression verifies the production PDF sample behavior.
- The live vision LLM transcription path remains unproven against scanned/image-only real PDFs with credentials. SourceReview remains mandatory for such cases.

## Implementation Findings: 2026-06-01 / E8-40

### Cleanup Lifecycle Boundary
- `src-tauri/src/cleanup.rs` now owns cleanup mechanics:
  - diagnostics retention check through `load_diagnostics_settings`
  - removal of transient directories such as `uploads`, `cache`, `preview`, and `llm-suggestions`
  - removal of transient files such as `document-ir.json`, `split-candidates.json`, LLM call logs, transcription temp files, and validation reports
  - `cleanup-summary.json` generation
- `lib.rs` intentionally still owns `write_authoring_project` and job state transitions. The cleanup module receives those as closures, keeping state-machine decisions in the workflow layer.
- This is the right current boundary because cleanup is called by both single export and Pack export, while final job status depends on the surrounding workflow context.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test cleanup -- --nocapture)` | pass |
| `(cd src-tauri && cargo test export_core_writes_assets_after_static_runtime_gate -- --nocapture)` | pass |
| `(cd src-tauri && cargo test build_pack_core_writes_zip_after_static_runtime_gate -- --nocapture)` | pass |
| `(cd src-tauri && cargo test)` | pass, 53 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Remaining Risk
- Export and Pack orchestration still live in `lib.rs`; the next safe extraction would need tests around status transitions, readiness gates, and cleanup ordering before moving more side-effecting code.

## Implementation Findings: 2026-06-01 / E8-41

### LLM Suggestion Boundary
- `src-tauri/src/llm_suggestions.rs` now owns LLM suggestion data-shaping and safety helpers:
  - `llm_group_context`
  - deterministic low-confidence fallback output
  - `make_llm_input`
  - `make_vision_transcription_input`
  - `save_llm_suggestion` / `load_llm_suggestions`
  - `llm_suggestion_auto_apply_issues`
  - `apply_suggestion_to_authoring`
- `lib.rs` still owns stateful orchestration: loading jobs/profiles/secrets, running the Rust LLM gateway, updating AuthoringIR audit fields, SourceReview-aware job status updates, and auto-pipeline flow.
- This boundary preserves the important safety rule: high-confidence LLM suggestions may apply only whitelisted structural/question patches with provider evidence and source quotes; they do not count as human verification.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test llm -- --nocapture)` | pass, 6 targeted tests |
| `(cd src-tauri && cargo test)` | pass, 53 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Remaining Risk
- The LLM provider live path still lacks credential-backed integration evidence in this workspace. The Rust gateway and fallback safety are tested, but live provider quality and latency should be validated when a real OpenAI-compatible profile/API key is available.
- Auto-pipeline orchestration still lives in `lib.rs`; extracting it safely requires tests over parse -> split -> AuthoringIR -> LLM suggestions -> review status -> static validation.

## Implementation Findings: 2026-06-01 / E8-42

### AuthoringReview Rule Boundary
- `src-tauri/src/authoring_review.rs` now owns pure AuthoringIR review logic:
  - recursive empty-answer detection
  - low-confidence / verified checks
  - derivation of `audit.humanVerified`
  - group `verified` refresh based on all question verification
  - blocking AuthoringIR issues for low confidence, empty answers, and manual-question-import scaffolds
- `lib.rs` still owns publish readiness orchestration because it must combine job status, SourceReview, AuthoringReview, runtime/static validation, and report persistence.
- This boundary makes the review semantics easier to test and reduces risk of future LLM/OCR changes bypassing manual review gates.

### Verification
| Test | Status |
|------|--------|
| Targeted AuthoringReview tests | pass |
| `(cd src-tauri && cargo test)` | pass, 53 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Remaining Risk
- `validate_authoring`, `reading_source`, and publish readiness orchestration still live in `lib.rs`. They are viable future extraction points, but should be split with contract validation and export/Pack tests in the same pass.

## Implementation Findings: 2026-06-01 / E8-43

### Reading Source Contract Boundary
- `src-tauri/src/reading_source.rs` now owns the pure `ReadingExamSourceV1` assembly path:
  - HTML escaping and group-body HTML rendering
  - answer key projection from AuthoringIR
  - question order and question display map projection
  - `ReadingExamSourceV1` assembly for export/runtime validation
- `lib.rs` still owns the stateful orchestration around this contract:
  - job/source lookup
  - SourceReview and AuthoringReview gating
  - `validate_authoring` / `publish_readiness_gate`
  - runtime/export/Pack command handlers
- Keeping the source contract builder isolated helps ensure that future refactors or UI changes cannot silently distort the published runtime shape.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test reading_source -- --nocapture)` | pass |
| `(cd src-tauri && cargo test rust_contract_validator -- --nocapture)` | pass |
| `(cd src-tauri && cargo test export_core_writes_assets_after_static_runtime_gate -- --nocapture)` | pass |
| `(cd src-tauri && cargo test build_pack_core_writes_zip_after_static_runtime_gate -- --nocapture)` | pass |
| `(cd src-tauri && cargo test)` | pass, 53 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Remaining Risk
- `validate_authoring`, `validate_for_runtime_gate`, and `publish_readiness_gate` still live in `lib.rs`. They are the next plausible contract-layer seam, but should be split only with tests that directly cover all validation layers and publish blocking behavior.

## Implementation Findings: 2026-06-01 / E8-44

### `Questions 14-26` Opening Range Clarification
- User confirmed that opening instructions such as `Questions 14-26` are legitimate question-group information, even when they are presented as the overall Passage 2 instruction rather than a concrete interaction block.
- The implementation now treats this as an umbrella range:
  - It is preserved in `umbrellaQuestionRanges` for review and downstream context.
  - It is not duplicated as a concrete Q14-Q26 interaction when later concrete groups exist.
  - If it is the only detected question range, the app creates a low-confidence manual-question-import scaffold and AuthoringReview blocks publish until concrete prompts/answers are manually completed and verified.
- The detector was widened from one exact phrase to conservative Passage-level opening patterns while keeping concrete headings out of umbrella classification.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test umbrella_question_range_detection_keeps_opening_instructions_distinct -- --nocapture)` | pass |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `npm run check` | pass |
| `(cd src-tauri && cargo test)` | pass, 54 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `git diff --check` | pass |

### Remaining Risk
- This is still heuristic text detection over parsed PDF text blocks. Vision LLM transcription and manual SourceReview remain required for scanned/image PDFs or ambiguous layouts.

### Revalidation
- User reconfirmed on 2026-06-01 that opening `Questions 14-26` ranges are correct题组信息 and should be included.
- The production Rust parser/split path and browser dev fallback already implement this as a two-level model: `umbrellaQuestionRanges` for the Passage-level range, plus concrete `questionGroupCandidates` for publishable interaction groups or a manual-import scaffold when concrete prompts are absent.
- Targeted regression evidence: `umbrella_question_range_detection_keeps_opening_instructions_distinct` and `files_pdf_samples_reach_expected_review_paths` both passed.

## Implementation Findings: 2026-06-01 / E8-45

### Authoring Validation Boundary
- `src-tauri/src/authoring_validation.rs` now owns pure AuthoringIR validation and validation-report merging:
  - `validate_authoring`
  - `merge_sidecar_validation`
  - `merge_validation_issues`
- `lib.rs` still owns stateful workflow orchestration:
  - static runtime gate file writes and preview asset generation
  - optional Node validator diagnostics
  - SourceReview and AuthoringReview publish readiness
  - export/Pack job state transitions and cleanup
- This boundary reduces monolithic backend size while keeping high-risk lifecycle behavior in the already-tested command layer.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test validate_authoring_blocks_duplicate_display_numbers_and_gaps -- --nocapture)` | pass |
| `(cd src-tauri && cargo test validation_warning_does_not_block_runtime_gate_progress -- --nocapture)` | pass |
| `(cd src-tauri && cargo test rust_contract_validator -- --nocapture)` | pass, 10 tests |
| `(cd src-tauri && cargo test)` | pass, 54 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Remaining Risk
- `validate_for_runtime_gate`, `publish_readiness_gate`, export, and Pack orchestration still live in `lib.rs`. They are now clearer seams, but should only be extracted with tests that directly cover validation-report persistence, job status transitions, cleanup, and publish-blocking behavior.

## Implementation Findings: 2026-06-01 / E8-46

### Export/Pack Publish Gate Side Effects
- Positive export/Pack tests already proved successful static gate + publish readiness writes assets and cleanup.
- New negative tests now prove that publish readiness failure after static validation does not produce final output artifacts and does not trigger cleanup.
- This is important for PDF/LLM/manual-review safety because LLM output or umbrella-only scaffolds must not become exported runtime content until human verification and SourceReview gates are satisfied.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test export_core_publish_gate_failure_writes_no_export_or_cleanup -- --nocapture)` | pass |
| `(cd src-tauri && cargo test build_pack_publish_gate_failure_writes_no_pack_or_cleanup -- --nocapture)` | pass |
| `(cd src-tauri && cargo test)` | pass, 56 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Remaining Risk
- The same publish gate protections should eventually be covered at the Tauri command boundary or UI integration level, but core backend behavior is now directly tested.

## Implementation Findings: 2026-06-01 / E8-47

### Command Lifecycle And Dialog Completion
- `validate_authoring_ir` now explicitly synchronizes `current_step` with validation outcome:
  - unresolved SourceReview/parser issues -> `NeedsReview` / `DocumentReview`
  - AuthoringIR validation failure -> `NeedsReview` / `Authoring`
  - AuthoringIR validation pass -> `DraftSaved` / `Authoring`
- This avoids stale workflow state such as a job remaining on `Export` or `Preview` after validation re-runs.
- `choose_export_dir` is no longer a backend stub. The Rust command opens a native Tauri folder picker on a blocking worker thread and returns the selected path.
- Browser dev fallback now mirrors the validation-step update so development preview state does not diverge from production Rust behavior.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test validation_job_state_routes_review_and_authoring_steps -- --nocapture)` | pass |
| `(cd src-tauri && cargo test validate_authoring_state_update_overwrites_stale_current_step -- --nocapture)` | pass |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 58 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Remaining Risk
- The native directory picker itself is not exercised in an automated UI test because it requires OS dialog interaction. Compile, clippy, and command wiring prove the backend API is implemented; manual desktop smoke can verify user interaction.

## Implementation Findings: 2026-06-01 / E8-48

### Workflow State Module Boundary
- `src-tauri/src/workflow_state.rs` now owns lifecycle state projection for validation and preview-E2E commands:
  - `apply_preview_e2e_job_state`
  - `validation_job_state`
  - `update_validation_job_state`
  - shared issue-count projection for validation/runtime report severities
- This is a deliberately narrow extraction. It moves state transition rules and their tests out of `lib.rs` without moving command orchestration, filesystem artifact generation, export, Pack, or publish readiness gates.
- The module adds direct tests for both validation and preview lifecycle routing:
  - SourceReview issues route back to `DocumentReview`.
  - AuthoringIR validation failures stay in `Authoring`.
  - Passing AuthoringIR validation becomes `DraftSaved`/`Authoring`.
  - Preview failure overwrites stale `ExportReady`/`Export`.
  - Preview success becomes `ExportReady` only when publish readiness also passes.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test workflow_state -- --nocapture)` | pass, 4 tests |
| `(cd src-tauri && cargo test failed_runtime_validation_downgrades_stale_export_ready_status -- --nocapture)` | pass |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 60 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Remaining Risk
- `lib.rs` is still large at 5375 lines. Export/Pack and auto-pipeline orchestration remain in `lib.rs`; those should be extracted only with stronger command-boundary coverage because they coordinate filesystem side effects, cleanup, publish gates, and job state.

## Implementation Findings: 2026-06-01 / E8-49

### Runtime Validation Module Boundary
- `src-tauri/src/runtime_validation.rs` now owns the runtime/static validation helper boundary:
  - `preview_assets_for_source`
  - `validate_for_runtime_gate`
  - `run_node_validator_diagnostic`
  - `validate_with_node_sidecar`
  - `validate_preview_with_node_sidecar`
  - `publish_readiness_gate`
- This module keeps the production validation policy explicit:
  - Rust static contract validation is the production gate.
  - Node validator is optional diagnostics only.
  - Real preview E2E is diagnostics only and cannot by itself determine publishability.
  - Publish readiness still merges SourceReview and AuthoringReview issues into the validation report.
- `lib.rs` still owns command orchestration and side-effect-heavy export/Pack flows, which is safer until there is broader command-boundary lifecycle coverage.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test rust_contract_validator -- --nocapture)` | pass, 10 tests |
| `(cd src-tauri && cargo test publish_gate_blocks_no_text_pdf_until_source_review_resolved -- --nocapture)` | pass |
| `(cd src-tauri && cargo test preview_e2e_diagnostic_failure_does_not_block_static_export_ready -- --nocapture)` | pass |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 60 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Remaining Risk
- `lib.rs` is still 5185 lines. Export/Pack orchestration and auto-pipeline remain the largest unextracted stateful areas; moving them should wait for command-boundary tests that prove artifact write/cleanup/status side effects.

## Implementation Findings: 2026-06-01 / E8-50

### Opening Umbrella Range Revalidation
- User clarified again that opening instructions such as `Questions 14-26` are correct question-group information, even when they appear in the passage introduction rather than as a later concrete interaction heading.
- Current production behavior is correct and should be preserved:
  - `umbrellaQuestionRanges` stores the Passage-level total range for review/downstream context.
  - Later concrete headings remain the source of publishable `questionGroupCandidates`.
  - If only the opening umbrella range is detected, the app creates a low-confidence `requiresManualQuestionImport` scaffold and AuthoringReview blocks publish until concrete prompts/answers are manually imported and verified.
- The Rust parser already accepts hyphen, en dash, and em dash range separators. E8-50 added explicit regression coverage for the en-dash spelling so future refactors do not accidentally narrow this behavior.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test umbrella_question_range_detection_keeps_opening_instructions_distinct -- --nocapture)` | pass |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `npm run check` | pass |

### Remaining Risk
- This remains heuristic detection over parsed text blocks. Scanned/image PDFs still require vision transcription plus SourceReview, and umbrella-only detections still require human AuthoringReview before publish.

## Implementation Findings: 2026-06-01 / E8-51

### Authoring Pipeline Module Boundary
- `src-tauri/src/authoring_pipeline.rs` now owns the pure business rules that transform parsed `DocumentIRV1` plus split candidates into initial `ReadingAuthoringIRV1`:
  - DocumentIR block flattening/text/html helpers.
  - Question range, concrete heading, and umbrella range detection.
  - Answer token parsing and answer-source merge behavior.
  - Dynamic split candidate generation.
  - Prompt extraction and initial group/question/answerKey/questionOrder/displayMap construction.
- The extraction intentionally leaves side effects in `lib.rs`:
  - Tauri command handlers.
  - file IO and parser execution for separate answer sources.
  - SourceReview and AuthoringReview state updates.
  - LLM profile/provider orchestration.
  - export/Pack artifact writes, cleanup, and job lifecycle transitions.
- The module boundary is aligned with current safety constraints: it improves maintainability without moving the filesystem-heavy export/Pack orchestration that still requires strict command-boundary protection.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test umbrella_question_range_detection_keeps_opening_instructions_distinct -- --nocapture)` | pass |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `(cd src-tauri && cargo test)` | pass, 60 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Remaining Risk
- `authoring_pipeline.rs` is still JSON-heavy and heuristic-based. It is now isolated enough for future typed-domain refactors, but scanned/image PDFs still require vision LLM transcription plus SourceReview, and low-confidence/manual-import outputs still require AuthoringReview before publish.
- `lib.rs` remains sizeable at 4250 lines. Export/Pack orchestration and auto-pipeline command flow remain the largest stateful areas and should only move with strong artifact/write/cleanup/status tests.

## Implementation Findings: 2026-06-01 / E8-52

### Cleanup And AuthoringProject Boundary
- `src-tauri/src/cleanup.rs` now owns the successful-export archival/cleanup lifecycle:
  - `AuthoringProjectV1` generation.
  - source summary, review summary, validation summary, and export summary assembly.
  - diagnostics-retention behavior.
  - transient artifact deletion.
  - final `Cleaned` job-state transition.
- `lib.rs` still owns export/Pack artifact writes and publish-gate orchestration. This keeps the high-risk artifact-writing command flow in place while removing the lower-level cleanup/archive details from the command monolith.
- This boundary matches the product lifecycle decision: successful exports retain editable structured drafts and summaries, while process files are transient unless the developer diagnostics retention switch is enabled.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test cleanup_respects_diagnostics_artifact_retention -- --nocapture)` | pass |
| `(cd src-tauri && cargo test export_core_writes_assets_after_static_runtime_gate -- --nocapture)` | pass |
| `(cd src-tauri && cargo test export_core_publish_gate_failure_writes_no_export_or_cleanup -- --nocapture)` | pass |
| `(cd src-tauri && cargo test build_pack_core_writes_zip_after_static_runtime_gate -- --nocapture)` | pass |
| `(cd src-tauri && cargo test build_pack_publish_gate_failure_writes_no_pack_or_cleanup -- --nocapture)` | pass |
| `(cd src-tauri && cargo test)` | pass, 60 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Remaining Risk
- `lib.rs` remains 4165 lines and still coordinates export/Pack artifact writes plus auto-pipeline. Those should move only after command-level tests prove every side effect: validation, output writes, cleanup, and job-state transitions.

## Implementation Findings: 2026-06-01 / E8-53

### Auto Pipeline Command-Core Safety Coverage
- `run_auto_pipeline` now delegates to `run_auto_pipeline_core(root, job_id, input)`, making the real filesystem-backed pipeline directly testable without launching a Tauri app.
- Added regression coverage for two high-risk user-upload flows:
  - Clear-text `TXT` fixture can parse/split/build AuthoringIR and pass static runtime validation, but if the LLM gateway fails it stays `NeedsReview` at `LlmReview`; no cleanup/export artifacts are written.
  - No-text PDF fixture triggers the image/vision path and remains `NeedsReview` at `DocumentReview` with unresolved SourceReview; no cleanup/export artifacts are written.
- These tests are aligned with the intended product flow: upload should be automatic as far as deterministic parsing/vision/LLM suggestions can go, but uncertainty must route to human review before publish.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test auto_pipeline_llm_failure_keeps_text_import_in_llm_review -- --nocapture)` | pass |
| `(cd src-tauri && cargo test auto_pipeline_keeps_no_text_pdf_blocked_for_source_review -- --nocapture)` | pass |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `(cd src-tauri && cargo test)` | pass, 62 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Remaining Risk
- Positive high-confidence LLM auto-apply is still tested at helper level, not through the full command-core path with a controlled provider/mock seam. Add that before extracting auto-pipeline orchestration from `lib.rs`.
- Live vision LLM transcription remains dependent on provider credentials and should get a diagnostic/live test path when credentials are available.

## Implementation Findings: 2026-06-01 / E8-54

### Opening `Questions 14-26` / `Questions 14–26` As Valid Group Metadata
- User clarified that an opening instruction range such as `Questions 14-26` is a valid题组范围, even when it is not presented as a later concrete interaction heading.
- Current product model remains correct:
  - Store the opening total range in `umbrellaQuestionRanges`.
  - Use later concrete headings as publishable/editable `questionGroupCandidates`.
  - Do not create a duplicate concrete Q14-Q26 group when concrete subgroups exist.
  - Only create `requiresManualQuestionImport` scaffolds when no concrete subgroup prompts are available.
- Added a minimal Rust regression proving `Questions 14–26 are based on Reading Passage 2 below` is retained while concrete groups `14-19`, `20-23`, and `24-26` remain the actual editable groups.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test opening_umbrella_range_is_included_without_replacing_concrete_groups -- --nocapture)` | pass |
| `(cd src-tauri && cargo test umbrella_question_range_detection_keeps_opening_instructions_distinct -- --nocapture)` | pass |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `npm run check` | pass |
| `(cd src-tauri && cargo test)` | pass, 63 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |

### Remaining Risk
- The umbrella/concrete distinction is still heuristic over parsed text blocks. Image-only/scanned pages remain dependent on vision transcription plus SourceReview before concrete prompts can be trusted.

## Implementation Findings: 2026-06-01 / E8-55

### Full Auto-Pipeline High-Confidence LLM Auto-Apply Coverage
- Added an internal command-core seam, `run_auto_pipeline_core_with_gateway`, so tests can inject a controlled LLM gateway without weakening production behavior. `run_auto_pipeline_core` still calls the real Rust `run_llm_gateway`.
- The new regression proves the complete automatic upload pipeline can auto-apply high-confidence, source-evidenced LLM structure suggestions while preserving the human trust boundary:
  - Valid evidence-backed high-confidence suggestions are applied.
  - `autoApplied` metadata is recorded on affected groups.
  - The LLM cannot create `audit.humanVerified` or question `verified=true`.
  - The LLM does not erase parsed answers in this path.
  - The job remains in review (`NeedsReview` / `Authoring`) until human verification is completed.
- This materially strengthens the earlier helper-level tests by proving command orchestration, filesystem writes, suggestion persistence, validation, and job-state projection interact correctly.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test auto_pipeline_high_confidence_llm_auto_applies_without_human_verification -- --nocapture)` | pass |
| `(cd src-tauri && cargo test auto_pipeline_llm_failure_keeps_text_import_in_llm_review -- --nocapture)` | pass |
| `(cd src-tauri && cargo test auto_pipeline_keeps_no_text_pdf_blocked_for_source_review -- --nocapture)` | pass |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 64 tests |
| `npm run check` | pass |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `git diff --check` | pass |

### Remaining Risk
- The Rust backend still keeps auto-pipeline orchestration in `lib.rs`. With command-core coverage now in place for the major safety paths, extracting that orchestration into a dedicated module is lower risk but still needs careful side-effect preservation.
- Live-provider LLM and vision transcription coverage remain dependent on credentials and representative scanned PDFs.

## Implementation Findings: 2026-06-01 / E8-56

### Auto Pipeline Module Boundary
- `src-tauri/src/auto_pipeline.rs` now owns the stateful automatic upload pipeline orchestration:
  - source parsing when no DocumentIR exists,
  - SourceReview initialization,
  - optional vision transcription for no-text/image PDFs,
  - split candidate generation and answer-source merge,
  - initial AuthoringIR generation,
  - LLM suggestion execution/fallback,
  - high-confidence auto-apply with evidence checks,
  - static runtime validation,
  - final job-state and pipeline-report projection.
- `src-tauri/src/lib.rs` keeps the Tauri command wrapper and other user-facing command orchestration.
- The module boundary is now backed by command-core tests for LLM failure, no-text PDF SourceReview, and high-confidence LLM auto-apply.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test auto_pipeline_high_confidence_llm_auto_applies_without_human_verification -- --nocapture)` | pass |
| `(cd src-tauri && cargo test auto_pipeline_llm_failure_keeps_text_import_in_llm_review -- --nocapture)` | pass |
| `(cd src-tauri && cargo test auto_pipeline_keeps_no_text_pdf_blocked_for_source_review -- --nocapture)` | pass |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 64 tests |
| `npm run check` | pass |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `git diff --check` | pass |

### Remaining Risk
- `auto_pipeline.rs` is still JSON-heavy and stateful. It is now isolated for future typed-domain refactors, but export/Pack orchestration remains in `lib.rs` and is still the next high-risk area.
- Live provider coverage is still not automated because it requires credentials and representative provider configuration.

## Implementation Findings: 2026-06-01 / E8-57

### Export/Pack Module Boundary
- `src-tauri/src/export_pack.rs` now owns export/Pack artifact-writing orchestration:
  - single ReadingExamSource asset export,
  - static runtime and publish-readiness gate enforcement,
  - Pack source collection and validation,
  - zip/expanded pack artifact writing,
  - job transition to `Exported`,
  - successful-export cleanup invocation.
- `src-tauri/src/lib.rs` keeps only the Tauri command wrappers for `export_reading_assets` and `build_pack` so Tauri handler macro generation remains in the command module.
- The boundary is protected by existing tests for both success and failure semantics. This is important because failed publish gates must not write final artifacts or cleanup summaries.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test export_core_writes_assets_after_static_runtime_gate -- --nocapture)` | pass |
| `(cd src-tauri && cargo test export_core_publish_gate_failure_writes_no_export_or_cleanup -- --nocapture)` | pass |
| `(cd src-tauri && cargo test build_pack_core_writes_zip_after_static_runtime_gate -- --nocapture)` | pass |
| `(cd src-tauri && cargo test build_pack_publish_gate_failure_writes_no_pack_or_cleanup -- --nocapture)` | pass |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 64 tests |
| `npm run check` | pass |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `git diff --check` | pass |

### Remaining Risk
- `export_pack.rs` remains JSON-heavy and should eventually receive typed request/result structs, but its side-effect boundary is now isolated and covered by command-core tests.
- `lib.rs` is smaller but still contains many Tauri commands and test fixtures; future splits should prioritize low-risk command domains with existing regression coverage.

## Implementation Findings: 2026-06-01 / E8-58

### Standalone `Questions 14-26` Opening Range Semantics
- The valid Passage-level range may appear as a full sentence (`Questions 14-26 are based on Reading Passage 2 below`) or as a standalone heading block (`Questions 14–26`) near `READING PASSAGE` after PDF extraction.
- Production Rust now supports both forms:
  - Single-block contextual instructions are still recognized by text context.
  - Bare full-passage opening headings are recognized using neighboring block context and opening position.
  - Later concrete subgroup headings are not replaced and remain the publishable interaction candidates.
- The recognition is intentionally conservative:
  - Bare heading must represent a full-passage span, currently requiring a range width of at least 9.
  - It must appear in the opening passage position or be followed by concrete subgroups within its range.
  - Short concrete headings such as `Questions 14-19` should not become umbrella ranges merely because they mention `Reading Passage`.
- Browser dev fallback mirrors the same behavior so local no-Tauri development remains representative.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test umbrella -- --nocapture)` | pass, 3 tests |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 65 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Remaining Risk
- The full-passage threshold is heuristic. It fits IELTS Reading passage ranges such as P1 `1-13`, P2 `14-26`, and P3 `27-40`, but future non-IELTS content could require a configurable range policy.
- Scanned/image PDFs still depend on vision transcription plus SourceReview; this change only hardens deterministic split behavior after text/transcription has produced blocks.

## Implementation Findings: 2026-06-01 / E8-59

### LLM Command-Core Boundary
- `src-tauri/src/llm_commands.rs` now owns the user-triggered LLM command-core flow:
  - profile save/test core behavior,
  - classify/extract suggestion creation,
  - gateway fallback to deterministic low-confidence suggestions,
  - suggestion persistence,
  - high-confidence suggestion application,
  - answerKey/questionOrder/questionDisplayMap regeneration,
  - AuthoringReview and SourceReview issue projection into job state.
- `src-tauri/src/lib.rs` keeps only Tauri wrappers for LLM commands, preserving frontend command names and `generate_handler!` macro scope.
- `load_llm_api_key` now lives in `llm_profiles.rs`; this keeps API-key retrieval policy next to secret storage policy and avoids coupling command modules to auto-pipeline internals.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test llm -- --nocapture)` | pass, 8 tests |
| `(cd src-tauri && cargo test auto_pipeline_high_confidence_llm_auto_applies_without_human_verification -- --nocapture)` | pass |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 65 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Remaining Risk
- `llm_commands.rs` still uses JSON-heavy data movement. The extraction makes the boundary explicit, but future typed request/result structs are still needed to reduce field drift.
- Live-provider coverage remains limited by available credentials/config. Current tests prove fallback, validation, and auto-apply safety semantics, not real provider quality.

## Implementation Findings: 2026-06-01 / E8-60

### Preview/Validation Command-Core Boundary
- `src-tauri/src/preview_commands.rs` now owns the command-core layer for validation and preview:
  - `validate_authoring_ir_core`,
  - `generate_preview_assets_core`,
  - `run_preview_e2e_core`.
- The boundary keeps responsibilities clear:
  - `runtime_validation.rs` owns low-level static contract validation, preview asset writing, publish readiness, and optional Node diagnostics.
  - `workflow_state.rs` owns job-state transitions for validation and preview diagnostic outcomes.
  - `lib.rs` only exposes Tauri wrappers for the frontend.
- The production dependency strategy remains unchanged: static Rust validation is authoritative; Node/real runtime E2E is development/diagnostic only.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test preview -- --nocapture)` | pass, 3 tests |
| `(cd src-tauri && cargo test runtime_gate -- --nocapture)` | pass, 3 tests |
| `(cd src-tauri && cargo test validation_job_state -- --nocapture)` | pass |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 65 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Remaining Risk
- `preview_commands.rs` still passes JSON validation reports around directly. Future typed report structs would reduce coupling across `authoring_validation`, `runtime_validation`, and UI-facing command results.
- Broader UI E2E remains limited; current coverage proves command-core/static runtime behavior, not every rendered interaction path.

## Implementation Findings: 2026-06-01 / E8-61

### Authoring Command-Core Boundary
- `src-tauri/src/authoring_commands.rs` now owns the command-core layer for document/source-review/split/AuthoringIR operations:
  - `parse_document_core`,
  - `apply_manual_transcription_core`,
  - `apply_vision_transcription_core`,
  - `resolve_source_review_core`,
  - `run_rule_split_core`,
  - `save_split_adjustments_core`,
  - `build_authoring_ir_core`,
  - `update_authoring_ir_core`,
  - `render_group_html_core`.
- `src-tauri/src/lib.rs` now keeps Tauri command wrappers and shared app wiring for this domain, which preserves command names and macro scope while reducing backend monolith size.
- The user clarification is now an explicit product invariant: `Questions 14-26` / `Questions 14–26` at the start of Passage 2 is a valid umbrella question-range indicator. It must be preserved under `umbrellaQuestionRanges`, not discarded as instruction noise.
- When later concrete subgroups exist, the umbrella range must not be duplicated as a concrete Q14-Q26 group. When only the umbrella range exists, the app creates a low-confidence `requiresManualQuestionImport` scaffold and keeps the item in AuthoringReview.

### PDF Upload Chain Finding
- The current tested chain matches the intended strategy:
  - text-layer PDF/DOCX/TXT/MD use deterministic Rust parsing and can reach split/AuthoringIR;
  - image/no-text or mixed PDF pages are eligible for vision transcription and require SourceReview before publish;
  - manual and vision transcriptions can produce DocumentIR and reach split, but vision output remains review-gated.
- This confirms the production direction: no bundled local OCR engine is required for MVP/macOS; visual OCR replacement belongs to the LLM vision path plus mandatory human verification.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo check)` | pass |
| `(cd src-tauri && cargo test umbrella -- --nocapture)` | pass, 3 tests |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `(cd src-tauri && cargo test complex_ -- --nocapture)` | pass, 4 tests |
| `(cd src-tauri && cargo test transcription -- --nocapture)` | pass, 2 tests |
| `(cd src-tauri && cargo test preview -- --nocapture)` | pass, 3 tests |
| `(cd src-tauri && cargo test runtime_gate -- --nocapture)` | pass, 3 tests |
| `(cd src-tauri && cargo test validation_job_state -- --nocapture)` | pass |
| `(cd src-tauri && cargo test)` | pass, 65 tests |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Remaining Risk
- `authoring_commands.rs` is still JSON-heavy at the boundary with `authoring_pipeline.rs`, `parser.rs`, and `reading_source.rs`. The module split improves ownership, but typed DocumentIR/SplitCandidate/AuthoringIR structs are still needed.
- Live vision-provider behavior is not covered by these tests. Current verification proves routing, safety gates, and transcription DocumentIR semantics, not external model quality.

## Implementation Findings: 2026-06-01 / E8-62

### Job/Import/Settings Command-Core Boundary
- `src-tauri/src/job_commands.rs` now owns the command-core layer for local job and import operations:
  - create/list/get/update/delete job,
  - import source file,
  - reveal job folder,
  - choose export directory,
  - list LLM profiles,
  - environment preflight,
  - diagnostics settings load/save.
- `src-tauri/src/lib.rs` keeps the Tauri command wrappers and app setup, preserving frontend command names and handler macro scope.
- App directory setup and file import helpers moved into `src-tauri/src/util.rs`, so file-type detection, filename sanitization, source hashing, app directory creation, and job directory creation are in one utility boundary.
- `parser.rs` and `llm_profiles.rs` now import `environment::{command_failure, find_sidecar}` directly, reducing root-level coupling.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo check)` | pass |
| `(cd src-tauri && cargo test job -- --nocapture)` | pass, 3 tests |
| `(cd src-tauri && cargo test environment_preflight_reports_required_dependency_names -- --nocapture)` | pass |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `(cd src-tauri && cargo test complex_ -- --nocapture)` | pass, 4 tests |
| `(cd src-tauri && cargo test)` | pass, 65 tests |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Remaining Risk
- Further mechanical extraction from `lib.rs` may have diminishing returns because Tauri command wrappers and shared public DTOs must remain discoverable. The next architecture improvement should prioritize typed domain models for DocumentIR/SplitCandidate/AuthoringIR/validation reports.
- Job/import command tests mostly prove storage/path safety indirectly through existing fixtures. Broader UI E2E for actual desktop file picker behavior remains outside current automated coverage.

## Implementation Findings: 2026-06-01 / E8-63

### Typed Domain Seam: SourceReviewV1
- `SourceReviewV1` is now a real Rust struct in `src-tauri/src/source_review.rs`.
- `source_review_status` constructs the typed struct and serializes it back to the same JSON contract used by the rest of the app.
- `write_source_review_status` round-trips through the typed struct before persisting.
- `source_review_issues` can parse either typed or legacy JSON input, so the seam is backward-compatible with existing saved fixtures and tests.

### Why This Seam Matters
- Source review is the hard publish gate for parser warnings and low-confidence PDF paths. Typing this boundary reduces the risk of field drift where `resolvedAt`, `note`, or the `parserWarnings` / `lowConfidenceBlocks` arrays are renamed or malformed.
- The seam is narrow enough to verify thoroughly without changing the frontend contract or the existing persistence file format.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test source_review_status_preserves_v1_json_contract -- --nocapture)` | pass |
| `(cd src-tauri && cargo test source_review -- --nocapture)` | pass, 7 tests |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `(cd src-tauri && cargo test)` | pass, 66 tests |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Remaining Risk
- Only one typed seam is in place. `DocumentIR`, `SplitCandidates`, `ReadingAuthoringIR`, and validation/export report structures still rely heavily on `serde_json::Value`.
- The current seam proves the pattern, but it does not yet remove the broader JSON drift risk in the main pipeline.

## 2026-06-01 / E8-64 Findings

### SplitCandidates Typed Seam
- `make_dynamic_split_candidates` now constructs typed DTOs before returning the existing JSON contract. This preserves frontend compatibility while reducing backend field-name drift risk.
- The DTO coverage includes `SplitCandidatesV1`, `PassageCandidateV1`, `SplitGroupCandidateV1`, `UmbrellaQuestionRangeV1`, and `AnswerKeyCandidateV1`.
- Optional manual-review flags on split groups use `skip_serializing_if = "Option::is_none"`, so ordinary concrete groups keep the same compact JSON shape while umbrella-only scaffolds still expose `isUmbrellaRange` and `requiresManualQuestionImport`.

### Opening `Questions 14-26` / `Questions 14–26` Product Rule
- The opening total question range is valid metadata and must be preserved in `umbrellaQuestionRanges`.
- It must not become a duplicate concrete Q14-Q26 group when later concrete subgroups such as Q14-19, Q20-23, and Q24-26 are available.
- If the opening range is the only question range recognized after PDF extraction or vision transcription, the app must create a low-confidence manual import scaffold and block publish/release until concrete prompts are manually imported and reviewed.

### Verification Snapshot
| Command | Result |
|---------|--------|
| `(cd src-tauri && cargo test umbrella -- --nocapture)` | pass, 4 tests |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `(cd src-tauri && cargo test)` | pass, 67 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

## 2026-06-01 / E8-65 Findings

### ReadingAuthoringIR Typed Seam
- `make_dynamic_authoring_ir` now constructs `ReadingAuthoringIrV1` typed DTOs before serializing to the existing JSON contract.
- Stable contract fields are typed: `schemaVersion`, `jobId`, `exam`, `passage`, `groups`, `questions`, `answerKey`, `questionOrder`, `questionDisplayMap`, and `audit`.
- Flexible DSL fields remain JSON values by design: `interaction` and `layout` still allow future controls/templates without requiring immediate Rust enum churn.
- `answerKey`, `questionOrder`, and `questionDisplayMap` are now derived from typed groups/questions, reducing the risk of mismatched ids or display numbers during generation.

### Review Gate Boundary
- Structural validation and publish readiness are intentionally different gates.
- Umbrella-only `Questions 14-26` scaffolds may pass structural validation because the generated source shape is valid, but they remain blocked by AuthoringReview/publish readiness until manual prompt import and verification are complete.
- This is the correct product behavior for scanned/partial PDF cases: the app can preserve recoverable structure while refusing to publish unverified concrete question prompts.

### Verification Snapshot
| Command | Result |
|---------|--------|
| `(cd src-tauri && cargo test reading_authoring_ir_v1_preserves_manual_import_contract -- --nocapture)` | pass |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `(cd src-tauri && cargo test)` | pass, 68 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

## 2026-06-01 / E8-66 Findings

### ReadingExamSourceV1 Typed Seam
- `reading_source(authoring)` now constructs `ReadingExamSourceV1` typed DTOs before serializing to the existing JSON contract.
- The passage block contract is now normalized explicitly to `{ blockId, kind: "html", html }`, which matches the frontend/runtime `ReadingExamSourceV1` type definition.
- The export/runtime boundary continues to work with existing preview, runtime validation, and pack/export consumers because the external JSON shape is unchanged.

### Verification Snapshot
| Command | Result |
|---------|--------|
| `(cd src-tauri && cargo test reading_source_v1_preserves_export_contract -- --nocapture)` | pass |
| `(cd src-tauri && cargo test reading_source_uses_real_source_metadata_and_review_status -- --nocapture)` | pass |
| `(cd src-tauri && cargo test)` | pass, 69 tests |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

## 2026-06-01 / E8-67 Findings

### ValidationReport Typed Seam
- `validate_authoring` now emits `ValidationReportV1` through typed Rust DTOs while preserving the existing frontend JSON contract.
- `validation_layers` now returns typed `ValidationLayerReportV1` entries, keeping `issueCount`, `errorCount`, and `warningCount` stable across AuthoringIR, ReadingExamSourceV1, DomProtocol, and RuntimePreview layers.
- The `runtime` field intentionally remains an optional JSON extension because real-runtime diagnostics and fallback reports can add variable provider-specific payloads.

### Verification Snapshot
| Command | Result |
|---------|--------|
| `(cd src-tauri && cargo test validation_report_v1_preserves_static_runtime_contract -- --nocapture)` | pass |
| `(cd src-tauri && cargo test runtime_gate -- --nocapture)` | pass, 3 tests |
| `(cd src-tauri && cargo test preview -- --nocapture)` | pass, 3 tests |
| `(cd src-tauri && cargo test)` | pass, 70 tests |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

## 2026-06-01 / E8-68 Findings

### Auto-Pipeline Publish Safety
- Rust production auto-pipeline already considered `remaining_authoring_review` when projecting status, so umbrella-only/manual-import drafts do not become `ExportReady` even if structural validation and static runtime contract pass.
- Added an explicit regression for umbrella-only `Questions 14–26` auto-pipeline behavior. The pipeline report can show `validationPassed=true` and `staticRuntimePassed=true`, but final status remains `NeedsReview` while `authoring.remainingReviewItems > 0`.
- Browser dev fallback was weaker: it did not include AuthoringReview in `run_auto_pipeline` status projection. It now mirrors Rust by using `refreshReviewState` and adding `authoring.remainingReviewItems` to the report.

### Verification Snapshot
| Command | Result |
|---------|--------|
| `(cd src-tauri && cargo test auto_pipeline_blocks_umbrella_only_manual_import_from_export_ready -- --nocapture)` | pass |
| `(cd src-tauri && cargo test auto_pipeline -- --nocapture)` | pass, 4 tests |
| `(cd src-tauri && cargo test)` | pass, 71 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

## 2026-06-01 / E8-69 Findings

### Opening Question Range Product Semantics
- User clarified that opening instructions like `Questions 14-26` / `Questions 14–26` are correct question-group information, even when they are presented as passage-level setup rather than a concrete interactive subgroup heading.
- The correct model is two-level:
  - Opening full-passage range is retained as umbrella metadata and review context.
  - Concrete later subgroups such as `Questions 14-19`, `Questions 20-23`, and `Questions 24-26` remain the publishable interaction groups when present.
  - If only the umbrella range exists, the app still creates a low-confidence `requiresManualQuestionImport` scaffold and blocks publish until concrete prompts are manually entered/verified.

### Implementation Finding
- Previously, the umbrella range was visible in `SplitCandidatesV1.umbrellaQuestionRanges`, but it did not reliably survive into later AuthoringIR/export metadata.
- `ReadingAuthoringIRV1.passage.questionUmbrellaRanges` now carries the opening range forward after AuthoringIR generation.
- `ReadingExamSourceV1.meta.questionUmbrellaRanges` and `meta.questionIntroHtml` now preserve and render the opening range in preview/export source metadata.
- Browser dev fallback and frontend template rendering now mirror the Rust path, preventing production/dev drift.
- GroupEditor and UnifiedPreview now show the preserved opening total range, so users can see that the opening instruction was included instead of discarded.

### Verification Snapshot
| Command | Result |
|---------|--------|
| `(cd src-tauri && cargo test opening_umbrella_range_is_included_without_replacing_concrete_groups -- --nocapture)` | pass |
| `(cd src-tauri && cargo test umbrella -- --nocapture)` | pass, 5 tests |
| `(cd src-tauri && cargo fmt --check && cargo test)` | pass, 71 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

## 2026-06-01 / E8-70 Findings

### Opening `Questions 14-26` Reconfirmation
- User reconfirmed that `Questions 14-26` / `Questions 14–26` at the start of the passage is a valid题组范围, just presented differently from later concrete subgroups.
- Current production behavior matches this rule:
  - Full-sentence openings such as `Questions 14–26 are based on Reading Passage 2 below` are detected as umbrella question ranges.
  - Standalone extracted headings such as `Questions 14–26` near `READING PASSAGE 2` are also detected as umbrella question ranges.
  - The opening range is preserved as metadata in split, AuthoringIR, ReadingExamSource, preview, and export context.
  - Concrete later groups remain the publishable interaction groups when available.
  - If the opening range is the only detected question structure, the app creates a low-confidence `requiresManualQuestionImport` scaffold and keeps AuthoringReview blocking export.

### Verification Snapshot
| Command | Result |
|---------|--------|
| `npm run check` | pass |
| `(cd src-tauri && cargo test umbrella -- --nocapture)` | pass, 5 tests |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 72 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `git diff --check` | pass |

## 2026-06-01 / E8-71 Findings

### Browser UI-Flow Diagnostic Coverage
- Added `sidecars/ui-flow-e2e/ui-flow-e2e.mjs` as a development/CI diagnostic script. It uses host Chrome/Chromium through the DevTools Protocol directly, so no Playwright/Puppeteer dependency is added and no browser automation runtime is bundled into production.
- Added `npm run e2e:ui-flow` as the entrypoint.
- The diagnostic currently covers:
  - Clear-text import through ImportWizard and auto-pipeline.
  - Low-confidence dev LLM fallback routing to `LlmReview`.
  - Static runtime validation evidence present as `runtime.mode = static-rust`.
  - OCR/scanned-like import through `parseMode=ocr`.
  - Vision transcription application in dev fallback.
  - SourceReview-first routing to `DocumentReview` when parser/vision review is required.

### Drift Fixed
- The first UI-flow run found that dev fallback `mergeValidationReports` kept RuntimePreview layer pass/fail counts but dropped the `runtime` extension object, so UI diagnostics could not observe `runtime.mode = static-rust`.
- `src/services/devFallbackBackend.ts` now preserves `sidecar.runtime ?? base.runtime` when merging validation reports.
- This is a dev fallback alignment fix, not a production gate change. Rust production validation reports already preserve runtime evidence.

### Verification Snapshot
| Command | Result |
|---------|--------|
| `npm run e2e:ui-flow` | pass; clear text routed to `LlmReview` with `runtimeMode=static-rust`; OCR routed to `DocumentReview` with SourceReview `required` and `visionApplied=true` |
| `npm run check` | pass |
| `(cd src-tauri && cargo fmt --check && cargo test)` | pass, 72 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `git diff --check` | pass |

## 2026-06-01 / E8-72 Findings

### UI E2E Expanded To Review/Preview/Export/Pack
- The UI-flow diagnostic now proves more than initial routing:
  - Clear-text import reaches low-confidence LLM review.
  - A simulated human verification step marks all AuthoringIR questions verified and preserves answer key/order/display-map consistency.
  - GroupEditor validation routes to UnifiedPreview.
  - Preview assets and RuntimePreview diagnostics are generated through the UI.
  - Export emits four files in the UI (`json`, exam wrapper JS, `manifest.js`, `preview.html`) and then cleanup advances the job to `Cleaned`.
  - PackBuilder can select the exported/cleaned job and build a pack result.
- Added stable `data-testid` hooks to GroupEditor, UnifiedPreview, and PackBuilder for those user-visible actions.
- The OCR/scanned path remains intentionally blocked at SourceReview, validating that image/vision output does not skip human source verification.

### Verification Snapshot
| Command | Result |
|---------|--------|
| `npm run e2e:ui-flow` | pass; clear text review/preview/export/pack completed with finalStatus `Cleaned`, `runtimeMode=static-rust`, `exportedFileCount=4`, `packBuilt=true`; OCR remained `NeedsReview` at `DocumentReview` with SourceReview `required` and `visionApplied=true` |
| `npm run check` | pass |
| `(cd src-tauri && cargo fmt --check && cargo test)` | pass, 72 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `git diff --check` | pass |

## 2026-06-01 / E8-73 Findings

### Scanned PDF Manual Recovery Path
- The UI-flow diagnostic now covers the full scanned/image-PDF recovery path:
  - `parseMode=ocr` causes vision transcription to be attempted/applied in dev fallback.
  - The job remains `NeedsReview` at `DocumentReview` with SourceReview required.
  - The author can paste manual transcription text through DocumentReview.
  - The manual transcription creates a `manual-transcription` DocumentIR and resolves source review risk for the test fixture.
  - Rule split and AuthoringIR generation work from that manual text.
  - Human verification then allows Preview, RuntimePreview diagnostic, Export, and Pack.
- This matches the intended product workflow: scanned/vision output is never silently trusted; manual or human-verified transcription can become publishable only after review gates are satisfied.

### Verification Snapshot
| Command | Result |
|---------|--------|
| `npm run e2e:ui-flow` | pass; OCR path initial SourceReview `required`, `visionApplied=true`, manual provider `manual-transcription`, finalStatus `Cleaned`, `runtimeMode=static-rust`, `exportedFileCount=4`, `packBuilt=true`; clear text path also passed |
| `npm run check` | pass |
| `(cd src-tauri && cargo fmt --check && cargo test)` | pass, 72 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `git diff --check` | pass |

## Implementation Findings: 2026-06-01 Cross-Platform API Key Storage

### Secret Storage Product Rule
- User clarified that API-key storage cannot overfit to macOS Keychain because the authoring app also targets Windows.
- The implementation now uses the Rust `keyring` crate as the default OS secure storage adapter instead of shelling out to `/usr/bin/security` directly.
- Runtime semantics:
  - macOS: system Keychain via `keyring`.
  - Windows: Credential Manager via `keyring`.
  - Other desktop platforms: system keyring/secret-service where available.
  - Plaintext app-data secret files remain disabled unless `EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK=1` is explicitly set.
- UI copy now says “系统安全存储” and lists macOS Keychain / Windows Credential Manager / system keyring instead of implying a macOS-only production path.
- Environment preflight now includes `security:os-secret-storage` plus the existing plaintext fallback warning.

### Verification Evidence
| Command | Result |
|---------|--------|
| `npm run check` | pass |
| `(cd src-tauri && cargo fmt --check)` | pass after formatting |
| `(cd src-tauri && cargo test)` | pass, 72 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `git diff --check` | pass |

### Residual Risk
- This machine verifies macOS compilation and behavior only. Windows Credential Manager must still be smoke-tested on Windows before claiming cross-platform release readiness.

### Keyring Feature Audit
- `keyring` 3.6.3 does not enable native platform backends through default features alone.
- `src-tauri/Cargo.toml` now explicitly enables `apple-native`, `windows-native`, `linux-native-sync-persistent`, and `crypto-rust`.
- `cargo tree -e features -i keyring` verified that the compiled feature graph includes all three production backend families.

### Real Provider Smoke Evidence
- User-supplied OpenAI-compatible endpoint/model was tested without persisting the API key into repository files.
- Text path returned HTTP 200 and valid JSON content.
- Vision path accepted an OpenAI-style `image_url` message and returned HTTP 200 with image token usage.
- Next deeper provider validation should run the Rust `run_llm_gateway`/auto-pipeline path against a real rendered PDF page, but the provider capability itself is no longer unverified.
