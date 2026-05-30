# Progress Log

## Session: 2026-05-29

### Phase 1: Requirements & Discovery
- **Status:** in_progress
- **Started:** 2026-05-29 Asia/Shanghai
- Actions taken:
  - 确认当前活动 goal 已存在，沿用该目标推进。
  - 读取 `planning-with-files` 技能说明。
  - 执行 session catchup，未发现输出的未同步上下文。
  - 扫描工作区，确认当前只有两份 Epic8 设计文档。
  - 创建 `Plan With Files/task_plan.md`、`findings.md`、`progress.md`。
- Files created/modified:
  - `Plan With Files/task_plan.md` created
  - `Plan With Files/findings.md` created
  - `Plan With Files/progress.md` created

## Test Results
| Test | Input | Expected | Actual | Status |
|------|-------|----------|--------|--------|
| Planning files exist | `test -f ...` | 三份文件存在 | 待执行 | pending |

## Error Log
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-05-29 | `create_goal` failed: active goal already exists | 1 | 使用已有 goal 继续推进 |

## 5-Question Reboot Check
| Question | Answer |
|----------|--------|
| Where am I? | Phase 1: Requirements & Discovery |
| Where am I going? | 拆分任务后搭建 Tauri 工程并实现 MVP 到完整本地端功能 |
| What's the goal? | 实现 Epic8 Tauri 作者端本地应用全部开发任务并维护工程追踪 |
| What have I learned? | 当前仓库只有设计文档；需要从零搭建工程 |
| What have I done? | 已创建 Plan With Files 三文档和初始任务表 |

### Phase 1 Update: Document Decomposition
- **Status:** in_progress
- Actions taken:
  - 读取 Tauri 设计文档的产品形态、页面、模型、Command API、服务伪代码、MVP 和开发顺序。
  - 读取旧 Web 工程设计的输出契约、流水线、LLM 边界、模板生成、DOM 协议、校验层和工程任务拆分。
  - 检测本地工具链：Node/npm 可用；Rust/Tauri CLI 暂不可用或未在 PATH 中。
  - 读取 `frontend-skill`，确定应用 UI 采用产品工作台而非营销页设计。
- Files created/modified:
  - `Plan With Files/findings.md` updated
  - `Plan With Files/progress.md` updated

### Scope Correction: Local App Only
- **Status:** in_progress
- Actions taken:
  - 接收用户更正：作者端不是独立 Web，主要目标是本地 Tauri 应用端。
  - 调整实现策略：内嵌界面只是 Tauri 壳内界面；旧 Web 设计文档仅作输出契约参考。
  - 准备将 Rust 本地 command 与 app data 存储实现提到当前阶段。
- Files created/modified:
  - `Plan With Files/findings.md` updated
  - `Plan With Files/progress.md` updated

### Phase 2/3 Implementation Snapshot
- **Status:** in_progress
- Actions taken:
  - 初始化 Tauri 本地应用工程：`package.json`、Vite/React/TypeScript、`src-tauri`、capabilities、sidecar 目录。
  - 实现桌面内嵌界面页面：Dashboard、JobList、ImportWizard、DocumentReview、SplitAndAnswers、GroupEditor、LlmReview、UnifiedPreview、ExportPage、PackBuilder、Settings。
  - 实现同名 Tauri command 调用封装；Tauri 运行时走 Rust command，非 Tauri 开发预览走 dev fallback。
  - 实现 Rust 本地 command 源码骨架：job app data 目录、job.json、uploads/preview/exports、Document IR 占位解析、规则粗切、Authoring IR、校验、预览资产、导出、Pack、LLM profile 占位。
  - 添加 `.gitignore`，忽略 `node_modules/`、`dist/`、`src-tauri/target/`、`.DS_Store`。
  - 使用 Browser 冒烟验证：演示任务创建 -> 结构编辑器 -> 校验预览 -> 生成预览资产 -> E2E 按钮可用 -> 导出 JSON/JS/manifest/preview HTML。
