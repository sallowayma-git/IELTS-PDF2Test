# PDF multi-passage split investigation

Goal: diagnose development/customer differences in the real PDF import workflow and repair detection and user notification.

1. [complete] Trace detector, packaged dependencies and import UI; reproduce concrete failure modes.
2. [complete] Repair supported failures with multiple evidence channels and actionable warnings.
3. [complete] Verify Rust detector/command behavior and actual import UI where available; record remaining gaps.

Final evidence: pre-change release UI reproduces one-row/no-preview failure for the same synthetic PDF; final embedded application passes five real Tauri UI scenarios at artifacts/e2e-cdp/run-passage-split-2026-11-06T09-39-12-593Z/report.json. Rust 63 passed; frontend 38 passed; tsc and attributable build passed. Four rendered subset pages pixel-identical. No new installer built and no automatic cloud passage planner implemented. Customer-specific cause remains unproven without original PDF/version/log.

Errors/adjustments: initial Rust test command waited briefly for build lock, then completed. reportlab is not installed in host Python; developer E2E fixtures use an in-script minimal PDF writer instead (no new runtime dependency). No original customer PDF is available, so the specific customer cause cannot be proven from screenshots.

Constraints: main agent owns implementation and final verification; subagents explore only. Preserve existing work. Images show READING PASSAGE 1/2 and Questions 1-13 / 14-26; original customer PDF/logs unavailable.
