# 逐章节对抗审计汇总报告（audit-2026-09-12）

> 审计对象计划：`../IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md`（28 章 + 附录 A/B/C）
> 审计方式：13 个只读子代理，按章节分组逐条核对计划断言与代码现状（对抗立场）
> 审计日期：2026-09-12
> 子报告目录：`./A1-*.md` ~ `./A13-*.md`
> 主线程独立校验：`npm run check`、`cargo check --locked`、`git` 只读核对、关键符号全局 grep

---

## 0. 最高优先级的审计前提告警：基线未冻结

**本次审计的基线在工作树中被并发修改，结论存在时序不一致。这是本轮审计最重要的元发现，先于所有功能发现。**

证据：

| 事实 | 证据 |
|---|---|
| 审计开始时 HEAD = `47a3806`，工作树含 40+ 项未提交改动 | `git status --porcelain`（审计中变为 43 项） |
| 审计期间出现**新的未跟踪模块** `src-tauri/src/recognition/local/{mod.rs,question_blocks.rs}`（1326 行） | `git status --porcelain` → `?? src-tauri/src/recognition/` |
| 其 mtime 落在审计窗口内 | `mod.rs` = 2026-09-12 11:44:30；`question_blocks.rs` = 2026-09-12 11:47:02 |
| 同一时刻 `auto_pipeline.rs` 也被改写以引用该模块 | `auto_pipeline.rs:36` import、`:280-312` `materialize_question_layout_graph` |

**后果**：先运行的第 1–2 批子代理（A4、A5）报告 `recognition/` 目录不存在、`QuestionLayoutGraphV1` grep 为 0；后运行的第 3 批（A11、A12）报告该模块已存在。两者都是对**各自时刻**工作树的正确观察，但拼在一起会自相矛盾。

**主线程裁定**（以 2026-09-12 11:47 快照为准）：

| 子代理结论 | 裁定 |
|---|---|
| A4-F07 / A5-F01「`recognition/` 目录不存在」 | **已过时**（降级为 P1）：目录与 `QuestionLayoutGraphV1`/`SemanticRegionRole`/`QuestionBlockCandidateV1` 类型**已存在** |
| A5-F01 的实质主张「M4 本地识别主链未交付」 | **仍成立**：`recognize_local` grep = 0；`assemble_question_stem`/`detect_question_number_tokens`/`OptionLabelToken`/`assemble_option`/`compile_table_stimulus`/`validate_basic_task` 全部 grep = 0；新图仅作为**附加 artifact** 写出（`auto_pipeline.rs:289-312` 注释自述 "deliberately non-fatal … additive evidence"），主链仍走 `make_dynamic_split_candidates → build_authoring_v2_shadow` |

**建议**：任何后续审计或施工前，先 `git add -A && git commit` 固化基线并记录 SHA。当前状态下，"计划 vs 代码"的逐条比对**不可复现**——这正是 A1-F01、A12-F11 指出的问题在工作树层面的重演。

### 0.1 处置结果（2026-09-12 已执行）

基线已按用户确认固化：

| 项 | 结果 |
|---|---|
| 固化提交 | **`6affc571f43b175ffdb5a13d1823ba6f2d4962a3`** |
| 提交信息 | `chore(baseline): freeze reproducible baseline for the 2026-09-12 audit` |
| 提交前校验 | `npm run check` 通过；`cargo check --locked` 通过（88 warnings，0 errors） |
| 密钥扫描 | 仅命中测试夹具 `lib.rs:2292` `"apiKey": "sk-profile-secret"`，非真实凭据 |
| 基线记录重录 | `fixtures/product-baseline.json`：`401ca76` → `6affc57`（修掉 A11-F12 的 `commitSha` 过期）；`--strict` 校验 **`no drift`** |
| 刻意排除 | `.workbuddy/memory/*`（并行会话草稿）未入库 |

