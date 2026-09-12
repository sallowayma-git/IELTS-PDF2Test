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

## 2026-09-12 — P4-T02 Question Layout Graph (§6.2–§6.7)

New recognition layer that reads `DocumentIRV2` directly instead of starting from V1 `questionGroupCandidates`, plus the geometric intermediate described in plan §6.2.

### Added

- `src-tauri/src/recognition/mod.rs` — module root. Owns the product-path artifact names and `write_question_layout_graph_artifact`, the single entry point that turns a physical `DocumentIRV2` value into `question-layout-graph.json`.
- `src-tauri/src/recognition/local/mod.rs` — the graph types (`QuestionLayoutGraphV1`, `PageLayoutGraph`, `QuestionBlockCandidateV1`, `OptionBankCandidateV1`, `VisualStimulusCandidateV1`, `OptionRunCandidateV2`, `UnassignedEvidence`, `InstructionZoneCandidate`, `RegionLayoutNode`, `QuestionNumberToken`, `SemanticRegionRole`) and both schema gates.
- `src-tauri/src/recognition/local/question_blocks.rs` — the §6.3 pipeline in the mandated order: region role segmentation → instruction zones → token-first number detection → geometric block expansion → local option-run detection → shared option-bank detection → visual stimuli → unassigned ledger. Task classification is deliberately absent (§6.3: classification must not precede boundary recovery).
- `mod recognition;` in `lib.rs`; `ielts_grammar::{question_number, issue_codes}` widened to `pub(crate)` so the new layer reuses the existing "Questions N-M" parser and the stable issue-code vocabulary instead of duplicating either.

### Product-path wiring (not CLI-only)

`auto_pipeline::materialize_question_layout_graph` derives the graph from the same physical shadow the authoring layer consumes, immediately after the unified physical ingest. Non-fatal by design: the import has already produced a valid physical document, so a graph failure is recorded in `question-layout-graph.error.json` rather than failing the job. The writer verifies its own output by reading the artifact back through the consumer schema gate — a graph that cannot be re-read is not reported as written.

### Deviations from the plan's pseudocode §6.2 (both deliberate)

- `number_anchor` is `Option<SourceAnchorV2>`. A numeric span may carry no source anchor; defaulting it to a fabricated anchor would invent provenance.
- `ambiguities` is `Vec<String>` of the stable codes in `ielts_grammar::issue_codes`. The plan's `RecognitionIssueCode` type does not exist in this crate, and the existing vocabulary is already the single source of truth.

### Three behavioural findings the tests forced out

1. **The last question's interval swallowed the shared bank.** The final question's search interval is open to the page bottom, so a `List of Headings` bank sitting below the questions was being read as that question's own option run. Fixed by reserving the lines of regions whose role is `SharedOptionBank` or `AnswerKey` from stem continuation and from local option-run detection. Passage and instruction lines are deliberately **not** reserved (completion tasks interleave stems with passage prose; that belongs to §6.8 classification).
2. **A single-choice prompt and its own option run are one physical region.** Requiring `owningRegion == QuestionPrompt` for the §6.5 "+0.15 prompt left edge" bonus dropped the number token below the 0.55 threshold for a standard single-choice block. The bonus now applies when the owning region is `QuestionPrompt` **or** `OptionRun`.
3. **The bank heading landed in the unassigned ledger.** `List of Headings` is consumed by no question and is 12+ characters, so it was reported as unexplained evidence even though the bank was emitted as an artifact. Exoneration is now evidence-based: a bank region's lines are exonerated **only when a bank was actually emitted for that region**, so a bank-shaped region that fails the label-run test stays in the ledger.

### Verification

- `cargo test --locked --lib recognition`: 4 passed. Two fixture-driven tests (a matching task with a trailing shared bank; a single-choice task with a local option run) assert instruction zones, region roles, token scores, block numbers and stems — including a wrapped continuation joining its stem — option runs, banks, `sourceCoverage == 1.0`, and the unassigned ledger. Two gate tests assert the producer rejects malformed/foreign documents and that a foreign graph generation is rejected on read.
- `cargo test --locked` (full): 581 run, **571 passed / 0 failed / 10 ignored** (567 → 571 with the four new tests).
- `product_chain::product_chain_pdf_import_...` and `product_chain::product_chain_docx_import_...` both assert the graph through one shared helper, deliberately format-agnostic: the P4-T01 defect was exactly a PDF-only branch, so a new physical-derived artifact is asserted on both branches. The helper checks schema gating, job binding, `documentId` parity and per-page `pageIndex`/`widthPt`/`heightPt` parity against the physical document (derived, not fabricated), that every question block traces back to a reported number token, and that no block carries a silently empty stem (empty stems must declare `PROMPT_EMPTY`).
- `npm run check`: pass (no frontend change).

### Scope boundary — this is **not** M4 main-chain replacement yet

The graph is now produced on the real product path, but **nothing consumes it yet**: `auto_pipeline` still builds candidates through `make_dynamic_split_candidates` → V1 IR → compiled V2 authoring shadow, exactly as recorded in the 2026-09-12 audit. P4-T02 as specified here (the graph layer) is done; §6.8 task classification, §6.10 hard closures and the main-chain switch remain ahead (P4-T03~T06).

## 2026-09-12 (continued) — P4-T03~T06, main-chain consumption, contract hash repair

### Added

