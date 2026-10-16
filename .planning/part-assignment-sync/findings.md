# Findings
Prompt confirms library part currently only updates columns; canonical category drives preview/export. Preserve examId and title.
Existing root plan is historical and preserved. New tracking scoped here.

Found two additional author UI constants: workspace sub-header and reading bottom nav were fixed Part 1, now use category. Local canonical V2 builder independently rebuilt exam from job.category, so it also needed detection.
NAS audit: NasJsDirectReadingAssetProvider.ts:143/218 manifest category; :502 payload.meta.category; reading-suite-sessions.ts:100 category filter; no Part inference from ID.
User DB backed up via SQLite online backup (read-only source) to artifacts/part-sync-user-db-backup.db. Pearls target version 5/P3 manual vs P1 category reproduced read-only.

Independent review caught filename-as-content priority error; filename/originalName now excluded, regression added. Existing processing notifications skipped rows without durable jobs; added content clock fallback on same event, independent frontend dedup. Lazy repairs in workspace/list/row wrappers now notify on version advance. E2E deletes the Tea processing row to test this boundary.
UI screenshots exposed wide generic select; added compact header styling and kept title/Part adjacent.

Followups complete: read-only repair errors logged without aborting reads; eligible cached auto labels use PartBackfill origin/no human protection with CAS and cloud lock. Preserve latest human category journal marker beyond 200-row pruning to avoid false never-edited inference. Source schema-invalid read regression isolates Part by current skipped V1 compatibility marker, preserving existing quality migration behavior.
UI now one accessible Part select, disabled for cloud/loading/recovery. Final native screenshots visually verified for editable and controlled cloud-lock states.
