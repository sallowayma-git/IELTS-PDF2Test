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
# 2026-09-01 AI 能力完整性审计

- 审计范围：真实 Tauri UI、Rust command handler、PDF/DOCX ingestion、LLM gateway/sidecar、authoring artifacts、editor/student preview、publish gate、runtime 和相关测试。
- 重点能力：AI 生成、AI 核验、AI 补全，以及模型/工具调用的结构化约束、可观测性、失败降级和人工确认。
- 初始已知：历史记录提到本地生成、视觉答案补全和云端整卷对照已有实现，但需要验证它们是否覆盖 UI 到导出的完整路径，及 Node/Rust 工具调用是否一致。

## 2026-09-02 主线程复核与有效探针证据

有效的网关探针确认以下事实（其余五个探针因上游代理 502 终止，未作为证据）：

- `src-tauri/src/llm_gateway.rs:23-29` 将所有命令路由到 OpenAI-compatible 实现；`src-tauri/src/llm_gateway.rs:88-99` 固定拼接 `/chat/completions` 并只读取 `/choices/0/message/content`。设置中的 Anthropic/Ollama/Custom 类型没有对应协议分派。
- `src-tauri/src/llm_gateway.rs:102-114` 在严格 JSON 失败后截取首个 `{` 到末个 `}`；`src-tauri/src/llm_gateway.rs:462-505` 和 `:691-795` 对缺失字段默认补齐/归一化，未执行 required/additionalProperties、置信度范围或输出 schema 拒绝。
- `src-tauri/src/llm_gateway.rs:117-144` 只对少量字符串化 HTTP 错误重试三次，未覆盖 429/408/500、Retry-After 或模型 JSON/schema 失败；`:402-428` 会把所有 direct PDF 错误都转为图片 fallback。
- `src-tauri/src/llm_gateway.rs:304-318`、`:330-345`、`:348-374` 的请求没有 `tools`/`tool_choice`，响应没有 `tool_calls`/审批/执行；当前不存在受控工具调用闭环。
- `src-tauri/src/llm_gateway.rs:159-170` 把响应正文直接拼入错误；`src-tauri/src/llm_gateway.rs:18-33` 失败时没有统一 error/run record。`src-tauri/src/cleanup.rs:121-148` 和 `:180-207` 默认清除 `cache`、`llm-suggestions`、`llm-calls.jsonl`、视觉原始输出，AI 证据无法长期追溯。
- `src-tauri/src/llm_commands.rs:149-176` 手动题组调用失败时直接降为 deterministic suggestion，且没有把 fallback/失败作为结构化 run 状态；`:199-213` 只按置信度和有限白名单阻止应用。`src-tauri/src/auto_pipeline.rs:1626-1657` 只记录视觉答案摘要，需继续确认其候选是否写回和人工接受入口。
- `src-tauri/src/llm_suggestions.rs:545-574` 只验证 evidence block ID 属于题组且 quote 非空，不验证 quote 是否真实存在于源块；`:458-470` 对 layout template 仅检查非空字符串。

### 初步缺口矩阵

| 优先级 | 能力/缺口 | 影响 | 证据 |
|---|---|---|---|
| P0 | AI 工具调用尚未实现；没有工具 schema、白名单、参数校验、审批、执行结果和审计 | 无法安全接入“AI 调用核验/生成/补全工具” | `src-tauri/src/llm_gateway.rs:304-345` |
| P0 | 视觉答案候选的生成、接受/拒绝、写回、revision/audit 闭环不完整 | 扫描答案可能只停留在诊断文件，用户无法逐项确认 | `src-tauri/src/auto_pipeline.rs:1626-1657`，待主线程核验 |
| P1 | provider 配置与实际请求协议不一致 | Anthropic/Ollama 配置可保存但运行失败或误发请求 | `src-tauri/src/llm_gateway.rs:88-99` |
| P1 | AI 生成输出 fail-open，缺字段默认补齐，JSON 外围文本被接受 | 模型幻觉/畸形结果进入 suggestion 或比较结果 | `src-tauri/src/llm_gateway.rs:102-114`, `:462-505` |
| P1 | endpoint 没有 scheme/host/私网/重定向约束 | 文档内容和密钥可被发送到任意地址 | `src-tauri/src/llm_commands.rs:74-88`, `src-tauri/src/llm_gateway.rs:49-56` |
| P1 | AI 核验没有持久化任务/逐项 diff/证据审阅和导出门禁契约 | 核验状态重启丢失，用户难以判断是否可发布 | `src/pages/UnifiedPreview.tsx:333-499`, `src-tauri/src/cleanup.rs:121-148` |
| P1 | AI 补全没有统一候选状态和人工决策记录 | 无法区分生成、建议、接受、拒绝和最终权威答案 | `src/types/ielts-authoring-v2.ts:230-237` |
| P2 | 重试、错误分类、调用耗时/request id/输入输出 hash 不完整 | 失败不可诊断，限流/认证错误可能被误判为降级 | `src-tauri/src/llm_gateway.rs:117-171` |
| P2 | 测试主要覆盖 OpenAI happy path，缺 provider/工具/负例/真实 UI | 关键安全和产品闭环无回归保护 | `src-tauri/src/lib.rs:1736`, `:2218-2285`, `:3159` |

