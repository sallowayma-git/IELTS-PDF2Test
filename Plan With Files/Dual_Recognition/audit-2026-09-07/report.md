# Dual Recognition / WYSIWYG Implementation Audit

Date: 2026-09-07. Scope: current `main` at `47a3806`, including the preexisting uncommitted Cargo, schema, lib.rs, and processing module changes. The locally stored `origin/main` also points to `47a3806`; no remote fetch was performed. Exactly one read-only explorer was used. Product source was not changed.

## Findings

### F01 [P1] The current Tauri application cannot compile

`cargo check --manifest-path src-tauri/Cargo.toml --locked` exits 1 with six errors:

- `scheduler.rs:220-231`: `app` and `job_id` are moved into a closure and then borrowed again (two E0382 errors).
- `scheduler.rs:433` and `scheduler.rs:460`: borrowed `job_id` escapes into a `'static` blocking closure (two E0521 errors).
- `processing/commands.rs:154`: `&Path` passed to a function accepting `&PathBuf` (E0308).
- `lib.rs:1495`: calls private `ProcessingSettings::defaults` (E0624).

Evidence: [scheduler.rs](/F:/workspace/PDF2Test/src-tauri/src/processing/scheduler.rs:220), [commands.rs](/F:/workspace/PDF2Test/src-tauri/src/processing/commands.rs:154), [lib.rs](/F:/workspace/PDF2Test/src-tauri/src/lib.rs:1495).

This blocks building or running the current Tauri implementation. Existing executable or historical test results cannot establish current-worktree acceptance.

### F02 [P1] Migration ignores the user's current saved revision

`candidate_authoring` reads `authoring/current-revision.json` as an embedded or standalone authoring document. The actual `CurrentRevisionV2` object is a pointer containing `revision`, with the document stored separately in `authoring/revisions/{revision}.json`. The shape check rejects the pointer and falls back to the initial shadow. Legacy edits append revisions, so the first V2 migration can silently seed content from before those edits. Subsequent migration returns early once canonical content exists.

Evidence: [migration.rs](/F:/workspace/PDF2Test/src-tauri/src/library/migration.rs:29), [revision pointer](/F:/workspace/PDF2Test/src-tauri/src/artifact_store.rs:109), [revision path](/F:/workspace/PDF2Test/src-tauri/src/artifact_store.rs:77), [legacy save](/F:/workspace/PDF2Test/src-tauri/src/authoring_v2_commands.rs:157).

Resolve and validate the referenced revision before considering shadow fallback. Verification: static cross-module trace; not run through the currently unbuildable application.

### F03 [P1] Different windows generate identical edit request IDs

Each editor instance generates `edit-{itemId}-{baseVersion}-{localSequence}`. Two windows opened on version N both start their sequence at 1. After window A saves, window B's different edit has the same ID. The repository checks that ID before checking the version or comparing the payload, and returns success with `replayed: true`. The second window can show saved even though its edit was never applied.

Evidence: [request construction](/F:/workspace/PDF2Test/src/features/editor/useCanonicalEditor.ts:147), [repository replay](/F:/workspace/PDF2Test/src-tauri/src/library/repository.rs:254).

Browser reproduction observed two different command batches with the identical ID `edit-audit-item-1-1`. Backend replay behavior was verified statically. Use globally unique request IDs, preserve the same ID for an actual retry, and reject payload-mismatched reuse.

### F04 [P1] Post-transaction quality refresh can overwrite a newer save

After committing the editor transaction, the command reads canonical DS, refreshes quality outside that transaction, then executes a second update replacing the entire `canonical_ds_json` using only `WHERE id = ?1`. If a newer edit commits between the read and this write, the older refresh restores older content without reverting the newer version number or journal record. The original transaction's version check does not protect this write.

Evidence: [refresh sequence](/F:/workspace/PDF2Test/src-tauri/src/library/commands.rs:67), [unconditional DS replacement](/F:/workspace/PDF2Test/src-tauri/src/library/commands.rs:116).

Keep derived updates in the versioned transaction or condition them on the version that was read, with explicit retry behavior. Verification: static concurrency trace.

### F05 [P1] Edits queued during a slow save stall while the UI says saved

When `persist()` finds an in-flight request, it waits for that request and returns without draining the pending queue. A second edit whose debounce expires during the first save therefore remains queued; no timer is rearmed. The first save then sets the shared status to saved.

Evidence: [early return](/F:/workspace/PDF2Test/src/features/editor/useCanonicalEditor.ts:129), [saved state](/F:/workspace/PDF2Test/src/features/editor/useCanonicalEditor.ts:162).

Reproduced in the current React workspace: `AUDIT_SECOND_SAVE` was visible with the saved label, but the only transmitted text was `AUDIT_FIRST_SAVE`. Drain edits arriving during the request before reporting the draft saved.

