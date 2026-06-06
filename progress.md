# Progress

## 2026-06-06 Settings Preflight + 100-PDF Live Regression

- Started current task at 2026-06-06 17:43:02 CST.
- User provided a test-only OpenAI-compatible API key and endpoint `https://icoe.pp.ua/`; key will be used only as a transient environment variable and not written to repo files.
- Read `frontend-skill` and `planning-with-files` instructions for this UI + long regression task.
- Confirmed Settings preflight UI is currently bulky because every preflight check is rendered as a full `.layer-list` card.
- Confirmed `/Users/maziheng/Downloads/0.3.1 working/ReadingPractice/PDF` contains 262 PDFs.
- Confirmed `ReadingPractice` has no legacy JS oracle directory, so a new no-legacy 100-PDF Live pipeline diagnostic script is needed.

## 2026-06-06 Windows Package and Compatibility

- Started Windows package and compatibility goal from existing active `/goal`.
- Used `planning-with-files` workflow because the task requires a persistent, trackable task record across many implementation steps.
- Session catchup skipped because Codex native session parsing is not implemented by the helper script.
- Read `Files/Windows包体与兼容规划.md` and split the work into W0-W10.
- Detected dirty worktree before edits; no existing changes were reverted.
- Spawned three concurrent Subagents:
  - release/audit worker for `package.json`, package audit, macOS path compatibility, offline WebView2 config.
  - runtime worker for Rust Python resolver, PDF renderer adapter, environment preflight.
  - dev/e2e worker for Windows-compatible UI flow, preview, and PDF regression scripts.
- Created `Files/Windows包体与兼容任务追踪.md`.
- Updated `Files/Windows包体与兼容规划.md` with an execution tracking index.
- Added the Windows compatibility active goal to the top of `task_plan.md`, `findings.md`, and `progress.md` while preserving prior task history.
- W6 dev/e2e Subagent completed: `ui-flow-e2e` now uses `fileURLToPath`, Windows `npm.cmd`, and Windows Chrome/Edge paths; `preview-e2e` Python resolver supports `EPIC8_PYTHON`/Windows `py -3`; `pdf-regression-sample` no longer has a local macOS corpus default and supports Windows `.exe`.
- W6 validation reported passed: `node --check` for the three scripts, `node scripts/pdf-regression-sample.mjs --help`, expected exit 2 without explicit corpus dirs, and `npm run check`.
- Sent W6 integration finding to release/audit Subagent: `package.json` must update `test:pdf-regression` because the script now requires explicit `--pdf-dir` and `--legacy-dir`.
- W1/W2 release/audit Subagent completed: split macOS/Windows release scripts, platformized package audit, added Windows artifact metadata/sha256/WebView2/lockfile reporting, fixed `repack-macos-dmg.mjs` path handling, and added `src-tauri/tauri.windows.offline.conf.json`.
- W1/W2 validation reported passed: `npm run check`, `node scripts/package-audit.mjs --help`, `npm run audit:package`, expected Windows missing-artifact failure, fake Windows artifact success branch, expected `npm run test:pdf-regression` exit 2 without corpus dirs, and `git diff --check`.
- W8 remains open because Authenticode/PowerShell signature audit was not added in the release/audit worker pass.
- W3-W5 runtime Subagent completed and parent fixed one Rust borrow-check issue in `merge_rendered_page_images`.
- Runtime validation passed: `cargo check`, `cargo test environment_preflight_reports_required_dependency_names`, and `cargo test pdf_render_adapter`.
- Added `scripts/audit-windows-signatures.ps1`, `scripts/windows-install-instructions.txt`, and `.github/workflows/windows-smoke.yml` to close W7-W9.
- Final local verification passed: `npm run check`, `cargo fmt --check`, `cargo check`, `cargo test --manifest-path src-tauri/Cargo.toml` (115 passed, 2 ignored), `git diff --check`, `node --check` for the changed JS scripts, `node scripts/package-audit.mjs --help`, and `npm run audit:package`.
- Expected local limitations recorded: `npm run audit:package:windows` fails because this macOS workspace has no NSIS/MSI artifact; `pwsh` is unavailable locally for the Windows signature script; `npm run test:pdf-regression` now intentionally requires explicit `--pdf-dir` and `--legacy-dir`.

