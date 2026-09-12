# A2 — 第 2 章（验收标准）与第 3 章（信息架构与用户流程）对抗审计

审计日期：2026-09-12
审计对象：`Plan With Files/IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md` 第 169–330 行（§2、§3）
代码基线：工作树 HEAD `47a3806` + 未提交改动（`git status` 显示 `src-tauri/src/processing/*` 等为未跟踪/已修改）
立场：默认每条论断错误/过时/未完成，用当前代码证伪。
约束：只读审计；未运行 `cargo build/test`、`npm run build`；无产品级 E2E。

---

## 章节范围

- §2.1 导入与任务管理（7 条）
- §2.2 本地识别（5 条）
- §2.3 云端识别（5 条）
- §2.4 所见即所得编辑（6 条）
- §2.5 题库与发布（5 条）
- §2.6 设置（4 条）
- §3.1 路由收敛
- §3.2 单文件流程
- §3.3 批量流程
- §3.4 禁词表

判定口径：`HOLDS`＝当前代码已满足；`PARTIAL`＝部分满足或仅在非权威路径满足；`FALSE`＝与当前代码矛盾或未实现；`UNVERIFIABLE`＝静态不可判。§2/§3 是目标验收标准，凡"目标要求"与"当前实现"不一致的，按任务要求记为 FALSE/PARTIAL 并标注其为实现计划缺口。

---

## 断言核对表

### §2.1 导入与任务管理

| # | 断言 | 判定 | 证据 |
|---|---|---|---|
| 1 | 题库首页点"导入"，一次选 1-N 份 PDF/DOCX | HOLDS | `src/features/import/ImportDrawer.tsx:63-68`（`chooseSourceFiles` / `choosePdfFolderSources`）；抽屉 hint 含 PDF/DOCX/TXT/MD `:61` |
| 2 | 文件选中后**立即**建立 N 条题库行，无需 category/frequency/tags/parseMode | PARTIAL | 建行在 `src-tauri/src/processing/commands.rs:59-131`（`import_files`）且无必填表单；但 `import_files_core` 先对每份文件同步 `stage_source_file`（复制+sha256，`src-tauri/src/job_commands.rs:193`）后才返回，`useImportFiles.ts:19-41` 期间 `busy=true`。大 N 时并非"立即"，UI 被 ingest 阻塞 |
| 3 | 标题默认来自文件名，工作区可改 | HOLDS | 默认名 `commands.rs:71-80`；原位改名 `src/features/editor/ExamWorkspacePage.tsx:278-332`（`EditableTitle`）+ `useCanonicalEditor.ts:179-190` |
| 4 | 每行一个简短阶段（排队/本地/云端/合并/待检查/可发布/失败） | PARTIAL | `src/features/library/libraryTypes.ts:16-25` 的 `STAGE_LABEL` 含全部 7 项，但多出"已发布"；文档未列 |
| 5 | 本地与云端在**单题维度并行**；批量由后端限流、**不冻结 UI** | **FALSE** | `src-tauri/src/processing/scheduler.rs:255-361`：先 `STAGE_LOCAL_RECOGNITION`（await 本地 `run_auto_pipeline_core`），再 `STAGE_CLOUD_RECOGNITION`，**顺序执行**，非并发。批量"限流"仅指识别并发（semaphore `scheduler.rs:67-68`），而 ingest 复制+哈希仍在命令内同步完成（见 #2），大批次 UI 仍冻结 |
| 6 | 随时可打开处理中题目；有本地结果先显示本地草稿，云端回来增量提示差异 | PARTIAL/FALSE | `useCanonicalEditor.ts:141-143` 在 `workspace.ds` 为空时抛 `ITEM_DS_NOT_SEEDED`；本地识别进行中即无 DS，`ExamWorkspacePage.tsx:200-214` 显示错误+「运行本地识别」而非本地草稿。云端无增量 diff UI，仅 `subscribeProcessing` 触发 `editor.reload()` 与 `processingNote`（`:59-65, :105`） |
| 7 | 返回题库后任务继续；重启恢复未完成任务或标记可重试 | HOLDS | 后端调度循环 `scheduler.rs:127-167`；`queue.rs:232-254` `recover_on_startup`（非终态→queued 或 ready_for_review）；`retry` `queue.rs:257-271` |

