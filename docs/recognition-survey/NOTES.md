# 识别层事实盘点（2026-09-20，含答案页证据链实施结果）

## 0. 范围、证据等级与运行方式

本文 §1–§5 记录识别层事实盘点；盘点阶段没有改产品代码、识别架构、modality、发布链或 source coverage。答案页证据链的实施与验收结果见 §6，仍不涉及听力、modality、发布链或 source coverage。

样本均走真实 Tauri UI 的导入入口、真实 scheduler、真实 SQLite 和真实 Rust 后端。CDP 使用了仓库现有的诊断启动参数 `--no-sandbox --disable-gpu`，因此这里的产品证据是“真实 Tauri/CDP 诊断通道”，不是默认启动参数的验收结论。

主要运行档案：

- 听力识别与 pdfium 原始产物：`artifacts/e2e-cdp/run-listening-parser-retain-2026-09-19/`。
- 听力无云调度链：`artifacts/e2e-cdp/run-local-chain-2026-09-19T22-35-10-585Z/`。
- 阅读 9 份批量导入：`artifacts/e2e-cdp/run-local-chain-2026-09-19T22-43-59-328Z/`。
- 正确的 `sleep-study` 补跑：`artifacts/e2e-cdp/run-local-chain-2026-09-19T22-55-51-604Z/`。

无云 local-chain 的共同结果是 `source=not_run`，所以脚本自身报红。这是原文核验链的运行状态，不是下面识别稿结构的判据；结构结论来自每份 job 中的 `document-ir.json`、`authoring-ir.json`、`authoring-ir-v2.shadow.json` 和 recognition candidate。批量运行还暴露了脚本/入口的一个操作事实：PDF 的 folder hook 会导入目录内全部 PDF；9 个 PDF 放在同一 staging folder 后，每一轮 import 都会重新处理整目录。因此本次批量档案里有重复 job，结构统计只取第一轮每个 source hash 的 9 个 job。

## 1. 文本层缺空格：根因和影响面

### 1.1 pdfium 路径的事实

V1 `document-ir.json` 的 parser 元数据是：

```json
{
  "provider": "rust-parser:pdf:pdfium",
  "degradedFallback": false,
  "warnings": [],
  "pages": 8
}
```

所以这次观察不是 `pdf_extract` 或 legacy text-layer fallback 造成的。产品路径在 [`parser.rs:492-515`](../../src-tauri/src/parser.rs:492) 先调用 pdfium，只有失败时才会写 `GEOMETRY_PIPELINE_UNAVAILABLE` 并退回文本层；本次没有走 fallback。

按 pdfium 产物的 `pageIndex` 统计，每页 block 中含空白字符的数量如下：

| pageIndex | block 总数 | 含空白 | 不含空白 | 代表性输出 |
|---:|---:|---:|---:|---|
| 1 | 28 | 26 | 2 | 候选人说明页，普通句子基本有空格 |
| 2 | 28 | 6 | 22 | `Readthetextandanswerq ue s tions1-10`；`Completetheformbelow` |
| 3 | 16 | 4 | 12 | `Questions8-10`；`Completetheformbelow` |
| 4 | 29 | 13 | 16 | `SECTION2`；`Choosethecorrectanswer.` |
| 5 | 15 | 11 | 4 | `ChooseFOURcorrectanswe rs,A-F,nextto qu es tions 17-20.` |
| 6 | 32 | 16 | 16 | `ChooseFIVEcorrectletters, A-G, nextt oque s tions 21-25.` |
| 7 | 8 | 6 | 2 | 选项行仍有大量连续字串 |
| 8 | 23 | 13 | 10 | `Completethenotesbelow`；`WriteNOMORETHAN TWOWORDSfor eac hanswer.` |

关键结论：第 1 页的普通文本有空格，但题目页 2–8 的文本流确实没有可靠词边界。不是“完全没有任何空格字符”——pdfium 的几何拼接偶尔会插入空格——而是没有足以支持语义规则的词边界。

