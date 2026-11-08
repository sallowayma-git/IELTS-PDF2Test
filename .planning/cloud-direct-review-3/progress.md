# Progress
Started review-3. Working tree initially only has three untracked user review documents.

P0: scheduler 21/21 tests passed, including final READY SQL-trigger failure -> failed + PROCESSING_ADVANCE_FAILED. Stale worker canonical seed/replace test passed. cargo check passed. Product E2E pending.

Offline regression first compile found regex crate absent; replaced range parsing with dependency-free bounded parser. Latest AGENTS instructions received; same product-first scope continues.

P0 offline group committed: reading 13/13 tests pass; A-Z, exact count/mismatch, duplicate title covered. Historical Maori has 10 body paragraphs plus duplicated title; corrected title handling. Five real-PDF product regression remains final E2E.

P1 warnings committed. direct suite 26/26, quality 65/65, authoring validation 6/6 passed. Tests preserve ANSWER_MISSING and assert <10 direct warnings and no local evidence/source-review warnings.

P2 audit and explicit no-sandbox switch committed 576aad4; node --check passed. Product build started with authoritative build-app script. No default sandbox bypass.

First default sandbox E2E: online all checks passed including warning counts, preview and publish; NAS96/96. Offline Tea fixed, Maori still missing map. Root cause: physical shadow omits instruction pages; local recovered V1 instructions contain A-J but were not included in derivation. Added recovered instructions/section evidence and regression that compiles actual preview. Must rebuild and rerun both rounds.

Maori followup d9739ab: recovered V1 instruction arrays plus split section evidence now supply declaration independently of physical pages. Preview compiler regression passes; full grammar suite26/26 passes. Found and fixed warning targetType passage (not valid schema) -> document. Final product rebuild started.

Final build succeeded: exe SHA256 7390f6e13f13af9719da1620a006780906e6dfcd81d45a00f55b06296df00aa7. Main-thread default acceptance restarted, no flags. Await final report. Source has no outstanding edits; only local planning and user review docs untracked.

Final default sandbox main-thread run artifacts/e2e-cdp/run-cloud-direct-2026-10-14T22-11-03-915Z/report.json: verdict=passed; diagnosticRun=false; sessionBrowserArgs empty. Online18/18, offline6/6, NAS96/96. Answers66/66. Online warning counts1/1/3/1/1; offline Tea map8 and Maori map10 + exactly one PARAGRAPH_LABELS_DERIVED warning. QA report rewritten with default primary evidence and restricted no-sandbox supplemental classification. Product source unchanged after final build.
