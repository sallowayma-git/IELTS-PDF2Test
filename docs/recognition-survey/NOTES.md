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

## 7. 未见样本泛化验证与回归冻结（2026-09-20）

### 7.1 固定样本

从 `F:\workspace\IELTS Atlas\ReadingPractice\PDF` 的 277 份 PDF 中，按固定
`seed=20260920` 和 `seeded-sha256-rank` 选取 28 份；排除了 §6 使用的 7 份 golden、
`231. P1 - The History of the Pencil 铅笔的历史（流程图版）.pdf` 和
`121. P2(仅原文无题) - Muscle Loss 肌肉流失.pdf`。文件名、SHA-256、大小和可复算
排序规则已冻结在
[`fixtures/golden/answer-page-generalization-2026-09-20.manifest.json`](../../fixtures/golden/answer-page-generalization-2026-09-20.manifest.json)，
对应提交为 `fab14d3`。后续不得因版式或运行结果替换样本。

### 7.2 真实运行阻断

本轮使用产品实际 Tauri profile，而不是受控答案返回值。profile 的
`hasApiKey=true`，凭据来自 OS secret store，endpoint/model 保持配置原值。
同一 profile、endpoint 和 `glm-5.3-flash` model 的两次 Tauri `test_llm_profile`
均返回 `HTTP 503 model_service_unavailable`。因此视觉答案请求没有完成，不能产出
候选答案、约束筛查结果或人工复核集合。首份样本曾启动单文件 staging，但在答案视觉
产物生成前停止；它不计入样本统计，也不构成准确性证据。

本轮真实错误率为 **未测得（有效样本 0/0，不是 0%）**。覆盖率、答案页抽取成功率、
resolved 率、约束违反率、低置信度触发数和人工确认错误数同样为 **N/A**，不是零。
没有调模型、提示词、阈值或产品约束，也没有因为服务失败而改抽样清单。服务恢复后应
直接按该 manifest 继续逐文件 staging、约束筛查和人工复核。

## 8. 视觉服务失败态与答案语义守卫（2026-09-20）

本轮先在不调用外部视觉服务的受控 gateway 下补齐失败态证据。答案页结果现在明确分为：

| 状态 | 典型原因 | 用户动作 | 是否写入答案 |
|---|---|---|---|
| `succeeded` | 返回了可核验候选 | 在题稿中复核识别结果 | 只有通过置信度、证据和语义约束的答案才写入 |
| `failed` | HTTP 成功但返回畸形/不可核验内容 | 重试视觉识别 | 不写入，保持 `unresolved` |
| `not_executed` | 503、超时、凭据失效等 | 修复服务/凭据后重试 | 不写入，保持 `unresolved` |
| `not_executed` + `no_answer_page` | 原文件没有可识别的扫描答案页 | 手工填写 | 不编造答案 |

红色反例是“有扫描答案页 + 503”：它记录 `not_executed/service_unavailable`，和
`no_answer_page` 分开；本地题目结构、编辑和预览仍可继续，任务状态不会停在 `Working`。
重试测试实际再次进入 `extract_pdf_image_answers`，没有复用上次候选。401/403、超时、5xx
和畸形 JSON 也分别覆盖了凭据失效、未执行和执行失败的用户提示路径。

同时加入答案语义守卫，守卫位于答案页候选进入 canonical `setAnswer` 之前：TFNG/Y-N-NG、
选项/配对字母范围、实际选项集合、词数/数字限制、题号范围与连续性都会检查；不合格项
从 `acceptedAnswers` 中剔除，继续保持 `kind=unresolved`，而不是依赖置信度阈值放行。同组
选项答案完全相同的情况只标记人工复核，不擅自判错。

已冻结的 7 份 golden 结构中，95 个已知正确答案全部通过，0 条语义违反、0 条分布复核
误报；注入 `Yes please`、A-F 范围外的 `K`、五词填空三个反例全部被拦截，且命令构造器
没有生成任何写入命令。未见样本 28 份仍未启动：§7.2 记录的视觉服务 503 尚未形成可用的
外部答案候选，因此本轮泛化真实错误率仍是 **未测得（有效样本 0/0）**，不能写成 0%。
## 9. 真实模型云端自主修复基线（2026-09-20）

