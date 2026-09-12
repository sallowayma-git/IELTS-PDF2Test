# A5 审计报告 — 第 6 章（本地 V2 几何识别重构）

- 审计对象：`Plan With Files/IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md` 第 690–1021 行（§6.1–§6.11）
- 审计日期：2026-09-12
- 审计立场：默认每条论断为错/过时/未完成，用当前代码证据证伪
- 实际审计基线：`git rev-parse HEAD` = `47a3806`（`feat(m1): canonical DS repository, transactional editing, typed preflight`），工作树含大量未提交改动（`src-tauri/src/processing/`、修复轮改动、M4/P4-T01 起步改动）
- 约束遵守：只读审计，未修改任何产品代码/配置；未运行 `cargo build` / `cargo test` / `npm run build`
- 关键前置事实：**`src-tauri/src/` 下不存在 `recognition/` 目录，全仓也不存在 `recognition/` 目录**（`find . -type d -name recognition -not -path "*/node_modules/*"` → 空）；本地识别仍散落在 `ielts_grammar/`、`pdf_ingest/`、`parser.rs`、`authoring_pipeline.rs`、`pdf_geometry.rs`

核心结论：**第 6 章基本是"目标设计"，不是"当前实现"。§6.2–§6.7、§6.10 的具名类型/函数在代码中一个都不存在；实际生效的是 `ielts_grammar/` 里一套基于 V1 `questionGroupCandidates` + `SemanticLine` 行序的启发式识别。§6.1 的"当前断点"是唯一完全成立的章节。§6.8 的硬闭包只在前端 `actionableIssues.ts` 实现，后端无 `validate_basic_task`，且 issue code 命名三方（计划 / 后端 `issue_codes.rs` / 前端）互不一致。§18 Phase 4 的五项量化验收指标没有任何度量实现——`fixtures/golden/metrics.json` 定义了指标却无任何消费者，两个 `verify-phase4-*.mjs` 都不度量这些阈值。**

---

## 章节范围

- §6.1 当前断点（第 692–699 行）：4 条断点断言
- §6.2 Question Layout Graph（第 701–727 行）：`QuestionLayoutGraphV1` / `QuestionBlockCandidateV1` 及 7 个配套类型
- §6.3 数据处理顺序（第 729–744 行）：10 步流水线 + 一条顺序约束
- §6.4 页角色与区域角色（第 746–777 行）：`SemanticRegionRole` 枚举 + 11 项打分特征 + "不修改原始 facts"
- §6.5 题号识别（第 779–811 行）：`detect_question_number_tokens` / `score_number_token` + 8 项加权规则 + 阈值 0.55
- §6.6 题干扩展（第 813–864 行）：`assemble_question_stem` + 5 种形态
- §6.7 选项识别（第 866–899 行）：`OptionLabelToken` / `assemble_option` + 5 项合法 run 证据
- §6.8 简单题型硬闭包（第 901–937 行）：`validate_basic_task` + 7 个 issue code
- §6.9 Matching Headings 与 passage A-G（第 939–959 行）：`passage.paragraphMap` / `task.optionBank` / response groups 三分 + 分类条件
- §6.10 表格/流程图/图片（第 961–1004 行）：`compile_table_stimulus` / hybrid hotspot `normalizedRect`
- §6.11 未分配证据账本（第 1006–1021 行）：7 种 disposition 标签 + 大面积 unassigned 阻断
- 交叉核对：§17.7 模块映射（第 2832–2855 行）、§8.5 `ActionableIssueV1`（第 1479–1496 行）、§18 Phase 4 验收指标（第 3307–3313、3878–3880 行）、§24 DoD 本地识别（第 3917–3922 行）

---

## 断言核对表

判定口径：`HOLDS` = 当前代码支持；`PARTIAL` = 部分成立或成立但描述已偏移；`FALSE` = 不实/类型或函数不存在；`UNVERIFIABLE` = 静态证据不足。

### §6.1 当前断点

| # | 断言 | 判定 | 证据 | 说明 |
|---|---|---|---|---|
| 6.1-1 | "`ielts_grammar` 仍然从 V1 `questionGroupCandidates` 开始" | **HOLDS** | `src-tauri/src/ielts_grammar/mod.rs:134-139` 直接 `for ... in split.get("questionGroupCandidates")`；`src-tauri/src/auto_pipeline.rs:202,206,208` `make_dynamic_split_candidates` → `make_dynamic_authoring_ir` → `build_authoring_v2_shadow` | 主链确为 V1 split → V1 IR → V2 shadow；`DocumentIRV2` 仅是辅助输入 |
| 6.1-2 | "把 physical layer 主要转为 `SemanticLine`" | **HOLDS** | `src-tauri/src/ielts_grammar/instruction_zone.rs` `semantic_lines_from_v2_shadow` 只遍历 `pages[].lines[]`，输出 `SemanticLine{id,text,page_index,order,role:"",bbox}` | 确为"物理层→一维行序" |
| 6.1-3 | "table/region/vector 不能直接成为题面结构" | **HOLDS** | 同上函数不读 `pages[].tables[]` / `regions[]` / vector；`build_stimulus`（`mod.rs:1521-1587`）只接受 `SemanticLine` | 物理表格/区域被丢弃 |
| 6.1-4 | "题号独立行或选项正文折行时字符串规则容易失败" / "V1 candidate 错误限制 V2 搜索范围" | **HOLDS** | `mod.rs:134-139` 搜索域由 V1 candidate 决定；`prompt_assembler.rs:51-64` 仅在 `anchor.line_index..next.line_index` 行区间内取文本 | 描述属实 |

### §6.2 Question Layout Graph

