# Phase 1 contract bundle

This directory is the canonical contract source for Phase 1. The schema bundle is deliberately independent of the V1 parser and is not loaded by any production command or feature flag.

The stable schema identifiers are:

- `DocumentIRV2` — immutable physical extraction snapshot.
- `ContentDocV2` — editable rich-content AST.
- `IeltsAuthoringIRV2` — IELTS task, response-group, answer-slot, and answer-key semantics.
- `QualityReportV2` — readiness, hard failures, source coverage, and review issues.
- `ReadingExamSourceV2` — the student-runtime reading source compiled from an immutable V2 authoring revision.
- `ListeningExamSourceV1` — the first structured Listening runtime source, including explicit scope, audio probe metadata, Parts, cues, playback policy, tasks, and slots.
- `ListeningAttemptV1` — a revision-bound Listening attempt with serializable playback state; controller policy is never inferred from the DOM.
- `ListeningAudioProbeResultV1` — a persisted local decode/hash/duration/signal-quality result; `passed` is only valid with complete decoded media facts and zero issue codes.

`contract-manifest.json` pins the bundle version (`2026.08.0`) and the exact SHA-256 of every JSON Schema file. The schema files use relative `$ref` paths so the same bytes can be copied into the NAS repository later.

## Compatibility boundary

- V1 artifacts remain readable and immutable. PR-01 does not rewrite V1 JSON or route production reads through V2.
- V1 → V2 is a best-effort migration boundary only. The skeleton reports `needs_review`, preserves the V1 artifact, and never marks inferred text, answers, options, slots, or provenance as verified.
- V2 → V1 is blocked until a lossless compatibility compiler exists. Unsupported rich layout or shared-response semantics must not be silently flattened into V1 HTML.
- `schemaVersion` is an exact dispatch key. Unknown versions are rejected by the migration boundary and must be opened read-only or reported as blocked.
- The Phase 1 artifact store uses `jobs/<jobId>/sources`, `extraction`, `authoring/revisions`, `authoring/patches`, `assets`, `preview`, `export-history`, and `legacy`. Canonical V2 JSON is written through a same-directory temp file, flush/sync, and replace sequence; revision commits use an optimistic base revision and keep immutable prior artifacts.
- Existing V1 paths (`job.json`, `uploads`, `document-ir.json`, `authoring-ir.json`, and the current V1 export paths) remain readable and are not rewritten by the V2 store.
- V2 runtime routing, V2 semantic parsing, and feature-flag enablement remain outside Phase 1.

## Phase 6 runtime contract

`ReadingExamSourceV2` is validated in the same contract bundle and references the existing typed task, answer-slot, content-node, and asset definitions. Cross-field rules that JSON Schema cannot express—slot assignment, question order, response-group closure, and source/asset exam identity—are enforced by the runtime compiler/probe. The source is deliberately separate from the legacy `ReadingExamSourceV1`; V1 payloads remain opaque to the V2 interaction model.

## Phase 7 Listening contract

The former open-ended `listening.sections` placeholder is replaced by typed media, Part, cue, playback-policy, transcript, and explicit scope fields. `complete_exam` enforces four Parts and forty scoring slots in the semantic gate; `partial_practice` validates only its declared Part/task/slot closure. Cues are optional, but any published cue must be confirmed, monotonic, non-overlapping, and inside the probed duration. `ListeningExamSourceV1` requires a passed audio probe and a content-addressed audio asset; `ListeningAttemptV1` binds recovery to the exact exam, authoring revision, media asset, and playback policy.

Playback policy explicitly carries pause, seek, replay/max-play, refresh, and crash-recovery behavior. Mock mode is fail-closed to one play with no pause/seek/replay and `resume_from_snapshot` recovery. The serialized controller rejects reset snapshots, preserves consumed play count across refresh/crash, and uses `restart_pending` so an allowed recovery restart consumes a new play before audio resumes.

The audio probe is a pure-Rust Symphonia 0.6 path with WAV/PCM, MP3, and AAC-in-ISO-MP4 decoder features compiled in. It streams SHA-256 computation and fully decodes the audio to derive duration, channel/rate, RMS, peak, and clipped-sample ratio. Open/decode failure, unsupported MIME, hash mismatch, near-silence, and severe clipping produce stable blocking issue codes.

## PR-02 PDF facts shadow extraction

PR-02 records physical PDF facts in a development-only shadow artifact without changing the V1 parser result or routing any production read through V2:

- `document-ir-v2.shadow.json` is written under the job directory only when the debug build environment variable `EPIC8_DOCUMENT_IR_V2_SHADOW=true` is explicitly set.
- The artifact is validated as `DocumentIRV2`, uses 0-based `pageIndex`, top-left point coordinates for glyph boxes/quads, page-local character ranges, and source-file hashes for traceability.
- Each page records native glyph text, bounding box, quad, origin, baseline, angle, font size, Unicode-map status, quality facts, and OCR candidate regions for pages without native glyphs. Semantic spans, lines, regions, tables, and reading order remain empty until PR-03.
- If a classic PDF has recoverable xref or direct stream-length defects, the shadow loader repairs a memory copy only and records a parser warning. The uploaded PDF and V1 artifact are never rewritten.
- `document-ir-v2.shadow.error.json` records a non-fatal shadow failure; V1 parsing continues unchanged.
- `debug_document_ir_v2_overlay(jobId)` writes `debug/document-ir-v2.shadow.overlay.svg` for physical-fact inspection. It requires an existing shadow artifact and is not a production workflow.

The corresponding TypeScript defaults are in `src/config/featureFlags.ts`. All Phase 0 and Phase 1 flags remain `false` by default; the Rust guard also disables the shadow path in release builds.

## Verification

From the PDF2TEST repository:

```powershell
npm run verify:phase1:schema
cargo test --manifest-path src-tauri/Cargo.toml schema
cargo test --manifest-path src-tauri/Cargo.toml artifact_store
cargo test --manifest-path src-tauri/Cargo.toml migration_v1
cargo test --manifest-path src-tauri/Cargo.toml pdf_facts_shadow
```

When the peer contract mirror is intentionally present in another checkout, run the same checker with its contract root:

```powershell
node scripts/verify-schema-contract.mjs --peer-root "E:\NAS\developer\contracts\authoring"
```

The peer comparison is opt-in in PR-01 because this change does not modify the NAS repository. If a peer root is supplied, every schema must exist there and match byte-for-byte.
