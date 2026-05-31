# Epic 8 Sidecars

These sidecars are local-app implementation hooks for the Tauri authoring app.
They are not a standalone Web authoring service.

- `python-parser/parser.py`: deterministic TXT/MD/PDF/DOCX parser command contract. PDF uses `pypdf` when available; DOCX uses OOXML zip/XML parsing from the Python standard library. It also exposes `extract_pdf_images` so image-only PDFs can be handed to a vision LLM without bundling a heavyweight local OCR engine.
- `node-validator/validate-reading-source.mjs`: local ReadingExamSourceV1 + DOM protocol validator entrypoint.
- `llm-gateway/gateway.mjs`: JSON-only LLM gateway for group classification/extraction and PDF-image transcription. It calls OpenAI-compatible chat completions; group fallback suggestions stay low confidence, and failed vision transcription returns an empty result so production code never synthesizes fake source content.
- `preview-e2e/preview-e2e.mjs`: local unified-runtime contract simulator. It executes generated `manifest.js` and exam wrapper JS, verifies registry registration, simulates answer collection from the DOM protocol, checks correct-answer score is 100%, checks wrong-answer score decreases, and reports RuntimePreview diagnostics.
- `preview-server/`: reserved for unified reading runtime preview integration.

Rust command integration:
- `parse_document` first tries `python3 sidecars/python-parser/parser.py parse ...` for TXT/MD/PDF/DOCX files. TXT/MD can fall back to deterministic local text parsing; PDF/DOCX parser failures or missing sources create low-confidence failure IR with parser warnings and require source review. Production commands must not synthesize sample reading content for real jobs.
- `run_auto_pipeline` detects no-text / low-confidence PDF output and, when an enabled LLM profile exists, tries `extract_pdf_images` plus `llm-gateway transcribe_pdf_images`. Successful vision transcription becomes `DocumentIRV1` with `parser.provider=vision-llm-transcription`; unresolved `SourceReviewV1` still blocks publish until the operator verifies the source text. Failed vision transcription leaves the parser warning path intact for manual transcription.
- `validate_authoring_ir` first runs built-in Authoring IR checks, then tries `node sidecars/node-validator/validate-reading-source.mjs` against generated `ReadingExamSourceV1` and merges the ReadingExamSourceV1/DOM layer results.
- `run_preview_e2e`, `export_reading_assets`, and `build_pack` all run the RuntimePreview gate through `preview-e2e/preview-e2e.mjs` before allowing export or publication.
- `llm_classify_group` and `llm_extract_group` call `node sidecars/llm-gateway/gateway.mjs` with redaction-friendly profile/group context and save `llm-last-suggestion.json` plus `llm-calls.jsonl` for audit. High-confidence suggestions are still blocked from auto-apply unless Rust verifies allowed patch paths, valid question/interaction schema, and evidence quotes tied to the current group `sourceBlockIds`.
- Tauri bundle config includes `../sidecars` as resources so packaged builds can resolve these scripts from app resources.