## 2026-06-04
- Started validation from existing dirty worktree.
- Confirmed key files are modified and `scripts/pdf-regression-sample.mjs` exists.
- `npm run check` passed.
- `cargo test --manifest-path src-tauri/Cargo.toml` passed: 94 passed, 2 ignored.
- `jq` is not installed locally; switched JSON inspection to Node one-liners.
- Fixed dev fallback routing so clear-text PDF lands directly on the editable draft page.
- `npm run e2e:ui-flow` passed with real fixture PDFs; scanned/manual transcription flow now creates a new draft revision instead of being blocked by overwrite protection.
- Added Rust regression coverage for manual transcription replacing the source document, archiving the previous draft, and regenerating a new editable draft.
- Tightened classifier rules after inspecting the 30-PDF regression report: explicit A-D evidence is required for single choice, explicit `Choose TWO/THREE letters` is required for multi choice, `according to` no longer triggers classification, `Questions X and Y` now preserves both question ids, and `match each statement/opinion/person` is ordinary matching rather than paragraph information matching.
- Updated PDF regression normalization so legacy JS table-rendered paragraph matching and old completion display variants compare by semantic kind instead of raw legacy field names.
- Changed auto-pipeline routing so generated drafts open the editable draft page by default; only source-document review keeps the user on document confirmation.
- Fixed remaining random-regression structure issues: sentence-ending matching now wins over single-choice detection, overlapping ranges are normalized, flow/diagram completion uses inline completion layout, and explicit completion groups can extend to numbered blanks present in the same source span.
- Final pre-build checks passed: `npm run check`, `cargo test --manifest-path src-tauri/Cargo.toml` (98 passed, 2 ignored), `npm run e2e:ui-flow`, and `npm run test:pdf-regression` with 30/30 structure pass on seed `1780508509492`.
- Random regression answer comparison still reports missing answers when answer pages have no extractable text; this is tracked as parser/scan limitation and is routed to user confirmation rather than production overwrite.
- Built fresh DMG: `/Users/maziheng/Downloads/Desktop/copy/PDF2Test/src-tauri/target/release/bundle/dmg/IELTS Author Studio_0.1.0_aarch64.dmg`, size 5.7 MB, SHA-256 `278d89b830ff5021f25bfd6918f9aa2358a77ce6456ec49606f01a28fa615a7d`.
- Re-ran the real OpenAI-compatible API chain with the new test profile on the Margaret Preston PDF; the CLI completed successfully and wrote `tmp/live-auto-pipeline-margaret-new-key.json`.
- Real vision answer extraction succeeded with 13 answers and populated `q8`-`q13` as `symbols`, `titles`, `stencilling`, `books`, `travel`, and `400`.
- Real cloud whole-paper comparison was attempted. Direct PDF upload was rejected by the provider as unsupported, the image fallback returned JSON, and the local draft remained authoritative because the cloud comparison disagreed with the local `8-13` layout/kind.
- Verified the resulting local draft status is `Authoring`, `nextRoute=groups`, `Questions 8-13` is one `sentence_completion` group with `layoutHint=inline_completion`, and `q8`-`q13` remain in the same group.
- `npm run check` passed after updating the routing tests.
- `cargo test --manifest-path src-tauri/Cargo.toml` passed: 98 passed, 2 ignored.
- `npm run e2e:ui-flow` failed because the script still expects the old source-review route during the OCR flow; product behavior now routes generated drafts to `groups`.
- `npm run test:pdf-regression` completed on seed `1780555370790`: 27/30 structure pass, 3 structure failures, 30/30 answer failures due to parser limitations on answer pages.

## 2026-06-06
- Read the pasted export failure: publish gate was blocked by `NeedsReview`, source-review parser warnings for page 5/6 no extractable text, low-confidence groups/questions, and empty answers.
- Added import UI stage tracking. During `runAutoPipeline`, the user now sees a cloud model wait message rather than only "正在生成".
- Added pipeline report fields and persisted audit summaries for vision answer extraction: attempted/applied state, answer count, filled question ids, and missing question ids.
- Added cloud comparison report fields and persisted audit summaries: issues, observations, local outline summary, and cloud outline summary. Local authoring remains authoritative.
- Added editor UI blocks for "视觉答案补全" and "云端整卷对照", including local-vs-cloud outline summaries and missing-answer warnings.
- Added "确认当前题组" action in the group editor to mark all questions in the active group verified after the user checks them.
- Reworked export-page error handling to parse `*_validation_failed:{...}` JSON payloads and show actionable guidance for empty answers, source review, verification, and cloud comparison.
- Extended the Node LLM sidecar command whitelist with `extract_pdf_image_answers` and `generate_pdf_reading_outline` for parity with the Rust gateway.
- Updated Rust cloud/vision regression coverage: cloud comparison summary persists after minimization, cloud mismatch does not overwrite local answers, and vision answer extraction exposes filled/missing question ids.
- Updated PDF image extraction tests to match full-page vision rendering behavior rather than the older single `sips` preview assumption.
- Verification passed: `npm run check`; `node --check sidecars/llm-gateway/gateway.mjs`; `cargo check --manifest-path src-tauri/Cargo.toml`; `cargo test --manifest-path src-tauri/Cargo.toml` (114 passed, 2 ignored).
- Started Vite at `http://127.0.0.1:1420/` after approval and checked the import wizard with the in-app browser.