### 1.2 几何识别有没有恢复词边界

有一个基于字形横向间距的启发式，但不能可靠恢复：

- [`pdf_geometry.rs:351-371`](../../src-tauri/src/pdf_geometry.rs:351) 只收集 pdfium 字符及 `origin_x/origin_y`。
- [`pdf_geometry.rs:422-439`](../../src-tauri/src/pdf_geometry.rs:422) 用相邻字形 advance 的中位数乘 2.5 作为 word-gap threshold。
- [`pdf_geometry.rs:2137-2164`](../../src-tauri/src/pdf_geometry.rs:2137) 按阈值切 word，再用空格拼回行文本。
- [`pdf_geometry.rs:457-506`](../../src-tauri/src/pdf_geometry.rs:457) 只做 whitespace collapse、标点和连字符的窄归一化，没有词典、语言模型或 OCR 级词切分。

真实输出同时证明了“恢复不足”和“误恢复”两件事：

- 没有恢复：`Readthetextandanswerquestions1-10`、`Completetheformbelow`、`Choosethecorrectanswer.`。
- 错误恢复：`q ue s tions`、`TWOWORDSfor eac hanswer`、`answe rs`、`AN D/O RANUM BER`。

因此本次几何规则只能说“尝试按大间距切分”，不能说“从字形间距恢复了词边界”。V2 physical shadow 另走 `rust-parser:pdf:pdf-extract:shadow`，同一份档案中也没有弥补这个事实；听力结论以 V1 pdfium 产物为准。

### 1.3 直接文本匹配规则的数量和失效方式

按 `src-tauri/src/**/*.rs` 的 `Regex`/`regex::` 静态搜索，后端识别层使用正则的规则数是 **0**。当前 Rust 识别主要是 literal `contains`/`starts_with`、字符扫描和间距规则：

- 题号范围：[`question_number.rs:17-22`](../../src-tauri/src/ielts_grammar/question_number.rs:17) 的解析器依赖 `Questions ` / `Question ` 等带空格的结构。
- 题型和选择数：[`instruction_signature.rs:161-225`](../../src-tauri/src/ielts_grammar/instruction_signature.rs:161) 依赖 `complete the form`、`choose`、`two/four/five` 等带空格短语。
- 字母区间：[`instruction_signature.rs:286-350`](../../src-tauri/src/ielts_grammar/instruction_signature.rs:286) 可识别 `A-F`、`A-G`，但前提是 instruction 已经形成可匹配的规范文本。
- word limit：[`instruction_signature.rs:383-402`](../../src-tauri/src/ielts_grammar/instruction_signature.rs:383) 直接查 `one/two/three/four words`、`a number`、`and/or`。

前端 `src/services/devFallbackBackend.ts` 还有 8 个正则匹配/`RegExp` 源码位置（541、543、545、1412、1798、2033、2219、2237），但那是 dev fallback，不是本次真实 Tauri 后端使用的识别路径。

在无空格输入上，影响不是“某一条正则漏匹配”，而是入口层连续失效：`Questions1-4` 不能成为题号表达式，题组没有 expected range；没有题组就不会调用出可用的 instruction signature；因此 A-F/A-G 和 word limit 即使在函数能力上有实现，真实稿中也没有 `optionAlphabet` 或 `wordLimit` 可供质量门使用。

## 2. 听力卷的真实识别结果

### 2.1 总量

原卷有 4 个 section、40 题。按连续 instruction block，期望是 8 个 task group：

| Section | 期望 task group | 期望题数 |
|---|---|---:|
| 1 | form Q1–4；single choice Q5–7；form Q8–10 | 10 |
| 2 | single choice Q11–16；choose FOUR A–F Q17–20 | 10 |
| 3 | choose FIVE A–G Q21–25；single choice Q26–30 | 10 |
| 4 | note completion Q31–40 | 10 |
| **合计** | **8** | **40** |

