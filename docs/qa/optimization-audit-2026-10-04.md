# 实际产品优化审计：2026-10-04

基线为已推送的 `7713f5c`，审计开始时工作区干净。三个子代理并发审查界面与启动、导入与存储、模型与成本，主代理交叉核查结论。此轮只保存审计结果，没有修改产品代码，没有调用收费 API，没有新增安全、哈希、版本或审计框架。

## 结构与产品路径

- React 入口 `src/main.tsx` → `src/app/App.tsx`：题库、编辑工作区、设置三个页面。题库组合 jobs、旧库摘要、回收站与 V2 摘要；工作区从 canonical editor 读取权威稿，处理事件驱动状态与建议刷新。
- Tauri `src-tauri/src/lib.rs`：命令入口与启动调度。`processing/scheduler.rs` 分别管理本地与云端队列；本地管线经 PDF/DOCX 提取、文档层与题型解析生成稿件。
- `library/repository.rs` 中 SQLite canonical DS 是编辑权威；`library/commands.rs` 将编辑、质量重算与版本提交放入同一事务。文件 artifacts 保留原文、识别证据和派生产物。
- 云端完整候选入口在 scheduler → auto_pipeline → candidate_evidence；校核入口在 scheduler → cloud_repair → llm_gateway。上一轮已完成段落恢复、短批量裁决、上传副本去重和完整本地历史恢复。
- 发布从 canonical DS 编译运行时并导出 NAS 包。本文不把 CLI 或开发诊断的通过当作真实 UI 端到端证据。

## 建议按收益与实施成本排序

### 1. 列表查询只取摘要，不取整份题稿（先做，低成本）

`library/repository.rs:65` 的 `row_from` 读取 `canonical_ds_json` 为字符串，仅用于 `is_some()`；`:83` 的 ITEM_COLUMNS 与 `:132` 的 list_items 因而复制全库题稿，随后不向前端返回这些正文。首次 Part 判定另有按需全文读取，该读取仍有实际用途，应保留。

建议摘要查询投影 `canonical_ds_json IS NOT NULL` 并读取布尔值；真正打开或推断题稿仍使用现有全文接口。无需连接池、缓存服务或新 schema。

受控 SQL 测量使用现有 parser-complex-reading 稿件，压缩 JSON 为 128,468 字节/条，保持相同排序：

| 条目数 | 避免复制的全文 | 原查询中位数 | 摘要投影中位数 |
| ---: | ---: | ---: | ---: |
| 10 | 1.28 MB | 0.598 ms | 0.060 ms |
| 100 | 12.85 MB | 24.411 ms | 0.634 ms |
| 500 | 64.23 MB | 143.840 ms | 3.281 ms |

**边界**：这是 Python/sqlite 等价 SQL 诊断，不是 Rust 命令或原生界面耗时；当前用户真实题库较小，不能宣称现有首屏因此快了多少秒。实施后验收应测真实 command p50/p95、返回字节及列表内容一致性，保留 Part、回收站和 canonical existence 语义。

### 2. 单题进度不读取全库，题库事件不重复拉四份全量列表（随后做）

`ExamWorkspacePage.tsx:163` 每次处理事件调用 listLibraryItems，再 find 当前题；后端 `library/commands.rs:142` 遍历全库并对每行查询 processing 状态。请求与题库大小一起增长，而页面只需要一条状态。

真实 React 页面与 canonical hook 在 jsdom 中、隔离 IPC 测得：打开工作区调用全库列表 1 次；5 次同 editVersion 的进度事件之后累计 6 次。已有版本 guard 正确避免 canonical 重读，不能为了减少事件把这个 guard 或云端锁状态刷新破坏。

题库真实 `useLibraryStore` 的四源读取在 `libraryStore.ts:77`；首次加载及订阅完成各刷新一次，共 8 次请求，5 次独立事件之后累计 28 次。四源仍承担旧库、写作与回收站兼容，不能简单删除其中几份。

建议：工作区初次读取与后续刷新改为单项 processing；题库保持首次完整快照和 focus/manual refresh，但订阅完成不重复全量读取，后续只更新变化条目。事件与首次快照的交错、终态、取消与恢复必须验证。SQL 投影改善每次列表成本；单项刷新进一步去掉 O(N) 工作，二者互补。

**可测验收**：工作区事件不查询无关条目；题库首次四源各一次，五次单项事件的请求目标为 4+5=9 次，属于待实现的请求数目标，不能称已提速 68%。成本中等，主要在订阅竞态与四源行兼容，先做单题状态再做题库增量。

### 3. 减少误分类导致的重复 PDF 页输入（模型成本，低成本但需准确性对照）

`candidate_evidence.rs:36` 将数字开头且后续不超过 5 词的行视为答案行，例如 `1 Choose the correct answer.`；这会把题页加入每个 chunk 的共享辅助页。

同一合成回归 PDF `fixtures/parser/chunked-reading-evidence.pdf`，10 页、3 个已确认分段：当前共享页为 1、4、7、10，三个请求累计 20 页。离线窄规则只排除明确操作指令行作为“答案行”的原因，累计为 16 页；原有分段边界两侧、真正答案页、扫描页与未知边界回退仍保留。

交叉质疑否决了“含 section/Questions 的整页不共享”方案：题页可能同时含无标题答案或上一节答案。只逐行排除明确指令，页面存在其他未归类答案短行就继续共享。追加 `1 TRUE / 2 Arctic Ocean`、`14 B` 和显式 Answer key 的反例均保持共享。