### §2.2 本地识别

| # | 断言 | 判定 | 证据 |
|---|---|---|---|
| 1 | 新任务**直接以 `DocumentIRV2` 为输入**，不先压平 V1 再作主要依据 | **FALSE** | 主链 `src-tauri/src/auto_pipeline.rs:1692` `make_dynamic_split_candidates` → `:1701` `make_dynamic_authoring_ir`（V1）→ `:2067` `build_authoring_v2_shadow`。`DocumentIRV2`（`physical_shadow`）仅作辅助输入 `:1502-1518, :1557`。与 `task_plan.md` M4「主链仍走 `make_dynamic_split_candidates` → V1 IR」一致，与 §24 DoD「新题 direct DocumentIRV2，不经 V1 authoring 主链」冲突 |
| 2 | 单选/多选/TFNG/YNNG/Matching 的 Ready 条件含完整题干与完整选项 | PARTIAL | 前端 `actionableIssues.ts:66-102, 116-129` 检查题干/选项；后端 Ready 由 `set_item_status_ready`（`scheduler.rs:487-499`）读 `quality/state == "ready"` 决定，未见题干完整性作为 Ready 前置的独立证据 |
| 3 | 任一简单题 `prompt == empty` 必须产生 blocker，不允许显示"可发布" | PARTIAL | 前端 `actionableIssues.ts:117-125` 产生 `QUESTION_PROMPT_MISSING` blocker，但**仅限** `CHOICE_TASK_TYPES`/`MATCHING_TASK_TYPES`；发布门 `authoring_v2_commands.rs:352-456` 不计算 prompt-empty，只依赖 `quality.state`/`hardFailures`/unresolved answer；工作区发布按钮未按 blocker 禁用（`ExamWorkspacePage.tsx:142` 仅 `busyAction`） |
| 4 | 题号独立一行/题干折行/选项分行/双栏/跨页仍能几何邻接恢复 | UNVERIFIABLE | 属 P4-T02/T03 范围，`task_plan.md` M4 明确「P4-T02~T06 未开始」；未运行产品级识别验证 |
| 5 | 表格/流程图/diagram 无法结构化时保留源视觉区域与答案槽覆盖层 | PARTIAL | 表格结构编辑与答案槽增删已接入（`src/exam-canvas/structureActions.ts:22-107`），但"源视觉区域+覆盖层"未见实现，P4-T05 未开始 |

### §2.3 云端识别

| # | 断言 | 判定 | 证据 |
|---|---|---|---|
| 1 | 云端接收 PDF/分页图、versioned skill/prompt、题型定义、完整输出 JSON Schema | PARTIAL | 请求体构造于 `src-tauri/src/llm_suggestions.rs:212-264`，含 `outputContract`，但无 versioned skill bundle，`outputContract.schema` 为 `CloudReadingOutlineV1`（`:235`），非完整候选 |
| 2 | 云端**必须返回完整 `CloudRecognitionCandidateV1`** | **FALSE** | 全仓无 `CloudRecognitionCandidateV1` 符号（`Grep src-tauri/src` 零命中）；prompt 明确 "Return an outline for comparison only" `llm_suggestions.rs:253` |
| 3 | 本地 JSON 提取/别名归一化/schema 校验/语义校验；失败最多一次受约束修复 | PARTIAL | 归一化存在（`llm_gateway.rs:1000-1340`）；`repairContract` 字段被序列化（`llm_gateway.rs:461`）但未见"一次 repair 重试循环"接线；P5-T04/T05 pending |
| 4 | 修复仍失败时按 task group 分块保留；前端只显示"云端有 2 个题组未采用" | **FALSE** | 无 salvage；云端结果仅写诊断审计（`auto_pipeline.rs:2372-2444`），从不部分采用；无该 UI 文案 |
| 5 | 第二次"校对"输出 `ReconciliationProposalV1`，只给局部差异建议 | **FALSE** | 无该类型；"校对"实为 `cloud_outline_check_for_job` 写 `cloud_comparison_summary` 审计项（`auto_pipeline.rs:2381-2397`），无 proposal、无 reconciliation engine（P6-T03 pending） |

