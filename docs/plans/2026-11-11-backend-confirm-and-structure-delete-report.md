# 入库确认与编辑器结构删除：后端交付报告

日期：2026-11-11。分支：`codex/backend-confirm-structure-delete`。本地提交，不 push。

后端已实现权威确认、发布前确认门禁、无内容节点的答案位增删、作答组/大题删除和带冲突保护的逆补丁。未修改页面组件、结构动作编译器或学生仓。探索与独立复核均使用 Terra / medium；代码修改和最终验证由主代理完成。

## 变更文件

| 范围 | 文件与用途 |
| --- | --- |
| 确认持久化 | `src-tauri/src/library/schema.rs`：v12；新增 `library/confirmation.rs`：单条、批量、本地集合导入、在用判断与发布锁 |
| 行输出 | `library/repository.rs`：`confirmedAt`；`library/commands.rs`：无副作用的共享行序列化；`library/mod.rs` 注册模块 |
| Tauri 接口 | `src-tauri/src/lib.rs`：注册三个确认命令和结构补丁模块 |
| 权威稿替换 | `cloud_direct.rs`、`library/migration.rs`：成功替换时同事务清除确认，并与发布互斥 |
| 发布 | `nas_package_v2.rs`：快照中核对确认，NAS 写入前拒绝；`publish_final_tests.rs`：发布夹具显式确认及拒绝回归 |
| 结构编辑 | 新增 `src-tauri/src/authoring_structure.rs`；`authoring_v2_commands.rs` 将五类结构补丁派发到该模块 |
| 云端与门禁 | `cloud_adoption.rs`：分类及删除证据重放；`library/repository.rs`：影响范围与删除证据；`cloud_repair/tools.rs`：明确排除用户删除/撤销；`ielts_grammar/quality.rs`：题号间隙诊断 |
| TS 镜像 | `src/services/authoringV2Patches.ts`、`src/types/authoring-editor-v2.ts`；新增 `src/types/authoring-structure.test.ts`、`authoring-structure.cloudLock.test.tsx` |
| 契约 | `contracts/ielts-authoring-ir-v2.schema.json`、`contract-manifest.json`；新增共享 `contracts/fixtures/authoring-structure-parity.json` |

## 迁移与确认接口

v11 → v12 在现有迁移事务中增加 `confirmed_at TEXT NULL`、`confirmed_by TEXT NULL`。`user_version` 保证只执行一次；重复开库不会重新确认已退回的旧条目。已有发布记录取 `MIN(created_at)`；只有 published 前缀状态的历史行，优先取最终版发布时间，再退回该行 `updated_at`。后一种旧数据无法恢复精确首次发布时间，该退回值不应被当作精确历史证据。其他行保持 NULL。

| Tauri 命令 | 输入 | 返回 |
| --- | --- | --- |
| `set_library_item_confirmed` | `{ itemId: string, confirmed: boolean }` | 更新后的完整 V2 行，含 `confirmedAt`、`processing`、`questionTypes`、`examId` |
| `set_library_items_confirmed` | `{ itemIds: string[], confirmed: boolean }` | `{ results: [{ itemId, ok: true, row } 或 { itemId, ok: false, error }] }` |
| `import_local_confirmed_ids` | `{ itemIds: string[] }` | 已接受 ID 的去重数组；重复导入保留原确认时间 |

批量使用一个 Immediate 事务。业务拒绝逐条返回；数据库错误回滚整批。确认不改 `current_edit_version`、`status`、`updated_at`，不写编辑日志，返回行也不触发 Part 惰性修复。`confirmed_by` 为本地通用操作人 `user`，不引入账号体系。

确认错误为 `LIBRARY_ITEM_NOT_CONFIRMABLE:not_found|deleted|processing|failed|canonical_missing`。processing 同时检查题库状态和活动处理任务，覆盖重试只更新队列的情形。取消确认对已发布状态或 NAS 当前卷使用的条目返回 `LIBRARY_ITEM_IN_USE`。当前指针损坏返回 `LIBRARY_ITEM_NOT_CONFIRMABLE:current_paper_invalid`；可读 live NAS 优先，NAS 不可达时用本地已确认归档保守保护。

`list_library_items`、`get_library_item_summary` 返回 `confirmedAt`；`get_library_row` 的 V2 `item` 同样包含此字段。回收站行仍遵循现有 wrapper 合并约定。