| # | 断言 | 判定 | 证据 | 说明 |
|---|---|---|---|---|
| 6.2-1 | `QuestionLayoutGraphV1` 存在 | **FALSE** | 全仓 `grep -rl QuestionLayoutGraph src-tauri/src src` = 0 | 类型不存在 |
| 6.2-2 | `QuestionBlockCandidateV1` 存在 | **FALSE** | 同上 = 0；`number_anchor` / `stem_node_ids` / `boundary_confidence` 均 = 0 | 6 个字段无一存在 |
| 6.2-3 | `OptionBankCandidateV1` / `VisualStimulusCandidateV1` / `InstructionZoneCandidate` / `UnassignedEvidence` 存在 | **FALSE** | 四者 grep 均 = 0。最接近的真实类型是 `OptionBankCandidate`（**无 V1 后缀**，`src-tauri/src/ielts_grammar/option_bank.rs:7`）与 `InstructionZone`（`src-tauri/src/ielts_grammar/instruction_zone.rs:19`），二者都不是图模型节点 | 命名近似但结构完全不同 |
| 6.2-4 | `SourceAnchorV2` 作为锚点类型 | **PARTIAL** | `src-tauri/src/schema/common.rs:67` 存在 `SourceAnchorV2` | 字段类型存在，但容器类型不存在，不能算该结构已落地 |

### §6.3 数据处理顺序（10 步）

| # | 步骤 | 判定 | 对应实现位置 | 说明 |
|---|---|---|---|---|
| 6.3-1 | `DocumentIRV2` | **HOLDS** | `src-tauri/src/pdf_facts_shadow.rs`、`src-tauri/src/schema/document_ir_v2.rs` | 物理层物化存在（PDF + M4 新增 DOCX） |
| 6.3-2 | page role segmentation | **FALSE** | 生产代码无 `PageRole` 枚举（grep = 0）；`pageRole` 仅出现在验收测试元数据 `src-tauri/src/ielts_grammar/real_pdf_acceptance.rs:291,892,988,...` | 无页角色分割算法，只有测试期望值 |
| 6.3-3 | instruction zone detection | **PARTIAL** | `src-tauri/src/ielts_grammar/instruction_zone.rs` `collect_instruction_zone` | 存在，但输入是 `SemanticLine` + V1 heading_index（`mod.rs:190`），非几何区域 |
| 6.3-4 | question number token detection | **PARTIAL** | `src-tauri/src/ielts_grammar/anchors.rs:14` `detect_question_anchors`（整行开头解析）；`question_number.rs` 解析题号表达式 | 是 line-first，不是计划的 token/geometry-first |
| 6.3-5 | geometric question-block expansion | **FALSE** | 无 `QuestionLayoutGraph` / `geometric_interval` | 完全缺失 |
| 6.3-6 | local option-run detection | **PARTIAL** | `src-tauri/src/ielts_grammar/option_run.rs:20` `detect_option_runs` | 行相邻启发式，非几何 |
| 6.3-7 | shared option-bank detection | **PARTIAL** | `src-tauri/src/ielts_grammar/option_bank.rs:15` `detect_option_bank` | 存在，基于 instruction 文本 marker |
| 6.3-8 | task semantic classification | **PARTIAL（且顺序被违反）** | `mod.rs:196-200` 传 `kind_hint = candidate.get("kindHint")`（**来自 V1**）给 `infer_instruction_signature` | 分类由 V1 kindHint + 指令文本驱动，发生在几何边界恢复之前/之外 |
| 6.3-9 | ContentDoc/TaskGroup compilation | **PARTIAL** | `mod.rs:76` `build_authoring_v2_shadow` | 存在，但入口是 V1 candidate |
| 6.3-10 | hard completeness validation | **PARTIAL** | `src-tauri/src/ielts_grammar/quality.rs` `evaluate_quality` + `issue_codes.rs` | 存在，但 code 命名与 §6.8 不一致（见 6.8 行） |
| 6.3-约束 | "题型分类不得先于题面边界恢复" | **FALSE** | `mod.rs:196-200` | 现实现正是先拿 V1 kindHint 分类，几何边界恢复根本不存在 |

### §6.4 页角色与区域角色

| # | 断言 | 判定 | 证据 | 说明 |
|---|---|---|---|---|
| 6.4-1 | `SemanticRegionRole` 枚举（9 变体）存在 | **FALSE** | `grep -rl SemanticRegionRole src-tauri/src src` = 0；无 `region_role` / `semantic_role`（均 = 0） | 枚举不存在 |
| 6.4-2 | 11 项区域角色打分特征（instruction signature、Questions N-M、font/bold/spacing、x/y、题号密度、选项序列、A-G、页眉页脚位置、页码位置、nearby table/figure） | **FALSE** | 无区域角色打分器。相关的零散能力分属其它用途：`instruction_signature.rs`（指令签名，非区域角色）、`quality.rs:3461`（仅按 region `kind` 判 header/footer/page_number）、`completion.rs:702-758`（bbox 行合并，用于 completion 行） | 计划列出的 11 项无一作为"区域角色打分"实现 |
| 6.4-3 | "不修改原始 facts" | **HOLDS** | `src-tauri/src/ielts_grammar/mod.rs:1-7` 模块文档明写 "It never writes either V1 artifact"；grammar 只写 shadow artifact | 该约束被遵守 |
| 6.4-4 | 角色推断（而非透传） | **FALSE** | `instruction_zone.rs:150` 只 `block.get("roleHint")` 透传 V1 提供的 roleHint（来源 `parser.rs`/`authoring_pipeline.rs`） | 无新推断，仅转发 V1 标签 |

### §6.5 题号识别 token/geometry-first

