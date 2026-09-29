# Plan: Complete task-presentation rollout (Stages 2–5)

## Goal

Continue from the completed Stage 1 contract and finish the user's complete IELTS presentation task across the PDF2Test editor and the separate IELTS-NASfor-WenDao student worktree. Preserve the current editor worktree and Stage 1 changes. Verify the real authoring, cloud-candidate, canvas, export, and student-rendering paths. Deliver local commits and a complete evidence report; do not push or merge feature branches back to main.

## Worktrees and boundaries

- Editor: `F:\workspace\PDF2Test-types`, branch `types-round`; Stage 1 changes are uncommitted and must be preserved.
- Student: reuse `F:\workspace\IELTS-NASfor-WenDao-types` only after confirming its branch and dirty state. Never edit `F:\workspace\IELTS-NASfor-WenDao` or `F:\workspace\IELTS-NASfor-WenDao-listening`.
- Stage 2 corpus requirement lists nine names while calling it eight; inspect the files and metadata and cover every listed physical PDF, recording the discrepancy rather than silently dropping `listening-vol7-t9`.
- Final local branch commits are expected by the requested report. No pushes and no merge of either feature branch into main. If refreshing a feature branch from its own main is needed, do that before final validation and record it.
- Keep Stage 1 uncommitted work intact until the planned editor commit; keep student changes isolated to its feature worktree.

## Phases

- [complete] Recover prior context, reread the exact Stage 2–5 requirements, check both worktrees and instructions, and establish test/build preflight.
- [complete] Stage 2: consolidate V1/V2 local task classification; recognize paragraph labels without inventing them; build correct response groups, anchors, option banks, and conservative direct-canonical rules; manually inspect the private PDFs and add exact corpus expectations.
- [complete] Stage 3: generate cloud candidate rules/examples from the Rust contract; provide source paragraph anchors; repair candidate adaptation, option-bank scope, repair prompts, setTaskType structure changes, and roman-answer case handling; pass controlled cloud-chain E2E including heading anchor adoption.
- [complete] Stage 4: implement interaction-driven canvas controls and pointer-based drag/drop for passage, row, and inline targets; preserve editing commands and review-lock behavior; add and pass real Tauri/CDP heading persistence/reopen/publish E2E.
- [complete] Stage 5: preserve the editor export projection while exporting the complete new presentation contract; implement student V2 matching/drag/drop/select/matrix rendering and source-reference validation in its feature worktree; verify the real Electron student flow.
- [complete] Run final editor/student tests and serial E2Es with one app build; inspect both diffs; create Chinese local commits; prepare per-stage changes, red evidence, nine-PDF corpus, test layers, hashes, and remaining evidence gaps. No push or merge to main.

## Operating constraints

- Root repository instructions require product-path evidence; CLI/schema success cannot stand in for product E2E.
- Every behavior change gets a failing regression first. Keep the quality gate strict. Never use `let _ =` to discard errors.
- Before Cargo/build/Tauri commands: verify at least 2.5 GB free RAM and no competing cargo/rustc/link/Tauri build; set `CARGO_BUILD_JOBS=1` in that same PowerShell invocation. Never retry a system-killed compiler.
- Run the app build once, then run the final E2E scripts serially in the order specified in the original request.
- Treat `listening-vol7-t9.pdf` explicitly because Stage 2 names it despite the phrase “eight PDFs”; determine corpus membership from actual files and write down the resolved set.
- Do not publish, push, merge feature branches into main, or edit either protected main checkout.

## Errors and decisions

| Error or discrepancy | Context | Resolution |
|---|---|---|
| Stage 1 requested eight PDFs but its official manifest omitted the physically present `listening-vol7-t9.pdf`; Stage 2 explicitly names it while still saying eight | Corpus scope | Inspect original PDFs and metadata; include all nine listed physical files in the Stage 2 manually verified corpus unless the authoritative request data proves otherwise, and report the count discrepancy. |
| Stage 4 says stop before Stage 5, while the user now explicitly requests every stage | Continuation authority | Proceed through Stage 5 in the authorized student feature worktree; the latest user instruction supersedes the prior wait point. |
| Stage 4 requests local main refresh, while final instructions prohibit merging | Git boundary | Distinguish refreshing feature branches from their own main from merging the feature branch back into main; perform no final integration merge or push. |

## Final verification snapshot (2026-09-29)

- Editor: `npx tsc --noEmit` passed; `npx vitest run` passed 512/512 across 46 files; `cargo test` passed 1292, failed 0, ignored 14; `npm run build:app` passed once. Built executable SHA-256: `685c97dcf58e9e56d4c770f6edb2e52c3a63ca6c6389016336c5c03ca5ce1373`.
- Product E2E on that build: heading presentation UI→adopt→drag→save/reopen→source review→preflight→publish passed; TFNG layout 9/9; option drag 5/5 plus five concurrent writes; edit/save under recognition 30/30; cloud repair chain overall passed 17 checks. The chain report leaves derive-answer non-executable because the selected source golden has no `answerErrors` annotation and marks the retry defect as not reproduced; neither was counted as a product failure.
- Export/student evidence: NAS package contract 43/43; the compiled student NAS provider loaded the published V2 package 19/19, including 13 question slots, three task groups, full answer-key coverage, and preserved task presentation. The complete Electron student practice flow passed after the E2E explicitly closed the Notes modal before using the part-navigation overlay.
- Student static suite passed. Its optional author/student-contract check skipped because `IELTS_PDF2TEST_REPO` was not set; separate Reading V2 vertical-slice and Phase 6 cross-repository checks passed. No UI product defect was found in the earlier Notes click failure: the modal overlay correctly intercepted the click, and the E2E harness now follows the visible close control.
- Stage 2 corpus coverage includes all nine named PDFs (the request's count says eight but the list contains nine). The manually checked source expectations and source ambiguities are in `findings.md` and `fixtures/golden/private-pdf-task-presentation-stage2.json`; the real-document service pipeline passed 1/1. No source PDF was changed.
- Product evidence levels: heading path and student flow were exercised through real Tauri/Electron UIs; the cloud chain used the controlled cloud service; PDF corpus validation and Cargo contract suites are service-level evidence. No live external LLM provider request was made.
