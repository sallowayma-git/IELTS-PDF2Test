# Student preview feedback — 2026-10-31

Goal: complete Note, matching matrix parity in author/NAS, and reproduce then repair Part persistence using real product paths. Base PDF2Test 0880eba. No commits, release/golden changes, or real user DB writes.

1. [complete] Inspect constraints, current UI/save/backend, prepare isolated cloud-direct structural fixture and before-fix CDP evidence.
2. [complete] Implement preview-only Notes and matching matrix parity with tests in both repositories.
3. [complete] Reproduce Part paths, repair proven root cause and own-save echo, add frontend/Rust regression tests.
4. [complete] Run requested check/test suites, fresh app builds and real Tauri/NAS product acceptance with reports/screenshots.
5. [complete] Review diffs and deliver file lists, evidence levels, commands/results and explicit gaps.

## Errors
- Initial root historical findings/progress read was too broad; use scoped plan and targeted current documents from now on.
- No root README or standalone StudentReadingTools/MatchingMatrix tests at guessed paths; locate actual tests before reading.
- Existing exe initially failed freshness check; rebuilt 0880eba before reproduction.
- Initial NAS test staging assumed historic cloud-direct exports contained P2; actual manifest was all P1. Use a separately verified, unchanged real P2 publication as companion.
- NAS directory picker rejects releases-only packages (electron/main.js requires root-level exam .js). Test now uses supported isolated exam-runtime.json configuration and records this separate gap.
- NAS fetch response is wrapped in data; test now unwraps and requires real V2 payload before asserting no answerKey.
- A natural scheduler/autosave race did not deterministically reject saves. Acceptance now uses an isolated persisted cloud-lock state and emits the matching processing notification; actual IPC/backend rejection and UI stop/retry remain real.
