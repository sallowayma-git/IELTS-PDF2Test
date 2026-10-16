# Progress
2026-10-16: Read prompt and AGENTS. Spawned read-only NAS consumer and UI/test scouts.
Initial search assumed workspace folder; rg located actual files next.

Implemented canonical CAS projection, reading setter/clear and lazy historical repair, editor patch/control, import detection. First check passed; vitest 552 passed. First added Rust command tests exposed incomplete synthetic passage fixture (missing sourceAnchors); corrected fixture. Full Rust and native build running; will rebuild after final code changes.

Full Rust exploratory run had three new incomplete-fixture failures (fixed sourceAnchors) and long private corpus scans; stopped that exact test process because Windows held its executable open, causing next test link LNK1104. Final module runs will be sequential against current code. First native build rejected input drift correctly; final frozen-input rebuild launched.

Current final checks: TypeScript passed; vitest 556 passed. Rust library71, authoring38, export30(+1 ignored), cloud_direct32(+2 ignored), scheduler21, modality4, local-import1 and direct-import1 passed. Real Tauri CDP chain previously passed all assertions including historical Pearls5→6; final rebuild/rerun underway after no-job notification fix.

Final build SHA2564fd846bfe190c5ae1baf1a4f6d786905b3607e61afb14e8f35c143c386a7d8ca, frozen input hashes valid. Final e2e report artifacts/e2e-cdp/part-assignment-sync-2026-10-16T21-31-38-793Z/report.json passed10/10 with 4 screenshots, no-job remote refresh and Pearls5→6 idempotence. QA report docs/qa/2026-10-16-part-assignment-sync.md written with complete file list, commands, evidence layers and limitations. No commits.

Followup implementation: best-effort alignment read wrapper/scan; PartBackfill origin and cloud lock; legacy automatic-label preservation during CAS; retain latest human category journal marker across 200-entry pruning; one accessible dropdown disabled during cloud review/loading/recovery. Added malformed-schema+healthy list/read tests, automatic-source idempotence/no-protection, long human history, unrelated-save/lock regressions. npm check passed; npm test558 passed. First Rust run188 passed/1 failed/3 ignored: malformed fixture carried old V1 compatibility quality so an independent existing migration rewrote quality on open. Fixture now uses current skipped compatibility status to isolate Part failure; full document equality assertion retained. Rerun underway.

Followup final verification: npm check exit0; npm test558/55files; exact requested Rust multi-filter command189 passed/0failed/3ignored. Fresh build SHA5baa51f7a8126517fc51d9fb7df87ce68bbd6412046addc55a9e10952dff61fe, no input drift. E2E artifacts/e2e-cdp/part-assignment-sync-2026-10-16T21-59-27-424Z/report.json passed12/12; includes editable and controlled-lock screenshots (verified visually). Report docs/qa/2026-10-16-part-sync-followups.md. No commits/live DB edits/golden/gate changes.