### §2.4 所见即所得编辑

| # | 断言 | 判定 | 证据 |
|---|---|---|---|
| 1 | 导入完成/本地草稿可用后，打开即最终 IELTS 界面 | HOLDS | `ExamWorkspacePage.tsx:215-232` 渲染 `ExamCanvas mode="author"` |
| 2 | Reading 左 passage 右 questions，无需编辑/预览切换 | HOLDS | `ExamCanvas.tsx:411` `passage-pane`、`:416` `question-pane`；全仓无 edit/preview 开关 |
| 3 | 点击文字原位编辑；选项改文字/增删排序；表格单元格编辑；答案槽可定位 | HOLDS | `ExamCanvas.tsx:114+` `EditableTextNode`/`InlineTextEditor`；`structureActions.ts:22-107`（选项 add/move/delete、表格行列、答案位插删）；定位见 `ExamWorkspacePage.tsx:177-181` |
| 4 | 编辑后当前页立即更新，因为直接读同一 canonical DS | HOLDS | 乐观本地 patch `useCanonicalEditor.ts:192-211`（`applyLocalPatches` + `setDraft`） |
| 5 | 保存改 canonical DS 不改生成 JS；发布时编译 | HOLDS | 编辑走 `apply_editor_commands`（`lib.rs:935`）；发布编译 `nas_package_v2.rs:294` `export_authoring_snapshot` |
| 6 | 普通编辑**只显示三态**（正在保存/已保存/保存失败），不显示 revision/hash/schema | PARTIAL | `ExamWorkspacePage.tsx:20-26` `SAVE_LABEL` 有**四态**，多出 `conflict: 保存冲突`；`saveMessage` 亦显示冲突文案（`useCanonicalEditor.ts:125-127`）。未显示 revision/hash/schema 名称（该部分成立） |

### §2.5 题库与发布

| # | 断言 | 判定 | 证据 |
|---|---|---|---|
| 1 | 题库只呈现每题一个当前版本 DS | PARTIAL | `libraryStore.ts:14-39` 按 id 合并 job/summary/v2 为一行；但合并仍依赖旧三源（`listJobs`/`listLibraryExams`），非纯粹"一个当前版本 DS" |
| 2 | 历史 patch/临时 OCR/LLM IO/预览 HTML 不作为题库条目 | HOLDS | 列表行仅来自 job/summary/v2 item，无过程 artifact 行 |
| 3 | 题库多选"发布到 NAS"；工作区有发布主按钮 | HOLDS | `LibraryPage.tsx:83-106` `publishSelected` + `LibraryBatchBar`；`ExamWorkspacePage.tsx:142` |
| 4 | 发布检查只展示可执行问题（如"第 8 题没有答案""第 17 题缺少 B 选项"） | PARTIAL | `publishClient.ts:18-35` 解析 `blockers[].userMessage`；但 `authoring_v2_commands.rs:352-456` 的多数文案是通用句（"这道题还有未确认的内容…"），只有 `ISSUE_UNRESOLVED` 透传 issue.message，未达到逐题可执行文案 |
| 5 | 成功发布后自动清理**未被 DS 引用**的过程文件 | **FALSE** | `publish_items_core`（`nas_package_v2.rs:261-352`）不调用任何 cleanup；`cleanup_transient_job_artifacts`（`cleanup.rs:99-159`）删除的是**固定清单**，非引用感知；`task_plan.md` M6 明确「引用集合清理未做」 |

