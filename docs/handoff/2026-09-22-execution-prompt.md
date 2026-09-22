# PDF2Test 执行任务书（2026-09-22）

> 给执行 agent：请**完整读完本文件**再动手。你只负责开发；质量把关由产品侧另一位 agent 负责，
> 他会按本文件第 6 节逐条复核你的回报。先读仓库根目录 `AGENTS.md`。

---

## 1. 背景

### 1.1 产品

Tauri 桌面应用（Rust 后端 `src-tauri/src`，React/TS 前端 `src/`），把雅思试卷（PDF/DOCX）转成可编辑、可发布的题稿，
发布到 NAS 给学生端使用。学生端是另一个仓库：`F:\workspace\IELTS-NASfor-WenDao`（真实加载器在
`server/dist/lib/library/reading/NasJsDirectReadingAssetProvider.js`）。

用户流程（全自动，目标是一次典型导入用户只做 1–2 个决策）：

```
导入 PDF/DOCX → 本地识别秒出初稿 → 云端独立生成完整候选（generate_authoring_candidate，按原文声明的题段分块）
→ 云端修复循环（repair_authoring_step：read_draft / read_source / apply_edits / record_ruling / finish）
   对照原文件、候选与当前稿，自主编辑权威稿 → 用户只补原文确实缺的答案 → 一键发布
```

### 1.2 不可动摇的边界

- **canonical（`IeltsAuthoringIRV2`，存在 SQLite `library_items_v2.canonical_ds_json`）是唯一权威稿**。预览、导出、学生端全部由它编译。
- **原文件是内容依据**。原文没有的答案绝不编造；伪造判分依据比缺答案严重得多。
- **用户明确修改受保护**，机器编辑不得覆盖（`human_protected_targets`；测试 `cloud_edit_rejects_protected_human_target` 必须保持绿）。
- 网关校验（引文必须有出处、`apply_edits` 必须带 `baseVersion`、题型语义守卫）**不得为了让模型通过而放宽**；修的是 prompt，不是校验器。
- 门禁结论 `PublishVerdict { Ready / Blocked / Undetermined }` **永不被改写成 Ready**。

### 1.3 已定产品决策（不要重开）

1. **一键发布**：前端不展示任何门槛/阻断/校验清单，点「发布」即用户认可。Ready → 正常发布；否则后端以显式记录的放行发布
   （`publishOverride`、`publish_records_v2.forced=1`、条目状态 `published_forced`）。学生端打不开时只写授权快照
   （`studentLoadable=false`），提示「已发布，但学生端暂时无法打开这道题」。可见提示永远不是「发布完成」。
2. **题库保存**：发布后只在 SQLite 保留可编辑最终版，**过程产物与源 PDF/DOCX 全部删除**；发布时冻结证据摘要
   （原文声明题号集合等），清理后仍可编辑、保存、正常再发布；需要原文件的操作（重新识别、答案页重试、查看原文）禁用并说明。
3. **听力**：导入时识别为听力 → 弹窗确认并上传 MP3（拖拽 / 选文件 / 选文件夹）；音频**复制**到
   `<appData>/audio/<itemId>/<sha256>.<ext>`（在 job 目录之外，发布后清理不得触碰），SQLite 记录；
   **每个 part 独立音频**（part 级 `media`）。
4. **题型**：「Choose FOUR/FIVE … write the correct letter next to questions 17–20」→ 每题一个 slot、组内共享一个选项库、
   `assignment: "unordered_set"`（与阅读多选同一模型）。
5. 导入入口：目录批量与单文件并存（UI 已有两个按钮）。

### 1.4 这个项目反复踩过的坑（务必避开）

- **单元测试全绿 ≠ 产品链路通**。以真实 Tauri 链为准。
- **红色信号别归因到身边最方便的解释**。先证明根因。（本项目至少三次误归因，包括把超时说成"本地准备 14 秒"。）
- **验收自证**：受控假模型是照着校验器写的，会掩盖 prompt 与校验器的不一致。
- **三态坍缩**：`未能执行` 不得折叠成 `执行了且通过`；0/0 不是 0%。
- **断言要能失败**；不要用逃生舱（not-executable）混过产品失败。
- 汇报按"用户能感知的变化"排序，测试数字放最后。

