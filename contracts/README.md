# Phase 1 contract bundle

This directory is the canonical contract source for Phase 1. The schema bundle is deliberately independent of the V1 parser and is not loaded by any production command or feature flag.

The stable schema identifiers are:

- `DocumentIRV2` — immutable physical extraction snapshot.
- `ContentDocV2` — editable rich-content AST.
- `IeltsAuthoringIRV2` — IELTS task, response-group, answer-slot, and answer-key semantics.
- `QualityReportV2` — readiness, hard failures, source coverage, and review issues.

`contract-manifest.json` pins the bundle version (`2026.08.0`) and the exact SHA-256 of every JSON Schema file. The schema files use relative `$ref` paths so the same bytes can be copied into the NAS repository later.

## Compatibility boundary

- V1 artifacts remain readable and immutable. PR-01 does not rewrite V1 JSON or route production reads through V2.
- V1 → V2 is a best-effort migration boundary only. The skeleton reports `needs_review`, preserves the V1 artifact, and never marks inferred text, answers, options, slots, or provenance as verified.
- V2 → V1 is blocked until a lossless compatibility compiler exists. Unsupported rich layout or shared-response semantics must not be silently flattened into V1 HTML.
- `schemaVersion` is an exact dispatch key. Unknown versions are rejected by the migration boundary and must be opened read-only or reported as blocked.
- The Phase 1 artifact store uses `jobs/<jobId>/sources`, `extraction`, `authoring/revisions`, `authoring/patches`, `assets`, `preview`, `export-history`, and `legacy`. Canonical V2 JSON is written through a same-directory temp file, flush/sync, and replace sequence; revision commits use an optimistic base revision and keep immutable prior artifacts.
- Existing V1 paths (`job.json`, `uploads`, `document-ir.json`, `authoring-ir.json`, and the current V1 export paths) remain readable and are not rewritten by the V2 store.
- V2 runtime routing, V2 semantic parsing, and feature-flag enablement remain outside Phase 1.

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