| # | 断言 | 判定 | 证据 | 说明 |
|---|---|---|---|---|
| 6.5-1 | `detect_question_number_tokens(page, expected_range)` 存在 | **FALSE** | grep = 0 | 函数不存在 |
| 6.5-2 | `score_number_token` 存在 | **FALSE** | grep = 0 | 函数不存在 |
| 6.5-3 | 加权规则 +0.30/+0.20/+0.15/+0.10/+0.10，-0.30/-0.25/-0.20 | **FALSE** | `src-tauri/src/ielts_grammar/anchors.rs:81,84,87` 实际是**固定分值**：裸数字 `0.62`、带 `.`/`)`/`:` `0.9`、其它 `0.72`，再 `+0.1 if has_prompt`（`anchors.rs:31`）。无任何加/减权项 | 权重体系完全不同 |
| 6.5-4 | 阈值 `score >= 0.55` | **FALSE** | `anchors.rs` 无阈值过滤，仅 `expected_numbers.contains(&number)`（`:24`） | 阈值不存在 |
| 6.5-5 | `inside_header_footer` / `looks_like_year_or_measurement` 过滤 | **FALSE** | `ielts_grammar/`、`pdf_ingest/` 内均无此过滤；`header/footer/page_number` 只在 `quality.rs:3461` 按 region kind 使用 | 页脚/年份误检防护在题号层不存在（仅靠 expected_numbers 范围兜底） |
| 6.5-6 | "当前 anchor 主要从整行开头解析数字" | **HOLDS** | `anchors.rs:59-90` `parse_leading_question_number` | 对现状的描述属实 |

### §6.6 题干扩展算法

| # | 断言 | 判定 | 证据 | 说明 |
|---|---|---|---|---|
| 6.6-1 | `assemble_question_stem(number, next_number, graph)` 存在 | **FALSE** | grep = 0 | 函数不存在 |
| 6.6-2 | `geometric_interval` / `text_nodes_right_of` / `within_baseline_tolerance` / `text_lines_below` / `hanging_indent` / `vertical_gap_below_threshold` | **FALSE** | 全在 `ielts_grammar/` grep = 0 | 无几何 API |
| 6.6-3 | 现状等价物 | **PARTIAL** | `src-tauri/src/ielts_grammar/prompt_assembler.rs:16` `assemble_prompt` 仅按 `anchor.line_index..next.line_index` 行区间拼接（`:51-64`） | 纯行序，无几何 |
| 6.6-4 | 支持"同行 / 题号独立一行 / 折行 / 大缩进 / 跨页" 5 种形态 | **PARTIAL** | 题号独立一行：`anchors.rs:74-82`（裸数字成 anchor）+ 行区间拼接可覆盖；跨页：因 `semantic_lines_from_v2_shadow` 按页顺序展开，行区间**可能**偶然跨页；**折行靠"行区间内所有行拼接"顺带覆盖，但会误吞指令/相邻正文**；**大缩进、baseline 对齐、vertical gap 无任何判断** | 无一种形态是按几何条件显式实现的 |
| 6.6-5 | `StemCandidate{text,node_ids,coverage,boundary_confidence}` | **FALSE** | grep = 0 | 返回类型不存在 |

### §6.7 选项识别

| # | 断言 | 判定 | 证据 | 说明 |
|---|---|---|---|---|
| 6.7-1 | `OptionLabelToken{label,bbox,node_id}` 存在 | **FALSE** | grep = 0 | 类型不存在 |
| 6.7-2 | `assemble_option(label,next_label,block)` 存在 | **FALSE** | grep = 0 | 函数不存在 |
| 6.7-3 | 悬挂缩进续行收集（`hanging_indent_matches`） | **FALSE** | `src-tauri/src/ielts_grammar/option_run.rs:20-62` 仅在 label 正文为空时吸收**紧邻下一行**（`:30-46`），不按 x 列/悬挂缩进；无 bbox 使用 | 无悬挂缩进逻辑 |
| 6.7-4 | 现状等价物 | **PARTIAL** | `option_run.rs:146-175` `parse_option_line`（字母或小写罗马 label）+ `advances_option_sequence`（`:177-184`）+ `materialize_run`（`:186-204`） | 行序启发式 |
| 6.7-5 | label 连续 / x 列稳定 / 非空正文 / 同 region 证据 | **PARTIAL** | 连续：`option_run.rs:187-194`；非空：`:192-194`（空则 `incomplete`）；**x 列稳定、同 region 无实现** | 5 项证据实现 2 项 |
| 6.7-6 | "不把 `Paragraph A`、文章首字母 A、Section B 当作选项" 有防护 | **PARTIAL** | 有测试 `option_run.rs:296-300`（单条 "A study of western celebrity" → run `incomplete`）；`run_matches_alphabet`（`:114-125`）要求完整且匹配字母表。**但没有显式 "Paragraph A" / "Section B" 排除**：`parse_option_line` 仍把 "A ..." 解析为 label=A 的 option（只是标记 incomplete），边界误判风险靠下游 `run_matches_alphabet` 兜底 | 无计划所述显式防护 |

### §6.8 简单题型硬闭包

| # | 断言 | 判定 | 证据 | 说明 |
|---|---|---|---|---|
| 6.8-1 | 后端 `validate_basic_task(task)` 存在 | **FALSE** | grep = 0；`src-tauri/src/ielts_grammar/issue_codes.rs` 无该函数 | 后端硬闭包函数不存在 |
| 6.8-2 | 7 个 issue code 在**后端**实现 | **PARTIAL** | 后端实际使用不同命名：`issue_codes.rs:19 PROMPT_EMPTY`、`:20 PROMPT_BOUNDARY_AMBIGUOUS`、`:21 OPTION_RUN_INCOMPLETE`、`:23 OPTION_BANK_MISSING`、`:53 SIGNIFICANT_REGION_UNASSIGNED`；另有 `authoring_review.rs:445 QUESTION_PROMPT_MISSING`、`:457 OPTION_RUN_INCOMPLETE`（另一条 review 链，非 `validate_basic_task`） | 计划的 7 个 code 与后端命名**不一致** |
| 6.8-3 | 硬闭包实际实现位置 | **前端** | `src/features/editor/actionableIssues.ts:11-17` 定义 `IssueCode`；`:122 QUESTION_PROMPT_MISSING`、`:85 OPTION_TEXT_MISSING`、`:74 OPTION_RUN_INCOMPLETE`、`:98 SHARED_OPTION_BANK_MISSING`（另加 `ANSWER_MISSING`/`ANSWER_UNRESOLVED`）。**缺** `QUESTION_PROMPT_BOUNDARY_AMBIGUOUS`、`OPTION_LABEL_MISSING`、`SIGNIFICANT_SOURCE_TEXT_UNASSIGNED` | **"后端未实现硬闭包"确认成立**：`deriveActionableIssues` 在 `src/features/editor/actionableIssues.ts:105`，纯前端派生 |
| 6.8-4 | `require_question_source_coverage(0.92)` / `require_unique_option_labels` / `require_exact_fixed_response_set` | **FALSE** | `actionableIssues.ts` 无覆盖度、无重复 label 检查、无固定应答集检查 | 计划闭包项前端也未全实现 |
| 6.8-5 | "下列问题直接阻止 Ready" | **PARTIAL** | 前端 `blockerCount`（`actionableIssues.ts:164`）可用于阻断；后端 `quality.rs:146-159` 以 `SIGNIFICANT_REGION_UNASSIGNED` 阻断 | 阻断机制存在，但 code 集合与触发条件与计划不符 |