带 `paper` 的发布在读取 canonical 的同一 SQLite 快照中检查全部卷内条目。缺确认返回 `PAPER_ITEM_NOT_CONFIRMED:<itemId>`，强制发布也不能绕过，且此时 NAS 目录、锁和 staging 均未创建。作者本地的内核文件锁覆盖确认写入、发布快照至发布结束以及整稿替换，关闭“校验后又被取消确认”的竞争窗口；进程退出自动释放锁。

| 写入路径 | 确认处理 |
| --- | --- |
| 改名、设 Part、普通编辑保存 | 保留；分别有回归断言 |
| 移入回收站、恢复 | 保留 |
| `processing/scheduler.rs → retry_job_at_root → queue::retry_with_cloud` | 仅重新排队，不替换权威稿；此时不清除，活动任务期间拒绝新确认 |
| scheduler 本地完成 → `ensure_initial_canonical` | 只填空，已有稿不覆盖；不误清已有确认 |
| `cloud_direct::write_direct_canonical[_leased]` | 成功整稿替换的 CAS/lease 事务内清除两列；冲突、失去 lease、失败回滚不清除 |
| 历史 `migration::repair_shadow_seed` | 实际 CAS 替换时清除两列 |
| `migration::save_refreshed_quality` | 仅刷新质量报告，保留 |
| `seed_canonical_ds` | 只对 NULL 稿播种；清空两列，避免新稿继承外壳的残留确认 |
| 普通云端采纳/修复 | 沿现有编辑事务；不将增量编辑误当作重新识别 |

## 结构补丁与撤销

| 补丁 | 字段及行为 | 逆补丁 |
| --- | --- | --- |
| `deleteAnswerSlot` | `taskId, responseGroupId, slotId`；`nodeId?, expression?`。删除注册表、答案、组引用、所有嵌入节点、热点、flow step 引用及独占逐题题干；组至少保留一槽 | `restoreStructure`，恢复全部被删内容、位置、绑定及热点 |
| `insertAnswerSlot` | `taskId, responseGroupId, slotIndex, slot, value, expression?`；节点模式还须 `node,target,index,parentId?`；逐题模式可用 `promptNode?` | `restoreStructure`，撤回实际新增路径 |
| `deleteResponseGroup` | `taskId, responseGroupId`；删除组内全部槽位和引用；只在无其他组引用时移除该选项库。若为大题最后一个作答组，一并删除该空大题 | `restoreStructure` |
| `deleteTaskGroup` | `taskId`；删除该大题及其槽位、答案、内容、选项和不再使用的库。其他大题保持原题号 | `restoreStructure` |
| `restoreStructure` | `taskIds: string[]`、`changes: [{ path: string[], before?, after? }]`；仅允许结构字段；缺失与 JSON null 有别；恢复前逐路径验证当前值仍等于 `after`，整条失败不留下半次撤销 | 同 op，交换每项 `before/after` |
| 既有 `deleteNode` | `nodeId`；题目区手动段落可删；普通删除仍不能绕过答案位保护 | 既有 `insertNode`，恢复原容器和位置 |

`expression` 省略时按保留/新增槽位的原始题号计算集合表达式，并同步 instruction signature。保留现有调用者显式传 expression 的兼容性；不自动给其余大题重编号。

逐题题干实际位于 `responseGroup.prompt` 顶层，通过 `answerSlot.hostNodeId` 关联，按 `perSlotPrompts.ts` 的全组一对一规则判断。只删除独占逐题题干；共用题干和原文段落保留。`passage.paragraphMap` 是段落标签到段落 ID 的映射，不是槽位映射；仅清理实际被删节点的映射与残留 host 绑定。

无节点插入支持列表型题目；已有逐题题干布局时自动生成 `<slotId>-prompt` / `<slotId>-prompt-text` 和“新题干”，也可传 `promptNode`。新子树的所有节点 ID 都检查内部及文档内唯一性。TS 类型对节点模式与逐题模式作互斥约束。

听力仅按实际题号的前后差集更新对应 Part，保留未识别题号的原始声明、taskIds 顺序、Part、音频和 cue。空 Part 可继续保存，完整卷的数量、缺题和音频门禁继续如实报告；不因删题删除受管音频。因此共享 authoring schema 中 Part 的 taskIds / expectedQuestionNumbers 允许空数组。

