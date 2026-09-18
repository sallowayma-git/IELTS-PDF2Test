# Cloud repair implementation brief

Goal: Produce a code-grounded Chinese execution prompt with data flow, file/symbol changes, sequencing, and product acceptance. Do not implement product changes.

Baseline: 867600a; concurrent memory changes belong to others.

## Phases
1. Trace scheduler, seed, gateway, candidate and write paths — complete.
2. Define minimal repair loop and integration/migration boundaries — complete.
3. Deliver implementation brief and verify references — complete.

## Constraints
- User now authorizes autonomous cloud correction; old proposal-only product rules are superseded.
- Canonical draft remains source of editor/preview/export.
- Luna high is unavailable; user accepted low. Three Luna workers used fork_context=false (current interface equivalent of no history), returned read-only findings and were closed immediately. Main agent spot-checked critical claims.
- Keep these planning files isolated from shared root planning files.

## Errors
PowerShell rg wildcard paths returned invalid-path errors; switched to exact discovered paths. Output truncations addressed with targeted reads.
