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
  - 修复关键门禁策略：发布链路默认 strict real-runtime gate，避免 fallback 通过掩盖真实兼容性问题。
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
