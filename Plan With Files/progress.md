# Progress Log

## Session: 2026-05-29

### Phase 1: Requirements & Discovery
- **Status:** complete
- Actions taken:
  - 确认当前活动 goal 已存在，沿用该目标推进。
  - 读取 `planning-with-files` 技能说明。
  - 执行 session catchup，未发现输出的未同步上下文。
  - 扫描工作区，确认当前只有两份 Epic8 设计文档。
  - 创建 `Plan With Files/task_plan.md`、`findings.md`、`progress.md`。

### Phase 2: Architecture & Scaffold
- **Status:** complete
- Actions taken:
  - 决定 Tauri/Rust/内嵌界面技术栈与目录结构。
  - 初始化 package、src、src-tauri 等工程文件。
  - 建立共享类型、API 调用封装与基础路由。

### Phase 3: Core Local Backend
- **Status:** complete
- Actions taken:
  - 实现 Rust 数据模型、存储、命令 API。
  - 实现导入、解析骨架、粗切、校验、导出服务。
  - 将规则粗切与 Authoring IR 生成改为优先从 `DocumentIRV1` 动态推导。
  - 将 TXT/MD Python parser sidecar 与 Node validator sidecar 接入 Rust command 优先路径。
  - 建立 Rust 工具链后的命令级验证。
  - 建立桌面文件选择器与导出目录选择 UI。
  - 添加 txt/md parser sidecar 与 Node validator sidecar 入口。
  - 添加 PDF/DOCX deterministic parser adapter：PDF 走 `pypdf`，DOCX 走 Python stdlib OOXML 解析。

### Phase 4: Authoring UI
- **Status:** in_progress
- Actions taken:
  - 实现 9 个页面的主要交互骨架。
  - 接通页面到 Tauri command/dev fallback 的主流程。
  - 添加本地 RuntimePreview contract simulator，执行生成的 manifest/wrapper 并校验自动填正确答案 100%。
  - 完成真实/模拟 runtime 状态语义修正，区分 PreviewReady 与 ExportReady。
  - 完成导入页、LLM 审阅页、DocumentReview、JobList、PackBuilder 的风险边界修订。
- Remaining:
  - 真实 unified runtime / OCR / scanned PDF 复杂场景的最终接入与验证。

### Phase 5: Verification & Completion
- **Status:** in_progress
- Actions taken:
  - 实现上传后自动流水线：自动解析、粗切、生成 AuthoringIR、批量 LLM 建议、高置信落库、低置信待审、校验/E2E。
  - 修复关键门禁策略：发布链路默认 Rust 静态合同 gate + SourceReview/AuthoringReview，真实 runtime E2E 为诊断项。
  - 修复配置移植性风险：移除代码内本机绝对路径默认值，仅保留 `EPIC8_UNIFIED_HTML_PATH`/`EPIC8_UNIFIED_PYTHON` 注入。
  - 强化低置信与人工审核边界：LLM fallback 置信度统一降级、parser warnings / low-confidence blocks 进入人工审核。
  - 完成 Tauri release build 与前端 build / Rust lint / sidecar 语法 / browser smoke 验证。
  - 修正工程追踪文档中的过度乐观状态，恢复真实风险边界。
- Remaining:
  - 完成最终验收清单中的真实 unified runtime / OCR 最终接入验证。

## Test Results
| Test | Input | Expected | Actual | Status |
|------|-------|----------|--------|--------|
| Planning files exist | `test -f ...` | 三份文件存在 | 三份文件存在 | pass |
| Frontend production build | `npm run build` | TypeScript + Vite build pass | Build passed, assets emitted under `dist/` | pass |
| Rust format/check/lint | `cargo fmt --check && cargo check && cargo clippy --all-targets -- -D warnings` | No Rust compile/lint failures | Passed | pass |
| Full Tauri build | `npm run tauri build` | Release binary and macOS bundles generated | `.app` and `.dmg` generated under `src-tauri/target/release/bundle` | pass |
| Sidecar syntax checks | `node --check` / `python3 -m py_compile` | sidecars parse/execute cleanly | Passed | pass |
| LLM fallback smoke | `gateway.mjs extract_group ...` with no API key | confidence below auto-apply threshold | confidence `0.64` and fallback warnings present | pass |
| Browser smoke: import page | `#/jobs/new` | submit disabled without file, production boundary visible | passed | pass |
| Browser smoke: pack page | `#/packs` | only ExportReady is publishable | passed | pass |
| Browser smoke: LLM page | `#/jobs/nonexistent/llm-review` | page renders | passed | pass |

## Deep Audit Log: 2026-05-31 13:05 CST

### Scope
- 审计设计文档要求与当前实现的一致性。
- 重点链路：上传 PDF/DOCX -> parser/切分 -> LLM 识别 -> 高置信自动落库 -> 低置信人工审核 -> 预览/E2E -> 导出/Pack。
- 重点细节：状态机、字段、人工确认、低置信、真实 runtime、sidecar fallback、测试覆盖。

### Evidence Read
- `Files/Epic8-Tauri作者端应用详细设计.md`: 核心流程、状态机、MVP 范围。
- `Files/Epic8-作者端Web导题与组卷器工程设计.md`: 输出契约、LLM 边界、四层校验、关键验收用例。
- `src-tauri/src/lib.rs`: Rust 命令、数据模型、pipeline、parser、LLM、validator、export、pack。
- `sidecars/python-parser/parser.py`: TXT/MD/PDF/DOCX deterministic parser。
- `sidecars/llm-gateway/gateway.mjs`: LLM JSON patch gateway。
- `sidecars/node-validator/validate-reading-source.mjs`: ReadingExamSourceV1 + DOM validator。
- `sidecars/preview-e2e/preview-e2e.mjs`: fallback simulator + external unified runtime runner。
- `src/pages/*.tsx`, `src/types/*.ts`, `src/services/devFallbackBackend.ts`: UI 路由、字段、dev fallback。

### Findings
- Product state: 可运行 MVP 原型，不是最终生产发布态。
- P0: `export_reading_assets` / `build_pack` 没有统一回查 parser warnings、low-confidence blocks、`NeedsHumanReview`、`verified=false` 和 `audit.humanVerified=false`。
- P0: PDF/DOCX parser sidecar 失败时，Rust 仍可能生成 sample Document IR，存在把真实上传失败替换成演示内容的风险。已于 2026-05-31 13:26 CST 修复为 failure Document IR。
- P0: 低置信人工审核路径有 UI 可见性，但缺少“人工确认完成后才可发布”的后端硬状态闭环。
- P1: OCR 只是 `mode=ocr` 重跑当前 parser，无真实 OCR adapter；no-text PDF 可以低置信进入人工，但没有自动化 fixture 验证不能发布。
- P1: 真实 unified runtime runner 已写入 sidecar，但尚未完成外部运行时最终验收。
- P1: Rust 核心后端单文件过大，业务边界混杂，继续扩展复杂 PDF/OCR 会增加回归风险。
- P1: 仓库没有自有自动化测试/fixture，当前主要依赖 build/lint/smoke。

### Plan Updates
- 更新 `task_plan.md`：新增 E8-11/E8-12，修正 Phase 4/5 过度乐观表述。
- 更新 `findings.md`：记录业务链路、字段契约、代码质量、下一步实现顺序。
- Goal 不标记 complete；真实 runtime/OCR/发布硬门禁仍需继续实现。

## Session: 2026-05-31 13:26 CST

### Phase 5 / E8-11: Publish Readiness Gate
- **Status:** in_progress, P0 subitems implemented
- Actions taken:
  - 修改 Rust parser fallback：非 TXT/MD parser 失败不再生成 sample 题，改为 failure Document IR。
  - 新增后端人工确认派生逻辑：`refresh_authoring_review_state` 根据 confidence、answer、verified 推导 `audit.humanVerified` 与 needsReview。
  - 新增统一发布门禁：`publish_readiness_gate` 在导出/Pack 前阻断未人工确认、低置信未确认、空答案、parser warning、low-confidence blocks、`NeedsHumanReview` 和非真实 runtime。
  - 修正 `run_preview_e2e` 状态推进：只有真实 runtime + readiness gate 都通过才进入 `ExportReady`。
  - 修正 `generate_preview_assets` 状态推进：未完成审核时不再误标 `PreviewReady`。
  - 修正 `run_auto_pipeline`：高置信自动应用的题组/题目标记 verified，低置信仍进入人工审核。
  - 同步 dev fallback 的 readiness 语义，避免前端开发环境误判导出/Pack。
  - 新增 Rust 单元测试覆盖 parser failure、低置信人工确认和空答案发布阻断。

### Verification
| Test | Status |
|------|--------|
| `npm run check` | pass |
| `npm run build` | pass |
| `cargo fmt --check` | pass |
| `cargo test` | pass, 3 tests |
| `cargo clippy --all-targets -- -D warnings` | pass |
| `node --check sidecars/llm-gateway/gateway.mjs` | pass |
| `node --check sidecars/preview-e2e/preview-e2e.mjs` | pass |
| `node --check sidecars/node-validator/validate-reading-source.mjs` | pass |
| `python3 -m py_compile sidecars/python-parser/parser.py` | pass |
| `git diff --check` | pass |

### Remaining
- 真实 unified runtime E2E 仍需接入执行。
- OCR/scanned PDF fixture 仍需补齐。
- Rust backend 模块拆分与更系统的 integration tests 仍未完成。

## Error Log
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-05-29 | `create_goal` failed: active goal already exists | 1 | 使用已有 goal 继续推进 |
| 2026-05-31 | `cargo fmt --check` failed on formatting | 1 | Run `cargo fmt` and rerun checks |
| 2026-05-31 | `cargo clippy` flagged identical `if` branches | 1 | Simplified `next_step` fallback logic |
| 2026-05-31 | Initial import flow still allowed demo-style fallback path | 1 | Changed production import to fail on unreadable source file; demo flow stays isolated |

## 5-Question Reboot Check
| Question | Answer |
|----------|--------|
| Where am I? | Phase 5: Verification & Completion |
| Where am I going? | Finish remaining verification and the final audit of true runtime / OCR risk |
| What's the goal? | 实现 Epic8 Tauri 作者端本地应用全部开发任务并维护工程追踪 |
| What have I learned? | 主链路已通，但真实 unified runtime / OCR 仍是最后的高风险边界 |
| What have I done? | 已完成自动流水线、状态语义修正、LLM fallback 降置信、前端/后端/sidecar 门禁与冒烟验证 |

## Session: 2026-05-31 13:38 CST

### Deep Architecture and Business Audit
- **Status:** complete for audit pass; implementation fixes still pending.
- Actions taken:
  - Re-read Plan With Files state and aligned current phase with remaining E8-06/E8-07/E8-10/E8-11/E8-12 work.
  - Re-read design-document headings and output-contract requirements for local app, runtime contract, LLM boundary, four-layer validation, Pack/export, and file permissions.
  - Audited `src-tauri/src/lib.rs` across status model, import, parser fallback, split, Authoring IR, LLM gateway, suggestion application, validation, runtime gate, publish readiness, export, Pack, and tests.
  - Audited sidecars: Python parser, LLM gateway, Node validator, and preview E2E runner.
  - Audited frontend pages and types for ImportWizard, DocumentReview, SplitAndAnswers, GroupEditor, LlmReview, UnifiedPreview, PackBuilder, ExportPage, Settings, desktop dialogs, and dev fallback.
  - Identified new P0 risks around parser-warning readiness bypass and unrestricted workflow status mutation.

### Verification
| Test | Status |
|------|--------|
| `npm run check` | pass |
| `cargo test` | pass, 3 tests |
| `node --check sidecars/llm-gateway/gateway.mjs` | pass |
| `node --check sidecars/preview-e2e/preview-e2e.mjs` | pass |
| `node --check sidecars/node-validator/validate-reading-source.mjs` | pass |
| `python3 -m py_compile sidecars/python-parser/parser.py` | pass |
| `git diff --check` | pass |

### Audit Result
- Product remains MVP-complete but not production-complete for complex PDF/OCR.
- Highest-priority fixes: add explicit parser/source review provenance, restrict workflow state mutation, remove production sample fallbacks, stop treating high-confidence LLM auto-apply as human verification, then add scanned PDF and real runtime fixture tests.

## Session: 2026-05-31 14:04 CST

### Phase 5 / E8-11: Source Review and State Machine Hardening
- **Status:** complete for this sub-pass.
- Actions taken:
  - Implemented independent `SourceReviewV1` backend state and `source-review.json` persistence.
  - Added `resolve_source_review` command and DocumentReview UI button to explicitly resolve parser warning / low-confidence source review.
  - Changed `publish_readiness_gate` so parser warnings and low-confidence blocks are blocked independently of `audit.humanVerified`.
  - Removed `status` / `currentStep` from Rust and TypeScript `JobMetaPatch`, preventing metadata updates from mutating workflow state.
  - Replaced production sample fallback for missing/unsupported main source with `missing_source_document_ir` and `no-sample-content-generated` warning.
  - Changed high-confidence LLM apply/auto-pipeline behavior to record `autoApplied` rather than setting `verified=true`.
  - Synced dev fallback with the same source review and LLM verification semantics.
  - Added Rust tests covering source-review blocking, missing-source non-sample behavior, and LLM auto-apply not creating human verification.

### Verification
| Test | Status |
|------|--------|
| `npm run check` | pass |
| `npm run build` | pass |
| `cargo fmt --check` | pass |
| `cargo test` | pass, 13 tests |
| `cargo clippy --all-targets -- -D warnings` | pass |
| `node --check sidecars/llm-gateway/gateway.mjs` | pass |
| `node --check sidecars/preview-e2e/preview-e2e.mjs` | pass |
| `node --check sidecars/node-validator/validate-reading-source.mjs` | pass |
| `python3 -m py_compile sidecars/python-parser/parser.py` | pass |
| `git diff --check` | pass |

### Remaining
- Complete command-level export/Pack real-runtime proof.
- Decide OCR adapter vs explicit manual transcription for scanned PDFs.
- Split Rust backend modules for E8-12.


## Session: 2026-05-31 14:30 CST

### Phase 5 / E8-10/E8-11: Fixture and Runtime Verification
- **Status:** complete for this sub-pass.
- Actions taken:
  - Added `fixtures/parser/no-text.pdf` and verified it with `sidecars/python-parser/parser.py`.
  - Added Rust fixture test proving no-text PDF produces parser warning, low-confidence block, and unresolved `SourceReviewV1` issues.
  - Added Rust test proving `ReadingExamSourceV1` derives source metadata and audit status from real imported source provenance.
  - Updated frontend/dev fallback types and `templateRenderer` to keep source metadata behavior consistent with Rust output.
  - Fixed preview E2E runner so real-runtime structured failures are not masked by simulator fallback.
  - Fixed radio wrong-answer E2E generation; external unified runtime minimal fixture now passes in real mode.

### Verification
| Test | Status |
|------|--------|
| `npm run check` | pass |
| `npm run build` | pass |
| `cargo fmt --check` | pass |
| `cargo test` | pass, 13 tests |
| `cargo clippy --all-targets -- -D warnings` | pass |
| `node --check sidecars/preview-e2e/preview-e2e.mjs` | pass |
| `node --check sidecars/llm-gateway/gateway.mjs` | pass |
| `node --check sidecars/node-validator/validate-reading-source.mjs` | pass |
| `python3 -m py_compile sidecars/python-parser/parser.py` | pass |
| no-text PDF parser smoke | pass, warning + confidence 0.2 |
| external unified runtime minimal E2E | pass, `mode=real`, correct score 100%, wrong sample 50% |
| `git diff --check` | pass |