## 2026-09-02 主线程第一轮模块地图

- 6 个重新派发的只读探针均在约 10 分钟内未返回；已关闭，不能作为审计证据。主线程按关键源码和产品路径复核。
- AI 入口与产品面至少分布在导入向导、统一预览、V2 结构化编辑器、设置页和导出页；背景云端复核由 `UnifiedPreview.tsx` 的队列/调度器触发，需确认它是否覆盖真实 Tauri 导入路径以及结果是否回写持久化 artifact。
- AI 逻辑存在 Rust 网关、Rust command handler、Node sidecar 和历史/诊断脚本多套实现；必须检查协议、输出约束和错误语义是否一致，不能只依赖 CLI/sidecar 的通过结果。

## 2026-09-02 六路并发审计结论（网关/编排/视觉落盘/前端/测试/产品要求）

### 执行流与并发（最重要）
- 产品唯一 runAutoPipeline 调用点 `src/pages/ImportWizard.tsx:162` 硬编码 `executionMode:"localOnly"` 且不传 profileId → 真实 Tauri 后端下**从不发起任何 LLM/云端调用**（`auto_pipeline.rs:1178-1179` cloud_diagnostics_opted_in=false）；后台云复核队列因 profileId=null 永不入队（`UnifiedPreview.tsx:475-476`），前端文案「云端复核自动转入后台队列」在真实后端是空头支票；dev fallback 后端掩盖了这一点（浏览器 e2e 全绿）。
- full 模式内部纯顺序：解析→视觉识别→视觉答案→split→草稿→逐组 LLM 循环→云端 outline（`auto_pipeline.rs:1678-1705`，此刻才发网络调用）→比对→质量门禁，全部阻塞返回。云端 outline 只依赖 PDF 抽图+profile，**不依赖本地解析结果**（比对才依赖），具备并发条件。无任何 spawn/锁原语；`update_job`/`write_json` 非原子。
- `run_cloud_review_core`（`auto_pipeline.rs:1946-2274`）重跑 vision 抽图+outline，会重复计费；只做 outline 对照，不含视觉答案候选。

### 视觉答案落盘
- 候选 `vision_answer_candidate_for_job`（`auto_pipeline.rs:443-480`）只存活于内存，applied=false/diagnosticOnly=true 硬编码（:1406-1407）；唯一落盘 `vision-answer-output.json`（:1404）全仓零读者、且不在 cleanup 删除名单（孤儿）。候选不进 `split.answerKeyCandidates` 合并（:1452-1454 只并本地候选）。
- 审计摘要语义失真：`vision_answer_extraction_summary` 的 filled/missing 按本地 authoring-ir 现状计算（:1627-1630），把本地填的答案说成「视觉模型已补全」；attempted=true 就追加，不看 failure/applied。
- 命令面无 `apply_vision_answers`（对照已有 `apply_vision_transcription` `authoring_commands.rs:285-315`）；UnifiedPreview 只渲染 `vision_transcription_summary`（:298-306），`vision_answer_extraction_summary` 与逐题候选无任何 UI。
- 导出门禁（`runtime_validation.rs:360-389`）与导出链路 grep "vision" 零命中，视觉来源不可查。

