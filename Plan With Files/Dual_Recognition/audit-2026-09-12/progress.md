# Progress — audit-2026-09-12

## 2026-09-12 逐章节对抗审计

- 读取计划全文（28 章 + 附录 A/B/C，约 4308 行），确认章节边界与行号区间。
- 侦察仓库现状：HEAD `47a3806`（`feat(m1)`），工作树 40+ 项未提交；`Plan With Files/Dual_Recognition/` 下已存在 `audit-2026-09-07` 与 `repair-2026-09-07` 两轮历史。
- 读取历史审计报告（13 项发现 F01–F13）与修复轮记录，确认修复声明，避免重复劳动。
- 建立输出目录 `Plan With Files/Dual_Recognition/audit-2026-09-12/`。
- 主线程启动后台构建校验：`npm run check` 通过；`cargo check --manifest-path src-tauri/Cargo.toml --locked` 通过（12.49s，88 warnings）→ 历史 F01（无法编译）确认修复。
- 按章节编排并派发 13 个只读对抗审计子代理，分三批执行：
  - 批 1：A1（§0–1）、A2（§2–3）、A3（§4+§11）、A4（§5+§12）、A5（§6）
  - 批 2：A6（§7）、A7（§8+§15）、A8（§9+§10）、A9（§13+§14）
  - 批 3：A10（§16+§17）、A11（§18+§19+§24）、A12（§20–23、§25+附录）、A13（§26–28）
- 全部 13 份子报告产出（合计 3352 行）。
- **发现基线漂移**：批 1/2 报告 `recognition/` 不存在，批 3 报告其已存在。主线程用 mtime 核实（`mod.rs` 11:44:30、`question_blocks.rs` 11:47:02、`auto_pipeline.rs` 11:43:48），确认审计窗口内有并发写入。
- 主线程独立复核 7 项关键结论：
  - R1 §10.1 溢出前提 → 逐选择器精确核对（排除子串误匹配），**确认 A8-F01 成立**
  - R2 legacy 路由可达性 → `App.tsx:26-33`、`router.ts:47,114`、`legacyRoutes` grep=0，**裁定 A10-F02 import 归属有误、A12-F10 正确**
  - R3 local/cloud 并发 → `scheduler.rs:255-361` 实测顺序执行，**确认 A2-F02/A4-F08 成立**
  - R4 云端窗口取消 → `scheduler.rs:334-359` 无取消检查，**确认 A4-F02 成立**
  - R5 `user_edited` → 只写不读，`ProposalOnly` grep=0，**确认 A7-F02 成立**
  - R6 基线漂移 → 确认 A1-F01 成立
  - R7 构建 → 确认 F01 已修复
- 产出汇总报告 `00-CONSOLIDATED-REPORT.md`、发现登记 `findings.md`、本文件与 `task_plan.md`。
- 未修改任何产品代码、配置或计划正文；未提交工作树。

## 2026-09-12 基线固化（用户确认后执行）

- 安全前置检查：对全部待入库文件做密钥扫描，命中仅 `lib.rs:2292` 的测试夹具 `"apiKey": "sk-profile-secret"`，非真实凭据；`.workbuddy/memory/*` 判定为并行会话草稿，**刻意排除在提交之外**。
- 提交前复核：`npm run check` 通过、`cargo check --locked` 通过（88 warnings，0 errors）；确认 `question_blocks.rs` 在 11:46 后仍被并行改写，但当前瞬间可编译。
- 固化提交：**`6affc571f43b175ffdb5a13d1823ba6f2d4962a3`**（`chore(baseline): freeze reproducible baseline for the 2026-09-12 audit`），含 M2 处理队列、M1 数据层与修复、M4 起步模块、结构编辑模块、三轮审计追踪目录。
- 基线记录重录：`fixtures/product-baseline.json` 由 `401ca76` 更新至 `6affc57`（修掉 A11-F12 的 `commitSha` 过期问题），`--reason` 门通过。
- 复核：`npm run verify:product-baseline:strict` → **`no drift from the recorded product surface.`**（exit 0）。
- 观察：提交后 `src-tauri/src/product_chain.rs` 又被并行写入者改写——该改动属 `6affc57` 之后的新基线，未纳入本次冻结。
- **修复 `--strict` gate 设计缺陷**（R8）：`diffSurface` 把 `commitSha` 计入漂移面，导致任何提交后 strict 必红（记录 SHA 必然落后 HEAD 一个提交）。已把 `commitSha` 降级为信息性输出，保留其余产品面比较项；修复后 `--strict` 恢复 `no drift`（exit 0）。
- 产出基线记录提交 `9cc195f`（`fixtures/product-baseline.json` + 审计追踪文件）。