### F06 [P1] Publication continues when flushing edits fails

`persist()` catches a failed save, restores the queue, and resolves normally. Workspace `publish()` awaits this promise and then proceeds to publish the last persisted DS. The user can publish old content while the latest visible edits remain unsaved.

Evidence: [swallowed save failure](/F:/workspace/PDF2Test/src/features/editor/useCanonicalEditor.ts:164), [publish after flush](/F:/workspace/PDF2Test/src/features/editor/ExamWorkspacePage.tsx:60).

Reproduced with a controlled IPC save failure: two failed save attempts were followed by one export invocation while the stored version remained 1. The adapter stopped before filesystem access. Flush must communicate failure and wait for all pending work; publication must stop on an unsuccessful flush.

### F07 [P1] The new queue cannot claim normal job IDs

The claim update binds `?1` to `now` and uses it both for `updated_at` and `WHERE id = ?1`. The selected `job_id` is never bound to the update. Normal generated IDs do not equal an RFC3339 timestamp, so zero rows update and the function returns `None`.

Evidence: [queue.rs](/F:/workspace/PDF2Test/src-tauri/src/processing/queue.rs:113).

This remains a blocker even after compilation is fixed. Bind the selected job ID separately. Verification: static SQL/parameter inspection; existing Rust queue tests could not run because of F01.

### F08 [P1] New import staging does not register the source file on the job

`import_files_core` saves a newly created job and copies the input into uploads, but `stage_source_file` never appends a `SourceFile` to the job or saves that metadata. `make_job` initializes `source_files` empty. The scheduler's legacy recognizer resolves the input exclusively through that collection, and takes the `no MainQuestion source file` path despite the copied file existing.

Evidence: [staging function](/F:/workspace/PDF2Test/src-tauri/src/processing/commands.rs:169), [empty source list](/F:/workspace/PDF2Test/src-tauri/src/job_store.rs:19), [recognizer source lookup](/F:/workspace/PDF2Test/src-tauri/src/auto_pipeline.rs:1440).

Register the staged source and recoverable task state consistently before enqueueing. Verification: static trace; the new command is currently unbuildable and not used by the main import UI.

### F09 [P1] Restart recovery skips actual recognition stages

Recovery and expired-lease claiming only handle `stage = 'running'`. The worker immediately advances this field to `local_recognition` and later `cloud_recognition`. A process exit during the substantial work therefore leaves a row neither startup recovery nor normal claiming will pick up.

Evidence: [recovery predicates](/F:/workspace/PDF2Test/src-tauri/src/processing/queue.rs:230), [claim predicates](/F:/workspace/PDF2Test/src-tauri/src/processing/queue.rs:101), [worker stage](/F:/workspace/PDF2Test/src-tauri/src/processing/scheduler.rs:231).

Recover every nonterminal execution stage and use an execution generation independent of event sequence. Verification: static state-transition trace.

### F10 [P1] Batch publication commits partial batches

`publishItems` publishes each item immediately, then accumulates successes and failures. A later failure leaves earlier items visible on NAS, violating the explicit all-or-nothing default. Per-item staging and recovery do exist, but no shared batch preflight/staging/commit wraps these calls.

Evidence: [publish loop](/F:/workspace/PDF2Test/src/api/publishClient.ts:142).

Implement one backend batch transaction with pinned versions and a shared staged manifest; make publishing only passing items an explicit second action. Verification: production call trace, no live NAS mutation.

### F11 [P2] Structural repair is not connected to the main workspace

`EditorCommandV1` contains only text, answer, and slot-placement variants. Workspace connects text and answer callbacks. Option add/delete/reorder, structural table repair, resource replacement, crop editing, and hotspot repair have not been connected to that workspace. The original-file drawer displays extracted text and redirects deeper repairs to retired pages.

Evidence: [command variants](/F:/workspace/PDF2Test/src/exam-canvas/editorCommands.ts:15), [workspace bindings](/F:/workspace/PDF2Test/src/features/editor/ExamWorkspacePage.tsx:192), [source drawer](/F:/workspace/PDF2Test/src/features/editor/ExamWorkspacePage.tsx:226).

Existing text inside options/table cells can be edited through text nodes; this does not supply the missing structural operations. M3's repair workflow is incomplete.

### F12 [P2] Canonical publication is still bound to a shadow file

The canonical export accepts DS from the caller, sets revision zero, and records `editVersion` only for audit. NAS validation resolves revision zero to the mutable shadow and compares content against it. A stale/missing shadow can block valid canonical publication; an edit during publication can invalidate the binding. There is no durable database snapshot/version binding for the batch.

Evidence: [export version semantics](/F:/workspace/PDF2Test/src-tauri/src/authoring_v2_commands.rs:592), [shadow binding](/F:/workspace/PDF2Test/src-tauri/src/nas_package_v2.rs:154).