### Remaining
- OCR adapter is still not implemented; current scanned/no-text PDF strategy is hard-stop/manual review.
- Command-level export/Pack core fixture with real runtime now passes; broader pipeline fixture still missing.
- Rust backend module decomposition remains open.

### Error Log Additions
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-05-31 14:18 CST | Batch edit failed because command ran from `src-tauri` while targeting `src-tauri/src/lib.rs` | 1 | Switched to `apply_patch` with absolute paths. |
| 2026-05-31 14:20 CST | `cargo test` invoked with two test-name filters | 1 | Ran full `cargo test` instead. |
| 2026-05-31 14:27 CST | preview E2E hid real-runtime structured failure behind simulator fallback | 1 | Changed `validateRuntime` to return real-runtime structured report when available. |
| 2026-05-31 14:28 CST | real runtime failed because wrong-answer radio sample fell back to first option and stayed correct | 1 | Changed radio filler to choose a different valid radio option when answer ends with `__wrong__`. |


## Session: 2026-05-31 14:45 CST

### Phase 5 / E8-10: Command-Level Export and Pack Runtime Gates
- **Status:** complete for this sub-pass.
- Actions taken:
  - Extracted `export_reading_assets_core` and `build_pack_core` from Tauri command wrappers.
  - Added publishable fixture helper that writes job/document/split/authoring state and resolved source review to a temp app root.
  - Added real-runtime export test verifying strict gate, output JSON/JS/manifest/report files, and `ExportReady` status.
  - Added real-runtime Pack test verifying zip output, pack manifest, entry count, and `Published` status.
  - Fixed `merge_sidecar_validation` so the `runtime` field from preview E2E sidecar is preserved in the validation report.

### Verification
| Test | Status |
|------|--------|
| `cargo test` with `EPIC8_UNIFIED_HTML_PATH` and `EPIC8_UNIFIED_PYTHON` | pass, 13 tests |
| `cargo fmt --check` | pass |
| `cargo clippy --all-targets -- -D warnings` | pass |
| `npm run check` | pass |
| `npm run build` | pass |
| sidecar syntax checks | pass |
| `git diff --check` | pass |

### Error Log Additions
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-05-31 14:42 CST | Export/Pack core tests failed because strict gate saw `runtime.mode=unknown` | 1 | Preserved sidecar `runtime` object in `merge_sidecar_validation`; tests passed. |
| 2026-05-31 14:43 CST | Clippy `needless_borrow` after extracting Pack core | 1 | Changed `build_pack_manifest(&input, ...)` to `build_pack_manifest(input, ...)`. |


## Session: 2026-05-31 15:00 CST

### Phase 5 / E8-10: Complex PDF/DOCX Fixture Coverage
- **Status:** complete for clear-layout parser fixture sub-pass.
- Actions taken:
  - Added `fixtures/parser/complex-reading.pdf` with passage text, two question groups, a table-like section, and answer key.
  - Added `fixtures/parser/complex-reading.docx` as minimal OOXML with paragraphs, table, and answer key.
  - Added Rust fixture tests for PDF and DOCX parser -> split -> AuthoringIR -> answerKey.
  - Confirmed no parser warnings or low-confidence blocks for both clear-layout fixtures.

### Verification
| Test | Status |
|------|--------|
| `cargo test` with external runtime env | pass, 13 tests |
| `cargo fmt --check` | pass |
| `cargo clippy --all-targets -- -D warnings` | pass |
| `npm run check` | pass |
| `npm run build` | pass |
| sidecar syntax checks | pass |
| `git diff --check` | pass |

### Remaining
- OCR/scanned-image PDF adapter remains unresolved.
- Complex image/flowchart/cross-page table layout recovery still requires manual review or a layout/OCR adapter.
- Rust backend module decomposition remains open.

## Session: 2026-05-31 15:40 CST

### Deep Code Quality and Business Chain Audit
- **Status:** audit pass complete; implementation follow-up pending.
- Actions taken:
  - Re-read Plan With Files state and current design-document anchors.
  - Audited Rust backend architecture, parser/source-review gates, state transitions, LLM gateway, validation, preview/runtime, export, Pack, settings/secrets, Tauri packaging, frontend pages, and sidecars.
  - Refreshed verification evidence with current working tree.
  - Added new audit findings AUD-16 through AUD-25 to `findings.md`.

### Verification
| Test | Status |
|------|--------|
| `npm run check` | pass |
| `npm run build` | pass |
| `cargo fmt --check && cargo clippy --all-targets -- -D warnings` | pass |
| `EPIC8_UNIFIED_HTML_PATH=... EPIC8_UNIFIED_PYTHON=... cargo test` | pass, 13 tests |
| `node --check` for LLM, preview E2E, node validator | pass |
| `python3 -m py_compile sidecars/python-parser/parser.py` | pass |
| `git diff --check` | pass |

### Audit Result
- Backend publish safety is materially improved and currently blocks unresolved source review, unverified authoring, non-real runtime, parser failures, empty answers, and low-confidence fields.
- Full completion remains blocked by OCR/scanned PDF support or explicit manual-transcription product decision, self-contained packaging of sidecar runtimes/dependencies, Rust module decomposition, stricter schema/evidence validation, and UI security hardening around raw HTML preview/CSP.

## Session: 2026-05-31 15:18 CST

### Deep Detail Audit Follow-up
- **Status:** audit pass complete; no product code changed in this sub-pass.
- Actions taken:
  - Re-audited the Rust LLM gateway path, parser/source review fingerprinting, Pack build sequence, Preview/E2E state updates, optional answer-file flow, split/answer UI, visible preview fidelity, validator completeness, provider support, and docs drift.
  - Identified new P0 secret-handling issue: LLM API keys are written into cached gateway input JSON under job cache directories.
  - Added AUD-26 through AUD-38 to `findings.md` and `task_plan.md`.

### Verification
| Test | Status |
|------|--------|
| `npm run check` | pass |
| `npm run build` | pass |
| `EPIC8_UNIFIED_HTML_PATH=... EPIC8_UNIFIED_PYTHON=... cargo test` | pass, 13 tests |
| `cargo fmt --check && cargo clippy --all-targets -- -D warnings` | pass |
| sidecar syntax checks + Python compile | pass |
| `git diff --check` | pass |

### Audit Result
- Product remains a working local MVP with strong backend publish gates.
- Full Epic 8 completion remains blocked by P0 secret persistence, OCR/scanned PDF policy, answer-file/manual split repair completeness, self-contained packaging, backend modularization, LLM evidence validation, and UI security/runtime-preview hardening.

## Session: 2026-05-31 15:59 CST

### Phase 5 / Deep Audit Follow-up Implementation
- **Status:** complete for this sub-pass.
- Actions taken:
  - Fixed `AUD-26` by removing API keys from cached LLM input JSON and passing secrets to the sidecar through `EPIC8_LLM_API_KEY`.
  - Added secret redaction regression tests for cached gateway input and `make_llm_input`.
  - Fixed `AUD-31` by centralizing preview/E2E job-state application and downgrading failed E2E reports to `ValidationFailed`.
  - Fixed `AUD-30` by delaying Pack `Published` status updates until after zip and Pack artifacts are written.
  - Strengthened `SourceReviewV1` fingerprinting for `AUD-32` with parser/source metadata and low-confidence block text hashes.
  - Added AuthoringIR question continuity and duplicate-display validation for `AUD-34`.
  - Fixed stale validation report semantics for `AUD-35` by recomputing layers after warning insertion.
  - Fixed `AUD-36` UI provider mismatch by only exposing OpenAI-compatible provider in Settings.
  - Fixed `AUD-38` README drift by documenting no-sample production parser behavior.

### Verification
| Test | Status |
|------|--------|
| `npm run check` | pass |
| `npm run build` | pass |
| `cargo fmt --check && cargo clippy --all-targets -- -D warnings` | pass |
| `cargo test` | pass, 18 tests |
| `node --check sidecars/llm-gateway/gateway.mjs` | pass |
| `node --check sidecars/preview-e2e/preview-e2e.mjs` | pass |
| `node --check sidecars/node-validator/validate-reading-source.mjs` | pass |
| `python3 -m py_compile sidecars/python-parser/parser.py` | pass |
| `git diff --check` | pass |

### Error Log Additions
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-05-31 15:49 CST | `cargo test` failed because duplicate qid masked numeric gap detection | 1 | Deduped numeric question ids before continuity check. |
| 2026-05-31 15:51 CST | `cargo test` failed because duplicate display-number check used derived `questionDisplayMap`, which overwrote duplicate qids | 1 | Switched duplicate display detection to raw AuthoringIR question list. |

### Remaining
- Build editable split/answer repair and answer-file merge.
- Decide and implement OCR/manual-transcription product policy.
- Add schema/evidence validation for high-confidence LLM suggestions.
- Continue Rust module split and packaging/dependency hardening.

## Session: 2026-05-31 16:17 CST

### Phase 5 / AUD-27-AUD-28 Answer Merge and Manual Repair
- **Status:** complete for this sub-pass.
- Actions taken:
  - Added Rust helpers to collect `AnswerKey` sources, parse them with the parser sidecar, extract answer maps, and merge them into split candidates.
  - Updated `run_rule_split`, `build_authoring_ir` fallback split creation, and `run_auto_pipeline` to include answer-source candidates.
  - Added a Rust regression test proving answer-source text merges into `answerKeyCandidates` and removes the missing-answer issue.
  - Reworked `SplitAndAnswers` from read-only display to editable repair surface for group heading, range, kind, block IDs, instruction, and answer values.
  - Wired the UI to `saveSplitAdjustments` and save-before-build so manual corrections feed AuthoringIR generation.
  - Added minimal CSS for warning/success messages and editable answer rows.
  - Mirrored answer-source behavior in the dev fallback path.

### Verification
| Test | Status |
|------|--------|
| `npm run check` | pass |
| `npm run build` | pass |
| `cargo fmt --check && cargo clippy --all-targets -- -D warnings` | pass |
| `cargo test` | pass, 19 tests |
| `node --check sidecars/llm-gateway/gateway.mjs` | pass |
| `node --check sidecars/preview-e2e/preview-e2e.mjs` | pass |
| `node --check sidecars/node-validator/validate-reading-source.mjs` | pass |
| `python3 -m py_compile sidecars/python-parser/parser.py` | pass |
| `git diff --check` | pass |

### Remaining
- OCR/manual-transcription policy and implementation.
- Stronger LLM evidence/schema validation.
- Runtime/dependency packaging hardening.
- Rust module decomposition and typed domain model extraction.

## Session: 2026-05-31 16:37 CST

### Phase 5 / AUD-17 Manual Transcription Fallback
- **Status:** complete for manual-transcription sub-pass.
- Actions taken:
  - Added `ManualTranscriptionInput` and `apply_manual_transcription` Tauri command.
  - Added `manual_transcription_document_ir` to create a deterministic `DocumentIRV1` from operator-pasted text while recording `parser.provider=manual-transcription`.
  - Updated `DocumentReview` to expose scanned-PDF/OCR-failure manual transcription UI.
  - Added `ManualTranscriptionInput` TS type and `applyManualTranscription` API wrapper.
  - Mirrored manual transcription in dev fallback.
  - Added Rust regression coverage proving manual transcript reaches split answer extraction.

### Verification
| Test | Status |
|------|--------|
| `npm run check` | pass |
| `npm run build` | pass |
| `cargo fmt --check && cargo clippy --all-targets -- -D warnings` | pass |
| `cargo test` | pass, 20 tests |
| `node --check sidecars/llm-gateway/gateway.mjs` | pass |
| `node --check sidecars/preview-e2e/preview-e2e.mjs` | pass |
| `node --check sidecars/node-validator/validate-reading-source.mjs` | pass |
| `python3 -m py_compile sidecars/python-parser/parser.py` | pass |
| `git diff --check` | pass |

### Remaining
- Real OCR/layout adapter remains optional/open depending on product scope; manual transcription fallback is now implemented.
- LLM evidence/schema validation.
- Runtime/dependency packaging hardening.
- Rust module decomposition and typed domain model extraction.

## Session: 2026-05-31 16:58 CST

### Phase 5 / AUD-17 Vision LLM Transcription
- **Status:** complete for this sub-pass.
- Actions taken:
  - Added `extract_pdf_images` command to the Python parser sidecar, producing `PdfImageExtractionV1` metadata and extracted page images.
  - Added `transcribe_pdf_images` to the Node LLM gateway for OpenAI-compatible vision models.
  - Added Rust `VisionTranscriptionInput`, `apply_vision_transcription`, `vision_transcription_document_ir`, and automatic pipeline detection for no-text/low-confidence PDFs.
  - Updated `DocumentReview` with a “视觉 LLM 转录” action while preserving manual transcription fallback.
  - Updated dev fallback behavior and TypeScript types for vision transcription reporting.
  - Added `fixtures/parser/image-only-reading.pdf` and Rust tests for embedded-image extraction and source-review-gated vision transcription.
  - Updated sidecar README and Plan With Files tracking.

### Verification
| Test | Status |
|------|--------|
| `npm run check` | pass |
| `npm run build` | pass |
| `cargo test` | pass, 22 tests |
| `cargo fmt --check && cargo clippy --all-targets -- -D warnings` | pass |
| `node --check sidecars/llm-gateway/gateway.mjs` | pass |
| `node --check sidecars/preview-e2e/preview-e2e.mjs` | pass |
| `node --check sidecars/node-validator/validate-reading-source.mjs` | pass |
| `python3 -m py_compile sidecars/python-parser/parser.py` | pass |
| `python3 sidecars/python-parser/parser.py extract_pdf_images ...` on image-only fixture | pass, extracted 1 image |
| `git diff --check` | pass |

### Remaining
- LLM suggestion schema/evidence validation before high-confidence auto-apply.
- Packaging/dependency self-containment and file-secret fallback hardening.
- Rust backend module split and typed models.
- Actual visual unified-runtime preview or explicit UI limitation label.

## Session: 2026-05-31 17:16 CST

### Phase 5 / AUD-22-AUD-33 LLM Evidence Gate
- **Status:** complete for high-confidence auto-apply path.
- Actions taken:
  - Added Rust `llm_suggestion_auto_apply_issues` validation for confidence, selected paths, patch schema, kind, question IDs, interaction schema, non-fallback evidence, source block IDs, and evidence quotes.
  - Wired the gate into both `apply_llm_suggestion` and `run_auto_pipeline`.
  - Updated automatic pipeline reporting with `blockedAutoApplyGroups`; blocked high-confidence suggestions now route to `LlmReview`.
  - Updated `llm-gateway` prompt/validation to request and normalize `evidence.sourceBlockIds` and `evidence.quotes` while keeping fallback suggestions non-auto-applicable.
  - Updated dev fallback with matching safety semantics.
  - Updated LLM Review UI copy to show blocked-auto-apply context.
  - Added Rust regression tests for high-confidence suggestions with and without source-block evidence.

### Verification
| Test | Status |
|------|--------|
| `npm run check` | pass |
| `npm run build` | pass |
| `cargo test` | pass, 24 tests |
| `cargo fmt --check && cargo clippy --all-targets -- -D warnings` | pass |
| `node --check sidecars/llm-gateway/gateway.mjs` | pass |
| `node --check sidecars/preview-e2e/preview-e2e.mjs` | pass |
| `node --check sidecars/node-validator/validate-reading-source.mjs` | pass |
| `python3 -m py_compile sidecars/python-parser/parser.py` | pass |
| `git diff --check` | pass |

### Remaining
- Packaging/dependency self-containment and file-secret fallback hardening.
- Rust backend module split and typed models.
- Actual visual unified-runtime preview or explicit UI limitation label.