实际结果：

- V1 `authoring-ir.json`：0 groups、0 question order、0 answer key entries。
- V2 `authoring-ir-v2.shadow.json`：0 task groups、0 answer slots，quality `blocked`。
- local candidate：`status=partial`、0 task groups、0 slots。
- `question-layout-graph.json`：13 个临时 task-group 区域，但全部 `taskHint=null`、`questionRange=null`、`INSTRUCTION_SIGNATURE_UNRESOLVED`；它们不是已识别的 task group，不能计入产出。

### 2.2 五种题型对照

| 题型 | 原卷期望范围 | 实际识别 | `option_alphabet` / `wordLimit` |
|---|---|---|---|
| 表单填空 | Q1–4、Q8–10，共 7 slots | 0 group、0 slot | 没有生成 signature；没有 word limit |
| A/B/C 三选一 | Q5–7、Q11–16、Q26–30，共 14 slots | 0 group、0 slot | 没有生成 signature |
| Choose FOUR A–F | Q17–20，共 4 slots | 0 group、0 slot | 原文 block 含 `A-F`，实际没有 `optionAlphabet` |
| Choose FIVE A–G | Q21–25，共 5 slots | 0 group、0 slot | 原文 block 含 `A-G`，实际没有 `optionAlphabet` |
| 笔记填空 | Q31–40，共 10 slots | 0 group、0 slot | 原文含 `NOMORETHAN TWOWORDS` 的残片，实际没有 `wordLimit` |

这里的“没有推导出 A-F/A-G”和“word limit 解析失败”要精确理解：本次首先卡在题号/题组范围未解析，所以没有进入可以产出 signature 的阶段；不能把它缩减成 `infer_option_alphabet` 或 `parse_word_limit` 单函数的失败。

### 2.3 blocking code 与归因

听力 V2 quality report 的硬阻断为：

```text
QUESTION_RANGE_UNPARSED
SIGNIFICANT_REGION_UNASSIGNED
V1_COMPATIBILITY_COMPILER_FAILED
```

其中 `SIGNIFICANT_REGION_UNASSIGNED` 有 16 个 source node，source coverage 为 `0.893333`（134/150 assigned）；`V1_COMPATIBILITY_COMPILER_FAILED` 的细节是 `questionGroups` 为空、`answerKey` 为空。这三个都是当前真实结构未识别后的连带结果，第一落点是 `QUESTION_RANGE_UNPARSED`。

没有出现 `RUNTIME_PASSAGE_MISSING`。这个 code 只在 [`reading_source_v2.rs:83-88`](../../src-tauri/src/reading_source_v2.rs:83) 的 `passage` 缺失早退时产生；本次听力稿有 passage，实际失败的是空题组/空答案键触发的通用 Reading V1 compatibility compiler。`DomProtocol` 和 `RuntimePreview` 在该产物中均通过，但因为没有 slots，学生预览没有可作答控件；这不等于听力已可用。

### 2.4 听力 epic 工作量重估

结论是上调，不是下调。原来的 12–13 个集成点不能继续作为封闭估算；本次事实至少暴露了 14–16 个可独立验收的接点，其中约 5 个可以直接复用阅读侧能力，约 9–11 个是听力输入或无空格输入特有的接点。

可复用的阅读侧能力：

1. pdfium 字符坐标、页面/region/block 产出；
2. 页面图像与 born-digital/image-only 判定、资源落盘；
3. task/response/answer-slot 的 schema 与 materializer；
4. 已有 instruction signature 的题型、选择数、A-F/A-G、word-limit 语义；
5. quality report、recognition candidate、decision 和前端问题展示的证据链。

必须新增或针对听力补强的接点：

