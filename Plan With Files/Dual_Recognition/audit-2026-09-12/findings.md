# Findings — 逐章节对抗审计（audit-2026-09-12）

> 计划：`../IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md`
> 汇总报告：`./00-CONSOLIDATED-REPORT.md`；章节子报告：`./A1-*.md` ~ `./A13-*.md`
> 证据层级：`product` / `command` / `static` / `cli` / `browser-dev-fallback` / `doc-only`

---

## 0. 主线程独立复核（对子代理结论的裁定）

子代理并行运行期间工作树被并发修改，且部分结论基于子串匹配，因此主线程对 6 条关键结论做了独立复核。**其中 1 条被纠正、1 处冲突被裁定。**

### R1. §10.1「当前溢出原因」前提 —— **复核确认成立（A8-F01）**

对 6 个选择器逐个精确核对（排除子串误匹配）：

| 选择器 | 定义位置 | 组件使用点 | 可达性 |
|---|---|---|---|
| `.editor-grid` | **不存在**（`settings-editor-grid`/`phase5-editor-grid`/`slot-editor-grid` 是不同类） | — | — |
| `.llm-grid` | 仅 `legacy.css:8` 注释 | 无 | — |
| `.settings-grid` | 仅 `legacy.css:8` 注释 | 无 | — |
| `.review-grid` | `legacy.css:61`（360/1fr/380） | `pages/DocumentReview.tsx:97` | **不可达** |
| `.metric-row` | `legacy.css:54`（repeat(6,1fr)） | `pages/Dashboard.tsx:33,51`、`pages/LibraryPage.tsx:102` | **不可达** |
| `.app-surface` | **不存在** | 无 | — |

**裁定**：§10.1 所述 6 条"当前溢出原因"中，3 条选择器不存在、3 条只服务**不可达页面**。与项目自身 Phase 0 实测"72 项 0 溢出、死 CSS 才是真问题"一致。**A8-F01 成立。**

### R2. legacy 路由可达性 —— **裁定 A10-F02 部分有误，A12-F10 正确**

| 事实 | 证据 |
|---|---|
| `App.tsx` 只渲染 library / workspace / settings 三页 + `legacy` 下的 **WritingStudio** | `App.tsx:26-33` |
| `router.ts` 仅放行 `items`、`settings`、`legacy/writing`，**其余 legacy 路径一律重定向** | `router.ts:47`、`router.ts:114` |
| `legacyRoutes.tsx` **没有任何 import 者**（真孤儿） | 全仓 grep `legacyRoutes` = 0 命中（除自身） |

**裁定**：
- A12-F10（`legacyRoutes.tsx` 是孤儿）**正确**；
- A10-F02「退休页面由 `legacyRoutes.tsx` 统一 import」**错误**——该文件确实是孤儿，退休页面（DocumentReview/Dashboard/旧 LibraryPage/UnifiedPreview/StructuredAuthoringEditorV2/ExportPage）**均不可达**；
- A10-F02 的实质结论（第 16 章落地率极低）**仍成立**。

### R3. local/cloud 顺序执行 —— **确认成立（A2-F02 / A4-F08）**

`scheduler.rs:255-361` 实测为：`local permit → spawn_blocking(run_auto_pipeline_core) → await 完成 → drop → 若失败直接 fail_job 返回 → 之后才进入 cloud 分支`。**无 `tokio::join!`，无并发**，且本地失败时云端**根本不执行**。§5.5 伪代码与 §2.1/§3.2 的并发承诺落空。

### R4. 取消在云端窗口失效 —— **确认成立（A4-F02）**

`scheduler.rs:334-359`：云端 `spawn_blocking` 返回后**无取消检查、无迟到结果守卫**，直接 `advance(STAGE_READY_FOR_REVIEW, …, Some("succeeded"))`。取消仅在本地阶段前（`:261-265`）与云端**发起前**（`:312-316`）检查。§12.4"云端调用取消 request 或忽略迟到结果"未实现。

### R5. `user_edited` 是假护栏 —— **确认成立（A7-F02）**

`provenanceStatus` 只被写入（`repository.rs:205-223,306`），全仓无读取点；`ProposalOnly`/`MergeDecision` grep = 0。当前不覆盖用户编辑的唯一原因是**云端尚不具备写 canonical 的能力**——M5 落地即失效。

### R6. 基线漂移 —— **确认成立（A1-F01）**