### Error Log Addition
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-05-31 17:31 CST | Ran `cargo fmt --check` from repo root; Cargo.toml is under `src-tauri`. | 1 | Re-run Rust commands with working directory `src-tauri`. |

## Session: 2026-05-31 17:34 CST

### Phase 5 / AUD-18 Environment Preflight
- **Status:** complete for diagnostic preflight sub-pass.
- Actions taken:
  - Added Rust `environment_preflight_report` and `run_environment_preflight` Tauri command.
  - Preflight checks Node.js, python3, pypdf, sidecar script presence, unified runtime env vars, and strict runtime gate state.
  - Added TypeScript `EnvironmentPreflightReport` types and API wrapper.
  - Updated Settings page to load and rerun preflight with visible OK/error/warning rows.
  - Added browser dev fallback preflight report.
  - Added Rust regression test for required preflight check names.
  - Logged command-directory error when `cargo fmt --check` was accidentally run from repo root instead of `src-tauri`.

### Verification So Far
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test)` | pass, 25 tests |
| `npm run check` | pass |

### Remaining This Pass
- Run final `npm run build`, `cargo fmt --check`, `cargo clippy`, sidecar syntax checks, Python compile, and `git diff --check`.
| 2026-05-31 17:39 CST | `git diff --check` found trailing whitespace in `sidecars/README.md`. | 1 | Removed trailing whitespace and reran `git diff --check`; pass. |
| 2026-05-31 17:40 CST | `perl` emitted locale warnings while trimming whitespace. | 1 | Non-blocking host locale warning; command still completed and checks passed. |

## Session: 2026-05-31 17:55 CST

### Phase 5 / AUD-19 Secret Fallback Hardening
- **Status:** complete for default production path.
- Actions taken:
  - Added `plaintext_secret_fallback_allowed` guard using `EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK`.
  - Changed `save_profile_secret` so Keychain failure no longer writes plaintext files unless the guard is enabled.
  - Changed `load_profile_secret` and UI profile redaction so legacy plaintext secret files are ignored unless explicitly opted in.
  - Added preflight check `security:plaintext-secret-fallback`.
  - Updated Settings copy to describe Keychain-only default and dev/emergency plaintext fallback.
  - Added Rust regression tests for default-disabled and explicit opt-in plaintext secret fallback behavior.

### Verification
| Test | Status |
|------|--------|
| `npm run check` | pass |
| `npm run build` | pass |
| `(cd src-tauri && cargo test)` | pass, 27 tests |
| `cargo fmt --check && cargo clippy --all-targets -- -D warnings` | pass |
| `node --check sidecars/llm-gateway/gateway.mjs` | pass |
| `node --check sidecars/preview-e2e/preview-e2e.mjs` | pass |
| `node --check sidecars/node-validator/validate-reading-source.mjs` | pass |
| `python3 -m py_compile sidecars/python-parser/parser.py` | pass |
| `git diff --check` | pass |

### Remaining
- Rust backend module split and typed models.
- Safe path segment validators.
- Actual visual unified-runtime preview or explicit UI limitation label.

## Session: 2026-05-31 18:24 CST

### Phase 5 / AUD-21-AUD-23 Security Hardening
- **Status:** complete for this sub-pass.
- Actions taken:
  - Added Rust `is_safe_path_segment`, `validate_path_segment`, `safe_path_segment`, and `safe_job_dir` helpers.
  - Hardened `load_job`, `save_job`, export, Pack build, LLM secret file fallback, wrapper generation, manifest generation, and preview asset generation against unsafe path/id segments.
  - Added regression tests for unsafe path segments, job traversal IDs, secret profile traversal IDs, unsafe `examId`, and unsafe Pack/job IDs.
  - Replaced `GroupEditor` `dangerouslySetInnerHTML` preview with a sandboxed iframe.
  - Added `sandbox=""` to UnifiedPreview iframe.
  - Replaced Tauri `csp: null` with an explicit CSP.

### Verification
| Test | Status |
|------|--------|
| `npm run check` | pass |
| `npm run build` | pass |
| `(cd src-tauri && cargo test)` | pass, 32 tests |
| `cargo fmt --check && cargo clippy --all-targets -- -D warnings` | pass |
| `node --check sidecars/llm-gateway/gateway.mjs` | pass |
| `node --check sidecars/preview-e2e/preview-e2e.mjs` | pass |
| `node --check sidecars/node-validator/validate-reading-source.mjs` | pass |
| `python3 -m py_compile sidecars/python-parser/parser.py` | pass |
| `git diff --check` | pass |
| `npm run tauri build` | pass, produced `.app` and `.dmg` |

### Remaining
- Rust backend module split and typed models (`AUD-16`).
- Actual visual unified-runtime preview or explicit UI limitation label (`AUD-29`).
- Broader rendered-page scan coverage where `pypdf` cannot expose embedded images.
- Full dependency self-containment or signed setup flow; preflight only diagnoses host dependencies.

## Session Update: 2026-05-31 18:31 CST

### Phase 5 / AUD-29 Preview Semantics
- **Status:** clarified, not fully replaced with real visual runtime.
- Actions taken:
  - Added a warning/status banner to UnifiedPreview explaining that the visible iframe is a sandboxed local template preview.
  - Displayed current `runtime.mode` and clarified that export/Pack uses Rust static contract gate; real unified runtime E2E is diagnostic.

### Verification
| Test | Status |
|------|--------|
| `npm run check` | pass |
| `npm run build` | pass |
| `(cd src-tauri && cargo test)` | pass, 32 tests |
| `cargo fmt --check && cargo clippy --all-targets -- -D warnings` | pass |
| Sidecar syntax checks | pass |
| `git diff --check` | pass |

## Session Update: 2026-05-31 18:34 CST

### Documentation Sync: Lifecycle, Cleanup, SQL, PDF Dependencies
- **Status:** documentation updated for latest product decision.
- Actions taken:
  - Appended latest requirements to `Files/Epic8-Tauri作者端应用详细设计.md`.
  - Appended latest product-shape correction to `Files/Epic8-作者端Web导题与组卷器工程设计.md`.
  - Appended execution addendum to `Plan With Files/task_plan.md`.
  - Appended findings covering lifecycle/storage and PDF dependency research to `Plan With Files/findings.md`.
  - Logged this progress entry.

### Latest Product Decisions Captured
- MVP does not introduce SQL.
- SQL is reserved for future question-bank indexing/search, not raw process artifact storage.
- The retained long-term artifact is an editable structured draft plus minimal metadata/review/export summaries.
- Original PDF/DOCX copies and large process files should be deleted automatically after successful export/Pack.
- Developer/debug artifact retention remains available behind a default-off diagnostics option.
- Heavyweight local OCR is not bundled by default.
- Image/no-text PDFs use vision LLM transcription plus mandatory human source review; manual transcription remains fallback.
- Rust backend module splitting is deferred until production flow stabilizes.

### PDF Dependency Research Summary
| Option | Use | Product Fit |
|--------|-----|-------------|
| Current Python `pypdf` | Text-layer PDF extraction | Acceptable transition/dev path, but depends on host Python/pypdf. |
| Rust text extraction crate (`pdf-extract`, `pdf_text_extract`, similar) | Clear text PDF extraction | Best MVP production direction if fixture quality is acceptable. |
| `pdfium-render` | Render pages/extract images/text | Good future adapter for page images -> vision LLM; heavier than pure text extraction. |
| MuPDF/PyMuPDF | Powerful rendering/extraction | Not default due AGPL/commercial licensing implications. |
| Local OCR stack | OCR on device | Not MVP default because package size and maintenance cost are too high. |
| Vision LLM | Image PDF transcription | Preferred current strategy with human review gate. |

### Remaining Implementation Work
- Implement lifecycle semantics in code/UI: `Working`, `NeedsReview`, `DraftSaved`, `ExportReady`, `Exported`, `Cleaned`.
- Add automatic cleanup after successful JS/Pack export.
- Add default-off developer diagnostics option to retain full process artifacts.
- Evaluate Rust PDF text extraction adapter against current parser fixtures.
- Optionally add PDFium page-render adapter for image PDFs whose embedded images cannot be extracted.


## Session: 2026-05-31 19:06 CST

### Phase 5 / Latest Requirements E8-14-E8-16
- **Status:** complete for lifecycle, cleanup, and diagnostics-retention sub-pass.
- Actions taken:
  - Updated Rust `JobStatus` to latest product lifecycle states with serde aliases for old state values.
  - Updated TypeScript `JobStatus`, status labels, dashboard metrics, Pack publishable filters, and dev fallback state semantics.
  - Added `DiagnosticsSettings` backend persistence and Tauri commands `get_diagnostics_settings` / `save_diagnostics_settings`.
  - Added Settings Developer/Diagnostics UI for default-off `keepFullProcessArtifacts`.
  - Added `authoring-project.json` long-term project writer with source/review/validation/export summaries.
  - Added automatic post-export cleanup for single JS export and Pack export.
  - Added cleanup summaries and UI export cleanup notice.
  - Added regression test for diagnostics retention.

### Verification
| Test | Status |
|------|--------|
| `npm run check` | pass |
| `npm run build` | pass |
| `(cd src-tauri && cargo test)` | pass, 33 tests |
| `cargo fmt --check && cargo clippy --all-targets -- -D warnings` | pass |
| `node --check sidecars/llm-gateway/gateway.mjs` | pass |
| `node --check sidecars/preview-e2e/preview-e2e.mjs` | pass |
| `node --check sidecars/node-validator/validate-reading-source.mjs` | pass |
| `python3 -m py_compile sidecars/python-parser/parser.py` | pass |
| `git diff --check` | pass |
| `npm run tauri build` | pass, produced `.app` and `.dmg` |

### Remaining
- Evaluate lightweight Rust PDF text extraction (`E8-17`).
- Optional rendered-page adapter for PDFs whose embedded images cannot be extracted (`E8-18`).
- Dependency self-containment/setup strategy after parser decision.

### Error Log Addition
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-05-31 19:07 CST | Ran `cargo fmt --check` from repo root after lifecycle edits; Cargo.toml is under `src-tauri`. | 1 | Re-ran Rust formatting/test/clippy with working directory `src-tauri`; pass. |

## Session: 2026-05-31 19:52 CST

### Phase 5 / Latest Requirement E8-17
- **Status:** complete for clear-text PDF parser dependency reduction.
- Actions taken:
  - Added `pdf-extract = "0.10"` to the Rust backend.
  - Added a Rust PDF text-layer parser that emits `DocumentIRV1` with provider `rust-parser:pdf:pdf-extract`.
  - Updated `parse_source_document` so PDFs use Rust extraction first and Python parser fallback only on Rust extractor errors.
  - Preserved no-text PDF semantics: parser warning, low-confidence placeholder block, source-review gate, and follow-on vision LLM/manual transcription path.
  - Updated environment preflight to show built-in Rust PDF extraction and downgrade `python:pypdf` from clear-text PDF hard blocker to warning for image extraction/legacy fallback.
  - Updated sidecar README and Plan With Files records.

### Verification
| Test | Status |
|------|--------|
| `npm run check` | pass |
| `npm run build` | pass |
| `(cd src-tauri && cargo test)` | pass, 33 tests |
| `(cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings)` | pass |
| `node --check sidecars/llm-gateway/gateway.mjs` | pass |
| `node --check sidecars/preview-e2e/preview-e2e.mjs` | pass |
| `node --check sidecars/node-validator/validate-reading-source.mjs` | pass |
| `python3 -m py_compile sidecars/python-parser/parser.py` | pass |
| `git diff --check` | pass |
| `npm run tauri build` | pass, produced `.app` and `.dmg` |

### Remaining
- `E8-18`: optional rendered-page adapter if embedded-image extraction is insufficient for scanned PDFs.
- Remaining host dependency strategy: Node sidecars, Python DOCX/image extraction, pypdf image extraction, and external unified runtime.

### Error Log Addition
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-05-31 19:41 CST | `cargo clippy --all-targets -- -D warnings` failed on `unnecessary_lazy_evaluations` for `unwrap_or_else(|| text.len())`. | 1 | Replaced with `unwrap_or(text.len())` and reran Rust fmt/clippy successfully. |
| 2026-05-31 19:44 CST | `cargo fmt --check` failed after the clippy fix because rustfmt wanted the expression on one line. | 1 | Ran `cargo fmt`; `cargo fmt --check` then passed. |

## Session: 2026-05-31 20:11 CST

### Phase 5 / Latest Requirement E8-18
- **Status:** complete for macOS rendered-page fallback.
- Actions taken:
  - Added `sips` rendered PNG fallback inside `sidecars/python-parser/parser.py extract_pdf_images`.
  - Kept embedded-image extraction as the first path; rendered fallback is only used when no embedded images are exposed.
  - Added `renderedFallback` metadata and explicit warnings so the result cannot be mistaken for OCR or full human verification.
  - Added backend preflight check `renderer:macos-sips`.
  - Added Rust regression test `no_text_pdf_fixture_renders_page_fallback_for_vision`.
  - Updated sidecar README and Plan With Files records.

### Verification
| Test | Status |
|------|--------|
| `python3 -m py_compile sidecars/python-parser/parser.py` | pass |
| `python3 sidecars/python-parser/parser.py extract_pdf_images` on `no-text.pdf` | pass, `renderedFallback=true`, one PNG image |
| `python3 sidecars/python-parser/parser.py extract_pdf_images` on `image-only-reading.pdf` | pass, embedded image path still works |
| `(cd src-tauri && cargo test)` | pass, 34 tests |
| `(cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `npm run build` | pass |
| Sidecar syntax checks | pass |
| `git diff --check` | pass |
| `npm run tauri build` | pass, produced `.app` and `.dmg` |

### Remaining
- Decide production dependency strategy for remaining host runtimes and sidecars.
- Full cross-platform PDFium adapter remains future work if macOS `sips` fallback is not enough.

### Error Log Addition
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-05-31 20:04 CST | Tried `cargo test` with multiple bare test names; Cargo accepts only one test filter before `--`. | 1 | Re-ran full `(cd src-tauri && cargo test)`; pass, 34 tests. |

## Session: 2026-05-31 20:46 CST

### Phase 5 / Latest Requirement E8-20
- **Status:** complete for Rust DOCX primary parsing.
- Actions taken:
  - Added Rust DOCX OOXML parsing through `zip` + `quick-xml` and provider `rust-parser:docx:ooxml`.
  - Updated `parse_source_document` so DOCX uses the Rust parser first and only falls back to `python3 sidecars/python-parser/parser.py parse ...` if Rust parsing fails.
  - Added `rust:docx-ooxml` to environment preflight and downgraded `python3` messaging from clear-text DOCX/PDF requirement to image-extraction/legacy-fallback requirement.
  - Updated browser dev fallback preflight and `sidecars/README.md` to match the latest dependency strategy.
  - Narrowed `zip` features to `flate2` + `deflate-flate2`; verified `zopfli` is not in the dependency tree.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test complex_docx_fixture_reaches_authoring_ir)` | pass |
| `(cd src-tauri && cargo test)` | pass, 34 tests |
| `(cd src-tauri && cargo fmt --check)` | pass after formatting |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `npm run build` | pass |
| Sidecar syntax checks | pass |
| `git diff --check` | pass |
| `cargo tree -i zopfli` | no matching package, so `zopfli` is not pulled in |
| `npm run tauri build` | pass, produced `.app` and `.dmg` |

### Remaining
- Python/pypdf remain host dependencies for PDF image extraction, rendered-page fallback orchestration, and legacy parser fallback.
- Node.js is no longer a production dependency for LLM, validation, export, or Pack; it remains optional for diagnostics.
- External unified runtime is no longer required for export/Pack; it remains an explicit diagnostic/CI path.

### Error Log Addition
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-05-31 20:24 CST | Rust compile failed because `is_word_tag` expected `&[u8]` and DOCX parser passed `Vec<u8>`. | 1 | Borrowed tag names with `&name`; DOCX fixture test passed. |
| 2026-05-31 20:28 CST | `zip` with only `deflate-flate2` failed because optional `flate2` dependency was not enabled. | 2 | Used `features = ["flate2", "deflate-flate2"]`; compile passed and `zopfli` stayed absent. |
| 2026-05-31 20:36 CST | `cargo fmt --check` failed on new DOCX parser formatting. | 1 | Ran `cargo fmt`; subsequent `cargo fmt --check` passed. |

## Session: 2026-05-31 21:02 CST

### Phase 5 / Latest Requirement E8-21
- **Status:** complete for Rust-orchestrated macOS rendered-page fallback.
- Actions taken:
  - Added Rust `render_pdf_page_with_sips_fallback` that emits `PdfImageExtractionV1` with `renderedFallback=true` and `renderSource=rust-macos-sips`.
  - Added `extract_pdf_images_for_vision`: Python/pypdf embedded-image extraction remains first choice, but Rust `sips` fallback is used if Python extraction fails or returns zero images.
  - Updated `vision_transcription_for_job` to use the unified Rust entrypoint and shared `image_count_from_extraction` helper.
  - Updated environment preflight copy and `sidecars/README.md` to make clear Python/pypdf is no longer required for rendered-page vision fallback on macOS.
  - Added `rust_sips_fallback_renders_pdf_without_python_extraction` regression test.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test no_text_pdf_fixture_renders_page_fallback_for_vision)` | pass |