1. 面向无空格 PDF 的词边界恢复/不误切分；
2. `SECTION 1–4` 与听力 question page 的分段；
3. 不依赖空格的 `Questions1-4` 等题号范围识别；
4. 跨行/跨页 instruction zone 拼接；
5. form completion 的标签、空位和行结构识别；
6. note completion 的标签、空位和复合 word-limit 识别；
7. single-choice 与多选（FOUR/FIVE）选项库、cardinality 和绑定；
8. 听力稿没有 reading passage/answer-page 假设时的质量判据与 runtime 编译边界；
9. scheduler → local candidate → source verification → decision 在听力输入上的真实可观测链；
10. 40 题/4 section 的端到端回归与分题型 golden 标注。

这只是工作量重估，不是本轮架构方案。最主要的新信息是：若不先解决无空格文本，五种题型并不是五个局部适配点，而是全部被同一个上游题号/instruction 入口挡住。

## 3. 阅读抽样盘点

### 3.1 样本和结果

7 份已有 golden 来自外部目录，另加用户指定的流程图版和仅原文无题，共 9 份。下表的“实际”取 `authoring-ir-v2.shadow.json`；括号中的 V1 差异用于说明 V2 相对已有 baseline 的归一化效果。

| 样本 | 期望结构 | 实际 V2 结构 | 数量 | 与 golden / 识别差异 |
|---|---|---|---:|---|
| Chili peppers | TFNG 1–6；notes 7–13 | TFNG 1–6；notes 7–13 | 2 / 13 | 结构一致；V1 把 notes 记成 `sentence_completion`，V2 已归一化为 `note_completion` |
| Fishbourne Roman Palace | TFNG 1–6；notes 7–13 | TFNG 1–6；notes 7–13 | 2 / 13 | 结构一致；V1 同样把 notes 记成 `sentence_completion` |
| Conformity | Y/N/NG 27–30；summary 31–35；notes 36–40 | 完全一致 | 3 / 14 | V1 的最后一组为 `sentence_completion`，V2 已识别为 notes |
| Petri dish | matching information 14–19；matching features 20–25；summary 26–29 | 完全一致 | 3 / 16 | V1 第二组只写 `matching`，V2 已归一化为 `matching_features` |
| Organisational design | multiple choice 14–15、16–17、18–19、20–21；matching features 22–26 | 完全一致 | 5 / 13 | V1 写成 `multi_choice`/`matching`；V2 识别出四组 shared TWO-letter choice 和 matching features |
| Western celebrity | headings 14–20；matching features 21–23；summary 24–26 | 完全一致 | 3 / 13 | V1 写成 `heading_matching`/`matching`；V2 归一化正确 |
| Sleep study（139） | TFNG 1–4；notes 5–13 | 完全一致 | 2 / 13 | V1 把 notes 记成 `sentence_completion`，V2 已归一化 |
| 231. 铅笔的历史（流程图版） | 本仓库无 golden | notes 1–3；TFNG 4–8；flowchart 9–13 | 3 / 13 | 真实导入能形成结构；V2 另报 `PROMPT_EMPTY`，答案未解析，page 5 无可抽取文本 |
| 121. P2（仅原文无题） | 有意作为 passage-only 负例 | 0 groups / 0 slots | 0 / 0 | `QUESTION_RANGE_UNPARSED`；与文件名所述“无题”一致，不应当当成题型识别通过 |

7 份 golden 的共同差异不是 task group 数或 slot 数：它们的 V2 数量全部与 metadata expected 一致。共同阻断是答案页/答案证据：每份的 answer slots 都有 entry，但 `answerKey` 的每个值都是 `kind=unresolved`，resolved answer 数为 0；quality 共同出现 `ANSWER_KEY_MISSING_SLOT` 和 `RUNTIME_COMPILER_FAILED`，runtime probe 主要报 `RUNTIME_ANSWER_UNRESOLVED`。对应的 source-review 也分别记录了 answer/explanation page 无可抽取文本（例如 chili/fishbourne/conformity page 5，petri page 6–7，organisational page 7–8，sleep-study page 6）。

