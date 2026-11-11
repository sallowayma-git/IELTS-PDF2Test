# 后端遗留项与听力运行时交付报告

日期：2026-11-11。任务书 1–5 全部纳入；作者端独立 worktree `F:/workspace/PDF2Test-backend-leftovers`，分支 `codex/backend-leftovers`，基于 `8d46b4b`；学生端 `F:/workspace/IELTS-NASfor-WenDao`，新分支 `codex/listening-runtime-version`，基于 `384ba33`。没有修改页面组件，没有 push。Terra / medium 子代理只探索和独立复核，代码与最终验证由主代理执行。

## 实现与前端接入

### 空听力 Part 门禁

编译后的 runtime 按 Part → taskIds → responseGroups.slotIds → answerSlots 的实际 **scoring** 答案位计数，partial_practice、complete_exam 一律拒绝空 Part，声明题号非空也不能绕过。诊断 `LISTENING_PART_EMPTY`，targetId 为 partId；发布命令错误形状 `LISTENING_PART_EMPTY:<partId>:第 N 部分没有题目，请删除该部分或补题后再发布`。

schema 仍允许空 Part 编辑。保存刷新质量时写入中文 blocking；NAS 发布在创建锁目录、staging、manifest、current-paper 之前编译检查，force 也不能发布空 Part。正常已有 Part 的编译规则不变。

涉及 `src-tauri/src/schema/listening_runtime_v1.rs`、`listening_source_v1.rs`、`ielts_grammar/quality.rs`、`nas_package_v2.rs`、`publish_final_tests.rs`。

### 显式重新编号

补丁 `renumberQuestions {scope:"task"|"exam",taskId?,startAt?}`。task 必须传 taskId，缺省起点为其当前最小题号；exam 缺省 1。起点需正整数，终点不得溢出 u32。按 taskGroups 顺序、大题内的内容嵌入答案位与作答列表顺序编号。更新 questionNumber/displayLabel、嵌入 answer_slot 标签、displayRange、instructionSignature；runtime questionDisplayMap 从 displayLabel 编译。slotId、答案、热点、音频身份不变。

scope=task 和其他大题冲突返回 `AUTHORING_PATCH_RENUMBER_COLLISION:<number>`，不做部分修改。其他拒绝包括 `AUTHORING_PATCH_RENUMBER_SCOPE_INVALID`、`AUTHORING_PATCH_RENUMBER_START_INVALID`、`AUTHORING_PATCH_RENUMBER_SLOT_REUSED` 及既有 missing-task/slot/empty-expression 错误。

听力规则：显式重新编号代表用户确认新的实际题号域。受影响 Part 的 expectedQuestionNumbers 从当前实际 scoring 答案位重建，旧的缺失声明不再保留。云端采纳保留用户确认后的 Part 结构与编号；无法一致合并的候选仍由结构校验拒绝。

逆补丁复用 restoreStructure；共享 Rust/TS fixture 覆盖 forward、撤销、重做、嵌入顺序、任务起点、全卷跨任务编号、听力声明。顺便修复 TS 表达式和 signature 共用数组导致撤销路径相互影响的问题。编号连续后 QUESTION_NUMBER_NONCONTIGUOUS 消失。两种新 op 不在模型工具白名单；现有结构编辑锁与冲突检测生效。

前端后续接入：结构动作 `questions.renumber {scope,taskId?,startAt?}` 编译为上述补丁，明确用户触发，沿用现有 undo/redo 和错误提示。此次不接按钮。

### 矩阵追加

补丁 `insertResponseGroup {taskId,index,responseGroup,slot,value,promptNode?}`。index 为 0..当前组数。responseGroup.slotIds 必须恰好为新 slotId；组/slotId 必须全卷唯一。继承插入位置前一组（头插取第一组）的 kind、cardinality、assignment、scoringPolicy、duplicatePolicy、allowOptionReuse；模板没有的可选策略会清除。optionBankRef 指向同一个库，清除传入内联 options，不复制库内选项。模板没有引用时清除调用方提供的其他引用；内联选项场景仍需调用方提供合法、唯一 ID 的选项。无题干时生成唯一的“新题干”占位，promptNode 可显式提供。