### §2.6 设置

| # | 断言 | 判定 | 证据 |
|---|---|---|---|
| 1 | 默认只展示：模型地址/模型名/API Key/测试连接/云端开关 | PARTIAL | `SettingsPage.tsx:209-299` 默认还展示「文件（保留原始 PDF）」与「发布（NAS 目录）」两组 |
| 2 | Provider 只展示当前真正支持的协议 | HOLDS | `SettingsPage.tsx:29-32` `SUPPORTED_PROVIDERS`＝OpenAiCompatible/Ollama |
| 3 | `forceJson` 固定开启，无复选框 | HOLDS | `SettingsPage.tsx:141` `forceJson: true` 硬编码，无对应控件 |
| 4 | 环境预检只在有错误时一条摘要；完整诊断在开发者模式 | HOLDS | `SettingsPage.tsx:201-205`（仅 `blockingChecks`）与 `:374-404`（开发者模式诊断） |

### §3.1 路由收敛

| 断言 | 判定 | 证据 |
|---|---|---|
| 目标 `RouteName` 只保留 `library`/`workspace`/`settings` | **FALSE** | `src/app/router.ts:6` `RouteName = "library" \| "workspace" \| "settings" \| "legacy"` |
| 旧链接重定向 | HOLDS | `router.ts:42-60` `legacyRedirect`、`:110-123` `applyLegacyRedirect` |
| 是否存在 `#/legacy/*` 兼容路由 | TRUE（存在） | `router.ts:11-28, 78-82`；`App.tsx:31-33` 仅渲染 `legacy/writing`，其余 legacy 页解析后经重定向离开 |
| 导入不是独立页面，是题库抽屉 | HOLDS | `ImportDrawer.tsx`；`App.tsx` 无独立 import 路由 |

### §3.2 单文件流程

| 流程步骤 | 判定 | 证据 |
|---|---|---|
| `create ProcessingItem + LibraryItem shell` | HOLDS | `commands.rs:135-170` `queue_import`（`upsert_item_shell` + `enqueue`） |
| `copy source to transient workspace` | HOLDS | `job_commands.rs:172-222` `stage_source_file` |
| `start local worker and cloud worker concurrently` | **FALSE** | `scheduler.rs:255-361` 顺序 local→cloud |
| `local candidate → create initial Canonical DS → workspace openable` | PARTIAL | 本地跑 `run_auto_pipeline_core`（`scheduler.rs:271-281`）产出 authoring-ir/shadow；DS 由 `migrate_single_item`（`scheduler.rs:491`）落地；期间不可打开（见 §2.1#6） |
| `cloud candidate → validate/repair/salvage` | **FALSE** | 无完整候选、无 repair/salvage（见 §2.3） |
| `reconciliation → safe additions applied automatically → conflicts become localized issues` | **FALSE** | 无 reconciliation；云端只写诊断审计，从不自动应用（`auto_pipeline.rs:2372-2444`）。且与 §2.3「不直接写权威稿」自相矛盾 |
| `user edits → publish → compact DS → clean transient artifacts` | PARTIAL | 发布成立；"compact + clean"非发布路径的一部分，cleanup 为固定清单且未被 `publish_items` 调用 |

### §3.3 批量流程

