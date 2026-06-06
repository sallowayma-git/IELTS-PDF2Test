# Findings

## 2026-06-06 Settings Preflight + 100-PDF Live Regression

- 设置页当前 `settings-grid` 把预检、模型列表、新建配置、高级诊断同时排成 3 列；预检项又复用 `.layer-list div` 大卡片样式，导致依赖排查信息在 warning 较多时显得臃肿。
- 预检报告已有足够结构化字段：`ok/errors/warnings/checks[]`，前端可以直接按 severity/ok 归类并默认展示需要处理的项。
- `/Users/maziheng/Downloads/0.3.1 working/ReadingPractice/PDF` 当前有 262 份 PDF，足够抽样 100 篇。
- `ReadingPractice` 目录没有 legacy generated JS 对照文件；现有 `scripts/pdf-regression-sample.mjs` 需要 `--legacy-dir`，不能直接用于这批 PDF 的 100 篇测试。
- CLI 已有 `--run-auto-pipeline`，会创建临时 app root、保存临时 LLM profile 并调用 `run_auto_pipeline_core`，适合作为 100 篇真实 PDF Live smoke 的调度目标。

## 2026-06-06 Windows Package and Compatibility

- `Files/Windows包体与兼容规划.md` 明确第一版 Windows 目标：TXT/MD、文字层 PDF、DOCX、LLM 辅助、导出和 Pack 构建不依赖 Node/Python/OCR；扫描 PDF 继续走云端 vision/人工确认，不打包 Tesseract 或 Python runtime。
- 当前根目录已有 `task_plan.md`、`findings.md`、`progress.md`，但内容属于上一轮 PDF/LLM 导入目标；本轮已在顶部追加当前 Windows 包体与兼容目标，保留旧历史。
- 仓库启动时已有 dirty worktree：多处 Rust、前端、脚本和原 Windows 规划文件已修改。本轮必须按文件范围隔离，不能 revert 既有用户/历史改动。
- 当前 `scripts/package-audit.mjs` 只审计 macOS `.app/.dmg`，且使用 `new URL(...).pathname`，Windows 会有 `/C:/...` 路径风险。
- 当前 `package.json` 的 `verify:release` 固定串联 macOS DMG repack，Windows 构建会失败。
- 当前 Rust `parser.rs` 写死 `python3`，`render_pdf_pages_with_adapter` 实际固定落到 macOS `sips`。Windows 下需要 Python resolver 和结构化 renderer unsupported/manual-review 降级。
- 当前 `environment.rs` 固定探测 `python3`、`pypdf` 和 `renderer:macos-sips`，Windows 预检文案会误导用户。
- 当前 `sidecars/ui-flow-e2e/ui-flow-e2e.mjs` 使用 `new URL(...).pathname`、直接 spawn `npm`，且 Chrome 查找缺 Windows 默认路径。
- 当前 `scripts/pdf-regression-sample.mjs` 默认数据路径绑定本机 macOS 目录，Windows/CI 需要显式传参或 fixture 默认。
- 实现完成后，release/audit 已拆成 macOS/Windows 独立入口；Windows audit 会输出 WebView2 模式、artifact size/SHA-256、git/lockfile 摘要，并阻止默认包体混入 Node/Python/Tesseract/OCR/PDFium。
- Rust runtime 已支持 `EPIC8_PYTHON`、平台 Python 候选、`EPIC8_PDF_RENDERER`、结构化 renderer unsupported/manual-review 降级，以及平台化环境预检。
- Windows 签名/分发已落地为说明文档、`audit-windows-signatures.ps1` 和 Windows smoke workflow；本机 macOS 无法执行 `pwsh` 或生成真实 NSIS/MSI。

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

## 2026-06-06 Cloud/Vision UX Follow-up
- The observed "stuck at 正在生成 1/6" state is a missing user-facing phase indicator, not necessarily a deadlock. `runAutoPipeline` is a blocking call while it performs vision answer extraction and cloud whole-paper comparison, so the import page must show that the cloud model is running.
- `pipeline-report.json` is removed by ordinary process minimization. Any user-visible cloud/vision state must be copied into `authoring-ir.audit.issues` before minimization; otherwise the editable draft page loses the distinction between local output, vision answer extraction, and cloud comparison.
- The Rust LLM gateway already supports `extract_pdf_image_answers` and `generate_pdf_reading_outline`. The Node sidecar did not, so development/diagnostic parity needed explicit command support.
- The export failure pasted by the user is publish-readiness behavior: empty answers, source-review parser warnings, low-confidence questions/groups, and missing human verification are supposed to block export. The fix is clearer guidance plus surfacing vision/cloud evidence, not bypassing publish readiness.
- PDF image extraction now intentionally tries full-page renderers before falling back to `sips`, so tests should not assume a single rendered preview image.
