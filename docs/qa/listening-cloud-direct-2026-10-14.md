# 听力云端直出与共享请求体瘦身 QA（2026-10-14）

实现范围：方案 §4、§5；不修改 docs/plans，不推送远端。产品听力契约唯一来源为 src-tauri/resources/cloud-direct/listening/，以方案 prompt 副本为起点，补充了答案附件说明。音频受管、探测、绑定与分配模块未修改，直出写稿调用既有 apply_bindings_to_authoring / sync_item_audio_media。

## 证据等级

- 阅读：主线程实际 Tauri 内嵌构建、WebView2 CDP、默认产品沙箱；五份在线导入、编辑器、学生预览、NAS 发布及加载器契约探针。最终指标见下文。
- 听力：真实 DeepSeek 命令层测试，不是产品端到端验收。唯一真实样本 listening-vol7-t9.pdf 没有音频或答案。
- 合成媒体编译探针：只在测试内存副本中使用既有合成媒体，未写入产品稿，不作为音频产品证据。

## 听力真实服务结果

运行 cargo test --manifest-path src-tauri/Cargo.toml --lib real_deepseek_listening_direct_vol7_t9 -- --ignored --nocapture，通过。

| 指标 | 结果 |
|---|---|
| 真实调用 | 1 次 |
| 请求体 | 26,628B，0 张图片，≤60,000B |
| 总耗时 | 46,307ms |
| Part / 槽位 / 题组 | 4 / 40 / 8，题型及题号范围逐组匹配 golden |
| 未解答案 | 40 / 40，未猜答案 |
| 结构编译问题 | 0 |
| 真实稿完整编译 | 4 个 LISTENING_MEDIA_MISSING，明确保留 |
| 测试内合成媒体副本编译 | 0 问题；不是实际音频证据 |

原始证据：artifacts/qa/listening-direct-real.json、artifacts/qa/listening-live-verified.log。真实卷的题稿及派生产物留在忽略目录，不提交题目内容。

首次真实调用暴露 list_item 被模型用作非法答案宿主；修复为保留原文、补段落容器并绑定合法宿主后重跑通过。未将缺音频交给模型修复；预览与发布继续执行未过滤的正式编译器。听力 unresolved、缺键及空答案均为发布硬门禁；缺少地图资源也阻止发布。

## 自动回归

- 共享瘦身 direct_：23 通过、1 个真实服务测试未在该轮运行。
- 最终直出模块：31 通过。
- 最终发布校验模块：7 通过。
- 最终 listening_：97 通过、1 个真实服务测试由上面的独立命令运行。
- 音频导入分配规则：16 通过；TypeScript 检查通过。
- 分块回归覆盖每块共享答案文字和图片；列表笔记回归覆盖合法宿主且文字保留。独立核验发现并修复了错误使用嵌入小图作为整页地图的回退。

## 阅读产品回归

主线程运行 `node scripts/e2e/tauri-cdp-cloud-direct.mjs --online-only`，未传 no-sandbox 开关、相关环境开关关闭。最终 **verdict=passed**，sessionBrowserArgs 为空、diagnosticRun=false、构建新鲜度检查通过。此默认沙箱运行为本次产品级主证据；未重跑离线轮（本次验收要求为五份在线）。

| 阅读卷 | 答案 | 每次请求体 B | 图片数 | 作业耗时 ms |
|---|---:|---:|---:|---:|
| Tea | 13/13 | 222986 | 2 | 57035 |
| Maori | 13/13 | 180759 | 1 | 40685 |
| musical expert | 14/14 | 167876 | 1 | 43090 |
| western celebrity | 13/13 | 180382 | 1 | 19228 |
| Pearls | 13/13 | 224862 | 2 | 20935 |

合计 **66/66**，五份均一次调用、编译问题 0、正文覆盖率 ≥95%、预览无阻断且画布渲染，五份 NAS 发布成功；NAS 学生加载器**契约**探针 96/96，不声称真实 student-exam 浏览器作答提交通过。所有请求均 ≤350,000B，无需提高答案页至 1800px。western celebrity 请求从方案测得的 1,771,341B 降至 180,382B（约减少 89.8%）。

主证据：`artifacts/e2e-cdp/run-cloud-direct-2026-10-14T22-55-23-591Z/report.json`；控制台日志 artifacts/qa/listening-reading-online.log。

实际产品构建 SHA256：`45ce94956e52a267a93ed7bc28a62a1c32e78fe8be15316b44e71484e1ff1cf5`；构建清单 artifacts/build-manifests/45ce94956e52a267a93ed7bc28a62a1c32e78fe8be15316b44e71484e1ff1cf5.json；frontendInputs=6e7d8390355c、backendInputs=7e6691877852，构建期间无漂移。只做验收用实际 Tauri 内嵌构建，本次没有重新打安装包。


## 尚未覆盖

听力产品级验收仍待用户提供至少三套题目 PDF、每套四段真实音频、答案键：导入 → 音频绑定 → 学生预览 → NAS 发布 → 真实加载器 → student-exam 作答提交。现有唯一听力样本缺少音频与答案，不能声称真实媒体、答案准确率或听力产品端到端通过；也没有真实地图卷和答案附件样本。方案 §3 的 NAS 学生听力页面修复属于另一仓库任务，本次没有执行。