---

## 2. 当前进度（`main` @ `86849fc`）

### 2.1 已完成并验证

| 能力 | 证据等级 |
| --- | --- |
| 云端自主修复链；受控模型下真实 Tauri 链 **13/13**（含导出与学生端真实 provider 加载） | 产品端到端（受控模型） |
| 一键发布 + 放行记录（`publish_records_v2`、`published_forced`、`data-publish-outcome` 测试钩子） | 命令处理器层 + 真实链干净发布 |
| 题库保存：冻结证据、清理过程产物与源文件、清理后可正常再发布 | 命令处理器层（完整验收链） |
| 去门控化 UI：唯一「待补充」清单（顶栏按钮开合的侧栏）、状态行说真话、stale 只看人工编辑、剩余任务读时重算、保存冲突自动 rebase、答案页单步重试、设置页去死项 | 单元 + 命令处理器层 + 真实链第 14 步 |
| 云端链路：超时不走图片回退（曾导致最坏 4× 超时）、每次调用记录 attempts/bytes/usage、被拒回复落盘、所有 prompt 与校验器对齐、修复回合一次受约束重试、候选按原文声明题段分块合并、modality 钩子 `cloud_recognition_modality()` | 假服务器 + 单元（**无真实模型**） |
| 听力基础：导入检测、MP3 弹窗、受管音频 + `listening_audio_assets_v1`、per-part `media` 合同、pdfium 几何保留词间距（20 份阅读 PDF 文本 0 变化）、无空格门槛容忍、`listening_parts.rs`（真实听力卷模块级 4 parts / 8 组 / 40 题） | 单元 + 命令处理器层；**真实 App 里的弹窗/拖拽/播放从未跑过** |
| source coverage 漏题检测（两向比对，解析不到返回 Undetermined） | 命令处理器层 + 真实链 |

基线数字（合并后实测）：Rust **973 passed / 0 failed / 11 ignored**；Vitest **390 passed / 27 files**；`tsc --noEmit` 干净。

### 2.2 未完成（即本任务书的任务）

听力从"草稿"到"学生端播放"这一整段还没接上；学生端仍只认单一音频；云端候选不应用听力结构；
若干收尾项；以及三项被外部资源阻塞的验证。详见第 4 节。

---

## 3. 工作约定（必须遵守）

1. **反例先写再修**：每个行为先写会失败的测试，亲眼看它红，再实现，看它绿。回报里说明哪些测试是"先红后绿"。
2. **频繁提交**（WIP 提交可以）：本项目的 agent 多次被 API 会话额度打断，已提交的工作不会丢。提交信息以
   `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>` 结尾。**不要 push**。
3. 在独立分支或 worktree 上开发，不要直接在 `main` 上提交；完成后由质量把关方合并。
4. 不要碰 `customer-delivery/`、`.workbuddy/`（两个仓库都一样）。
5. 环境注意：
   - `src-tauri/lib/pdfium-windows` 与 `fixtures/golden/private-real/listening-vol7-t9.pdf` 是 gitignored，新 worktree 需从主检出**复制**（不要提交）。
   - 新 worktree 里用 `npm ci` 装依赖；**不要**用 junction 链接主检出的 `node_modules`（清理 worktree 时有误删主目录的风险）。
   - 每次 `cargo test` 前先杀残留测试进程，否则链接报 `LNK1104`：
     `Get-Process | ? { $_.ProcessName -like '*ielts_author_studio*' } | Stop-Process -Force`，等约 4 秒。
   - 测试命令：`cd src-tauri && cargo test --lib`；前端 `npx vitest run`、`npx tsc --noEmit`。
   - 真实产品链：`npm run build:app` 后 `node scripts/e2e/tauri-cdp-cloud-repair-chain.mjs`。构建脚本若报"构建期间输入漂移"
     （常见原因：Tauri CLI 重写 `src-tauri/Cargo.toml` 的换行），`git add` 该文件后再构建一次即可。
6. 文件归属：若多个任务并行，按各任务"可改文件"执行；需要改别人归属的文件时，**不要改**，在回报里写清需要的精确改动。

