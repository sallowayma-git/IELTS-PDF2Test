# 后端数据 V2 验收报告（2026-11-08）

实现已覆盖 M1–M4；完整产品最终验收仍缺 Windows 作者端 Tauri/WebView2 跨仓库 UI 链。没有把服务层或 CLI 通过等同于该产品链通过。没有 push，没有修改原目录的用户未提交文件。

## 工作目录与提交

- 作者端：`/Users/maziheng/Downloads/Desktop/copy/PDF2Test-backend-data-v2`，分支 `codex/backend-data-v2`，基线 `a7fa8ca`。
- 学生端：`/Users/maziheng/Downloads/Desktop/IELTS-NASfor-WenDao-backend-data-v2`，分支 `codex/backend-data-v2`，先在新 worktree 快进至远端 `f685552`（原目录 main 保持 `848c76a`）。
- M1：作者端 `1a43569`，学生端 `a8be840`。
- M2：学生端 `4ccf390`；证据保留 `91ed58e`；快速选项改答记录 `d77b617`。
- M3：作者端 `ff8b7a8`。
- M4：作者端当前 HEAD 的 reviewer/command/export 提交；所有提交均附指定 Co-Authored-By。

## 实现与迁移

- 契约先行：SubmissionSnapshotV2 / terminal schema 2；作者端继续接受 V1/1 与 V2/2。版本过新、meta 与实际库版本不一致、V2 缺表均进入异常队列。迁移仅针对本地作者库与本终端 NAS 库，作者绝不迁移或打开 NAS 活库。
- 学生端：slot/max_score/score；批量粗粒度事件（文本编辑合并，离散选择立即记录，eventId 幂等）；Part/app 活动；听力逐题、分数、播放快照事务冻结；作废原因及作废后 ready 快照；session_id/student_uuid 预留。保留 DELETE 日志、单连接、禁止本机作答镜像策略。
- 兼容修复：阅读/听力统一能力常量 1.0.0；考试听力仅按指针锁定 entry；音频读取按批次 resourcesBase；阅读保存路由接入 events，保存/恢复实际逐题时间线。
- 作者端：发布前冻结实际 wrapper payload，指针成功后登记 papers/paper_items，切换旧卷 superseded_at；历史恢复幂等且不把未提交的孤立历史文件当成功卷。原文、题干、选项按冻结哈希查询，缺失返回 CONTENT_UNAVAILABLE。
- 分析导入：原始 JSON、高亮/笔记/计时、写作全文、逐 slot 分数、事件和听力；新 exportedAt 可以补全证据，不能改写已交卷答案、判分或正文；冲突拒绝并保留原始记录。人工批改/改判不会被重导覆盖。
- 判卷：有效改判与撤销、预览令牌保护的批量重判、写作草稿 CAS/批注/复核/唯一发布版、计入统计、身份别名与循环保护、操作人设置、审计。所有读取返回 N，统计默认有效改判并能切回原始。
- 导出：8 张表，保留原始成绩并分列改判后、写作四维和听力，导出前后都记审计。无 IELTS Band 输出，含机构模拟成绩声明。

接口参数详见 [命令接口](../2026-11-08-backend-data-v2-api.md)，模型语义详见 [V2 契约](../2026-11-08-backend-data-v2-contract.md)。改动文件清单见同目录 `changed-files.txt`。

## 真实验证结果与证据层级