- Files created/modified:
  - `package.json`, `package-lock.json`, `vite.config.ts`, `tsconfig*.json`, `index.html`, `.gitignore`
  - `src/**`
  - `src-tauri/**`
  - `Plan With Files/*.md`

## Test Results
| Test | Input | Expected | Actual | Status |
|------|-------|----------|--------|--------|
| Planning files exist | `test -f Plan With Files/{task_plan.md,findings.md,progress.md}` | 三份文件存在 | 三份文件存在 | pass |
| Frontend production build | `npm run build` | TypeScript + Vite build pass | Build passed, assets emitted under `dist/` | pass |
| Toolchain probe | `rustc --version`, `cargo --version` | Rust/Cargo available for Tauri compile | `rustc not found`, `cargo not found` | blocked |
| Browser smoke: demo job | Click `生成演示任务` | Opens GroupEditor with generated Authoring IR | GroupEditor opened with group-1/group-2 and answer fields | pass |
| Browser smoke: preview | Click `校验并预览`, `重新生成预览` | Validation pass and preview assets generated | Validation layers pass, answerKey populated | pass |
| Browser smoke: export | Click E2E then export | Export page lists JS/manifest output | JSON, single JS, manifest, preview HTML listed | pass |

## Error Log
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-05-29 | `create_goal` failed: active goal already exists | 1 | 使用已有 goal 继续推进 |
| 2026-05-29 | `GroupEditor` TypeScript narrowing produced `QuestionGroupDraft | undefined` errors | 1 | Added explicit `QuestionGroupDraft` type and narrowed through `currentGroup` |
| 2026-05-29 | Browser smoke script redeclared `snapshot` in persistent runtime | 1 | Retried with unique variable names, flow passed |
| 2026-05-29 | `rustc` and `cargo` are not installed/in PATH | 1 | Recorded as environment blocker for Tauri backend compile; source implemented but not compiled locally |

## 5-Question Reboot Check
| Question | Answer |
|----------|--------|
| Where am I? | Phase 3: Core Local Backend |
| Where am I going? | Replace placeholders with real parser sidecar, secure LLM gateway, real runtime preview/E2E, then Pack publishing polish |
| What's the goal? | 实现 Epic8 Tauri 作者端本地应用全部开发任务并维护工程追踪 |
| What have I learned? | 作者端没有独立 Web；旧 Web 文档仅是输出契约参考；本机缺 Rust/Cargo |
| What have I done? | 已搭建本地 Tauri 工程、内嵌界面、Rust command 源码、开发 fallback，并通过前端构建与冒烟流程 |

### Phase 3 Update: Local File Selection and Parser Sidecars
- **Status:** in_progress
- Actions taken:
  - Installed `@tauri-apps/plugin-dialog` and added `src/api/desktopDialogs.ts`.
  - Replaced browser file inputs in ImportWizard with explicit local path selection through the desktop dialog wrapper.
  - Updated ExportPage to select an export directory before generating reading assets.
  - Implemented Rust `.txt/.md` parsing fallback that reads copied uploads and emits real `DocumentIRV1` blocks instead of static sample blocks.
  - Added `sidecars/python-parser/parser.py` and `sidecars/node-validator/validate-reading-source.mjs` as local-app sidecar command entrypoints.
- Files created/modified:
  - `src/api/desktopDialogs.ts` created
  - `src/pages/ImportWizard.tsx` modified
  - `src/pages/ExportPage.tsx` modified
  - `src/styles.css` modified
  - `src/services/devFallbackBackend.ts` modified
  - `src-tauri/src/lib.rs` modified
  - `sidecars/python-parser/parser.py` created
  - `sidecars/node-validator/validate-reading-source.mjs` created
  - `sidecars/README.md` created
  - `package.json`, `package-lock.json` modified