---

## 4. 任务（按建议顺序；T1、T3、T4、T5 可并行，T2 依赖 T1 的草稿形状但可按合同先行）

### T1 听力草稿成形 + 音频写入 part media + 听力检查

**目标**：听力条目的权威稿里有真实的 `listening` 结构（4 个 part、各自题组与 40 个 slot），绑定的音频出现在对应 part 的 `media` 上。

**现状（请核对行号）**
- `src-tauri/src/ielts_grammar/listening_parts.rs`：纯模块，已在 `ielts_grammar/mod.rs` 声明（`#[allow(dead_code)]`），未接入草稿构建器。真实卷输出 1-4, 5-7, 8-10 | 11-16, 17-20 | 21-25, 26-30 | 31-40。
- `build_authoring_v2_shadow`（`ielts_grammar/mod.rs` ~367）与 `recognition/direct_canonical.rs` ~869 仍写死 `"reading"`；modality 权威在库行（`queue_import` 写入，`ensure_initial_canonical` 把种子稿设为 listening）。
- 合同：`ListeningPartV2.media` 可选（assetId, mime, durationMs, sha256, probe…）；`validate_listening_structure_media_v2` 要求"每个 part 都有 media"或"顶层 media"。夹具 `fixtures/golden/synthetic/ielts/phase7-listening-four-part-media-source-v1.json`。
- 受管音频：`src-tauri/src/listening_audio/`（bind / rebind / unbind / 文件夹绑定，表 `listening_audio_assets_v1`），**尚未写进草稿**。
- `quality.rs` ~164–175 对 listening 跳过 source coverage。
- golden 元数据：`fixtures/golden/metadata/listening-vol7-t9.json`（8 页、8 组、40 slot、A-C / A-F / A-G、两种词数限制，needs_review）。

**要做**
1. modality 为 listening 时用 `listening_parts` 构建 `listening` 结构与题组/slot（复用阅读语法：填空宿主、选项库、选择题组），无 passage；修正上述两处 `"reading"`。按决策 4 处理 Q17–20 / Q21–25。
2. 绑定/替换/解绑音频以及种子稿生成时，经**正式编辑事务**（机器来源、CAS）写入/清空对应 part 的 `media`；不得覆盖该字段上的人工保护编辑；assetId 规则稳定且与 T2 的资产清单一致（基于 sha256，写进文档）；探测未通过的音频如实写入 probe 状态。
3. 听力质量检查（**不得**动 `quality.rs` 编译探针区 ~330–342、~959 起，那归 T2）：按 part 的原文声明题段做 source coverage；任一 part 缺少通过探测的音频时产出独立 issue 码（如 `LISTENING_AUDIO_MISSING` / `LISTENING_AUDIO_PROBE_BLOCKED`）。

**先红的测试**：合成 4 section 文本 → 4 parts；真实卷（文件缺失时优雅跳过）→ 4 parts / 8 组 / 40 slot、题号 1–40 各一次、选项库挂对组、词数限制记录；阅读导入逐字节不变（跑现有阅读 golden）；绑 part 2 → 草稿 part 2 media 与绑定一致，4 个都绑后 `validate_listening_structure_media_v2` 通过；替换生效、解绑清空、探测失败如实记录。

**可改文件**：`ielts_grammar/mod.rs`、`ielts_grammar/listening_parts.rs`、`recognition/*`、`library/migration.rs`、`listening_audio/*`、`quality.rs`（编译探针区以外）。

### T2 听力编译 / 打包 / 预览

**目标**：听力条目能编译成 `ListeningExamSourceV1`、打成学生端可加载的 NAS 包、在编辑器里学生预览。