审计窗口内新增未跟踪模块 `src-tauri/src/recognition/local/`（mtime 11:44:30 / 11:47:02），`auto_pipeline.rs`（11:43:48）同时被改写以引用它。**结论具有时序性，且当前工作树不可复现。**

**后续处置（2026-09-12）**：已按用户确认固化基线为 **`6affc571f43b175ffdb5a13d1823ba6f2d4962a3`**，并重录 `fixtures/product-baseline.json`（`--reason` 门通过，`--strict` 校验 `no drift`）。
并行写入者在本轮期间持续活动（提交后 `src-tauri/src/product_chain.rs` 仍被改写），因此**该 SHA 之后的新改动属于新基线**，本报告所有结论以 `6affc57` 为准。

### R7. 构建校验 —— **确认 F01 已修复**

`npm run check` 通过；`cargo check --manifest-path src-tauri/Cargo.toml --locked` 通过（12.49s，88 warnings）。

---

## 1. P0 发现（14 条，阻断级）

| ID | 章节 | 结论 | 证据 | 复核 |
|---|---|---|---|---|
| A1-F01 | §1 | 冻结基线 `bb978be` 与实际 HEAD/工作树不符，全章"当前实现"系统性过时 | 43 项未提交 | R6 已复核 |
| A2-F01 | §2.2 | "新任务直接以 DocumentIRV2 为输入"不成立，主链仍 V1-first | `auto_pipeline.rs:1692` → `:2067` | static |
| A2-F02 | §2.1/§3.2 | 单题 local/cloud 顺序执行，非并发 | `scheduler.rs:255-361` | **R3 已复核** |
| A2-F03 | §2.3 | 云端完整候选/repair/salvage/`ReconciliationProposalV1` 全部缺失 | grep = 0 | static |
| A4-F01 | §12.2 | 导入队列失败留下不可见孤儿文件（DB 无行、磁盘有 job.json+uploads） | `commands.rs:112-127`、`repository.rs:168-176` | static |
| A5-F01 | §6 | ~~第 6 章具名类型完全不存在~~ → **降级 P1**（模块已出现，但主链未接入） | 见 §2 | R6 裁定 |
| A5-F02 | §6.8 | 硬闭包只在前端；后端无 `validate_basic_task`，issue code 三方命名冲突 | `actionableIssues.ts:105` vs `issue_codes.rs` | static |
| A6-F01 | §7.3 | `CloudRecognitionCandidateV1` 零实现，云端仍只产 `CloudReadingOutlineV1` | `llm_suggestions.rs:235` | static |
| A6-F02 | §7.6 | 无 repair/salvage/状态机，单点校验失败即整份丢弃 | `llm_gateway.rs:1176-1365` | static |
| A7-F01 | §8 | 合并引擎整章零实现（`align_task_groups`/`can_auto_fill` grep = 0） | 全仓 grep | static |
| A7-F02 | §8.7 | `user_edited` 只有写入没有读取，M5 落地后必然覆盖用户编辑 | `repository.rs:205-223,306` | **R5 已复核** |
| A11-F01 | §24 | 唯一真实 Tauri E2E 报告 `verdict=failed`，且早于当前 HEAD | `artifacts/e2e-tauri/run-2026-09-05T11-34-16-269Z/report.json` | product（失败） |
| A11-F02 | §18 P0-T02 | 要求的 3 个 `scripts/e2e/tauri-*.mjs` 只有 1 个存在 | `scripts/e2e/` | static |
| A11-F03 | §18 P0-T01 | "8 份 Reading PDF fixture"源文件缺失，`privateCorpusReady=false` | `fixtures/private-real/` | static |
| A13-F01 | §26.3 | 5 条"已修订"的云端设计代码零实现——四轮审计只审文档未审实现 | 见 A6/A7 | static |

---

## 2. P1 发现（46 条）

### 数据面 / 多事实源
| ID | 结论 |
|---|---|
| A1-F02 | §1.2 目标文件 `src/components/ExamCanvasV2.tsx` 不存在，实际在 `src/exam-canvas/ExamCanvas.tsx` |
| A1-F03 | `App.tsx`「11 页面路由」与 `styles.css`「约 64 KB」两处硬数据均被推翻（现为 3 路由 / 582 字节） |
| A1-F04 | `router.ts`/`AppShell.tsx` 职责描述已被 Phase 1 推翻 |
| A1-F05 | `devFallbackBackend.ts` 判断列过时：F13 已修复 |
| A1-F06 | §1.5 缺口矩阵 6 条已修复仍按"当前缺口"呈现 |
| A1-F07 | §1.5 UX-002 根因与当前代码不符 |
| A1-F08 | LIB-001（多事实源）经 M1 后**仍成立**，与 M1"complete"口径存在张力 |
| A1-F09 | CODE-001 低估：`authoring_pipeline.rs` 547 KB，突破计划自设上界 |
| A3-F01 | §11.5 清理机制三个函数、四个触发点全部未实现 |
| A3-F02 | §11.4 目录布局与 §11.6 内容寻址去重未实现，深层目录树仍在 |
| A3-F03 | 双事实源未终止：新导入仍 `save_job` → 写 `job.json` + 双写 legacy DB |
| A3-F04 | §4.5"保留原文件"开关是空操作（仅 localStorage，Rust 零读取） |