### §6.9 Matching Headings 与 passage A-G

| # | 断言 | 判定 | 证据 | 说明 |
|---|---|---|---|---|
| 6.9-1 | `passage.paragraphMap` 承接 Paragraph A-G | **FALSE** | `src-tauri/src/ielts_grammar/mod.rs:535` 硬编码 `"paragraphMap": {}`；`build_passage`（`:383-537`）无 A-G 抽取 | 恒为空对象 |
| 6.9-2 | `List of Headings i-x` → `task.optionBank` | **PARTIAL** | `option_bank.rs:15` `detect_option_bank` 识别 "list of headings" marker；测试 `option_bank.rs:333` | 存在，但非按 §6.9 的三段显式分离 |
| 6.9-3 | 分类条件 `has_list_of_headings_title && has_roman_option_run && has_paragraph_targets` | **FALSE** | `instruction_signature.rs:169` 仅凭指令文本返回 `MatchingHeadings`；`:287` 用 `lower.contains("roman")||contains("list of headings")`；**无 `has_paragraph_targets`** | 三元几何条件不存在 |
| 6.9-4 | "被 `option_bank_candidate` 消费的 region 不得进入 passage" | **FALSE** | `build_passage` 按 V1 `passageCandidates[0].range` 选行（`mod.rs:395-443`），无与 option bank 的互斥扣除 | 无该约束 |

### §6.10 表格、流程图和图片

| # | 断言 | 判定 | 证据 | 说明 |
|---|---|---|---|---|
| 6.10-1 | `compile_table_stimulus(PhysicalTableV2)` 存在 | **FALSE** | grep = 0 | 函数不存在 |
| 6.10-2 | "优先使用 `DocumentIRV2.pages[].tables[]`" | **FALSE** | 物理表格**确实存在**（schema `document_ir_v2.rs:194-197` `row_span/col_span/content_region_ids`；检测器 `pdf_ingest/table_detector.rs:229-261`），但 grammar 表格节点由 `completion.rs:1019` `completion_table_node` 从 `SemanticLine.text` 按 `'|'` 切分构造 | 物理表格能力存在却未被识别链消费 |
| 6.10-3 | 不再创建"一题一行一列"假表格 | **PARTIAL** | `mod.rs:1541-1557`：仅当 `structure.closes_slots` 才提升为 table，否则回退源文本行；`mod.rs:993-1008` 注释说明避免"一题一行" | 已避免假表格，但依据是 slot 闭合而非物理几何 |
| 6.10-4 | hybrid hotspot `normalizedRect` | **PARTIAL** | `src-tauri/src/ielts_grammar/diagram.rs:7-50` 生成 `{type:"diagram",assetId,hotspots:[{hotspotId,slotId,normalizedRect,labelAnchor}]}`；schema 支持（`content_doc_v2.rs` hotspot）；编辑器 `authoring_v2_commands.rs:1301` `setHotspot` | 结构存在，但**仅** `DiagramLabelCompletion`/`PlanMapLabelCompletion`（`diagram.rs:14-19`），且 `asset_id` 必须已存在（`diagram.rs:21` `asset_id?`），无 source-crop 兜底生成 |
| 6.10-5 | 失败不导致题面缺失 / complex visual 100% fallback | **FALSE** | `diagram.rs:21` 无 asset 直接返回 `None`；无 crop fallback 路径 | 无 fallback |
| 6.10-6 | 计划 JSON 示例字段（`display.widthPercent=100/align=center`、`id:"task-27-31-visual"`） | **FALSE** | 实现为 `id: "{task_id}-diagram"`、`display:{align:"left"}`（`diagram.rs:43,48`） | 与示例不一致 |

### §6.11 未分配证据账本

| # | 断言 | 判定 | 证据 | 说明 |
|---|---|---|---|---|
| 6.11-1 | 存在未分配账本 | **PARTIAL** | `src-tauri/src/ielts_grammar/quality.rs:3026-3103` 生成 `coverageLedger`，含 `{sourceNodeId,significant,disposition,targetIds[,reason]}` | 账本存在 |
| 6.11-2 | disposition 标签为 `assigned_to_instruction` / `assigned_to_prompt:q5` / `assigned_to_option:q5:A` / `assigned_to_option_bank:task2` / `assigned_to_stimulus:task3` / `ignored_header_footer` / `unassigned` | **FALSE** | 实际只有三种：`"assigned"` / `"ignored_with_reason"` / `"unassigned"`（`quality.rs:3077,3080,3083`）；`assigned_to_prompt`/`assigned_to_option_bank` grep = 0 | 粒度更粗，无角色化标签 |
| 6.11-3 | 对"每个可见 line/region/table/visual object"标记 | **PARTIAL** | 账本按 `source_nodes` + anchor `targetIds` 展开（`quality.rs:3030-3094`），非按可见对象枚举 | 依赖 anchor 目标，非全量对象 |
| 6.11-4 | 大面积 unassigned 生成 blocker | **HOLDS** | `quality.rs:146-159` 当 `unassigned_ids` 非空产生 `SIGNIFICANT_REGION_UNASSIGNED` 阻断；测试 `quality.rs:4317` | 阻断成立 |

### §18 Phase 4 验收指标（第 3307–3313 行）