## Additional Test Results - 2026-05-29
| Test | Input | Expected | Actual | Status |
|------|-------|----------|--------|--------|
| Dialog dependency install | `npm install @tauri-apps/plugin-dialog` | JS dialog package available | Installed 2 packages, 0 vulnerabilities | pass |
| Embedded interface build after dialog changes | `npm run build` | TypeScript + Vite build pass | Build passed | pass |
| Python parser unsupported extension guard | process-substitution path without extension | Reject unsupported input | `unsupported_parser_input:none` | pass |
| Python parser txt smoke | `/tmp/epic8-parser-smoke.txt` | DocumentIRV1 with role hints | Generated passage/question/answer blocks | pass |
| Node validator usage | no args | Print usage and exit non-zero | Printed usage | pass |
| Node validator positive case | `/tmp/reading-source-smoke.json` | ReadingExamSourceV1 + DOM pass | `passed: true` | pass |
| Import fallback smoke | ImportWizard create without desktop runtime | Opens DocumentReview | DocumentReview opened with parsed blocks | pass |

## Additional Error Log - 2026-05-29
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-05-29 | Parser smoke used process substitution path with no extension and failed with `unsupported_parser_input:none` | 1 | Re-ran with a real `.txt` temp file; parser generated DocumentIRV1 successfully |
| 2026-05-29 | Browser read-only evaluate could not override `window.prompt` | 1 | Avoided mutating browser API and validated default fallback import path instead |

### Resume Check - 2026-05-29
- **Status:** in_progress
- Confirmed active goal and existing Plan With Files state.
- Re-read current planning files, TypeScript fallback backend, Rust command source, shared IR types, and renderer contract.
- Next edits: fix fallback split/authoring heuristics, then port dynamic split/authoring generation into Rust command layer.

### Dynamic Split/Authoring Fix - 2026-05-29
- **Status:** in_progress
- Fixed non-Tauri fallback split generation to reassign contiguous `group-1`, `group-2`, ... IDs after filtering detected question headings.
- Fixed prompt extraction to stop final question prompts before answer/next heading boundaries.
- Browser smoke then exposed duplicate prompt content caused by combining `block.text` and stripped `block.html`; fixed fallback `blockText` and Rust dynamic `dynamic_block_text` to prefer source `text` and only fall back to stripped HTML when text is absent.
- Ported dynamic split/authoring helper logic into Rust command source and switched `run_rule_split`/`build_authoring_ir` to use `document-ir.json` where available.

## Additional Test Results - Dynamic Split/Authoring - 2026-05-29
| Test | Input | Expected | Actual | Status |
|------|-------|----------|--------|--------|
| Embedded UI build after dynamic split changes | `npm run build` | TypeScript + Vite build pass | Build passed | pass |
| Python parser txt smoke | `/tmp/epic8-parser-smoke.txt` | DocumentIRV1 with passage/question/answer hints | 4 blocks, roles `[passage,null,question,answer]` | pass |
| Node validator positive case | `/tmp/reading-source-smoke.json` | ReadingExamSourceV1 + DOM pass | `passed: true` | pass |
| Browser fallback smoke: generated Authoring IR | Click `生成演示任务` | Opens GroupEditor with contiguous groups and clean prompts | `group-1 Q1-5`, `group-2 Q6-8`; Q1-Q5 prompts clean, Q5 no duplicate text | pass |
| Rust/Tauri compile probe | `rustc --version && cargo --version` | Rust toolchain available | `rustc: command not found` | blocked |

## Additional Error Log - Dynamic Split/Authoring - 2026-05-29
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-05-29 | Large Rust patch failed because exact static sample context did not match | 1 | Switched to additive dynamic helpers plus command entrypoint patch, preserving static sample fallback |
| 2026-05-29 | Browser automation reused stale tab from another browser session | 1 | Created a new in-app browser tab for this session |
| 2026-05-29 | Browser script redeclared `demoButton` in persistent runtime | 1 | Retried with unique variable names |
| 2026-05-29 | Browser read-only evaluate could not clear `localStorage` | 1 | Avoided storage mutation and created a fresh demo job from the visible UI |
| 2026-05-29 | Browser smoke showed Q5 prompt duplicated text/html content | 1 | Updated fallback and Rust block text extraction to prefer `text` and only fallback to stripped HTML |