### 后端调度 / 导入
| ID | 结论 |
|---|---|
| A4-F02 | 云端执行窗口内的取消被吞掉（**R4 已复核**） |
| A4-F03 | 启动恢复 `action_required` 分支向用户谎称已重试 |
| A4-F04 | 设置页的并发/云端开关与调度器完全脱节 |
| A4-F05 | 过期 lease 认领覆盖 `local_status`，破坏"本地已完成"事实 |
| A4-F07 | §5.1 目标模块布局绝大部分不存在（`reconcile/`、`editor/`、`publish/`） |
| A4-F08 | §5.2 状态模型类型全部缺失（`StageStatus`/`ProcessingItemV1`/`ProcessingStage`） |
| A4-F09 | §5.3 七条命令只落地 5 条且签名不符 |
| A5-F01 | （降级后）主链未交付：`recognize_local` 零命中，新图未接入主链 |

### 本地识别
| ID | 结论 |
|---|---|
| A5-F02 | 硬闭包只在前端，后端无 `validate_basic_task`，issue code 三方命名冲突 |
| A5-F03 | §18 Phase 4 五项验收指标零度量实现 |
| A5-F04 | §6.5–§6.7 token/geometry-first 算法全部未实现，现为行序启发式 |
| A5-F05 | §6.10 表格未使用 physical tables；hybrid 热点无 crop fallback |
| A5-F06 | §6.9 `paragraphMap` 恒为空，passage A-G 与 List of Headings 未分离 |

### 云端 / 合并
| ID | 结论 |
|---|---|
| A6-F03 | `recognition/skills/ielts-reading-v1/` 及 8 类示例完全不存在 |
| A6-F04 | 第二条校对链缺失，且未评估 100 份批量成本 |
| A6-F05 | Evidence Resolver 与 `IdAllocator` 缺失，canonical 入口无护栏 |
| A6-F06 | §26.3-D 要求的 `LlmTransport: Send + Sync` 未落地，gateway 仍串行 |
| A6-F07 | 大文件阈值与页覆盖校验缺失（"截断当完整"风险） |
| A7-F03 | issue 命名四方不一致；后端 `actionable_issues_v1` 是死表，前端丢弃后端 issues |
| A7-F04 | §15.1/§15.2 类型零实现；原始机器码仍在多条路径直送 UI（F-M0-3 只修一半） |
| A7-F05 | 内部细节"记录到日志"不成立：`fail_job` 丢弃完整 error 且无日志写入 |
| A7-F06 | §15.3 第 2 行与实现直接矛盾；`reconcile_status` 硬编码 `"succeeded"` |
| A7-F07 | §8.6 原位 diff UI 完全不存在，`reconciling` 为不可达死代码 |

### 前端 / UI
| ID | 结论 |
|---|---|
| A8-F01 | §10.1 前提虚假，与项目自身 Phase 0 实测矛盾（**R1 已复核**） |
| A8-F02 | §9.4 `EditorCommandV1` 只有 3/9 op；`expectedText` 不做服务端校验 |
| A8-F03 | §9.2 声称删除的 4 个产物全部仍存在并仍被引用；V1 HTML 仍直接渲染 |
| A10-F01 | 文档路径与仓库实际路径系统性不一致，按计划施工会找错文件 |
| A10-F02 | 第 16 章落地率极低，退休页面整套仍在（**import 归属经 R2 纠正**） |
| A10-F03 | 第 17 章核心零推进：`lib.rs` 11151 行、`authoring_pipeline.rs` 13848 行仍被调用 |
| A10-F04 | 孤儿文件无 import 但仍被 `tsconfig include:["src"]` 编译 |