| # | 指标 | 判定 | 证据 | 说明 |
|---|---|---|---|---|
| 18-1 | `empty prompt = 0`（golden corpus） | **FALSE** | 无脚本计算；`fixtures/golden/metrics.json:33` 定义 `missing-prompt-for-scored-slot` 但**无任何消费者**（`grep -rn "metrics.json" src-tauri/src scripts src` = 0） | 无度量实现 |
| 18-2 | option label recall >= 99.5% | **FALSE** | `metrics.json:25` 定义 `option-label-recall`（`readyTarget = 1`，与计划 99.5% 亦不一致），无计算脚本 | 无度量实现 |
| 18-3 | statement completeness >= 99% | **FALSE** | 无对应 metric id，无脚本 | 无度量实现 |
| 18-4 | Matching item + bank exact >= 98% | **FALSE** | `metrics.json` 无 matching exact 指标；无脚本 | 无度量实现 |
| 18-5 | complex visual 100% 有 semantic/source-faithful fallback | **FALSE** | 无脚本；且 `diagram.rs:21` 无 fallback 路径（见 6.10-5） | 无度量实现且功能未实现 |
| 18-6 | 是否存在 golden corpus 与度量脚本 | **PARTIAL** | 存在 `fixtures/golden/`（`manifest.json`、`baseline/v1/*`、`phase4-eight-pdf-acceptance.json`、`synthetic/ielts/phase4-grammar-fixtures.json`）与 `scripts/verify-phase4-*.mjs` | corpus 存在，但脚本不度量 §18 指标 |
| 18-7 | `scripts/verify-phase4-grammar.mjs` 度量了上述指标 | **FALSE** | 该脚本（231 行）只做：必需文件存在性检查（`:5-37`）、fixture schema 版本/数量检查（`:42-54`）、feature flag 与源码哈希漂移检查（`:56+`） | 是"文件/契约漂移门"，非质量度量 |
| 18-8 | `scripts/verify-phase4-eight-pdf-acceptance.mjs` 度量了上述指标 | **FALSE** | 该脚本（120 行）校验 manifest 身份（`:36-61`）后调用 Rust 测试 `real_pdf_acceptance::phase4_eight_real_pdfs_...`（`:69-86`），收集 `check.code` 失败项（`:93-98`）；检查码为结构/定性码（`PHYSICAL_AUTHORING_QUALITY_CHAIN`、`FALSE_READY_FORBIDDEN`、`QUALITY_BLOCKER_POLICY`、`WESTERN_QUESTIONS_BEFORE_PASSAGE` 等），**无 recall/completeness/exact 数值** | 无 §18 数值度量 |

### 与 §17.7 / §8.5 / §24 的一致性

| # | 断言 | 判定 | 证据 | 说明 |
|---|---|---|---|---|
| C-1 | §6.2 图模型与 §17.7 模块映射一致 | **FALSE** | §17.7（第 2839-2847 行）映射为 `recognition/local/{question_numbers,question_blocks,stems,options,matching,completion,document,reliability,current_ds_validation}.rs` + `recognize_local(document)`（第 2852 行）；§6.2 声明的是 `QuestionLayoutGraphV1` 及 `instruction_zones/visual_stimuli/unassigned_evidence` 字段。两者文件/类型名不对应，且**都未实现**（`recognize_local` grep = 0，无 `recognition/` 目录） | 内部矛盾 + 双份未实现 |
| C-2 | §6.8 issue code 与 §8.5 `ActionableIssueV1` 一致 | **FALSE** | §8.5（第 1482-1495 行）字段为 `issue_id/library_item_id/target_id/severity/code/title/user_message/suggested_action/source_anchor/local_value/cloud_value/status`；前端 `actionableIssues.ts:19-27` 只有 `issueId/targetId/severity/code/userMessage`。§8.5 未枚举 7 个 code；§6.8 的 code 与后端 `issue_codes.rs` 命名冲突 | 三方（计划/后端/前端）不一致 |
| C-3 | 与 §24 DoD"本地识别"4 条一致 | **PARTIAL** | "direct DocumentIRV2 不经 V1 authoring 主链" → **FALSE**（`auto_pipeline.rs:202-208` 仍走 V1）；"基础题型 Ready 题干/选项完整" → 前端 blocker 部分覆盖；"significant unassigned 阻止错误 Ready" → **HOLDS**（`quality.rs:146`）；"表格/流程图/diagram 至少一种完整呈现" → **PARTIAL**（diagram 有，table 非物理） | 4 条中 1 条 HOLDS、1 条 FALSE、2 条 PARTIAL |

---

## 算法实现核查

### 1. 题号打分（§6.5）实现位置与差异

- 计划：`detect_question_number_tokens` 遍历 `page.spans`，先过滤（expected_range、header/footer、年份/单位），再 `score_number_token` 加权，`score >= 0.55`。
- 实际：`src-tauri/src/ielts_grammar/anchors.rs:14-42` 遍历 `lines`（非 spans），`parse_leading_question_number`（`:59-90`）从**行首**取 ≤3 位数字，分值为固定值（裸 `0.62`、标点 `0.9`、其它 `0.72`），再 `+0.1 if has_prompt`，`min(1.0)`，无阈值、无加减权、无页脚/年份过滤。
- 结论：**算法不同源**。计划的"8 项加权 + 0.55 阈值"未实现。

### 2. 几何题干扩展（§6.6）

- 计划：`geometric_interval` 定搜索域 → 同行取 baseline 容差内右侧节点 → 折行取下方满足"非下一题号 / 非 option run / 同列或悬挂缩进 / 垂直间距阈值"的行。
- 实际：`prompt_assembler.rs:16-88` 以 `anchor.line_index..next.line_index` 为域，跳过指令行与 option 起点行，其余行 `join(" ")`。**无 bbox/baseline/悬挂缩进/垂直间距**。
- 风险：折行可被"区间内全拼接"顺带覆盖，但会把区间内的非题干正文、页眉残留一并吞入；大缩进、跨页无判定。