| 流程步骤 | 判定 | 证据 |
|---|---|---|
| 选择 50 份 → **一次事务**建立 50 条 library item + processing job | **FALSE** | `commands.rs:71-131` 为**逐文件循环**：每份各自 `save_job` + `stage_source_file` + `queue_import`（各自 `open_library_connection` + 独立事务，`:135-170`）。无批量单事务，也无 §26.2 修订所述的 batch staging |
| 题库立即显示 50 行 | PARTIAL | 行在 `import_files` 返回后由 `prependOptimistic` 插入（`LibraryPage.tsx:62-71`），非"立即" |
| `local concurrency = min(cpu_count-1, 3)` | HOLDS | `scheduler.rs:42-51` `(cpus-1).clamp(1,3)` |
| `cloud concurrency = configured 1-3` | **FALSE** | `scheduler.rs:48` 硬编码 `cloud_concurrency: 2`；设置页并发输入仅写 localStorage，后端从不读取（见 A2-F08） |
| `PDF rendering concurrency = 1-2` | **FALSE** | 无该设置或 semaphore |
| 每行独立更新 stage/progress/action count | PARTIAL | 事件 `emit_item_updated`（`scheduler.rs:108-124`）带 `progressPercent`；但 `libraryStore` 只把事件当刷新信号（`libraryStore.ts:106-107`），行进度实际由 `job.currentStep` 推导（`libraryTypes.ts:163`），粒度较粗 |
| 打开任意行不影响其他任务 | HOLDS | 每 job 独立 `run_job`（`scheduler.rs:162-164`） |
| 关闭应用后未完成 job 保持 queued/running-interrupted | HOLDS | stage 持久于 SQLite（`queue.rs`），进程退出不清理 |
| 下次启动 running-interrupted → queued 或 action_required | HOLDS | `queue.rs:232-254` `recover_on_startup`；入口 `scheduler.rs:135` |

### §3.4 禁词表

| 断言 | 判定 | 证据 |
|---|---|---|
| 11 个禁词不出现在前端用户可见字符串 | HOLDS（主表面）/ PARTIAL（整体） | 在 `src/features`、`src/exam-canvas`、`src/components`、`src/app` 中检索 `DocumentIRV2/IeltsAuthoringIRV2/SourceAnchor/ResponseGroup/AssetManifest/SHA-256/sha256/Revision conflict/LLM JSON parse failed/CloudReadingOutlineV1/QualityReportV2/CAS`：仅命中**类型标识符/import/注释**，无中文字面量或 title/label 泄漏。唯一用户可见中文泄漏在**孤儿遗留页** `src/pages/StructuredAuthoringEditorV2.tsx:720`「保存冲突：服务器已有新 revision」，该文件经 `legacyRoutes.tsx` 引入，而 `legacyRoutes.tsx` 已不被 `App.tsx` 引用（`Grep` 确认无 import），故当前不可达 |

---

## 发现清单

### A2-F01 [P0] §2.2「新任务直接以 DocumentIRV2 为输入」不成立，主链仍是 V1-first
- 结论：FALSE。
- 证据：`src-tauri/src/auto_pipeline.rs:1692`（`make_dynamic_split_candidates`）→ `:1701`（`make_dynamic_authoring_ir`，V1）→ `:2067`（`build_authoring_v2_shadow`）；`DocumentIRV2` 仅作辅助输入 `:1502-1518, :1557`。`task_plan.md` M4 自认「主链仍走 … V1 IR」。
- 影响：§2.2 第 1 条与 §24 DoD「新题 direct DocumentIRV2，不经 V1 authoring 主链」均未达成；本地识别质量目标（几何直接识别）无法由当前路径保证。
- 建议：把该条标注为 P4 交付物（当前 P4-T01 仅完成物理入口统一），不要在 §2 以既成事实陈述；同步修正 §24 DoD 勾选状态。

### A2-F02 [P0] §2.1/§3.2「单题 local 与 cloud 并发」不成立，实际顺序执行
- 结论：FALSE。
- 证据：`src-tauri/src/processing/scheduler.rs:255-361`：先 await 本地 `run_auto_pipeline_core`，drop local permit 后才进入 cloud 段；`scheduler.rs:5` 注释自述「单题 local→cloud 两段」。
- 影响：单题端到端时延＝本地+云端串行，而非 max(本地, 云端)；§2.1#5、§3.2、§26.3 攻击 D、ch18 P6-T02「单题两路并发/独立 semaphore」全部落空。本地失败时仍会跑云端（`:311` 仅在 cloud_enabled && is_pdf 时跑，本地失败已 `fail_job` 返回 `:289-292`），与「一路失败不取消另一路」也不一致。
- 建议：§2.1#5 与 §3.2 改述为"识别由后端串行/并行调度（待 P6-T02）"，或先实现真并发再声明。

