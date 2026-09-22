# Listening epic implementation plan

## 1. Outcome and boundary

The target is a real product path for an IELTS Listening paper:

`PDF import -> listening structure recognition -> audio binding/probe -> canonical editing -> cloud repair -> student preview -> package/export -> student submission/scoring`

This plan does not create a second question model. Listening keeps using the existing
`IeltsAuthoringIRV2` task-group, response-group, answer-slot and answer-key structures. The
listening-only data is limited to `modality=listening`, parts, managed audio, playback policy,
optional transcript and audio cues.

The first acceptance fixture is `fixtures/golden/private-real/listening-vol7-t9.pdf`: four
sections, eight expected task groups and forty scoring questions. The observed baseline is
0 task groups / 0 slots, so every milestone below must report expected versus actual structure;
an empty preview is never a pass.

Not in this epic: reading source-coverage expansion, answer-page accuracy tuning, or changes to
the reading publish verdict.

## 2. What already exists and should be reused

- `IeltsAuthoringIRV2` already supports `modality: reading | listening` and an optional
  `listening` structure. `TaskGroupV2`, `ResponseGroupV2`, `AnswerSlotV2` and `AnswerValueV2`
  are shared with Reading.
- `ListeningExamSourceV1` already defines parts, media metadata, playback policy, task groups,
  slots, answer keys and revision audit. Rust and TypeScript validators already check four-part /
  forty-question complete exams, part-to-task assignment, asset/hash closure and playback state.
- `probe_listening_audio_v1` already decodes WAV/MP3/M4A, computes duration/signal metrics and
  fails closed for corrupt, silent, clipped, unsupported or hash-mismatched audio. It is not wired
  into the product path today.
- The frontend already has listening attempt, scoring and playback-controller services. The NAS
  contract/package probes demonstrate synthetic provider and student-runtime compatibility, but
  the authoring application does not currently produce that package from a real imported paper.
- Reading's editor, issue/task surface, canonical CAS journal, cloud repair loop, option banks,
  slot interactions, preview canvas and answer protection should be reused.

## 3. Current blockers

1. Pages 2-8 of the real fixture do not contain reliable word boundaries after the pdfium path.
   Current grammar therefore misses `Questions1-4`, `Completetheformbelow`, A-F/A-G and word-limit
   instructions before task-group construction begins.
2. Scheduler/candidate identity is still hard-coded to `reading` at several production call sites.
   The library UI also narrows rows to `reading | writing` and derives anything non-writing as
   reading.
3. There is no product audio import/binding command. `source_assets` can hold a managed asset and
   the audio probe exists, but neither is connected to import or canonical authoring.
4. The reading runtime/compiler is not the listening compiler. A synthetic
   `ListeningExamSourceV1` contract exists, but no real canonical-to-listening-source product
   conversion and publish path has been proven.
5. The current cloud candidate prompt explicitly says "IELTS Reading" and asks for a passage.
   The repair machinery is reusable after a listening canonical exists, but initial listening
   candidate generation needs a modality-aware contract.

## 4. Delivery phases

### Phase A - Freeze the real baseline and modality entry

Deliverables:

- Add metadata for the real fixture: source SHA, four parts, eight task groups, ranges 1-40,
  expected task types, A-F/A-G alphabets and word limits. Keep the private PDF outside Git.
- Add an explicit Listening choice/confirmation to import; do not infer a publishable modality
  solely from a filename.
- Carry `modality=listening` through ingest job, scheduler, candidate identity, canonical seed,
  library row and workspace route. Remove reading literals only where modality is already known;
  do not introduce a parallel scheduler.
- A listening item with no recognised groups must display “Listening structure not recognised”,
  not appear as a valid Reading draft.

Acceptance:

- Selecting Listening creates exactly one listening library item and the canonical draft reports
  `modality=listening`.
- The same Reading fixture remains `modality=reading`.

### Phase B - Recover instruction tokens without rewriting source evidence

Deliverables:

