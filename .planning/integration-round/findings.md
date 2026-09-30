# Findings & Decisions

## Requirements
- Follow the root AGENTS.md product-first requirement; do not substitute CLI or schema results for product e2e.
- Read `.planning/integration-round/handoff.md` as authority; its requested one-line update was committed first (`9bef6e5`).
- Implement e2e mechanism changes using natural derived-candidate feedback, preserving the original acceptance intent.
- Add a clean-candidate second phase: no review targets, zero repair calls, overall adoption, and no conflict todo.
- Run required Rust/frontend checks, app build, cloud-repair and heading e2e serially under process/resource rules.
- Build/copy/checksum NSIS only after all required checks are green. No merge, push, install, or cleanup.

## Research Findings
- The fixture-derived candidate naturally contains group-2 instruction/stem overlap, a low-similarity passage sentence, and unresolved q27/q28 answers.
- The architecture intentionally does not create repair packets for passage-level targets.
- Process inspection before starting showed no cargo, rustc, link, or IELTS Author Studio processes.
- Current branch is `integration-round`; handoff update commit is `9bef6e5`.

## Technical Decisions
| Decision | Rationale |
|----------|-----------|
| Inspect e2e scripts/fixture sources before editing | Maintain real product workflow coverage and use existing artifacts. |
| Use one e2e invocation at a time | Fits the 15.6 GB memory constraint. |
| Stop after the third still-failing e2e round | Explicit limit; report the blocking assertion rather than retrying further. |

## Issues Encountered
| Issue | Resolution |
|-------|------------|
| None so far | — |

## Resources
- `F:\workspace\PDF2Test\AGENTS.md`
- `F:\workspace\PDF2Test-integration\.planning\integration-round\handoff.md`
- `scripts/e2e/tauri-cdp-cloud-repair-chain.mjs`
- `scripts/e2e/lib/cloud-repair-scenario.mjs`

## Additional Findings (2026-09-30)
- `tauri-cdp-cloud-repair-chain.mjs` is 2,396 lines and was read in full before editing. It currently imports `textOfNodes` but not `collectTextNodes`; the shared helper exports `collectTextNodes(nodes, out)` returning mutable text-node references.
- The current derivation clones the local draft, repairs only the q40 prompt to the golden value, and records a `response_group.prompt` ruling. Its `answerClaim` deliberately searches the last page because the annotation marks answer-key absence; this is not yet the requested q27/q28 same-evidence-page packet escalation.
- The clean-candidate generator must use `collectTextNodes` to update nested passage text and should preserve the real candidate/group structure while clearing only the three specified triggers.
- Existing chain has an independent answer scenario marked not-executable because there is no golden answerErrors annotation; the requested q27/q28 packet behavior must be tested from actual review targets/call traces rather than treating this disabled scenario as coverage.
- Product candidate review is generated from source alignment: a group whose question-content nodes are not aligned adds `needsCloudReview {taskId, reason: content_not_aligned}`; instruction/prompt source-region overlap adds `instruction_stem_overlap` (cloud_adoption.rs around 590–625).
- Cloud repair edits allow `setResponseGroup`, `setOptionBank`, and `setAnswer`, but no direct `setTaskGroup` instruction edit. The repairable group-level mismatch should therefore be a response-group stem under that group, while group-2 overlap should be adjudicated with an evidence-backed `record_ruling`.
- The controlled service already has packet-mode helpers for prompt rewrite from page text, packet rulings, and an `answerClaim` read_source path; current answerClaim behavior is specifically “no answer key, leave unresolved.” The user-requested q27/q28 scenario may need a distinct evidence-page answer path rather than renaming this existing no-answer-key flow.
- Repository-local option-bank acceptance data records the fixture answers q27=C (“originality”) and q28=F (“interests and feelings”) (`src-tauri/src/ielts_grammar/option_bank.rs` around lines 418–436). These are better grounded as fixture truth than an ad hoc guess.
- The existing chain's packet trace already captures `packetPages`, per-call `pagesIncluded`, `escalationLevel`, and per-tool observation page IDs, so the new first-page-missing / same-page-fetch / L1 check can be evidence-based from persisted requests.
- The latest existing product run artifact (pre-change) confirms the natural group-2 `instruction_stem_overlap` and 0.55 passage target. It did not include a group-3 `content_not_aligned` review target; the candidate already carried q40's golden prompt. Its `answerClaim` was a synthetic q27 claim, not a direct unresolved-answer target.
- The same artifact's candidate had q27 and q28 values available, but the initial diagnostic projection accidentally serialized nested source anchors and was truncated. Follow-up inspection will use narrow JSON projections only.
- In packet mode, `plan.answerClaim.searchPages` controls explicit same-packet `read_source` fetches; a distinct `answerFetch: 'read_source'` path can apply `setAnswer` only when an explicit answer line exists. The passage-derived q27/q28 mechanism therefore needs a deterministic fixture-backed value/evidence pair and must only emit the edit after fetching and validating the evidence page.

