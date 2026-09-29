# Findings: task-presentation rollout

## Completed Stage 1 baseline

- The previous stage produced a Rust task-presentation table, TypeScript parity mirror, paragraph label/map semantics, `PassageParagraph`, cloud source audit typing, and strict quality-gate issue codes.
- Stage 1 validation: Rust quality suite 63/63; Rust schema/parity/backcompat suite 63/63; TypeScript typecheck; 5 Vitest tests; local schema validation errorCount=0 and 8 official shadow pairs validated.
- Existing Stage 1 private-PDF report is `F:\workspace\PDF2Test-types\tmp\phase4-real-pdf-acceptance\report.json`. All eight manifest-backed fixtures remain blocked, covering 11 task groups and 12 new gate findings. This is service/harness evidence, not UI E2E.
- Stage 1 source and corpus notes are in `.planning/tasktype-presentation-contract/findings.md`; preserve the findings and do not relax their contract to make fixtures pass.

## Stage 2–5 code surveys

### Stage 2 implementation and corpus evidence

- Consolidated instruction classification behind `instruction_signature.rs`; V1 authoring and QLG now use the shared task type while V1 serialization keeps a narrow legacy vocabulary adapter.
- Passage recognition identifies explicit standalone, `Paragraph A`, and conservative flattened-inline paragraph markers and emits labelled nodes plus a real `paragraphMap`; unlabelled source passages stay unmapped.
- Rule-table-driven response construction now emits fixed truth-value options for TFNG/YNNG, one response group per matching-information question, passage-paragraph heading anchors, summary inline drag/drop banks, and correct task grouping/reuse behavior. Direct-canonical recognition no longer treats arbitrary Roman lists or generic multiple-choice wording as evidence.
- Red evidence included expected failures for missing paragraph labels, matching-information grouping, and form classification before the corresponding fixes.
- Added manually checked expectations for all nine PDFs named in Stage 2 to `fixtures/golden/private-pdf-task-presentation-stage2.json`. The request's “8 PDFs” count conflicts with its nine-item list; all nine physical files were tested.
- The final corpus service-pipeline gate passed 1/1 across nine real PDFs. Detailed ignored runtime report: `F:\workspace\PDF2Test-types\tmp\stage2-private-pdf-task-presentation\report.json`. Focused Rust suites passed 7/7 and 24/24. This is service-level real-document evidence, not Tauri UI evidence.

### Stage 2: local recognition

- There are at least three active classifiers: V1 `detect_dynamic_group_kind`, V2 `infer_instruction_signature`, and QLG `classify_group`; their outputs disagree. Examples: “Complete the form below” becomes table / form / note respectively; “Label the map below” becomes diagram / plan-map / diagram-label; “Choose FOUR … A-F” becomes matching / matching-features / multiple-choice.
- V1 `build_passage` writes an empty paragraph map; direct canonical groups physical passage lines into arbitrary groups of four, also with an empty map. Neither emits paragraph labels. Direct canonical separately constructs a reduced signature, so it does not inherit the full V2 instruction inference.
- QLG has a heading bank special case, but its generic matching path requires option banks for matching-information even though the Stage 1 rule sources choices from `paragraphMap`. The normal missing-question route also loses declared question numbers before direct-canonical generation; its existing regression manually restores the number and misses that path.
- V1 `detect_dynamic_group_kind` returns a legacy `&str` vocabulary and its `classify_dynamic_group` independently derives interaction/options/reuse. `instruction_signature` has more precise `TaskTypeV2` cues and fallback hints. QLG independently classifies from zone text, block stems, and bank metadata; its multi-answer predicate treats FOUR/FIVE as multiple choice before checking matching/completion evidence.
- The V1 shadow's `build_passage` currently serializes `paragraphMap: {}`; `paragraph_node` does not accept a label. V1 passage nodes are derived from semantic lines, so labels must be recognized within passage content without regrouping the source into invented boundaries. Direct canonical needs a separate paragraph parser over its region lines; its current fixed-width four-line chunking is not defensible as paragraph semantics.
- Direct-canonical `interaction_for` currently maps the entire matching family to `select`, diagram/map to `hotspot`, and text-entry tasks with a bank to `select`; its group builder emits one per-slot response for every question and anchors all slots to prompt nodes. The serialized contract therefore needs to come from the Stage 1 rule table, with true-false fixed labels and task-level multiple-choice cardinality preserved.
- The current V2 signature module already owns the most complete cue parser and task-type enum mapping. A shared pure classifier there can replace V1 and QLG decision branches while their adapters keep only legacy serialization/context details; direct canonical should consume the rule table for interaction, host, response kind, assignment, and options instead of adding another mapping.
- V1 `GroupClassificationV1.kind` is used by many legacy helpers, so changing the string vocabulary has a broad compatibility surface. Before changing it, identify which aliases are only internal family keys and which are persisted task type values; preserve the existing V1 document contract while ensuring the actual V1/V2 task type no longer folds form, note, flowchart, or map tasks.
- Downstream V1 split construction uses the classification string to choose blank extension, group range, option-bank extraction, and layout hints. The shared classifier must own the semantic decision, while a narrow adapter may translate only where the V1 legacy projection requires a family key; exact task type must remain available to the V2 shadow rather than being lost in that adapter.
- Tests to extend: V1 classifier probes in `authoring_pipeline.rs`; signature tests in `instruction_signature.rs`; QLG classifier tests in `recognition/local/task_groups.rs`; paragraph split/map and missing-question tests in `recognition/direct_canonical.rs`.

