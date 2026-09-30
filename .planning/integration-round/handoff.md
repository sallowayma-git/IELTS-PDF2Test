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