| `(cd src-tauri && cargo test rust_sips_fallback_renders_pdf_without_python_extraction)` | pass |
| `(cd src-tauri && cargo test)` | pass, 35 tests |
| `(cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `npm run build` | pass |
| Sidecar syntax checks | pass |
| `git diff --check` | pass |
| `npm run tauri build` | pass, produced `.app` and `.dmg` |

### Remaining
- Python/pypdf remain useful for embedded PDF image extraction and legacy parser fallback.
- Node.js is no longer a production dependency for LLM, validation, export, or Pack; it remains optional for diagnostics.
- External unified runtime is no longer required for export/Pack; it remains an explicit diagnostic/CI path.
- The Rust `sips` fallback is macOS-specific and still not OCR; SourceReview remains mandatory.

### Error Log Addition
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-05-31 20:54 CST | `cargo test` was invoked with two test-name filters, which Cargo rejects. | 1 | Re-ran each target test with a single filter. |
| 2026-05-31 20:54 CST | `cargo fmt --check` failed on new Rust fallback formatting. | 1 | Ran `cargo fmt`; later `cargo fmt --check` passed. |

## Session: 2026-05-31 21:19 CST

### Phase 5 / Latest Requirement E8-22
- **Status:** complete for Rust-primary TXT/MD parsing.
- Actions taken:
  - Added `parse_text_with_rust_parser` and moved TXT/MD handling before Python sidecar dispatch.
  - Added parser providers `rust-parser:text:plain` and `rust-parser:text:markdown`.
  - Added Markdown answer-list role detection so answer-key blocks are not misclassified as TFNG question blocks.
  - Added `fixtures/parser/complex-reading.txt` and `fixtures/parser/complex-reading.md`.
  - Added Rust fixture tests for TXT and Markdown parser -> split -> AuthoringIR -> answerKey.
  - Added `rust:text-parser` to environment preflight and synced sidecar README/dev fallback messaging.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test complex_txt_fixture_reaches_authoring_ir)` | pass |
| `(cd src-tauri && cargo test complex_markdown_fixture_reaches_authoring_ir)` | pass |
| `(cd src-tauri && cargo test environment_preflight_reports_required_dependency_names)` | pass |
| `(cd src-tauri && cargo test)` | pass, 37 tests |
| `(cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `npm run build` | pass |
| Sidecar syntax checks | pass |
| `git diff --check` | pass |
| `npm run tauri build` | pass, produced `.app` and `.dmg` |

### Remaining
- Python/pypdf remain useful for embedded PDF image extraction and legacy parser fallback only.
- Node.js is no longer a production dependency for LLM, validation, export, or Pack; it remains optional for diagnostics.
- External unified runtime is no longer required for export/Pack; it remains an explicit diagnostic/CI path.
- Cross-platform rendered-page support remains future work if macOS-only `sips` is insufficient.

### Error Log Addition
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-05-31 21:10 CST | Markdown fixture failed because answer lines after `## Answers` were classified as question content due TFNG words. | 1 | Added `looks_like_answer_key_block` before question heuristics; Markdown fixture passed. |
| 2026-05-31 21:09 CST | `cargo fmt --check` failed on new preflight/parser formatting. | 1 | Ran `cargo fmt`; later `cargo fmt --check` passed. |

## Session: 2026-05-31 21:58 CST

### Phase 5 / Production Dependency Direction
- **Status:** in progress.
- User decision recorded: do not package Node/Python/OCR into production. Continue Rust-first implementation and use vision LLM as the OCR substitute for image/no-text PDFs.
- Immediate implementation target: migrate `sidecars/llm-gateway/gateway.mjs` production behavior into Rust HTTP calls so Node is no longer required for LLM group structuring or vision transcription.

### Error Log Addition
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-05-31 22:05 CST | `cargo test` was invoked with two test-name filters again; Cargo accepts only one filter before `--`. | 1 | Re-run target tests separately and keep this command pattern out of future verification. |
| 2026-05-31 22:12 CST | `cargo clippy --all-targets -- -D warnings` failed on `needless_lifetimes` in `llm_profile`. | 1 | Elide the explicit lifetime and rerun clippy. |


## Session: 2026-05-31 22:49 CST

### Phase 5 / Production Dependency Path Consolidation
- **Status:** complete for Node removal from the production path and static gate migration.
- Actions taken:
  - Renamed Rust runtime gate parameters from `require_real_runtime` to `require_static_runtime_gate` to match current behavior.
  - Changed export/Pack core tests to assert `runtime.mode=static-rust` and removed external unified runtime env skips.
  - Updated dev fallback so publish readiness no longer requires `runtime.mode=real`; browser fallback now mirrors the Rust static contract gate.
  - Added `staticRuntimePassed` to the auto-pipeline report type while preserving `realRuntimePassed` only as a diagnostic field.
  - Changed `run_preview_e2e` so diagnostic failure is saved/returned but does not demote a static-gate-ready job from `ExportReady`.
  - Added regression test `preview_e2e_diagnostic_failure_does_not_block_static_export_ready`.
  - Synced Plan With Files and design-doc notes with the latest no-Node/no-Python/no-local-OCR production direction.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 51 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `npm run build` | pass |
| `node --check sidecars/llm-gateway/gateway.mjs` | pass |
| `node --check sidecars/preview-e2e/preview-e2e.mjs` | pass |
| `node --check sidecars/node-validator/validate-reading-source.mjs` | pass |
| `python3 -m py_compile sidecars/python-parser/parser.py` | pass |
| `git diff --check` | pass |
| `npm run tauri build` | pass, produced `.app` and `.dmg` |

### Remaining
- Continue code-quality audit and module split planning after the Rust-first production path stabilizes.
- Add broader UI E2E and live OpenAI-compatible provider coverage.

### Error Log Addition
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-05-31 22:34 CST | `cargo fmt --check` failed on formatting after static gate edits. | 1 | Ran `cargo fmt`; target regression test passed. |


## Session: 2026-05-31 23:16 CST

### Phase 5 / Backend Architecture Split First Cut
- **Status:** complete for first low-risk module extraction.
- Actions taken:
  - Added `src-tauri/src/util.rs`.
  - Moved pure utility helpers out of `src-tauri/src/lib.rs`: path segment validation, job directory helpers, JSON/text/binary IO, delete-if-exists, append text, and stored ZIP writer.
  - Kept public behavior stable by importing the same helper names back into `lib.rs`.
  - Fixed missed `job_dir` / `ensure_job_dirs` / `safe_job_dir` migration after the first compile attempt.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 51 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Error Log Addition
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-05-31 23:07 CST | `cargo test` first failed with missing `libzip...rlib` under `target/debug/deps`. | 1 | Ran `cargo clean -p zip` and reran tests; Cargo rebuilt dependencies and moved on to real compile errors. |
| 2026-05-31 23:10 CST | First util split accidentally removed `job_dir`, `safe_job_dir`, and `ensure_job_dirs` from scope. | 1 | Added those helpers to `util.rs` and imported them in `lib.rs`; tests passed. |


## Session: 2026-05-31 23:34 CST

### Phase 5 / Validator Module Split
- **Status:** complete for second backend decomposition cut.
- Actions taken:
  - Added `src-tauri/src/validator.rs`.
  - Moved pure ReadingExamSourceV1/DOM contract validation helpers out of `src-tauri/src/lib.rs`.
  - Kept `validate_authoring` in `lib.rs` to avoid crossing the AuthoringIR rendering/source-generation boundary in this step.
  - Reduced `lib.rs` to about 8258 lines while preserving all existing validation tests.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 51 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Remaining
- Parser and LLM are the next meaningful backend module seams.
- A full Tauri release build was not rerun in this continuation; the prior turn's release build passed after the Rust-first dependency migration.

## Session: 2026-05-31 23:58 CST

### Phase 5 / LLM Gateway Module Split
- **Status:** complete for Rust LLM gateway extraction.
- Actions taken:
  - Added `src-tauri/src/llm_gateway.rs`.
  - Moved OpenAI-compatible HTTP calls, JSON content parsing, request-cache redaction, vision image data URL encoding, group suggestion output validation, and vision transcription output normalization out of `src-tauri/src/lib.rs`.
  - Kept LLM profile storage, secret loading, orchestration, deterministic fallback, suggestion persistence, SourceReview, and auto-apply policy in `lib.rs` to avoid mixing authoring business logic into the gateway module.
  - Reduced `src-tauri/src/lib.rs` from about 8258 lines to 7885 lines.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 51 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Error Log Addition
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-05-31 23:49 CST | `cargo fmt --check` failed after introducing `llm_gateway.rs` because rustfmt wanted a compact import list in `lib.rs`. | 1 | Ran `cargo fmt`, reran `cargo fmt --check`, and it passed. |

### Remaining
- Split parser code in smaller seams: text/DOCX/PDF text extraction first, then source-review and vision-image extraction.
- Keep Node/Python/OCR out of the production hard path; sidecars remain diagnostic/legacy fallback only.
- Add live-provider coverage for Rust LLM gateway when credentials/test endpoint are available.

## Session: 2026-06-01 00:20 CST

### Phase 5 / Parser Module Split First Cut
- **Status:** complete for first parser module extraction.
- Actions taken:
  - Added `src-tauri/src/parser.rs`.
  - Moved Rust TXT/MD parser, text-layer PDF parser, DOCX OOXML parser, Python legacy parser fallback, PDF embedded-image extraction, macOS `sips` rendered-page fallback, parser failure/missing-source DocumentIR generation, manual transcription DocumentIR, and vision transcription DocumentIR conversion out of `src-tauri/src/lib.rs`.
  - Kept source review, split candidate generation, AuthoringIR generation, LLM orchestration, export, and Pack state in `lib.rs` for now because those functions still coordinate multiple business domains.
  - Preserved the latest dependency strategy: clear-text import remains Rust-primary; Python is fallback/embedded-image extraction only; image/no-text PDF remains vision LLM candidate plus SourceReview gate; no local OCR engine was introduced.
  - Reduced `src-tauri/src/lib.rs` from about 7885 lines to about 6922 lines.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 51 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Error Log Addition
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-06-01 00:08 CST | First parser split compile failed because `mod parser`/imports and cross-module helper visibility were missing. | 1 | Added `mod parser`, explicit parser imports, and `pub(crate)` visibility for shared app helpers used by parser. |
| 2026-06-01 00:12 CST | Parser extractor and sips helper tests could not access moved private functions. | 1 | Exposed those helpers as `pub(crate)` and imported them only under `#[cfg(test)]` in `lib.rs`. |

### Remaining
- Consider a second parser split later: separate `document_ir`, `docx`, `pdf_text`, `pdf_images`, and `legacy_python` submodules if parser.rs keeps growing.
- Continue architecture audit with export/pack/storage seams next.
- Keep all future PDF changes aligned with the current rule: no bundled OCR engine; image PDF uses rendered/extracted images plus vision LLM plus SourceReview.

## Session: 2026-06-01 00:44 CST

### Phase 5 / LLM Profile And Secret Storage Module Split
- **Status:** complete for profile/secret storage seam extraction.
- Actions taken:
  - Added `src-tauri/src/llm_profiles.rs`.
  - Moved LLM profile file storage, profile redaction for UI, macOS Keychain access, plaintext secret fallback gate, file secret helpers, profile secret save/load, and profile lookup out of `src-tauri/src/lib.rs`.
  - Kept Tauri commands (`list_llm_profiles`, `save_llm_profile`, `test_llm_profile`) in `lib.rs` because they still orchestrate app root, command payloads, and gateway testing.
  - Preserved the security policy at that stage: plaintext file fallback remains disabled unless `EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK` is explicitly enabled. Superseded on 2026-06-01 by cross-platform OS secure storage via `keyring`.
  - Reduced `src-tauri/src/lib.rs` from about 6922 lines to about 6673 lines.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 51 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Error Log Addition
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-06-01 00:38 CST | First profile split compile failed because `write_text` was still used by preview/export/manual transcription but was removed from `lib.rs` imports. | 1 | Restored `write_text` import from `util`; moved `is_safe_path_segment` to test-only import. |

### Remaining
- Export/Pack remains the largest business-flow seam. It should be split only after isolating pure helpers from job-state updates and cleanup side effects.
- Continue keeping LLM profile commands and gateway calls production Rust-first; Node gateway remains non-production diagnostic/legacy resource only.

## Session: 2026-06-01 01:08 CST

### Phase 5 / Export Artifact Builder Module Split
- **Status:** complete for pure export/Pack artifact builder extraction.
- Actions taken:
  - Added `src-tauri/src/export_artifacts.rs`.
  - Moved pure `safe_exam_id`, ReadingExam wrapper JS generation, manifest JS generation, and ReadingExamPack manifest generation out of `src-tauri/src/lib.rs`.
  - Added `ReadingAssetBundle` and `build_reading_asset_bundle` so preview/export share the same deterministic source + wrapper + manifest construction.
  - Kept `export_reading_assets_core`, `build_pack_core`, job-state updates, filesystem writes, publish gates, and cleanup orchestration in `lib.rs` because those remain side-effecting workflow logic.
  - Reduced `src-tauri/src/lib.rs` from about 6673 lines to about 6620 lines.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 51 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Remaining
- The next export/Pack split should isolate side-effect-free pack entry assembly before moving filesystem/job-state orchestration.
- Do not make preview E2E or Node runtime a production gate while splitting export code.

## Session: 2026-06-01 / E8-33

