# Phase 4：8 份真实 PDF 可执行验收门

## 目的

本门禁把 Phase 0 冻结的 8 份 IELTS PDF 逐份送入真实的离线链路：V1 PDF parser → V1 split/authoring → PDF physical `DocumentIRV2` → `IeltsAuthoringIRV2` shadow → `QualityReportV2`。它不调用 LLM，不修改 V1 基线，不启用任何 V2 feature flag，也不把 V2 送入 UI、export、runtime 或 NAS。

验收清单由 `fixtures/golden/phase4-eight-pdf-acceptance.json` 固定；实现入口是仅在 `cfg(test)` 下编译的 `src-tauri/src/ielts_grammar/real_pdf_acceptance.rs`。运行产物写入已被 Git 忽略的 `tmp/phase4-real-pdf-acceptance/`，包含每份 PDF 的实际 V1、physical V2、authoring V2 和总报告。

## 验收内容

- manifest、metadata、真实 PDF 的 SHA-256/大小与 V1 baseline 身份一致；
- 当前可执行 V1 输出摘要与冻结 baseline 一致；
- physical shadow 来自真实 PDF，且 QualityReportV2 明确报告 physical shadow available；
- metadata 中 task group、slot、option bank、response group 的结构真值进入 V2；
- Organisational Design 四个 `Choose TWO` 均为一个共享 prompt、两个 slot、exact=2、unordered set；
- Petri Q20–25 仅引用一个共享 `List of People` option bank；
- Organisational 的答案页为 image-only raster；当前本地链路没有可执行 OCR/vision 结果时，answer key 必须保持 unresolved，QualityReportV2 必须诚实保持 `blocked`/`review_required`，runtime compiler probe 不得借助 golden oracle 或人工答案注入伪装为通过；
- Western Celebrity 证明题页先于 passage，且题页不混入 passage；
- Listening to the Ocean Q9–13 为 single choice，内联选项严格为 A–D；
- Chili Peppers 的 passage image 从 physical asset 绑定到 passage；
- answer/explanation 页文字与 raster asset 均不进入 passage。

如果结构真值失败，门禁会先完整写出 8 份证据，再以非零状态退出。若 QualityReportV2 在验收失败时声称 `ready`，会额外产生 `FALSE_READY_FORBIDDEN` 失败；因此本门禁不会伪造 Ready。

## 2026-08-10 首次执行状态（历史记录）

门禁已真实执行 8/8 PDF，physical shadow、V2 authoring 和 QualityReportV2 均有产物；所有 quality state 均为 `blocked`，没有出现 false Ready。当前门禁按设计返回失败，明确暴露了尚未达到 Phase 4 真实语料验收标准的缺口：

- Chili/Fishbourne 的 Q7–13 被误判为 `plan_map_label_completion`，且 Chili passage image 未被 physical extractor/authoring passage 绑定；
- Petri Q20–25 仍把 A–D 复制到逐 slot response，没有形成单一共享 `List of People` bank；
- Western passage 仍包含位于 passage 之前的 question page，且 group-2 类型与 metadata truth 不一致；
- 多数普通题组仍输出逐 slot response/inline options，未满足 metadata 中一个 response group/共享 option bank 的结构契约；
- Organisational 四个共享双 slot 结构已形成，但 runtime/V1 compatibility compiler probe 因空 answer key 与共享控件协议失败，不能宣称可编译；
- Organisational 第 7–8 页当前只产生空白 glyph/line，且 physical shadow 仍误报 `born_digital`、未生成 `requiresOcrRegions`；在 OCR 计划与真实 answer-key parser 完成前，该 fixture 必须保持阻断，验收不得注入人工答案绕过；
- Ocean Q9–13 的每个单选 response 都保留了严格 A–D，此专项事实已通过。

机器可读的精确失败码和 actual/expected 值位于 `tmp/phase4-real-pdf-acceptance/report.json`。这些结果表明真实 physical → authoring → quality 链路可执行，但当前 Phase 4 架构证明尚未全部通过；在上述缺口修复前，不应把 Phase 4 真实语料状态标为完成或 Ready。

## 2026-08-12 修复后复核状态

已在检查点提交后重建最新 Rust CLI，并重新执行完整 acceptance。结果：

- 8/8 真实 PDF 全部通过，`failureCount=0`；
- Phase 4 Rust grammar/quality suite：73 passed、2 ignored、0 failed；
- 海洋正文的同页/后续正文段落均纳入 passage source evidence；
- Organisational 四个 `Choose TWO` 共享 prompt/two slots 结构保持通过；
- narrow empty table layout artifact 与扫描答案说明文字进入带稳定 reason 的 `ignored_with_reason` ledger，不再伪装成未覆盖题目；
- V1 仍是 authoritative，V2 flags 仍关闭，image-only answer page 仍不注入人工 OCR/answer oracle。

机器可读结果：`tmp/phase4-real-pdf-acceptance/report.json`，其中 `passed=true`、`fixtureCount=8`、`failureCount=0`。

## 命令

```powershell
node scripts/verify-phase4-eight-pdf-acceptance.mjs
npm run verify:phase4:grammar
```