**仍存在的动态性**：提交后 `src-tauri/src/product_chain.rs` 又被并行写入者改写。该改动属于 `6affc57` **之后**的新基线，本报告全部结论以 `6affc57` 为准。

---

## 1. 主线程独立校验结果

| 校验 | 结果 | 覆盖层级 |
|---|---|---|
| `npm run check`（tsc --noEmit） | **通过** | 静态类型 |
| `cargo check --manifest-path src-tauri/Cargo.toml --locked` | **通过**（12.49s，88 warnings） | 静态编译 |
| `git status` | 43 项未提交 / 未跟踪 | — |

**历史结论更新**：`audit-2026-09-07` 的 F01「当前 Tauri 应用无法编译（6 个错误）」**已修复**，修复轮声明属实。

**未执行**（主线程明确不做，避免以低成本证据冒充产品证据）：`cargo test`、`npm run build`、任何 e2e 脚本、任何真实 LLM 调用、任何真实 NAS 写入。

---

## 2. 总体判定统计

13 个子代理合计核对 **437 条可验证断言**，产出 **157 条发现**。

| 子报告 | 覆盖章节 | 断言数 | HOLDS | PARTIAL | FALSE | UNVERIFIABLE | 发现数 |
|---|---|---:|---:|---:|---:|---:|---:|
| A1 | §0–1 文档目的 / 当前状态审计 | 88 | 67 | 13 | 8 | 0 | 13 |
| A2 | §2–3 验收标准 / 目标 IA | 53 | 22 | 17 | 13 | 1 | 14 |
| A3 | §4 + §11 Canonical DS / 题库数据面 | 33 | 13 | 8 | 12 | 0 | 11 |
| A4 | §5 + §12 后端调度 / 批量导入 | ~60 | 14 | 19 | 26 | 1 | 14 |
| A5 | §6 本地几何识别 | 70 | 8 | 27 | 35 | 0 | 10 |
| A6 | §7 云端完整识别 | 17 | 1 | 3 | 13 | 0 | 12 |
| A7 | §8 + §15 对齐合并 / 错误处理 | 28 | 3 | 6 | 18 | 1 | 11 |
| A8 | §9 + §10 WYSIWYG / CSS 溢出 | 52 | 17 | 16 | 19 | 0 | 12 |
| A9 | §13 + §14 发布 / 设置 | 36 | 11 | 14 | 11 | 0 | 13 |
| A10 | §16 + §17 前后端逐文件清单 | 文件级 | 1 达成 | 18 部分 | — | 2 未动 / 1 不符 | 11 |
| A11 | §18 + §19 + §24 计划 / 测试 / DoD | — | — | — | — | — | 13 |
| A12 | §20–23、§25 + 附录 | — | — | — | — | — | 11 |
| A13 | §26–28 自审记录 / 矩阵 / 建议 | 29 修订 | 26 已落 | 2 部分落 | 1 落空 | — | 12 |

### 2.1 阶段与 DoD 判定（A11）

- **Phase 完成度**：`complete` **0**；`partial` **6**（P0/P1/P2/P3/P6/P7）；`not started` **3**（P4 仅 T01/T02 增量、P5、P8）。
- **DoD 30 项**：**满足 0 / 部分 10 / 不满足 15 / 无法验证 5**。按仓库 `AGENTS.md` 证据分层，**无任何一项达到 `product`（端到端产品行为）级满足**。
- 计划自称 Phase 0–8 合计 71–112 工程日；实际计划相关提交仅 3 个（`5daa04f`、`401ca76`、`47a3806`），其余为实现中的未提交工作树改动。

### 2.2 第 26 章"四轮对抗审计通过"的裁定（A13）

**不可信**。29 条"修订"中 26 条确实写进了正文对应章节，但：

