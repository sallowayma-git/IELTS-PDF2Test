# Progress

- Started main-agent investigation and two independent read-only scouts for persistence and gateway/test integration.
- Read cloud-direct architecture in full and exact touched code. Implemented batch envelope, cloud boundary fallback after truncation, fan-out persistence, scoped rerecognition and cloud import UI path. Cargo check passed; frontend related tests 39 passed and TypeScript passed. Rust cloud_direct regression running.
- Full frontend: 57 files / 594 tests passed. Gateway: 68 passed, 1 ignored. Cloud-direct initial verified run: 35 passed, 2 ignored. Processing module: 63 passed. New regression coverage includes repeated q1 across tests, cloud page manifest after truncation, idempotent writes, edited sibling preservation, invalid page evidence and scoped rerecognition.
- Hardened batch atomic completion/cancellation and durable scope after independent review; rerunning Rust tests and building current embedded Tauri app. Added actual UI test scripts/e2e/tauri-cdp-cloud-multi-document.mjs for 3 source PDFs -> 5 items, independent editor/preview, save/reopen and scoped retry.
- Hardened Rust cloud-direct rerun: 35 passed / 2 real-provider tests ignored. Actual Tauri run artifacts/e2e-cdp/run-cloud-multi-2026-11-06T10-28-18-920Z/report.json passed: 3 PDFs -> 5 rows, all 5 open in editor/student preview, child save/reopen, child rerecognition limited to source pages 3/4 without duplicate rows. All processing records show local_status=not_run/cloud_status=succeeded.
- Visual QA found the import notice still called initial source jobs final "questions"; changed it to recognition tasks with automatic article-save wording. Latest related frontend 48 passed plus tsc passed. Building final UI and extending product E2E with explicit cloud failure/retry preservation and 5 independent UI exports.
- Final embedded build produced with stable hashes. Offline actual UI regression: 5/5 passed. Extended online E2E exposed test harness races (title clicked before header settled; Node SQLite read connection had zero busy timeout during the legitimate write transaction); added stable real-click waiting and a 5-second SQLite read busy timeout. These were harness-only fixes; rerunning complete online workflow.
- Final online product run 10:33:59 passed all six assertions, including controlled cloud failure/retry and real UI publication of 5 independently loadable packages (5 exam JS files plus manifest). Final source/embedded build hashes stable; git diff --check clean. No real-provider call or installer build was performed.
- Final processing/scheduler module rerun against the final source: 63 passed / 0 failed. Work complete; all earlier local split changes preserved and no changes committed automatically.

## 2026-11-06 护栏放宽 + 漏篇追问 + 真实模型
- 入库放宽：越界页码丢弃并警告；单篇归一化失败/逐篇截断只跳过该篇；调度器只在"无题组且无正文"时判失败；局部重识别多返回只取第一篇。
- 漏篇追问：≥1500 字符的文字页未被任何篇 sourcePages/answerPages 认领 → 同前缀追问一次，只带已返回篇章清单；仍未覆盖则警告 cloud_direct_pages_uncovered。
- 真实 DeepSeek（deepseek-flash）桌面验证：3 篇合并 PDF → 3 条，页码 1-5/6-10/11-16 全对；首调 30.5k 入 / 16.4k 出 / 55s；顺序重识别 18.4k 入其中 15.6k 命中缓存。
- 报告 artifacts/e2e-cdp/run-cloud-real-2026-11-06T11-54-50-430Z/report.json；受控多篇与离线拆分 E2E 复跑通过。

## 2026-11-06 会话式修复 + 暖缓存 + 用户真实卷
- 修复回合改为同一会话：上一轮输出作为 assistant 轮，问题清单作为新 user 轮，前缀不变可命中缓存；多篇时只带回并只重出编译失败的篇。
- 暖缓存闸门：键=模型配置+题型，冷时首份独占发出，30 分钟有效；受控 E2E 断言首请求独占、之后并发 2。
- 无题目文章可入库：prompt 禁止生成占位题；归一化允许"有正文无题组"；批级警告点名标题时只挂到该篇。
- 用户卷（38. P2 Egypt merge，16 页）真实 DeepSeek：3 篇 1-5/6-10/11-16 正确；P3 原 PDF 无 27-40 题，现 0 题 + 单独警告。报告 run-cloud-real-2026-11-06T12-20-58-957Z。
- 待查：首轮快照中主条目状态短暂为 action_required，重识别后为 ready（与质量报告一致）。