### 网关
- **生产代码无任何 tool-calling**（全仓 grep tools/tool_calls 零命中）；实际模型是「JSON 建议 + 人工/半自动审批」，闸门密集但有三处 fail-open：
  - confidence 无 0..1 校验（`llm_gateway.rs:462-464` 只查 is_number 补 0.65；`llm_suggestions.rs:388-394` >=0.85 自动应用）→ confidence:87 可绕过人工审查。
  - evidence quote 只验 blockId 隶属+非空（`llm_suggestions.rs:559-574`），不验 quote 文本存在于源块 → 幻觉引文可通过自动应用。
  - provider 字段是摆设：网关无 provider 分支，全部 POST {base}/chat/completions（`llm_gateway.rs:88-92`）；AnthropicCompatible/Custom 选项误导。
- base_url 零校验（`llm_gateway.rs:49-56` 仅 trim）；429/408/Retry-After 不处理（:135-144）；JSON 校验失败不重试；单次最坏 3×300s 阻塞、逐组串行放大；无请求级耗时/错误分类落盘（llm-calls.jsonl 只记 suggestion 本体）。
- API key 存储良好：keyring 优先、明文回退需 EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK=1、缓存脱敏（`llm_profiles.rs:52-102`、`llm_gateway.rs:36-43`）。
- legacy sidecar gateway.mjs 与 Rust 网关行为漂移（缺 2 种 kind、接受明文 key、静默兜底），已标注 legacy 不在生产路径。

### 前端
- `llm_classify_group/llm_extract_group/apply_llm_suggestion` 已注册（`lib.rs:1431-1433`）且 API 已封装（`tauriCommands.ts:199-209`）但**零 UI 消费方**；`group.llmReview` 持久但无渲染。
- UnifiedPreview 预览编译双重吞错（:225-236 `catch {}`）；Settings testLlmProfile/removeProfile 无 catch（:190-204）；rerunOcr/applyManualTranscription 无 busy 无错误态。
- V2 导出门禁错误格式（`authoring_v2_commands.rs:201-262` 的 `authoring_v2_export_blocked:...`）与前端解析（`ExportPage.tsx:32-82` 只认旧 `nas_export_validation_failed:{json}`）不匹配 → 分类指导全部不可达。
- 导入不可取消、无后端进度事件（全仓 grep `.emit(` 零命中）；云复核队列存 sessionStorage 重启丢失。

### 测试
- Rust 536 测试；AI 全部证据在命令处理器级。零覆盖：JSON 容错解析、重试状态码、provider 分派、endpoint 安全、tool calling、并发顺序。mock server 只接受 1 个请求只回 200（`lib.rs:1736`），重试/负例在结构上不可测。前端零单测；ui-flow-e2e 走 dev-fallback 不经真实 Tauri 后端。
- 关键已有覆盖：云端 mismatch 不覆盖本地答案（`lib.rs:6281-6286`）、对照 summary 最小化后存活（:6287-6299）、localOnly 零云端调用（:9860）、LLM 失败不阻断草稿（:4292）。

### 产品要求（对并发目标的裁定）
- 文档要求：本地确定性解析是唯一可自动写入 AuthoringIR 的来源（总计划:334）；云端只能只读对照 DiagnosticComparison、不得覆盖题源（总计划:45、338）；不把 cloud LLM 作为必经阶段（:3799）；规则优先、规则不足再调 LLM（Tauri设计:1482）。
- 文档**没有**「LLM 与本地规则并发」的表述。用户本轮目标的合规落地方式：LLM 转化与本地解析**并发执行但结果只作只读比对**（缩短总时延），本地权威与「云端不回写」约束不变；云端调用必须显式 opt-in（R44），UI 需提供该开关。
- contracts 侧：confidence 全部 0..1；extractionMode 枚举无 vision/llm 取值（视觉产物溯源只能走 sourceVariant.provider）；document-ir 表格 detectionMode 含 vision_model。

