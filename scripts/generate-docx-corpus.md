# PDF-derived Word corpus

`generate-docx-corpus.mjs` converts a directory of PDF IELTS samples into a
temporary DOCX regression corpus. It uses the application's existing
`--generate-reading-source` CLI to obtain `DocumentIR`, then rebuilds that
evidence as native Word paragraphs and tables.

The generated documents preserve:

- `Questions 1-13` range headings;
- explicit question-number lines;
- IELTS instructions preceding each question group;
- consecutive `A-D` and `i-xii` marker runs (two-column Word tables by default);
- tables already represented in `DocumentIR`;
- source page boundaries, heading styles, and paragraph order.

Use `--numbering-mode word` to create a native Word-numbering variant. In that
mode leading question numbers use decimal numbering with the detected start
value, while structured `A-D` and `i-xii` marker tables use `upperLetter` and
`lowerRoman` numbering definitions. Consecutive table markers share a Word
numbering instance; sparse markers such as `A/E/F/G` receive per-row start
overrides so Word cannot silently renumber them as `A/B/C/D`. The default
`explicit` mode keeps markers as literal text. The numbering modes use
different output filenames and can coexist. `--option-layout paragraph` also
adds a filename suffix, so table and paragraph variants cannot silently reuse
one another's files.

The output directory contains one `.docx` per source PDF, a latest-run
`manifest.json`, and a variant manifest such as
`manifest-word-numbered-table.json`. The manifest maps source PDF paths to DOCX
paths and records hashes, parser warnings, structure counts,
omitted-image/manual-review flags, and optional round-trip verification. It
also stores the normalized AuthoringIR signature for both source and round-trip
data, including ranges, kinds, instructions, question IDs, prompts,
interaction types, option labels, `optionTexts`, and option-reuse policy.
Verification compares all of those semantic fields. A source with zero groups
only passes when the DOCX conversion also has zero groups.

Existing DOCX files are not trusted only by name. On a repeat run the generator
rebuilds the deterministic bytes and requires the existing hash to match; it
then still records source semantics and performs `--verify` when requested. A
stale or differently configured file fails with a message to rerun using
`--overwrite`.

Filenames containing markers such as `仅原文无题`, `passage-only`, or
`no questions` are treated as negative samples with an expected question-group
count of zero. `--expect-zero <text>` adds another filename substring for a
corpus-specific negative sample. Generated files for those samples include a
`passage-only` filename suffix, so the explicit zero-group expectation survives
the PDF-to-DOCX filename normalization and is honored again during round-trip
parsing. Unmarked umbrella-only documents still retain the manual question
import scaffold because they may represent extraction loss rather than a true
question-free source.

## Small verified sample

```powershell
node scripts/generate-docx-corpus.mjs `
  --pdf-dir "E:\tmp\PDF" `
  --out-dir "$env:TEMP\pdf2test-word-corpus" `
  --sample 3 `
  --seed word-smoke `
  --verify `
  --numbering-mode explicit `
  --skip-build `
  --overwrite
```

## Full corpus

```powershell
node scripts/generate-docx-corpus.mjs `
  --pdf-dir "E:\tmp\PDF" `
  --out-dir "$env:TEMP\pdf2test-word-corpus" `
  --verify `
  --skip-build
```

Use `--option-layout paragraph` to keep every marker line as a paragraph
instead of converting consecutive marker runs to tables. Run without
`--skip-build` when the Rust CLI needs rebuilding. `--strict` makes generation
or round-trip verification failures return a non-zero exit code.

Run `node scripts/generate-docx-corpus.mjs --self-test` for deterministic
native-numbering model checks without reading the PDF corpus or building the
Rust CLI.

`--sample` is seeded random sampling over a stable path order. It is
reproducible, but it is not stratified by passage number or question type. Use
the full corpus for regression, or curate a separate smoke list when coverage
of P1/P2/P3, matching, flow/table/diagram, and passage-only negatives must be
guaranteed.

This is a cross-format consistency corpus, not OCR ground truth: text defects
already present in PDF extraction remain visible by design. Real user-authored
Word samples are still needed to cover floating text boxes, complex merged
tables, tracked changes, and unusual Word automatic-numbering schemes.