### F13 [P2] The production bundle still ships the test backend and retired UI

The production build emits `devFallbackBackend-BVm-q9ld.js`, 97.43 kB. The runtime check prevents automatic use inside Tauri, but dynamic import still includes the adapter in bundled assets. `App.tsx` also imports and renders `LegacyRoutes`.

Evidence: [adapter import](/F:/workspace/PDF2Test/src/api/tauriCommands.ts:70), [legacy assembly](/F:/workspace/PDF2Test/src/app/App.tsx:6), observed `npm run build` output.

## Milestone Assessment

| Milestone | Current evidence | Assessment |
|---|---|---|
| M0 | Baseline tracking, isolated Tauri harness, browser probes, NAS contract entry exist. Retained reports do not show a successful complete import/edit/publish run. | Foundation present; release acceptance not established. |
| M1 | Canonical repository, versioned schema, editor transactions, title editing, typed preflight exist. F02-F06 compromise migration/save guarantees. | Partial; exit conditions unmet. |
| M2 | Uncommitted backend queue exists but fails compilation and contains F07-F09. Import UI still owns local/cloud orchestration; library refresh polls every two seconds. | In progress, not integrated. |
| M3 | Text/title/answer editing, undo/redo, and a two-pane canvas exist. Structural repair and actual NAS parity remain incomplete. | Partial. |
| M4 | New import unconditionally calls V1 split and authoring; V2 grammar iterates V1 questionGroupCandidates. | Direct V2 main path not delivered. |
| M5 | Cloud gateway emits comparison-only CloudReadingOutlineV1 plus answer diagnostics. No full candidate/skill/repair/salvage/three-way merge path was found. | Core replacement not delivered. |
| M6 | Single-item staging/publish exists. Batch atomicity, pinned database versions, and reference-aware cleanup covering recovery/running/staging are absent. | Partial. |
| M7 | Legacy routes, V1 main path, frontend scheduling, and dev fallback assets remain. Current Windows build cannot complete; actual student runtime and corpus/fault acceptance are unverified. | Incomplete. |

Production-path anchors: [frontend import](/F:/workspace/PDF2Test/src/features/import/useImportFiles.ts:124), [local-then-cloud pool](/F:/workspace/PDF2Test/src/features/import/useImportFiles.ts:148), [polling](/F:/workspace/PDF2Test/src/features/library/libraryStore.ts:105), [V1 import](/F:/workspace/PDF2Test/src-tauri/src/auto_pipeline.rs:1675), [V1-dependent grammar](/F:/workspace/PDF2Test/src-tauri/src/ielts_grammar/mod.rs:134), [comparison-only cloud prompt](/F:/workspace/PDF2Test/src-tauri/src/llm_gateway.rs:484).

## Verification And Limits

| Check | Result | Coverage |
|---|---|---|
| `npm run check` | Passed | TypeScript compilation. |
| `npm run build` | Passed; test backend chunk still emitted | Frontend production bundle. |
| `npm run e2e:library-workspace` | 13 steps passed | Browser with dev fallback, not Tauri/SQLite/NAS. |
| `node 'Plan With Files/Dual_Recognition/audit-2026-09-07/editor-repro.mjs'` | Three defects reproduced | Current React workspace with controlled IPC responses. |
| `cargo check --manifest-path src-tauri/Cargo.toml --locked` | Failed: six errors | Current Rust application cannot compile. |
| Current Rust tests / Tauri E2E | Not run after compilation failure | Blocked by F01; no current product acceptance claimed. |
| Actual NAS student runtime | Not run | Peer repository exists; dependencies absent. Contract checks are not runtime acceptance. |
| 100-PDF quality, batch restart, low disk, NAS interruption, performance matrix | Not executed | Unverified release gates. |

The newest retained Tauri report (`2026-09-05T11-34-16-269Z`) records seven passed and two failed steps, including title persistence; older editing-success reports record publication blocked. Implementation notes mention a later successful editing run, but a matching complete successful publication report was not found. This discrepancy is a traceability gap, not proof of a current title regression.

Reproduction artifacts: [JSON report](/F:/workspace/PDF2Test/artifacts/audit-2026-09-07/editor-repro.json), [slow-save screenshot](/F:/workspace/PDF2Test/artifacts/audit-2026-09-07/pending-edit-reported-saved.png), [failed-save publication screenshot](/F:/workspace/PDF2Test/artifacts/audit-2026-09-07/publish-after-save-failure.png).

## Assessment

The current checkout has meaningful M0/M1 infrastructure and an incomplete M2 draft, plus earlier UI simplification. It is not a completed M0-M7 implementation. Restore buildability and resolve saved-edit/migration defects before relying on the remaining recognition, publishing, and acceptance work.