### 修复优先级（本轮执行序）
1. [complete] P0-A 可达性：ImportWizard 增加「云端对照」opt-in（存在启用 profile 时可选），传 profileId + executionMode full；报告回传 profileId 使后台复核队列可用；去重避免双重云端调用。
2. [complete] P0-B 并发：full 模式下用 std::thread::scope 把「PDF 抽图→视觉答案候选→云端 outline」与本地解析并发；本地草稿落盘后 join，做比对+审计+门禁；LLM 失败不阻断草稿。
3. [complete] P0-C 视觉闭环：持久化 vision-answer-candidates.json（逐题 answer/confidence/evidence）；新增 apply_vision_answer_candidates 命令（接受→写入 question.answer+confidence+revision，拒绝→audit）；UnifiedPreview 渲染候选+逐题采用/忽略；修正摘要三态语义；cleanup 收编孤儿文件。
4. [complete] P1 校验收紧：confidence 越界 fail-closed 置 0；evidence quote 文本与源块比对；base_url scheme/host 校验（http 仅限本地/私网）；provider 枚举校验+UI 说明；429/408/Retry-After 重试；网关调用耗时/错误分类落盘 llm-calls.jsonl；profile.enabled 检查。
5. [complete] P1 前端补口：题组「获取 AI 建议」入口（classify+extract+diff 卡+应用/忽略）；V2 导出错误分类解析；预览编译/Settings 静默失败补错误态。
6. [complete] 测试：网关负例矩阵（JSON 容错/重试/429）、视觉候选闭环、并发顺序、endpoint 校验、quote 验真、confidence 越界。

### A7 后续增量改进（非阻断）
1. 导入取消/进度事件：当前导入过程阻塞 UI，无取消按钮；Rust 后端无 `.emit()` 进度事件推送。影响：长文档导入期间用户无法取消，也看不到详细进度。
2. 云复核队列持久化：当前队列存 sessionStorage，刷新/重启丢失。影响：后台队列任务在浏览器重启后需要手动重新触发。
3. Preflight LLM 连通性检查：Settings 环境预检目前只检查 Python/pypdf/renderer，不检查 LLM profile 网络连通性。影响：用户导入时才发现 LLM 配置无效。
4. V2 revision 链分歧：Phase 5 V2 编辑器使用独立 revision 命名空间，legacy `apply_llm_suggestion` 等命令仍写 V1 namespace。影响：系统性架构遗留，需 Phase 6 统一迁移规划。

## 2026-09-03 用户确认的设计修正

- durable audit 不按 HTTP 请求计数：一次 AI 阶段/run 归并网络重试；原始 prompt/response 仍按诊断设置选择性保存。必须保留能解释产品决策的候选生成、接受/拒绝、权威写回、阻断和失败摘要。
- 输出校验的“补默认字段”是 fail-open：例如缺 `confidence` 被写成 `0.65`、缺 `patch/questions/evidence` 被写成空容器；这会把协议违规结果伪装成合法候选。业务字段缺失/类型错/证据或 patch 不合法应拒绝并进入 needs-review。
- 目标 UI 是代码补全式候选审阅：原文为基线，AI 新增绿色、删除红色，接受后才写入 revision；拒绝不改变权威稿，视觉答案同样只是候选。
- 当前工作区已有未提交的并发/候选实现，后续审计以其实际代码为基线，不回滚或覆盖；需重点检查其 `Mutex` 是否把网络调用重新串行化，以及拒绝-only、重复接受、重启后的决策语义。

## 2026-09-03 对抗审计复核基线

- 两轮共 12 个只读探针均在延长等待窗口内没有返回可引用正文，已按异常规则停止；不得将代理空结果视为通过。
- 当前主线程读到的网关入口将 `classify_group`、`extract_group`、视觉转写、视觉答案和云端 outline 全部路由到 OpenAI-compatible `/chat/completions`；`tools`、`tool_choice`、`tool_calls` 尚未进入请求/响应契约。
- 当前 `run_llm_gateway` 的 `llm-calls.jsonl` 是调用级诊断记录，和用户确认的阶段级 durable audit 不是同一层；后续需保留诊断可选性，同时将候选生成/接受/拒绝/写回/阻断/失败归并为阶段事件。

## 2026-09-02 修复实施记录（第一轮优化）