### A2-F03 [P0] §2.3 云端完整候选 / repair / salvage / ReconciliationProposalV1 全部缺失
- 结论：FALSE。
- 证据：`CloudRecognitionCandidateV1` 全仓零命中；`llm_suggestions.rs:235` 输出契约仍为 `CloudReadingOutlineV1`，`:253` 明确 "for comparison only"；`auto_pipeline.rs:2372-2444` 只写 `cloud_comparison_summary` 诊断，无 salvage/proposal/reconciliation。`task_plan.md` M5 = pending。
- 影响：§2.3 五条中 #2/#4/#5 直接证伪；§3.2 的 "validate/repair/salvage → reconciliation → safe additions" 整段不存在。§2.3#5 与 §3.2「safe additions applied automatically」内部还互相矛盾（一个说只给建议、不写权威稿）。
- 建议：§2.3 明确标注为 P5/P6 目标并删除"必须返回"的既成语气；§3.2 删除"applied automatically"，改为"经 reconciliation 生成 proposal，用户确认后写入"。

### A2-F04 [P1] §3.3「一次事务建立 50 条」与 §2.1「不冻结 UI」均不成立
- 结论：FALSE / PARTIAL。
- 证据：`src-tauri/src/processing/commands.rs:71-131` 逐文件循环，每份文件独立 `save_job`+`stage_source_file`（复制+sha256，`job_commands.rs:193`）+ 独立事务 `queue_import`（`commands.rs:135-170`）；`useImportFiles.ts:19-41` 全程 `busy`。
- 影响：批量 50 份大 PDF 时 ingest 阶段复制+哈希串行阻塞在命令内，UI 冻结（违反 ch18 Phase 6 验收「50 文件批量时 UI 不冻结」）；与 §26.2 声称已改成的"batch staging + 短事务"实现不一致。
- 建议：把文件复制/hash 移出命令（先建行返回、后台 promote），或将 §3.3 与 §2.1#2/#5 改为如实描述。

### A2-F05 [P1] §2.5「发布后自动清理未被 DS 引用的过程文件」未实现
- 结论：FALSE。
- 证据：`nas_package_v2.rs:261-352` `publish_items_core` 无 cleanup 调用；`cleanup.rs:99-159` 删除固定清单（cache/preview/llm-suggestions/document-ir.json 等），非引用感知；`task_plan.md` M6「引用集合清理未做」。
- 影响：发布不触发清理；"未被 DS 引用"的资产（孤儿资产 GC）无实现，长期磁盘占用与 §24 DoD「临时 artifact 能在成功/取消/启动/退出后清理」不符。
- 建议：§2.5 标注为 P7-T05 目标；实现按 canonical DS 引用集合清理，并在 publish 成功路径触发。

### A2-F06 [P1] §3.1 路由类型与代码不一致，且存在 `#/legacy/*` 逃生通道
- 结论：FALSE（对"只保留三个"的精确断言）。
- 证据：`src/app/router.ts:6` 含 `"legacy"`；`:11-28` `LegacyPageName` 9 项；`App.tsx:31-33` 渲染 `legacy/writing`；`legacyRoutes.tsx` 已孤儿但仍存在于源码树。
- 影响：文档以代码块形式给出精确 `RouteName`，与实现不符，易误导后续删除（P10）范围判断。
- 建议：§3.1 补注"过渡期额外保留 `legacy` 逃生名与 `#/legacy/*`，P10 删除"，或直接以实际类型为准。