主要错误码：`AUTHORING_PATCH_TASK_NOT_FOUND:<id>`、`RESPONSE_GROUP_NOT_FOUND:<id>`、`SLOT_NOT_FOUND:<id>`、`SLOT_NODE_NOT_FOUND:<id>`、`SLOT_ALREADY_EXISTS:<id>`、`SLOT_NODE_MISMATCH`、`LAST_ANSWER_SLOT`、`PROMPT_INVALID`、`INDEX_INVALID:<index>`、`EXPRESSION_EMPTY`、`RESTORE_INVALID`、`RESTORE_CONFLICT`（除第一个外同样带 `AUTHORING_PATCH_` 前缀）。缺必填字段返回 `AUTHORING_PATCH_FIELD_REQUIRED:<field>`；既有 expression 校验错误保留。云端校核期间所有这些人工编辑/撤销沿用 `CLOUD_REVIEW_IN_PROGRESS:<itemId>`。

删除后的内部题号间隙以 `QUESTION_NUMBER_NONCONTIGUOUS` warning 报告。原卷明确声明但缺失的题号仍由既有 blocking 诊断处理；不把刻意保留的原卷编号改成自动连续编号。云端采纳按后端生成的删除槽位/题号及大题 ID 证据防止复活，覆盖云端重新生成 ID 的情况；用户撤销后的最终当前稿优先。模型仍不能直接调用删除/恢复补丁。

原仓没有独立的结构补丁 JSON schema，本次在 authoring schema 的 `$defs/authoringStructurePatch`、`$defs/authoringContentTarget` 增加定义并更新清单 SHA256。共享 fixture 同时由 Rust 和 TS 读取，逐步比较相同补丁序列的结果，并在 Rust 执行 TS 生成的逆补丁。

## 验证结果与证据层级

- `npx tsc --noEmit`：通过。
- `npx vitest run`：**74 files / 704 tests passed**（最终复验 7.34 秒）；包含新增 Rust/TS parity 的 TS 一侧、删除/撤销/重做、共享原文/选项库、嵌套节点、听力数据保留、ID 去重和三个结构动作的 hook 云端锁测试。
- 首轮 `cargo test --manifest-path src-tauri/Cargo.toml --lib`：**1615 passed / 3 failed / 19 ignored**，耗时 513.67 秒。两项云端多包失败来自最初将所有间隙当 blocking 的实现，已修正并在最终复验中通过。第三项 `phase4_eight_real_pdfs_preserve_instruction_zone_structure` 是七个私有样本的 V1 `blockCount` 基线漂移；例如 chili 50 对 56、conformity 51 对 59、fishbourne 42 对 49。本次未修改 V1 提取器或 golden 基线；此项保留为未解决的语料诊断，不能把整体 cargo test 宣称为全绿。
- 最终 Rust 复验使用下面命令，覆盖 library、authoring_v2、authoring_structure、publish_final、nas_package_v2、cloud_adoption、cloud_repair 及其余未过滤测试。六条过滤项已在首轮实际执行，除上述 V1 漂移外其余五条通过；为了不重复约 8.5 分钟的私有语料运行而过滤。最终结果：**1613 passed / 0 failed / 19 ignored / 6 filtered out**，执行 173.90 秒；main 单测与 doc-tests 均为 0 项且退出成功。

```powershell
cargo test --manifest-path src-tauri/Cargo.toml -- --skip phase4_eight_real_pdfs_preserve_instruction_zone_structure --skip part_detection_matches_expected_label_for_eight_real_pdfs --skip phase4_metrics_report_only_over_available_corpus --skip stage2_nine_private_pdfs_match_manual_task_presentation_and_anchor_expectations --skip nine_private_pdfs_ideal_candidate_aligns_against_real_text_layer --skip available_private_real_pdf_corpus_preserves_physical_layer_invariants
```

Rust 中原有的 19 个 ignored 保持原样。其中私有语料/外部配置有关的包括真实 DeepSeek 两条、chili/petri/八卷质量/真实 PDF 编辑导出四条及 chili 诊断、ReadingPractice geodiversity、Files 段落审计、旧 Files 两条回归和 live Files repair。其余为外部 live gateway、保留 job 的成本诊断或显式导出跨仓/Electron/听力验收夹具的测试。完整名字与原因见 `.planning/backend-confirm-structure-delete/cargo-final.log`。其中因私有语料、图像答案页或本地语料缺失而保持 ignored 的用例明确列为：

- `cloud_direct::real_model_tests::real_deepseek_listening_direct_vol7_t9`
- `ielts_grammar::real_pdf_acceptance::chili_group1_instruction_zone_truncation_diagnostic`
- `ielts_grammar::real_pdf_acceptance::chili_real_pdf_reaches_all_declared_acceptance_truth`
- `ielts_grammar::real_pdf_acceptance::petri_real_pdf_reaches_all_declared_acceptance_truth`
- `ielts_grammar::real_pdf_acceptance::phase4_eight_real_pdfs_reach_physical_authoring_quality_truth`
- `ielts_grammar::real_pdf_acceptance::phase5_real_pdf_edit_and_v2_export_round_trip`
- `pdf_geometry::tests::real_geodiversity_pages_keep_table_and_right_column_blocks_separate`
- `product_chain::product_chain_same_pdf_paragraph_corpus`
- `tests::files_pdf_samples_auto_pipeline_minimizes_artifacts_and_preserves_review_gate`
- `tests::files_pdf_samples_reach_expected_review_paths`
- `tests::live_llm_repair_contract_on_files_pdf_samples`（也需要外部 live provider 配置）

