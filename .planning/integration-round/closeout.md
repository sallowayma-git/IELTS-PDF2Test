# 综合回归收尾记录

## 修改及先红证据

- 511a9b3：提交三份任务记录。
- P0 a9c3d63：cloud_repair/mod.rs、tests.rs、processing/scheduler.rs、cloud-repair-chain 与 heading-presentation 脚本。重复指纹剔除 CAS baseVersion，并跨重切保留；旧版本被拒的编辑允许刷新版本再试；无进展及用尽预算的包不再重排。先红：packet_loop_stops_when_the_same_edit_reappears_after_replanning 实际 5 次调用，违反 <=2。修复专项 81 passed。错误落终态并释放编辑锁测试已在全量首轮通过。
- P2 e9a5031：reconcile/alignment.rs。重建物理行混入右栏，原生 glyph charRange 保存正确顺序；新增严格连续、精确匹配的 PDF 字符证据回退，不改阈值。模拟与真实 PDF 回归先红均为 0.55；原文 wanting crass protagonists 保留。新增物理行集合供说明吞题检测。
- P1 f6271cd：cloud_adoption.rs、cloud_repair/mod.rs。任一说明/题干/选项节点 min_similarity <0.6 独立进入 content_not_aligned，保留组比例门禁；单题带 slotId，并优先经槽位映射所属组。父区域共享不代表吞题，比较真实物理行。两个逐句复核测试先红因清单为空；采纳专项 11 passed。
- P3 81b9ffa：clean-authoring-candidate.mjs、chain 脚本、test-inferred-answers.json。按原卷恢复 group-2 说明和 q32–35 题干并转移来源锚点，补 q27–40 全部人工推断测试答案。此前真实运行干净阶段因缺答案/对齐/overlap 失败；所有断言保留，待新产品运行验证。

## P0 原运行时间线

不合格候选回退：run-cloud-repair-chain-2026-09-30T19-03-33-551Z，import-20260930190712-d1a4ba0a。

| UTC | 证据 |
| --- | --- |
| 19:07:12.535796 | processing job 创建 |
| 19:08:07.381919 | 候选请求，HTTP 200、一次尝试 |
| 19:08:30.057241 | 首次修复调用 |
| 19:12:54.104780 | 最后观测修复调用；共 18 个修复请求，全部一次尝试 HTTP 200 |
| 19:12:54.069853 | DB 仍 reconciling/cloud running；从导入 341.534 秒，lease 未释放 |

缓存步骤 8–19 是同一 setResponseGroup，group-3、相同内容和证据，仅 baseVersion 1–12。指纹包含版本，且重切每次清空重复计数，导致无效编辑被视为进展。不是提供方重试。repair_json running，rounds17/applied11；没有最终 repair 文件。

heading：run-heading-presentation-2026-09-30T19-13-22-564Z，import-20260930191357-02586bdb，实际 52.929 秒进入 ready_for_review/cloud partial/reconcile succeeded，lease 已释放；repair needs_attention、3 包完成。超时来自脚本缓存文件名正则遗漏 -000001 后缀。预导入 36.274 秒 ready/cloud failed，亦无 lease。

SQLite 只留当前 processing row，无阶段历史表；时间线由创建/更新、调用时间和批次摘要重建。读取均只读、窄字段投影，不读取密钥。

## 验证进度

- P1 先红 0 passed/2 failed；P2 先红 0 passed/2 failed。
- 九卷修复前表：9 卷命中全部句子，覆盖 0.99–1.00，长度比 1.00–1.01，顺序比 1.00。
- 第一次全量构建 LNK1104 测试 EXE 无法打开；检查未见进程仍在，未见系统杀进程证据。
- 随后全量 1362 passed/2 failed/15 ignored（既有 ignored 未新增）：CAS 重试回归与上限原因优先级，新实现已修正；81 项修复专项通过。
- 前端产品源码未修改；脚本 node --check 通过。最终全量、真实应用 E2E、安装包待完成。

