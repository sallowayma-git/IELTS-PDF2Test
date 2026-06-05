# Progress

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