专用错误包括 `AUTHORING_PATCH_RESPONSE_GROUP_TEMPLATE_REQUIRED`、`AUTHORING_PATCH_RESPONSE_GROUP_ALREADY_EXISTS:<id>`、`AUTHORING_PATCH_RESPONSE_GROUP_SLOT_MISMATCH`，另有既有 SLOT_ALREADY_EXISTS、INDEX_INVALID、PROMPT_INVALID；失败原子回滚。逆补丁使用 restoreStructure。

前端后续接入：矩阵上的 `answer-slot.append` 改编译为此补丁，新组一题并共享选项库。普通逐题列表仍可沿用 insertAnswerSlot。此次没有修改前端 action compiler，所以现有矩阵按钮尚需同事接入。

实现/分类文件：`authoring_structure.rs`、`authoring_v2_commands.rs`、`library/repository.rs`、`cloud_adoption.rs`、`cloud_repair/tools.rs`；TS 镜像与类型、结构 parity/cloud-lock 测试；authoring schema、manifest、共享 fixture。学生 schema 镜像已再次同步。

### 学生运行时与真实音频

学生 `server/src/lib/library/runtime-version.ts` 同时定义阅读 0.2.0、听力 1.0.0 和唯一比较函数；原 reading-runtime-version.ts 保留兼容 re-export。与安装包 0.0.2-beta 无关。听力清单 minimumRuntimeVersion 与 source.audit.minimumRuntimeVersion 都校验；阅读所有原有比较也改用同一函数。

两类原因码分别沿用 `reading_runtime_version_incompatible` / `listening_runtime_version_incompatible`，共同中文文案“试卷需要更高版本的学生端，请升级”，前端错误映射也覆盖听力。

真实发布加载又暴露音频读取遗漏 entry.resourcesBase，已修复主音频和 getAssetBytes 两条路径，仍保留资源路径与完整性校验。作者打包器默认最低版本与校验上限也按模态区分，听力用编译器常量 1.0.0，阅读仍 0.2.0。全链路脚本默认学生路径修复为真实仓库。

学生新增 5 条服务回归：低安装版本加载支持的听力及批次音频；高安装版本不能绕过未来引擎要求；source audit 不能被低清单掩盖；篡改脚本仍拒绝；同源常量与比较兼容。没有在学生根目录执行 npm ci/install。

首次含听力的全链路还暴露发布态 E2E 脚本此前完全跳过听力阶段：登记后直接等阅读 P1。已补 `answer_published_listening`，通过真实 UI 填 40 个答案，切换并播放四段音频至结束，点击 Submit 进入阅读。重发钩子移到听力/阅读都尚未提交时，验证已锁定学生沿用旧答案；不做直接路由、API代答、播放状态注入或门禁绕过。涉及学生 `developer/tests/e2e/student_exam_altu_electron_flow.py`。

完整链继续暴露当前分支未包含既有 V2 实现，Submit 报 per-slot cardinality 错误，且没有正式听力表和冻结批次。已确认并整合现成 `codex/backend-data-v2`：837b078（契约入口）、70b0699（M1 数据库与快照）、aab9821（M2 事件与听力 pin/落库）、2be1275（逐位 cardinality 与事务/幂等回归）。本次没有重写上述功能；学生新分支从任务指定的 chore 起点建立后，以 merge commit `0cdfa68` 合入。冲突解决保留本次统一引擎版本与 source audit 校验、resourcesBase 修复，以及真实四段播放/提交的发布态验收；保留 V2 的进入听力前锁卷、正式提交表、每题 Part/题型归属、快照和事件。此整合还带入已有学生阅读事件与签到恢复 UI，对作者页面没有修改。

既有夹具生成器也仍指向已删除的 PDF2Test-backend-data-v2 目录；默认改为同级 PDF2Test，仍支持 IELTS_PDF2TEST_REPO 显式指定。本轮验收指定作者独立 worktree。整合后 npm run verify:exam-contracts 为 23/23 文件通过。

## 验证记录