### Phase 3 Update: Rust Sidecar Dispatch - 2026-05-29
- **Status:** in_progress
- Actions taken:
  - Added Rust sidecar path discovery helpers for development and packaged resource layouts.
  - Connected `parse_document` TXT/MD flow to `python-parser/parser.py` with deterministic built-in parser fallback.
  - Connected `validate_authoring_ir` to `node-validator/validate-reading-source.mjs` and merged ReadingExamSourceV1/DOM validation layers with built-in Authoring IR checks.
  - Added `../sidecars` to Tauri bundle resources and documented Rust command integration in `sidecars/README.md`.
- Files modified:
  - `src-tauri/src/lib.rs`
  - `src-tauri/tauri.conf.json`
  - `sidecars/README.md`
  - `Plan With Files/task_plan.md`, `findings.md`, `progress.md`

## Additional Test Results - Sidecar Dispatch - 2026-05-29
| Test | Input | Expected | Actual | Status |
|------|-------|----------|--------|--------|
| Embedded UI build after sidecar dispatch | `npm run build` | TypeScript + Vite build pass | Build passed | pass |
| Python parser sidecar smoke | `python3 sidecars/python-parser/parser.py parse ...` | Writes DocumentIRV1 JSON | Provider `python-parser-sidecar`, 4 blocks | pass |
| Node validator sidecar smoke | `node sidecars/node-validator/validate-reading-source.mjs /tmp/reading-source-smoke.json` | ReadingExamSourceV1 + DOM pass | `passed: true` | pass |
| Rust/Tauri compile probe | `rustc --version && cargo --version` | Rust toolchain available | `rustc: command not found` | blocked |

### Phase 4/E8-06 Update: LLM Gateway and Review - 2026-05-30
- **Status:** in_progress
- Actions taken:
  - Added `sidecars/llm-gateway/gateway.mjs` for JSON-only group classification/extraction suggestions.
  - Added Rust local profile secret file storage fallback and redaction-friendly profile public fields.
  - Replaced Rust placeholder `test_llm_profile`, `llm_classify_group`, `llm_extract_group`, and `apply_llm_suggestion` with gateway dispatch, audit files, low-confidence guard, and patch application.
  - Updated dev fallback LLM suggestions and apply logic to match the structured patch behavior.
  - Expanded Settings and LlmReview UI to show LLM safety controls, key status, patch/questions/evidence, and low-confidence auto-apply blocking.
- Files modified/created:
  - `sidecars/llm-gateway/gateway.mjs` created
  - `src-tauri/src/lib.rs` modified
  - `src/pages/Settings.tsx` modified
  - `src/pages/LlmReview.tsx` modified
  - `src/types/settings.ts` modified
  - `src/services/devFallbackBackend.ts` modified
  - `sidecars/README.md` modified
  - `Plan With Files/*.md` updated

## Additional Test Results - LLM Gateway - 2026-05-30
| Test | Input | Expected | Actual | Status |
|------|-------|----------|--------|--------|
| LLM gateway deterministic smoke | `node sidecars/llm-gateway/gateway.mjs classify_group ...` | JSON-only suggestion with patch/questions/evidence | `kind=true_false_not_given`, confidence `0.72`, 2 patch ops | pass |
| Embedded UI build after LLM changes | `npm run build` | TypeScript + Vite build pass | Build passed | pass |
| Browser smoke: Settings LLM controls | Open Settings | Provider/forceJson/key safety/no-JS rule visible | All expected text visible | pass |
| Browser smoke: LLM Review high-confidence apply | Demo job -> LLM Review group-1 -> apply | Suggestion applies and returns GroupEditor | Apply enabled, returned to `题组结构化编辑器` | pass |
| Browser smoke: LLM Review low-confidence guard | Demo job -> LLM Review group-2 | Apply disabled and low-confidence warning visible | Apply disabled, warning visible | pass |
| Rust/Tauri compile probe | `rustc --version && cargo --version` | Rust toolchain available | `rustc: command not found` | blocked |