### 发布 / 设置
| ID | 结论 |
|---|---|
| A9-F01 | 设置项未接入后端，并发/文件保留开关为装饰性控件 |
| A9-F02 | 批量发布无崩溃恢复，`releases` 永不回收且资产重复拷贝 |
| A9-F03 | 批量失败不返回每题问题，也无"仅发布通过项"二次动作 |
| A9-F04 | §13.4 typed `PublishCheckResultV1` 未交付，前端仍解析字符串 |

### 计划 / 流程
| ID | 结论 |
|---|---|
| A11-F04 | Golden Corpus 标注格式与 §19.2 不一致，10 项指标无计算实现 |
| A11-F05 | 云端/合并链完全缺失，§19.3 17 例与 §19.4 9 例覆盖率为 0 |
| A11-F06 | 无前端测试基础设施，§19.5 14 项与 §19.6 parity 无独立实现 |
| A11-F07 | 以 CLI/schema/browser-dev 证据冒充产品验收（over-claim 模式） |
| A11-F08 | Phase 8 删除项几乎全未执行，V1 主链仍在写 |
| A12-F01 | PR-01 声称"不改行为"，但基线冻结提交改了两处产品判定行为 |
| A12-F02 | 附录 A 两条源码路径已失效（`ExamCanvasV2.tsx` 相关） |
| A12-F03 | 附录 B 交付物：7/7 contracts 缺失、技能包与多个模块目录不存在 |
| A12-F04 | §23 监控指标与发布门零实现（10 指标无采集、7 目标无度量、6 步灰度无开关） |
| A12-F05 | §23.3"不允许同一新题由 V1/V2 双写"无代码强制，且确实双写 |
| A13-F02 | 26.4 A 的 NAS renderer 截图 parity 修订落空 |
| A13-F04 | §27 矩阵无完成状态列，把未实现的 §7/§8 与已实现章节并列 |
| A13-F05 | §28 冻结建议无强制手段，6 个"冻结"文件中 4 个在计划后被改 |

---

## 3. P2 / P3 发现（97 条）

完整清单见各章节子报告 `A1-*.md` ~ `A13-*.md` 的「发现清单」小节。按主题归纳：

- **数据/迁移**：A1-F10~F13、A3-F05~F10、A12-F06~F09
- **调度/导入**：A4-F06、A4-F10~F12、A12-F11
- **识别**：A5-F07~F09、A6-F08~F10、A6-F12
- **合并/错误**：A7-F08~F10、A7-F11
- **前端/UI**：A2-F07~F09、A2-F11、A2-F13~F14、A8-F04~F09
- **发布/设置**：A9-F05~F11、A10-F05~F08、A10-F11
- **测试/成本**：A11-F09~F11
- **文档自洽**：A2-F10、A2-F12、A5-F10、A8-F10~F12、A13-F03、A13-F06~F12

---

## 4. 与 audit-2026-09-07 的关系

### 已确认修复（本轮独立证伪旧结论）
F01（编译）、F02（迁移读指针）、F04（提交后覆盖 DB）、F05/F06（保存丢编辑/失败仍发布）、F07/F08/F09（队列绑参/源文件登记/恢复范围）、F11（结构修复未接入）、F13（生产含测试替身）；F12 产品路径已改用冻结 DS + 版本 CAS。

### 历史项仍未修复
- **F03** 多窗口 request id：并发冲突处理仍只做"一句提示"（A7-F08）
- **F10** 批量发布：可见性原子已成立，但崩溃恢复与 releases 回收仍缺（A9-F02）
- **F-M0-3** 原始错误码直送 UI：只修一半（A7-F04）

### 本轮新增（上一轮未指出）
A1-F01、A2-F02、A4-F01、A4-F02、A4-F03、A4-F05、A5-F06、A6-F05、A7-F02、A8-F01、A9-F02、A11-F03、A12-F01、A13-F01。

---

## 5. 证据层级与局限

| 层级 | 覆盖 |
|---|---|
| `product` | **仅 1 条且为失败证据**（A11-F01） |
| `command` | `cargo check` 通过；`cargo test` 未运行 |
| `static` | 绝大多数发现 |
| `cli`/`schema` | contracts 与脚本存在性 |
| `browser-dev-fallback` | 溢出矩阵 72/84 项、13 步冒烟——**不可作为产品验收证据** |
| `doc-only` | §26–28 与附录 |

**局限**：基线未冻结（§0/R6）；未运行 `cargo test`/`npm run build`/e2e/真实 LLM/真实 NAS；100-PDF 语料、50 文件 batch、故障矩阵、性能目标均未执行；NAS 学生端运行时刻未验证。
