# Audit Evidence

## Initial Observations

- The supplied continuation plan requires all M0-M7 milestones and real Tauri/NAS acceptance before unified delivery.
- HEAD is 47a3806. Git history records M0 and M1, with uncommitted processing module changes.
- Existing task_plan.md marks M2 next and M3-M7 pending.
- Existing gate-status.md reports Tauri publishing blocked and NAS validation at contract level; actual Electron testing was deferred to M6.
- These are observations to verify, not proof that every planned capability is absent.

## Explorer Leads (Awaiting Targeted Verification)

- Migration reads current-revision.json as a document, but it is a pointer. It falls back to the initial shadow instead of resolving revisions/{revision}.json.
- library/commands.rs refreshes quality after the editor transaction and replaces canonical_ds_json without a version predicate, creating a lost-update race.
- auto_pipeline.rs still unconditionally produces V1 split/authoring; ielts_grammar builds groups from V1 candidates before producing V2 shadow.
- Cloud still uses CloudReadingOutlineV1 and comparison-only prompts; no full candidate/repair/salvage/three-way merge path found.
- publishItems performs sequential independent single-item commits; no batch atomicity.
- Canonical publishing depends on shadow revision zero; editVersion is not a checked database binding.
- Cleanup retains fixed legacy files and does not derive reference sets from current/recovery/running/staging state.

## Root Verification

- Current working tree does not compile: cargo check --manifest-path src-tauri/Cargo.toml --locked exits 1 with six errors in processing/scheduler.rs, processing/commands.rs, and lib.rs (E0382 twice, E0521 twice, E0308, E0624). Current Tauri E2E is blocked by this build failure.
- npm run check passes.
- Confirmed migration pointer/document mismatch against artifact_store::CurrentRevisionV2 and append_revision save path.
- Confirmed post-transaction whole-DS overwrite without version predicate in library/commands.rs:116-119.
- Additional deterministic multi-window issue: useCanonicalEditor.ts:151 builds request IDs from item/version/per-component sequence; repository.rs:254-272 accepts matching IDs before checking version or payload. Two windows' first saves from the same version collide.
- Import UI still calls createImportJob/importSourceFile and runs local pool to completion before cloud pool; it never calls new import_files. Library refresh still polls every two seconds.
- EditorCommandV1 has only three variants. Workspace connects text and answer callbacks, not structural patch callbacks or hotspot updates; original-file drawer displays extracted text and redirects repairs to legacy pages.
- Save persist() only waits for an existing in-flight request and returns, leaving later queued edits without a new timer. Save failures are swallowed, so workspace publish can continue after a failed flush. Browser reproductions are being prepared.

## Focused Verification Results

- npm run build passes, but emits dist/assets/devFallbackBackend-BVm-q9ld.js (97.43 kB). Dynamic loading has not removed the adapter from production assets.
- npm run e2e:library-workspace passes all 13 browser-adapter steps. This does not run Rust/SQLite/NAS.
- editor-repro.mjs reproduces three current React UI defects through a controlled IPC adapter: only the first of two edits is sent while the second is displayed as saved; failed flush still invokes export; two windows generate identical request IDs for different commands.
- Machine-readable evidence and screenshots: artifacts/audit-2026-09-07/editor-repro.json and adjacent PNGs. Root inspected the slow-save screenshot.
- Queue claim_next binds ?1 to a timestamp both for updated_at and WHERE id. Normal job IDs therefore update zero rows and are never claimed (queue.rs:113-124).
- New import staging never adds a SourceFile to job.source_files. make_job initializes it empty; auto_pipeline requires it to locate the uploaded file (processing/commands.rs:169-191, job_store.rs:19, auto_pipeline.rs:1440-1462).
- Startup recovery only handles stage='running'; actual running work uses local_recognition/cloud_recognition, which remain stranded after restart (queue.rs:230-249, scheduler.rs:231 and cloud advance).
- Stored Tauri reports contain no fully successful import/edit/publish result. The latest retained report failed title persistence and then publication; earlier successful editing reports record publication blocked. These are historical evidence, not current reruns.
- NAS repository exists, but node_modules is absent. Actual NAS student workflow was not executed in this audit.
- main and the locally stored origin/main both point to 47a3806. No remote fetch was performed.
