# IELTS Document Recognition Phase 2 完成记录

本记录对应总计划 `Files/IELTS_Document_Recognition_Overhaul_Plan_CN.md` 的
16.3、PR-03 和 PR-04。Phase 0 V1 baseline、现有导出链路和默认关闭的 flags
保持不变；本阶段只把 PDF 物理层写入 V2 shadow artifact/job store。

## 已完成

### PR-03：line / region / order shadow

- 新增 `src-tauri/src/pdf_ingest/line_builder.rs`：按 baseline、字体尺寸、垂直重叠和水平间隔做 adaptive line clustering；同一基线上的跨栏 glyph 会先按大间隔拆线。
- 生成 page-level `spans`、`lines`，保留 `glyphIds`、`spanIds`、bbox、baseline、writing mode、line height、hard break、confidence 和 `SourceAnchorV2`。
- 新增 `region_builder.rs`：projection gutter/column candidate；过滤顶部 banner 对 gutter 的干扰；保留 `columnIndex`、`sectionIndex`、child line ids 和 anchor。
- 新增 `reading_order.rs`：按 column-major 与 y/x 候选生成主顺序和 alternatives，写入 `readingOrderRank`、confidence；不改变 V1 文本或 authoring。
- 新增 `compare_report.rs`：旁路写入 `document-ir-v2.shadow.compare.json`，逐页比较 V1/V2 文本、token diff、line/region/table/asset 数量和 anchor coverage，并明确 V1 authoritative policy。

### PR-04：asset / path / table / selective OCR shadow

- `pdf_facts_shadow` 采集 native vector path、矩形/曲线命令、top-left bbox 和 path source anchor。
- 嵌入 image XObject 写入 `jobs/<jobId>/assets/shadow/`；DCT/JPX 保留原格式，ASCII85/Flate 的 8-bit Gray/RGB payload 封装为 PNG；descriptor 写入 mime、尺寸、hash、byteLength、relativePath 和 source anchor。
- PDF annotation/AcroForm Widget 以 `form` region、object id、bbox、source anchor 和 coverage entry 保留；普通 annotation 也保留为 `unknown` object region，不触发 V1 authoring。
- vector-heavy page 生成 job-store `vector_render` SVG fallback asset，并以 `diagram` region 连接路径对象和 source anchor；无法得到精确 image placement 时显式标注 full-page fallback confidence。
- `table_detector.rs` 同时支持 ruling-lines/矩形 grid 和 borderless text-alignment candidate；cell 保留 row/col/span、content region ids、border evidence、confidence 与 anchors。
- `ocr_router.rs` 检测 glyph overlap、native/OCR conflict 和 PDF content 中的白色/invisible text；生成 `PdfSelectiveOcrPlanV1`，native text 始终 primary，`nativeTextOverwrite=false`，`llmRepairEnabled=false`。
- overlay 扩展为 source、glyph、line、region、reading-order、table/assets、OCR、unassigned 八层，并由现有 debug command 输出到 job 的 `debug/` 目录。
- coverage ledger 继续保留 glyph，并追加 line/region/path/table/annotation/widget 的 unassigned disposition；本阶段不把任何 shadow 节点写入 V1 authoring/export。
- PDF 资源边界除单图 `MAX_IMAGE_PIXELS` / `MAX_EMBEDDED_ASSET_BYTES` 外，增加
  document-wide 累计预算。image XObject 先按 object reference 去重再解码，同一 blob
  跨页复用不重复计费；累计 decoded pixels 和实际持久化 asset bytes 分别以
  `PDF_RESOURCE_LIMIT_TOTAL_IMAGE_PIXELS`、`PDF_RESOURCE_LIMIT_TOTAL_IMAGE_BYTES`
  稳定失败，并在 preflight 同时记录 actual totals 与 limits。边界测试证明恰好达到
  上限通过、再增加一个单位失败。

## 运行边界

- `documentIrV2Shadow` 仍为默认关闭、开发环境显式 opt-in；现有 `document-ir.json` 写入顺序和内容未改写。
- OCR 目前是路由/计划 shadow，不捏造 OCR engine 输出；PDF per-question LLM repair 继续强制关闭。
- compare report 的差异只供 review/diagnostics，不作为 authoring 输入。

## Phase 2 验收覆盖

| 验收点 | 覆盖方式 |
| --- | --- |
| 文本不因 block collapse 丢行 | 15 份 synthetic PDF 逐份提取、line/span/region 严格 schema round-trip；有 glyph 的 page 必须有 lines/spans |
| 合成双栏/三栏顺序 | `pdf-two-column.pdf` 与 `pdf-three-column.pdf` 检查 column 数、reading order 集合和 child line 完整性 |
| vector/path/table | `pdf-ruled-table.pdf`、`pdf-borderless-table.pdf` 检查 path、table mode、cells 和 anchors |
| image/figure asset | `pdf-image-only.pdf` 生成可验证 PNG；`pdf-vector-diagram.pdf` 生成可验证 SVG fallback；检查文件、mime、byteLength、SHA-256、page assetIds 与 visual region |
| widget/annotation | 临时 AcroForm Widget PDF 检查 `form` region、bbox/source anchor、annotation summary 与 coverage ledger |
| hidden OCR mismatch | `pdf-hidden-ocr.pdf`、`pdf-native-ocr-conflict.pdf` 检查 mismatch warning 和 native overwrite=false |
| V1/V2 compare | compare report 与 shadow artifact 同目录生成；V1 remains authoritative、V2 不进入 authoring |
| overlay diagnostics | 检查八个 `data-layer` 与 debug command 返回的 layer manifest |
| document-wide resource budget | 唯一 image XObject 累计 pixels/bytes、稳定错误码、exact-limit/next-unit 边界测试与 preflight metadata |

## 验收命令

```text
npm run verify:phase0:strict
npm run check
npm run build
npm run verify:phase1:schema
npm run verify:phase2:shadow
cargo test --manifest-path src-tauri/Cargo.toml
```

`verify:phase2:shadow` 会检查默认 flags、Rust formatting，并先枚举再执行
`pdf_facts_shadow` 与 `pdf_ingest` 的完整 Phase 2 回归集；当前实测分别为
19 和 8 个 Rust lib tests，合计 27/27 passed、0 failed。覆盖 15 份 synthetic、
登记的 private-real corpus、vector/widget 边界、OCR merge、坐标和 reading-order。
门禁会拒绝 0-test/过滤器漂移，并检查子进程启动错误。Rust 全量测试中的既有
环境失败项仍按 Phase 0/总审计记录处理，不归因于 Phase 2；本阶段新增的 shadow
测试不得失败。

PDF shadow artifact、compare report 与 shadow assets 使用独立 staging bundle；
commit 中途失败会逐项回滚。若文件系统拒绝回滚，门禁错误会包含
`PDF_SHADOW_ROLLBACK_FAILED` 并保留 backup 目录，避免把上一套成功 bundle 静默删除。
门禁还通过安装第二个组件前的 fault injection，逐字节验证 artifact、compare 和旧资产
在中途失败后完整恢复，并单独验证 rollback 自身失败时稳定返回上述错误码。

## 后续边界

本阶段没有启用生产 shadow flag，也没有把 physical layer 误当成 IELTS semantic
layer。私有样例的业务命名（例如具体 passage 名称）不写入仓库 fixture；接入私有
corpus 时，应使用同一 shadow artifact、compare report、asset hash 和 overlay
验收链路继续验证，不改变 V1 baseline。