- `node scripts/verify-schema-contract.mjs --local-only`：通过，`errorCount: 0`。默认跨仓路径 `../NAS` 不存在；显式指定 `--peer-root F:/workspace/IELTS-NASfor-WenDao/developer/contracts/authoring` 后，结果为两个预期漂移：manifest 和 IeltsAuthoringIRV2 SHA256。**学生端镜像尚需同步**，本任务未写学生仓。
- `node scripts/e2e/build-app.mjs`：通过。最终 exe SHA256 `527c8e17bb2725a7b4af4de5f755bcf923060957591d3e3dbe1885899b33e751`，frontend→dist→exe 与 backend→exe 内容指纹均匹配。
- `node scripts/e2e/tauri-cdp-confirm-into-library.mjs`：**13 个断言全部通过**，真实 UI 确认、进入题库、重启、退回待确认，以及既有选项添加/删除。证据：[报告](../../artifacts/e2e-cdp/confirm-into-library-2026-11-11T12-47-28-856Z/report.json)。该脚本 `backendConfirmCommand` 仍是前端同事原先写死的“not-yet-implemented”文本；本次受修改范围约束未改脚本，这个旧文本不是实际命令探测结果。
- 补充 `node .planning/backend-confirm-structure-delete/tauri-structure-e2e.mjs`：通过。真实 IPC 返回非空 confirmedAt，localStorage 观察为空；删除 q1 和整道 task-2，保存成功；关闭并重启同一数据目录后，确认时间不变，q1/q5 已消失，q6 仍为 6；真实工作区学生预览渲染 q2/q3/q4/q6；最后真实 IPC 退回确认成功。证据：[报告](../../artifacts/e2e-cdp/backend-structure-2026-11-11T12-47-28-856Z/report.json)、[预览截图](../../artifacts/e2e-cdp/backend-structure-2026-11-11T12-47-28-856Z/03-student-preview-after-delete.png)。主代理已打开截图核验。

两条 Tauri 验证均为 WebView2 CDP 自动化通道，带诊断参数。确认入库流程是 UI 端到端证据；新结构删除为**真实命令调用 + 持久化 + 重启重开 + UI 学生预览**证据，未声称已通过尚不存在的删除按钮完成端到端操作。Rust 命令处理层与 TS/hook 单测另行分层，均不能替代 UI 证据。未运行独立学生 Electron 应用；验证的是作者端使用学生画布的预览。

## 前端接入与剩余事项

1. 启动时调用 `import_local_confirmed_ids`，成功后清空旧 `confirmedLibraryItemIds`；后端 accepted 数组只表明哪些仍存在且可确认，清空本地集合后以行上的 confirmedAt 为权威。当前页面/confirmedStore 已有的调用保持原样，本任务未修改。
2. `ExamCanvasStructureAction` 增加无 nodeId 的 `answer-slot.delete`、`response-group.delete`、`task-group.delete`，由 `structureActions.ts` 编译成上述补丁；列表型插入传 slot/slotIndex，无嵌入 node 时省略整套 target/index/node。建议不传 expression，由后端与 TS 镜像从保留题号计算。处理 LAST_ANSWER_SLOT 与 RESTORE_CONFLICT 的用户提示。
3. 学生仓同步 `ielts-authoring-ir-v2.schema.json` 与 `contract-manifest.json` 后运行其 `verify:authoring-schema-contract`。本地 authoring schema 与 TS/Rust 行为已一致，跨仓镜像仍未完成。
4. 历史仅有 published 状态却没有任何发布时间证据的行采用上述 updated_at 退回；若产品要求精确首次发布时间，需要补历史证据，不能从现有行凭空恢复。
5. 私有八卷旧 V1 blockCount 漂移待独立定位。没有为获得全绿而改写 golden 基线，也没有扩展到页面或 V1 识别器实现。

本次实现采用两个明确行为：最后作答组删除时一并移除空大题；听力删题保留 Part 音频与缺题声明。这些行为已有回归，后续页面接入需保持一致。
