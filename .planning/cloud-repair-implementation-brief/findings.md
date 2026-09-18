# Findings

- Existing editor transaction has version checking and request id deduplication, but marks each command target as user-edited. Machine writes need explicit provenance.
- Need inspect full cloud transport: tool-call support must not be assumed from existing JSON-only model commands.
- Product source-of-truth and partial validation behavior must remain compatible with initial imperfect drafts.

- scheduler::run_job_inner runs local pipeline and cloud outline concurrently, then freezes canonical before publishing ready. No seed call exists between local completion and freeze (current freeze failure).
- migrate_single_item selects revision before shadow and has a repair_shadow_seed branch; cannot blindly invoke this whole legacy repair routine for all new imports. Isolate seed-if-empty behavior.
- run_recognition_cycle_core_with_channels currently compares, automatically applies answer candidates, and persists outcomes in one orchestration function. New repair needs compare-only extraction, not an extra competing writer.
- llm_gateway has JSON response commands only; no tool_calls/function_call implementation found. Implement explicit bounded tool protocol using existing JSON transport, or add native support deliberately; do not pretend it already exists.
- Existing A3/A4 document read. Its answer-only, no-overwrite, proposal-only restrictions are explicitly superseded by user's latest product direction. Old reported tests are historical, not current acceptance.
- validate_authoring only checks schemaVersion and deserializes IeltsAuthoringIRV2; it is NOT referential/semantic/runtime validation.
- apply_patch exposes 19 operations, including resolveIssue and provenance attributes. Cloud needs a server-side allowlist and cannot receive those control fields.
- Provenance marking happens in both patch handlers and repository; repository marker only handles nodeId. Existing preserveProvenance/restoreProvenanceStatus offer internal hooks, but origin must be trusted backend context, never model-controlled.
- undo_patch_for only supports setAnswer; cannot claim generic cloud structural undo already exists.
- replaceContent refuses lost answer_slot nodes; setResponseGroup only replaces existing group; task insertion and cross-field structural updates need an atomic bundle operation.
- RecognitionCandidateV1 is a flattened comparison view, not a renderable full authoring draft (no passage). Full cloud candidate must retain rich nodes separately or reuse the full authoring type under a candidate wrapper.

## Inspection errors
- PowerShell rg wildcard paths such as schema/authoring* do not expand reliably. Use rg --files and explicit paths; no product impact.

## Independent exploration and corrections
- Three Luna low agents inspected candidate mapping, write/undo, and UI/publish. Main agent checked schema, editor journal, UI event handler, publish snapshot call and mapping branch.
- Important correction to prior reports: scheduler::set_item_status_ready ALREADY invokes migrate_single_item (~1220); seeding is not exclusive to UI. Actual defect is freeze BEFORE this call. Move/extract initialization, not add a second workflow.
- Preserve distinction: accepting cloud candidate IDs in normalization is not evidence of a current exploit. It becomes a trust boundary to fix before granting edit access.
- Existing publish preflight already returns editVersion; do not add duplicate canonicalEditVersion/checkedAt fields. It currently omits runtime compile checks that actual export executes.
- quality::validate_recognition_blockers re-emits stored recognitionBlockers without checking repaired content. Repair needs targeted reassessment, not clearing all blockers or repeatedly regenerating stale ones.
- Repository marks only nodeId; rules::node_provenance searches only id, not slotId/taskId. Existing value/baseline guards reduce current risk; future machine editor must protect semantic footprints including answers, groups and descendants.
- UI subscribeProcessing deduplicates stateVersion and only reloads when pendingCount=0. Need deferred refresh tracking and dirty-buffer protection for cloud writes.