## 第一轮产品回归新增发现

- 最初最终 Rust：1365 passed/0 failed/15 既有 ignored。build-app 仅运行一次，日志目录 artifacts/build-logs/2026-09-30T21-12-29-871Z，EXE f859e255fc8433feaa520d71cdc5d0b14ef313a08b844914a7a0411386ce177c，输入无漂移。
- chain 第一轮 run-cloud-repair-chain-2026-09-30T21-14-13-452Z：15 passed/4 failed。真实 UI 编辑锁解除、保存重开、缺页同包 L1、导出与真实学生运行通过。
- 回退任务 import-20260930211711-1fddb7a0 于 21:17:11.602657 创建，21:18:39.089603 ready_for_review/cloud partial/lease NULL，87.487 秒完成。失败断言读取了修复摘要完成后、答案页步骤完成前的 reconciling 行；增加同一 180 秒期限内等待 processing job 终态，未增加期限。
- 两个机制触发前提失效：q40 的 BLANK PAGE 来自原卷页脚，不是低于 0.6 的凭空句；group-2 原说明文字已正确，旧 overlap 来自共享父区域。原卷页/引文/正确题面落库断言通过，但 content_not_aligned、真实 overlap 反馈及其裁定缺失。保持门禁与断言，不伪造天然触发。
- 干净阶段 needsCloudReview 已空且整体采纳，但 7 个 repair 请求、17 冲突待办。产品根因：建包及 remaining_tasks 仍把已采纳稿与本地快照全部差异当争议。增加复核所属题组范围过滤；原生阻断质量问题继续独立建包。新增 clean 零调用和阻断不可隐藏测试。首红为该真实产品运行；单测首跑遇到第二次 LNK1104，随后正常编译通过，不把链接错误当单测先红。
- heading 第一轮 run-heading-presentation-2026-09-30T21-20-41-644Z：终态等待已过，但 candidateTrace 的 input 文件名解析也遗漏序号后缀；补齐同一格式。

## 补充提交与第二轮

- 8dfdd24：采纳后仅校核复核所属范围；mod.rs、packets.rs、tests.rs。真实先红是第一轮干净候选的 7 次调用/17 待办。修复后 Rust lib 1367 passed/0 failed/15 既有 ignored；新增空清单零调用、阻断质量仍建包两项测试均通过。
- ddadded：chain 等待 processing 终态与 heading 请求文件名序号兼容。保留 180 秒回退修复期限。先红是第一轮脚本读取过早与 heading 缺请求痕迹误报。
- 后端因新增真实产品修复重编译，未第二次执行 build-app；复用其前端 dist，按同一清单算法捕获构建前输入/构建后内容并检查漂移。新 debug EXE aad835f663312346569c653be53a5f1f0f769636d536ac0a3379712a0a72feb7，后台输入 b3c73b60cd57766464403c4130f9c34b85e527e70cf491d87d403c269d8ec770，无漂移。
- 第二轮 chain：run-cloud-repair-chain-2026-09-30T21-33-21-322Z，17 passed/2 failed。回退 import-20260930213655-6dc79a5f 129.632 秒 ready_for_review/cloud partial/lease NULL；干净 import-20260930213932-241470c7 64.964 秒 ready/cloud succeeded/lease NULL，完整采纳、needsCloudReview=[]、repair 请求 0、冲突待办 0。
- 第二轮 heading：run-heading-presentation-2026-09-30T21-41-22-531Z，passed。实际题目 import-20260930214152-d7ad9220 48.180 秒 ready/cloud partial/lease NULL；预导入 30.216 秒 ready/cloud failed/lease NULL。
- 79c8ebc：按交接笔记 Item 3 在无天然低相似触发时允许单组最小改动，仅派生 q40 一个题干的词序；原卷改后题面/引文仍从原卷提取，干净种子在此变更之前复制。先红是前两轮 content_not_aligned 前提缺失。group-2 不额外人为扰动，原断言保留；第三轮正在运行。