**现状**
- 编译写死阅读：`compile_reading_source_v2` 直接调用于 `authoring_v2_commands.rs` ~798（导出）、`nas_package_v2.rs` ~194、`ielts_grammar/quality.rs` ~963（编译探针，~330 起对所有 modality 运行）、`authoring_validation.rs` ~500。没有 passage 时返回 `RUNTIME_PASSAGE_MISSING`，所以听力稿今天永远导不出。`runtime_compiler.rs` 的 `ExamCompiler` trait 是死代码。
- 导出/manifest 写死 reading：`nas_package_v2.rs` ~4, 19, 64, 93, 315, 340, 742–743, 772–781；`export_nas_library.rs` ~180；`export_artifacts.rs` ~13–66。
- 学生端期望（只读参考 `IELTS-NASfor-WenDao/server/src/lib/library/listening/listening-v1-loader.ts`）：与阅读共用根 `manifest.js`；条目含 `modality:"listening"`、`schemaVersion:"ListeningExamSourceV1"`、`script`（`__READING_EXAM_DATA__.register(...)` 包装）、`assetManifest`、`resourcesBase`、`checksums{scriptSha256, assetManifestSha256, runtimeSha256=canonical JSON 的 sha256}`；资源在 `resources/<examId>/<relativePath>`，用 `ExamAssetManifestV2` 描述，单个资源上限 50 MB；`audit.sourceRevision >= 1`；slot 全计分、题号 1–40；未解析答案加载即拒。PDF2Test 的 `scripts/verify-phase7-listening-package.mjs` 用 `audio/<sha>.wav` 构建了这套布局，照着做（`audio/<sha256>.<ext>`）。
- 发布路径：`publish_items` → `publish_items_core` → `export_authoring_snapshot`（Strict/Forced、`publish_records_v2`、`studentLoadable`）。

**要做**
1. 新建 `src-tauri/src/listening_source_v1.rs`：canonical 听力 IR → `ListeningExamSourceV1`（每 part 的 media、题组/slot/答案键、`audit.sourceRevision` 取编辑版本），返回 (source, issues)，用 `validate_listening_exam_source_v1` 校验。
2. 单一编译入口按 modality 分派（复活 `ExamCompiler` 或写一个普通分派函数；不用就删掉死 trait），四处调用点全部改走它。**阅读行为逐字节不变**：现有阅读测试（含 `publish_verdict_is_equivalent_across_preflight_two_legacy_exports_and_publish`）原样保持绿。
3. 听力 NAS 包：manifest 条目 + script 包装 + `ExamAssetManifestV2` + 从受管音频复制 `resources/<examId>/audio/<sha256>.<ext>` 并校验 sha256。严格发布要求每个 part 已绑定且探测通过、答案全部解析；否则走一键放行发布且 authoring-only（`studentLoadable=false`）——**绝不把学生端会拒绝的听力 runtime 写进学生 manifest**。
4. 编辑器内听力学生预览（`src/features/editor/studentPreview.ts` ~51 只编译阅读）：走分派编译，按 part 渲染并用各自音频（可复用 `src/services/listeningRuntimeV1.ts`、`listeningPlaybackControllerV1.ts`）。

**先红的测试**：合成 4 part / 40 slot / 4 media 的 IR 编译无 issue；缺 part media、未解析答案、探测未通过各产出特定 issue；严格听力发布产出能通过 verify-phase7 式检查的包（命令处理器层）；缺音频 → 放行 + authoring-only；预览视图模型（vitest）。

**可改文件**：新 `listening_source_v1.rs`、`runtime_compiler.rs`、`authoring_v2_commands.rs`、`nas_package_v2.rs`、`authoring_validation.rs`、`export_nas_library.rs`、`export_artifacts.rs`、`quality.rs` 编译探针区、`studentPreview.ts` 及预览相关前端。

### T3 学生端每 part 独立音频（**另一个仓库** `F:\workspace\IELTS-NASfor-WenDao`）

**隔离要求**：不要在主检出里开发——PDF2Test 的验收链会加载它的 `server/dist`。用
`git -C F:\workspace\IELTS-NASfor-WenDao worktree add F:\workspace\IELTS-NASfor-WenDao-listening -b feat/listening-per-part-media`，
只在那里工作；不要碰其未跟踪的 `.workbuddy/`、`server/dist-prev-r14/`；先读该仓库的 `AGENTS.md`。

