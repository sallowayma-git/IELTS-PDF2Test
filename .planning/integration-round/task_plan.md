# Task Plan: Integration Round Regression Closeout

## Goal
Finish the resumed regression on `integration-round` at `1547bdf`: fix cloud repair terminal-state handling first (P0), then per-line review targeting (P1), exact-text alignment (P2), and the clean-candidate fixture (P3). Verify through the real Tauri product path and required checks; package NSIS only under the user's release condition.

## Current Phase
Phase 1: record the new work scope, then investigate P0 execution traces.

## Phases
### Phase 1: Read instructions and record scope
- [x] Read root `AGENTS.md` and `.planning/integration-round/handoff.md` in full.
- [x] Confirm current branch/worktree and preserve the shared target junction.
- [ ] Update and commit the three untracked planning records before code work.
- **Status:** in_progress

### Phase 2: P0 repair-cycle terminal-state bug
- [ ] Inspect the two retained product runs: `llm-usage` / `llm-calls`, `cache/llm`, processing-job stage and cloud-status timelines, and repair-packet queue states.
- [ ] Identify whether the loop waits, retries, spins, swallows an error, or fails to advance persisted state.
- [ ] Add bounded termination for success, step limit/no progress, and errors; all outcomes persist an existing terminal status and release edit locks.
- [ ] Add tests for error-to-terminal/unlock and step-limit termination.
- [ ] Capture first-red evidence and commit this item separately in Chinese.
- **Status:** pending

### Phase 3: P1 per-line large-difference review targets
- [ ] Emit slot/node `content_not_aligned` targets for individual instruction, stem, or option text below 0.6, while retaining the group threshold below 0.8.
- [ ] Preserve slot-to-group repair-packet mapping.
- [ ] Add a group test where one question is reviewed and its peers are adopted.
- [ ] Capture first-red evidence and commit separately in Chinese.
- **Status:** pending

### Phase 4: P2 exact-source sentence alignment
- [ ] Trace why the exact source sentence scores 0.55 (segmentation, order, OCR/text artifacts, or source typo).
- [ ] Fix `reconcile/alignment.rs`, add an exact-sentence regression at >=0.9, and rerun the 9-book alignment table without regressions.
- [ ] Capture first-red evidence and commit separately in Chinese.
- **Status:** pending

### Phase 5: P3 clean-candidate fixture
- [ ] Add the user-provided q27–q40 inferred answers to a test-only fixture with explicit inferred provenance.
- [ ] Repair group-2 swallowed stems according to the source structure, preserving all content.
- [ ] Ensure the clean-candidate phase still asserts overall adoption, empty `needsCloudReview`, zero repair calls, and no conflict todos.
- [ ] Capture first-red evidence and commit separately in Chinese.
- **Status:** pending

### Phase 6: Verification and package
- [ ] Run Rust lib tests; run frontend checks only if frontend files change.
- [ ] Confirm no cargo/rustc/link/IELTS app process before each heavy run; use the shared cargo target and one job.
- [ ] Run `build-app` once, then cloud-repair-chain and heading-presentation serially, at most three E2E rounds.
- [ ] If P0 is fixed and the only remaining failures after three rounds are fixture-only, package is allowed; if P0 is unresolved, do not package.
- [ ] Build NSIS only with two jobs, copy to the requested builds directory, and record size/SHA256/commit.
- [ ] Report commits, first-red evidence, P0 timeline/root cause, test outcomes and run directories, evidence levels, and unverified items.
- **Status:** pending

## Constraints
- Do not merge, push, stash, install, or clean shared build state.
- Never delete `src-tauri\\target`; it is a junction to the main repository's shared target.
- Keep heavy commands serial. Use `CARGO_TARGET_DIR=F:\\workspace\\PDF2Test\\src-tauri\\target`; use `CARGO_BUILD_JOBS=1` except NSIS packaging (`2`).
- Keep Chinese commit messages, one commit per item. Do not relax gates, add always-pass assertions or `#[ignore]`, or swallow errors.
- Do not access or print API keys.
