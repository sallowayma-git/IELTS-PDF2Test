# Findings & Decisions

## Requirements
- 基于 `Files/Epic8-Tauri作者端应用详细设计.md` 与 `Files/Epic8-作者端Web导题与组卷器工程设计.md` 开始开发。
- 最终目标是实现 Tauri 本地应用端的全部开发任务。
- 根目录必须建立 `Plan With Files` 文件夹，放置并持续维护 `task_plan.md`、`findings.md`、`progress.md`。
- 需要建立工程追踪记录表，追踪从设计文档拆分出的细分开发任务及实时状态。

## Research Findings
- 当前工作区初始状态只有两份设计文档，尚无应用源码。
- `Epic8-Tauri作者端应用详细设计.md` 共 1290 行，覆盖产品形态、目录、本地数据、9 个页面、Rust 模型、Tauri Command API、服务伪代码、前端封装、安全、权限、状态机、MVP、任务映射和开发顺序。
- `Epic8-作者端Web导题与组卷器工程设计.md` 共 866 行，覆盖上传、解析、规则粗切、Authoring IR、LLM、模板生成、校验、页面、后端模块、工程任务拆分和分阶段计划。

## Technical Decisions
| Decision | Rationale |
|----------|-----------|
| 使用 Plan With Files 作为持久工程记忆 | 任务规模大且需要跨阶段持续追踪 |
| 优先实现本地端 MVP 闭环 | 设计文档明确 MVP 是导入 PDF/DOCX/TXT/MD、解析、粗切、编辑、预览和导出 JS |

## Issues Encountered
| Issue | Resolution |
|-------|------------|
| 已存在活动 `/goal`，无法重新创建 | 沿用当前活动目标并在本地计划文件记录 |

## Resources
- `Files/Epic8-Tauri作者端应用详细设计.md`
- `Files/Epic8-作者端Web导题与组卷器工程设计.md`
- `Plan With Files/task_plan.md`
- `Plan With Files/progress.md`

## Visual/Browser Findings
- 尚未使用浏览器或视觉材料。

## Document Decomposition - 2026-05-29
- Tauri 本地端推荐形态：Tauri + Rust backend + 桌面内嵌界面，本地应用生成学生端题库 JS/manifest/Pack，本身不随学生端交付。
- 前端页面树包含 Dashboard、ImportWizard、DocumentReview、SplitAndAnswers、GroupEditor、LlmReview、UnifiedPreview、PackBuilder、Settings/LLM/Parsers/Storage/About。
- Rust command API 覆盖 Jobs、Files、Parser、Split/Authoring、LLM、Validation/Preview/Export、Pack。
- 状态机：Draft -> Uploaded -> Parsed -> SplitReady -> AuthoringReady -> NeedsHumanReview/ValidationFailed -> PreviewReady -> ExportReady -> Published。
- MVP 范围：Tauri 壳、本地数据目录、LLM Profile、上传 PDF/DOCX/TXT/MD、Document IR、手动粗切、TFNG/YNNG/单选/文本填空/表格填空、Authoring IR、模板渲染、JS/manifest、统一阅读页预览、正确答案 E2E 100%。
- 旧 Web 工程设计作为输出契约参考，明确最终输出三层契约：ReadingExamSourceV1 数据字段、统一阅读页 DOM 协议、单题 JS/manifest 包装。
- LLM 边界：只输出结构化 JSON patch/草案，不能直接输出最终 JS、不能覆盖人工确认、不能绕过 schema/DOM/E2E 校验。
- 发布前四层校验：Authoring IR 校验、ReadingExamSourceV1 校验、DOM 协议校验、统一阅读页运行时 E2E 校验。
- 当前工作区没有现有学生端运行时代码；设计文档提到目标项目 `/Users/maziheng/Downloads/0.3.1 working`，后续如需真实统一阅读页 E2E 可能要读取/接入该目录。
- 工具链探测：Node v22.20.0、npm 10.9.3 可用；`pnpm`、`rustc`、`cargo`、`cargo tauri` 暂无输出，当前阶段优先实现 Tauri 本地 command、app data 存储与内嵌界面骨架。