### Phase 3/8 Update: Global Toolchain, Rust Verification, Pack Zip - 2026-05-30
- **Status:** complete for toolchain setup; in_progress for full Epic8 scope.
- Actions taken:
  - Stopped/confirmed no Vite dev server remained on port 1420.
  - Probed global dependencies: Homebrew, Xcode Command Line Tools, Node/npm/npx, and Tauri CLI were present; Rust/Cargo/rustup were missing.
  - Attempted `brew install rustup-init`; Homebrew began source-building CMake/Rust dependencies on macOS 13, so the brew install tree was terminated and caches were cleaned.
  - Installed official Rust stable via rustup: `rustc 1.96.0`, `cargo 1.96.0`, `rustup 1.29.0`.
  - Added `. "$HOME/.cargo/env"` to `~/.zshrc` and `~/.zprofile`; added `rustfmt` and `clippy` components.
  - Installed global `@tauri-apps/cli@2.11.2` through npm so `tauri` is available without `npx`.
  - Implemented Pack publishing improvement: `ReadingExamPackV1` manifest, publish-before-validation gate, standard stored `.zip` writer, Pack UI metadata fields, and dev fallback Pack manifest/zip metadata.
  - Added minimal `src-tauri/icons/icon.png` to satisfy Tauri build requirements.
  - Fixed Rust compile errors found by first `cargo check` and cleaned clippy warnings.
  - Added `.playwright-mcp/` to `.gitignore`.
- Files modified/created:
  - `src-tauri/src/lib.rs`
  - `src-tauri/Cargo.lock`
  - `src-tauri/icons/icon.png`
  - `src/services/devFallbackBackend.ts`
  - `src/pages/PackBuilder.tsx`
  - `src/types/reading-source.ts`
  - `src/styles.css`
  - `.gitignore`
  - `Plan With Files/*.md`

## Additional Test Results - Toolchain and Pack - 2026-05-30
| Test | Input | Expected | Actual | Status |
|------|-------|----------|--------|--------|
| Global Rust versions | `rustc --version`, `cargo --version`, `rustup --version` | Rust toolchain available | `rustc 1.96.0`, `cargo 1.96.0`, `rustup 1.29.0` | pass |
| Rust components | `cargo fmt --version`, `cargo clippy --version` | Formatter/linter available | `rustfmt 1.9.0-stable`, `clippy 0.1.96` | pass |
| Global Tauri CLI | `tauri --version` | Global command available | `tauri-cli 2.11.2` | pass |
| Rust format/check/lint | `cargo fmt --check && cargo check && cargo clippy --all-targets -- -D warnings` in `src-tauri` | No Rust compile/lint failures | Passed | pass |
| Embedded UI build | `npm run build` | TypeScript + Vite build pass | Passed | pass |
| Full Tauri build | `npm run tauri build` | Release binary and macOS bundles generated | `.app` and `.dmg` generated under `src-tauri/target/release/bundle` | pass |
| Pack fallback smoke | Browser demo -> preview -> E2E -> Pack | PackBuilder shows publishable job and Pack result metadata | Flow reached PackBuilder and selected publishable job before toolchain pivot; full fallback Pack result not re-smoked after Tauri build | partial |