### Phase 5 / Pack Entry Assembly Split
- **Status:** complete for pure Pack entry construction.
- Actions taken:
  - Added `PackSource` and `PackEntryBundle` in `src-tauri/src/export_artifacts.rs`.
  - Added `build_pack_entry_bundle` to build `pack.json`, `reading-exams/manifest.js`, and per-exam wrapper entries without filesystem/job-state side effects.
  - Simplified `build_pack_core` so it validates job IDs, loads AuthoringIR, runs runtime/publish readiness gates, calls the pure Pack builder, writes the ZIP/files, updates jobs, and performs cleanup.
  - Added regression coverage for missing `examId` fallback consistency across script file name, Pack manifest, and wrapper registration key.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 52 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Error Log Addition
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-06-01 | `cargo fmt --check` failed after the Pack split because rustfmt wanted import wrapping changes in `src-tauri/src/lib.rs`. | 1 | Ran `cargo fmt`, reran `cargo fmt --check`, and it passed. |
| 2026-06-01 | Initial `cargo test` passed but reported unused imports after Pack builder calls moved out of `lib.rs`. | 1 | Moved wrapper/manifest/safe exam imports to `#[cfg(test)]` and removed unused `safe_path_segment` import from `lib.rs`. |

### Remaining
- `lib.rs` is still large and owns command orchestration, source/authoring workflow, export state, cleanup, and settings commands.
- The next low-risk split should avoid PDF/LLM business semantics and target storage/settings or cleanup/export orchestration boundaries.

## Session: 2026-06-01 / E8-34

### Phase 5 / Diagnostics Settings Module Split
- **Status:** complete for diagnostics settings persistence.
- Actions taken:
  - Added `src-tauri/src/diagnostics.rs`.
  - Moved `DiagnosticsSettings`, default-loading behavior, and settings file write/read helpers out of `src-tauri/src/lib.rs`.
  - Kept cleanup behavior in `lib.rs`, preserving the current rule that diagnostics retention skips transient cleanup and keeps job status unchanged.
  - Reduced `src-tauri/src/lib.rs` to about 6579 lines.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 52 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Error Log Addition
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-06-01 | `cargo fmt --check` failed after adding `diagnostics.rs` because rustfmt reordered module declarations. | 1 | Ran `cargo fmt`, reran the full verification set, and it passed. |

### Remaining
- `lib.rs` still owns environment preflight, job repository helpers, source/authoring business flow, LLM orchestration, export workflow, and cleanup. These should be split incrementally with tests at each seam.

## Session: 2026-06-01 / E8-35

### Phase 5 / Environment And Preflight Module Split
- **Status:** complete for environment/preflight extraction.
- Actions taken:
  - Added `src-tauri/src/environment.rs`.
  - Moved sidecar candidate discovery, `find_sidecar`, `command_failure`, `command_probe`, `EnvironmentPreflightV1` construction, external unified runtime path resolution, runtime strict-mode parsing, and Node validator diagnostic flag parsing out of `src-tauri/src/lib.rs`.
  - Kept parser/profile/runtime callers using the same `pub(crate)` functions through the module boundary, avoiding duplicated sidecar or command error logic.
  - Preserved the latest production dependency decision: preflight reports Node/Python/pypdf/sidecars as optional diagnostic or legacy capabilities, not production hard gates.
  - Reduced `src-tauri/src/lib.rs` to about 6210 lines.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 52 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Error Log Addition
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-06-01 | First `cargo fmt --check` after environment split failed because rustfmt wanted import wrapping changes in `src-tauri/src/lib.rs`. | 1 | Ran `cargo fmt` and reran formatting checks. |
| 2026-06-01 | `cargo test`/`cargo clippy --all-targets` failed because test-only assertions still used `plaintext_secret_fallback_allowed` after removing the normal production import from `lib.rs`. | 1 | Restored `plaintext_secret_fallback_allowed` as a `#[cfg(test)]` import only, then reran Rust test/clippy successfully. |

### Remaining
- `lib.rs` still owns source review, dynamic split/AuthoringIR generation, validation orchestration, LLM suggestion orchestration, export/Pack workflow, cleanup, and Tauri command handlers.
- Next useful seams are job repository helpers or source-review workflow, but those touch state transitions and should be covered by targeted tests before extraction.

## Session: 2026-06-01 / E8-36

### Real PDF Sample Regression And `Questions 14-26` Semantics
- **Status:** complete for the four current `Files/*.pdf` samples.
- Actions taken:
  - Added split logic that preserves opening P2 umbrella ranges in `umbrellaQuestionRanges`.
  - Prevented umbrella `Questions 14-26` from becoming a duplicate concrete group when later concrete ranges exist.
  - Added manual-question-import scaffold behavior for article-only/umbrella-only PDFs, with low confidence and AuthoringReview blocking.
  - Adjusted heading detection so Markdown headings remain valid while inline parenthetical references do not start new groups.
  - Updated frontend types and UI to show umbrella ranges and manual-import warnings in Split and Authoring editor screens.
  - Added real PDF sample regression over the four user-provided PDFs in `Files/`.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 53 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Notes
- Mixed text/image PDFs are intentionally not treated as fully clean text-layer PDFs. They stay on the vision-transcription/SourceReview route because image-only pages can contain missing questions or answer keys.
- The current regression does not prove live vision model transcription quality; it proves deterministic routing and review gates for the provided files.
- Full Tauri release build was not rerun in this session after E8-36.

## Session: 2026-06-01 / E8-37

### Phase 5 / SourceReview Module Split
- **Status:** complete for SourceReview extraction.
- Actions taken:
  - Added `src-tauri/src/source_review.rs`.
  - Moved parser warning extraction, low-confidence block id extraction, SourceReview fingerprinting, status persistence, and SourceReview issue generation out of `src-tauri/src/lib.rs`.
  - Kept workflow orchestration and job state transitions in `lib.rs` to avoid changing business behavior.
  - Preserved existing SourceReview semantics for text/image PDF routing, vision transcription review, stale fingerprints, and publish readiness gates.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test source_review -- --nocapture)` | pass, 5 targeted tests |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 53 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Notes
- This was an architecture-only extraction; no frontend contract or persisted JSON shape changed.
- Full Tauri release build was not rerun in this session after E8-37.

## Session: 2026-06-01 / E8-38

### Phase 5 / Job Store Module Split
- **Status:** complete for job persistence extraction.
- Actions taken:
  - Added `src-tauri/src/job_store.rs`.
  - Moved job factory, load, save, update, and list/filter/sort helpers out of `src-tauri/src/lib.rs`.
  - Kept Tauri command handlers and workflow state transitions in `lib.rs`.
  - Preserved existing status aliases, path traversal defenses, list ordering/filtering, and all review/export behavior.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 53 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Notes
- This was an architecture-only extraction; no frontend contract or persisted JSON shape changed.
- Full Tauri release build was not rerun in this session after E8-38.

## Session: 2026-06-01 / E8-39

### Dev Fallback Umbrella Range Parity
- **Status:** complete for browser dev fallback parity.
- Actions taken:
  - Confirmed the production Rust path already treats opening `Questions 14-26` as a valid Passage 2 umbrella range, not a false positive.
  - Updated `src/services/devFallbackBackend.ts` so browser dev fallback preserves opening umbrella ranges in `umbrellaQuestionRanges`.
  - Updated fallback split logic so umbrella ranges do not become duplicate concrete question groups when later headings exist.
  - Added fallback low-confidence `requiresManualQuestionImport` scaffolding when only the umbrella range is present.
  - Propagated `isUmbrellaRange` and `requiresManualQuestionImport` into fallback AuthoringIR groups/questions and validation/readiness checks.

### Verification
| Test | Status |
|------|--------|
| `npm run check` | pass |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `git diff --check` | pass |

### Notes
- Product semantics: the opening `Questions 14-26` instruction is a valid umbrella range for Passage 2. It should be included as source metadata and review context, but it is not sufficient by itself to publish concrete interactive questions.
- If later concrete groups exist, those groups remain the publishable interaction groups and the umbrella range is retained separately.
- If only the umbrella range exists, the app must require manual import/editing of concrete question prompts and answers before publish.

## Session: 2026-06-01 / E8-40

### Phase 5 / Cleanup Lifecycle Module Split
- **Status:** complete for cleanup mechanism extraction.
- Actions taken:
  - Added `src-tauri/src/cleanup.rs`.
  - Moved transient artifact cleanup mechanics out of `src-tauri/src/lib.rs`, including diagnostics retention handling, transient directory/file removal, and `cleanup-summary.json` writing.
  - Kept `write_authoring_project` and job status transitions in `lib.rs` via closure injection, so export/Pack lifecycle semantics remain owned by the workflow layer.
  - Preserved the cleanup behavior for normal export, Pack build, and diagnostics retention.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test cleanup -- --nocapture)` | pass |
| `(cd src-tauri && cargo test export_core_writes_assets_after_static_runtime_gate -- --nocapture)` | pass |
| `(cd src-tauri && cargo test build_pack_core_writes_zip_after_static_runtime_gate -- --nocapture)` | pass |
| `(cd src-tauri && cargo test)` | pass, 53 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Notes
- `src-tauri/src/lib.rs` is now about 6381 lines; `src-tauri/src/cleanup.rs` is 65 lines.
- This was an architecture-only extraction. It does not change PDF parsing, SourceReview, AuthoringReview, export gate, Pack gate, or no-Node/no-Python/no-OCR production dependency boundaries.

### Error Log Addition
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-06-01 | A cleanup test micro-patch attempted to remove a duplicate `temp_test_root()` declaration from stale context, but the current file no longer had the duplicate. | 1 | Re-read the actual cleanup test and skipped the unrelated edit. |

## Session: 2026-06-01 / E8-41

### Phase 5 / LLM Suggestion Helper Module Split
- **Status:** complete for LLM suggestion helper extraction.
- Actions taken:
  - Added `src-tauri/src/llm_suggestions.rs`.
  - Moved pure LLM suggestion helpers out of `src-tauri/src/lib.rs`: group context lookup, deterministic low-confidence fallback output, LLM request input builders, vision transcription input builder, suggestion persistence/loading, auto-apply safety checks, and selected patch application.
  - Kept provider profile selection, API-key loading, `run_llm_gateway` invocation, job state transitions, and auto-pipeline orchestration in `lib.rs` because those still own app root, storage, and workflow state.
  - Removed now-unused `safe_json_filename` helper from `lib.rs`; LLM suggestion filenames now use module-local sanitization.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test llm -- --nocapture)` | pass, 6 targeted tests |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 53 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Notes
- `src-tauri/src/lib.rs` is now about 5860 lines; `src-tauri/src/llm_suggestions.rs` is 546 lines.
- High-confidence auto-apply semantics are unchanged: provider-backed evidence, source block ids, quotes, allowed patch paths, and supported interaction types are still required before automatic application.
- LLM output still never creates human verification. Human verification remains separate in AuthoringReview.

## Session: 2026-06-01 / E8-42

### Phase 5 / AuthoringReview Rules Module Split
- **Status:** complete for AuthoringReview rule extraction.
- Actions taken:
  - Added `src-tauri/src/authoring_review.rs`.
  - Moved pure AuthoringIR review rules out of `src-tauri/src/lib.rs`: empty-answer detection, confidence/verified helpers, `refresh_authoring_review_state`, and `authoring_review_issues`.
  - Kept publish readiness orchestration in `lib.rs`, where job status, SourceReview issues, runtime reports, and report persistence are combined.
  - Preserved the core safety boundary: low-confidence groups/questions, empty answers, manual-question-import scaffolds, and missing human verification still block publish.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test refresh_authoring_review_state_requires_low_confidence_verification -- --nocapture)` | pass |
| `(cd src-tauri && cargo test publish_review_issues_block_empty_answers -- --nocapture)` | pass |
| `(cd src-tauri && cargo test validate_authoring_blocks_duplicate_display_numbers_and_gaps -- --nocapture)` | pass |
| `(cd src-tauri && cargo test auto_applied_llm_patch_does_not_create_human_verification -- --nocapture)` | pass |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 53 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Notes
- `src-tauri/src/lib.rs` is now about 5713 lines; `src-tauri/src/authoring_review.rs` is 152 lines.
- This was a pure rule extraction. It does not change SourceReview, runtime validation, export/Pack gates, or LLM auto-apply behavior.

## Session: 2026-06-01 / E8-43

### Phase 5 / Reading Source Contract Builder Split
- **Status:** complete for reading-source contract construction.
- Actions taken:
  - Added `src-tauri/src/reading_source.rs`.
  - Moved pure contract-building helpers out of `src-tauri/src/lib.rs`: group HTML rendering, answer key projection, question order/display map projection, and `ReadingExamSourceV1` assembly.
  - Kept publish/validate orchestration in `lib.rs`, because those functions still combine SourceReview, AuthoringReview, runtime validation, and report persistence.
  - Preserved the exact `ReadingExamSourceV1` shape, including `sourceRefs`, `audit`, `questionOrder`, and `questionDisplayMap` semantics.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test reading_source -- --nocapture)` | pass |
| `(cd src-tauri && cargo test rust_contract_validator -- --nocapture)` | pass, 10 tests |
| `(cd src-tauri && cargo test export_core_writes_assets_after_static_runtime_gate -- --nocapture)` | pass |
| `(cd src-tauri && cargo test build_pack_core_writes_zip_after_static_runtime_gate -- --nocapture)` | pass |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 53 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Notes
- `src-tauri/src/lib.rs` is now about 5514 lines; `src-tauri/src/reading_source.rs` is 270 lines.
- This is a pure contract extraction. It does not change publish gate semantics, SourceReview gating, or the actual runtime/export/Pack output shape.

## Session: 2026-06-01 / E8-44

### Product Semantics / Opening Umbrella Range
- **Status:** complete for the `Questions 14-26` clarification.
- Actions taken:
  - Removed duplicate stale `E44` Plan With Files entries and kept the canonical `E8-43` record.
  - Confirmed the current Rust PDF sample regression already preserves the opening P2 `Questions 14-26` instruction in `umbrellaQuestionRanges`.
  - Broadened Rust production umbrella detection beyond the exact `which are based on reading passage` wording to include conservative Passage-level opening instructions such as `Questions 14-26 are based on Reading Passage 2 below` and `You should spend about 20 minutes on Questions 14-26...`.
  - Mirrored the same detection semantics in the browser dev fallback.
  - Added a Rust regression test proving concrete headings like `Questions 14-19 Do the following statements agree with the information given in Reading Passage 2?` are not treated as umbrella ranges.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test umbrella_question_range_detection_keeps_opening_instructions_distinct -- --nocapture)` | pass |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `npm run check` | pass |
| `(cd src-tauri && cargo test)` | pass, 54 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `git diff --check` | pass |

### Notes
- Product invariant: `Questions 14-26` at the opening of P2 is valid range metadata and should appear in review context.
- Safety invariant: an umbrella range alone is not enough to publish concrete interactive questions; if no concrete groups are recognized, the app must keep the low-confidence manual-question-import path.

## Session: 2026-06-01 / E8-45