### 9.1 阶段 0：聚合网关与模型探测

探测对象为用户提供的 OpenAI-compatible `/v1` 网关；凭据只进入测试进程环境，未写入本文件或仓库产物。

`GET /v1/models` 返回 200（707 ms），共 3 个模型：`grok-4.3`、`grok-4.5`、`grok-4.6`，`owned_by` 均为 `xai`。不能仅凭名称推断能力，因此三者都做了最小实测。

| 模型 | JSON 最小请求 | 原生工具调用 | `image_url` | 阶段 0 结论 |
| --- | --- | --- | --- | --- |
| `grok-4.3` | 200；`json_object` 返回标准 `chat.completion`，内容为合法 JSON；2.768 s，461 tokens | 200；`finish_reason=tool_calls`，正确调用 `ping`；2.985 s | data URL 实测 503 `Service temporarily unavailable` | 文本/工具协议可用；视觉未打通 |
| `grok-4.5` | 200；`json_object` 返回标准 `chat.completion`，内容为合法 JSON；2.066 s，325 tokens | 200；正确调用 `ping`；3.105 s | data URL 与 HTTPS URL 均为 503 `Service temporarily unavailable` | 文本候选中响应最快、token 最少；视觉未打通 |
| `grok-4.6` | 首轮工具调用 200；后续 JSON 探测出现 502，未得到稳定标准响应 | 200；正确调用 `ping`；2.070 s | data URL 实测 503 | 当前网关路由不稳定，不作为完整链首选 |

网关本身存在瞬时 502/503：同一文本请求重试后 `grok-4.3/4.5` 可恢复为 200，而视觉请求三款均稳定失败。因此本轮完整链文本模型选 `grok-4.5`：理由是它在现有 `response_format=json_object` 与工具消息探测中均成功，且成功样本延迟/token 最低；`grok-4.3` 作为备选。阶段 0 **没有确认任何可用视觉模型**，这只作库存记录，本轮不启动答案页视觉批跑。

### 9.2 阶段 1：五类生产请求的真实契约实测

运行方式：`cargo test --lib real_gateway_five_semantic_contract_probe -- --ignored`（临时 ignored 探针，
位于 `src-tauri/src/lib.rs` 测试模块）。它把五类请求**全部经由产品自己的** `llm_suggestions::make_*_input`
→ `llm_gateway::run_llm_gateway` → prompt 构造 → `reqwest` → 契约校验器发出，不复刻任何 prompt 文本，
因此测的是产品实际会发的东西。模型 `grok-4.5`，凭据从 OS 凭据库读取，未落盘。

| 请求 | 首次基线 | 时延 | 证据 |
| --- | --- | --- | --- |
| `generate_pdf_reading_outline` | 通过 | 32.1 s | `title/groups/answerKey/confidence/warnings/metadata`；total 5714 tokens（reasoning 1944） |
| `verify_source_answers`（A3） | **拒绝** | 1.8 s | `source_verification_findings_missing_or_invalid` |
| `adjudicate_divergence`（A4） | **拒绝** | 2.4 s | `adjudication_rulings_missing_or_invalid` |
| `generate_authoring_candidate` | 通过 | 83.0 s | `passage/taskGroups/answerSlots/answerKey/unresolvedRegions/sourceCoverageNotes/warnings` |
| `repair_authoring_step` | 通过 | 48.8 s | 首个工具调用 `read_draft`（先读后改） |

三态判读：两条失败发生在 HTTP 200、JSON 解析成功**之后**（`llm_gateway.rs` 的顺序是
`openai_post` → `openai_chat_content` → `parse_llm_json_content` → 契约校验），`llm-calls.jsonl` 的
errorClass 也是契约类而非 `llm_http_*`。这是"执行了且不合格"，不是"未能执行"。

#### 根因：prompt 从不声明校验器强制要求的顶层信封

