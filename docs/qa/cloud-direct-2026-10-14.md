# cloud-direct 验收报告（2026-10-14，第 3 版）

按 `docs/plans/2026-10-14-cloud-direct-review-3.md` 的 §1 → §2 → §3 → §4 顺序完成修复。最终由主线程以默认参数运行真实 Tauri 产品在线、离线两轮，**verdict=passed**；online-direct 18/18, offline-local 6/6。未启用 no-sandbox。

## 1. 产品级主证据与历史证据的地位

- **修后主证据**：`artifacts/e2e-cdp/run-cloud-direct-2026-10-14T22-11-03-915Z/report.json`。当前主线程运行 `node scripts/e2e/tauri-cdp-cloud-direct.mjs`，`sessionBrowserArgs=""`、`diagnosticRun=false`，真实 DeepSeek、真实 SQLite、UI 导入/预览/发布。页面为 `http://tauri.localhost/#/library`。构建哈希一致断言通过。
- **审查提供的主线程默认沙箱主证据（修前）**：`artifacts/e2e-cdp/run-cloud-direct-2026-10-14T21-45-17-698Z/report.json`。在线 **16/16**，答案 **66/66**，预览和发布 **5/5**，NAS 契约 **96/96**；离线 Tea 为 `RUNTIME_PASSAGE_HOST_UNMAPPED`，Maori 为 `RUNTIME_OPTION_BANK_MISSING`，没有 readonly database。本轮以这次主线程默认沙箱运行确认修前产品事实，不再以受限会话的异常推断产品缺陷。该旧报告整体未通过，且记录过构建清单漂移，不能作为修后通过证据。
- `…T21-36-08-477Z` 等 **no-sandbox 运行降为“受限会话补充证据”**，不替代默认沙箱主证据。旧版关于 WebView2 CHECK 崩溃的结论仅适用于该受限会话。review-3 的主线程默认启动存活及本次默认运行均不支持把它定为产品问题。
- 本轮首次默认运行 `…T22-04-30-963Z/report.json`：在线全通过，离线 Tea 已修、Maori 仍失败，NAS96/96。保留失败现场；据此补齐本地恢复说明的证据入口后，重建并完整重跑本次最终两轮。

最终 exe SHA256：`7390f6e13f13af9719da1620a006780906e6dfcd81d45a00f55b06296df00aa7`。
构建日志 `artifacts/qa/review3-final-build.log`；构建清单 `artifacts/build-manifests/7390f6e13f13af9719da1620a006780906e6dfcd81d45a00f55b06296df00aa7.json`，源码、dist、exe 内容哈希一致。二进制来自 `tauri build --debug --no-bundle`，不是加载 devUrl 的 `cargo build` 产物。

## 2. 修复与提交

| 组 | 变更 | 提交 |
|---|---|---|
| §1 P0 收口 | 心跳临时错误继续重试，失去租约时记录退出；Drop guard 保证 worker 任意退出时 abort 心跳。advance 返回 Result，区分 lease lost 与数据库/Join 错误，重试后仍失败写 `PROCESSING_ADVANCE_FAILED` 终态。正式稿在事务内检查 owner/有效期/取消标记，旧 worker 不能播种或替换。仅 READY 推进成功才设条目 ready；去掉 app_root 失败直接 Completed 的分支。 | `1f596fd` |
| §2 P0 离线 | inline 段落标签扩至 A–Z；排除已表示为标题的重复正文标题。当说明明确声明范围且正文段数恰好相等时顺序派生标签，provenanceStatus=derived，只给一条 `PARAGRAPH_LABELS_DERIVED`；不匹配时不派生，保持编译阻断。 | `c448686` |
| §2 产品复测补齐 | physical shadow 可缺题目说明页，派生证据同时读取已恢复的本地 instruction 和 sectionEvidence；警告使用 schema 支持的 document 目标。新增回归经过实际预览编译器。 | `d9739ab` |
| §3 P1 警告 | cloud_direct 跳过本地锚点来源检查、物理覆盖率、来源复核和本地/云端差异选择；仍保留结构、答案、保真度和真实语义问题。发布按 code 合并同类提示并保留 items/targetIds。未将未测量的本地覆盖率伪造为完整。 | `18129f9` |
| §4 P2 请求与验收开关 | 核查 wire 请求图片/文字构成；默认沙箱，仅显式 flag/env 开启诊断。新增警告数量和构建一致断言。 | `576aad4` |

## 3. 最终在线轮（产品 UI 路径）

| PDF | 调用 | 答案位 | 答案一致率 | 生成耗时 | token | 发布提示 | 预览 / 发布 |
|---|---:|---:|---:|---:|---:|---:|---|
| 1. P1 - A Brief History of Tea 茶叶简史.pdf | 1 | 13 | 100% | 71.4s | 26,006 | 1 | 通过 / published |
| 10. P1 - Maori Fish Hooks 毛利鱼钩.pdf | 1 | 13 | 100% | 54.8s | 28,871 | 1 | 通过 / published |
| 100. P3 - What makes a musical expert_ 音乐天赋.pdf | 1 | 14 | 100% | 57.4s | 31,334 | 3 | 通过 / published |
| 107. P2 - A study of western celebrity 西方名人.pdf | 1 | 13 | 100% | 26.5s | 27,142 | 1 | 通过 / published |
| 24. P1 - The Pearls 珍珠.pdf | 1 | 13 | 100% | 27.9s | 29,831 | 1 | 通过 / published |

