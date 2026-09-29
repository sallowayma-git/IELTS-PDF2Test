# Task Plan: IELTS task type presentation contract — Stage 1

## Goal

Complete only Stage 1 from the user's request in the `types-round` PDF2Test worktree: define a Rust single source of truth for task type presentation, mirror it in TypeScript with parity tests, add paragraph-anchor schema semantics, add strict quality gates, and report the existing failures across the private-real corpus. Stop before Stage 2 for coordinator review.

## Phases

- [complete] Verify repository instructions, worktree, private fixtures, pdfium, and npm setup.
- [complete] Review the existing uncommitted Stage 1 changes and current schema/gate architecture; add red contract tests.
- [complete] Finish the Rust/TypeScript presentation contract and parity coverage for every TaskTypeV2.
- [complete] Finish paragraph labels, paragraphMap semantics, host type, and compatibility coverage.
- [complete] Add quality-gate regression tests first, observe red, then implement strict contract checks.
- [complete] Run the gate against the official eight private-real PDFs and record every blocked group.
- [complete] Run targeted verification, prepare the Stage 1 report, and stop before Stage 2.

## Constraints

- Product contract/schema changes stay in this worktree; do not touch the student repository.
- Preserve existing uncommitted changes and do not edit the unrelated root planning history.
- Do not relax quality gates to make the PDF corpus pass.
- No cargo/build/e2e work without checking competing processes and at least 2.5 GB free RAM; use `CARGO_BUILD_JOBS=1` in the same PowerShell invocation.
- Do not proceed into Stage 2 or commit/push/merge before coordinator review.

## Errors

| Error | Context | Resolution |
|---|---|---|
| PowerShell parser rejected a pipeline inside a `foreach` block | Initial fixture/path inventory command | Rewrote the command to collect rows before formatting; no repository files changed. |
| Rust parity test found generated `src/types/taskPresentationRules.json` absent | Initial contract baseline | Recorded as expected red; generate the checked-in TS mirror from the Rust table after its schema is complete. |
| New rule-completeness test found `hostPlacement` absent | Current uncommitted presentation table | Expected red before production table changes; implementation will add location, grouping, option source, and reuse policy fields. |
| A combined `apply_patch` for quality tests could not match the exact neighboring lines | `quality.rs` test insertion | No files changed; read exact insertion context and split the patch into smaller hunks. |
| A combined patch for rule metadata constructors had a mismatched `GroupGranularity` context | `task_presentation.rs` | No files changed; reread the exact declaration and split the type/constructor edits into smaller patches. |
| Follow-up patch for A–D alphabet matching targeted the Rust rules table instead of the gate helper | `task_presentation.rs` / `quality.rs` | No files changed; reread the exact code and split the rule-row and gate-helper edits. |
| Cargo exact test filter omitted the full module path | New quality tests | Cargo ran 0 tests; discarded as evidence and reran by unique test-name substring. |
| Quality regression pass initially found six existing ready-fixture tests blocked because multiple-choice was modeled with inline group options | `quality.rs` presentation gate | The representative Choose TWO fixture has a shared task-group option bank. Updated the rule to require that shared bank, preserving the strict gate and the fixture's intended structure; rerunning the quality suite. |
| Planning-file patch used the wrong scoped path | `.planning` docs | No files changed; corrected to `.planning/tasktype-presentation-contract/`. |
| TypeScript test patch did not match the actual test ordering | `taskPresentation.test.ts` | No files changed; reread the exact current test block and split assertions from the barrel export edit. |
| Initial quality-gate red tests all failed, but compiler probes also reported invalid test authoring (`missing field sourceAnchors`) | New heading test helper | Fix the fixture to satisfy the typed IeltsAuthoringIRV2 contract before interpreting the expected red as gate evidence. |
| Initial full-evaluator paragraphMap red test produced unrelated provenance/compiler blockers | ParagraphMap target-identity regression | Replaced it with a direct gate-helper regression; the red result then had only the expected missing-anchor assertion, and the implementation now rejects targets outside the passage or without the matching label. |
| First local schema verification reported 16 errors for fields already emitted by current typed outputs | IeltsAuthoringIRV2 and QualityReportV2 contract schemas | Added optional `recognitionWarnings` and `questionCoverage` shapes, then updated the schema manifest digests; local verification now reports zero errors. |
| `rustfmt` reformatted several unrelated legacy expressions in `quality.rs` | Formatting touched file scope | Restored those formatting-only hunks while keeping the new implementation formatted; no behavior changed. |