## Frontend Design Notes
- visual thesis: 内部生产工具采用“审稿台 + 暗墨纸张 + 单一青绿色操作光”的克制工作台风格，突出流程状态和可审计产物。
- content plan: 左侧工作流导航、顶部任务状态、中心编辑/预览工作台、右侧校验/审计上下文、底部主操作。
- interaction thesis: 页面进入时轻量 stagger；流程步骤切换使用横向滑入；校验问题和导出产物使用可展开的 focused inspector。

## Scope Correction - 2026-05-29
- 用户明确更正：作者端没有独立 Web；`Epic8-作者端Web导题与组卷器工程设计.md` 是旧实现，当前主要开发目标是 Tauri 本地应用端。
- React/TypeScript 代码仅作为 Tauri 桌面应用的内嵌界面，不作为浏览器 Web 作者端交付。
- 后续优先级调整：Rust 本地 command、app data 文件存储、导入权限边界、sidecar 调度和导出服务优先于开发 fallback。
- 旧 Web 工程设计仍保留价值：仅用于 ReadingExamSourceV1、DOM 协议、JS wrapper、manifest、校验层和题型模板契约。

## Implementation Findings - 2026-05-29
- The first implementation pass now provides the Tauri project skeleton and local-app command surface. Several services are intentionally placeholder implementations: parser output, rule split heuristics, LLM suggestion, runtime E2E, and pack zip generation.
- `npm run build` passes for the embedded interface. Tauri/Rust compile is not verified because `rustc` and `cargo` are not installed or not in PATH.
- Browser smoke verification used the non-Tauri development fallback only to exercise the UI flow while Rust tooling is unavailable; it does not change the product scope.

## Implementation Findings - File Dialog and Text Parser - 2026-05-29
- Desktop import flow now uses `@tauri-apps/plugin-dialog` from `src/api/desktopDialogs.ts`; Tauri runtime opens system dialogs and passes explicit local paths into `import_source_file`.
- Non-Tauri development preview keeps a prompt/local fallback only for UI smoke testing while Rust/Cargo are unavailable.
- Export flow now asks for a directory path before calling `export_reading_assets`; Rust still maps `local://exports` to the job `exports/` directory for fallback.
- Rust `parse_document` now reads uploaded `.txt`/`.md` files from app data `uploads/` and generates deterministic `DocumentIRV1` blocks with role hints. PDF/DOCX still use placeholder output until Python parser adapters are integrated.
- Added sidecar entrypoints: `sidecars/python-parser/parser.py` for deterministic txt/md parsing and `sidecars/node-validator/validate-reading-source.mjs` for ReadingExamSourceV1 + DOM protocol validation.

## Resume Findings - 2026-05-29
- 本轮继续执行活动 goal；目标未完成，不能标记 complete。
- 用户更正已作为硬约束：作者端没有独立 Web，React/TypeScript 仅作为 Tauri 桌面内嵌界面；旧 Web 文档只用于 ReadingExamSourceV1、DOM 协议、JS wrapper、manifest 与校验契约。
- 当前核心实现差距：`src/services/devFallbackBackend.ts` 的动态题组生成仍需修正 groupId 连续性与末题 prompt 截断；`src-tauri/src/lib.rs` 的 `split_candidates`/`authoring_ir` 仍主要依赖固定样例，需要改为读取 `document-ir.json` 后动态生成 SplitCandidates 与 ReadingAuthoringIR。

## Dynamic Split and Authoring Findings - 2026-05-29
- Dynamic rule split now derives passage candidates, question group candidates, answer key candidates, and issues from `DocumentIRV1` blocks instead of relying only on fixed sample data.
- The same heuristic is mirrored in the non-Tauri fallback and Rust command source: detect `Questions N-M`, classify group kind, collect answer blocks, map display numbers to qids, and generate `ReadingAuthoringIRV1` groups/questions.
- Browser smoke found and confirmed a duplicate prompt bug caused by joining `block.text` with stripped `block.html`; the fix is to prefer `text` and only fall back to HTML when text is missing.
- `ReadingExamSourceV1.sourceRefs.primaryProvider` remains `author_web` to preserve the old output contract; this is a compatibility field, not a product-scope statement. Rust audit notes use `provider:author_tauri`.
- Tauri backend compile remains unverified because this machine still has no `rustc` or `cargo` in PATH.