## 第三轮 chain 结果及发布条件

- run-cloud-repair-chain-2026-09-30T21-43-21-635Z：18 passed/1 failed；不再迭代 chain。
- content_not_aligned 单题派生触发成功（实际复核节点 similarity=0.285714），原卷第 4 页请求、引文核验及正确题面落库均通过；同组其他内容仍采纳。缺页同包抓取并升 L1、编辑锁、保存重开、导出及真实学生运行全部通过。
- 回退 import-20260930214632-cca74f0a：21:46:32.151809 → 21:48:17.239297，105.087 秒 ready_for_review/cloud partial/lease NULL。
- 干净 import-20260930214844-4a5892ac：21:48:44.319232 → 21:49:48.158128，63.839 秒 ready/cloud succeeded/lease NULL；整份采纳，复核 0、repair 调用 0、冲突待办 0。
- 唯一失败 model-adjudicated-adopted-cloud-from-real-feedback：原说明文本正确，精确物理行无交集；旧触发来自父区域共享。请求自然没有 instruction_stem_overlap，故无对应 current_is_correct 裁定。断言（包括 >=2 工具调用）未删；此特定机制意图尚未在真实应用中验证。属于夹具触发前提不足，不是产品挂起；不凭空制造 group-2 扰动。
- 用户明确允许 P0 修好且第三轮只剩测试夹具失败时出包。最终 heading 回归与 NSIS 待完成；不合并、不 push、不安装。

## 最终交付

- 最终 heading：run-heading-presentation-2026-09-30T21-51-02-517Z，11/11 passed；import-20260930215131-c324fb71 47.305 秒 ready_for_review/cloud partial/lease NULL。
- NSIS 仅打一个包，CARGO_TARGET_DIR 指向主仓库共享 target、CARGO_BUILD_JOBS=2；release 编译 4m36s，构建成功。构建开始 2026-09-30 21:55:14 UTC，本次安装包原文件更新于 22:00:07.2710755 UTC。未见系统杀进程。
- 原文件：F:\workspace\PDF2Test\src-tauri\target\release\bundle\nsis\IELTS Author Studio_0.1.0_x64-setup.exe。
- 交付：F:\workspace\PDF2Test-builds\IELTS-Author-Studio-0.1.0-79c8ebc-setup.exe。
- 大小：8,177,875 bytes；SHA256：E4788CDC441C66285ACA72FA27DFAC9D3AA7110E037BC2E37182D3BA4BDAB5CA；对应提交 79c8ebc。后续收尾提交只改规划/交接文档，不改包对应代码。
- Rust lib 最终 1367/0/15（passed/failed/既有 ignored）；九卷表无回退；本轮前端源码未改，build-app 前端 tsc/vite 通过，Vitest 本轮未重跑，上一轮 46 文件/525 测试通过。
- 其余前轮 E2E 未重跑：tfng-layout、option-drag（1 round）、edit-save-stress、ui-audit、nas-contract；相关实现未改。上一轮路径见 progress.md；NAS-contract 13/13 属于前轮记录，参考 publish-ready 报告在本 worktree 缺失。
- 证据分层：真实 Tauri+受控 HTTP 服务验证导入/锁/编辑保存/预览刷新/导出及学生运行；命令层提供可归因构建、NSIS、脚本语法和只读 DB 时间线；单测覆盖错误终态/解锁、预算、重复编辑、逐句清单、精确 PDF 对齐、零调用与阻断保护。
- 未验证：本夹具 group-2 真实 overlap 的模型裁定；无 answerErrors 标注的额外答案纠错派生场景；真实外部云供应商故障的 UI 注入（错误路径以测试覆盖）；安装/升级覆盖（按要求未安装）。未放宽门禁、未新增 ignore、未吞新错误、未接触密钥；不合并、不 push、不 stash、不清理。