### Phase 5 / Authoring Validation Module Split
- **Status:** complete for pure validation/report merge extraction.
- Actions taken:
  - Added `src-tauri/src/authoring_validation.rs`.
  - Moved `validate_authoring`, `merge_sidecar_validation`, and `merge_validation_issues` out of `src-tauri/src/lib.rs`.
  - Kept `validate_for_runtime_gate`, `publish_readiness_gate`, `run_node_validator_diagnostic`, export, and Pack orchestration in `lib.rs` because they still coordinate filesystem artifacts, SourceReview, AuthoringReview, runtime reports, and job status transitions.
  - Preserved the existing static Rust gate behavior and optional Node validator diagnostic semantics.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test validate_authoring_blocks_duplicate_display_numbers_and_gaps -- --nocapture)` | pass |
| `(cd src-tauri && cargo test validation_warning_does_not_block_runtime_gate_progress -- --nocapture)` | pass |
| `(cd src-tauri && cargo test rust_contract_validator -- --nocapture)` | pass, 10 tests |
| `(cd src-tauri && cargo test)` | pass, 54 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Notes
- `src-tauri/src/lib.rs` is now about 5355 lines; `src-tauri/src/authoring_validation.rs` is 206 lines.
- This was a pure validation/reporting boundary extraction. It does not change PDF parsing, LLM suggestion safety, SourceReview gating, AuthoringReview gating, export, or Pack semantics.

## Session: 2026-06-01 / E8-46

### Publish Gate Negative Lifecycle Coverage
- **Status:** complete for export/Pack negative side-effect coverage.
- Actions taken:
  - Added `export_core_publish_gate_failure_writes_no_export_or_cleanup`.
  - Added `build_pack_publish_gate_failure_writes_no_pack_or_cleanup`.
  - Both tests create otherwise publishable jobs, then revoke `audit.humanVerified` so static contract validation can pass while publish readiness fails.
  - The tests assert failed export/Pack calls do not write final artifacts, do not create `cleanup-summary.json` or `authoring-project.json`, keep transient review artifacts available, and leave job status before `Exported`/`Cleaned`.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test export_core_publish_gate_failure_writes_no_export_or_cleanup -- --nocapture)` | pass |
| `(cd src-tauri && cargo test build_pack_publish_gate_failure_writes_no_pack_or_cleanup -- --nocapture)` | pass |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 56 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Notes
- This pass intentionally did not move workflow code. It strengthens the protection needed before extracting publish/export orchestration.
- `src-tauri/src/lib.rs` is about 5430 lines after adding the two tests; `src-tauri/src/authoring_validation.rs` remains 206 lines.

## Session: 2026-06-01 / Umbrella Range Revalidation

### Product Semantics
- **Status:** confirmed.
- User reconfirmed that opening instructions such as `Questions 14-26` are valid question-group information and must be included, even though they are presented as the Passage-level instruction rather than a concrete interactive subgroup.
- Current implementation already matches this semantics:
  - `umbrellaQuestionRanges` preserves the opening total range.
  - `questionGroupCandidates` still represents concrete publishable subgroups when later headings exist.
  - If only the umbrella range exists, the app creates a low-confidence `requiresManualQuestionImport` scaffold and blocks publish until concrete prompts are manually imported/verified.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test umbrella_question_range_detection_keeps_opening_instructions_distinct -- --nocapture)` | pass |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |

### Notes
- No code change was needed in this pass. Rust production parsing and the browser dev fallback already share the same umbrella-range distinction.
- `SplitAndAnswers` exposes the preserved total range under "总题组范围", so the user can see that the opening `Questions 14-26` instruction was retained.

## Session: 2026-06-01 / E8-47

### Command Lifecycle And Export Directory Picker
- **Status:** complete for CQ-04/CQ-05.
- Actions taken:
  - Added `validation_job_state` and `update_validation_job_state` in `src-tauri/src/lib.rs`.
  - Updated `validate_authoring_ir` so validation re-runs overwrite stale `current_step` values instead of only changing `status`.
  - Added tests for SourceReview routing, AuthoringIR validation failure routing, passing validation routing, and stale `Export` step overwrite.
  - Replaced the `choose_export_dir` backend stub with a native Tauri folder picker using `tauri_plugin_dialog::DialogExt`.
  - Synced browser dev fallback validation state with the Rust command behavior.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test validation_job_state_routes_review_and_authoring_steps -- --nocapture)` | pass |
| `(cd src-tauri && cargo test validate_authoring_state_update_overwrites_stale_current_step -- --nocapture)` | pass |
| `npm run check` | pass |
| `(cd src-tauri && cargo test)` | pass, 58 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `git diff --check` | pass |

### Notes
- `validate_authoring_ir` does not advance to Preview; preview asset generation and runtime/static validation remain separate commands.
- The native OS folder picker is not automated because it requires desktop dialog interaction, but compile/clippy coverage proves the backend command is no longer a stub.

## Session: 2026-06-01 / E8-48

### Workflow State Module Split
- **Status:** complete for lifecycle-state extraction.
- Actions taken:
  - Added `src-tauri/src/workflow_state.rs`.
  - Moved preview-E2E job status projection and AuthoringIR validation job status projection out of `src-tauri/src/lib.rs`.
  - Moved the validation stale-step regression tests into the new module.
  - Added two preview lifecycle tests proving failed preview downgrades stale `ExportReady`, and successful preview only reaches `ExportReady` when publish readiness passes.
  - Kept command orchestration, validation report generation, publish readiness, export, Pack, and cleanup in `lib.rs`.

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

### Notes
- `src-tauri/src/lib.rs` is now 5375 lines; `src-tauri/src/workflow_state.rs` is 224 lines.
- This reduces the monolith without changing business behavior. The next extraction should still be gated by lifecycle tests because the remaining orchestration has filesystem side effects.

## Session: 2026-06-01 / E8-49

### Runtime Validation Module Split
- **Status:** complete for runtime/static validation helper extraction.
- Actions taken:
  - Added `src-tauri/src/runtime_validation.rs`.
  - Moved Rust static runtime gate and preview asset writing out of `src-tauri/src/lib.rs`.
  - Moved optional Node validator diagnostics and preview E2E sidecar helpers into the new module.
  - Moved publish readiness merging into the new module while keeping export/Pack side-effect orchestration in `lib.rs`.
  - Cleaned stale imports after extraction.

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

### Notes
- `src-tauri/src/lib.rs` is now 5185 lines; `src-tauri/src/runtime_validation.rs` is 223 lines.
- Production policy is unchanged: Rust static contract gate is authoritative, Node validator and real runtime E2E remain diagnostics, and unresolved SourceReview/AuthoringReview issues block export/Pack.

## Session: 2026-06-01 / E8-50