### Stage 3: cloud chain

- Rust owns production cloud calls; `sidecars/llm-gateway/gateway.mjs` is explicitly a limited legacy sidecar without the candidate/repair commands.
- The Stage 1 Rust prompt-table serializer exists but has no production call site. The candidate contract and prompt contain a universal option-bank instruction and a misleading TFNG letter/bank example. They do not carry presentation invariants or source paragraph anchors.
- `reconcile/candidate.rs` has a legacy mapping that hard-codes headings/matching to radio/prompt and folds notes into summaries; modern full-authoring normalization preserves candidate structure, then quality blocks incompatible output. `setTaskType` changes only the task type string and signature, in both Rust and frontend patches; no direct command regression exists.
- The controlled cloud E2E returns candidate fixtures as-is and currently has no presentation assertion. Its prompt selection relies on exact opening-text markers, so changing them requires synchronized fixture-harness updates.

### Stage 5: student runtime

- The existing `IELTS-NASfor-WenDao-types` worktree is on `types-round` and was clean in the read-only survey. The base student repository and `IELTS-NASfor-WenDao-listening` were not touched.
- Passage/prompt `answer_slot` nodes render through `AnswerSlotNode`; the question pane independently renders each response slot. A slot embedded in source content can therefore be rendered twice. The loader validates structural identity but does not enforce a single render location.
- The question pane does not resolve task option-bank references in all render paths; `OptionBankNode` only renders inline `options`. Per-slot choices can also reuse a non-reusable label in the UI, although the server later rejects duplicate labels.
- Loader option and answer labels are uppercased; option scoring is case-insensitive but roman labels and normalized free-text comparison metadata need targeted coverage. Existing V2 Electron flow is available but depends on an author-generated package/fixture.

### Stage 4: editor canvas

- `ExamCanvas` renders embedded answer slots, but interactive rendering is text-only; non-text slots render badges. Completion slots embedded in task stimuli suppress the detached response list only when all are embedded.
- `MatchingMatrix` is gated on at least two single-slot matching response groups and an option bank, then suppresses the standard list. It has no matrix-specific component test and offers no option editing controls. The Stage 1 per-slot matching-information contract therefore makes it eligible once rules are enforced.
- Existing option reordering already uses a canvas-owned pointer session, stable IDs, window-level pointer events, and keyboard movement. It is a reference for drag/drop, but an answer-slot move action is absent. Editor mutations flow through `apply_editor_commands`; review locking removes callbacks and makes the canvas inert.
- Existing product verification scripts include `e2e:option-drag` and a real Tauri CDP harness. Build the desktop app only once, after implementation, then run final E2Es serially.

### Source PDF audit

Read-only page-rendering audit returned these source-based task expectations:

- `chili-peppers`: Q1–6 TFNG; Q7–13 note completion; no paragraph labels.
- `conformity`: Q27–30 YNNG; Q31–35 summary text; Q36–40 notes; no paragraph labels.
- `fishbourne-roman-palace`: Q1–6 TFNG; Q7–13 note completion; no paragraph labels.
- `listening-to-the-ocean`: Q1–4 TFNG; Q5–8 matching-information over paragraph labels A–G with explicit letter reuse; Q9–13 per-question A–D single choice.
- `listening-vol7-t9`: Q1–4 and Q8–10 forms; Q5–7, 11–16, and 26–30 per-question single choice; Q17–20 and Q21–25 each bind separate collection rows to one item from a shared letter bank, so both use per-slot feature matching despite the source's Choose FOUR/FIVE wording. Neither group states that answers may be reused.
- `organisational-design`: four Choose TWO groups Q14–21 (separate two-question groups with A–E shared banks); Q22–26 feature matching with A–D and explicit reuse. No paragraph labels.
- `petri-dish`: Q14–19 matching-information on A–F with explicit reuse; Q20–25 feature matching with A–D and explicit reuse; Q26–29 summary text. Printed answer-box line incorrectly says 29–32 and must be flagged as source numbering drift.
- `sleep-study`: Q1–4 TFNG; Q5–13 note text; no paragraph labels. Q3 is review-sensitive but the answer appendix marks NOT GIVEN.
- `western-celebrity`: Q14–20 heading matching for paragraph A–G, Roman i–x bank, no reuse; Q21–23 feature matching A–D, no reuse permission; Q24–26 summary text anchored to F–G.

The reports are cues; primary code/fixture work must retain source PDF/page references and independently verify any ambiguous task or numbering.

Primary visual spot checks confirmed the TFNG fixed-label blocks in chili peppers, Fishbourne, and listening-to-the-ocean; the YNNG and summary-text block in conformity; listening-to-the-ocean's A–G passage labels and explicit repeated-letter permission; the Choose TWO task grouping in organisational design; listening-vol7's four/five collection rows and shared A–F/A–G banks; the petri-dish paragraph-letter matrix; sleep-study's Q3 wording; and western-celebrity's A–G heading targets with an i–x shared bank.