## Existing pre-change e2e evidence
- Run `artifacts/e2e-cdp/run-cloud-repair-chain-2026-09-30T10-59-50-886Z/report.json` is an existing red run and is useful diagnostic evidence. Cloud adoption accepted the candidate, with group-2 `instruction_stem_overlap` and passage `similarity: 0.55`; it had no group-3 `content_not_aligned` target.
- That run failed the old q40 correction/evidence scenario (no request carried page 4 and no quotes), failed the adopted-cloud adjudication scenario (no adopted comparison mode/ruling, zero tool calls), and failed the answer packet scenario (no repair packet contained q27). Its repair ended `needs_attention`, 5 rounds, 0 edits, 1 adjudication.
- Its attempted clean-candidate phase is also red: review targets still contained group-2 overlap and the 0.55 passage sentence; repair had 5 rounds/0 edits; q27 and q28 were cloud-labeled `A` while local truth was unresolved, producing conflict todos, plus an unresolved q27 claim. The current checked-out chain script does not contain that clean-candidate scenario, so this run artifact is diagnostic evidence from an earlier attempt, not proof of current source behavior.
- Use this artifact for the report's first-red evidence and adapt from its specific failure points; do not repeat the oversized nested JSON projection that truncated output.
- Narrow inspection of that earlier candidate shows `group-2` is actually the q32–q35 YES/NO/NOT GIVEN group; `group-3` is q36–q40. q27/q28 live elsewhere, and both earlier candidate values were A due the synthetic claim + human-protection probe. For the new mechanism, preserve q27/q28 as unresolved in the main candidate and reserve fixture truths C/F for the clean candidate.
- The earlier candidate's q40 prompt was “The writer recommends that to be effective, social history must” and the original local draft added `BLANK PAGE`; the source-backed correct line is on PDF page 4. This makes its local error a suitable minimal group-3 stem trigger if it lowers group alignment enough.
- A PowerShell text-only projection now reads candidate node text without printing source anchors; the previous oversized JSON query's output truncation is resolved.
- The narrow artifact projection confirms `group-1` owns q27–q31, with a shared A–H bank; the derived candidate's q27 was set to A by the synthetic answer claim, while q28 remained unresolved. The intended main run should avoid setting either q27/q28 in the candidate so the product emits its natural `answer_unresolved` review targets; human-edit protection should use a different slot.
- The candidate passage text contains both q27 and q28 support phrases (“new insights … professional advancement” and “mobilises popular enthusiasm and engages popular passions”). The old run's `document-ir.json` page projection returned blank via `.lines`; inspect `.spans` instead to confirm page indexes and exact quote text before implementing evidence assertions.
- Existing clean-candidate artifact confirms the previous sanitizer did not change group-2 text, q27/q28 remained A, and it did not remove the low-similarity paragraph. This is a concrete failed attempt to avoid repeating.
- Prepass `document-ir.json` page objects contain `blocks` (top-level keys: `assets/jobId/pages/parser/schemaVersion`; page keys: `blocks/height/pageIndex/width`), while the chain's `sourcePageTextsFromJob()` currently reads only `lines` or `spans`. Its current evidence validator therefore cannot verify original-page quotes on this artifact. The implementation needs to read block text and normalize actual page indexing rather than assuming lines/spans are present.
- The candidate passage nodes have source anchors on page indexes 0 and 1; q27/q28 support text is on the first passage node page (index 0) in the generated candidate.
- Text projection from the prepass job's `document-ir.json` finds q27 support text (“new insights … professional advancement”) and q28 support (“popular enthusiasm … popular passions”) on PDF page 1; q27–31 questions are on page 3 and q36–40 on page 4. The `DocumentIR.pages[].pageIndex` values are 1-based (1–5) in this artifact, and each page has text under `blocks[].text`.
- Only the prepass item has `document-ir.json`; the second imported item has `job.json` only. Because the root fixture hash is checked against golden and the same PDF is staged twice, source quote validation must fall back to the prepass's imported DocumentIR when the second item's page text is absent.
### 2026-09-30 resumed audit

