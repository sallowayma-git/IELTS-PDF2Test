# cloud-direct 请求体核查（review-3 §4）

证据：主线程默认沙箱运行 `artifacts/e2e-cdp/run-cloud-direct-2026-10-14T21-45-17-698Z/report.json`，读取在线各 item 的 `calls[0]`。字节数来自真实 wire 统计，不使用脱敏后的 `inputBytes`。

| PDF | 请求 bytes | 图片段 bytes | 图片数 | promptText bytes | prompt tokens |
|---|---:|---:|---:|---:|---:|
| 1. P1 - A Brief History of Tea 茶叶简史.pdf | 2,300,880 | 2,211,084 | 2 | 82,010 | 22,073 |
| 10. P1 - Maori Fish Hooks 毛利鱼钩.pdf | 1,851,714 | 1,760,786 | 1 | 83,305 | 21,195 |
| 100. P3 - What makes a musical expert_ 音乐天赋.pdf | 1,583,739 | 1,491,422 | 1 | 84,588 | 21,508 |
| 107. P2 - A study of western celebrity 西方名人.pdf | 1,771,341 | 1,680,814 | 1 | 82,807 | 21,112 |
| 24. P1 - The Pearls 珍珠.pdf | 2,307,700 | 2,216,536 | 2 | 83,435 | 22,331 |

`cloud_direct::select_image_pages` 只为无文字层，或文字层少于 200 字符且有图像的页面附图；正文文字页只发文字。`llm_gateway::generate_exam_direct` 将所选扫描/稀疏页作为 data URL 加入消息。1.5–2.2 MB 图片段占请求绝大多数，并非每张正文页都重复附图。保留这些页是为了读取扫描答案页及稀疏文字页里的实际内容。

`llm_gateway::exam_direct_prompt` 包含产品 system 契约、题型规则、三份完整范例（heading/features/summary；TFNG/单选；table/flowchart），另附逐页文字层。这解释约 82–85 KB promptText 与约 21k prompt token。范例采用 pretty JSON。当前实现有意用完整范例覆盖不同宿主/交互类型；没有以“有缓存”为理由跳过首次请求成本。约 11k token 的 Python 验证没有相同的完整产品提示材料；这里只能确认产品构成，不能据此宣称两者是等价请求。

本组保留图片和完整范例，不在未经同质量验证时压缩证据。后续可独立验证图片缩放/压缩和精简范例；本轮优先保证答案页与结构正确。

验收脚本的默认参数现为空；仅 `--session-no-sandbox` 或 `PDF2TEST_SESSION_NO_SANDBOX=1` 开启受限会话诊断，并在报告里标注 `diagnosticRun=true`。默认报告为 `diagnosticRun=false`。同时记录发布警告分布，并增加个位数警告与构建哈希一致断言。
