# Findings: IELTS task type presentation contract — Stage 1

- Root `AGENTS.md` prioritizes real Tauri/product paths and requires reports to distinguish end-to-end, service, and CLI/schema evidence.
- The `types-round` worktree already exists at `F:\workspace\PDF2Test-types`, on branch `types-round`, with HEAD `4c8d1df` matching `main` at inspection time.
- The worktree already contained uncommitted Stage 1 work in Rust/TS schema and contract files before this session. Preserve and review it; do not treat it as authored here.
- The private-real directory contains nine PDFs: chili-peppers, conformity, fishbourne-roman-palace, listening-to-the-ocean, listening-vol7-t9, organisational-design, petri-dish, sleep-study, western-celebrity. This conflicts with the task's repeated count of eight.
- Matching private PDFs and `src-tauri/lib/pdfium-windows/pdfium.dll` are already present in the worktree.
- `npm ci` completed successfully. npm reported 39 audit findings (34 moderate, 5 high) and a pending esbuild install-script approval notice; no audit fix or script approval was performed.
- The current collaboration agent list contains only `/root`; no direct Codex-session SendMessage/thread tool is exposed in this session.
- At initial discovery, the uncommitted `TaskPresentationRule` was incomplete and its generated TypeScript JSON mirror was absent. Stage 1 now defines explicit response kind, assignment, interaction, host type/placement, answer-slot policy, option source/alphabet, reuse semantics, and response-group granularity for all `TaskTypeV2` values and the `summary_completion:word_bank` and `plan_map_label_completion:letters` variants. The TypeScript mirror is generated from the Rust table and checked for parity.
- At initial discovery, `cloud_candidate_adoption` had not propagated through the TypeScript runtime audit unions. It now appears in `AuthoringAuditV2.source`, the reading/listening source revision contracts, the corresponding TypeScript unions, and runtime builders.
- `evaluate_quality_inner` / `evaluate_group` is the Rust quality path. Stage 1 adds task-presentation, heading option-count, and paragraph-anchor identity gates there, with focused regression coverage.
- `real_pdf_acceptance.rs` already provides a per-fixture pipeline and metadata-backed per-group checks. Its Phase 4 eight-PDF spec is distinct from the actual private-real on-disk set; listening-vol7-t9 is an extra PDF with metadata but no v1 baseline/manifest record. The broad corpus manifest currently also refers to eight absent random fixtures.

## Stage 1 outcome (2026-09-29)

- Paragraph nodes now accept the optional `paragraphLabel`; `paragraphMap` is defined as label → passage paragraph node ID, with the target paragraph carrying the same label. `PassageParagraph` is available as an answer-slot host and is backward compatible because the new fields are optional. Rust round-trip tests cover legacy paragraph and authoring-slot shapes.
- The quality gate now checks task presentation shape, task-group/per-slot response grouping, option-source and alphabet closure, option reuse, heading paragraph-map target identity, and that heading choices cover all scoring dropzones. The new issue codes are `TASK_PRESENTATION_CONTRACT_MISMATCH`, `PASSAGE_PARAGRAPH_ANCHOR_INVALID`, and `HEADING_OPTIONS_INSUFFICIENT`.
- TypeScript source audit candidates now accept the cloud adoption revision kind across both runtime types/builders. The schema bundle also permits the already-emitted optional `recognitionWarnings` and `questionCoverage` fields so checked-in schemas match product output; all manifest SHA-256 values match.
- Test-first evidence: parity and type-contract checks were red before implementation; the new quality tests were red for their intended missing-gate assertions after fixture compiler probes were repaired. Choose TWO first exposed incorrect per-slot grouping and inline options; the representative authoring fixture established a task-group shared option bank, which is now the rule. A–D single-choice subset validation and ParagraphMap-derived matching-information options each had targeted red regressions before their implementation changes.
- Targeted final checks passed: Rust quality unit suite 63/63; Rust schema/parity/backward-compatibility suite 63/63; TypeScript `tsc --noEmit`; 2 Vitest files / 5 tests; local schema contract verification (all schemas compile, real shadow pairs discovered=8 / validated=8, errorCount=0). Schema verification used `--local-only`; the peer NAS repository was not checked or edited.
- The private PDF run uses the ignored `phase4_eight_real_pdfs_reach_physical_authoring_quality_truth` service harness: PDF parsing → V1 authoring → physical `DocumentIRV2` → V2 authoring shadow → `QualityReportV2`. It is not a Tauri UI end-to-end run. The run preserved the quality blockers and failed the existing acceptance allowlist (`QUALITY_BLOCKER_POLICY`) for all eight official manifest fixtures.

### Private-real blocked groups

There are 11 distinct blocked task groups and 12 new gate findings (11 presentation mismatches plus one paragraph-anchor finding). No `HEADING_OPTIONS_INSUFFICIENT` issue occurred in this corpus.

| PDF | Blocked group(s) | Gate finding |
|---|---|---|
| `chili-peppers` | `group-1` — `true_false_not_given` | Per-slot response shape and fixed truth labels disagree with the current group shape/option bank. |
| `conformity` | `group-1` — `yes_no_not_given` | Per-slot response shape and fixed truth labels disagree with the current group shape/option bank. |
| `fishbourne-roman-palace` | `group-1` — `true_false_not_given` | Per-slot response shape and fixed truth labels disagree with the current group shape/option bank. |
| `listening-to-the-ocean` | `group-1` — `true_false_not_given`; `group-2` — `matching_information` | Judgment group shape/options disagree; information group uses `select`/explicit options instead of radio rows with paragraph-map letters. |
| `organisational-design` | `group-5` — `matching_features` | Five slots use `select` rather than drag-and-drop. |
| `petri-dish` | `group-1` — `matching_information`; `group-2` — `matching_features` | Information group lacks the expected radio/paragraph-map-letter shape; feature group has six `select` slots rather than drag-and-drop. |
| `sleep-study` | `group-1` — `true_false_not_given` | Per-slot response shape and fixed truth labels disagree with the current group shape/option bank. |
| `western-celebrity` | `group-1` — `matching_headings`; `group-2` — `matching_features` | Heading slots use `select` and prompt hosts; q14–q20 also lack valid paragraph-map anchors. Feature group has three `select` slots rather than drag-and-drop. |

`listening-vol7-t9.pdf` is physically present as a ninth PDF but is not one of the authoritative eight: it has no official manifest/baseline entry, so it was not included in the corpus acceptance run. It must not be silently counted as covered by the eight-PDF result.

### Scope and handoff

- The test/evaluator evidence is service/harness and schema/unit level. No real Tauri import/review/preview/export UI workflow was exercised, and no student renderer/browser path was exercised.
- Changes remain uncommitted in `F:\workspace\PDF2Test-types` (`types-round`). The main worktree and student/NAS repository were not modified; no commit, push, or merge was made.
- The requested coordinator session is not addressable from the exposed session/thread tools in this run. The report is prepared here for transfer; it was not sent to a different session or agent.