### Opening `Questions 14-26` Range Revalidation
- **Status:** complete for the latest clarification.
- Actions taken:
  - Rechecked the Rust production split logic and browser dev fallback behavior for opening Passage-level question ranges.
  - Confirmed the implementation already preserves opening `Questions 14-26` as `umbrellaQuestionRanges` and does not duplicate it as a concrete Q14-Q26 group when later concrete headings exist.
  - Confirmed umbrella-only detections still create low-confidence `requiresManualQuestionImport` scaffolds and are blocked by AuthoringReview until the user imports/verifies concrete prompts and answers.
  - Added explicit Rust regression coverage for the en-dash spelling `Questions 14\u{2013}26`, matching the user-described presentation variant.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test umbrella_question_range_detection_keeps_opening_instructions_distinct -- --nocapture)` | pass |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `npm run check` | pass |

### Notes
- Runtime logic did not need to change; the parser already supports `-`, en dash, and em dash between question numbers.
- The new assertion protects the exact presentation variant from future refactors.

## Session: 2026-06-01 / E8-51

### Authoring Pipeline Module Split
- **Status:** complete for this architecture pass.
- Actions taken:
  - Added `src-tauri/src/authoring_pipeline.rs`.
  - Moved pure dynamic split and initial AuthoringIR construction logic out of `src-tauri/src/lib.rs`.
  - Moved shared DocumentIR text helpers, umbrella/concrete range detection, answer parsing, split merge, prompt extraction, and initial group/question construction into the new module.
  - Updated parser/source review/LLM suggestion imports to use the new module boundary where they rely on shared text/range helpers.
  - Kept file IO, answer source parser execution, SourceReview, AuthoringReview, LLM provider orchestration, export/Pack side effects, cleanup, and job lifecycle transitions in `lib.rs`.

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

### Notes
- `src-tauri/src/lib.rs` is now 4250 lines.
- `src-tauri/src/authoring_pipeline.rs` is 956 lines.
- Production behavior is unchanged: clear text parsing stays Rust-primary, scanned/image PDFs still require vision LLM plus SourceReview, generated AuthoringIR is not human-verified by default, and export/Pack remain gated by static Rust validation plus SourceReview/AuthoringReview.

## Session: 2026-06-01 / E8-52

### Cleanup And Project Archive Module Split
- **Status:** complete for this architecture pass.
- Actions taken:
  - Expanded `src-tauri/src/cleanup.rs` from transient file removal into the full successful-export archive/cleanup boundary.
  - Moved `AuthoringProjectV1` writing, source summary, review summary, validation summary, export summary assembly, diagnostics-retention branch, transient file deletion, and final `Cleaned` state transition out of `src-tauri/src/lib.rs`.
  - Kept export/Pack artifact-writing orchestration and publish-gate checks in `lib.rs`.

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

### Notes
- `src-tauri/src/lib.rs` is now 4165 lines.
- `src-tauri/src/cleanup.rs` is now 135 lines.
- Production behavior is unchanged: successful export/Pack keeps editable project artifacts and summaries, cleanup removes transient process files by default, diagnostics retention keeps process files, and failed publish gates still write no final export/Pack artifacts or cleanup summaries.

## Session: 2026-06-01 / E8-53

### Auto Pipeline Command-Core Coverage
- **Status:** complete for this safety-coverage pass.
- Actions taken:
  - Added `run_auto_pipeline_core(root, job_id, input)` and kept the Tauri command as a thin `app_root` wrapper.
  - Added a fixture-upload helper for tests so pipeline tests use the same `uploads/` path and metadata style as real user imports.
  - Added `auto_pipeline_llm_failure_keeps_text_import_in_llm_review`.
  - Added `auto_pipeline_keeps_no_text_pdf_blocked_for_source_review`.
- Behavior now directly covered:
  - Clear text upload can parse, split, build AuthoringIR, and pass static runtime validation, but LLM gateway failure keeps the job in `NeedsReview`/`LlmReview` and writes no cleanup/export artifacts.
  - No-text PDF upload attempts the vision path and remains in `NeedsReview`/`DocumentReview` with unresolved SourceReview and no cleanup/export artifacts.

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

### Notes
- `src-tauri/src/lib.rs` is now 4322 lines because this pass intentionally added command-core tests rather than only extracting code.
- This improves confidence in the user-facing automatic upload pipeline before any future auto-pipeline module extraction.

## Session: 2026-06-01 / E8-54

### Opening Umbrella Range Regression Hardening
- **Status:** complete for this clarification pass.
- User clarified that `Questions 14-26` / `Questions 14–26` appearing in the opening instructions is valid question-group information and must be included.
- Actions taken:
  - Added a minimal Rust regression fixture for the exact two-level behavior.
  - The fixture preserves opening `Questions 14–26` in `umbrellaQuestionRanges`.
  - The same fixture keeps later concrete groups as `Questions 14-19`, `Questions 20-23`, and `Questions 24-26`.
  - The fixture proves the opening umbrella range is not converted into a duplicate concrete Q14-Q26 group and does not trigger `requiresManualQuestionImport` when concrete groups are present.

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

### Notes
- No runtime logic change was required; the existing parser behavior was correct.
- The new regression is intentionally smaller than the real PDF sample test so future parser/module refactors cannot accidentally drop this product semantic while still passing broad fixture tests.

### Tooling Note
- A final `rg` verification command accidentally used shell backticks inside the query string, so zsh attempted to execute `Questions`. This did not affect code or validation results; subsequent focused diff/status checks confirmed the intended files and test addition.

## Session: 2026-06-01 / E8-55

### Auto Pipeline High-Confidence LLM Command-Core Coverage
- **Status:** complete for this safety-coverage pass.
- Actions taken:
  - Added `run_auto_pipeline_core_with_gateway(...)` as an internal test seam. The production `run_auto_pipeline_core(...)` still delegates to the real Rust `run_llm_gateway`.
  - Added `auto_pipeline_high_confidence_llm_auto_applies_without_human_verification` with a controlled mock gateway, avoiding real HTTP while exercising the full filesystem-backed upload -> parse -> split -> AuthoringIR -> LLM suggestion -> auto-apply -> validation -> job-state report path.
- Behavior now directly covered:
  - High-confidence LLM structure suggestions with valid source evidence can be auto-applied inside the full automatic pipeline.
  - Auto-apply records `autoApplied` and `lastAutoAppliedSuggestionId` on the group.
  - LLM-suggested prompts/interactions are applied, but parsed answers are not erased or rewritten by the mock suggestion.
  - Auto-apply does not mark questions or `audit.humanVerified` as human verified.
  - The job remains `NeedsReview`/`Authoring` until human verification completes, rather than being promoted to publish readiness solely by LLM confidence.

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

### Errors / Fixes
| Error | Resolution |
|-------|------------|
| `cargo fmt --check` reported formatting differences after adding the test. | Ran `cargo fmt`, then `cargo fmt --check` passed. |
| Rust borrow checker rejected mutating a JSON question while holding an immutable borrow of `displayNumber`. | Copied `displayNumber` into an owned `String` before mutating the JSON object. |

### Notes
- This fills the main E8-53 follow-up gap: positive high-confidence auto-apply is now covered through the full command-core pipeline, not only helper-level functions.
- The next architecture-safe step is extracting auto-pipeline orchestration from `lib.rs`, now that parse failure, no-text PDF, LLM failure, and high-confidence auto-apply paths are covered at command-core level.

## Session: 2026-06-01 / E8-56

### Auto Pipeline Module Extraction
- **Status:** complete for this architecture pass.
- Actions taken:
  - Added `src-tauri/src/auto_pipeline.rs`.
  - Moved automatic upload pipeline orchestration out of `src-tauri/src/lib.rs` into the new module.
  - Moved related helpers with it: answer-source parsing, LLM profile selection, LLM API key loading, PDF vision-transcription eligibility, and vision transcription execution.
  - Kept Tauri command wrapper `run_auto_pipeline(...)` in `lib.rs`, delegating to `auto_pipeline::run_auto_pipeline_core(...)`.
  - Kept the controlled test seam `run_auto_pipeline_core_with_gateway(...)` in the new module for command-core tests.
- Behavior preserved:
  - Parser/vision uncertainty still routes through SourceReview.
  - LLM failure/low-confidence still routes to LlmReview.
  - High-confidence LLM auto-apply still records structure changes without creating human verification.
  - Automatic pipeline still writes no cleanup/export artifacts before review/publish gates pass.

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

### Notes
- `src-tauri/src/lib.rs` is now 3977 lines.
- `src-tauri/src/auto_pipeline.rs` is 632 lines.
- This is a meaningful reduction in the command monolith while preserving tested behavior at the command-core boundary.

## Session: 2026-06-01 / E8-57

### Export And Pack Module Extraction
- **Status:** complete for this architecture pass.
- Actions taken:
  - Added `src-tauri/src/export_pack.rs`.
  - Moved `export_reading_assets_core(...)` and `build_pack_core(...)` out of `src-tauri/src/lib.rs` into the new module.
  - Kept Tauri command wrappers `export_reading_assets(...)` and `build_pack(...)` in `lib.rs`, because Tauri `generate_handler!` expects command macros in scope.
  - Preserved export/Pack artifact-writing behavior, publish-readiness gates, static runtime gates, job status updates, zip writing, and cleanup calls.
- Behavior preserved:
  - Successful single export writes JSON/JS/manifest/validation report and then runs cleanup.
  - Failed publish gate writes no final export artifacts and no cleanup summary/project archive.
  - Successful Pack writes standard zip plus expanded pack directory and then runs cleanup.
  - Failed Pack publish gate writes no zip/pack directory artifacts and no cleanup summary/project archive.

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

### Errors / Fixes
| Error | Resolution |
|-------|------------|
| Initial extraction accidentally moved the Tauri `build_pack` command wrapper into the new module, causing `generate_handler!` to fail to find `build_pack` command macros. | Moved `build_pack(...)` wrapper back to `lib.rs`; kept only `build_pack_core(...)` in `export_pack.rs`. |
| Extraction left imports with wrong visibility/usage (`cleanup_transient_job_artifacts`, `validate_path_segment`, `write_bytes`). | Restored test-only imports where needed and removed runtime unused imports. |

### Notes
- `src-tauri/src/lib.rs` is now 3820 lines.
- `src-tauri/src/export_pack.rs` is 173 lines.
- Export/Pack artifact orchestration is now isolated behind existing positive/negative command-core tests.

## Session: 2026-06-01 / E8-58

### Standalone Opening Umbrella Range Handling
- **Status:** complete for this parser-safety pass.
- User clarified that `Questions 14-26` / `Questions 14–26` appearing at the start of the passage instructions is a correct question-group range and must be included, even if it is presented as a standalone heading block.
- Actions taken:
  - Updated `src-tauri/src/authoring_pipeline.rs` so umbrella detection is block-context aware, not only single-text-block aware.
  - Added conservative handling for bare full-passage opening ranges near `READING PASSAGE`, including cases where PDF extraction splits the opening sentence into separate blocks.
  - Kept concrete subgroup ranges such as `Questions 14-19`, `Questions 20-23`, and `Questions 24-26` as the publishable interaction groups when present.
  - Synced `src/services/devFallbackBackend.ts` with the Rust semantics so browser dev fallback does not drift from production behavior.
  - Added `split_opening_bare_umbrella_heading_is_included_without_duplication` as a targeted regression.
- Behavior preserved:
  - Opening full-passage `Questions 14-26` is stored in `umbrellaQuestionRanges`.
  - No duplicate concrete Q14-Q26 group is created when later concrete subgroups exist.
  - Umbrella-only detections remain low-confidence `requiresManualQuestionImport` scaffolds and are blocked by AuthoringReview until manually completed.
  - Real `Files/*.pdf` sample regression still passes.

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

### Errors / Fixes
| Error | Resolution |
|-------|------------|
| Initial umbrella-range JSON construction had a mismatched delimiter after refactor. | Fixed the `Some(json!(...))` closure syntax and reran formatting/tests. |
| Cargo was invoked with multiple test filters in one command. | Re-ran using the single `umbrella` filter, which covers the relevant regression tests. |
| Clippy flagged consecutive `str::replace` calls. | Replaced them with a single `replace(['\u{2013}', '\u{2014}'], "-")` call. |

### Notes
- This closes the immediate product clarification for standalone opening range headings.
- The remaining architectural risk is not this rule itself; it is the continued JSON-heavy split/AuthoringIR construction. A typed intermediate model would reduce future field drift.

## Session: 2026-06-01 / E8-59

### LLM Command-Core Module Extraction
- **Status:** complete for this architecture pass.
- Actions taken:
  - Added `src-tauri/src/llm_commands.rs`.
  - Moved LLM profile save/test command-core logic out of `src-tauri/src/lib.rs`.
  - Moved `llm_classify_group`, `llm_extract_group`, and `apply_llm_suggestion` core orchestration into the new module.
  - Kept Tauri command wrappers in `lib.rs`, delegating to `save_llm_profile_core`, `test_llm_profile_core`, `llm_run_group_core`, and `apply_llm_suggestion_core`.
  - Moved shared `load_llm_api_key` into `llm_profiles.rs` so both auto-pipeline and LLM command orchestration use the same secret-loading policy.
- Behavior preserved:
  - Low-confidence/fallback LLM suggestions remain non-auto-applicable.
  - High-confidence auto-apply still requires source evidence and selected path validation.
  - Applying an LLM suggestion regenerates answerKey/questionOrder/questionDisplayMap and refreshes AuthoringReview.
  - LLM output still never sets `audit.humanVerified`.
  - SourceReview issues still merge into job `NeedsReview` state after applying suggestions.

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

### Notes
- `src-tauri/src/lib.rs` is now 3637 lines.
- `src-tauri/src/llm_commands.rs` is 241 lines.
- This reduces command monolith size while preserving the current JSON contract and command names.

## Session: 2026-06-01 / E8-60

### Preview And Validation Command-Core Extraction
- **Status:** complete for this architecture pass.
- Actions taken:
  - Added `src-tauri/src/preview_commands.rs`.
  - Moved `validate_authoring_ir`, `generate_preview_assets`, and `run_preview_e2e` command-core orchestration out of `src-tauri/src/lib.rs`.
  - Kept Tauri command wrappers in `lib.rs`, delegating to `validate_authoring_ir_core`, `generate_preview_assets_core`, and `run_preview_e2e_core`.
  - Kept `runtime_validation.rs` responsible for low-level static runtime validation, publish readiness, Node validator diagnostics, and preview asset writing.
  - Kept `workflow_state.rs` responsible for validation/preview job-state transitions.
- Behavior preserved:
  - Static Rust runtime validation remains the production gate.
  - Real runtime E2E remains diagnostic and is merged into the validation report without becoming the ordinary export hard gate.
  - Preview generation still fails fast on static validation failure and moves the job back to Authoring/NeedsReview.
  - Preview readiness still reflects SourceReview, AuthoringReview, and human verification state.
  - Missing AuthoringIR still returns the authoring validation report path without inventing preview assets.

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

### Notes
- `src-tauri/src/lib.rs` is now 3523 lines.
- `src-tauri/src/preview_commands.rs` is 146 lines.
- This further separates command wiring from validation/preview business orchestration while keeping command names stable for the frontend.

## Session: 2026-06-01 / E8-61

### Authoring Command-Core Module Extraction And Umbrella Range Verification
- **Status:** complete for this architecture and parser-rule pass.
- Actions taken:
  - Added `src-tauri/src/authoring_commands.rs`.
  - Moved parse document, manual transcription, vision transcription, source-review resolution, rule split, split adjustment save, AuthoringIR build/update, and group HTML render command-core logic out of `src-tauri/src/lib.rs`.
  - Kept Tauri command wrappers in `lib.rs`, delegating to the new `authoring_commands::*_core` functions.
  - Cleaned production/test imports so normal `cargo check` is warning-free and test-only helpers stay inside `#[cfg(test)]` scope.
  - Re-verified the user clarification: opening `Questions 14-26` / `Questions 14–26` is valid umbrella range metadata and must be retained, not ignored.
- Behavior preserved:
  - Opening umbrella ranges are stored in `umbrellaQuestionRanges`.
  - Concrete later subgroups such as `Questions 14-19`, `Questions 20-23`, and `Questions 24-26` remain the actual question-group candidates when present.
  - The umbrella range is not duplicated as a concrete Q14-Q26 group.
  - Umbrella-only samples create a low-confidence `requiresManualQuestionImport` scaffold and remain blocked by AuthoringReview.
  - Text-layer PDF/DOCX/TXT/MD can reach AuthoringIR; manual and vision transcription branches can reach split; vision transcription remains SourceReview-gated.

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

### Errors / Fixes
| Error | Resolution |
|-------|------------|
| `cargo check` passed but warned about production unused imports after extraction. | Moved `refresh_authoring_review_state` and `serde_json::json` usage to test-only imports. |
| `cargo test umbrella` initially failed because tests relied on root-level test imports that had been removed during extraction. | Moved test-only dependencies into `#[cfg(test)] mod tests`, including parser, source-review, runtime, LLM, and utility helpers. |

### Notes
- `src-tauri/src/lib.rs` is now 3304 lines.
- `src-tauri/src/authoring_commands.rs` is 289 lines.
- E8-12 architecture work is materially underway, but typed domain models are still the next quality step.

## Session: 2026-06-01 / E8-62

### Job/Import/Settings Command-Core Module Extraction
- **Status:** complete for this architecture pass.
- Actions taken:
  - Added `src-tauri/src/job_commands.rs`.
  - Moved create/list/get/update/delete job, source file import, reveal job folder, export directory picker, LLM profile list, environment preflight, and diagnostics settings command-core logic out of `src-tauri/src/lib.rs`.
  - Kept Tauri command wrappers in `lib.rs`, delegating to `job_commands::*_core` functions so frontend command names and handler macro scope remain stable.
  - Moved app directory setup and file import helpers (`ensure_app_dirs`, `file_type_from_name`, `sanitize_filename`, `hash_file_or_path`) into `util.rs`.
  - Removed root-level `command_failure` / `find_sidecar` coupling by importing `environment::{command_failure, find_sidecar}` directly in parser/profile modules.
- Behavior preserved:
  - Imported source files are still hashed, sanitized, copied under app data job uploads, and move the job to `DocumentReview`.
  - Job metadata updates still cannot mutate workflow status/current step directly.
  - Environment preflight still reports the required dependency and policy checks.
  - PDF sample and complex parser regression paths still pass after the extraction.

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

### Errors / Fixes
| Error | Resolution |
|-------|------------|
| Initial `cargo check` failed because `DiagnosticsSettings`, `DialogExt`, `ensure_app_dirs`, and `find_sidecar` paths were still implicit/root-level. | Added explicit module imports and moved shared helpers into `util.rs`. |
| Initial test build failed because tests referenced helpers that were no longer imported from `super::*`. | Added explicit test-only imports for job store, util, diagnostics, environment, source review, `std::fs`, `Path`, and `Uuid`. |

### Notes
- `src-tauri/src/lib.rs` is now 3146 lines.
- `src-tauri/src/job_commands.rs` is 186 lines.
- Additional mechanical extraction from `lib.rs` now has lower marginal value; typed domain models are the next higher-quality architecture step.

## Session: 2026-06-01 / E8-63

### Typed Domain Seam: SourceReviewV1
- **Status:** complete for this typed-domain pass.
- Actions taken:
  - Added `SourceReviewV1` as a Rust struct in `src-tauri/src/source_review.rs`.
  - Updated `source_review_status` to construct the typed struct and serialize it back to the existing JSON contract.
  - Updated `write_source_review_status` to round-trip through the typed struct before persisting `source-review.json`.
  - Updated `source_review_issues` to accept typed or legacy JSON input, preserving backward compatibility with existing tests and saved files.
  - Added a regression test that locks the JSON contract fields and null behavior for `schemaVersion`, `jobId`, `required`, `resolved`, `stale`, `fingerprint`, `parserWarnings`, `lowConfidenceBlocks`, `resolvedAt`, and `note`.
- Behavior preserved:
  - Publish gating semantics are unchanged: parser warnings and low-confidence blocks remain blocking until source review is resolved.
  - The frontend / persisted JSON contract is unchanged.
  - Existing no-text PDF, vision transcription, and umbrella-range tests still pass.

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

### Notes
- `src-tauri/src/source_review.rs` is now 263 lines.
- This is the first narrow typed-domain seam in the backend and establishes the pattern for future `DocumentIR` / `SplitCandidates` / `ReadingAuthoringIR` refactors.

## Session: 2026-06-01 / E8-64

### Typed Domain Seam: SplitCandidatesV1 And Opening Umbrella Range Contract
- **Status:** complete for this typed-domain pass.
- Actions taken:
  - Added typed Rust DTOs in `src-tauri/src/authoring_pipeline.rs` for `SplitCandidatesV1`, `PassageCandidateV1`, `SplitGroupCandidateV1`, `UmbrellaQuestionRangeV1`, and `AnswerKeyCandidateV1`.
  - Kept the public `make_dynamic_split_candidates` return type as `serde_json::Value`, but now the dynamic production path builds typed structs before serializing to the existing frontend JSON contract.
  - Preserved the product rule that opening `Questions 14-26` / `Questions 14–26` is valid Passage-level umbrella question-range metadata under `umbrellaQuestionRanges`.
  - Preserved the safety rule that the umbrella range is not duplicated as a concrete Q14-Q26 group when later concrete subgroups exist.
  - Preserved the umbrella-only fallback: if no concrete question subgroup exists, the app creates a low-confidence group with `isUmbrellaRange: true` and `requiresManualQuestionImport: true`, keeping the flow blocked for human import/review.
  - Added `split_candidates_v1_preserves_umbrella_contract_and_manual_scaffold`, a field-level JSON contract regression for top-level keys, umbrella fields, manual scaffold flags, confidence, kind hint, and answer key parsing.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test umbrella -- --nocapture)` | pass, 4 tests |
| `(cd src-tauri && cargo test split_candidates_v1_preserves_umbrella_contract_and_manual_scaffold -- --nocapture)` | pass |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `(cd src-tauri && cargo test complex_ -- --nocapture)` | pass, 4 tests |
| `(cd src-tauri && cargo test source_review -- --nocapture)` | pass, 7 tests |
| `(cd src-tauri && cargo test)` | pass, 67 tests |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Notes
- The user clarification is now locked in both behavior and typed DTO serialization: opening total ranges are retained as source/review metadata, while publishable interactions still require concrete subgroups or manual question import.
- The dynamic split path is less prone to camelCase field drift because internal Rust fields now serialize through `serde(rename_all = "camelCase")` instead of hand-written `json!` maps.

## Session: 2026-06-01 / E8-65

### Typed Domain Seam: ReadingAuthoringIRV1 Generation
- **Status:** complete for this typed-domain pass.
- Actions taken:
  - Added typed Rust DTOs in `src-tauri/src/authoring_pipeline.rs` for `ReadingAuthoringIrV1`, exam metadata, passage draft, question group draft, question draft, authoring audit, and source-file metadata.
  - Kept the public `make_dynamic_authoring_ir` return type as `serde_json::Value`, but now the generation path builds typed structs before serializing to the existing frontend `ReadingAuthoringIRV1` JSON contract.
  - Preserved the dynamic interaction/layout extension points as `serde_json::Value`, because those are intentionally flexible template/control DSL objects.
  - Generated `answerKey`, `questionOrder`, and `questionDisplayMap` from typed groups/questions rather than re-walking ad hoc JSON.
  - Added `reading_authoring_ir_v1_preserves_manual_import_contract`, a field-level contract regression covering top-level keys, exam/source file fields, passage block fields, umbrella/manual-import group flags, question prompts, answer key values, display map, question order length, audit issues, AuthoringReview blocking, and structural validation behavior.
- Behavior clarified:
  - `validate_authoring` validates structural source/runtime contract shape. It can pass for an umbrella-only manual-import scaffold because the structure is coherent.
  - Publish safety for that scaffold remains blocked by `refresh_authoring_review_state`, `authoring_review_issues`, and the publish readiness gate until concrete prompts are manually imported and verified.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test reading_authoring_ir_v1_preserves_manual_import_contract -- --nocapture)` | pass |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `(cd src-tauri && cargo test authoring -- --nocapture)` | pass, 10 tests |
| `(cd src-tauri && cargo test)` | pass, 68 tests |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Notes
- This is the second major typed-domain seam after `SplitCandidatesV1`. It reduces field-name drift in the core split -> authoring -> review/export chain while keeping the frontend and persisted JSON contracts unchanged.
- Remaining high-value typed seams are validation/report DTOs and the `ReadingExamSourceV1` export boundary.

## Session: 2026-06-01 / E8-66

### Typed Domain Seam: ReadingExamSourceV1 Export Boundary
- **Status:** complete for this typed-domain pass.
- Actions taken:
  - Added typed Rust DTOs in `src-tauri/src/reading_source.rs` for `ReadingExamSourceV1`, source meta, passage blocks, question groups, source refs, and audit.
  - Kept the public `reading_source(authoring)` return type as `serde_json::Value`, but now the export/runtime boundary builds typed structs before serializing to the existing `ReadingExamSourceV1` JSON contract.
  - Normalized passage blocks to the frontend/runtime contract shape with explicit `kind: "html"` entries before serialization.
  - Preserved compatibility with the Rust validator, preview renderer, runtime gate, export artifacts, and pack builder, all of which consume the existing JSON shape.
  - Added `reading_source_v1_preserves_export_contract`, a field-level contract regression covering schema version, exam metadata, passage block shape, question groups, source refs, audit, question order, and question display map.
- Behavior clarified:
  - `ReadingExamSourceV1` remains the authoritative export/runtime contract for preview, runtime validation, export, and Pack generation.
  - The typed seam reduces drift risk without changing the frontend-facing or persisted JSON contract.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test reading_source_v1_preserves_export_contract -- --nocapture)` | pass |
| `(cd src-tauri && cargo test reading_source_uses_real_source_metadata_and_review_status -- --nocapture)` | pass |
| `(cd src-tauri && cargo test authoring -- --nocapture)` | pass, 10 tests |
| `(cd src-tauri && cargo test)` | pass, 69 tests |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Notes
- This completes the export boundary typed seam that sits between `ReadingAuthoringIRV1` and preview/export/runtime consumers.
- The remaining highest-value typed seam is the validation/report contract layer, which still emits mostly ad hoc JSON values.

## Session: 2026-06-01 / E8-67

### Typed Domain Seam: ValidationReportV1 And Validation Layers
- **Status:** complete for this typed-domain pass.
- Actions taken:
  - Added typed Rust DTOs in `src-tauri/src/validator.rs` for `ValidationReportV1` and `ValidationLayerReportV1`.
  - Updated `validation_layers` to return typed layer reports with stable `layer`, `passed`, `issueCount`, `errorCount`, and `warningCount` serialization.
  - Updated `validate_authoring` to construct `ValidationReportV1` and serialize to the existing frontend `ValidationReport` JSON contract.
  - Preserved `runtime` as an optional JSON extension field because preview E2E diagnostics can carry provider/runtime-specific data.
  - Added `validation_report_v1_preserves_static_runtime_contract`, a field-level regression covering top-level validation report keys, static runtime metadata, layer ordering/counts, and zero-issue pass behavior.
- Behavior preserved:
  - Warning-only validation merges still do not block runtime gate progress.
  - Static Rust runtime gate continues to write `runtime.mode = static-rust` and `runtime.adapter = rust-static-contract`.
  - Preview/export/Pack gates continue consuming the same JSON contract.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test validation_report_v1_preserves_static_runtime_contract -- --nocapture)` | pass |
| `(cd src-tauri && cargo test runtime_gate -- --nocapture)` | pass, 3 tests |
| `(cd src-tauri && cargo test preview -- --nocapture)` | pass, 3 tests |
| `(cd src-tauri && cargo test)` | pass, 70 tests |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Notes
- The core backend contract chain is now typed at the major seams: SourceReview, SplitCandidates, ReadingAuthoringIR, ReadingExamSource, and ValidationReport.
- Remaining high-value architecture work is less about DTO drift and more about deeper runtime/provider diagnostics, live vision provider tests, and any remaining large module consolidation.

## Session: 2026-06-01 / E8-68

### Auto-Pipeline Review Gate Audit: Umbrella-Only And AuthoringReview Blocking
- **Status:** complete for this audit/hardening pass.
- Actions taken:
  - Audited Rust production `run_auto_pipeline_core_with_gateway` status projection around `requires_parser_review`, `requires_authoring_review`, LLM low-confidence groups, and static runtime pass state.
  - Added `auto_pipeline_blocks_umbrella_only_manual_import_from_export_ready`, a regression proving that an umbrella-only `Questions 14–26` split can pass structural validation/static runtime but still remains `NeedsReview` and cannot advance to `ExportReady` while manual concrete prompt import is pending.
  - Confirmed the regression preserves split and AuthoringIR flags: `requiresManualQuestionImport` remains true at both candidate and AuthoringIR group levels, `audit.humanVerified` remains false, and no cleanup summary is written.
  - Synced browser dev fallback `run_auto_pipeline` with Rust production behavior by including `refreshReviewState` / `authoringReview.needsReview` in status, current step, and issue count projection.
  - Extended frontend `AutoPipelineReport` type with `authoring.remainingReviewItems`, matching Rust pipeline report output.
- Behavior clarified:
  - `staticRuntimePassed=true` is not enough to advance to `ExportReady` if SourceReview, AuthoringReview, LLM review, or manual question import remains unresolved.
  - `LlmReview` may be the current step before `Authoring` when no usable LLM profile exists or suggestions are low-confidence; this is an earlier blocking state, not a publish-safety weakness.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test auto_pipeline_blocks_umbrella_only_manual_import_from_export_ready -- --nocapture)` | pass |
| `(cd src-tauri && cargo test auto_pipeline -- --nocapture)` | pass, 4 tests |
| `(cd src-tauri && cargo test)` | pass, 71 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `npm run check` | pass |
| `git diff --check` | pass |

### Notes
- This closes the main reviewed risk where auto-pipeline could theoretically treat a structurally valid umbrella-only draft as export-ready. The Rust path already had the correct blocker; the new regression locks it, and the dev fallback is now aligned.
- Remaining audit work should focus on live provider diagnostics and broader UI/E2E coverage rather than more status-projection fixes unless new edge cases appear.

## Session: 2026-06-01 / E8-69

### Opening `Questions 14-26` / `Questions 14–26` Inclusion Across Authoring And Export
- **Status:** complete for the clarified product requirement.
- Actions taken:
  - Revalidated the current umbrella-range split behavior: opening full-passage `Questions 14-26` / `Questions 14–26` is preserved in `umbrellaQuestionRanges` and not duplicated as a concrete Q14-Q26 group when later concrete subgroups exist.
  - Extended `ReadingAuthoringIRV1` generation so `passage.questionUmbrellaRanges` carries the opening total range beyond the split page.
  - Extended `ReadingExamSourceV1` generation so `meta.questionUmbrellaRanges` and `meta.questionIntroHtml` preserve the opening total range for preview/export/runtime source metadata.
  - Synced TypeScript types, browser dev fallback authoring generation, frontend `toReadingExamSource`, GroupEditor display, and UnifiedPreview/dev preview rendering with the Rust production path.
  - Strengthened `opening_umbrella_range_is_included_without_replacing_concrete_groups` to assert split, AuthoringIR, and ReadingExamSource all preserve the opening range while keeping concrete groups at `14-19`, `20-23`, and `24-26`.

### Verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo test opening_umbrella_range_is_included_without_replacing_concrete_groups -- --nocapture)` | pass |
| `(cd src-tauri && cargo test reading_source_v1_preserves_export_contract -- --nocapture)` | pass |
| `(cd src-tauri && cargo test umbrella -- --nocapture)` | pass, 5 tests |
| `npm run check` | pass |
| `(cd src-tauri && cargo fmt --check && cargo test)` | pass, 71 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `git diff --check` | pass |

### Notes
- This change intentionally does not make the umbrella range itself a publishable concrete question group when later subgroups are available.
- The publish-safety invariant remains unchanged: umbrella-only detections require manual concrete question import and AuthoringReview before export.

## Session: 2026-06-01 / E8-70

### Opening `Questions 14-26` Reconfirmation And Verification
- **Status:** complete; no additional code change required after audit.
- User clarified again that `Questions 14-26` / `Questions 14–26` appearing in the opening instructions is a correct题组 range and should be included.
- Rechecked the production Rust path and frontend/dev fallback:
  - `src-tauri/src/authoring_pipeline.rs` detects full-sentence and standalone heading opening ranges as umbrella question ranges.
  - `SplitCandidatesV1.umbrellaQuestionRanges` preserves the opening range.
  - `ReadingAuthoringIRV1.passage.questionUmbrellaRanges` carries it into authoring state.
  - `ReadingExamSourceV1.meta.questionUmbrellaRanges` and `meta.questionIntroHtml` carry it into preview/export/runtime metadata.
  - GroupEditor, SplitAndAnswers, UnifiedPreview, template rendering, and dev fallback expose the same context.
- Confirmed the safety invariant:
  - Later concrete subgroups are not replaced by a duplicate Q14-Q26 group.
  - Umbrella-only detections remain low-confidence `requiresManualQuestionImport` scaffolds and cannot publish until manually completed and verified.

### Verification
| Test | Status |
|------|--------|
| `npm run check` | pass |
| `(cd src-tauri && cargo test umbrella -- --nocapture)` | pass, 5 tests |
| `(cd src-tauri && cargo test files_pdf_samples_reach_expected_review_paths -- --nocapture)` | pass |
| `(cd src-tauri && cargo fmt --check)` | pass |
| `(cd src-tauri && cargo test)` | pass, 72 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `git diff --check` | pass |

## Session: 2026-06-01 / E8-71

### Browser UI-Flow E2E Diagnostic
- **Status:** complete for this diagnostic pass.
- Actions taken:
  - Added `sidecars/ui-flow-e2e/ui-flow-e2e.mjs`.
  - Added `npm run e2e:ui-flow`.
  - Updated `sidecars/README.md` to document that this is a development/CI diagnostic, not a production dependency.
  - Implemented direct Chrome DevTools Protocol automation using the host browser, avoiding new Playwright/Puppeteer dependencies.
  - Covered clear-text upload -> auto-pipeline -> `LlmReview` and OCR/scanned upload -> vision transcription -> SourceReview-first `DocumentReview`.
  - Fixed dev fallback validation-report drift: `mergeValidationReports` now preserves the `runtime` extension from the RuntimePreview report.

### Verification
| Test | Status |
|------|--------|
| `npm run e2e:ui-flow` | pass; clear text `NeedsReview` / `LlmReview` / `static-rust`, OCR `NeedsReview` / `DocumentReview` / SourceReview `required` / `visionApplied=true` |
| `npm run check` | pass |
| `(cd src-tauri && cargo fmt --check && cargo test)` | pass, 72 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `git diff --check` | pass |

### Notes
- The E2E script intentionally validates dev fallback browser behavior, not packaged Tauri runtime behavior.
- This raises confidence in UI routing and review visibility while preserving the production decision not to bundle Node/Python/OCR/browser automation runtimes.
- Remaining browser coverage should extend into manual SourceReview resolution, manual transcription, GroupEditor verification, preview generation, export, and Pack.

## Session: 2026-06-01 / E8-72

### UI-Flow E2E: Review To Export/Pack Closure
- **Status:** complete for this browser diagnostic expansion.
- Actions taken:
  - Added stable test selectors to GroupEditor (`validate-and-preview`), UnifiedPreview (`generate-preview-assets`, `run-preview-e2e`, `go-export`), and PackBuilder (`pack-builder`, `pack-job-checkbox`, `build-pack`, `pack-result`).
  - Expanded `sidecars/ui-flow-e2e/ui-flow-e2e.mjs` clear-text path to simulate human verification after low-confidence LLM review, validate through GroupEditor, generate preview assets, run runtime diagnostic, export four files, and build a pack.
  - Kept the OCR/scanned path as a blocking SourceReview check so scanned/vision output remains gated by human source verification.
  - Kept the script dependency-free beyond host Node and host Chrome/Chromium DevTools Protocol.

### Verification
| Test | Status |
|------|--------|
| `npm run e2e:ui-flow` | pass; clear text finalStatus `Cleaned`, `runtimeMode=static-rust`, `exportedFileCount=4`, `packBuilt=true`; OCR SourceReview `required`, `visionApplied=true` |
| `npm run check` | pass |
| `(cd src-tauri && cargo fmt --check && cargo test)` | pass, 72 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `git diff --check` | pass |

### Notes
- This remains a dev fallback browser diagnostic, not a packaged Tauri runtime proof.
- It materially increases coverage of the author-facing workflow: import, review, authoring verification, preview, export, and pack are now exercised from real browser UI actions.
- Remaining UI diagnostic gap is the scanned/manual transcription success path after SourceReview resolution.

## Session: 2026-06-01 / E8-73

### UI-Flow E2E: Scanned Manual Transcription To Export/Pack
- **Status:** complete for this diagnostic expansion.
- Actions taken:
  - Added a manual transcription fixture to `sidecars/ui-flow-e2e/ui-flow-e2e.mjs`.
  - Refactored the UI E2E release closure into a shared `completeReviewPreviewExportPack` helper so clear-text and scanned/manual paths use the same preview/export/Pack assertions.
  - Added `build-authoring-ir` test selector to SplitAndAnswers.
  - Extended the OCR/scanned path to paste manual transcription through DocumentReview, verify SourceReview is resolved/not required, run rule split, build AuthoringIR, simulate human verification, generate preview, run runtime diagnostic, export, and build Pack.
  - Fixed runtime-mode reporting in the E2E helper by capturing `static-rust` before export cleanup removes intermediate diagnostics.

### Verification
| Test | Status |
|------|--------|
| `npm run e2e:ui-flow` | pass; clear text and OCR/manual transcription both finalStatus `Cleaned`, `runtimeMode=static-rust`, `exportedFileCount=4`, `packBuilt=true`; OCR initial SourceReview `required`, `visionApplied=true`, manual provider `manual-transcription` |
| `npm run check` | pass |
| `(cd src-tauri && cargo fmt --check && cargo test)` | pass, 72 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `git diff --check` | pass |

### Notes
- This validates the intended scanned PDF workflow at browser-dev level: vision transcription is useful for extraction, but human source review/manual transcription remains the publish-enabling step.
- Remaining high-value external gap is live provider coverage for Rust vision/LLM calls when credentials are available.

## Session: 2026-06-01 Cross-Platform Secret Storage

### Phase 5 / E8-06: LLM Profile Secret Storage Hardening
- **Status:** complete for this sub-pass.
- Actions taken:
  - Replaced the macOS-only Keychain command-path assumption in `src-tauri/src/llm_profiles.rs` with the cross-platform Rust `keyring` adapter.
  - API keys now use OS secure storage by default: macOS Keychain, Windows Credential Manager, and system keyring/secret-service on other desktops where available.
  - Kept plaintext app-data file fallback disabled by default and still gated by `EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK=1` for dev/emergency use only.
  - Updated Settings UI and TypeScript profile types so the product copy no longer implies macOS-only storage.
  - Added environment preflight visibility for `security:os-secret-storage`.

### Verification
| Test | Status |
|------|--------|
| `npm run check` | pass |
| `(cd src-tauri && cargo fmt --check)` | pass after `cargo fmt` |
| `(cd src-tauri && cargo test)` | pass, 72 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `git diff --check` | pass |

### Remaining
- Real Windows Credential Manager behavior still needs verification on an actual Windows build machine; the production code path is now present through `keyring`.

### Follow-up Correction
- Explicitly enabled `keyring` features `apple-native`, `windows-native`, `linux-native-sync-persistent`, and `crypto-rust` in `src-tauri/Cargo.toml` after verifying the crate does not enable platform backends by default.
- `cargo tree -e features -i keyring` now confirms all intended platform secure-storage backends are enabled.

### Re-verification
| Test | Status |
|------|--------|
| `(cd src-tauri && cargo check)` | pass |
| `(cd src-tauri && cargo tree -e features -i keyring)` | pass; apple/windows/linux features enabled |
| `npm run check` | pass |
| `(cd src-tauri && cargo fmt --check && cargo test)` | pass, 72 tests |
| `(cd src-tauri && cargo clippy --all-targets -- -D warnings)` | pass |
| `git diff --check` | pass |

### Real Provider Smoke
- Used the temporary OpenAI-compatible test endpoint supplied by the user without writing the secret into repo files.
- Text JSON smoke passed: HTTP 200, parsed JSON `{ ok: true, task: "epic8-provider-smoke" }`, latency about 4.4s.
- Vision message-format smoke passed: HTTP 200, returned parsed JSON with `acceptedImage: true`, and provider usage included `image_tokens: 16`.
- This proves the configured provider accepts both structured JSON chat completions and OpenAI-style `image_url` content required by the Rust vision transcription path.