这说明本地识别层对这 7 份阅读卷的“题组/题号/题型/slot 结构”已经有可用事实基础，但不能据此声称答案解析、质量 Ready 或学生端可评分已经完成。答案/OCR/vision 是另一条证据门。

### 3.2 240 份语料能不能做系统回归

可以作为系统化**结构回归语料**，但不能原样当成一个单一的“全链路通过率”门禁。

理由和接入建议：

1. 已有 `fixtures/golden/metadata/*.json` 的 schema 能表达 source SHA、page roles、task groups、slots、option banks、known issues；这 7 份样本证明“expected structure vs actual V2”是可比较的。
2. 真实 PDF 继续放授权的私有目录/CI secret store，不提交到仓库；用 SHA-256、原始文件名和稳定 fixture id 做 manifest。仓库已有 `scripts/register-phase0-plan-corpus.mjs` 和 `fixtures/golden/manifest.json`，可作为接入入口。
3. 每个 fixture 应单独 staging folder、单独 Tauri run，或让 runner 显式传单文件；当前 PDF folder hook 会把一个目录里的所有 PDF 一起导入，本次 9 文件共用目录导致重复处理，不适合作为逐样本基线。
4. 回归至少分三层统计：结构（groups/slots/types/ranges）、答案/OCR（resolved/unresolved answer key）、runtime/export。像 121 这样的 passage-only 样本应是明确负例，不应和普通题卷混入同一成功率。
5. 先用 7 个 curated golden 覆盖题型/页面顺序/答案页缺失等已知问题，再用固定 seed 的更大样本做分布回归；当前 manifest 的历史登记是 population 272、sample 8，而当前外部目录实测为 277 个 PDF（其中 270 个以编号开头）。因此“240 份”这个数字目前没有对应的仓库 manifest 证据，接入前应先冻结实际 population 和筛选规则。

最终判断：这批语料适合做系统化回归，前提是把私有源、SHA、人工结构标注和 known issue 固定下来，并把结构门与答案/vision/runtime 门拆开统计；不适合直接把“无云链 source not_run”或“答案页 unresolved”混入题型识别结论。

## 4. 本轮结论边界

- 已证明：真实听力 PDF 经过 pdfium 后题目页词边界不足以支撑当前 grammar；真实产品得到 0/0 识别结构；阅读 7 份 golden 的 V2 结构识别与 expected group/slot 数一致。
- 未证明：听力 modality 已接入；听力能导出；阅读样本的答案/OCR 已完成；云端修复或发布链通过。
- 上述盘点阶段未修改：产品代码、测试断言、脚本逻辑、golden metadata 和发布链；答案页实施见 §6。

## 5. 答案页证据链盘点（2026-09-20，实施前）

### 5.1 物理层分类和图像资产

七份 golden 的答案页不是普通 born-digital 文本页。按每次真实导入落盘的
`document-ir-v2.shadow.json` 的 `pages[].quality`、`imagePlacements` 和 `assetIds`
检查，答案页都被物理层标为 `scanned`，并带有
`PDF_IMAGE_ONLY_PAGE_REQUIRES_OCR`、`requiresOcrRegions` 和图像 placement/asset：

| 样本 | 答案页 | 物理分类 | 图像资产事实 |
|---|---:|---|---|
| Chili peppers | 5 | scanned | 1 placement / 1 asset |
| Conformity | 5 | scanned | 3 placements / 3 assets |
| Fishbourne Roman Palace | 5 | scanned | 1 / 1 |
| Organisational design | 7–8 | scanned | 各 1 / 1 |
| Petri dish | 6–7 | scanned | 各 1 / 1 |
| Sleep study | 6 | scanned | 1 / 1（同卷第 5 页另有正文图像资产） |
| Western celebrity | 5 | scanned | 1 / 1 |

