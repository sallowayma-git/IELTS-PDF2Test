# 综合回归 / 云端为主采纳权重提升 —— 交接笔记

分支 `integration-round`（worktree F:\workspace\PDF2Test-integration，共享主仓库 target 联接）。
所有汇报只发协调 session，不发用户。合并 / push / 覆盖安装 / 清理均由协调方做，本 session 不做。

## 产品决定（协调方裁决）
云端结果基本正确，提高第一层完整候选权重：顺序对、文本大体相似即直接用云端覆盖；修复循环只处理
确有必要的目标（needsCloudReview 非空才有活）。小差异静默采纳，只有大差异的修复结果与仍无法确定项进待办。
架构选择 (b)：修复循环维持「不改 passage」边界；原文里的低相似句留复核清单作顾问级提示，不生成修复包。

## 已完成、已验证（Rust lib 全绿 1352+）、已提交
- 早期综合回归：候选新协议 6e56f03、单调判定顺序感知、schema unassignedReason、llm 建目录、页眉规则、A1/A2/A3。
- f257a59 放宽采纳门槛 + 按差异分流 + 两类扰动单测（门槛集中在 AlignmentConfig：句命中0.8、原文0.85、
  覆盖0.7、凭空句≤2且≤3%、题组≥0.8、顺序比0.95不变）。
- b22ebfd item4：修复循环任何一步的 unresolved 都进剩余任务 + 包模式单测。
- e2e 直读库只读 + busy_timeout（db 锁），产品侧确认采纳事务外做组装、无 >1s 持锁。
- 653959b 采纳复核清单两触发：answer_unresolved、answer_conflicts_answer_page + 单测。

## 关键架构发现
- cloud-repair-chain e2e 全程只跑一份 derived.candidate，需同时满足采纳类与机制类场景。
- 修复循环建包器按 task_group / part / document 分组（cloud_repair/packets.rs），**没有 passage 级包**。
- 夹具 demanding-reading-passage-3 派生候选天然带触发：group-2 instruction_stem_overlap、
  passage-paragraph-2 一句 ~0.55、q27/q28 未解析答案。

## Item 2（轴连接）—— 已完成，已提交 1b092b9
把修复循环目标接成「needsCloudReview（taskId/slotId 类）+ 采纳后阻断级质量问题」，而不仅是 candidate-vs-draft 差异。
- RepairRunRequest 加 `review_targets: &'a [Value]`；scheduler.rs:1192 处传入 needs_cloud_review（987 行在作用域）；
  tests.rs 的 request() 助手传 &[]。
- 在修复循环里 build_repair_context 产出 context 后，把 review_targets 中 taskId/slotId 类目标合并进
  context["differences"]（slot 映射到所属组，task_group 直接用），复用现有建包器。passage 级（passage_sentence_unverified）
  不生包（决定 b）。清单空 + 无采纳后阻断质量问题 ⇒ 零包零调用。
- 单测：空清单→0 包；overlap 目标→生包；未解析答案→生包。
- 完成后提交 + 回报。

## Item 3（机制场景改造）—— 待做
scripts/e2e/tauri-cdp-cloud-repair-chain.mjs，用夹具天然触发，不人为扰动：
- correction-and-evidence → 改用**题组级** content_not_aligned：选一个题组让其说明/题干与原卷第4页不符，
  断言修复请求带原卷原文、引文经校验并落库。（不再用 0.55 passage 句。）
- model-adjudicated → group-2 instruction_stem_overlap，模型基于真实反馈裁定，保留「≥2 轮工具调用」。
- packet-mode → q27/q28 未解析答案，其证据页不在首轮包；若首轮已带则换另一道证据页不在首轮的题。
- 采纳类场景继续断言：无触发项部分被整体采纳。不削弱断言。

## Item 4（干净候选第二阶段）—— 待做
同一 e2e 加第二阶段：小脚本从 derived.candidate 生成消毒版（去 group-2 overlap、把 0.55 句换成原卷原文、
用夹具真值补 q27/q28 答案），复用 controlled-service-restarted 步骤换成干净候选，把同一 PDF 作为新条目导入。
断言：整体采纳、needsCloudReview 空、repair_authoring_step 调用 0 次、无冲突类待办。
注意 collectTextNodes 需从 ./lib/cloud-repair-scenario.mjs 导入（当前 chain 脚本未导入）。

## 收尾
e2e 迭代上限 3 轮；第 3 轮仍不全绿就停下报卡点。全绿后重跑 cloud-repair-chain + heading-presentation，
附对照表回报。之后等协调方「已合并」再做覆盖安装与清理。

## 2026-09-30 P0–P3 收尾（最新）

- P0 a9c3d63：重复编辑不再靠 baseVersion 和重切伪装进展；错误/预算/无进展终态与解锁测试通过。heading 原运行实际早已 terminal，旧超时是 trace 文件名解析误报，ddadded 补齐请求格式并等待 processing 终态。
- P2 e9a5031：跨栏行文本混入右栏，原生 glyph charRange 的严格连续精确证据恢复原句 1.0；九卷表全部字段无回退。Q34 原卷印字未改。
- P1 f6271cd：单句 <0.6 独立按节点/槽位进 content_not_aligned，保留整组 <0.8；说明吞题检测以物理行为准。
- P3 81b9ffa：测试专用人工推断 q27–40 答案夹具、真实结构和来源角色恢复。8dfdd24 补修产品范围漏洞：清单空且无阻断质量问题时零修复调用、无本地快照冲突待办；阻断质量仍处理。
- 79c8ebc：旧 q40 页脚仍能对齐，按 Item 3 允许的无天然触发最小派生，仅一个题干改词序，真实原卷纠正路径通过。
- Rust lib 1367 passed/0 failed/15 既有 ignored；最终 chain 第三轮 18 passed/1 failed；heading 第二、三轮均 11/11 passed。回退 105.087s、干净 63.839s、heading 47.305s 进入 ready，lease 均释放。
- 唯一失败：group-2 本夹具无真实 instruction_stem_overlap；旧触发来自共享父区域。真实反馈/裁定断言保持，未制造额外扰动。这条机制在此真实应用夹具中尚未验证；发布遵循用户授权的「P0 修好、第三轮只剩夹具失败也出包」例外。
- 安装包 F:\workspace\PDF2Test-builds\IELTS-Author-Studio-0.1.0-79c8ebc-setup.exe，8,177,875 bytes，SHA256 E4788CDC441C66285ACA72FA27DFAC9D3AA7110E037BC2E37182D3BA4BDAB5CA；只生成/复制，未安装。
- 完整原因、时间线、先红、run 目录及证据分层见 closeout.md。合并、push、覆盖安装、清理由协调方处理。