### 3. 选项悬挂缩进（§6.7）

- 计划：先找 label token，再收集右侧 + 后续悬挂缩进行。
- 实际：`option_run.rs:20-62` 仅在 label 正文为空时吸收**紧邻**下一行（`:30-46`）；不检查 x 列对齐，不收集多行续行。
- 防护：`run_matches_alphabet`（`:114-125`）与 `incomplete` 标志（`:192-194`）提供下游兜底，但没有计划所述的 `Paragraph A` / `Section B` 显式排除。

### 4. 表格编译（§6.10）

- 计划：`compile_table_stimulus` 遍历 `PhysicalTableV2.rows` → `physical_cells_in_row` → 用 `cell.content_region_ids` 组装 `TableCell{row_span,col_span,children}`。
- 实际：`completion.rs:1019+` `completion_table_node` 从 `SemanticLine.text.split('|')` 造 cell，`rowSpan/colSpan` 恒为 `1`，无 `content_region_ids`。物理表格 schema 字段（`document_ir_v2.rs:194-197`）与检测器（`table_detector.rs`）已具备却未被消费。
- 结论：**物理表格到题面的编译链缺失**。

### 5. 未分配账本（§6.11）

- 计划：7 种角色化 disposition 标签。
- 实际：`quality.rs:3075-3084` 三态 `assigned` / `ignored_with_reason` / `unassigned`，按 anchor target 展开；阻断经 `SIGNIFICANT_REGION_UNASSIGNED`（`:146-159`）。
- 结论：账本骨架存在，角色化标签缺失。

### 6. 双栏 / 跨页 / OCR 噪声的几何可行性（遗漏与不可行）

- **双栏**：物理阅读顺序层已按 `columnIndex` 排序（`src-tauri/src/pdf_ingest/reading_order.rs:22,36-55`），但 `semantic_lines_from_v2_shadow` 把物理层压成 `SemanticLine` 时**丢弃列信息**（`SemanticLine` 无 column 字段，`instruction_zone.rs:15-19` 只有 `bbox`）。因此第 6 章"几何恢复"若在 `SemanticLine` 之上做，必须先从 `bbox` 反推列——计划未提这一步，属遗漏的必要步骤。
- **跨页**：行序按页展开，题干跨页只能靠行区间"顺带"拼接，无页边界/页眉页脚扣除逻辑（§6.5 的 `inside_header_footer` 未实现）。
- **OCR 噪声**：`pdf_ingest/ocr_merge.rs` 存在合并能力，但 §6.5 的 token 打分对"年份/单位/百分号"等误检的负权防护未实现，OCR 噪声下的假题号风险被低估。
- **工作量**：§6.2–§6.11 实际要求新建整个 `recognition/local` 模块（§17.7 列出 9 个文件）+ 几何算法 + 区域角色打分器 + 物理表格编译 + 度量脚本与 golden corpus。`task_plan.md` M4 状态仅记 "P4-T01 起点"（统一 DOCX 物理提取入口），P4-T02~T06 "未开始"，与第 6 章描述的规模差距极大。

---

## 发现清单

### A5-F01 [P0] 第 6 章的具名类型与函数在代码中完全不存在，M4 本地识别主链未交付

- 结论：§6.2/§6.4/§6.5/§6.6/§6.7/§6.10 声明的 `QuestionLayoutGraphV1`、`QuestionBlockCandidateV1`、`SemanticRegionRole`、`detect_question_number_tokens`、`score_number_token`、`assemble_question_stem`、`OptionLabelToken`、`assemble_option`、`compile_table_stimulus` 全部 grep = 0；全仓无 `recognition/` 目录，无 `recognize_local`。
- 证据：`grep -rl <各类型> src-tauri/src src` 全为 0；`find . -type d -name recognition` 为空；`src-tauri/src/auto_pipeline.rs:202,206,208` 主链仍是 `make_dynamic_split_candidates → make_dynamic_authoring_ir → build_authoring_v2_shadow`。
- 影响：§6 描述的是"目标态"，不能作为"已实现"或"接近完成"依据。M4 实际进度 = 仅 P4-T01 的物理提取入口统一。
- 建议：在 `task_plan.md` M4 中把 §6 明确标注为"设计，未实现"；实施时按 §17.7 的模块映射落地 `recognition/local`，并同步修订 §6.2 的类型命名。
- 层级：`static`（源码 grep）+ `command`（git 链）

### A5-F02 [P0] §6.8 硬闭包只在前端实现，后端无 `validate_basic_task`，且 issue code 三方命名不一致

- 结论：后端不存在 `validate_basic_task`；硬闭包实际在 `src/features/editor/actionableIssues.ts:105 deriveActionableIssues`。7 个计划 code 中前端只实现 4 个，缺 `QUESTION_PROMPT_BOUNDARY_AMBIGUOUS`、`OPTION_LABEL_MISSING`、`SIGNIFICANT_SOURCE_TEXT_UNASSIGNED`；后端 `issue_codes.rs` 用的是另一套名（`PROMPT_EMPTY`/`PROMPT_BOUNDARY_AMBIGUOUS`/`OPTION_BANK_MISSING`/`SIGNIFICANT_REGION_UNASSIGNED`）。
- 证据：`src/features/editor/actionableIssues.ts:11-17,74,85,98,122`；`src-tauri/src/ielts_grammar/issue_codes.rs:19-23,53`；`src-tauri/src/authoring_review.rs:445,457`。
- 影响：Ready 阻断依赖前端派生，后端/批处理路径无同等闭包；同一问题在不同层产生不同 code，P8 迁移时无法直接对比。
- 建议：先统一 code 词表（建议以后端 `issue_codes.rs` 为准并补齐 3 个缺失码），再在 P8 落地后端 `validate_basic_task`；前端保留为镜像并加一致性测试。
- 层级：`static`（源码）

### A5-F03 [P1] §18 Phase 4 五项验收指标没有任何度量实现