### A2-F07 [P2] §2.4「只显示三态」与代码/§15 冲突；§15 冲突自动重放未实现
- 结论：PARTIAL。
- 证据：`ExamWorkspacePage.tsx:20-26` `SAVE_LABEL` 含第四态 `conflict`；`useCanonicalEditor.ts:123-129` 冲突时保留未保存修改并要求用户处理；计划 §15（第 2447 行）要求"自动拉取最新 DS；若只改不同 node，重放本地命令"，代码未实现自动拉取/重放。
- 影响：§2.4#6 与 §15 内部矛盾；用户遇到多窗口冲突时无 §15 所述自动恢复，只有手动提示。
- 建议：统一为"三态 + 冲突"并同步 §15，或实现 §15 的自动重放。

### A2-F08 [P2] §2.6/§3.3 并发设置是死配置（前端写、后端不读）
- 结论：FALSE（对 §3.3「configured 1-3」）。
- 证据：`SettingsPage.tsx:340-362` 写入 `appSettings.localConcurrency/cloudConcurrency`（localStorage，`appSettings.ts:20-21`）；后端 `lib.rs:1510-1511` 固定 `ProcessingSettings::defaults()`，全仓无 `localConcurrency/cloudConcurrency` 的后端读取。
- 影响：用户改并发数无任何效果；§3.3 声称"cloud concurrency = configured 1-3"不成立。
- 建议：接入后端设置读取，或从设置页移除该控件并如实描述固定并发。

### A2-F09 [P2] §2.1#6「打开处理中题目先显示本地草稿 + 云端增量提示差异」不成立
- 结论：PARTIAL/FALSE。
- 证据：`useCanonicalEditor.ts:141-143` 无 DS 即 `ITEM_DS_NOT_SEEDED`；`ExamWorkspacePage.tsx:200-214` 显示错误态；无云端 diff UI。
- 影响：用户在识别期间打开题目看到的是错误页而非本地草稿；§3.2「workspace becomes openable」的时点被高估。
- 建议：标注为 P6-T04（Workspace live update）目标；实现本地草稿先可见与云端增量 issue。

### A2-F10 [P3] §3.4 禁词在孤儿遗留页仍有中文泄漏
- 结论：PARTIAL。
- 证据：`src/pages/StructuredAuthoringEditorV2.tsx:720`「保存冲突：服务器已有新 revision」。该页经 `src/app/legacyRoutes.tsx:51` 引入，而 `legacyRoutes.tsx` 不被 `App.tsx` 引用（Grep 无 import），当前不可达。
- 影响：若 P10 删除前临时恢复 legacy 入口，或误以为 legacyRoutes 已在用，会直接泄漏禁词。
- 建议：P10 删除该页与 `legacyRoutes.tsx`；在此之前不要重新接入 legacy 路由。

### A2-F11 [P2] §2.2#3 prompt-empty blocker 仅在非权威前端路径，且覆盖不全
- 结论：PARTIAL。
- 证据：`src/features/editor/actionableIssues.ts:117-125` 仅对 choice/matching 生成 blocker；后端发布门 `authoring_v2_commands.rs:352-456` 不计算 prompt-empty；工作区发布按钮不按 blocker 禁用（`ExamWorkspacePage.tsx:142`）。
- 影响："不允许显示可发布"无法保证：若 `quality.state` 仍为 ready，题库会显示"可发布"，而发布门只能靠通用 quality 检查兜底。
- 建议：将 prompt/option 完整性纳入后端 `quality.rs` 硬闭包或 preflight，并让前端 blocker 与后端一致。

### A2-F12 [P3] §2.1#4 阶段清单缺"已发布"
- 结论：PARTIAL。
- 证据：`libraryTypes.ts:16-25` 含 `published: "已发布"`，§2.1 未列。
- 影响：轻微，文档与 UI 词表不完全一致。
- 建议：§2.1 阶段清单补"已发布"。

