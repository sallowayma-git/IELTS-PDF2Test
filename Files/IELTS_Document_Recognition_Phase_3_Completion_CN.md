# IELTS Document Recognition Phase 3 完成记录

本记录对应总计划 Files/IELTS_Document_Recognition_Overhaul_Plan_CN.md 的
16.4「DOCX 富结构层」。Phase 3 以 C-001 的安全 package reader 为基础，完成
C-002～C-009 的 DOCX 富结构 shadow 能力。V1 仍是当前 authoritative path；Phase 3
只增加 V2 documentIrV2Shadow 旁路产物，默认关闭，不覆盖 document-ir.json，
不进入 authoring/export 主链。

## 已完成范围

- C-002 style/numbering：读取 document defaults、basedOn 链、paragraph/run style
  和 direct formatting；解析 abstractNum/num/ilvl、level text、format、start
  override、indent/hanging。paragraph 直接 `numPr` 优先于 style/basedOn 继承值；
  以 numId/ilvl 状态机维护计数并重置更深层级，展开 `%1`～`%9`，支持 decimal、
  upper/lower letter、upper/lower Roman、decimalZero 与 bullet。渲染 label 进入首个
  line/span/glyph、render binding 语义文本和 `numberingFacts.renderedLabel`。
- C-003 paragraph/run：保留 OOXML 原始路径和 source anchor；不做全局 whitespace
  collapse；保留 xml:space="preserve"、tab、line/page break、w:cr、soft/
  non-breaking hyphen、field instruction、hyperlink、w:ins；field instruction 与
  display text 分离；w:del 默认忽略并产生 warning。semantic-only 页面按 block/
  text box source order 构造，并落实 page break、非 continuous section break。
  `w:br`/`w:cr` 不再只是同一 line 内的换行字符，而会生成真实的独立 line/region；
  page break 继续分页，自动编号 label 只在首 segment 出现。table cell 会收集所有
  分段后的 content region ids。
- C-004 table：读取 tblGrid/tr/tc，保留空 cell、gridSpan、vMerge restart/
  continue、rowSpan、cell width/type、row height/rule、table/cell padding、table/cell
  border、shading、vertical alignment、cell paragraph/list、nested table；V2 table
  topology 使用 detectionMode=ooxml。Rust/TypeScript/JSON Schema 的 TableCellV2
  同步提供 `widthPt`、`rowHeightPt`、`rowHeightRule`、`verticalAlignment`、
  `paddingPt`，布局构造使用显式行高和 padding origin，不再固定 28pt/3pt。