`source_verification_prompt` / `adjudication_prompt` 都逐条写了字段级规则（枚举、引用、页码、
逐字一致、非空理由），却通篇没有出现 `findings` / `rulings`；而两个校验器的第一步就是取这个数组，
取不到即整份拒绝。对照组决定性：三条通过的请求，prompt 里都写了
"Return exactly one JSON object with this shape: {…}"。

这个缺陷能长期存活，是因为受控假模型 `scripts/controlled-llm-service.mjs` **是照着校验器写的**
（其文件头明写"必须回 `{findings:[…]}`"）。于是被测链路的两端由同一份理解写成，两端之间的缝隙
没有任何测试能看见——与"断言自证"同类，只是换了位置。

修复只补信封声明，**不放宽任何校验规则**；并加了守卫测试
`every_prompt_declares_the_envelope_key_its_validator_requires`（先红后绿），把"prompt 必须声明
校验器要求的信封"钉成回归。

#### 未完成：真实模型侧复验被网关凭据阻断

修复后重跑同一探针，五条请求全部返回 `llm_http_401 INVALID_API_KEY`（约 460 ms/条），而首次基线
用**同一把 key、同一 endpoint、同一模型**是 200。凭据本身未被改动。因此：

- 本次修复的证据等级 = **仅单元级**；不写成"真实模型已通过 A3/A4"。
- 阶段 2（完整 CDP 链）与阶段 3（成本/时延）**未开始**，原因是凭据在阶段 1 末尾失效，不是因为
  链路不可跑。

决定性证据：用最朴素的请求单独探测凭据——`GET /v1/models`，只带 Bearer、无请求体——同样返回
`401 {"code":"INVALID_API_KEY","message":"Invalid API key"}`（485 ms）。凭据仍在 OS 凭据库中、长度 67
未变。§9.1 记录的同一个 `GET /v1/models` 在阶段 0 是 **200 并列出 3 个模型**。所以这不是请求形状、
不是 prompt 改动、也不是模型能力问题，而是**网关侧这把 key 失效了**（额度耗尽或被轮换）。
恢复条件：用户提供可用 key 或恢复该 key 的额度，之后直接重跑
`cargo test --lib real_gateway_five_semantic_contract_probe -- --ignored` 复验 A3/A4，再进入阶段 2。

## 10. Windhub / `grok-4.3` 真实模型基线（2026-09-20）

本节是对 §9 的新网关复测，不覆盖 §9 的历史结果。endpoint 为用户随后提供的
`https://windhub.cc/v1`，模型固定为 `grok-4.3`；密钥只在测试进程环境或临时 OS
secret 中使用，未写入仓库、报告或运行产物。

### 10.1 阶段 0：模型枚举与最小请求

`GET /v1/models` 返回 **200 / 199 ms**，共 12 个模型：
`gemini-3.1-flash-lite`、`gemini-3.8-flash`、`glm-5.2`、`glm-5.3`、`gpt-image-2`、
`gpt-oss-120b`、`gpt-oss-20b`、`grok-4.3`、`grok-4.5`、`grok-4.6`、`kimi-k3`、
`qwen-3.8-27b`。

选定 `grok-4.3` 的最小实测结果：

| 请求 | 结果 | 时延 | 备注 |
| --- | --- | ---: | --- |
| JSON 输出 | 200 | 14.274 s | 标准 `chat.completion`；JSON 合法；总 698 tokens（reasoning 457） |
| 原生工具调用 | 200 | 11.921 s | `finish_reason=tool_calls`；正确调用 `ping`；总 538 tokens（reasoning 243） |
| 视觉候选 `gemini-3.8-flash` | 200 | 1.777 s | `image_url` data URL 返回合法 JSON；仅作视觉库存探测，未用于本链 |

因此阶段 0 结论是：Windhub 的 `grok-4.3` 文本 JSON/工具协议可用，且网关同时
暴露了至少一个能接受 `image_url` 的视觉候选；本轮完整链只使用 `grok-4.3`，没有启动
答案页批跑。

### 10.2 阶段 1：五类生产请求

仍使用产品自己的 `make_*_input`、`run_llm_gateway` 和契约校验器。以下是当前工作树实测值：