### A2-F13 [P2] §2.5#4 发布检查文案未达"逐题可执行"
- 结论：PARTIAL。
- 证据：`authoring_v2_commands.rs:361-416` 多为通用文案（"这道题还有未确认的内容…""这道题还有答案没有填写。"），仅 `ISSUE_UNRESOLVED`（`:435`）透传 issue.message。
- 影响：用户无法从发布失败提示直接定位"第 8 题没有答案 / 第 17 题缺少 B 选项"。
- 建议：preflight 生成带题号/节点的 userMessage（复用前端 `actionableIssues` 的题号逻辑）。

### A2-F14 [P2] §3.2 与 §2.3 对"云端结果如何进入题稿"自相矛盾
- 结论：内部矛盾。
- 证据：§3.2（文档 277-279 行）"safe additions applied automatically"；§2.3（195 行）"只指出局部差异和建议，不直接写权威稿"；代码 `auto_pipeline.rs:2372-2444` 云端仅写诊断、从不应用。
- 影响：合并语义未定义（何者"safe"、谁判定、user_edited 如何保护），无法据此实现或验收。
- 建议：以 §2.3 为准统一表述，并引用 P6-T03 reconciliation 的字段级规则。

---

## 与历史审计的关系

- 上一轮 `audit-2026-09-07/report.md` 的 F01–F13 已按 `repair-2026-09-07/progress.md` 与 `task_plan.md` 修复（编译、队列认领、迁移、保存链、批量发布原子性、F13 测试替身出包等），本轮未照抄，回到当前代码复核：F01（编译）、F07（认领绑定）、F09（全阶段恢复）在当前代码中已不成立，判定为已关闭。
- 但上一轮 M4/M5/M7 的结论在本轮 §2/§3 中**依然成立**：M4「Direct V2 main path not delivered」（对应本轮 A2-F01）、M5「Core replacement not delivered」（A2-F03）、M7「Legacy routes … remain」（A2-F06）。`task_plan.md` 亦将 M5 标 pending、M4 标部分完成。
- 新增/加强：A2-F02（local/cloud 顺序而非并发）在上一轮未被单列，本轮由 `scheduler.rs` 直接证伪；A2-F04（批量单事务）与 §26.2 的自我修订记录矛盾；A2-F08（并发设置为死配置）为本轮新发现。
- 上一轮 F11（结构修复未接入）在 `repair` 后已部分关闭：`structureActions.ts` 覆盖选项/表格/答案位，本轮 §2.4#3 相应判 HOLDS。

---

## 证据层级与局限

| 证据层级 | 覆盖内容 | 说明 |
|---|---|---|
| `static` | 本报告绝大多数判定 | 通过 Read/Grep/Glob 阅读当前工作树源码（HEAD `47a3806` + 未提交改动）得出 |
| `command` | 函数/命令签名与调用链（`import_files`、`publish_items`、`apply_editor_commands` 等） | 仅静态追踪，未执行 |
| `doc-only` | §2.3/§3.2 中 `CloudRecognitionCandidateV1`、`ReconciliationProposalV1` 等契约 | 仅存在于计划文本，代码无对应符号 |
| `product` | 无 | 未运行 Tauri/SQLite/NAS 真实链路 |

局限：
1. 未运行 `cargo build/test`、`npm run build`、真实 Tauri E2E，所有"已实现/未实现"结论均为静态判定；运行时行为（尤其并发、恢复、发布清理）未经产品级验证。
2. `CloudRecognitionCandidateV1`/`ReconciliationProposalV1` 的"零命中"基于对 `src-tauri/src` 的 Grep，若存在动态字符串拼接的契约名可能漏检；已用 `CloudReadingOutlineV1` 对照确认为唯一云端输出契约。
3. §2.2#4/#5 的几何恢复能力属识别质量，静态无法判定，标 UNVERIFIABLE。
4. 工作树含未提交改动与未跟踪的 `src-tauri/src/processing/`，本报告以工作树现状为准；若后续提交改变结论，需重跑。