因此答案页图像没有在 document-ir 入口丢掉：页面质量和资产索引都在。真实渲染图也已抽查：
版式是表格、分组标题、多栏/答案与解释并列的组合，不是可以安全依赖“每行一个
`题号 答案`”的纯文本列表。解析器必须把页面图像当作证据，并要求模型同时返回题号、答案、页码和可复核的可见引用。

### 5.2 现有视觉 sidecar 的边界和默认调用

现有路径位于 [`parser.rs:2454-2511`](../../src-tauri/src/parser.rs:2454)：先尝试
Python `extract_pdf_images` sidecar，随后把嵌入图像与页面渲染图合并；sidecar 本身只负责
图像提取/渲染，不负责 OCR 或答案解析。当前开发机直接运行该 sidecar 因缺少 `pypdf`
返回 `missing_pdf_dependency:pypdf`，但产品 Rust 路径会继续走 Windows 默认的
`pdfium-render` 页面渲染（[`parser.rs:2161-2290`](../../src-tauri/src/parser.rs:2161)；
默认设置来自 [`environment.rs:253-265`](../../src-tauri/src/environment.rs:253)）。
`pdf_geometry.rs:2766-2868` 会把每页渲染为 PNG，因此即使 Python sidecar 不可用，答案页
仍可以获得视觉输入；若 pdfium 绑定也不可用，当前行为是 `requiresManualReview=true`
的降级产物，而不是伪造 OCR 结果。

现有 `auto_pipeline.rs` 的 `main_pdf_vision_extraction` →
`vision_answer_candidate_for_job` 已经调用 `extract_pdf_image_answers`，默认只在有云端
profile 的 PDF 云端诊断/识别路径中运行，并将输出写入
`vision-answer-output.json` / `vision-answer-candidates.json`。盘点时确认的缺口是：
它现在只产出“待人工采用”的诊断候选，`visionAnswerExtraction.applied` 固定为 false，
没有经 `library::repository::apply_editor_commands_tx_with` 写入权威 `answerKey`。

本轮实现边界因此明确为：复用这条已有的图像资产和视觉网关路径，只增加严格的答案页候选
校验与唯一 canonical 写入；没有图像证据、没有答案页候选、证据/置信度不足或题号无法映射
时，一律保持 `kind=unresolved`。自动写入的 journal provenance 使用
`answer_page_recognition`，与用户编辑和云端修复分开。

### 5.3 解析策略结论

不新增一个基于文本行正则的答案页解析器。七份真实页面已经证明答案与解释可能并列、同页
多段、跨页表格；现有 PDF 文本层对这些页面本来就是不可用的。采用“扫描页图像 → 视觉答案
候选（每项带页码/quote）→ 题号映射和置信度门 → canonical `setAnswer`”的证据链，且把
空白页作为显式负例：空图不能沿用上次视觉结果。

## 6. 答案页证据链实施与验收（2026-09-20）

### 6.1 写入边界

实现落在现有产品 scheduler 的云端阅读路径：物理 shadow 只挑
`pages[].quality.classification=scanned` 的页面，并把 shadow 的零基页号转换成渲染图的一基
页号；没有扫描答案页时直接生成空候选，不调用视觉模型，也不制造答案。

视觉输出必须同时满足：题号能映射到 canonical slot、答案与该 slot 的文本/选项约束相容、
置信度至少 `0.85`、且有真实页码和非空可见引用。通过后只生成 `setAnswer`，由
`library::repository::apply_editor_commands_tx_with` 写入 canonical；journal 的
`edit_origin` 是 `answer_page_recognition`。已有人工编辑受保护，不被答案页机器写入覆盖。
前一次答案页识别写入的 slot 在本次答案页为空、低置信度或答案缺失时会被清回
`kind=unresolved`；用户编辑过的 slot 不会被清除。实现位置见
[`auto_pipeline.rs:875`](../../src-tauri/src/auto_pipeline.rs:875)、
[`auto_pipeline.rs:1137`](../../src-tauri/src/auto_pipeline.rs:1137)、
[`auto_pipeline.rs:1261`](../../src-tauri/src/auto_pipeline.rs:1261) 和
[`repository.rs:235`](../../src-tauri/src/library/repository.rs:235)。

