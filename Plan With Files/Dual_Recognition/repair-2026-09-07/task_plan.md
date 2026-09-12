# Audit Repairs

## Goal

Fix the defects and disconnected product paths identified in the 2026-09-07 audit. Keep implementation simple: existing helpers, one authoritative draft, one save queue, and one batch publish path. Retain only checks required for saved edits, valid structure, usable assets, and complete publication.

## User Constraints

- Fix the reported problems; do not stop at a proposal.
- Avoid excessive security engineering, repeated data validation, or elaborate approval flows.
- Preserve existing worktree changes. Only one explorer was authorized and used in the audit; do not spawn another.
- Validate real Tauri workflows where possible. Distinguish browser adapters and command tests from actual product acceptance.

## Work

1. Restore backend compilation, task claiming/source registration/recovery, and correct legacy migration (F01/F02/F07/F08/F09). Status: complete.
2. Make editor saves reliable: request identity, one atomic DS update, draining saves, failed flush, draft retention (F03-F06). Status: complete.
3. Implement one batch publication path with frozen draft snapshots and no shadow dependency (F10/F12). Status: complete.
4. Connect backend processing and workspace structural repairs; remove test backend and retired Reading UI from production (F11/F13). Status: complete.
5. Address remaining production recognition/merge/cleanup gaps identified by the audit, using existing domain code where possible. Status: not started — M4/M5 are the plan's own milestone scope (direct V2 recognition main path, cloud full-candidate + three-way merge), not defects introduced by this worktree; the repair scope stopped at fixing the audited defects and connecting the built-but-unwired paths.
6. Run focused regressions, real Tauri import/edit/publish, and available student-runtime verification; report remaining external acceptance limits. Status: in_progress — Rust/browser layers verified; real Tauri + NAS runtime still not executed in this environment.

## Baseline

- HEAD: 47a3806.
- Preexisting dirty files: src-tauri/Cargo.lock, Cargo.toml, src/lib.rs, src/library/schema.rs, src/processing/.
- Previous audit evidence: ../audit-2026-09-07/report.md and artifacts/audit-2026-09-07/.

## Errors

None during repair yet. Initial current-worktree Rust build was already known to fail with six compilation errors.