## Additional Error Log - Toolchain and Pack - 2026-05-30
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-05-30 | `brew install rustup-init` attempted a long source build of CMake/Rust dependencies on macOS 13 | 1 | Terminated brew process tree, ran `brew cleanup --prune=0`, used official rustup installer instead |
| 2026-05-30 | First `cargo check` failed: duplicate `render_group_html`, missing `src-tauri/icons/icon.png`, `Value::String` mapping, unsized `write_json` generic | 1 | Renamed helper to `render_group_body_html`, added icon, fixed mapping and `Serialize + ?Sized` bound |
| 2026-05-30 | `cargo clippy --all-targets -- -D warnings` failed on 10 style lints | 1 | Applied clippy suggestions and reran successfully |

### Phase 3/4 Update: PDF/DOCX Parser Adapter and Cargo Tauri CLI - 2026-05-30
- **Status:** complete for E8-04 parser sidecar adapter; in_progress for full Epic8 scope.
- Actions taken:
  - Re-read Plan With Files state and confirmed the active goal remains incomplete.
  - Used the existing PDF and DOCX fixture generator at `/tmp/make_epic8_docs.py`.
  - Verified `sidecars/python-parser/parser.py` compiles and parses deterministic TXT/MD/PDF/DOCX inputs.
  - Confirmed PDF fixture now splits glued `pypdf` text into passage heading, passage body, question instructions, numbered question statements, and answer block.
  - Confirmed DOCX fixture parses paragraphs plus table blocks directly from OOXML with Python stdlib.
  - Avoided global `pip install --user` after PEP 668 errors; no `--break-system-packages` was used.
  - Installed Cargo Tauri CLI globally with `cargo install tauri-cli --version 2.11.2 --locked`, so `cargo tauri` now works in addition to npm global `tauri`.
  - Re-ran parser smoke tests, Rust format/check/lint, TypeScript check, frontend build, and full Tauri release build.
- Files modified:
  - `Plan With Files/task_plan.md`
  - `Plan With Files/findings.md`
  - `Plan With Files/progress.md`

## Additional Test Results - Parser Adapter and Cargo Tauri - 2026-05-30
| Test | Input | Expected | Actual | Status |
|------|-------|----------|--------|--------|
| Python parser compile | `python3 -m py_compile sidecars/python-parser/parser.py` | Syntax valid | Passed | pass |
| PDF parser fixture smoke | `/tmp/epic8-parser-fixtures/reading-sample.pdf` | DocumentIRV1 with passage/question/answer role hints | Provider `python-parser-sidecar:pdf:pypdf`, 6 blocks, roles include passage/question/answer | pass |
| DOCX parser fixture smoke | `/tmp/epic8-parser-fixtures/reading-sample.docx` | DocumentIRV1 with passage/question/answer role hints | Provider `python-parser-sidecar:docx:ooxml`, 7 blocks including table, roles include passage/question/answer | pass |
| Global cargo Tauri CLI | `cargo tauri --version` | Cargo subcommand available | `tauri-cli 2.11.2` | pass |
| Global npm Tauri CLI | `tauri --version` | npm/global command available | `tauri-cli 2.11.2` | pass |
| Rust format | `cargo fmt --check` in `src-tauri` | No formatting diffs | Passed | pass |
| Rust compile | `cargo check` in `src-tauri` | No compile failures | Passed | pass |
| Rust lint | `cargo clippy --all-targets -- -D warnings` in `src-tauri` | No warnings | Passed | pass |
| TypeScript check | `npm run check` | No TS errors | Passed | pass |
| Embedded UI production build | `npm run build` | Vite build pass | Passed | pass |
| Full Tauri release build | `npm run tauri build` | `.app` and `.dmg` generated | Generated `IELTS Author Studio.app` and `IELTS Author Studio_0.1.0_aarch64.dmg` | pass |

## Additional Error Log - Parser Adapter and Cargo Tauri - 2026-05-30
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-05-30 | Homebrew Python rejected global `pip install --user` with PEP 668 externally managed environment | 1 | Avoided global Python package mutation; used installed `pypdf` for PDF and stdlib OOXML parsing for DOCX |
| 2026-05-30 | `cargo tauri --version` initially failed with `no such command: tauri` | 1 | Installed Cargo Tauri CLI via `cargo install tauri-cli --version 2.11.2 --locked`; `cargo tauri --version` now passes |