- 第 26 章开头声称"重新对照用户需求和当前代码"，**全章无一条 `file:line` 证据**；
- 26.3（云端）的 5 条修订指向 §7.9/§7.11/§7.12/§7.13/§17.9，**代码零实现**；
- 26.4 A 的 cross-repo NAS 截图 parity 修订**落空**——未写进任何被指向章节（§9/§13/§19.6/§21 PR-07/PR-14）；
- 攻击多为自我确认式重述，缺少可失败判据；26.3 D 的攻击前提"当前 `Mutex<FnMut>`"在代码中字面不成立（实为 `&mut FnMut` 回调），属稻草人。

准确表述应为：**"四轮文档自洽复核通过"，而非"四轮对抗审计通过"**。

---

## 3. P0 发现汇总（跨章节去重后 14 条）

| 编号 | 章节 | 结论 | 证据 |
|---|---|---|---|
| A1-F01 | §1 | 冻结基线 `bb978be` 与实际 HEAD/工作树不符，全章"当前实现"描述系统性过时 | `git log`、43 项未提交 |
| A2-F01 | §2.2 | "新任务直接以 DocumentIRV2 为输入"不成立，主链仍 V1-first | `auto_pipeline.rs:1692` split → `:2067` `build_authoring_v2_shadow` |
| A2-F02 | §2.1/§3.2 | 单题 local 与 cloud **顺序执行**，非并发 | `scheduler.rs:255-361` |
| A2-F03 | §2.3 | 云端完整候选 / repair / salvage / `ReconciliationProposalV1` 全部缺失 | `CloudRecognitionCandidateV1` grep = 0 |
| A4-F01 | §12.2 | 导入队列失败留下**不可见孤儿文件**（DB 无行、磁盘有 job.json+uploads） | `commands.rs:112-127`、`repository.rs:168-176` |
| A5-F02 | §6.8 | 硬闭包只在前端；后端无 `validate_basic_task`，且 issue code **三方命名冲突** | `actionableIssues.ts:105` vs `issue_codes.rs` |
| A6-F01 | §7.3 | `CloudRecognitionCandidateV1` 零实现，云端仍只产 `CloudReadingOutlineV1` | `llm_suggestions.rs:235` |
| A6-F02 | §7.6 | 无 repair/salvage/状态机，单点校验失败即**整份丢弃** | `llm_gateway.rs:1176-1365` |
| A7-F01 | §8 | 合并引擎整章零实现（`align_task_groups`/`can_auto_fill` grep = 0） | 全仓 grep |
| A7-F02 | §8.7 | `user_edited` 保护**只有写入没有读取**，`ProposalOnly` 不存在——M5 落地后必然覆盖用户编辑 | `repository.rs:205-223,306`；`llm_suggestions.rs:664-737` |
| A11-F01 | §24 | 唯一真实 Tauri E2E 最新报告 **verdict=failed**，且早于当前 HEAD | `artifacts/e2e-tauri/run-2026-09-05T11-34-16-269Z/report.json` |
| A11-F02 | §18 P0-T02 | 要求的 3 个 `scripts/e2e/tauri-*.mjs` 只有 1 个存在 | `scripts/e2e/` |
| A11-F03 | §18 P0-T01 | "8 份 Reading PDF fixture"源文件在工作树缺失，`privateCorpusReady=false` | `fixtures/private-real/` |
| A13-F01 | §26.3 | 5 条"已修订"的云端设计在代码中零实现——四轮审计只审文档、未审实现 | 见 A6/A7 |

> 注：A5-F01 原判 P0「第 6 章具名类型完全不存在」，因基线漂移已按 §0 降级为 P1（实质主张仍成立）。

---

## 4. P1 发现汇总（按主题归并，共 46 条）

### 4.1 数据面与多事实源
- **A3-F01** §11.5 清理机制三个函数、四个触发点**全部未实现**；仅导出成功路径有清理（`cleanup.rs:99,161`）。
- **A3-F02** §11.4 目录布局与 §11.6 内容寻址去重未实现；深层目录树 `preview/legacy/export-history/patches/revisions` 仍在（`util.rs:223-244`）。
- **A3-F03** 双事实源未终止：新导入仍 `save_job` → 写 `job.json` + 双写 legacy DB（`job_store.rs:33-44`、`processing/commands.rs:98`）。
- **A3-F04** §4.5"保留原文件"开关是**空操作**：仅存 localStorage，Rust 零读取。
- **A1-F08** §1.5 LIB-001（多事实源）经 M1 后仍成立，与 M1"complete"口径存在张力。

