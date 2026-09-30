# P2 alignment handoff

Historical exploration checkpoint: the production patch was used to obtain the recorded red tests and nine-book baseline before e9a5031. Later physical-line metadata changed the final file; the reversal commands below describe that checkpoint, not a procedure for the final HEAD. Final validation is in closeout.md.

Only `src-tauri/src/reconcile/alignment.rs` changed; `NodeAlignment.min_similarity`, `has_invented`, group thresholds and cloud adoption interfaces are unchanged. No compile, Cargo, build or E2E was run by this agent.

## Root cause and retained evidence

In retained job `artifacts/e2e-cdp/run-cloud-repair-chain-2026-09-30T19-03-33-551Z/appdata/data/jobs/import-20260930190336-bc87c264/document-ir-v2.shadow.json`, page 0 line p001-l0015 has text `young r e s e a r c h e r ... s u p e r v i s o r s e c h o a m u c h m o r e w i d e s p r e a d c o l l a p s e o f`. Its bbox starts x=50.400002 and spans width=490.885998; inlineGapsPt contains 28.741998. The raw glyph stream retains `These may be gaps which the` (charRange around 296–323), `young researcher is advised by supervisors` (324–366), then `to fill, or established views which he or she` (367–412), followed by `is encouraged to challenge.`. Thus the unrelated right-column phrase entered reconstructed line text, not cloud text. Concatenating those merged lines by their minimum sourceOrder corrupts the source search stream; fixed-length fuzzy windows then legitimately produce low similarity against that corrupted stream.

## Implementation

Keep the normal line view for fuzzy matching, order, lengths and coverage. Add an exact-only native PDF evidence view ordered by retained glyph `sourceAnchor.charRange.start`, with chars mapped through `lines.spanIds -> spans.glyphIds` to existing physical source units. Only `pdf_native` evidence is allowed; missing character ranges or unassigned non-whitespace glyphs split the view, so no invented jump across absent evidence is allowed. Soft line-break hyphens use the existing normalization rule. A native exact match scores 1.0, maps start/end to existing physical order coordinates, and only covers supplying source units. No threshold changes. q34 wording untouched.

## Tests added

- `exact_researcher_sentence_survives_merged_pdf_columns`: exact full researcher sentence >=0.9 (also =1.0), not invented, source line IDs preserved.
- `native_exact_evidence_does_not_bridge_missing_source_characters`: missing glyph ranges cannot yield an exact hit.
- `native_exact_evidence_rejects_invented_sentence_and_preserves_order_gate`: unrelated invention remains invented; reverse source order remains rejected.
- `exact_researcher_sentence_aligns_against_real_pdf_glyphs`: extracts fixtures/parser/demanding-reading-passage-3.pdf, asserts >=0.9, no invented flag, includes p001-l0015.

The 9-book table test already exists, driven by fixtures/golden/private-pdf-task-presentation-stage2.json. That table excludes demanding-reading-passage-3, so its exact PDF regression is a separate corpus-gated test. No retained numeric table baseline was found in planning/artifact text logs; obtain it using the reversible production-only patch below rather than claim invented baseline numbers.

## Main-thread serial validation commands (PowerShell, integration worktree)

`p2-production.patch` contains production code only, preserving all new tests while reversed. It was generated against HEAD's original production prefix plus the current new test module. Apply/revert only while no other agent edits alignment.rs.

```powershell
# Before: old production + new regression tests, plus 9-book numeric baseline.
git apply -R .planning/integration-round/p2-production.patch
$env:EPIC8_REQUIRE_PRIVATE_CORPUS = "1"
cargo test --manifest-path src-tauri/Cargo.toml --lib exact_researcher_sentence_survives_merged_pdf_columns -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib exact_researcher_sentence_aligns_against_real_pdf_glyphs -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib nine_private_pdfs_ideal_candidate_aligns_against_real_text_layer -- --nocapture
# Restore implementation before any next validation, even if the red tests fail as expected.
git apply .planning/integration-round/p2-production.patch
cargo test --manifest-path src-tauri/Cargo.toml --lib reconcile::alignment::tests -- --nocapture
```

These are alignment/command-handler evidence below the UI. The main thread's real cloud-repair-chain Tauri run must additionally confirm the exact sentence no longer produces passage_sentence_unverified and that downstream review/repair routing is unchanged. No product E2E claim is made here.