- Preserve raw pdfium text and source anchors unchanged.
- Add a derived grammar-normalisation view for known IELTS instruction tokens and ranges. It must
  recognise compact and partially split forms such as `Questions1-4`, `Completetheformbelow`,
  `ChooseFOURcorrectanswersA-F`, and `NOMORETHANTWOWORDSAND/ORANUMBER`.
- Prefer geometry gaps when reliable; use bounded instruction lexicon matching only inside likely
  instruction regions. Do not globally dictionary-segment passage text.
- Join instruction lines across neighbouring blocks/pages before signature parsing while retaining
  every contributing source node as evidence.

Counterexamples first:

- Compact real snippets must parse to the same range/signature as spaced equivalents.
- Ordinary passage words containing `section`, `choose` or letters A-F must not create a task group.
- Raw evidence text and hashes must be byte-for-byte unchanged by grammar normalisation.

Acceptance: all eight expected instruction zones in the fixture have ranges and signatures; no
placeholder group is counted as recognised.

### Phase C - Materialise the five observed question families

Reuse the existing shared task/slot model:

- Form completion: Q1-4 and Q8-10 -> text-entry slots hosted in form/table cells.
- Single choice: Q5-7, Q11-16 and Q26-30 -> per-slot radio responses with actual A/B/C options.
- Choose FOUR A-F: Q17-20 -> one shared option bank, four selected labels, declared A-F alphabet.
- Choose FIVE A-G: Q21-25 -> one shared option bank, five selected labels, declared A-G alphabet.
- Note completion: Q31-40 -> text-entry slots hosted in the continuous note structure.

The instruction signature remains the source of cardinality, option alphabet and word/number
limits. Listening-specific code may locate sections and hosts; it must not duplicate option or
answer semantics already shared with Reading.

Acceptance for the real fixture: 4 parts, 8 task groups, 40 unique scoring slots, ranges 1-40,
A-F/A-G and both word-limit forms present. All group/slot/host references close.

### Phase D - Managed audio binding and probe

Deliverables:

- After Listening modality is confirmed, request audio in the import flow. Support file selection
  first; folder/multi-file binding follows the same managed-asset command, not a separate importer.
- Copy audio into the application's managed asset directory, register SHA/size/MIME/role and bind
  it to the item. Moving the user's original file must not break an authored exam.
- Run the existing Rust audio probe before the item can become runtime-ready. Surface decode,
  codec, silence, clipping and hash failures as actionable tasks with “replace/rebind audio”.
- Persist confirmed part cues only when their confidence/evidence satisfies the existing contract.

Contract decision required before implementation: `ListeningExamSourceV1` currently has one
`media` object and parts only reference time cues. A complete-exam audio file fits this shape.
Multiple independent Section MP3s do not. Choose explicitly between (a) v1 accepts one complete
audio file and multi-file support is deferred, or (b) extend each part with a media asset reference.
Do not silently concatenate files or pretend the current single-media contract supports both.

**Decision taken (option b): each part carries its own media reference.** `ListeningPartV2.media`
is `ListeningPartMediaV2`; the exam-level `ListeningStructureV2.media` stays optional and unused
for per-part audio. This is what the real paper needs (four independent Section MP3s) and what
`validate_listening_structure_media_v2` already closes against the draft's `assets` list.

Asset identity rule (stable, content-derived, no extra state to keep in sync):

- `assetId` = `audio-<sha256>` — identical bytes always resolve to the same asset id.
- Package-relative path = `audio/<sha256>.<ext>`, extension lower-cased from the managed file
  (falls back to `bin`). Both are produced by `listening_audio::canonical_media`
  (`audio_asset_id` / `audio_relative_path`) so the NAS package builder reproduces the same
  values without reading a second table.
- Managed file lives at `<appData>/audio/<itemId>/<sha256>.<ext>`, outside the job directory.