- C-005 image/drawing：读取 inline/anchor extent、位置、wrap、relativeHeight、
  alt/title、rotation/crop、r:embed/r:link；一个 drawing 的多个 relationship target
  均保留，raster preview 优先用于 region 绑定，diagram/chart XML 仍作为结构资产；
  word/media/* 原始 bytes 按 SHA-256 写入 assets/shadow/docx/，并输出
  AssetDescriptorV2 与 OOXML anchor。
  外部图片不触网，产生 DOCX_EXTERNAL_ASSET_MISSING error 并将
  parser.options.publishBlocked 设为 true。
- C-006 text box/floating/VML：抽取 w:txbxContent 和 VML shape text，保留独立
  paragraph/run；缺少完整浮动坐标时保留 OOXML 顺序并产生
  DOCX_FLOATING_ORDER_AMBIGUOUS warning，不伪造几何。
- C-007 SmartArt/chart：抽取 diagram/chart XML 中可访问文本和 relationship；
  无可验证 preview 或 render provider 时输出 `UNSUPPORTED_DOCX_COMPOSITE_DRAWING`
  error、保留 Figure region、结构、关系、原始 XML asset 和 anchor，并设置
  `publishBlocked=true`；有可验证 raster preview 时由 drawing relationship 绑定
  preview asset，同时保留 composite 的结构文本与 relationship 记录。relationships、drawing、
  composite、numbering、field、header/footer/notes/theme 等 auxiliary facts 均进入
  shadow artifact；word/embeddings/* 只登记 warning，绝不执行或导入。
- C-008 section columns：读取 page size、orientation、margins、columns width/
  spacing、separator、header/footer references，并写入 shadow parser options；保留
  OOXML 原始 page size 和显式 orientation，不根据方向猜测交换宽高。
- C-009 render assist：默认 semantic-only；只有
  EPIC8_DOCX_RENDER_ASSIST=1 才调用配置的 LibreOffice-compatible provider 执行
  DOCX -> PDF conversion。只有 provider 成功且产出带 `%PDF-` magic 的文件，并且
  PDF geometry extraction 成功，才设置 geometryAuthority=render-assisted；无
  provider、无输出、伪 PDF 或 geometry extraction 失败均回退
  geometryAuthority=ooxml-semantic-only，并输出显式 issue，不冒充 render geometry。
  `render-assisted-two-column-options.docx` 及其检入的真实有效 provider-output PDF
  构成无需 LibreOffice 的端到端证明：renderer 通过函数注入，不修改进程环境；PDF
  glyph/line 几何进入两列 region 和 reading order；同一 OOXML 段落被渲染拆成左右
  视觉行时，两行均绑定回同一 `ooxmlPath`，且 source anchor 的 `docx_ooxml`
  variant 保留含 tab/多空格的原始语义文本。

## V2 shadow 产物

在 documentIrV2Shadow 开启且输入为 DOCX 时，authoring parse command 额外写入：

- document-ir-v2.shadow.json
- document-ir-v2.shadow.compare.json
- assets/shadow/docx/*
- 失败时 document-ir-v2.shadow.error.json

shadow JSON 经过严格 DocumentIRV2 反序列化校验。页面、glyph/span/line/region、
table/cell、asset、coverage ledger 均带 docx_ooxml source anchor、OOXML path 和
source hash。现有 V1 parser 仍负责生成并写入 document-ir.json。

shadow 生命周期已经闭环：DOCX shadow 先在独立 staging root 生成 artifact、compare
和 `assets/shadow/docx/`，三者齐备后才以 backup/rename/rollback 作为一个 bundle
替换；解析、写入或 commit 失败会回滚并保留上一套完整成功 bundle。若文件系统在
回滚期间也失败，则返回 `DOCX_SHADOW_ROLLBACK_FAILED`，保留可人工恢复的 backup
目录并 fail closed，不静默删除旧 bundle。失败同时写入 error artifact；只要该
error marker 尚在，authoring V2 shadow 不消费旧 physical shadow。下一次成功
commit 后删除 error marker。authoring shadow 读取 physical shadow 前仍须同时匹配
当前 MainQuestion 的 sourceFileId 与 sha256。

## Fixture 与门禁

`scripts/generate-phase3-docx-fixtures.py` 以固定 ZIP timestamp 生成并检入 14 个
相互独立的 adversarial/degraded DOCX package：

- bad-01～bad-05：zip-slip、case-fold duplicate、encrypted entry、缺 content
  types、坏 internal relationship；
- bad-06～bad-10：external image、empty cell、无 restart 的 vMerge、缺 offset 的
  floating text box、VML text box；
- bad-11～bad-14：无 preview 的 SmartArt/chart、two-column section、raw
  whitespace/tab/page-break/fldSimple。

同时保留 5 个原命名兼容 fixture：

- docx-external-image.docx
- docx-floating-text-box.docx
- docx-section-columns.docx
- docx-smartart.docx
- docx-table-merged-cells.docx

另检入 `render-assisted-two-column-options.docx` 和
`render-assisted-two-column-options.provider-output.pdf` 作为 C-009 正向 vertical
slice；它不计入 14 项 adversarial matrix，但 Phase 3 gate 强制检查两者存在且
provider output 具有 `%PDF-` magic，Rust 测试继续验证其完整 PDF 几何提取结果。

`phase3-bad-word-fixtures.json` 的 14 项 source 全部指向上述实际文件。Rust matrix
test 逐项打开这些 package，并断言既定 reject code、publishBlocked/issue、table
topology/source anchor、VML geometry、SmartArt/chart XML asset、columns、page-break、
whitespace 和 field boundary；不再用 `rust-in-memory-*` 代替真实 fixture。另保留
rich embedded-image、external-image 和 5 个兼容命名 package 回归。

Phase 3 专用门禁：

    npm run verify:phase3:docx

该门禁检查 module/fixture/doc 文档完整性、14 个坏 Word 场景、所有 Phase 0/1 V2
flags 默认 false、V1 document-ir.json 路径仍存在、Rust formatting，以及以下
回归：

    cargo test --manifest-path src-tauri/Cargo.toml --lib docx_ingest -- --nocapture
    cargo test --manifest-path src-tauri/Cargo.toml --lib docx_facts_shadow -- --nocapture
    cargo test --manifest-path src-tauri/Cargo.toml --lib docx_ooxml -- --nocapture
    cargo test --manifest-path src-tauri/Cargo.toml --lib complex_docx_fixture_reaches_authoring_ir -- --nocapture

本次收敛实测结果：

- `docx_facts_shadow`：9 passed，含 14 项 adversarial fixture matrix、1 项真实
  provider-output PDF render-assisted vertical slice；
- `docx_ingest`：21 passed，含 numbering format boundary、合法 PDF/无输出/伪 PDF renderer；
- 新增 DOCX failed-attempt bundle preservation、commit 中途失败逐字节恢复、rollback
  自身失败稳定错误码、`w:br`/`w:cr` 分段和编号不重复回归；
- stale-shadow lifecycle：3 passed，含单项删除失败时继续清理其余产物；
- `docx_ooxml`：5 passed；复杂 DOCX authoring 集成：1 passed；
- `verify:phase3:docx-package`：27 个 Rust lib tests 执行、0 failed；
  `verify:phase3:docx`：36 个 Rust lib tests 执行、0 failed；两个脚本均先枚举
  最小测试数并拒绝 0-test/过滤器漂移；`cargo check`、`npm run check`、
  `npm run build` 全部通过。

专项门禁之外的全量 `cargo test --lib` 统计与环境依赖失败由 Phase 0-4 总审计记录
统一维护；本记录不把专项绿色扩大解释为全量绿色。Phase 3 新增与 DOCX 相关的
测试均在上述专用门禁中执行并通过。

## 边界与后续

- 未修改 V1 baseline 的语义、输出 schema 或 authoring/export authoritative route。
- 未开启任何生产 V2 flag；render-assisted 仅为显式开发诊断能力。
- 新解析失败时旧完整 bundle 保留用于诊断/恢复，但 error marker 会阻止它伪装为
  本次成功输出；source/hash 变化时同样不会被 authoring 消费。
- 没有 renderer/preview 时 floating drawing 保留可审计 warning/review；SmartArt/chart
  composite 升级为 blocking error，保留 Figure/asset contract 且不输出虚假坐标。
- Phase 4 可在此 shadow 事实层上继续做跨格式统一、question-level semantic
  projection 和人工 review UX；不得绕过 V1/V2 边界直接替换当前生产链。