| 层级 | 命令/路径 | 实际结果 |
| --- | --- | --- |
| 单测/类型 | `npx vitest run` | 78 文件、756 测试通过 |
| 类型 | `npx tsc --noEmit` | 通过 |
| 契约 | `node scripts/verify-schema-contract.mjs --local-only` | errorCount=0；跨仓镜像另由学生静态套件实测 |
| 命令层/Rust | `cargo test ... --lib authoring_structure` | 15 通过，包括 Rust/TS parity、云端保护、锁定与持久化 |
| 编译/Rust | `cargo test ... --lib listening_source_v1` | 14 通过 |
| 命令层/Rust | `cargo test ... --lib publish_final_tests` | 40 通过、1 个显式生成夹具的 ignored；该夹具另由真实 Tauri 脚本调用 |
| 打包/Rust | `cargo test ... --lib nas_package_v2` | 24 通过 |
| 私有诊断 | `cargo test ... --lib diagnose_short_tail_blockcount_drift -- --ignored --nocapture` | 1 通过，8 份真实 PDF 均参与 |
| 学生服务 | `npm run verify:exam-contracts` | 集成前20/20，集成既有 V2 后23/23；新增版本/资源回归 5/5 |
| 学生静态 | `py developer/tests/ci/run_static_suite.py` | 最终 V2 集成后重跑 status=pass，2026-11-11T23:13:26Z |
| 学生产品 | `py developer/tests/e2e/suite_practice_flow.py` | 最终 V2 集成后真 Electron PASS，19.4 秒（先 static，再 suite） |
| 作者产品 | `node .planning/backend-leftovers/product-check.mjs` | 真 Tauri/WebView2 CDP，全部断言通过 |

作者产品证据 `artifacts/e2e-cdp/backend-leftovers-2026-11-11T23-03-07-014Z/report.json`：真实 IPC 创建单题作答组并重新编号 → 保存按钮 → 关闭重启 → 学生预览连续 1–7 且保留一个矩阵；再真实 IPC 删空 partial Part，组卷 UI 确认发布，界面显示 `LISTENING_PART_EMPTY:part-1:第 1 部分没有题目，请删除该部分或补题后再发布`，NAS 路径始终不存在。界面没有 stub。

编辑页当前的“文档提示”尚未展示 Part 级质量项（显示0），而发布界面已展示中文阻断；这是前端提示接入缺口，已明确留给同事，不能把命令层 quality 项当成编辑页提示已经可见。

可归因构建 `node scripts/e2e/build-app.mjs` 通过；exe SHA256 `586c635f286b1c89c4e2c2e64568f79dec83166fce7348e2cde6c31903dc83c8`。产品证据均标记 CDP 自动化通道；普通默认启动未单独作为证据。首轮全量 Rust 1622 passed / 4 failed / 20 ignored，暴露的两条打包版本失败及一条新测试返回形状问题已修正，并由上述定向回归通过；剩下的 V1 基线差异按本任务要求保留。最终源码再次执行 `cargo test --manifest-path src-tauri/Cargo.toml`（独立 CARGO_TARGET_DIR 隔离 Windows 测试 exe）：**1628 passed / 1 failed / 20 ignored / 0 filtered**，672.20 秒，退出101。唯一失败是 `ielts_grammar::real_pdf_acceptance::phase4_eight_real_pdfs_preserve_instruction_zone_structure` 的七卷 blockCount 漂移；题组、答案位、题号等其他 V1 摘要字段一致。golden 未改。日志 `.planning/backend-leftovers/cargo-final-full.log`。此结果不能写成“Rust 全绿”。

含听力的最终完整链：`STUDENT_REPO=F:/workspace/IELTS-NASfor-WenDao node scripts/e2e/cross-repo-full-chain.mjs` **退出0、verdict=passed、workarounds=[]、57/57 断言通过**。证据 `artifacts/e2e-cdp/cross-repo-chain-2026-11-11T23-13-46-410Z/report.json`，学生代码 HEAD `0cdfa68803ec95cbac5127539a3228e2e06fa1dc`。

