# Cloud multi-document output and local fan-out

Goal: one-passage PDF stays one item; one PDF with multiple passages/tests produces multiple independently editable items determined by the cloud; many selected PDFs are queued as bounded independent batches.

1. [complete] Read current cloud-direct design and exact code; locate response, normalization, persistence, UI and truncation contracts.
2. [complete] Extend cloud prompt/response handling and per-document normalization; persist multiple documents with stable identity and retry behavior.
3. [complete] Route cloud-enabled PDF imports through cloud discovery; preserve offline/local path and existing user edits.
4. [complete] Validate single/multiple/repeated-numbering, failures/retries and actual Tauri UI fan-out; record limitations.

Final product evidence: artifacts/e2e-cdp/run-cloud-multi-2026-11-06T10-33-59-913Z/report.json passed. Three selected source PDFs became five independent canonical/library items; all opened in editor/student preview, child edits survived reopening, scoped rerecognition used only pages 3/4, malformed cloud output preserved all five rows and did not run local recognition, and all five published via the UI into student-loadable JS packages. Controlled model service + WebView2 diagnostic CDP port, no security relaxations; this is not a real-provider recognition-accuracy or standalone student Electron acceptance. Offline UI regression 5/5 passed. Final build SHA256 7a982159c7174c57ef80ea538ddb1b6326b6bf0f47de8479446749b6889c8132. No new installer produced.

Errors: first Rust run had 3 tests pinned to the old ONE-document prompt text; updated contract assertions to new batch contract, then 35 passed / 2 real-provider tests ignored. An overlapping gateway-test process held the Rust test executable during a relink (LNK1104); after process completion, relink/test succeeded. Gateway 68 passed / 1 ignored. Frontend 39 passed and tsc passed.

Existing work: previous local PDF split repair is uncommitted and must remain intact. User explicitly authorizes cloud multi-document architecture; no new confirmation needed. Subagents explore only; main agent implements and verifies.