| 命令 / 路径 | 真实结果 | 证据层级 |
|---|---|---|
| 作者端 `npx tsc --noEmit` | 通过 | 类型检查 |
| 作者端 `npx vitest run` | 65 文件 / 648 测试通过 | 前端单测 |
| 作者端 `cargo test -- --nocapture` | 1587 passed / 0 failed / 19 ignored；bin、doctest 通过 | Rust 全量；包含服务/命令核心 |
| 作者端 `cargo test analytics:: --lib`（最终边界检查） | 20 passed / 0 failed / 1 ignored（显式夹具生成器） | 命令核心、迁移、真实产物导入 |
| 作者端 `cargo test publish_final_tests --lib` | 38 passed / 0 failed；真实发布核心同时断言内容归档和旧卷撤下状态 | 发布命令处理层 |
| 学生端 `npm run build:server` | 通过 | 编译 |
| 学生端 `node developer/tests/exam/run-all.cjs` | 21/21 文件通过 | Fastify 保存 handler / 真实服务 / 数据库 |
| 学生端 `npm --prefix apps/student-exam test` | 7 文件 / 51 测试通过 | Vue 组件与单测 |
| 学生端 `npm run build:student-exam` 及最终 renderer build | 通过；根未运行 npm ci/install | 构建 |
| 学生端 `node --test developer/tests/js/*.test.js` | 13 通过 / 2 失败：Windows kiosk PowerShell 依赖 `pwsh` ENOENT | JS/平台契约，未全绿 |
| `IELTS_PDF2TEST_REPO=<作者 worktree> python3 developer/tests/ci/run_static_suite.py` | 18/20 通过；同两项 Windows kiosk PowerShell 检查失败；跨仓库契约/镜像/真实 loader 通过 | 静态与契约，未全绿 |
| `python3 developer/tests/e2e/suite_practice_flow.py` | macOS 真实 Electron 通过，阅读/高亮/笔记/写作/最终交卷/V2 ready 快照；`windowsKioskVerified=false` | 学生端产品 UI |
| `python3 developer/tests/e2e/listening_v1_student_flow.py` | macOS 真实 Electron 听力→阅读→写作→交卷通过；快照 9 条听力答案、14 条事件、2 篇作文 | 学生端产品 UI |
| 作者端导入真实 Electron 原字节快照 | 通过；`fixtures/golden/synthetic/backend-data-v2/v2-electron` 已提交原 sqlite/meta/ready 与实际题包 | 实际产品产物→作者命令核心，未经过作者 UI |
| `node scripts/e2e/cross-repo-full-chain.mjs` | 退出 3：缺少 Windows `ielts-author-studio.exe` | CANNOT RUN，未验收 |
| 新/改 E2E JavaScript / Python 语法检查 | 通过 | 脚本检查，不能证明 Windows 链通过 |

完整本机日志和 Electron JSON 报告保留在 `artifacts/backend-data-v2/`。其中两条 UI 报告为 `student-exam-altu-electron-flow-20261108-233251.json`、`student-exam-listening-v1-flow-20261108-232816.json`。真实学生产物夹具已进入 Git，均为合成考生，无真实学生数据。

## 仍未完成的验收与限制

1. Windows 作者端完整 UI 链未执行。脚本已扩展为：发布含听力的试卷 → 真实 Electron 含改答/听力/写作 → 作者导入详情 → 改判和写作发布 → 统计/导出/审计；通过条件强制 workarounds 为空。但本机不能声称这条扩展脚本运行通过，也未验证新命令的实际 Tauri IPC 调用。
2. Windows kiosk 两项静态检查未通过，原因是本机缺 `pwsh`；未跳过或伪造通过。macOS 托管能力以实际 `supported=false` 断言，不能替代 Windows kiosk 验收。
3. 作者端判卷页面本轮未实现（按任务边界）。现有导出需要先设置操作人或传 operator；新增接口供后续页面对接。
4. 当前运行时没有原生解析内容时，返回 explanation=null / explanationAvailable=false，不生成解析。历史 NAS 脚本已清理且无本地冻结内容时，不能恢复原文。
5. 学生名册、班级权限、AI 评分未实现，只有预留结构。写作 task_band 是内部平均计算值，不是 IELTS 官方 Band。
6. 本机工作树保留用于构建的依赖与 PDFium 目录链接，未提交。

## 需要产品拍板

没有新增的必需口径决定。原始作答不改、原始/有效改判切换、0–9 写作维度但不展示 IELTS Band、样本 N、模拟成绩声明均遵循任务书。Windows 产品验收仍须由具备该环境的执行方完成。

## 私有语料与显式忽略

以下 9 个测试在本次真实执行中因缺少私有语料提前返回（Rust 会计入 passed，不能当作实际验证通过）：