链路包括：真 Tauri 组卷确认发布 → 真 Electron 三人考试，每人40个听力答案、四段音频真实播放结束、听力 Submit 后三篇阅读/写作/最终交卷 → 第三人锁卷后真实重发并修改新卷听力答案 → 就绪 V2 快照/状态/hash/旧pin → 真 Tauri 数据导入3人，阅读分数和错题、40条听力答案、听力旧pin40分、四Part统计、原文证据、答题事件、改判、写作发布、八表xlsx与审计。没有标记脚本、跳过门禁、直接路由、直接答题API或播放状态注入。听力功能开关是现有产品配置，CDP为自动化通道。

首次失败的全链路报告保留，原因依次为：学生旧分支 E2E 未处理听力、旧分支 per-slot cardinality 与 V2 实现尚未合入。最终通过报告是完整重跑，没有修改旧报告结论。

测试日志保留于 `.planning/backend-leftovers/*.log`，不作为源码提交。最初学生 static 遇 Windows 正在运行的测试 exe 文件占用（LNK1104）失败，后续使用隔离 target、顺序复跑通过；没有跳过契约。

### Rust 显式忽略项

全量没有因缺少本任务的九份私有 PDF 过滤用例，相关实际语料均已运行。下列 20 项是仓库既有/诊断的 opt-in 项；其中契约生成与听力播种由对应验收显式执行，漂移诊断也已显式执行：

