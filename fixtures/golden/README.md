# Golden corpus（Phase 0）

这里保存文档识别重构的回归基线。`manifest.json` 是唯一入口，`metrics.json` 保存来自总计划 17.3–17.4 的指标和 hard gates；它区分三类数据：

- `fixtures`：工作区中实际存在、可以校验 SHA-256 的 fixture。
- `requiredPrivateCorpus`：计划要求的授权真实语料。本次从用户提供的 272 份 PDF 中以固定种子随机抽取 8 份；若来源缺失，必须保持 `missing-source`，不能填写猜测的题组或答案。
- `plannedSyntheticFixtures`：计划中的合成/对抗场景，只有生成源文件和标注后才能转为实际 fixture。
- `private/legacy-reference.json`：来自 `E:/reading-exams` 旧版 JS/HTML 的 reference-only 证据索引；它记录题组和 hash，但绝不替代真实 PDF。

## Metadata 契约

每个实际 fixture 的 metadata 使用 `GoldenFixtureMetadataV1`，正式 schema 位于 `schema/golden-fixture-metadata-v1.schema.json`：

```json
{
  "schemaVersion": "GoldenFixtureMetadataV1",
  "fixtureId": "stable-id",
  "source": {
    "path": "fixtures/parser/example.pdf",
    "sha256": "sha256-hex",
    "sizeBytes": 1234,
    "format": "pdf"
  },
  "expected": {
    "pageRoles": [{ "pageIndex": 1, "roles": ["passage", "question"] }],
    "taskGroups": [{ "id": "group-1", "displayRange": [1, 3], "kind": "...", "slotIds": ["q1"] }],
    "slots": [{ "id": "q1", "displayNumber": "1", "responseType": "radio" }],
    "assets": [{ "id": "page-1-raster", "type": "page_image", "required": true }]
  },
  "knownIssues": ["..."],
  "baseline": { "v1Path": "fixtures/golden/baseline/v1/example.json" }
}
```

`expected` 是人工/来源证据支持的目标标注，不是 V1 的实际结果。V1 结果放在 `baseline/v1/`，用于比较当前行为和未来 V2 行为；它不能反向证明识别正确。

本次 `private-random-*` 样本的初始结构标注由捕获到的 V1 IR 生成，并在 metadata 的 `knownIssues` 中明确要求人工复核；它们已满足可复现基线契约，但在复核完成前不应作为最终正确性 oracle。

当前合成 corpus 位于 `synthetic/`，包含 15 个 PDF 和 5 个 DOCX。它们覆盖多栏、旋转页、image-only、hidden OCR、native/OCR 冲突、表格、矢量图、地图热点、流程图、浮动文本框、外链图片、SmartArt、分栏和合并单元格。生成器是确定性的：

```text
npm run generate:phase0-synthetic
npm run register:phase0-synthetic
npm run capture:phase0-baseline
```

## 隐私与提交规则

真实 IELTS 题源只允许放入受授权的私有目录或 CI secret store。`private-real/*.pdf` 已由 Git 忽略；不要把未授权的题源、答案页或音频提交到公开仓库。提交 metadata 时应保留 hash、结构标注和问题说明，必要时不提交源文件本身。

## 验证

```text
npm run register:phase0:private -- --source-root "C:\Users\lenovo\Desktop\working space\0.3.1 working\ReadingPractice\PDF" --seed 20260809 --count 8
npm run verify:phase0
npm run capture:phase0-baseline
npm run annotate:phase0:private
npm run verify:phase0:strict
```

`--strict` 会把缺少私有语料或计划中的合成 fixture 视为未完成，但普通验证仍会检查当前可用基线的完整性。