- `available_private_real_pdf_corpus_preserves_physical_layer_invariants`
- `indirect_page_resources_preserve_real_chili_passage_and_answer_images`
- `nine_private_pdfs_clean_candidate_review_targets_by_tier`
- `nine_private_pdfs_ideal_candidate_aligns_against_real_text_layer`
- `part_detection_matches_expected_label_for_eight_real_pdfs`
- `phase4_eight_real_pdfs_preserve_instruction_zone_structure`
- `real_organisational_image_only_answer_pages_route_to_full_page_ocr_plan`
- `registered_phase4_pdf_corpus_reaches_frozen_v1_parse_split_and_authoring`
- `stage2_nine_private_pdfs_match_manual_task_presentation_and_anchor_expectations`

以下为 cargo 明确标记的 19 项 ignored（含真实模型/私有语料/诊断或显式产物生成）：

- `analytics::v2_tests::dump_backend_v2_listening_authoring` — explicit E2E fixture generation; requires BACKEND_V2_LISTENING_FIXTURE_OUT
- `cloud_direct::real_model_tests::real_deepseek_direct_round_trip_on_sample_pdfs` — 需要本机真实 DeepSeek 配置与网络；用 --ignored 显式运行
- `cloud_direct::real_model_tests::real_deepseek_listening_direct_vol7_t9` — Explicit real DeepSeek listening command acceptance; needs private PDF and saved credential
- `cross_repo_contract_fixture::writes_cross_repo_contract_fixture` — run through the student client's cross-repository contract test
- `ielts_grammar::real_pdf_acceptance::chili_group1_instruction_zone_truncation_diagnostic` — diagnostic probe; prints chili group-1 evidence previews vs full v1 block text
- `ielts_grammar::real_pdf_acceptance::chili_real_pdf_reaches_all_declared_acceptance_truth` — real private-PDF probe; run explicitly while repairing Chili acceptance
- `ielts_grammar::real_pdf_acceptance::petri_real_pdf_reaches_all_declared_acceptance_truth` — real private-PDF probe; run explicitly while repairing Petri acceptance
- `ielts_grammar::real_pdf_acceptance::phase4_eight_real_pdfs_reach_physical_authoring_quality_truth` — answer pages are image-only and the chain has no OCR yet; un-ignore once answer resolution lands (recognition round)
- `ielts_grammar::real_pdf_acceptance::phase5_real_pdf_edit_and_v2_export_round_trip` — image-only answer pages have no OCR yet; the export gate correctly requires human verification — un-ignore once answer resolution lands (recognition round)
- `llm_gateway::tests::dry_run_cached_job_requests` — invoke through scripts/llm-cost-dry-run.ps1 with a retained job directory
- `pdf_geometry::tests::real_geodiversity_pages_keep_table_and_right_column_blocks_separate` — requires the local ReadingPractice corpus
- `product_chain::dump_published_package_for_nas_contract` — writes artifacts/nas-contract-fixture for the cross-repo contract check; run explicitly
- `product_chain::dump_published_v2_visual_package_for_student_e2e` — writes artifacts/nas-e2e-fixture for the student Electron E2E; run explicitly
- `product_chain::product_chain_same_pdf_paragraph_corpus` — same-PDF paragraph audit; requires the local Files corpus
- `reading_source_v2::tests::writes_pr06_reading_source_v2_fixture` — writes the PR-06 cross-repository fixture to an explicit temporary path
- `tests::files_pdf_samples_auto_pipeline_minimizes_artifacts_and_preserves_review_gate` — superseded by the registered Phase 4 eight-PDF corpus; requires removed legacy Files/*.pdf inputs
- `tests::files_pdf_samples_reach_expected_review_paths` — superseded by the registered Phase 4 eight-PDF corpus; requires removed legacy Files/*.pdf inputs
- `tests::live_llm_repair_contract_on_files_pdf_samples` — requires EPIC8_LIVE_LLM_BASE_URL, EPIC8_LIVE_LLM_API_KEY, and EPIC8_LIVE_LLM_MODEL; calls live provider for Files/*.pdf groups
- `tests::live_rust_llm_gateway_provider_smoke_text_and_vision` — requires EPIC8_LIVE_LLM_BASE_URL, EPIC8_LIVE_LLM_API_KEY, and EPIC8_LIVE_LLM_MODEL
