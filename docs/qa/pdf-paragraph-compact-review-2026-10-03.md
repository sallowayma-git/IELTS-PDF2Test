# PDF 段落恢复与简短云端校核（2026-10-03）

## 实施结果

- PDF 无显式段落标签时，按真实物理行的间距、缩进、字号、列与标题边界恢复段落。借用版面坐标，保留原始提取文本；不采用物理行中可能出现的逐字空格。已有 A/B 等标签的文章保留原有路径，DOCX 不启用 PDF 几何推断。
- 默认 packet 校核支持 `submit_batch_decisions`，每项使用 `decisionId` 和 `Cloud / Local / Wrong / Unknown`。Cloud/Local 复用已有版本，Wrong 仅提交修正；证据可用已提供行的 `evidenceLineIds`，由本地展开并验证。普通独立单槽题允许同组逐题混选，共享选项库、选二及结构差异继续按依赖保护。
- 已采纳 Cloud 的模式中，Cloud 表示保留当前稿，避免覆盖前轮修复；Unknown 最多补证一次，仍不足则保持未核实。候选采纳与 Wrong 修正分别写入事务，后者失败不会撤销前者，也不会把整批标为成功。
- 上传副本去掉哈希、锚点、审计和质量信息；差异中与两个样本相同的内容改成 JSON 引用。完整本地信息仍保存，用于版本检查和撤销。
- 完整上下文、响应、执行结果保存在 `jobs/{jobId}/recognition/{batchId}.repair-history.jsonl`。恢复时只复用与当前内容匹配的反馈及证据，不重放旧编辑；旧记录仍保留。上传最近 12 条反馈和连续性摘要，重复证据使用引用，诊断长文本有长度限制。中断的最后一行另存后可继续追加。未增加历史查看 UI。

## 同批 PDF 对照

基线来自修改前的 Git HEAD b7ec555；两轮都使用真实产品导入与作者稿读取的 command-handler 核心 `run_auto_pipeline_core` / `get_authoring_v2_core`，每份文件使用全新临时应用目录。属于 UI 下层验证，并非 Tauri UI 端到端。

| PDF | 修改前段落数 | 修改后段落数 | 忽略空白的正文对照 |
| --- | ---: | ---: | --- |
| 节食与长寿（120） | 8 | 8 | 完全一致 |
| 南极考察（182） | 7 | 7 | 完全一致 |
| 植物交流（19） | 26 | 25 | 完全一致 |
| 孩子的机器人朋友（217） | 8 | 8 | 完全一致 |

检查了四份 PDF 第一页的实际版面；植物文章中图片旁与图片下连续正文已恢复为同段。此批本地导入的质量状态仍为 blocked，存在未解决答案等既有问题，段落通过不代表可发布。

报告：`tmp/paragraph-corpus-before.json`、`tmp/paragraph-corpus-after.json`，含文件 SHA、段落正文与锚点。

复跑当前产品核心对照：

```sh
PDF2TEST_PARAGRAPH_CORPUS_REPORT="$PWD/tmp/paragraph-corpus-after.json" cargo test --manifest-path src-tauri/Cargo.toml --lib product_chain_same_pdf_paragraph_corpus -- --ignored --nocapture
```

## 请求体与验证边界

- 固定候选提示词：13,010 → 11,615 字符（约减少 10.7%）；legacy 校核：12,376 → 11,946 字符。
- 最小单题型 compact HTTP 文本为 4,108 字符。该值来自受控夹具，不含真实 PDF 的完整动态内容，不能换算为这四份 PDF 的真实 Token 收益。
- 受控 HTTP 校核测试涵盖网关、默认 packet 主循环及 SQLite 落库，含混选、Wrong、Unknown、恢复和引用证据。真实模型的准确率、费用和缓存命中尚未测量：本机应用云端 profile 为空，live provider 配置未设置。
- 用户观察到的 8,468 输入不能仅按 PDF 大小解释；需分开查看提示词、文本/图像、两个样本和历史。若缓存输入 4,352 包含在总输入内，则未缓存部分为 4,116；缓存并不删除输入。现有总 Token 预算计入缓存输入，累计多轮仍可能到达上限，本次没有通过提高上限掩盖问题。
- Rust 全量库测试：1,442 通过、0 失败、16 忽略；四份 PDF 的额外 ignored 对照单独运行通过。前端构建通过。原生 UI 自动化服务启动失败，尚未验证实际 Tauri 点击流程；目前证据限于产品 command-handler 核心和受控 HTTP 服务层。