### 6.2 七份 golden 对照

基线来自 §3.1：7 份卷的 answerKey 均为 unresolved。下表的“人工核对”是先查看本轮已落盘
的真实答案页图像，再把核对结果作为受控视觉服务的答案返回值；因此它证明真实 Tauri →
pdfium 页面渲染 → vision gateway 图像请求 → canonical 写入链路和映射正确，不冒充外部
视觉模型的召回率测试。

| 样本 | 答案页 | 槽数 | 基线 resolved | 本轮 resolved | 人工核对错误 | 应用报告 |
|---|---:|---:|---:|---:|---:|---|
| Chili peppers | 5 | 13 | 0 | 13 | 0 | `answer_page_recognition`, 13 |
| Fishbourne Roman Palace | 5 | 13 | 0 | 13 | 0 | `answer_page_recognition`, 13 |
| Conformity | 5 | 14 | 0 | 14 | 0 | `answer_page_recognition`, 14 |
| Petri dish | 6–7 | 16 | 0 | 16 | 0 | `answer_page_recognition`, 16 |
| Organisational design | 7–8 | 13 | 0 | 13 | 0 | `answer_page_recognition`, 13 |
| Sleep study | 5–6 | 13 | 0 | 13 | 0 | `answer_page_recognition`, 13 |
| Western celebrity | 5 | 13 | 0 | 13 | 0 | `answer_page_recognition`, 13 |
| **合计** |  | **95** | **0** | **95** | **0** | 95 个 canonical `setAnswer` |

答案页图像在真实 gateway 请求中实际带入的物理页面分别是 `5`、`5`、`5`、`6,7`、
`7,8`、`5,6`、`5`，与上表一致。Western 首次回归暴露了一个真实映射缺陷：罗马数字
选项 `ii/iv/vii/viii/iii` 在 canonical option bank 已大写时没有做大小写归一化，导致当时
只有 8/13；先保留这个红色结果和单测，再修复选项标签比较，修复后 Western 复跑为 13/13。

### 6.3 反例和负控

- 空白答案页反例：`blank_answer_page_clears_a_previous_answer_page_value_instead_of_reusing_it`
  先证明旧的答案页识别值会被错误保留的风险，修复后断言重新落为 `kind=unresolved`；
  同一 canonical 写入测试还检查 journal provenance 为 `answer_page_recognition`。
- 低置信度沿同一命令构造逻辑不进入 `reliable`，已有答案页来源的 slot 只会清回
  unresolved，候选文件和 application report 保留模型 warning 说明原因。
- `121. P2(仅原文无题) - Muscle Loss 肌肉流失.pdf` 负控：两页均
  `born_digital`，`answerPageImageCount=0`，没有发出 `extract_pdf_image_answers` 请求，
  application 为 `attempted=true / applied=false`，warning 为
  `no image-only answer page was found`，无错误退出。该文件确实没有题目，故 canonical
  `answerSlots=0`、resolved=0；没有答案页就没有答案来源，系统不编造答案。

### 6.4 回归结果

- `cargo test --manifest-path src-tauri/Cargo.toml --lib`：**860 passed, 0 failed,
  11 ignored**。
- `npm test -- --run`（Vitest）：**311 passed**。
- 指定的 `node scripts/e2e/tauri-cdp-cloud-repair-chain.mjs`：**13/13 passed**，使用的是
  仓库内的 `fixtures/parser/demanding-reading-passage-3.pdf` 真实链路；与
  `fixtures/golden/private-real/` 无关。
- 这轮没有改题型识别或质量门禁判据。folder hook 的“选一份却导入同目录全部 PDF”仍未修，
  因为它是独立的导入 UX/调度边界，本轮先不扩大答案证据链范围。