## 统计

- 断言核对：437 条
- 发现：157 条（P0 14 / P1 46 / P2–P3 97）
- Phase：0 complete / 6 partial / 3 not started
- DoD：30 项中满足 0 / 部分 10 / 不满足 15 / 无法验证 5
- 第 26 章"四轮对抗审计通过"裁定：**不可信**（文档自洽通过，实现层落空）

## 2026-09-12 旧世代文档清理（用户授权后执行）

- 目标：删除已被当前计划取代的旧世代文档（Overhaul Plan + Phase 0–7 记录），但先解耦全部工具链引用。
- 引用清点：6 个 phase 校验脚本的 `requiredFiles`（phase2/phase3-docx/phase3-docx-package/phase4-grammar/phase6-runtime/phase7-listening-contract）+ `register-phase0-plan-corpus.mjs` 的 2 条证据行 + 8 个 golden metadata × 2 条证据行。
- 安全性前置核实：`review.evidence` / `review.method` **零消费者**（全仓 grep 仅命中生成器自身）；`verify-product-baseline.mjs` 的 `corpusManifest()` **不读** metadata JSON。
- 执行：先改 7 个脚本 + 8 个 metadata（`review.method` 由 `source-text-and-overhaul-plan-evidence` 改为 `source-text-evidence`），再 `git rm` 13 份文档（`Files/` 根 8 + `Files/archive/` 5），`Files/archive/` 目录随之消失。
- 校验：`verify:product-baseline:strict` → **`no drift`**（exit 0）；6 个 phase 脚本越过 `requiredFiles` 抵达各自的**既有**断言失败（源码顺序可证：循环行号 < 断言行号）；全部 metadata JSON 可解析；7 个脚本 `node --check` 通过。
- 提交：**`be68d4a`**（30 文件，-8128 行），只暂存本次清理路径。
- **重要观察**：工作树中存在**另一个并发 agent** 的未提交工作（`src-tauri/src/product_chain.rs` P4-T02、`Plan With Files/Dual_Recognition/repair-2026-09-07/progress.md`、`Plan With Files/Dual_Recognition/task_plan.md`，mtime 11:52–11:56）。已**刻意排除**在本次提交之外。该并发写入也是本审计期间 `recognition/` 目录"凭空出现"（A1-F01 基线漂移）与 `Files/` 目录瞬时消失的原因。
- 顺带修复：根 `task_plan.md` 中指向已删文档的悬空链接改为可追溯的 `git show <sha>:<path>` 形式。

## 2026-09-12 计划文档事实层校正（建议 #2 执行）

- 对象：`Plan With Files/IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md`（唯一写入文件）；以当前 `HEAD = 06005a5` 复核。
- 改动：§0 增勘误说明；§1.1 增基线漂移说明；§1.2 修正 App.tsx/router/AppShell/ExamCanvas/devFallback/styles 六行；§1.4 修正 ExamCanvasV2 引用；§1.5 缺口矩阵增"状态"列（6 条已修复/缓解、其余保持 open）；§10.1 重写溢出前提（3 个选择器不存在、其余仅服务不可达页，真实问题是死 CSS）；§16 增落地率与"计划路径 vs 实际路径"说明、修正 §16.6/16.7/16.16；§17 增"几乎零推进"说明；§26 四轮结论改为"文档自洽复核"并新增 §26.5；§27 增实现状态列；§28 增"冻结无强制手段"说明；附录 A 修正 2 条失效路径；附录 B 改为逐项真实状态表。
- 未改动：设计意图、架构承诺与待办事项；`src-tauri/` 全程只读。
- 已跳过：A13-F12（§28 "最先启动的 PR" 已过时）等不在授权清单内的项；`tauriCommands.ts` "过大" 判断（A1 断言 2-16）未列入本次修正。
- 未纳入：附录 A 第 10 行 `llm_gateway.rs`→`llm_suggestions.rs` 的指向偏差（任务书只要求修 2 条 `ExamCanvasV2` 路径，已记录待办）。
