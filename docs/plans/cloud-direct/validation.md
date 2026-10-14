# 云端直出低成本验证（2026-10-14，历史实验摘要）

本摘要保留实验结论；原型 harness 与模型原始输出已在主分支发布清理时删除，可从 `feat/cloud-direct` 历史取回。当前契约以 `src-tauri/resources/cloud-direct/` 为准；当前产品验证见 `docs/qa/cloud-direct-2026-10-14.md`。

## 结论

用 **1 次 DeepSeek（`deepseek-flash`，关闭思考）调用**，把整份 PDF 直接转成 `ReadingExamSourceV2`。3 份真实 PDF 的结果：

- **40/40 道题答案全对**：与 IELTS Atlas 现有 V1 题目的 `answerKey` 逐题对照。
- 全部通过应用自己的 V2 校验。
- 每份约 **21–28 秒**、**1.7–2.1 万 token**，没有输出截断。

对照：现有链路处理 Pearls 时，候选请求 15.6 秒（首轮 93 秒且无产出），答案页还要另发一次请求，后续校核超过 5 万 token 仍没完成。

## 方法

- 输入：
  - 有文字层的页只发文字层，扫描页（答案页）渲染成 110dpi PNG 图片；
  - system prompt：`prompt/system.md`；
  - 1 份范例：`prompt/example-headings-features-summary.json`（western celebrity，题型为 heading、features、summary）。
  - 不传任何本地解析结果。
- 请求参数：`response_format=json_object`、`thinking.disabled`、`temperature=0`、`max_tokens=32000`。
- 归一化：只补默认字段，不拒收（`harness/evaluate.py: normalize`）。实际只补了两类：
  - `sourceAnchors: []`；
  - `matching_information` 的段落选项，从 `paragraphMap` 生成。
- 校验：
  - PDF2Test 的 `validateReadingExamSourceV2` + `validateReadingAnswerKeyKinds` + `buildReadingRuntimeInteractionModel`（`harness/validate.ts`，用 vite-node 运行）；
  - 复刻的学生端加载检查（`student-exam/.../contracts-v2.ts: normalizeReadingSourceV2`）。
- 正文保真度：输出正文与 PDF 文字层逐词对齐。

## 结果

| PDF | 题型 | 耗时 | token（入/出） | V2 校验 | 答案 | 正文词精确率 | 正文覆盖 |
|---|---|---|---|---|---|---|---|
| 24. P1 The Pearls | 段落信息匹配、摘要选词（A–K）、TFNG | 22.9s | 11,545 / 7,103 | 0 issue | 13/13 | 1.000 | 0.972* |
| 100. P3 musical expert | 单选、YNNG、摘要选词（A–J） | 28.5s | 10,978 / 9,699 | 0 issue | 14/14 | 0.989 | 0.962* |
| 10. P1 Maori Fish Hooks | 段落信息匹配（A–J）、TFNG | 21.2s | 10,694 / 6,783 | 0 issue | 13/13 | 1.000 | 0.979* |

\* 覆盖率不到 1 的部分，逐项核对后是：
- 有意排除的 "You should spend about 20 minutes…" 样板文字；
- 段首字母（已转成 `paragraphLabel`）；
- musical 文末漏了一条术语注释（absolute pitch 的释义），这是唯一真实遗漏。

扣除前两项后，正文覆盖约 99.9%。

原实验产物位于当时的 `results/<name>/`（现已清理）：
- `source-v2.json`：归一化后的学生端源；
- `eval.json`：评分明细；
- `call.json`：耗时、用量、页面模式；
- `raw.txt`：模型原始输出。

## 未覆盖 / 局限（如实说明）

- **只是命令层与 schema 级证据，没有走真实产品路径。** 没有导入 Tauri 应用，没有过 Rust `compile_reading_source_v2`，也没有在 student-exam 的 Vue 渲染器里打开。产品路径验证由开发 Agent 完成。
- 样本只有 3 份，且都是同一来源、版式相近：文字层 + 末尾扫描答案页。以下情况未测：
  - 纯扫描件；
  - 表格、流程图、图表标注填空；
  - choose TWO 多选；
  - matching_headings（范例本身就是 heading 题，存在偏向）；
  - 超长 P3。
- 答案"全对"依赖 PDF 自带答案页；没有答案页的 PDF 应全部输出 `unresolved`，未验证。
- 每份只跑了一次，没有测稳定性（重复运行的方差）。

## 对开发的建议

1. `prompt/system.md` 和范例可以直接作为直出契约的起点。建议再补 2 份范例：一份 TFNG + 单选，一份表格或流程图填空。
2. 归一化保持"只补不拒"。本次需要补的字段极少，说明大部分结构模型能一次给对。
3. 扫描页走图片、文字页走文字层，成本约 2 万 token/份。如果要提升表格类版式的还原度，可以对含表格的页额外附图（`run.py --images all`）。
4. 术语注释、脚注类内容要在 prompt 中再强调一次，或者用保真度 diff 在编辑器里提示。

## 当前产品复现入口

```powershell
node scripts/e2e/build-app.mjs
node scripts/e2e/tauri-cdp-cloud-direct.mjs
```

原型脚本只属于历史命令/schema 实验，不作为当前产品验收入口。