五份都是 cloud_direct、一次模型调用、0 编译问题、正文覆盖率1.0，题数与 Atlas 一致；答案按卷面题号偏移对齐后 **66/66**。五份都落 ready_for_review，没有卡在 cloud_recognition。

## 4. 最终离线轮（关闭所有云端配置）

| PDF | 终态 | 云调用 | 段落映射数 | 真实学生预览 |
|---|---|---:|---:|---|
| 24. P1 - The Pearls 珍珠.pdf | ready_for_review | 0 | 7 | 通过 |
| 1. P1 - A Brief History of Tea 茶叶简史.pdf | ready_for_review | 0 | 8 | 通过 |
| 10. P1 - Maori Fish Hooks 毛利鱼钩.pdf | ready_for_review | 0 | 10 | 通过 |
| 100. P3 - What makes a musical expert_ 音乐天赋.pdf | ready_for_review | 0 | 0 | 通过 |
| 107. P2 - A study of western celebrity 西方名人.pdf | ready_for_review | 0 | 7 | 通过 |

五份都来自 auto_extract，0 次云调用，5/5 产稿且实际预览编译/画布通过。Tea 包含 H 映射；Maori 十段派生 A–J，给一条段落边界核对警告。没有通过猜测不相等的段落数来放行。

## 5. 警告噪音核查

修前主线程默认运行发布快照的 quality issue 按 code 统计如下。这些旧检查在发布时被转成“待核对提示”，不代表稿件真有同等数量的可操作错误。

| PDF | PROVENANCE_MISSING | INSTRUCTION_PROVENANCE_MISSING | INSTRUCTION_SIGNATURE_EVIDENCE_MISSING | PHYSICAL_SHADOW_MISSING | SOURCE_QUESTION_COVERAGE_UNDETERMINED | TASK_PRESENTATION_CONTRACT_MISMATCH |
|---|---:|---:|---:|---:|---:|---:|
| 107. P2 A study of western celebrity 西方名人 | 29 | 3 | 3 | 1 | 1 | 0 |
| 100. P3 What makes a musical expert 音乐天赋 | 69 | 3 | 3 | 1 | 1 | 1 |
| 1. P1 A Brief History of Tea 茶叶简史 | 31 | 2 | 2 | 1 | 1 | 0 |
| 24. P1 The Pearls 珍珠 | 41 | 3 | 3 | 1 | 1 | 0 |
| 10. P1 Maori Fish Hooks 毛利鱼钩 | 41 | 2 | 2 | 1 | 1 | 0 |

修前 UI 提示数：Tea39、Maori49、Music80、Western39、Pearls51。修后四份仅1条人工确认提示；Music3条（人工确认、质量状态、单选 response 的 allowOptionReuse 与题型规则不一致），各份均为个位数，旧本地来源复核/差异选择噪音已消失。Music 仍有一个语义质量 issue，未将它伪装为质量全绿；实际 runtime 编译通过，符合“编译为唯一硬门禁”的产品要求。具体 code/target 均在最终 report 的 publishPreflight/qualityIssueCodes。

## 6. 请求体核查（§4）

详见 [请求体核查报告](cloud-direct-request-audit-2026-10-14.md)。真实请求约1.6–2.3MB，其中约1.5–2.2MB 来自1–2张扫描或稀疏文字页图片；正常正文页只发文字，不是每页都附整页图。约21k prompt token 包含完整产品契约、题型规则、三份完整范例和逐页文字层。保留扫描答案页及完整范例是当前证据/题型覆盖选择；本轮未未经质量验证就压缩它们，也未把缓存命中当作首次成本消失。

## 7. 验证层级与范围

- **产品级**：上述主线程默认沙箱、真实 Tauri exe、UI 导入→编辑→学生预览，在线发布5/5；离线预览5/5。
- **契约级**：最终 NAS 加载器契约探针 96/96 通过。published 与静态契约检查不能写成独立学生端应用已经真实加载/作答/提交/计分；本轮未新增该独立运行验证。
- **UI 以下回归**：scheduler21/21（含最终 READY SQL-trigger 注入失败落 failed）、stale worker 播种/替换防护；reading13/13；grammar26/26（含缺物理说明页但恢复说明可用的实际预览编译）；cloud_direct26/26；quality65/65；authoring_validation6/6；direct_canonical11/11。日志均在 `artifacts/qa/review3-*-tests.log`。
- **前端回归**：53文件、546测试通过；产品构建含 TypeScript 检查。本轮未重跑全量 Rust 历史基线，不把旧版全量数字写成本轮验证。
- 未扩展：独立学生端完整交互链、整份无文字层扫描 PDF、DOCX 旧云端链、听力链。

## 8. 复现

```powershell
node scripts/e2e/build-app.mjs
node scripts/e2e/tauri-cdp-cloud-direct.mjs
```

默认 `sessionBrowserArgs=""`、`diagnosticRun=false`。仅受限会话补充诊断才用：

```powershell
node scripts/e2e/tauri-cdp-cloud-direct.mjs --session-no-sandbox
# 或显式设置 PDF2TEST_SESSION_NO_SANDBOX=1；默认不设置。
```

诊断结果会标记 `diagnosticRun=true`，不替代主线程默认沙箱产品证据。