- 结论：`fixtures/golden/metrics.json` 定义了 `option-label-recall`、`missing-prompt-for-scored-slot` 等指标与 `readyTarget`，但**无任何消费者**（grep = 0）。`scripts/verify-phase4-grammar.mjs` 是文件存在/契约漂移门；`scripts/verify-phase4-eight-pdf-acceptance.mjs` 调用的 Rust 验收只产出定性 check code，无 recall/completeness/exact 数值。
- 证据：`fixtures/golden/metrics.json:19,25,64,65`；`scripts/verify-phase4-grammar.mjs:5-54`；`scripts/verify-phase4-eight-pdf-acceptance.mjs:69-98`；`src-tauri/src/ielts_grammar/real_pdf_acceptance.rs` 的 `push_check` code 集合。
- 影响：无法证明 `empty prompt=0`、`option label recall>=99.5%`、`statement completeness>=99%`、`matching bank exact>=98%`、`complex visual 100% fallback` 中的任何一项；Phase 4 出口不可判定。
- 建议：新增 `scripts/verify-phase4-metrics.mjs` 消费 `metrics.json`，对 golden corpus 的 V2 输出计算五项指标；同时修正 `metrics.json` 与计划 §18 的阈值不一致（`option-label-recall` 现为 `=1` vs 计划 99.5%）。
- 层级：`static` + `doc-only`（阈值仅存在于文档/JSON）

### A5-F04 [P1] §6.5/§6.6/§6.7 的 token/geometry-first 算法全部未实现，现为行序启发式

- 结论：加权规则（+0.30/+0.20/+0.15/+0.10/+0.10，-0.30/-0.25/-0.20）与阈值 0.55 无对应实现（`anchors.rs` 用固定 0.62/0.72/0.9）；`assemble_question_stem` 的几何区间、baseline 容差、悬挂缩进、垂直间距判定均无；`assemble_option` 的悬挂缩进续行收集无。
- 证据：`src-tauri/src/ielts_grammar/anchors.rs:31,59-90`；`src-tauri/src/ielts_grammar/prompt_assembler.rs:51-64`；`src-tauri/src/ielts_grammar/option_run.rs:30-46`。
- 影响：题号独立行/折行/大缩进/跨页/OCR 噪声下的识别质量无法达到计划假设；字符串规则失败场景仍在。
- 建议：若采纳 §6 方案，需在 `recognition/local` 内基于 `DocumentIRV2` spans（而非 `SemanticLine`）实现；否则应下调 §6.5–§6.7 的表述为"启发式"，并把指标阈值改为可达成值。
- 层级：`static`

### A5-F05 [P1] §6.10 表格未使用 physical tables，hybrid 热点无 crop fallback

- 结论：`completion_table_node`（`completion.rs:1019`）从 `SemanticLine.text.split('|')` 造 cell，`rowSpan/colSpan` 恒 1；物理表格 schema（`document_ir_v2.rs:194-197`）与检测器（`table_detector.rs`）未被识别链消费。hybrid hotspot（`diagram.rs:7-50`）依赖已存在的 `assetId`（`:21`），无 source-crop 生成；仅覆盖 diagram/map 题型。
- 证据：`src-tauri/src/ielts_grammar/completion.rs:1019+`；`src-tauri/src/schema/document_ir_v2.rs:194-197`；`src-tauri/src/pdf_ingest/table_detector.rs:229-261`；`src-tauri/src/ielts_grammar/diagram.rs:21,41-49`。
- 影响：表格结构（跨行跨列、表头、单元格正文）无法保真；visual 任务在无预置 asset 时直接缺失题面，违反 §6.10"失败不能导致题面缺失"与 §18"100% fallback"。
- 建议：实现 `compile_table_stimulus` 消费物理表格；为 diagram/map/flowchart 增加 source-crop 兜底（可从 `pdf_geometry.rs` 现有裁剪能力派生）。
- 层级：`static`

### A5-F06 [P1] §6.9 `paragraphMap` 恒为空，passage A-G 与 List of Headings 未显式分离

- 结论：`build_passage` 硬编码 `"paragraphMap": {}`（`mod.rs:535`）；分类不满足 §6.9 的三元几何条件（`instruction_signature.rs:169,287` 仅文本线索）；无"option bank 消费 region 不得进入 passage"的扣除（`mod.rs:395-443`）。
- 证据：`src-tauri/src/ielts_grammar/mod.rs:395-443,535`；`src-tauri/src/ielts_grammar/instruction_signature.rs:169,287`。
- 影响：Matching Headings 的段落目标映射缺失，题干/标题银行/应答位三分未落地；`paragraphMap` 的运行时消费方（`src/services/readingRuntimeV2.ts:521`、`reading_source_v2.rs:34`）拿不到数据。
- 建议：在段落构建阶段抽取 A-G label 写入 `paragraphMap`；把 option bank 消费区域从 passage 行集合中扣除。
- 层级：`static`

### A5-F07 [P2] §6.3 数据处理顺序被违反：分类先于几何边界恢复

- 结论：§6.3 明确"题型分类不得先于题面边界恢复"，实际 `build_authoring_v2_shadow` 用 V1 `kindHint` 直接驱动 `infer_instruction_signature`（`mod.rs:196-200`），几何边界恢复（第 5 步）根本不存在。
- 证据：`src-tauri/src/ielts_grammar/mod.rs:134-139,196-200`。
- 影响：分类继承 V1 错误，正是 §6.1 所述"V1 candidate 错误限制 V2 搜索范围"的根因。
- 建议：按 §6.3 顺序重构时，先产出 question block 边界再分类；在重构前不要声称顺序已实现。
- 层级：`static`

### A5-F08 [P2] §6.11 账本 disposition 粒度粗，无角色化标签

- 结论：账本存在但只有 `assigned`/`ignored_with_reason`/`unassigned` 三态（`quality.rs:3075-3084`），无计划的 `assigned_to_prompt:q5` 等 7 类标签。
- 证据：`src-tauri/src/ielts_grammar/quality.rs:3026-3103`；`assigned_to_*` grep = 0。
- 影响：可检测"有未分配文字"，但不能定位"哪道题题干丢句"，与 §6.11 的意图（检测"题型对了但整句题干丢了"）有差距。
- 建议：在账本条目增加 role/target 角色维度，或在 P8 reconciliation 时从 targetIds 反推角色。
- 层级：`static`