### 4.2 后端调度与导入
- **A4-F02** 云端执行窗口内的取消被吞掉，无迟到结果守卫（`scheduler.rs:334-360`）。
- **A4-F03** 启动恢复 `action_required` 分支向用户**谎称已重试**（`queue.rs:244-252`、`scheduler.rs:88-93`）。
- **A4-F04 / A9-F01** 设置页的并发/云端/文件保留开关与调度器**完全脱节**，无 `get/save_app_settings` 命令。
- **A4-F05** 过期 lease 认领覆盖 `local_status`，破坏"本地已完成"事实。
- **A4-F07/F08** §5.1 模块布局（`recognition/`、`reconcile/`、`editor/`、`publish/`）与 §5.2 类型模型基本是**纸面设计**；`StageStatus`/`ProcessingItemV1`/`ProcessingStage` 全仓零命中。
- **A4-F09** §5.3 七条目标命令只落地 5 条且签名不符。

### 4.3 本地识别
- **A5-F01**（降级后）主链未交付：`recognize_local` 零命中，新图未接入主链。
- **A5-F03 / A11-F04** §18 Phase 4 五项验收指标**零度量实现**；`fixtures/golden/metrics.json` 无消费者。
- **A5-F04** §6.5–§6.7 的 token/geometry-first 算法未实现，现为行序启发式（`anchors.rs` 固定 0.62/0.72/0.9）。
- **A5-F05** §6.10 表格未使用 physical tables（按 `|` 拆行文本，`rowSpan/colSpan` 恒 1）；hybrid 热点无 crop fallback。
- **A5-F06** §6.9 `paragraphMap` 恒为空，passage A-G 与 List of Headings 未显式分离。

### 4.4 云端与合并
- **A6-F03** `recognition/skills/ielts-reading-v1/` 及 8 类示例**完全不存在**。
- **A6-F05** Evidence Resolver 与 `IdAllocator` 缺失，canonical 入口无护栏。
- **A6-F06** §26.3-D 要求的 `LlmTransport: Send + Sync` 未落地，gateway 仍串行。
- **A6-F04** 第二条校对链缺失，且未评估 100 份批量成本。
- **A6-F07** 大文件阈值与页覆盖校验缺失（存在"截断当完整"风险）。
- **A7-F03** issue 命名**四方不一致**；后端 `actionable_issues_v1` 是只建不读写的**死表**，`get_workspace_item` 返回的 issues 被前端丢弃。
- **A7-F04** §15.1/§15.2 类型零实现；F-M0-3 只修一半，`EDIT_VERSION_CONFLICT:current=2:base=1`、`ITEM_DS_NOT_SEEDED:<id>` 等原始机器码仍在多条路径直送 UI。
- **A7-F05** 内部细节"记录到日志"不成立：`fail_job` 丢弃完整 error 且无日志写入（`scheduler.rs:441-455`）。
- **A7-F06** §15.3 第 2 行与实现直接矛盾：本地失败即 `fail_job` 返回、云端从不运行；`reconcile_status` 硬编码 `"succeeded"`。