| 请求 | 结果 | 时延 | 结构/工具证据 |
| --- | --- | ---: | --- |
| `generate_pdf_reading_outline` | 通过 | 27.293 s | 顶层含 `answerKey/confidence/groups/metadata/title/warnings`；总 3529 tokens（reasoning 1724） |
| `verify_source_answers`（A3） | 通过 | 53.133 s | 顶层 `findings` |
| `adjudicate_divergence`（A4） | 通过 | 21.869 s | 顶层 `rulings` |
| `generate_authoring_candidate` | 通过 | 32.339 s | `answerKey/answerSlots/passage/taskGroups/sourceCoverageNotes/unresolvedRegions/warnings` |
| `repair_authoring_step` | 通过 | 16.364 s | 首个工具调用 `read_draft`；顶层 `arguments/callId/tool` |

A3/A4 的响应落在当前工作树已存在的 `llm_gateway.rs` prompt 信封改动之后；这两条结果
不能当成未改 prompt 的 HEAD 基线。该改动不是本阶段新增的。本阶段没有再改 prompt，也没有
放宽校验器。完整链的真正候选请求在进入 A3/A4 前就超时，所以该污染不影响完整链的根因
判定；若要得到干净 A3/A4 baseline，应先在独立干净树复测。

### 10.3 阶段 2：真实 Tauri 产品链

使用 `fixtures/parser/demanding-reading-passage-3.pdf` 和现有 CDP harness。真实 Tauri
导入的第一遍与第二遍都通过本地稿落盘，第二遍明确记录了 `local-draft-visible-before-cloud`；
但是两遍的生产候选请求均未在网关客户端预算内完成：

| 导入 | 命令 | 模型 | 时延 | 结果 |
| --- | --- | --- | ---: | --- |
| prepass | `generate_authoring_candidate` | `grok-4.3` | 134.043 s | `llm_timeout_budget_exhausted` |
| 被断言链 | `generate_authoring_candidate` | `grok-4.3` | 144.323 s | `llm_timeout_budget_exhausted` |

对应 artifact 为
`artifacts/e2e-cdp/run-cloud-repair-chain-2026-09-20T13-22-27-910Z`；两个作业的
`llm-calls.jsonl` 都只有上述候选失败记录，随后落盘的 cloud candidate 是
`status=not_run / reasonCode=CLOUD_DISABLED`。没有 `repair_authoring_step`、
`read_source`、`apply_edits`、`record_ruling` 或 `finish` 回合，因此本次不能声称模型
完成了自主修复，也没有 sourceAnchors/baseVersion 自修正证据。脚本在确认第二次候选失败后
停止，避免无意义地等待 900 秒的 repair 轮询；Tauri/Node 进程与临时 OS secret 已清理。

### 10.4 阶段 3：成本与时延结论

一次 harness 导入包含 **1 次候选请求**；由于脚本为真实初稿派生场景而实际导入两遍，
本次完整运行共发出 **2 次候选请求、0 次修复请求**。失败记录没有保存模型 usage，故真实
候选 token 数为 **不可得**，不能用阶段 1 的小输入 token 数冒充完整导入成本。两次候选等待
合计约 **278.366 s**，还不包括本地解析与 UI 导入；对产品而言已是候选单步约 2–2.5 分钟、
且仍未进入修复链的产品级时延问题。

当前可执行结论：真实 `grok-4.3` 能驱动五类请求的协议层，但**不能在现有客户端预算内
驱动这条真实云端自主修复链**；卡点是完整候选请求的时延/预算，不是工具校验放宽、视觉能力
或 `read_source` 语义。下一步建议先拿候选 payload 做独立的请求大小/服务端响应时延分析，
再决定是否需要模型路由、请求裁剪或预算策略调整；本轮不改这些行为。

### 10.5 复核这次失败的落库状态：一次真实超时在界面上等于"未启用云端"

§10.3 把候选失败后的 `status=not_run / reasonCode=CLOUD_DISABLED` 记成中性事实。直接查同一次
运行的库（`artifacts/e2e-cdp/run-cloud-repair-chain-2026-09-20T13-22-27-910Z/appdata/data/authoring_hub.db`）
后可以确定这不是中性的，它是一个用户可见缺陷：