### Phase 4/E8-07 Update: RuntimePreview Contract Gate - 2026-05-30
- **Status:** in_progress for E8-07 because external real unified runtime E2E is still not wired; complete for local RuntimePreview contract simulator.
- Actions taken:
  - Re-read Plan With Files and the preview/E2E requirements in both Epic8 design docs.
  - Confirmed no external `reading-practice-unified.html` runtime was discoverable in the current workspace or `/Users/maziheng/Downloads/0.3.1 working`.
  - Added `sidecars/preview-e2e/preview-e2e.mjs`.
  - Enhanced `sidecars/node-validator/validate-reading-source.mjs` for stronger ReadingExamSourceV1 and DOM protocol checks.
  - Refactored Rust validation so `run_preview_e2e`, `export_reading_assets`, and `build_pack` all enforce the four-layer gate before export/publish.
  - Mirrored the RuntimePreview contract simulator in `src/services/devFallbackBackend.ts` so non-Tauri UI smoke follows the same gate semantics.
  - Expanded `UnifiedPreview` diagnostics for collected answers, score info, wrong-answer score, nav/question count, console errors, and issues.
  - Added `__pycache__/` to `.gitignore` and removed the generated Python bytecode cache.
- Files modified/created:
  - `sidecars/preview-e2e/preview-e2e.mjs` created
  - `sidecars/node-validator/validate-reading-source.mjs` modified
  - `sidecars/README.md` modified
  - `src-tauri/src/lib.rs` modified
  - `src/services/devFallbackBackend.ts` modified
  - `src/pages/UnifiedPreview.tsx` modified
  - `src/types/validation.ts` modified
  - `.gitignore` modified
  - `Plan With Files/*.md` updated

## Additional Test Results - RuntimePreview Gate - 2026-05-30
| Test | Input | Expected | Actual | Status |
|------|-------|----------|--------|--------|
| Node syntax checks | `node --check sidecars/preview-e2e/preview-e2e.mjs` and validator | Syntax valid | Passed | pass |
| DOM validator positive smoke | `/tmp/epic8-runtime-source.json` | ReadingExamSourceV1 + DOM pass | Passed | pass |
| DOM validator negative smoke | malformed bodyHtml with wrong input name | DomProtocol fails | Failed as expected with `No collectible control found for q1` | pass |
| RuntimePreview positive smoke | generated manifest/wrapper for `runtime-smoke` | RuntimePreview pass, registered exam, correct score 100, wrong score 0 | Passed | pass |
| RuntimePreview negative smoke | answerKey value absent from radio options | RuntimePreview fails | Failed as expected with radio answer option error | pass |
| TypeScript check | `npm run check` | No TS errors | Passed | pass |
| Rust format | `cargo fmt --check` in `src-tauri` | No formatting diffs | Passed | pass |
| Rust compile | `cargo check` in `src-tauri` | No compile failures | Passed | pass |
| Rust lint | `cargo clippy --all-targets -- -D warnings` in `src-tauri` | No warnings | Passed | pass |
| Embedded UI production build | `npm run build` | Vite build pass | Passed | pass |
| Full Tauri release build | `npm run tauri build` | `.app` and `.dmg` generated with bundled sidecars | Passed | pass |

## Additional Error Log - RuntimePreview Gate - 2026-05-30
| Timestamp | Error | Attempt | Resolution |
|-----------|-------|---------|------------|
| 2026-05-30 | No external real unified runtime was found in current workspace or expected sibling path | 1 | Implemented local RuntimePreview contract simulator and kept E8-07 in progress until real runtime E2E can be wired |
| 2026-05-30 | RuntimePreview smoke initially failed because the temporary fixture had been intentionally mutated for a negative test | 1 | Regenerated the positive fixture wrapper and reran successfully |