## Sidecar Integration Findings - 2026-05-29
- Rust `parse_document` now attempts `python3 sidecars/python-parser/parser.py parse` for TXT/MD uploads before falling back to the built-in deterministic parser with a parser warning.
- Rust `validate_authoring_ir` now generates `ReadingExamSourceV1`, attempts `node sidecars/node-validator/validate-reading-source.mjs`, and merges sidecar ReadingExamSourceV1/DOM findings with built-in Authoring IR validation.
- `src-tauri/tauri.conf.json` now bundles `../sidecars` as resources; Rust sidecar path lookup checks dev cwd, executable-relative resource folders, and `CARGO_MANIFEST_DIR` fallback.
- Validator sidecar unavailability is recorded as a warning, not an error, so built-in validation can keep the authoring flow moving offline.

## LLM Gateway Findings - 2026-05-30
- LLM remains bounded to structured JSON suggestions: no final JS generation, no bypassing template rendering, validation, or human review.
- Added `sidecars/llm-gateway/gateway.mjs`, which calls OpenAI-compatible `/chat/completions` when a profile has an API key and otherwise returns deterministic offline suggestions with `patch`, `questions`, `warnings`, and `evidence`.
- Rust profile saving now writes API keys to app data `config/secrets/<profile>.key` and stores only `apiKeySecretRef`/`hasApiKey` in profile JSON. This is a local-file fallback toward the design requirement; true OS keychain/Stronghold remains pending.
- Rust `llm_classify_group` and `llm_extract_group` now read Authoring IR group context, call the LLM gateway, save `llm-last-suggestion.json`, append `llm-calls.jsonl`, and move low-confidence jobs to `NeedsHumanReview`.
- `apply_llm_suggestion` now rejects low-confidence suggestions and applies only whitelisted patch paths (`/kind`, `/layout/template`, question prompt/interaction) after user confirmation.
- Settings and LlmReview now expose provider/baseUrl/model/temperature/timeout/forceJson/key status, show patch/questions/evidence, and disable auto-apply for confidence below 0.85.

## Toolchain and Pack Build Findings - 2026-05-30
- User requested global machine setup for Rust/Tauri dependencies. The environment now has official `rustup` stable toolchain installed under `~/.cargo`, with `~/.cargo/env` sourced from both `~/.zshrc` and `~/.zprofile`.
- Verified global versions: `rustc 1.96.0`, `cargo 1.96.0`, `rustup 1.29.0`, `rustfmt 1.9.0-stable`, `clippy 0.1.96`, global `tauri-cli 2.11.2`, Node `v22.20.0`, npm `10.9.3`, Xcode Command Line Tools at `/Library/Developer/CommandLineTools`.
- Homebrew was available, but `brew install rustup-init` on this macOS 13 machine attempted to build CMake/Rust dependencies from source and was terminated to avoid a long package-manager compile. Official rustup installation succeeded quickly and is the active Rust toolchain path.
- `cargo check` now passes for `src-tauri`; first compile exposed and fixed real Rust issues: helper/command name collision, `Value::String` mapping, unsized slice serialization, and missing Tauri icon.
- `cargo fmt --check`, `cargo check`, `cargo clippy --all-targets -- -D warnings`, `npm run build`, and `npm run tauri build` all pass after fixes.
- Tauri release artifacts exist at `src-tauri/target/release/bundle/macos/IELTS Author Studio.app` and `src-tauri/target/release/bundle/dmg/IELTS Author Studio_0.1.0_aarch64.dmg`.
- Pack publishing now writes a `ReadingExamPackV1` manifest, `reading-exams/manifest.js`, exam wrapper JS files, and a real standard `.zip` using a small Rust stored-zip writer. Pack publishing validates each source before marking jobs `Published`.
- Added `src-tauri/icons/icon.png` as a minimal local application icon so Tauri build can complete. It is functional build infrastructure, not final branding.