**现状（请核对）**
- `server/src/lib/library/listening/listening-v1-loader.ts`：media 解析 ~171–177 只绑一个；cue 边界用 `media.durationMs` ~283；校验器单 media。
- `ExamListeningService.ts` ~152, ~210, ~226, ~249–264：播放/进度/提交绑定单一 media。
- `apps/student-exam` `ListeningExamPage.vue` ~18、~115（只读 `parts[0]`）、~145、~158（单个 `<audio>`）。
- `reading/NasJsDirectReadingAssetProvider.ts` `listAssets` ~214–219 只按 V2 runtime 版本过滤、**不按 modality**——听力条目很可能出现在阅读目录里。
- 功能开关 `IELTS_LISTENING_V1`（`routes/exam.ts` ~10–12）与 `VITE_IELTS_LISTENING_V1` 默认关闭：保持默认，测试里打开。
- 合同源头在 PDF2Test：`contracts/listening-exam-source-v1.schema.json`、`src/types/listening-runtime-v1.ts`、上面的四 part 夹具；PDF2Test 有 `npm run sync:phase1:peer`，先看它同步什么、目标是否本仓库；如是，对你的 worktree 路径执行或手工复制同样内容，并在回报中说明。

**要做**：加载器接受 per-part media（仍兼容单一顶层 media），逐 part 解析音频描述并校验 mime/sha/时长、按 part 自己的音频检查 cue；播放/进度按当前 part 的文件，提交不受影响；阅读目录排除听力条目；在你的 worktree 里重建 `server/dist` 并跑完整现有测试（`verify:exam-contracts` 等）。

**阅读必须保持完全可用**：前后各跑一遍全部阅读/考试测试并在回报中给出前后数字。

**先红的测试**：四 part 夹具包能加载；part 3 哈希不符被拒；单 media 夹具仍能加载；目录分离。

### T4 云端候选应用听力结构

**现状**：`auto_pipeline.rs` 的 `cloud_recognition_modality()` 是决定云端 prompt modality 的唯一处（prompt 构造器已支持听力变体）；`reconcile/candidate.rs` 对听力输出只记 `"cloud_listening_parts_not_applied"`，不应用 parts。

**要做**：`cloud_recognition_modality()` 读条目真实 modality（库行）；`candidate.rs` 应用模型给的听力 part 结构（序号、标题、题段 → 经既有引用重写得到 taskIds），**丢弃模型给出的任何 media/资产/音频字段并记 warning**；`candidate_differences` 能比较 part 边界差异；分块候选跨块合并 parts。

**先红的测试**：受控听力候选归一化为 listening、parts 保留且 taskIds 重写、media 被丢弃；阅读候选不变；分块听力候选合并正确。

**可改文件**：`auto_pipeline.rs`（`cloud_recognition_modality` 附近）、`reconcile/candidate.rs`、`cloud_repair/mod.rs`（差异比较）。

### T5 收尾小项

1. **放行发布时包检查失败仍会中断整批**（发布代理保留了这一行为）。与"点发布就一定完成一次导出"冲突：改为该条降级为 authoring-only（`studentLoadable=false`）并继续整批，IO/安全类硬错误仍中断。先写"批内一条放行项包检查失败 → 其余照常发布、该条 authoring-only"的红测。
2. **永久删除时清理受管音频**：找出 v2 是否存在永久删除路径；有则在同一逻辑操作里删 `listening_audio_assets_v1` 行（事务）再删 `<appData>/audio/<itemId>/`（文件失败报告但不致命）；没有则提供 `purge_item_audio(item_id)` 并测试，**不要**发明删除 UI。永不触碰其他条目的音频。
3. **发布后未清理解析缓存** `<appData>/cache/parser/`：条目按名称前缀匹配有误删相似 id 的风险。改为按精确 job id / 源 sha 匹配后清理，测试"相似 id 的另一条不受影响"。
4. **8 个旧 e2e 辅助脚本**仍匹配已删除的「发布完成」/`data-can-export`/「可以导出」/`workspace-recognition-repair-task`/`workspace-preflight-error`/`workspace-preview-runtime-*`：`tauri-cdp-product-chain.mjs`、`tauri-cdp-publish-ready.mjs`、`tauri-cdp-issue-list.mjs`、`tauri-cdp-workspace-layout.mjs`、`tauri-import-edit-publish.mjs`、`tauri-publish-ready.mjs`、`tauri-publish.mjs`、`tauri-direct-canonical.mjs`（再 grep 一遍其它用法）。改用新钩子：`.workspace-notice[data-publish-outcome]`（published / published_forced / published_forced_not_loadable / failed，**只有 published 算干净通过**，不匹配可见文案）；清单是顶栏 `[data-testid="workspace-issues"]` 开合的侧栏，条目 `[data-task-id]`/`[data-task-kind]`，动作 `[data-action-target]`；`workspace-tasks-headline` / `workspace-tasks-clear`；处理副标题 `[data-testid="workspace-processing-note"]`（比对清单前要等它消失）。参考实现：`scripts/e2e/tauri-cdp-cloud-repair-chain.mjs`（**不要改它**）。
5. **单文件导入端到端确认**：同目录放 3 份 PDF，只用「选择文件」选 1 份，产品链断言恰好 1 个条目、只处理这一份。

