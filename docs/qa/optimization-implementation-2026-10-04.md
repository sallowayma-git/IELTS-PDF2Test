# 并发审计优化实施：2026-10-04

基于 `7713f5c` 与 `optimization-audit-2026-10-04.md` 的四项优先建议实施。三个子代理分别负责前端刷新、摘要/导入、PDF 证据范围；主代理整合实际 Tauri 命令与单行摘要接口，并检查回归。未调用收费模型，完整本地校核历史保持原样。

## 已实施

1. **数据库摘要不读取全文**：`library/repository.rs` 的列表与 get_item 使用 `canonical_ds_json IS NOT NULL` 布尔投影；全文编辑读取保持 get_canonical_ds，首次 Part 推断仍按需读取正文。覆盖 null、空字符串、长正文、软删除与完整正文读取兼容性。
2. **单题进度与题库增量读取**：新增 `get_library_item_processing` 和 `get_library_row` 的真实 Tauri 命令。单行保留原始 job 元数据、legacy 摘要、V2 摘要、Part 和回收站语义，不读取完整 JobDetail 或正文 artifacts。回收站单行按首次列表规则排除已删除 V2 字段，避免事件和聚焦刷新之间标题/Part 变化。初始列表与单行读取按库条目查 processing，不要求 worker job ID 与 item ID 相同；这只是读取契约，既有取消/永久删除操作的任务身份处理不在本次变更范围。
3. **前端先订阅、再读一次初始快照**：题库首读仍包含四源；事件只重建变化条目。初始读取期间的事件排队，单行读取期间的新事件要求再读，旧响应不能覆盖取消或终态。聚焦、手动刷新、删除恢复仍有完整快照，单行失败回退快照。浏览器开发模式支持相同新接口。
4. **PDF 辅助页判断收窄**：仅在明确题目上下文中，取消编号 `Choose the correct answer.` 这一指令触发“答案页共享”的原因。页面其他短编号答案仍共享；不整页排除题页，不扩大到泛化指令词表，保留扫描页、未知边界回退、分段边界和原页引文。
5. **本地导入复用最终质量**：只有确实消费云端答案候选才构造约束稿；默认 localOnly 导入将最终 writer 已构造、校验的质量结果交给 gate。可选 direct-canonical 与 writer 失败仍走独立 gate，听力模态与缺音频检查保留。

## 已取得的验证证据

| 场景 | 修改前 | 修改后 | 证据层级 |
| --- | --- | --- | --- |
| 题库挂载 + 5 次独立处理事件 | 8 次初始 + 20 次全量读取，共 28 次 | 4 次初始 + 5 次单行，共 9 次 | 实际 React hook，模拟 IPC |
| 工作区挂载 + 5 次同稿版本事件 | 6 次全库处理列表 | 6 次单项处理读取，0 次全库列表 | 实际工作区与 canonical hook，模拟 IPC |
| 三段、10 页回归 PDF 候选附件 | 累计 20 页 | 累计 16 页 | 真实 PDF 子集、受控 HTTP 网关、图像回退 |
| 默认本地导入最终 V2 构造 | 三次（含无消费者与重复 gate 构造） | 一次 | PDF、DOCX、听力模态产品管线核心 |

PDF 引文仍还原到原卷答案页第 10 页；混合题页含无标题答案、跨节答案、Answers/Solutions 时继续共享。16 页减少量不是 Token 或费用减少 20% 的承诺。

导入计数测试完整比较最终质量结果的持久化 JSON 表示，PDF/DOCX/听力均通过，听力缺音频仍非 ready。DOCX 两个 bbox 浮点值在 JSON 读取时有既有精度差异，因此双方先按相同持久化表示比较，没有删除质量字段或采用数值容差。

前端全量 **53 个文件、544 项测试通过**，TypeScript 与生产构建通过。Rust 后端全量 **1,447 项通过、0 失败、16 忽略**。四份业务 PDF 的额外 ignored 对照单独执行。

同批业务 PDF 对照通过 `cargo test --manifest-path src-tauri/Cargo.toml --lib product_chain_same_pdf_paragraph_corpus -- --ignored --nocapture` 执行，1 项通过，144.15 秒。四份 PDF 与上一轮段落恢复后的同 SHA 输入对照如下：

| PDF | 上轮段落数 | 本轮段落数 | 逐段原始文字 |
| --- | ---: | ---: | --- |
| 120 | 8 | 8 | 逐字相同 |
| 182 | 7 | 7 | 逐字相同 |
| 19 | 25 | 25 | 逐字相同 |
| 217 | 8 | 8 | 逐字相同 |

四份 `quality.state` 均保持原有 blocked，quality.metrics、问题 code/severity 数量及 sourceCoverage 一致；这证明本轮对这些样本没有正文或既有质量结果漂移，不证明它们已达到发布标准。

第一次直接运行测试二进制漏带运行时 `CARGO_MANIFEST_DIR`，找不到 `src-tauri/lib/pdfium-macos/libpdfium.dylib`，转用 pdf_extract 回退，产生了解析差异。以 Cargo 正式重跑使用原生 PDFium 后，以上原文完全一致；没有修改旧基线来掩盖差异。对照产物为 `tmp/optimization-paragraph-corpus-cargo-2026-10-04.json`、`tmp/optimization-paragraph-corpus-cargo-comparison-2026-10-04.json`，日志同名前缀 `.log`。这仍是产品管线核心验证，不是原生 UI 点击测试。

日志：`tmp/optimization-implementation-frontend-2026-10-04.log`、`tmp/optimization-implementation-build-2026-10-04.log`、`tmp/optimization-implementation-backend-final-2026-10-04.log`。

## 验证边界

原生应用自动化仍因 Sky Computer Use 服务启动失败而不可用；没有取得真实 Tauri 点击到首屏或进度横幅延迟数据。上表的请求次数与产品 command-handler 核心验证分别说明，不能当作原生 UI 端到端。

没有真实云端 Token、缓存命中、收费成本或逐题准确率对照。已有 Python SQL 计时只作为审计诊断，本次不把它当作改动后 Rust/界面提速数字；摘要投影没有取消全库扫描、初次 Part 推断或整库处理状态查询。

保存事务物理证据缓存、全面 JobDetail 懒加载、路由拆包与历史索引不在本次实施范围。不引入新安全校验、哈希检查或框架，不删除本地历史、证据或真实质量门禁。