### A5-F09 [P2] §6.4 `SemanticRegionRole` 不存在，页/区域角色仅 V1 roleHint 透传

- 结论：无 `SemanticRegionRole`（grep = 0），无 `PageRole`（grep = 0，仅测试元数据有 `pageRoles`）；`instruction_zone.rs:150` 只透传 V1 `roleHint`。
- 证据：`src-tauri/src/ielts_grammar/instruction_zone.rs:150`；`src-tauri/src/ielts_grammar/real_pdf_acceptance.rs:291,892,988`（仅测试）。
- 影响：§6.3 第 2 步（page role segmentation）与 §6.4 的区域角色打分均缺失；区域角色相关指标（`metrics.json` `region-role-macro-f1`）无法计算。
- 建议：与 §6.3/§6.4 一并实施；在此之前把 §6.4 标注为设计。
- 层级：`static`

### A5-F10 [P3] 文档内部矛盾：§6.2 vs §17.7、§6.8 vs §8.5

- 结论：§6.2 的图模型类型名与 §17.7 的 `recognition/local/*.rs` 映射不对应，且 §17.7 要求的 `instruction_zone/visual_stimulus/unassigned` 相关文件在两处都不一致；§8.5 `ActionableIssueV1` 字段远多于前端实现，§6.8 的 code 与后端 `issue_codes.rs` 冲突。
- 证据：计划第 2839-2852 行 vs 第 703-726 行；第 1482-1495 行 vs `src/features/editor/actionableIssues.ts:19-27`；`src-tauri/src/ielts_grammar/issue_codes.rs:19-23,53`。
- 影响：实施者无法从计划确定唯一的目标类型/文件/code 词表，易产生第三套命名。
- 建议：在实施前统一 §6.2/§17.7 的模块与类型命名，并统一 §6.8/§8.5/`issue_codes.rs` 的 code 词表。
- 层级：`doc-only`

---

## 与历史审计的关系

- `audit-2026-09-07/report.md` M4 判定："New import unconditionally calls V1 split and authoring; V2 grammar iterates V1 questionGroupCandidates → Direct V2 main path not delivered"。本次审计**独立复核并确认该判定仍成立**（`auto_pipeline.rs:202-208`、`ielts_grammar/mod.rs:134-139`），且补充了"第 6 章声明的所有具名类型/函数均不存在"这一更强的证伪。
- `repair-2026-09-07/progress.md` 的 "Still Open" 明确：M4 "has started with P4-T01's DOCX parity above; P4-T02~T06 (Question Layout Graph, geometry recovery, hard type closures, completion/visual, unassigned ledger) are untouched"。本次审计逐项核实，**该陈述准确**：P4-T02（Question Layout Graph）、P4-T03（几何恢复）、P4-T04（硬类型闭包）、P4-T05（completion/visual）、P4-T06（未分配账本）均无对应实现，其中 T06 的账本骨架与 T04 的前端闭包为"半成品"（见 A5-F02/F08）。
- `task_plan.md` M4 状态"部分完成……主链仍走 `make_dynamic_split_candidates` → V1 IR → 编译 V2 authoring shadow"与代码一致（HOLDS）。
- 与 `audit-2026-09-07` 的 F01（无法编译）不同：本轮修复后 `cargo check` 已通过（见 `progress.md` 2026-09-11），故本次审计的"未实现"结论是功能缺口，而非构建阻塞。
- 与同轮 A1（第 0–1 章）结论一致：A1 断言 0-2 判定 §0.1 问题 3「本地识别仍经 V1 链路」为 HOLDS，本报告 §6.1 与之互相印证。
- `Plan With Files/Dual_Recognition/backend-map/`（12 reader 后端地图）未涉及识别链的具名类型，本报告未引用其结论。

---

## 证据层级与局限

证据层级分布：

| 层级 | 本报告使用 | 示例 |
|---|---|---|
| `product` | **未使用** | 未启动真实 Tauri、未跑真实导入 |
| `command` | 有限 | `git log --oneline -3`、`git status --short`、`wc -l` |
| `static` | 主体 | 对 `src-tauri/src/**`、`src/**`、`scripts/**`、`fixtures/**` 的 Grep/Read/Glob |
| `doc-only` | 交叉核对 | 计划第 6/8/17/18/24 章、`task_plan.md`、`progress.md`、`audit-2026-09-07/report.md` |

局限：

1. **未运行任何构建/测试**（按约束）。"类型/函数不存在"的结论基于全仓 `grep`（`grep -rl` 对 `src-tauri/src` 与 `src`），若存在拼写变体或宏生成代码，可能漏检；但 §6.2 的 9 个类型名均为唯一英文标识符，误检概率低。
2. **未验证运行时行为**：本报告只能证明"实现缺失/命名不符"，不能证明现有 V1 启发式在真实 PDF 上的具体失败率；§6.1 的"字符串规则容易失败"是设计判断，未在真实语料上量化。
3. **未审计云端链**（§7）与 reconciliation（§8）：§6.8 的后端闭包"未实现"结论仅限 `validate_basic_task` 与 code 词表；`authoring_review.rs` 存在另一条 review 链（`:445,:457`），其完整职责未在本报告范围内穷尽。
4. **`git` 未提交改动**：工作树含 M2/修复轮改动，本报告的"现状"以工作树（非 HEAD）为准；若这些改动被回滚，部分结论（如 `actionableIssues.ts` 的具体行号）可能漂移。
5. **`fixtures/golden/metrics.json` 的 `sourcePlanSection` 为 `17.3-17.4`**，说明它是更早阶段的产物；本报告据此判断它未覆盖 §18，但不排除存在其它未纳入 grep 的度量路径（已对 `scripts/`、`fixtures/`、`src-tauri/src`、`src` 全量搜索 `metrics.json` 引用，结果为空）。