### T6 真实 App 听力端到端（T1+T2+T3 合并后）

写一条听力 CDP 验收链（参照 `tauri-cdp-cloud-repair-chain.mjs` 的结构与防自证写法）：导入 `listening-vol7-t9.pdf` → 检测弹窗 → 绑定 4 个音频（没有真实 MP3 时用生成的 WAV，并在报告中标注）→ 草稿 4 parts / 40 slot → 各 part 播放器加载受管音频 → 发布 → 学生端真实 provider 加载并能取到每个 part 的音频。弹窗、拖拽、asset 协议播放**至今从未在真实 App 里跑过**，这是本条的核心价值。

### T7 被外部资源阻塞（拿到资源再做；未拿到时如实报"未执行"，不得报通过）

- **真实模型驱动整条链**：需要有额度的网关 key。先单独测一次完整候选请求（profile `timeoutMs` 设到上限）的真实完成时间、usage、`max_tokens`（默认 16384）是否够用，再跑完整链；记录真实模型与受控剧本的行为差异（是否调 `read_source`、`sourceAnchors` 被拒后能否自我修正、`finish` 是否带 note、轮数与是否收敛、有无结构合法但语义荒谬的输出、`record_ruling` 是否被滥用）。
- **28 份未见答案页准确率**：manifest `fixtures/golden/answer-page-generalization-2026-09-20.manifest.json`（seed 20260920，冻结于 `fab14d3`）；此前 `gemini-3.8-flash` 最小视觉请求可用，缺的是额度。逐份单独 staging（用 `PDF2TEST_AUTOMATION_SOURCE_FILES` 文件清单钩子，不要用目录钩子），用题型语义守卫自动筛查，只人工核对被标记项。结果用于重定答案页置信度门（现值 `auto_pipeline.rs` 附近写死 0.85，作用于整份候选的单一标量，全有或全无）。
- **DOCX 端到端**：需要真实 Word 试卷样本；现有只有 1.3 KB 合成文件。

---

## 5. 回报格式（每个任务一份）

1. 第一段：用户现在能做到而以前做不到的事。
2. 提交列表（hash + message）、所在分支/worktree。
3. 每个行为的证据等级：**产品端到端 / 命令处理器层 / 仅单元或 schema**，不得以低冒高。
4. 哪些测试亲眼看到先红后绿；哪些没有（如只是编译失败），如实写。
5. 全量数字：Rust passed/failed/ignored、Vitest、tsc；与基线（973/0/11、390、干净）对比。
6. 偏离任务书之处及理由；需要改别人归属文件的精确描述；未完成项及原因。

## 6. 质量把关方会逐条复核

- 在合并后的树上重跑 `cargo test --lib`、`npx vitest run`、`npx tsc --noEmit`，数字必须与回报一致。
- 重建 App 跑 `tauri-cdp-cloud-repair-chain.mjs`，**必须保持 13/13**（阅读链路不得回退）；T6 完成后另加听力链。
- 抽查"先红"：必要时回退实现提交，确认对应测试会失败。
- 检查边界：没有放宽任何网关校验；没有改写门禁结论；没有编造答案；机器编辑没有覆盖人工保护；清理没有触碰 `<appData>/audio/`。
- T3：阅读前后测试数字一致；主检出 `server/dist` 未被改动。