## Parser Adapter and Toolchain Completion Findings - 2026-05-30
- PDF/DOCX parser adapter now covers the local-app import contract without adding fragile global Python packages. PDF extraction uses installed `pypdf 6.9.2`; DOCX extraction reads `word/document.xml` directly through Python stdlib `zipfile` + `xml.etree.ElementTree`.
- Homebrew Python rejected `pip install --user` for `python-docx`, `reportlab`, and `pdfplumber` because of PEP 668 externally managed environment. The chosen resolution is to avoid global Python mutation and keep the parser deterministic with existing/stable dependencies.
- PDF parser boundary recovery handles common glued text from `pypdf.extract_text()`: `READING PASSAGE N`, `Questions N-M`, TFNG/YNNG instruction endings, numbered statements, and `Answers` are split into separate `DocumentIRV1` blocks.
- Fixture smoke results: `/tmp/epic8-parser-fixtures/reading-sample.pdf` produced provider `python-parser-sidecar:pdf:pypdf`, 6 blocks, and passage/question/answer role hints; `/tmp/epic8-parser-fixtures/reading-sample.docx` produced provider `python-parser-sidecar:docx:ooxml`, 7 blocks including a table block and passage/question/answer role hints.
- Rust `parse_document` now routes TXT/MD/PDF/DOCX uploads through the parser sidecar when the uploaded file exists. TXT/MD still have a built-in local fallback; PDF/DOCX sidecar failure falls back to review-required sample IR with a parser warning.
- Global Tauri tooling is now complete in both paths: npm global `tauri-cli 2.11.2` provides `tauri`, and Cargo-installed `tauri-cli 2.11.2` provides `cargo tauri`.
- Current verified quality gates after parser/toolchain work: `python3 -m py_compile sidecars/python-parser/parser.py`, PDF/DOCX parser smoke assertions, `cargo fmt --check`, `cargo check`, `cargo clippy --all-targets -- -D warnings`, `npm run check`, `npm run build`, and `npm run tauri build`.
- Remaining parser risk: this is deterministic text extraction, not layout/OCR-grade parsing. Scanned PDFs and complex layouts should still be routed to manual review/OCR flow before the Epic8 goal is considered complete.

## Runtime Preview and E2E Gate Findings - 2026-05-30
- The old `run_preview_e2e` implementation only reused built-in Authoring IR checks and did not execute generated JS/manifest or simulate answer collection. This was insufficient for the design requirement that the RuntimePreview layer auto-fill correct answers and score 100%.
- Added `sidecars/preview-e2e/preview-e2e.mjs` as a local unified-runtime contract simulator. It executes generated `manifest.js` and exam wrapper JS in a Node VM, verifies `__READING_EXAM_DATA__.register(...)`, checks manifest entry presence, finds runtime-collectible controls/dropzones, fills answers from `answerKey`, requires correct-answer score 100%, checks a wrong-answer sample lowers the score, and returns `runtime` diagnostics.
- Enhanced `sidecars/node-validator/validate-reading-source.mjs` beyond the minimal name check: it now validates allowed group kinds, explicit `allowOptionReuse` for matching/classification, answer coverage, `questionOrder`, `questionDisplayMap`, input/select/textarea/dropzone collectability, and malformed dropzones.
- Rust now has a shared runtime gate used by `run_preview_e2e`, `export_reading_assets`, and `build_pack`. Export and Pack publication cannot bypass the RuntimePreview layer anymore.
- The UnifiedPreview page now exposes runtime diagnostics in the inspector: collected answers, score info, wrong-answer score info, nav/question count, console errors, issues, and answerKey.
- Important limitation: no external student runtime project was found under the expected local paths during this pass. The new sidecar is a deterministic contract simulator, not a Playwright run against a real `reading-practice-unified.html`. E8-07 remains in progress until the actual unified runtime is located or provided and wired into a browser E2E.