### 4.5 前端与 UI
- **A8-F01** §10.1 以**虚假前提**描述"当前溢出原因"（`.editor-grid`/`.llm-grid`/`.settings-grid` 选择器根本不存在），与项目自身 Phase 0 实测"72 项 0 溢出、死 CSS 才是真问题"直接矛盾。
- **A8-F02** §9.4 `EditorCommandV1` 只有 3/9 op；`expectedText` **不做服务端校验**，并发只靠 baseVersion。
- **A8-F03** §9.2 声称删除的 4 个产物（`authoringTiptap.tsx`、`UnifiedPreview.tsx`、`LibraryExamDetail.tsx`、`ExamCanvasV2` 别名）**全部仍存在并仍被引用**；V1 HTML 仍 `dangerouslySetInnerHTML` 直接渲染。
- **A10-F01** 文档路径与仓库实际路径**系统性不一致**（`ExamWorkspacePage` 实际在 `src/features/editor/`；`ExamCanvasV2` 实际在 `src/exam-canvas/ExamCanvas.tsx`）——按计划施工会找错文件。
- **A10-F02** 第 16 章整体落地率极低，退休页面整套仍在并被**孤儿** `legacyRoutes.tsx` 统一 import。
- **A10-F03** 第 17 章核心零推进：`lib.rs` 仍 11151 行 / 约 120 命令；`authoring_pipeline.rs` 仍 13848 行且被 V1 链路调用。
- **A10-F04** 孤儿文件（`pages/LibraryPage.tsx`、`pages/Settings.tsx`、`legacyRoutes.tsx`）无 import 但仍被 `tsconfig include:["src"]` 编译。

### 4.6 发布与设置
- **A9-F02** 批量发布**无崩溃恢复**（不写 journal，`recover_incomplete_transactions` 只认 `*.journal.json`）；`releases/{batch_id}` 永不回收且资产重复拷贝。
- **A9-F03** 批量失败不返回每题问题，也无"仅发布通过项"二次动作。
- **A9-F04** §13.4 typed `PublishCheckResultV1` 未交付，前端仍解析 `publish_check_failed:` / `authoring_v2_export_blocked:` 字符串。

### 4.7 计划与流程
- **A11-F05/F06** §19.3 云端 17 例覆盖 0、§19.4 合并 9 例覆盖 0；无任何前端测试框架（§19.5 0/14、§19.6 无）。
- **A11-F07** 以 CLI/schema/browser-dev 证据冒充产品验收（over-claim 模式），与修复文档自身"真实 Tauri 不计为验收通过"矛盾。
- **A11-F08** Phase 8 删除项几乎全未执行，V1 主链仍在写。
- **A12-F01** PR-01 声称"只增测试和基线，不改行为"，但基线冻结提交 `5daa04f` 实际改了两处产品判定行为。
- **A12-F03** 附录 B 交付物：**7/7 contracts 缺失**、无 `recognition/` 技能包、多个后端模块目录不存在。
- **A12-F04** 第 23 章监控指标与发布门**零实现**：10 项产品指标无采集、7 项质量目标无度量脚本、6 步灰度无开关；`src/config/featureFlags.ts` 无任何调用者。
- **A12-F05** §23.3"不允许同一新题由 V1/V2 双写"**无代码强制**，且当前新题确实同时经过 V1 与 V2 写入。
- **A13-F02** 26.4 A 的 NAS renderer 截图 parity 修订落空。
- **A13-F04** §27 追踪矩阵无完成状态列，把未实现的 §7/§8 与已实现章节并列；行 4189 称"并发"，实现却是顺序执行。
- **A13-F05** §28 冻结建议无强制手段（无 CODEOWNERS、CI 不识别），6 个"冻结"文件中 4 个在计划后被改。

---

## 5. 各章节一句话结论

