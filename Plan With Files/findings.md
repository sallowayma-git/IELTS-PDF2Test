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
| 导出/Pack | 默认 strict real-runtime gate；Pack UI 只列 `ExportReady`；后端导出/Pack 调 `publish_readiness_gate` | P0 硬阻断已补齐 |

### Code Quality Findings
| ID | Severity | Finding | Evidence |
|----|----------|---------|----------|
| CQ-01 | P0 fixed | 发布门禁已统一检查 `NeedsHumanReview`、parser warnings、low-confidence blocks、未验证问题/答案 | `publish_readiness_gate` 已接入 `export_reading_assets` 和 `build_pack` |
| CQ-02 | P0 fixed | parser sidecar 失败时不再对 PDF/DOCX 生成 sample Document IR | `parser_failure_document_ir` 生成 failure IR |
| CQ-03 | P0 fixed | `generate_preview_assets` 已先校验，并在仍需人工审核时保持 `NeedsHumanReview` | 预览状态推进已修正 |
| CQ-04 | P1 | `validate_authoring_ir` 失败时没有更新 `current_step`，通过时只设 `PreviewReady` 不代表真实 runtime | 状态语义比自动流水线弱 |
| CQ-05 | P1 | `choose_export_dir` Rust command 返回 `None`，实际选择依赖前端 Tauri plugin；后端 command 未完成 | 后端 API 与设计文档不完全一致 |
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
| Preview/runtime | Built-in validation + Node validator + runtime E2E sidecar; export/Pack default to strict real-runtime mode. | Strong safety baseline. Packaging still relies on external runtime/node/python dependencies. |
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