- group-2 overlap naturally reproduces because source instruction line `p003-l0040` and q32 line `p003-l0042` share parent region `p003-r0012`; `cloud_adoption` currently compares source node IDs that include regions. This remains the real-feedback model ruling trigger.
- The fixture has no authoritative q27/q28 answer values. Its golden annotation says there is no answer page; option-bank entries C/F are labels only. Asked user how the clean phase should represent these slots.
- PyMuPDF extraction of the source PDF confirms the candidate sentence “These may be gaps which the young researcher is advised by supervisors to fill” matches the original. The 0.55 alignment comes from DocumentIR line `p001-l0015`, which merges the phrase with unrelated later text “echo a much more widespread collapse of”. Candidate-only replacement cannot honestly clear this review without correcting source extraction/alignment.
- Current source-grounding helper checks only `document-ir.json` / `.shadow.compare.json`; the retained product artifact is `document-ir-v2.shadow.json` with `pages[].lines[]` and zero-based pageIndex. It needs correct artifact support and page normalization for the original-file citation assertion.

## Final verification findings (2026-09-30)
- The final product-path chain proved group-2 adjudication from the real `instruction_stem_overlap` request and page-3 PDF quote; it also proved q27's first packet omitted page 5, then fetched page 5 through L1. No answer was applied.
- Group-3 q40 has the only actual local question-text residue. The adoption report contains no group-3 `content_not_aligned` review item. Alignment checks all non-passage group text with a 0.8 threshold (`src-tauri/src/reconcile/alignment.rs:521,933,1013`); one q40 residue cannot honestly trigger a group review. Do not lower the threshold or fabricate multiple errors to satisfy this scenario.
- The clean phase faithfully reported the remaining fixture constraints: q27/q28 answer truth is absent; q29–q40 remain unresolved in the real local draft; source reading-order alignment still leaves the passage review; paraphrasing group-2 removes overlap but produces a real instruction conflict todo. These prevent the requested zero-review / zero-repair assertion without changing product gates or inventing truth.
- Final cloud-repair run: 16 pass / 3 fail (`content_not_aligned` trigger, ineligible fallback terminal state, clean-candidate zero-review gate). Heading-presentation remains `cloud_status=partial`. Stop at the third chain iteration as requested.

## 2026-09-30 resumed P0–P3 work

### Scope from the current user request
- P0 takes priority: investigate the retained cloud-repair-chain fallback run and heading-presentation partial run using their task artifacts, `llm-usage` / `llm-calls`, `cache/llm`, processing-job state timeline, and repair-packet queue. Every repair outcome must terminate persistently and release the edit lock; report import-to-terminal elapsed time.
- P1: a single instruction, stem, or option line with similarity below 0.6 must create a `content_not_aligned` review target carrying its slot/node, independent of the group's alignment ratio. Keep the existing group-level `<0.8` rule and slot-to-group packet routing.
- P2: determine why an exact sentence from the PDF scores 0.55, fix alignment and add a >=0.9 regression; rerun the 9-book table.
- P3: the user authorizes a test-only fixture with inferred answers q27–40 `C F D H A / NO YES NO NOT GIVEN / D B B C A`. The clean candidate also restores any swallowed group-2 stems to their source-backed question slots without deleting text.
- Release policy: no NSIS if P0 remains unresolved. After at most three E2E rounds, packaging is allowed with remaining fixture-only failures if P0 is fixed.

### Baseline evidence before this resumed work
- At `1547bdf`, prior product E2E run `artifacts/e2e-cdp/run-cloud-repair-chain-2026-09-30T19-03-33-551Z` passed 16 assertions and failed 3. The ineligible-candidate fallback did not reach a terminal state within 180 seconds; clean-candidate assertions also failed. This is first-red evidence for current investigation, not yet a root-cause diagnosis.
- Prior heading run `artifacts/e2e-cdp/run-heading-presentation-2026-09-30T19-13-22-564Z` ended `cloud_status=partial`, remaining nonterminal at the wait assertion.
- P1 baseline: q40's lone bad stem was masked by group-3's high aggregate alignment, so no per-slot target was emitted.
- P2 baseline: the candidate phrase exactly matched the PDF, while the corresponding extracted `DocumentIR` line also contained unrelated text; it received 0.55. The exact cause is still to be established from current source and retained artifacts.
- P3 baseline: clean phase had 15 review targets, 6 repair calls, and one conflict todo. The golden annotation has no answer key; current user-provided inferred values are explicitly test-only, not authoritative source truth.

### Investigation log
- Pending: inspect P0 task artifacts and repair lifecycle source; record timestamps, statuses, call counts, terminal transition, and edit-lock evidence before choosing a fix.
- Pending: inspect the complete alignment scoring path and reproduce the exact-sentence score before implementing P1/P2.