- `src-tauri/src/recognition/local/task_groups.rs` — §6.8 task classification + hard closures, §6.9 matching-headings priority, §6.11 unassigned-evidence ledger. One group per instruction zone plus contiguous undeclared-number runs. Closures: source coverage ≥ 0.92 with option labels/text present, unique labels and ≥ 3 options for choice; the exact fixed IF/NG response set for TFNG/YNNG; a non-empty bank for the matching family; `SIGNIFICANT_SOURCE_TEXT_UNASSIGNED` when ≥ 80 chars of unassigned prose fall inside a question span; `QUESTION_NUMBER_MISSING` for declared-but-absent numbers.
- `src-tauri/src/recognition/local/stimulus.rs` — §6.10 physical tables (real rows, columns and row/col spans projected from `TableNodeV2`, never a fabricated one-question-per-row) and the diagram/image hybrid (source crop plus normalised-rect hotspots per slot). Low table topology without a crop, or slots that cannot be attached, block with `ASSET_REFERENCE_MISSING` / `SLOT_OUTSIDE_FIGURE` instead of silently dropping the surface.
- `QuestionLayoutGraphV1` gains `task_groups` and `table_stimuli`; `blocking_issues()` folds the table/visual issues in; `blocking_issue_targets()` returns `RecognitionBlockerTargetV2`. `issue_codes.rs` gains `OPTION_LABEL_MISSING` and `OPTION_TEXT_MISSING`.

### Main-chain consumption (staged switch)

- `build_authoring_v2_shadow` derives the graph's verdict from the physical shadow via `recognition::blocking_issues_from_physical` / `blocking_issue_targets_from_physical` and records it on the authoring document as `recognitionBlockers` / `recognitionBlockerTargets` (new optional schema fields, `skip_serializing_if` empty, so existing documents and golden fixtures are unaffected).
- The quality gate consumes that verdict through `validate_recognition_blockers`, gated by `recognition_blockers_gate_enabled()` (`LOCAL_RECOGNITION_BLOCKERS_GATE`, **default off**). `evaluate_quality` delegates to a new `evaluate_quality_with_gate(.., recognition_gate_enabled)` so the switch is unit-testable without mutating process-global env state.
- Boundary, stated plainly: the main chain still produces the authoring structure through `make_dynamic_split_candidates` → V1 IR. The graph is consumed as blocker codes + targets, not yet as the primary authoring producer.

### Repaired pre-existing contract drift (not introduced this round)

- `contracts/contract-manifest.json` pinned 6 stale sha256 values (`ContentDocV2`, `IeltsAuthoringIRV2`, `ReadingExamSourceV2`, `ListeningExamSourceV1`, `ListeningAttemptV1`, `ListeningAudioProbeResultV1`). `git show 6428fe8` proves the commit replaced the matching values with values that match no file; at `8806272` the manifest and the schema files were consistent. Result: `verify:phase1:schema` and `verify:phase6:runtime` failed on a clean checkout.
- Corrected the 6 hashes by targeted replacement (verified: only 64-hex substrings changed; the compact `readableInputVersions` line is preserved).
- `contracts/ielts-authoring-ir-v2.schema.json` root has `additionalProperties: false`, so a blocked document would have been rejected. Declared `recognitionBlockers` / `recognitionBlockerTargets` in root properties plus `$defs/recognitionBlockerTarget`. Added 1 positive acceptance fixture (`derived:early-approaches:authoring-recognition-blockers`) and 2 negative probes (`authoring-rejects-unknown-recognition-blocker-target-field` → `additionalProperties`, `authoring-rejects-empty-recognition-blocker-code` → `minLength`).

### Verification

- Recognition unit tests: 15 passed (4 question blocks, 7 task groups, 4 stimulus).
- `cargo test --locked` (full): 593 run, **583 passed / 0 failed / 10 ignored** (571 → 583).
- `npm run check`: pass.
- `verify:phase1:schema:local`: **0 errors** (was 6). `verify:phase6:runtime`: pass.
- `verify:phase7:listening-contract`: reaches the peer-repo step and fails only at `npm run build:server` inside `../NAS`, which is absent here; every gate before it (npm check, cargo listening tests, listening-package, manifest hash) passed.
- `verify:phase4:grammar`: still red — see the deliberate deferral note under Still Open.
- Real Tauri import/edit/publish and NAS runtime acceptance were not executed here.

## Still Open

- M4 is substantially advanced but not closed. P4-T01~T06 are implemented: the graph is produced from physical `DocumentIRV2`, and its §6.8/§6.10/§6.11 verdict is now consumed by the quality gate as `recognitionBlockers` (staged behind `LOCAL_RECOGNITION_BLOCKERS_GATE`, default off). What remains for M4 is making the graph the *primary producer* of the authoring structure: `auto_pipeline` still builds candidates through `make_dynamic_split_candidates` and compiles the V2 authoring shadow from that V1 IR. M5 (cloud full-candidate + three-way merge) is untouched.
- Real Tauri import/edit/publish and NAS student-runtime acceptance were not executed in this environment.
- `src/app/legacyRoutes.tsx` is orphaned (App.tsx no longer imports it). This is a documented deferral, not a defect: `task_plan.md` assigns legacy-chain deletion to P10 and `findings.md` F17 records the same policy for `src/pages/LibraryPage.tsx`. Deleting only this file would orphan its eight imported page components, so the whole legacy set must go together in P10.
- The historical phase gates `verify:phase2:shadow`, `verify:phase3:docx`, `verify:phase4:grammar` remain red because they assert the retired requirement that `documentIrV2Shadow` default to `false`. `gate-status.md` already records this as a deliberate "do not touch"; de-shadowing (P4-T01's remaining bullet) will widen the gap and needs a coordinated contract update.
- `verify:phase7:listening-contract` cannot complete here for an environmental reason: after all local gates pass it builds the peer repository at `../NAS`, which is absent in this checkout.