The mirror from `listening_audio_assets_v1` onto the draft runs through a **real edit
transaction** (`EditOrigin::ListeningAudio`, base-version CAS, editor journal), so a stale writer
loses to a concurrent human save and `human_protected_targets` is honoured: a part whose media the
user edited by hand keeps their value, and the quality report says the audio is missing instead of
the write silently losing. Triggers: bind / replace / unbind, seed generation
(`ensure_initial_canonical`), and one idempotent re-sync right after seeding in
`processing::scheduler` (the seed reads bindings before it writes the draft, so a bind landing in
between would otherwise be missed by both sides).

Acceptance: corrupt/unsupported/missing audio cannot become ready; valid managed audio remains
playable after the original source file is moved.

### Phase E - Listening-aware cloud candidate and shared repair loop

Deliverables:

- Parameterise candidate generation by modality. Listening asks for parts and shared task/slot
  structures, not a Reading passage. The model must use unresolved answers when the paper provides
  no answer key.
- Keep backend ownership of identity, quality, revision and publish fields exactly as in Reading.
- Reuse the existing repair tools (`read_draft`, `read_source`, `apply_edits`, `record_ruling`,
  `finish`), CAS `baseVersion`, human-edit protection and remaining-task projection.
- PDF question structure may be repaired from the original PDF. Audio-derived claims require an
  attached transcript or a separately authorised audio-capable evidence path; the model must not
  invent answers from the question paper.

Acceptance: a controlled listening candidate with one deliberate structural defect is corrected
through the real scheduler/tool loop; a human-edited slot remains protected; unresolved answers
remain visible tasks.

### Phase F - Authoring presentation

Deliverables:

- Reuse `ExamWorkspacePage` and `ExamCanvas` for task groups and slot editing.
- Add a listening header with four-part navigation, audio binding/probe state and playback-policy
  summary. Do not build a separate question editor.
- Author preview may allow diagnostic seeking; student preview must obey the selected playback
  policy and persist a playback snapshot.
- Blocking audio/part issues must have real actions (bind, replace, confirm cue, open source).

Acceptance: all five fixture question families are editable and render with the same option/text
interactions used by the student model; switching parts does not lose unsaved answers or edits.

### Phase G - Compile, package and student runtime

Deliverables:

- Implement canonical `IeltsAuthoringIRV2(listening)` -> `ListeningExamSourceV1` conversion.
- Validate exact task/slot/question-order closure, four parts/forty questions for complete scope,
  media hash/manifest closure, probe success and playback policy before export.
- Package the managed audio and listening source through the existing NAS listening provider.
- Keep `listeningV1` disabled by default until a real product-path fixture passes import, edit,
  preview, export, provider load, playback recovery, submit and score.

Acceptance: exported package loads through the real student provider, audio bytes match the source
manifest, an interrupted attempt resumes according to policy, and submission/scoring covers all 40
slots without exposing the answer key to the client.

## 5. Test ladder and merge discipline

Each phase starts with a failing product counterexample and lands independently:

1. Unit: compact instruction normalisation, ranges, signatures, audio probe and modality mapping.
2. Canonical: eight groups / forty slots / reference closure and user-edit protection.
3. Tauri service: single listening import, managed audio binding, retry and truthful failure states.
4. Tauri UI: workspace part navigation, task actions, author/student preview.
5. Package/runtime: real provider load, media hash, playback recovery, submit and score.

Synthetic Phase 7 contract scripts remain useful contract checks, but they are not acceptance for
the authoring product. The release gate is the real fixture through the real scheduler and student
provider.

## 6. Recommended implementation order

1. Phase A modality wiring.
2. Phase B no-space instruction recovery.
3. Phase C 4 parts / 8 groups / 40 slots.
4. Phase D single managed complete-exam audio and probe; resolve multi-file contract explicitly.
5. Phase E listening cloud candidate plus shared repair.
6. Phase F authoring presentation.
7. Phase G compile/package/student E2E, then enable the feature flag.

This order obtains a visible, structurally correct listening draft before adding playback and keeps
audio/runtime failures from obscuring recognition failures.
