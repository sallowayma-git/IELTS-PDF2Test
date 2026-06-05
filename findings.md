# Findings

## Current State
- Prior implementation added one-click editable draft generation, overwrite protection, inline completion layout hints, dev PDF/DOCX parsing, CLI generation, batch picking, and a random regression script.
- Known high-risk areas: no-space ellipsis blanks such as `11………`, answer-key extraction from scanned answer pages, sentence-ending matching classification, and post-import routing.
- Margaret Preston `Questions 8-13` now renders as one inline completion group with `q8` through `q13`; no `Questions 8-13 item 11` placeholders remain in the generated source.
- Random regression smoke sample structure passed, but answers failed when answer pages had no extractable text; report now classifies that as `answer_pages_have_no_extractable_text`.
- Latest 30-PDF regression report uses `comparison.groupShapeIssues`; high-frequency mismatches were dominated by: old JS table-rendered paragraph matching, old JS list-rendered completion layouts, single-choice groups misclassified by broad `according to`/`match`/`two` triggers, and `Questions 27 and 28` ranges dropping the second question.
- Production classifier should be semantic-first: paragraph/section matching stays matching even when old JS rendered it as a table; single-choice requires explicit A-D option evidence; multi-choice requires explicit `Choose TWO/THREE letters`.

## 2026-06-04 Silent LLM Chain
- User explicitly disallowed simulated API tests. Vision answer extraction and cloud comparison must be validated only through the real OpenAI-compatible API; if the API cannot run, mark that test as not run instead of substituting mocks.
- Current ordinary artifact minimization removes `pipeline-report.json`, so any user-visible cloud/answer warning must also be persisted into `authoring-ir.audit.issues` or the editable draft page will not see it after minimization.
- Existing `nextRoute=document` for source review explains why generated jobs can land on the first recognition page; the default route should remain `groups` once `authoring-ir.json` exists, while source-review warnings are surfaced inside the editable draft.
- Existing PDF vision fallback can degrade to macOS `sips`, which only renders one preview image. This is insufficient for answer pages at the end of IELTS PDFs; full-page rendering via PyMuPDF/Poppler should be attempted before `sips`.
- With the new API profile, the real Margaret Preston run confirms the intended split: local generation keeps `Questions 8-13` as one inline completion group and vision answer extraction fills the answer key. The cloud model instead labeled the range as `summary_completion` with list layout, so the quality gate correctly marked the job as needing confirmation without overwriting the local draft.
- The provider rejected direct PDF file input with HTTP 400 `file type is not supported`; image fallback is therefore required for this compatible endpoint.
- UI e2e still has an old assertion named `ocr source review route`; it should be updated to expect the editable draft page while preserving visible confirmation state.
- Latest random PDF sample has three structure mismatches; inspect `tmp/pdf-regression-sample/report.json` before deciding whether they are normalizer gaps or classifier bugs.