| 行 | 字段 | 实测值 |
| --- | --- | --- |
| `processing_jobs_v2` | `cloud_status` | `failed` ✅ |
| `processing_jobs_v2` | `last_error_code` | `llm_timeout_budget_exhausted:llm_http_timeout:error sending request for url (https://windhub.cc/v1/chat/completions)` ✅ |
| `processing_jobs_v2` | `progress_json` | `{"cloudEnabled":true,"cloudProfileId":"real-repair-chain-probe",…}` |
| `recognition_batches_v1` | `cloud_status` | `not_run` ❌ |
| `recognition_batches_v1` | `cloud_reason_code` | `CLOUD_DISABLED` ❌ |
| `recognition_batches_v1` | `stages_json.cloud.message` | 「本次导入未启用云端识别。」 ❌ |

**同一行的 `cloudEnabled` 是 true，`cloud_reason_code` 却是 `CLOUD_DISABLED`。** 任务行知道真相，
批次行说的是另一回事。

前端读的是**批次行**：`RecognitionPanel` 把 `view.cloudStatus` 交给
`describeVerificationStatus`；`recognitionClient.ts:224` 把 `not_run` 归一成 `not_started`，
于是走到 `:559` 的分支，返回 **「题稿已生成，可以开始编辑」**——与"用户压根没启用云端"逐字相同。
那次 134 秒的真实超时在界面上完全不存在。

成因是接缝而不是疏漏：批次行由本地周期建出，而本地周期按设计以 `cloud_enabled = false` 运行
（`scheduler.rs:736` 的注释明写这一点），因此它写下的 `CLOUD_DISABLED` 在当时是诚实的；
`scheduler.rs` 原本指望"下面的 advance 会用真实修复状态覆盖它"，但 `write_batch_repair`
只写 `repair_json`，从不碰这一格，而候选失败时修复循环压根没启动。于是没有任何代码改写它。

值得注意的是 `describeVerificationStatus` **本来就有**这一格的正确文案：
`unusable → unavailable → 「云端校验暂时不可用，不影响继续编辑」`。缺的不是文案，是把真实终态
送到批次行。

修复：`store::write_batch_cloud_failure` + `scheduler::batch_cloud_failure_for_job`，
只在"云端起了、但没交出可用结果"时改写批次行的 cloud 一格，原因码复用既有
`classify_cloud_error`（超时 → `unusable` / `MODEL_TIMEOUT`），不新造分类、不动其余三路。
成功/部分成功路径**本轮未改**：那种情况下批次行同样停在 `CLOUD_DISABLED`，但故事由
`repair_json` 讲，且改动会影响既有 13/13 CDP 断言，单独排卡。

红色证据来自真实运行产物（上表），不是构造出来的；两条单测把修复钉住。

## 11. Welfare / `grok-4.6` 复测（2026-09-20）

endpoint 为 `https://welfare.darkforger.com/v1`，凭据只进入测试进程，没有写入仓库或运行
产物。`GET /models` 返回 200 / 547 ms，共 13 个模型，目录中明确包含 `grok-4.6`。

阶段 0 的两个最小探测均一次成功：

| 探测 | 结果 | 时延 | usage |
| --- | --- | ---: | ---: |
| `response_format=json_object` | 200，标准 `chat.completion`，内容为合法 JSON | 6.679 s | 2551 total（343 reasoning） |
| 原生工具调用 | 200，`finish_reason=tool_calls`，正确调用一次 `ping` | 5.601 s | 533 total（231 reasoning） |

因此该 endpoint/model 的基础 JSON 与工具消息协议可用。随后没有用手写 prompt 代替产品请求，
而是临时 ignored 探针调用产品自己的 `generate_cloud_authoring_candidate_raw`：同一份
`fixtures/parser/demanding-reading-passage-3.pdf`、当前 `make_cloud_authoring_candidate_input`、
当前 prompt、当前 validator，profile HTTP 预算设为 clamp 上限 300 s。