### 已落地
- **P0-B 并发改造**（`src-tauri/src/auto_pipeline.rs`）：`run_auto_pipeline_core_with_gateway` 用 `std::thread::scope` 在本地解析前启动云转化工作线程（`run_cloud_conversion_worker`）：PDF 抽图一次 → 视觉答案候选 → 云端 outline 生成，经 mpsc 通道回传；主线程本地解析/split/草稿落盘/逐组 LLM 并行推进；草稿落盘后 recv 汇合做只读比对+审计。网关闭包经 `Mutex` 共享（`lock_gateway` 防 poison）。此前 full 模式下抽图最多跑 3 次，现在 1 次。`cloud_outline_check_for_job` 拆为 `cloud_outline_generate_with_gateway`（工作线程）+ `cloud_outline_report_from_output`（主线程比对）。
- **P0-C 视觉答案闭环**：候选结构化落盘 `vision-answer-candidates.json`（VisionAnswerCandidatesV1，逐题 questionNumber/questionId/answer/confidence/evidence）；新增 Tauri 命令 `apply_vision_answer_candidates`（`apply_vision_answer_candidates_core`，llm_commands.rs）：采用→写入 question.answer+confidence（verified 保持 false，人工确认门禁不变），忽略→audit；audit 追加 `vision_answer_adoption` issue；`get_job` 回传 `visionAnswerCandidates`；UnifiedPreview 渲染候选面板（逐题采用/忽略，`vision-answer-candidates` testid）+ `vision_answer_extraction_summary` 横幅；`run_cloud_review_core` 后台复核也产出候选；摘要消息三态化（失败/无候选/待确认），不再把本地答案说成视觉已补全；cleanup 把孤儿 `vision-answer-output.json` 加入删除名单，候选文件保留供重启后确认。dev fallback 同步实现（e2e 可用）。
- **P0-A 可达性**：ImportWizard 加载 `listLlmProfiles`，存在启用的非占位 profile 时显示「导入时并发运行云端对照（只读）」开关（默认勾选、可取消、发送内容范围说明），勾选时传 `executionMode:"full"+profileId`；full 模式报告含 cloudComparison.attempted，后台队列 guard 天然去重不重复调用。
- **P1 校验收紧**（llm_gateway.rs/llm_suggestions.rs/llm_commands.rs）：confidence 缺失/非数值/越界 fail-closed 置 0 + warning（suggestion/vision 转写/视觉答案）；cloud outline 置信度 clamp；evidence quote 文本与源块内容归一化比对（`llm_suggestion_quote_mismatches`，流水线与 apply 命令在 document-ir 可用时启用，最小化后人工采纳不受阻）；`save_llm_profile_core` 校验 provider 枚举 + base_url（http 仅限 localhost/私网、拒绝内嵌凭据）；`llm_run_group_core`/`test_llm_profile_core` 校验 profile.enabled；重试增加 429/408 + Retry-After（封顶 5s），非 2xx 先于 JSON 解析处理（HTML 错误页也能按状态重试，raw 截断 300 字符）；`run_llm_gateway` 每次调用写 `{recordType:"llm_call", ok, latencyMs, errorClass}` 到 llm-calls.jsonl（失败也落盘）。
- **前端补口**：Settings 测试连接/删除/预检补错误态与 busy（未保存配置测试给出明确提示）；学生端预览编译失败展示首条错误（`preview-compile-error`）。

### 验证结果
- `cargo test --lib`：**533 通过**（基线 525）；新增 7 个测试全过：429→200 重试、llm_call 观测记录、JSON 容错解析、confidence 越界 fail-closed、幻觉 quote 拦截、base_url/provider 校验、视觉候选采用闭环（含 verified 不被置真断言）。
- `npm run check`（tsc）：通过。
- **10 个失败全部为 Windows 本机预存环境问题**（stash 验证与 HEAD 相同）：8 个需要私有 PDF 语料或 Python pypdf（本机 pip 无网络）、2 个 reqwest 请求头断言（`authorization: Bearer` 在 HEAD 也失败）。
- `npm run e2e:ui-flow`：clear-text route 超时，**stash 验证 HEAD 基线同样失败**（疑与本机缺 pypdf/浏览器环境相关），非本轮改动引入。
- 有意的行为变更（golden drift 裁定）：`vision_answer_extraction_summary` 消息从「视觉模型已从 PDF 图片页补全答案」改为「视觉模型产出了答案候选，尚未写入题稿」，同步更新 lib.rs 测试断言——旧消息把本地解析填的答案错误归因给视觉模型（审计 P1-5）。

