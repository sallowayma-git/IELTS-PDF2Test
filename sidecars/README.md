# Epic 8 Sidecars

These sidecars are local-app implementation hooks for the Tauri authoring app.
They are not a standalone Web authoring service.

- `python-parser/parser.py`: deterministic TXT/MD/PDF/DOCX parser command contract. PDF uses `pypdf` when available; DOCX uses OOXML zip/XML parsing from the Python standard library; OCR remains a flagged manual-review fallback.
- `node-validator/validate-reading-source.mjs`: local ReadingExamSourceV1 + DOM protocol validator entrypoint.
- `llm-gateway/gateway.mjs`: JSON-only LLM gateway for group classification/extraction. It calls OpenAI-compatible chat completions when a profile has an API key and otherwise emits deterministic local suggestions for offline review.
- `preview-e2e/preview-e2e.mjs`: local unified-runtime contract simulator. It executes generated `manifest.js` and exam wrapper JS, verifies registry registration, simulates answer collection from the DOM protocol, checks correct-answer score is 100%, checks wrong-answer score decreases, and reports RuntimePreview diagnostics.
- `preview-server/`: reserved for unified reading runtime preview integration.

Rust command integration:
- `parse_document` first tries `python3 sidecars/python-parser/parser.py parse ...` for TXT/MD/PDF/DOCX files, then falls back to deterministic local parsing or a review-required sample IR with a parser warning.
- `validate_authoring_ir` first runs built-in Authoring IR checks, then tries `node sidecars/node-validator/validate-reading-source.mjs` against generated `ReadingExamSourceV1` and merges the ReadingExamSourceV1/DOM layer results.
- `run_preview_e2e`, `export_reading_assets`, and `build_pack` all run the RuntimePreview gate through `preview-e2e/preview-e2e.mjs` before allowing export or publication.
- `llm_classify_group` and `llm_extract_group` call `node sidecars/llm-gateway/gateway.mjs` with redaction-friendly profile/group context and save `llm-last-suggestion.json` plus `llm-calls.jsonl` for audit.
- Tauri bundle config includes `../sidecars` as resources so packaged builds can resolve these scripts from app resources.