完整候选没有进入模型生成。网关在 7.534 s 返回 HTTP 402
`user_quota_insufficient`：可用额度 `2.132969`，网关为该请求预留/预测需要
`6.01536 credits`。产品命令输入缓存为 12,414 bytes；这不是 wire payload 大小，因为实际
HTTP body 还包含内联 PDF/base64。没有输出缓存、没有 completion，也没有完整请求 token usage。

结论边界：

- 已证明：`grok-4.6` 在该网关上能做最小 JSON 和 native tool call。
- 未证明：完整 candidate 能在 300 s 内完成；更未进入 repair loop，仍没有
  `read_source` / `apply_edits` / `record_ruling` / `finish` 的真实模型证据。
- 本次阻断是网关明确的额度不足，不是客户端超时、prompt 校验失败或模型协议不兼容。
- 没有通过压低输出上限制造一个与真实产品请求不同的“通过”。临时探针运行后已从源码移除。

## 12. 最终提交后回归（2026-09-20）

- 受控 `tauri-cdp-cloud-repair-chain.mjs` 第一次被内容哈希护栏拒绝：旧 exe 不是当前源码/前端的构建产物；这不是链路失败。
- 按仓库既有 `scripts/e2e/build-app.mjs` 重建后，用同一份
  `fixtures/parser/demanding-reading-passage-3.pdf` 重跑，13/13 场景通过，覆盖本地初稿、云端自主修复、原文件证据、模型收到反馈后自修正、修复进度、画布刷新、剩余任务可操作、人工编辑重开、导出和学生端加载。
- 同次 harness 的 `defect-retry-cannot-rerun` 探针为 `not-reproduced`；不把它扩大解释为新的行为保证。
- 合并后全量验证：Rust `874 passed / 0 failed / 11 ignored`；Vitest `315 passed / 0 failed`。
- 回归完成后删除本次运行目录、构建日志/清单、旧 dist 临时目录、前端 dist、Rust target 和临时目录；凭据没有写入仓库或保留在验证产物中。

## 13. Source coverage：原文声明题域与 canonical 题号闭合（2026-09-20）

这张卡补的是阅读侧第三个独立视角：不能只看本地 task group 或云端候选，因为两者可能同时漏掉同一道题。
质量评估从 `DocumentIRV2.pages[].lines[].text`（空行页回退 `spans[].text`）收集原文自己的题域声明：
`Questions ...` 与带有 answer/write 上下文的 `boxes ...` / `box ...`，再与 canonical
`answerSlots[].questionNumber` 做集合比较。

状态语义固定为三态：

- `complete`：声明集合与 canonical 集合相同；
- `missing`：集合不相同，写入 `SOURCE_QUESTION_COVERAGE_MISSING` blocking issue，质量状态阻断；
- `undetermined`：没有可解析声明、声明无法解析或 physical shadow 不可用，写入
  `SOURCE_QUESTION_COVERAGE_UNDETERMINED` warning，绝不写成 complete。

反例先行证据：

- 人为从 canonical 草稿删掉 q15，而原文声明仍为 14–15 → `missingQuestionNumbers=[15]`，质量状态 `blocked`；
- `Questions are based on the passage below.` → `undetermined`，不产生伪造的“完整覆盖”；
- `questions1-10`、只有 `Write your answers in boxes ...`、空 `lines` 回退 `spans` 均有单测；
- 第一次真实 Tauri CDP 链还发现 pdfium 会把多位题号抽成 `Questions 2 7 – 3 1`，该真实反例先红，随后只在 coverage 数字声明入口合并相邻数字间的字形空格。

最终真实链证据：`fixtures/parser/demanding-reading-passage-3.pdf` 的 physical shadow 得到声明题号
`27..40`，canonical 同为 `27..40`，`missing=[]`、`extra=[]`、`status=complete`；重建当前 exe 后
`tauri-cdp-cloud-repair-chain.mjs` **13/13 passed**。这条逻辑不依赖网关额度、视觉服务或 DOCX 样本，也跳过显式 listening modality，避免把听力合同提前定死。
