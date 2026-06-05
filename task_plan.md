# PDF2Test Import Automation + Silent LLM Repair Plan

## Goal
Make PDF/DOCX import usable as a one-click/batch flow that lands on editable drafts, silently extracts scanned answer images with a vision model, runs a cloud full-paper comparison in the background, warns only on user-actionable uncertainty, validates against real fixtures, and produces a fresh DMG.

- [complete] Validate current changed code and test baseline.
- [complete] Fix remaining continuous completion, routing, parsing, or regression issues found by tests.
- [complete] Run real PDF regression sampling and record mismatches.
- [complete] Build a fresh DMG and report exact artifact path.
- [in_progress] Diagnose why auto-generation still lands on the import/recognition page.
- [pending] Add silent vision answer extraction JSON path for PDF image answer pages.
- [pending] Add background cloud whole-paper generation/comparison, with local output authoritative.
- [pending] Add or update tests for routing, LLM answer extraction, cloud comparison warning, and random PDF sampling.
- [complete] Run the full business chain with the provided OpenAI-compatible test profile.
- [pending] Build a fresh DMG after the new fixes.

## Decisions
- Production generation must depend on the uploaded source file, not legacy reading-exams JS.
- Legacy reading-exams JS is only a regression oracle with normalized fields.
- Existing editable drafts must not be overwritten unless explicitly requested.
- Normal users should see user-level text only; OCR/LLM/IR/runtime/rule split wording belongs only in advanced diagnostics.
- Local generated draft is authoritative; cloud model output is a background quality check and never overwrites the draft by default.