**收益边界**：4/20 是该合成样例的重复附件页减少量，不能宣称真实 Token 或费用减少 20%。当前 page scoping 的真实 PDF 子集 → 本地 HTTP 网关 → 图像回退与原页码引文测试通过；16 页的规则尚未实现。后续用业务多段 PDF 对照附件页数、imageCount、promptTokens、缓存命中与逐题准确率，再决定推广。

### 4. 本地导入按需构造中间稿，并复用最终质量结果（中等优先）

`auto_pipeline.rs:4400` 无条件构造 answer_constraint_document，只有后面的云端 worker 结果使用；localOnly 没有该结果也构造了整稿。`:4617` 写最终 V2 稿后，`:4644` 为质量门再次从同一输入构建 V2 稿。

建议将答案约束稿构造移到确实有答案候选需校验的分支；最终写稿返回此次实际构造的质量结果给 gate 使用。复用时必须保持同一最终 IR、modality 和物理 evidence，不复用云端处理前的旧稿，不跳过质量计算或改变既有门禁。

默认 localOnly、QLG_DIRECT_CANONICAL=false 路径中，上述三次完整构造可目标缩为一次；启用 direct-canonical 的特例先保留其现有 gate 路径，不能直接套用旧 builder 结果。代码能证明存在不消费的构造和重复调用，但尚未分离 Rust 的各阶段耗时；因此不承诺导入节省百分比。成本低至中，验收应保持 PDF/DOCX 的最终稿、quality、pipeline report 与编辑重载一致，并测 build 次数与阶段耗时。

## 待测候选，暂不实施

- **完整 JobDetail 的进度读取**：真实页面同版本 5 次事件使 getJob 从 1 次增至 6 次。`job_commands.rs:207` 每次读取文档、作者稿、建议等完整 artifacts；source_review 又读取文档。工作区还两次请求完整 workspace，第二次只取 sourcePurged。可先复用已加载的 sourcePurged，再映射消费者是否适合元数据投影；真实资料变化仍要刷新。当前小样本不足以证明明显停顿，不直接推动全面懒加载。
- **保存事务内读取物理 shadow**：`library/commands.rs:128` 的 prepare 闭包在 `repository.rs:2099` 的 IMMEDIATE 事务内调用；`authoring_v2_commands.rs:1298` 重读整份 shadow 并重算质量。现有 2.49/4.78 MB 物理稿的 Python 读+解析中位数约 20.7/40.3 ms，仅说明有可能缩短锁持有时间，不是 Rust 保存延迟。先测真实锁时间和质量计算分布，不引入缓存失效框架，不改写事务正确性。

## 排除的优化

- schema 初始化已按进程/库文件去重，不再建议每次建表优化或换 ORM。
- legacy 启动迁移有 migration_done_v1 标记早返；首次迁移不能冒充每次启动的瓶颈。
- 题库两秒轮询已经删除；本文针对事件触发的全量读取。
- 主 JS 约 430 KiB，但未测冷启动 CPU/首屏；不仅凭包体积提议路由拆包。
- 长历史保持完整。本机 23 份受控日志最大仅 35,580 字节/4 条，Python 暖缓存全解析约 0.24 ms；不值得新增索引、压缩框架或丢弃历史。该样本不代表用户长期大日志；若真实几十 MB 日志导致恢复慢，再测并考虑一次读取复用。
- 不继续盲删首次候选题型规则；本地题型可能误判，需要原卷独立识别。也不把扩大 Token 上限或改变缓存预算口径称为成本优化。
- 不增加安全、哈希、审计与抽象框架，不为获得审计条目而重写架构。

## 验证与下一步

真实 React 组件/hooks 的两项 IPC 次数诊断通过；产品核心 PDF 导入/编辑/重载、DOCX 导入生成物理文档层的定向回归通过；当前 PDF scoping 受控 HTTP 网关回归通过。具体笔记与诊断产物在 `tmp/audit-startup-2026-10-04.md`、`tmp/audit-storage-2026-10-04.md`、`tmp/audit-storage-measure-2026-10-04.json`、`tmp/audit-model-2026-10-04.md`。

定向用例：

- `product_chain::product_chain_pdf_import_reaches_editable_session_and_persists_one_character_edit`：通过，2.21 s。
- `product_chain::product_chain_docx_import_materializes_the_physical_document_ir_v2`：通过，0.53 s。
- `llm_gateway::tests::scoped_pdf_crosses_real_gateway_and_image_fallback_with_original_citations`：通过，4.29 s；当前三个请求的整卷累计 30 页、现有 scoping 累计 20 页。

测试总运行时间不是用户操作延迟，也不是任何提案实施后的性能提升。

原生 UI 自动化服务启动失败，未获得实际 Tauri 点击到首屏、进度到横幅或保存延迟数据。真实收费提供商 Token 与准确率也未测。所有数值已标明证据层级，不能用这些下层测试代替实际产品端到端验收。

建议下一轮先实施摘要 SQL 投影和工作区单项状态读取，再处理重复初次刷新与窄 PDF 页分类；重复构造在取得阶段计时后跟进。以同一批 PDF、同一题库规模对照结果，保留完整本地历史和当前正确行为。本报告完成后不自动扩大实施范围。
