# IELTS Document Recognition Phase 4 完成记录

> 真实语料状态补充：独立的 8-PDF 可执行验收门见 `Files/IELTS_Document_Recognition_Phase_4_8_PDF_Acceptance_CN.md`。该门禁已运行真实 PDF → V1 → physical V2 → authoring V2 → QualityReportV2 链路，当前因 Chili image、Petri shared bank、Western question-before-passage、metadata response/option contract 与 Organisational compiler probe 等缺口返回非绿色。因此本文此前的“完成”结论仅适用于合成 grammar shadow，不代表 8 份真实 PDF 已通过 acceptance，也不代表 V2 可进入生产。

## 范围

Phase 4 实现 IELTS Reading 题面 grammar 与可靠性 shadow layer，首个 PR 以 PR-05（Question expression + instruction signature）为主线，并继续完成 D-004～D-014 与 E-001～E-005 所需的最小可运行 vertical slice。

本阶段仍遵守以下边界：

- `document-ir.json` 与 `authoring-ir.json` 是 V1 authoritative 产物，读取和写入顺序不变；
- `authoring-ir-v2.shadow.json`、`authoring-ir-v2.shadow.compare.json` 和错误记录是独立 V2 shadow 产物；
- V2 不进入当前 authoring UI、export、runtime 或 NAS 发布路径；
- `authoringV2Shadow`、`documentIrV2Shadow` 以及其他 V2 flags 默认关闭；
- 不启用 per-question PDF LLM repair，也不使用 LLM 生成题干、选项或答案。

## 已完成内容

### D-001～D-005：题号、instruction 与题干

- 支持 range、`and`/逗号列表和 mixed range，统一处理 Unicode dash、`to` 和跨行空白；
- 拒绝 `In boxes 1–6`、`Reading Passage 1`、倒序或异常超长范围等嵌入式数字上下文；
- instruction zone 从题组 heading 开始，遇到首个题号、option run 或新题组时停止；同一行中的 legend 会保留，已出现的首个题干会被截断；
- signature 区分 TFNG、Y/N/NG、单选、多选、matching、completion 和 short answer，并提取 cardinality、option alphabet、reuse policy、assignment 与 word limit；
- question anchor、prompt boundary、source line ID 与 source anchor 一起保留；候选题干缺失或边界不确定时进入质量阻断。

### D-006～D-009：选项、共享 slot 与 passage 角色

- 只有连续、单调、闭合的 A/B/C… option run 才进入 V2；孤立的 `A ...` 普通段落不会被当成完整选项；
- matching 的 `List of Headings`、`List of People`、`List of Features`、`List of Categories` 与 box 选项池使用公共 `OptionBankV2`；公共池的 reuse policy 与 response 引用分开表达；
- `Questions 14 and 15` + `Choose TWO` 可形成一个 shared response group 和两个 answer slots，assignment 为 unordered set；
- fallback passage 会过滤题号题面、instruction 和 option label，因此题目先于 passage 的版式不会把题面误收进 passage。

### D-010～D-014：completion、figure、答案与证据

- sentence/summary/note/form/table/flowchart/diagram completion 生成带 host node 的 slot；completion 缺 word limit、host 或结构化 stimulus 时不标 Ready；
- diagram/map 仅在存在真实 shadow asset 时生成 diagram node 和 asset reference；无图或 hotspot 无 figure host 时明确阻断；
- V1 answer key 转换为 V2 `text`、`option` 或 `unresolved`，不确定答案不会被猜测为已确认答案；
- prompt、option、bank、slot、passage 和 asset 保留 source anchors；物理 shadow 有 pages/lines 时计算显著区域 coverage。

### E-001～E-005：可靠性门

`src-tauri/src/ielts_grammar/quality.rs` 现已检查：

- expected question numbers 与实际 slots 的一致性、重复题号和空题组；
- 空题干、题干边界歧义、instruction signature 低置信度；
- option run/option bank 闭合性、公共池引用和多选 cardinality；
- completion word limit、slot host、table/flowchart/figure stimulus；
- answer key 缺失、答案超出 word/number limit、option 不在 bank；
- assetId、relativePath、SHA-256 和 figure reference；
- `QualityReportV2` 的 `blocked` / `review_required` / `ready` 状态与稳定 issue codes。

`auto_pipeline::has_reliable_question_groups` 只有在显式开启 `EPIC8_AUTHORING_V2_SHADOW=1` 的 debug shadow 环境时才使用 V2 quality state；默认关闭时保留既有 V1 行为，避免改变现有生产路由。

## 夹具与门禁

- 新增 `fixtures/golden/synthetic/ielts/phase4-grammar-fixtures.json`：题号表达式、TFNG/YNNG、单选、shared multi-select、matching people option bank、completion word limit 和 question-before-passage 场景；
- Rust `ielts_grammar` 测试读取该矩阵，并验证表达式、signature、option bank 与 passage fallback；
- 新增 `npm run verify:phase4:grammar`，检查文件、夹具、稳定 issue codes、全部默认 flag、V1/V2 写入边界、Rust format 和 Phase 4 Rust 测试；
- Phase 3 DOCX 门禁同步检查 `authoringV2Shadow: false`；
- `check`、`build`、Phase 0 strict、Phase 1 schema、Phase 2 shadow、Phase 3 DOCX gate 与 Phase 4 grammar gate 应一并运行。

## 当前状态与后续 promotion 条件

本阶段产物已足够生成可比较、可审阅且不会误报 Ready 的 V2 shadow。它仍不是 V1 替换，也没有把 table/figure 编辑器、V2 renderer 或 NAS exporter 提前引入。

正式打开任何 V2 flag 前，还必须在冻结的 8 份 Reading 样例及扩展 corpus 上补齐 question/prompt/option/slot/answer/asset 覆盖统计，并人工审核所有 `blocked` 与 `review_required` 报告；未达到门槛时继续保持 shadow-only。