| 章 | 主题 | 一句话结论 |
|---|---|---|
| 0 | 文档目的与原则 | 工程原则成立，但"当前问题"清单已部分过时 |
| 1 | 当前远端状态审计 | **后端清单准确（26/27），前端清单 5 条 FALSE**；缺口矩阵 6 条已修复仍按当前缺口呈现 |
| 2 | 验收标准 | 产品表面类基本成立；**识别/云端/并发类核心验收 13 条 FALSE** |
| 3 | 目标 IA 与流程 | 三路由与工作区成立；**"单题并发""一次事务建 50 条"不成立** |
| 4 | Canonical DS | 建模与事务编辑成立；**§4.1 别名不存在、§4.4/§4.5 数量级偏差** |
| 5 | 后端调度架构 | **以纸面设计为主**；类型模型零实现、local/cloud 实为顺序 |
| 6 | 本地几何识别 | **§6.1 断点全成立，§6.2–§6.11 设计基本未实现**（35 条 FALSE） |
| 7 | 云端完整识别 | **基本未动**（13/17 FALSE），仍为只读对照提纲 |
| 8 | 对齐合并 | **整章零实现**，且 `user_edited` 是假护栏 |
| 9 | WYSIWYG | 骨架成立（contentEditable 已清除、IME 安全 textarea 已落），**协议与拆分未完成** |
| 10 | CSS 溢出 | **§10.1 前提被项目自身实测推翻**；§10.4/§10.7 自相矛盾 |
| 11 | 题库/DB/过程文件 | M1 数据建模成立；**§11.1/§11.4/§11.5/§11.6 大面积未实现** |
| 12 | 批量导入与实时状态 | 事件订阅与调度骨架成立；**失败孤儿、取消吞掉、进度权重未实现** |
| 13 | 发布流程 | 可见性原子成立；**崩溃恢复、typed 结果、每题问题、资产回收未做** |
| 14 | 设置页 | UI 已收敛；**除模型连接外所有偏好未接入后端** |
| 15 | 错误处理与文案 | **类型零实现**，原始机器码仍在多条路径直送用户 |
| 16/17 | 逐文件改造清单 | **落地率极低**：前端 1 达成 / 13 部分 / 2 未动；后端 0 达成 / 5 部分 / 7 未动 |
| 18 | 分阶段实施计划 | 0 complete / 6 partial / 3 not started |
| 19 | 测试与验收体系 | **7 层金字塔几乎全空**；Golden 指标不可度量 |
| 20–23、25 | 删除清单/PR/分工/指标/边界 | 删除清单 11/12 未执行；指标与灰度零实现 |
| 24 | Definition of Done | **满足 0 / 部分 10 / 不满足 15 / 无法验证 5** |
| 26–28 | 自审记录/矩阵/建议 | **文档自洽通过，实现层落空**；矩阵失真；冻结无强制 |
| 附录 A | 证据索引 | 2 条路径已失效（`ExamCanvasV2.tsx` 相关） |
| 附录 B | 交付物清单 | 7/7 contracts、技能包、多个模块目录**未产出** |
| 附录 C | 最终决策摘要 | 12 条中 3 条零实现（云端 full candidate、TTL cleanup、最小化诊断） |

---

## 6. 与上一轮审计（audit-2026-09-07）的关系

### 6.1 已确认修复（本轮独立证伪旧结论）
- **F01** 应用无法编译 → `cargo check --locked` **通过**（主线程实测）。
- **F02** 迁移读版本指针当题稿 → 已改走 `read_current_revision → read_revision`（`migration.rs:30-36`）。
- **F04** 提交后整份覆盖 DB 的丢稿竞态 → 单事务、提交后无整份写回（`library/commands.rs:62-80`），未发现新竞态。
- **F05/F06** 慢保存丢编辑 / 保存失败仍发布 → 前端 drain 循环 + `flush()` 抛错阻断发布**成立**（但回归脚本未接入 npm，证据不可复现）。
- **F07/F08/F09** 队列认领 SQL 绑参、源文件未登记、恢复只覆盖 running → **逐条确认已修复**（`queue.rs:114-122`、`job_commands.rs:206-221`、`queue.rs:101-105,240`）。
- **F11** 结构修复未接入工作区 → 已接入（`structureActions.ts` + `SelectionInspector.tsx`）。
- **F13** 生产包含测试替身 → 已由 `import.meta.env.DEV` 短路移出生产 bundle。
- **F12** canonical 发布绑定 shadow → 产品路径已改用冻结 DS + 版本 CAS（但文档所述修复点在死路径）。

