# Repository Agent Instructions

## Product Reality Comes First

This repository's product is the Tauri authoring application and its real document-import, review, preview, export, and student-runtime integration workflows. The command-line interface under development tooling is not the product.

When auditing, debugging, implementing, or validating changes:

1. Prioritize the real Tauri UI and command handlers, persisted job artifacts, PDF/DOCX ingestion, authoring editor, quality gates, NAS package export, and the actual student renderer/runtime.
2. Treat CLI commands and scripts only as development helpers for fixtures, diagnostics, contract checks, and targeted regression tests.
3. Never use a passing CLI or schema check as a substitute for validating the corresponding product workflow.
4. Do not spend the main implementation effort improving CLI-only behavior unless the user explicitly requests CLI work or the CLI defect prevents testing the product.
5. For user-facing defects, reproduce and verify through the closest available real product path. Add browser-driven or Tauri-level coverage when practical, and clearly state any remaining gap when only a lower-level test is available.
6. Golden-baseline drift is diagnostic evidence, not an automatic instruction to restore old behavior or rewrite baselines. Decide against current product requirements and end-to-end behavior first.

In reports and handoffs, distinguish clearly between:

- product behavior verified end to end;
- service or command-handler behavior verified below the UI;
- CLI-only or schema-only evidence.

