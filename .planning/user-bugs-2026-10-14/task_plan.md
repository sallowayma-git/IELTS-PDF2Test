# User bug repair 2026-10-14

Goal: diagnose and repair cloud usage stops, lost/truncated document content, heading placement, answer-page no-write behavior, and investigate crashes through real Tauri paths.

## Phases
- complete: establish code/build baseline and inspect independent bug paths.
- complete: implement fixes justified by concrete evidence, including later A-K and paragraph-option review blockers.
- complete: final release build, source fingerprint verification, package audit and native release verification; targeted regressions passed.
- complete: real Tauri/DeepSeek investigation and heading workflow verification; crash not reproduced, evidence gaps recorded.
- in_progress: follow-up reports reproduced TS preview/compiler mismatch; unify paragraph-map options in preview/editor, classify legacy transport failures, rebuild and verify real Pearls student interaction.

## Constraints
Preserve existing tauri.windows.conf.json icon changes. Scouts only explore; root edits and verifies. Do not expose keys. No paid cloud calls without assessing existing authorization/configuration.

## Errors
- Initial historical planning read produced excessive output; use latest foundational QA documents and narrow history sections.
- Two scouts were pending_init; interrupted, root completed exploration.
- First gateway rejected-request regression used HTTP400 with PDF fixture, which correctly triggered image fallback; changed test to HTTP401 to isolate accounting.
- Patch tool rejects multiple operations targeting same file; combined hunks into one file operation.
