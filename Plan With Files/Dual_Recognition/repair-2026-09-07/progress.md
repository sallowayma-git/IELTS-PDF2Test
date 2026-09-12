# Repair Progress

## 2026-09-07

- User authorized repair of the audit findings and requested simple, efficient implementation without excessive security or validation work.
- Rechecked worktree, audit report, repository instructions, and Cargo dependencies.
- Started backend compilation, queue, and migration repairs.
# 2026-09-08 Implementation

- Restored Rust compilation (six errors), queue claim binding, recovery across recognition stages, per-execution leases and heartbeat, release of local permits before cloud.
- Unified source staging/registration and moved import file I/O to blocking runtime; queue/item creation now shares one transaction.
- Fixed revision pointer migration and conservative repair of unchanged shadow-seeded rows. Library migration/repository tests: 12 passed.
- Replaced post-save DS writes with one transaction; checked request replay payload and bounded journal to 200 saves.
- Reworked frontend save drain, UUID retry identity, failed flush rejection and local recovery with title/content.
- Added canonical batch publication using immutable release files and one Windows atomic manifest replacement. Still needs fault tests and product verification.
- Main import/list UI now wired to backend processing commands/events. Completing structural tools and regression coverage next.

# 2026-09-11 Handover Continuation

The 2026-09-08 session ended on API rate limits (429 / concurrency), mid-verification. This continuation verified and finished the repair.

## Verified

- `cargo check --locked` and `npm run check` pass; F01 resolved.
- Full Rust suite: 565 passed, 0 failed, 10 ignored.
- `npm run e2e:library-workspace`: 13 steps passed.
- `scripts/e2e/workspace-save-regressions.mjs`: all three save-chain checks pass — edits queued during an in-flight save are drained and transmitted, a failed flush blocks publication and retries with the same request id, and two windows never share a request id.

## Fixed In This Session

- F04/F02 test contract: `library_v2_workspace_api_persists_edits_without_new_revisions` still asserted the removed post-commit shadow sync. Rewritten to assert the new contract — the DB draft is authoritative, the shadow is not rewritten, and export resolves the canonical DS from `editVersion` alone. Batch publish now has `publish_items_is_all_or_nothing_across_a_batch` covering mid-batch interruption (nothing reaches the manifest) and full-batch commit (immutable release paths, status marked published, edit version not advanced).
- F13: `desktopDialogs.chooseExportDirectory` guarded the fallback only on a runtime flag, so Rollup could not eliminate the dynamic import and still emitted `devFallbackBackend` (97 kB) into `dist/assets`. Both fallback call sites now short-circuit on `import.meta.env.DEV`; production bundle no longer contains the chunk.
- Copy layering (findings F-M0-3): workspace load errors and editor patch/conflict failures showed raw machine codes and paths. Users now get user-level text; the raw code appears only in developer mode.
- Stale comments corrected in `library/commands.rs`, `library/repository.rs`, `library/libraryStore.ts`.

## 2026-09-11 (continued) — residual defects + P4-T01 first increment

### Closed

- `load_current_authoring` (`authoring_v2_commands.rs`) preferred the on-disk shadow whenever a job had no revision, even when a canonical DS existed in the DB. After a DB edit the shadow is stale, so any reader on that path saw pre-edit content. Resolution order is now revision → DB canonical DS → shadow (shadow only for not-yet-migrated jobs). Regression test `product_chain::legacy_authoring_session_reads_the_db_draft_not_the_stale_shadow` asserts a legacy session returns the DB title, not the stale shadow title.
- Removed the unused `STAGE_QUEUED` / `STAGE_RUNNING` imports in `processing/scheduler.rs`.

### P4-T01 first increment — unified physical ingest entry (DOCX parity)

`run_auto_pipeline_core` materialized the physical DocumentIRV2 shadow only when the main source was a PDF (`file_type == "pdf"`), while the manual parse command (`authoring_commands.rs::parse_document`) already dispatched `pdf | docx`. An auto-imported DOCX therefore reached authoring and quality evaluation with no physical layer at all, so it could never produce an authoring V2 session and could never become ready/publishable — the previous test comment stated this outright ("the physical DocumentIRV2 shadow is only written for PDFs"). This is the plan's own "unify the physical extraction entry" bullet under P4-T01.

The auto path now dispatches `pdf → write_pdf_facts_shadow_with_v1` / `docx → write_docx_facts_shadow_with_v1` through a single branch, mirroring the manual path.

`product_chain::product_chain_docx_import_materializes_the_physical_document_ir_v2` drives `fixtures/parser/complex-reading.docx` through the real `run_auto_pipeline_core` and asserts the full chain, not just the artifact: `authoring-ir.json` exists, the physical `DocumentIRV2` is schema-gated, bound to the originating job and carries at least one page, the authoring V2 shadow is produced, and `get_authoring_v2_core` opens an `AuthoringEditorSessionV1` carrying a text node. That last assertion is the actual product gate — without a physical layer the session is never produced.

The test is a proven regression guard, not a tautology: reverting the filter to `== "pdf"` makes it fail at `assert_shadow` (`product_chain.rs:113`), and restoring the fix makes it pass.

Scope note: by default a DOCX physical layer is OOXML-structural — `build_pages` derives pages/regions from the document model because `docx_ingest::render_fallback::requested_from_environment()` reads `EPIC8_DOCX_RENDER_ASSIST` and defaults to false. Rendered pixel geometry requires that flag. This unifies the ingest entry (P4-T01) but does not by itself raise DOCX recognition quality to rendered-geometry level; that remains P4-T05 scope.

### Verification (2026-09-11 continued)

- `cargo test --locked`: 567 passed, 0 failed, 10 ignored (566 → 567 with the new DOCX test).
- `npm run check`: pass. Frontend was not touched this round, so the bundle gates from the earlier round still hold.

## Still Open

- M4 (direct V2 recognition main path) and M5 (cloud full-candidate + three-way merge) are the plan's own milestone scope, not regressions. `auto_pipeline` still builds candidates through `make_dynamic_split_candidates` and compiles the V2 authoring shadow from that V1 IR; DocumentIRV2 is an auxiliary input to `build_authoring_v2_shadow`, not the main path. M4 has started with P4-T01's DOCX parity above; P4-T02~T06 (Question Layout Graph, geometry recovery, hard type closures, completion/visual, unassigned ledger) are untouched.
- Real Tauri import/edit/publish and NAS student-runtime acceptance were not executed in this environment.
- `src/app/legacyRoutes.tsx` is orphaned (App.tsx no longer imports it). This is a documented deferral, not a defect: `task_plan.md` assigns legacy-chain deletion to P10 and `findings.md` F17 records the same policy for `src/pages/LibraryPage.tsx`. Deleting only this file would orphan its eight imported page components, so the whole legacy set must go together in P10.
- The historical phase gates `verify:phase2:shadow`, `verify:phase3:docx`, `verify:phase4:grammar` remain red because they assert the retired requirement that `documentIrV2Shadow` default to `false`. `gate-status.md` already records this as a deliberate "do not touch"; de-shadowing (P4-T01's remaining bullet) will widen the gap and needs a coordinated contract update.