- cloud_direct::real_model_tests::real_deepseek_direct_round_trip_on_sample_pdfs — 需要本机真实 DeepSeek 配置与网络；用 --ignored 显式运行
- cloud_direct::real_model_tests::real_deepseek_listening_direct_vol7_t9 — Explicit real DeepSeek listening command acceptance; needs private PDF and saved credential
- cross_repo_contract_fixture::writes_cross_repo_contract_fixture — run through the student client's cross-repository contract test
- ielts_grammar::real_pdf_acceptance::chili_group1_instruction_zone_truncation_diagnostic — diagnostic probe; prints chili group-1 evidence previews vs full v1 block text
- ielts_grammar::real_pdf_acceptance::chili_real_pdf_reaches_all_declared_acceptance_truth — real private-PDF probe; run explicitly while repairing Chili acceptance
- ielts_grammar::real_pdf_acceptance::petri_real_pdf_reaches_all_declared_acceptance_truth — real private-PDF probe; run explicitly while repairing Petri acceptance
- ielts_grammar::real_pdf_acceptance::phase4_eight_real_pdfs_reach_physical_authoring_quality_truth — answer pages are image-only and the chain has no OCR yet; un-ignore once answer resolution lands (recognition round)
- ielts_grammar::real_pdf_acceptance::phase5_real_pdf_edit_and_v2_export_round_trip — image-only answer pages have no OCR yet; the export gate correctly requires human verification — un-ignore once answer resolution lands (recognition round)
- llm_gateway::tests::dry_run_cached_job_requests — invoke through scripts/llm-cost-dry-run.ps1 with a retained job directory
- pdf_geometry::tests::diagnose_short_tail_blockcount_drift — private PDF A/B diagnostic for f922690; writes artifacts/v1-blockcount-drift.json
- pdf_geometry::tests::real_geodiversity_pages_keep_table_and_right_column_blocks_separate — requires the local ReadingPractice corpus
- product_chain::dump_published_package_for_nas_contract — writes artifacts/nas-contract-fixture for the cross-repo contract check; run explicitly
- product_chain::dump_published_v2_visual_package_for_student_e2e — writes artifacts/nas-e2e-fixture for the student Electron E2E; run explicitly
- product_chain::product_chain_same_pdf_paragraph_corpus — same-PDF paragraph audit; requires the local Files corpus
- publish_final_tests::dump_backend_data_listening_fixture — requires PDF2TEST_E2E_LISTENING_APP_ROOT; generates an isolated E2E fixture
- reading_source_v2::tests::writes_pr06_reading_source_v2_fixture — writes the PR-06 cross-repository fixture to an explicit temporary path
- tests::files_pdf_samples_auto_pipeline_minimizes_artifacts_and_preserves_review_gate — superseded by the registered Phase 4 eight-PDF corpus; requires removed legacy Files/*.pdf inputs
- tests::files_pdf_samples_reach_expected_review_paths — superseded by the registered Phase 4 eight-PDF corpus; requires removed legacy Files/*.pdf inputs
- tests::live_llm_repair_contract_on_files_pdf_samples — requires EPIC8_LIVE_LLM_BASE_URL, EPIC8_LIVE_LLM_API_KEY, and EPIC8_LIVE_LLM_MODEL; calls live provider for Files/*.pdf groups
- tests::live_rust_llm_gateway_provider_smoke_text_and_vision — requires EPIC8_LIVE_LLM_BASE_URL, EPIC8_LIVE_LLM_API_KEY, and EPIC8_LIVE_LLM_MODEL

另有一项历史外部夹具测试 `pdf_geometry::tests::real_raster_diagram_fixtures_emit_source_backed_fail_closed_regions` 未标 ignore，但在旧路径 `C:/Users/lenovo/Desktop/working space/0.3.1 working/ReadingPractice/PDF` 不存在时提前 return（roller coaster/elephant 两卷），因此 cargo 的 ok 不代表本环境实跑了这两份材料。Geodiversity 同目录的测试已经列在显式 ignored 中。本次当前九卷私有语料与 V1 八卷诊断均实际参与，未用旧外部测试的 ok 替代它们。

## V1 blockCount 漂移诊断

`git log -S aligned_short_line -- src-tauri/src/pdf_geometry.rs` 定位 `f9226902eedb4872f33e88a4adfe9508b3e1f54a`（2026-10-02）。该提交允许左对齐的短行尾和上文合并。新增 opt-in 诊断在同一 PDFium、同一 glyph 上只切换此规则，生产始终保留当前规则，没有改识别行为，没有改 golden。

当前抽样中，说明段 “You should spend ... based on Reading” 与下一行 “Passage 1 below.” 合并，左边界相同、垂直间距符合换行；文字与 bbox 并未丢失。这支持段落合并改进的判断，而不是依据旧 blockCount 回退。实际产品路径 `tmp/phase4-real-pdf-acceptance/<fixture>/document-ir-v1.actual.json` 的块数如下（golden → 当前）：

| 样卷 | golden | 当前 | 去掉短行尾合并后的几何计数 | 当前几何计数 |
| --- | ---: | ---: | ---: | ---: |
| chili-peppers | 56 | 50 | 55 | 49 |
| conformity | 59 | 51 | 58 | 50 |
| fishbourne-roman-palace | 49 | 42 | 48 | 41 |
| listening-to-the-ocean | 62 | 55 | 60 | 53 |
| organisational-design | 140 | 140 | 138 | 138 |
| petri-dish | 69 | 59 | 67 | 57 |
| sleep-study | 59 | 50 | 58 | 49 |
| western-celebrity | 105 | 103 | 104 | 102 |

最终产品计数与几何计数之间的 1–2 块已核实全部是无文本页占位块（chili/conformity/fishbourne/western 的第5页、ocean 的第5–6页、organisational 的第7–8页、petri 的第6–7页、sleep 的第6页），由最终解析路径补入；短行尾 A/B 的差值与七卷漂移逐一相等。原任务书中的“50→56”等方向与当前 baseline 文件相反：实际 golden 为 56，当前为 50。不得因此反向恢复旧拆段。详细 JSON/bbox 诊断产物：`artifacts/v1-blockcount-drift.json`。诊断 test 的文本包含匹配是抽样线索，需结合 bbox 复核，不能把其他位置出现的相同子串当作合并证据。本文实际引用的是已核对 bbox 的第一页说明段。

例如 chili 第1页旧块 bbox `[54,722.28,512.97,730.68]` 与 `[54,706.68,145.40,715.08]`，新块 `[54,706.68,512.97,730.68]`，文字完整拼为一句；western 第1页同样把末尾 `2 below.` 并回说明段。块数变化、段落文本和位置证据一起支持真实合并改进；本次不更新 golden。

## 需要产品确认与交接

V1 golden 是否接受新段落合并统计，待产品确认，本次保持原基线。

前端需接入 questions.renumber，以及矩阵 answer-slot.append → insertResponseGroup。后端补丁与 schema 已可使用。报告中的命令层/单测证据不等同于前端按钮已实现。