### 6.2 历史项仍未修复
- **F03** 多窗口 request id：修复轮改为每批 UUID，但**并发窗口下的 baseVersion 冲突处理仍只做"一句提示"**（A7-F08），§15.3 第 6 行的自动拉取与重放未实现。
- **F10** 批量发布原子性：**可见性原子已成立**，但崩溃恢复与 releases 回收仍缺（A9-F02）。
- **F-M0-3** 降级 UI 暴露原始错误码+路径：**只修一半**（A7-F04）。

### 6.3 本轮新增（上一轮未指出）
A1-F01（基线漂移）、A2-F02（local/cloud 非并发）、A4-F01（导入失败孤儿）、A4-F02（取消被吞）、A4-F03（恢复谎报）、A4-F05（lease 覆盖 local_status）、A5-F06（`paragraphMap` 恒空）、A6-F05（Evidence Resolver 缺失）、A7-F02（`user_edited` 假护栏）、A8-F01（§10.1 前提虚假）、A9-F02（发布无崩溃恢复）、A11-F03（PDF 语料缺失）、A12-F01（PR-01 改行为）、A13-F01（四轮审计未审实现）。

---

## 7. 证据层级声明与局限

| 层级 | 本轮覆盖 |
|---|---|
| `product`（真实 Tauri + WebView2 + SQLite + 文件系统端到端） | **仅 1 条且为失败证据**（A11-F01，2026-09-05 报告，早于当前 HEAD） |
| `command`（Tauri 命令 / Rust 服务层） | `cargo check` 通过；`cargo test` **未运行** |
| `static`（静态代码阅读 + 全局 grep） | 本轮 157 条发现的**绝大多数** |
| `cli` / `schema` | contracts 与脚本存在性核对 |
| `browser-dev-fallback` | 溢出矩阵 72/84 项、13 步冒烟——**不可作为产品验收证据** |
| `doc-only` | 第 26–28 章与附录的文档级核对 |

**明确局限**：

1. **基线未冻结**（见 §0），结论具有时序性；`recognition/local/` 相关判定以 11:47 快照为准。
2. 未运行 `cargo test` / `npm run build` / e2e / 真实 LLM / 真实 NAS，因此**任何"通过"结论都不构成产品验收**。
3. 100-PDF 语料、50 文件 batch、崩溃/断网/低磁盘故障矩阵、性能目标（§19.8/§19.9）**均未执行**。
4. NAS 学生端运行时刻未验证；跨仓 fixture 为 `#[ignore]`。
5. 部分子代理结论因基线漂移相互冲突，已由主线程裁定并标注（§0）。

---

## 8. 建议的下一步（按优先级）

1. **立即固化基线**：提交当前工作树并记录 SHA；此后每轮审计前禁止并发写入。
2. **修正计划文档的事实层**（成本低、收益高）：更新 §1.2 前端清单与缺口矩阵、修正 §10.1 虚假前提、修正 §16/§17 的文件路径、同步附录 A 失效路径、把 §26 的结论改为"文档自洽复核"。
3. **补齐产品级验收门**：修复 `artifacts/e2e-tauri` 的 `title-persists` / `publish` 两个失败步骤；补 `scripts/e2e/tauri-workspace-edit.mjs` 与 `tauri-publish.mjs`；接入 CI 作为 PR 门。
4. **修数据正确性缺陷**（用户可感知）：A4-F01 导入失败孤儿、A4-F02 取消被吞、A3-F04 空操作开关、A7-F04 原始错误码直送 UI。
5. **再动识别与云端**：M4 需把 `QuestionLayoutGraphV1` 真正接入主链（当前仅为附加 artifact）并实现 §6.5–§6.8；M5 需先建 skill bundle 与 `CloudRecognitionCandidateV1`，否则 §7/§8 永远停留在纸面。
6. **在 M5 落地前先修 `user_edited` 护栏**（A7-F02）：否则云端一旦获得写 canonical 的能力，会立即覆盖用户编辑。