### 遗留（按优先级）
- P0-D：V2 导出门禁错误解析（`authoring_v2_export_blocked:*` 与前端分类指导不匹配）。
- P1：题组「获取 AI 建议」UI 入口（llm_classify_group/llm_extract_group/apply_llm_suggestion 已有命令与 API，无 UI 消费方）。
- P1：导入不可取消、无后端进度事件；云复核队列 sessionStorage 重启丢失。
- P2：preflight 无 LLM 连通性检查；test_profile 缓存写到 jobs/profile-test/；请求体大小上限。

## 2026-09-02 对抗审计轮（第 2 轮红队）与修复

两个对抗审计代理分别攻击并发改造与视觉/网关修复。裁定与处理：
- **[阻断→已修复] worker panic 挂死**：Sender 原由函数帧持有，worker panic 时 `recv()` 永久阻塞、scope join 重抛 panic。修复：Sender 所有权移入 worker 闭包 + worker 体 `catch_unwind`（panic 转为 `cloud_conversion_worker_panicked` Err 结果），「LLM 故障不阻断草稿」在 panic 路径也成立。worker spawn 移到本地解析之后（最常见的解析失败提前返回不再等待云端）。
- **[严重→已修复] 「忽略」按钮必然失败**：reject-only 决策被 `accepted.is_empty()` 误判为错误。修复：接受空 accepted（rejected/alreadyAnswered 非空即成功），拒绝项持久化为候选文件的 `dismissedAt`（重启后不再重现），错误仅在决策全为 unmatched 时返回。
- **[严重→已修复] base_url 私网前缀绕过**：`http://10.evil.com` 等公网子域名命中 `starts_with("10.")`。修复：改为 IPv4 字面量严格解析（`Ipv4Addr::parse` + is_loopback/is_private/is_link_local）+ 精确主机名（localhost/::1/.local/host.docker.internal）。
- **[严重→已修复] 候选可覆盖已确认答案**：apply 命令现检查目标题现有 answer 非空或 verified=true → 计入 `alreadyAnsweredQuestionIds` 不写入；decision.answer 覆盖参数限定 string/string[]；数字型 questionNumber 兼容。新增测试覆盖 reject-only 与防覆盖。
- [一般→已修复] 候选 TOCTOU：apply 支持 `generatedAt` 回传校验（`vision_answer_candidates_stale`）；ImportWizard renderModelReport 旧文案「已尝试，未安全写入」改为候选语义。
- [一般→已修复] quote 验真盲区：document 存在但 blockId 查不到时报告 `evidence_quote_block_missing`（不再静默跳过）；归一化增加连字符容错变体。
- [已知留档] `run_cloud_review_core` 每次复核都会刷新候选（设计如此，但无去重）；置信度默认填充顺序导致「missing→0」注释偏差已改注释；Phase5 V2 revision 链与 legacy apply 命令的分歧是系统性既有缺口（apply_llm_suggestion_core 同模式），列为后续项。
- 默认勾选云端对照为产品向决定（用户目标要求 PDF 导入即并发云端对照；勾选框明示发送范围、可取消；localOnly 路径有零云端调用测试守护）。

## 2026-09-02 第三轮补充（遗留 P0/P1 前端口）
- **P0-D 已修复**：ExportPage 新增 V2 发布门禁错误解析（`authoring_v2_export_blocked:quality_state|unresolved_answers|hard_failures|issues=*` 与 `authoring_v2_export_compile_blocked:{json}`），分类为可操作指导，V2 下 canForce=false（与隐藏的强制导出按钮一致）。
- **题组 AI 建议 UI 入口已落地**：UnifiedPreview 题组工作台新增「AI 题组建议」面板——「获取 AI 建议」按钮（llmExtractGroup，需已启用 profile）、建议卡片（建议题型 vs 当前题型、置信度、warnings）、「应用到题组」（applyLlmSuggestion，kind/layout/questions 三路径，置信度 <0.85 时禁用并说明）、持久化 `group.llmReview` 警告横幅渲染。新增样式。
- 最终回归：cargo test --lib 533 通过 / 10 失败（全部为预存环境问题：8 个需私有 PDF 语料或 Python pypdf（本机 pip 无网络）、2 个 reqwest 请求头断言在 HEAD 基线同样失败）；`npm run check` 通过；`npm run e2e:ui-flow` 在 HEAD 基线同样失败（本机环境）。
