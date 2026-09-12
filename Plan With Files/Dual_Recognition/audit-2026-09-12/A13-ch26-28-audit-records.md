# A13 — 第 26–28 章对抗审计记录审计

- 审计对象：`Plan With Files/IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md` 第 26 章（3986–4181）、第 27 章（4184–4206）、第 28 章（4209–4232）
- 交叉背景：`Dual_Recognition/audit-2026-09-07/report.md`（13 项发现）、`repair-2026-09-07/progress.md`、`task_plan.md`、仓库根 `AGENTS.md`
- 代码现实：实际 HEAD `47a3806` + 未提交工作树（`src-tauri/src/processing/`、`src-tauri/src/recognition/` 均为 untracked）；文档声称的冻结基线为 `bb978be`
- 审计方式：只读。文档行号均为计划文档内行号；代码引用为 `path:line`。未运行 `cargo build/test`、`npm run build`

---

## 章节范围

| 章节 | 行号 | 内容 |
|---|---|---|
| 26 章 | 3986–4181 | 四轮对抗审计记录（26.1–26.4，每轮 3–5 个攻击问题 + “通过”结论） |
| 27 章 | 4184–4206 | 用户需求到任务的追踪矩阵（18 行） |
| 28 章 | 4209–4232 | 最终执行建议（4 个“最先启动的 PR” + 6 个冻结文件） |

第 26 章开头（3988）明确宣称：“每一轮都以‘假设该计划会失败’为前提，**重新对照用户需求和当前代码**，记录发现的矛盾并将修订写回正文。”本审计的核心即验证这句话。

---

## 修订落地核对表

判定口径：**已落** = 声称的修订内容确实存在于被指向的正文章节；**部分落** = 只落了一部分（或缺关键要件）；**落空** = 正文对应章节没有该内容。另附“实现层”列，区分 `doc-only` 与 `static`（代码静态证据）。

### 26.1 第一轮（产品简化）

| 轮次 | 攻击问题 | 声称修订指向 | 正文实际内容 | 判定 | 实现层 | 证据行号 |
|---|---|---|---|---|---|---|
| 26.1 | A 嘴上三页面 | §3 路由硬收敛 | `RouteName = library/workspace/settings` 已写入 | 已落 | doc；代码 router.ts 仍有第 4 个 `"legacy"` | 计划 226–237；`src/app/router.ts:6` |
| 26.1 | A | §16 App.tsx/router/AppShell 删除重写 | §16.1/16.2/16.3 三节齐备 | 已落 | doc；App.tsx 已重写为三页 + legacy writing | 2453–2514；`src/app/App.tsx:26-37` |
| 26.1 | A | Import→Library drawer / Source review→drawer / Publish→action | §3.1 流程图与 §13.1 已写 | 已落 | doc | 239–248、2236–2240 |
| 26.1 | A | §20 退休页面与代码清单 | §20.1/20.2 清单存在 | 已落 | doc；文件实际未删（P10 延后） | 3672–3704 |
| 26.1 | B | §4.4 有界恢复（hidden/bounded/非题库版本） | 两张恢复表 + 100 条 journal 策略 | 已落 | static：`library_item_recovery_v1`/`editor_journal_v1` 已建 | 417–444；`library/schema.rs:107,115` |
| 26.1 | B | §4.5 source evidence 可配置 | 三类文件生命周期 + 默认开启 | 已落 | doc | 446–462 |
| 26.1 | B | §11.5 清理不依赖正常退出 | “应用正常关闭不是唯一触发点” | 已落 | doc | 2082–2109 |
| 26.1 | C | §3.4 禁止暴露词 | 技术词清单 + 用户文案清单 | 已落 | doc | 302–329 |
| 26.1 | C | §8.5 ActionableIssue | `ActionableIssueV1` 结构定义 | 已落 | doc；后端零实现（前端派生） | 1479–1504 |
| 26.1 | C | §13.3 发布门缩为业务闭包 | 保留/移除清单 | 已落 | doc | 2254–2280 |
| 26.1 | C | §14 环境诊断移入高级/开发者 | §14.1/14.2 已写 | 已落 | doc | 2341–2391 |

### 26.2 第二轮（数据丢失与可行性）

| 轮次 | 攻击问题 | 声称修订指向 | 正文实际内容 | 判定 | 实现层 | 证据行号 |
|---|---|---|---|---|---|---|
| 26.2 | A 长事务 | §12.2 两阶段短事务 | `stage_import_batch` + 短事务 + commit 后 promote | 已落 | static：`processing/commands.rs` 存在对应函数 | 2152–2180 |
| 26.2 | B async 阻塞 | §5.5 `spawn_blocking` | 伪代码含 `tokio::task::spawn_blocking` | 已落 | static：scheduler 用 `spawn_blocking`，local/cloud 独立 semaphore | 616–664（630）；`processing/scheduler.rs:57-68,267` |
| 26.2 | B | 本地/云端/renderer 独立 semaphore + command 只 enqueue | 三句已写 | **部分落** | static：semaphore 已落；但 §5.5 伪代码用 `tokio::join!` 并发，实现为**本地 await 完再跑云端**（顺序） | 4047–4049；`scheduler.rs:267-284` 后 `:334` |
| 26.2 | C 改字符改 JS | §9.6 语义等价 | “canonical text node → Canvas → 发布时 compiler” | 已落 | doc（设计层） | 1675–1692 |
| 26.2 | D 删 revision | §17.15 分阶段 | “短期保留读、停止新写；长期删目录” | 已落 | static：迁移改走 `read_current_revision`→`read_revision` | 2933–2950；`library/migration.rs` |
| 26.2 | D | “§18 Phase 2 明确迁移幂等**和回滚**” | §18 Phase 2 只写“旧题迁移幂等”，**无“回滚”** | **部分落** | doc：`回滚`仅出现在 §24 DoD（3951），不在 §18 Phase 2 | 4067；对照 3207–3213、3951 |

### 26.3 第三轮（云端链路）

| 轮次 | 攻击问题 | 声称修订指向 | 正文实际内容 | 判定 | 实现层 | 证据行号 |
|---|---|---|---|---|---|---|
| 26.3 | A 证据不可信 | 新增 §7.11 Cloud Evidence Resolver | 该节存在，含 `resolve_cloud_evidence` 伪码 | 已落 | **落空**：全仓无 `resolve_cloud_evidence`/`EvidenceResolutionError` | 1345–1366 |
| 26.3 | B 无条件二次校对 | §7.13 条件式触发 | 五个触发条件已写 | 已落 | **落空**：无 proofreader / 二次校对调用 | 1379–1389 |
| 26.3 | C 静默截断 | §7.12 分片阈值 | 15 页阈值 + 按题组边界拆分 | 已落 | **落空**：无 shard / page manifest | 1368–1377 |
| 26.3 | D 隐式串行 | §17.9 `LlmTransport: Send + Sync` | trait 定义已写 | 已落 | **落空**：无 `LlmTransport`；且前提“`Mutex<FnMut>`”字面不存在 | 2883–2889 |
| 26.3 | E 整包丢弃 | §7.9 salvage + §7.6 一次 repair | `salvage_valid_task_groups` 与状态机已写 | 已落 | **落空**：无 `SalvageResult`/`CloudResponseState` | 1284–1299、1172–1235 |

### 26.4 第四轮（WYSIWYG / 跨仓 / UI 回归）

| 轮次 | 攻击问题 | 声称修订指向 | 正文实际内容 | 判定 | 实现层 | 证据行号 |
|---|---|---|---|---|---|---|
| 26.4 | A NAS renderer parity | “发布定义改为同一 renderer contract…并以实际 NAS renderer 做自动截图 parity”；“在 PR-07/PR-14 增加 cross-repo fixture” | **仅在 §26.4 自述**：§9.1/§13 无该发布定义；§19.6 无 NAS/跨仓；§21 PR-07/PR-14 无 cross-repo fixture | **落空** | static：只有 manifest 契约校验（非截图 parity），跨仓 fixture `#[ignore]` | 4136–4139；对照 1535–1559、3591–3610、3766–3768、3794–3796；`scripts/e2e/nas-student-contract.mjs`、`cross_repo_contract_fixture.rs:86` |
| 26.4 | B textarea 不再 WYSIWYG | §9.3 原位 auto-size textarea | 已写，含 IME/composition | 已落 | static：`InlineTextEditor.tsx` 已实现，注释引 §9.3 | 1580–1610 |
| 26.4 | C 只加 overflow-wrap | §10 根因整改 | §10.1–10.4 已写，含删三栏、不抬 minWidth、125%/150% 断言 | 已落 | static：styles.css 拆为 13 行 import；layout-matrix deviceScaleFactor 含 1.25/1.5 | 1775–1856、1950+；`src/styles.css`、`fixtures/ui/long-content.json:159,165` |
| 26.4 | D devFallback 假绿 | Phase 0 真实 Tauri smoke 设为启动门；§19.7；devFallback 出生产 bundle | §19.7 已写；chunk 已从 dist 消失 | **部分落** | static：`e2e:tauri` **未接入任何 CI**；“启动门”实为本地脚本；`devFallbackBackend.ts` 仍存在 176KB | 4161–4169、3612–3630；`.github/workflows/*.yml` |
| 26.4 | E 品牌资产 | §10.5 功能/视觉同构验收 | 已写，明确不复制商标 | 已落 | doc | 1895–1905 |

**落地统计（共 29 个修订条目）**：已落 26、部分落 2、落空 1。若只看“修订是否写进正文”，第 26 章的自述基本属实；但 26.3 的 5 个条目在**实现层全部落空**，26.4 A 在**正文层即落空**。

---

## 追踪矩阵核对表

矩阵共 18 行，**无“完成状态”列**。下表“设计匹配度”指引用章节是否真的覆盖需求，“任务匹配度”指 §18 是否真实存在且内容相符。

| 行号 | 需求 | 设计章节 | 匹配度 | 实施任务 | 任务匹配度 | 判定 | 说明 |
|---|---|---|---|---|---|---|---|
| 4188 | PDF 放入后本地 V2 识别 | 5、6 | 章节存在 | P4-T01~T06 | 存在且相符 | 已落 | M4 仅起步（P4-T01） |
| 4189 | 云端并发识别**完整 PDF** | 5、7 | §7 整章未实现 | P5-T01~T05、P6-T01~T02 | 存在 | **失真** | 云端仍只产 `CloudReadingOutlineV1`；§5.5 并发设计未落（实现为顺序）；M5 pending |
| 4190 | 同步 prompt/skill/题型/schema | 7.2~7.5 | 未实现 | P5-T01~T03 | 存在 | 设计未落 | — |
| 4191 | 云端 JSON 无法解析时内部处理 | 7.6~7.9 | 未实现 | P5-T04~T05 | 存在 | 设计未落 | 无 repair/salvage 代码 |
| 4192 | 第二条云端校对链 | 7.10、8 | 未实现 | P5-T06、P6-T03 | 存在 | 设计未落 | 无校对调用 |
| 4193 | 本地 JSON/云端 JSON 比对 | 8 | 未实现 | P6-T03 | 存在 | 设计未落 | 无 reconcile engine（唯一 `reconcile*` 是 V1 无关助手 `authoring_pipeline.rs:1917`） |
| 4194 | 编辑不应是独立技术页面 | 3、9 | 已覆盖 | P3-T01~T06 | 存在 | 已落 | — |
| 4195 | 最终渲染即编辑界面 | 9 | 已覆盖 | P3-T02~T05 | 存在 | 已落 | — |
| 4196 | 改一个字符同步最终输出 | 9.4~9.6 | 已覆盖 | P3-T03、P2-T03 | 存在 | 已落 | — |
| 4197 | 只保留题库/编辑/设置 | 3、16 | 已覆盖 | P1-T01~T04、P8-T01 | 存在 | 已落 | legacy 路由仍在 |
| 4198 | 题库只保留最终 DS | 4、11 | 已覆盖 | P2-T01~T04、P7-T05 | 存在 | 已落 | — |
| 4199 | 关闭后清理过程文件 | 11.5 | 已覆盖 | P7-T05 | 存在 | 已落 | — |
| 4200 | 批量 PDF 任务列表和转圈状态 | 3.3、12 | 已覆盖 | P1-T02~T03、P6-T01 | 存在 | 已落 | — |
| 4201 | 返回题库后任务继续 | 5、12.5 | 已覆盖 | P6-T01、P6-T04 | 存在 | 已落 | 12.5 为“重启恢复”，P6-T04 为 live update，口径略偏 |
| 4202 | 前端简洁美观、解决溢出 | 10、16 | 已覆盖 | P1-T04、**P3**、P8-T01 | **“P3”非任务编号** | 部分 | “P3”未细化到 T 级，不可追溯 |
| 4203 | 不暴露大量 hash/安全诊断 | 3.4、13、14 | 已覆盖 | P7-T02~T04 | 存在 | 已落 | — |
| 4204 | 深入代码并指出改哪些模块 | 1、16、17 | 已覆盖 | **全部 PR** | **不可追溯** | 部分 | 无任务编号，不可验证 |
| 4205 | 至少三轮对抗审计 | 26 | 存在 | **本文四轮** | **自指** | 部分 | 无外部验证；见下节裁定 |

### 矩阵遗漏检查

- 对照 §0.1 的五个核心问题（16–22 行）：信息架构、CSS 溢出、本地 V1 识别、云端 outline + localStorage 队列、多界面编辑——**五者均在矩阵中有行**（4194/4202/4188/4189/4195），无整项遗漏。
- 对照 §2.1–2.6 六组验收标准：2.1 导入（4200/4201）、2.2 本地识别（4188）、2.3 云端识别（4189–4193）、2.4 WYSIWYG（4194–4196）、2.5 题库发布（4198/4199）、2.6 设置（4203）——覆盖。
- **口径遗漏（非整项）**：§2.5（212）要求“**成功发布后**自动清理未被 DS 引用的过程文件”，矩阵仅以 4199“**关闭后**清理过程文件（§11.5）”映射；§11.5 确有“commit 后清理”函数，但矩阵未把该触发点显式对应到 §2.5 的发布后清理。

---

## 发现清单

### A13-F01 [P0] 第 26.3 章五个“已修订”在代码中零实现，四轮审计只审了文档、未审实现

- **结论**：26.3 的攻击 A–E 声称的修订（§7.11 Evidence Resolver、§7.13 条件式校对、§7.12 分片、§17.9 `LlmTransport: Send+Sync`、§7.9 salvage + §7.6 一次 repair）在正文中确实存在（`doc-only` 已落），但**代码中无一实现**：全仓 grep `resolve_cloud_evidence` / `EvidenceResolutionError` / `salvage_valid_task_groups` / `SalvageResult` / `LlmTransport` / `CloudResponseState` / `ReconciliationProposalV1` 均为 0 命中；云端唯一契约仍是 `CloudReadingOutlineV1`。
- **证据**：计划 1345–1366、1379–1389、1368–1377、2883–2889、1284–1299、1172–1235；`src-tauri/src/llm_suggestions.rs:235`（`"schema": "CloudReadingOutlineV1"`）；`task_plan.md:32`（M5 pending，云端仍只产对照提纲）；`audit-2026-09-07/report.md:121`（M5 core replacement not delivered）。
- **影响**：26.3 结论“通过。Cloud 已从…变成 versioned full candidate、证据重绑定、一次修复、分组 salvage 和条件式 proofreader”会被读作“设计已验证并推进”。实际上第 26 章开头宣称“对照当前代码”，但该章没有任何 `file:line` 代码引用，第 26.3 轮尤其只对照了正文文本。这是**过度声明**。
- **建议**：在 26.3 结论与 §27 相应行标注“设计已定稿 / 实现未开始（M5 pending）”，并给出代码核验命令；或将“四轮审计”限定为“文档自洽审计”。

### A13-F02 [P1] 26.4 A 的修订落空：cross-repo NAS 截图 parity 未写进任何被指向章节

- **结论**：26.4 A 声称“WYSIWYG 的发布定义改为…并以实际 NAS renderer 做自动截图 parity”“在 PR-07/PR-14 增加 cross-repo fixture”。但 §9.1（renderer 决定）、§13（发布）无该定义；§19.6（Author/Student Parity）只写同仓 author/student DOM+截图比较，无 NAS/跨仓；§21 PR-07（3766–3768）与 PR-14（3794–3796）无 cross-repo fixture。该“修订”只存在于 §26.4 自述。
- **证据**：计划 4136–4139 对照 1535–1559、3591–3610、3766–3768、3794–3796；代码侧仅 `scripts/e2e/nas-student-contract.mjs`（校验 manifest 规则，非像素 parity），`src-tauri/src/cross_repo_contract_fixture.rs:86` 为 `#[ignore]`。
- **影响**：这正是“修订只在本章声称、未写回正文”的典型样本；也使 26.4 结论“加入实际 NAS renderer parity”不成立。
- **建议**：把 parity 定义补入 §9.1/§13，把 cross-repo fixture 补入 §21 PR-07/PR-14 与 §19.6，否则删除该条结论。

### A13-F03 [P1] 26.2 D 部分落空：“§18 Phase 2 明确迁移幂等和回滚”中的“回滚”不在 §18 Phase 2

- **结论**：26.2 D 声称第 18 章 Phase 2 明确了“迁移幂等**和回滚**”。§18 Phase 2 验收（3207–3213）只写“旧题迁移幂等”，无“回滚”；`回滚` 一词在全文仅出现在 §1.5 CLD-003（156）与 §24 DoD（3951）。
- **证据**：计划 4067 对照 3207–3213、3951。
- **影响**：轻度修订落空；矩阵/DoD 与 Phase 计划口径不一致。
- **建议**：在 §18 Phase 2 补回滚验收，或修正 26.2 表述。

### A13-F04 [P1] 第 27 章矩阵失真：无完成状态列，把未实现的 §7/§8 设计呈现为已覆盖

- **结论**：矩阵 18 行全部以“设计章节 + 实施任务”呈现，**没有完成状态列**。行 4189（云端并发完整 PDF→§5、§7）、4191–4193（§7.6~7.9、§7.10、§8）所指向的设计**整章零实现**（M5 pending）。行 4189 还宣称“并发”，而实现是本地 await 完再跑云端的顺序执行。
- **证据**：计划 4189–4193 对照 1172–1389、1406–1531；`processing/scheduler.rs:267-284` 与 `:334`（顺序）；`task_plan.md:32`。
- **影响**：矩阵是主线程/评审据以判断“需求是否被接住”的表；缺少状态列会让读者把“有设计章节”误读为“有交付”。
- **建议**：增加“设计状态 / 实现状态 / 证据”三列；对 M5/M4 相关行显式标注 pending。

### A13-F05 [P1] 第 28 章“冻结状态”无任何强制手段，且 6 个文件中有 4 个在计划后被改动

- **结论**：§28（4220–4231）要求 6 个文件进入“只修阻断 bug、不加新功能”的冻结状态，但仓库**没有 CODEOWNERS，也没有针对这 6 个文件的路径级 CI 门禁**。`product-convergence-gates.yml` 只做产品面漂移（route/命令/表/flag）、溢出矩阵、浏览器主链与 Rust check/test，无法识别“在这些文件里新增业务功能”。
- **证据**：`.github/CODEOWNERS` 不存在；`.github/workflows/product-convergence-gates.yml:47-108`；`windows-smoke.yml` 亦无相关门。
- **文件改动核对**（`git log --oneline -- <file>` + `git status`）：
  - `src/pages/UnifiedPreview.tsx`：最后改动 `bb978be`（计划基线）——未再改。
  - `src-tauri/src/authoring_pipeline.rs`：最后改动 `5248c3d`（早于基线）——未再改。
  - `src-tauri/src/auto_pipeline.rs`：最后提交 `bb978be`，**工作树 M（未提交改动）**——已改。
  - `src/styles.css`：最后改动 `5daa04f`（冻结基线提交）——已改（现为 13 行 import，符合 §10.2，非临时功能）。
  - `src-tauri/src/lib.rs`：最后提交 `47a3806`（M1），**工作树 M**——已改。
  - `src/services/devFallbackBackend.ts`：最后提交 `47a3806`，**工作树 M**，文件仍存在（176 KB）——已改。
- **影响**：冻结是**口头建议**，无流程约束；`auto_pipeline.rs`/`lib.rs`/`devFallbackBackend.ts` 的后续改动虽多为计划内工作（M0/M1/P4-T01、F13），但“冻结”本身无法被验证或阻止。
- **建议**：加 CODEOWNERS 或一个“冻结文件 diff 需显式标注理由”的 CI 检查；或把 §28 措辞从“冻结状态”降级为“优先不在这些文件新增功能”。

### A13-F06 [P2] 第 26 章的攻击多为自我确认式弱攻击，缺少可失败判据

- **结论**：第 26 章开头宣称以“假设该计划会失败”为前提。但 26.1 的三条攻击（隐藏导航、只留一个 DS、置信度暴露）攻击的正是计划**已决定要做**的事——其“修订”基本是对同一决定的重述（例如攻击 A 的修订就是 §3/§16/§20 的既有内容）。26.3 的五条攻击同样把“当前实现是 outline”作为前提，而“改成完整 candidate”本就是 M5 的既定目标。**没有一条攻击给出可失败的判据**（如“若 X 断言为真则该设计不成立”），也没有任何 `file:line` 代码引用，与“对照当前代码”的宣称不符。
- **证据**：计划 3988、3992–4026、4073–4124。
- **影响**：四轮“通过”实质是**文档自洽性检查**，而非对计划的证伪性攻击；结论的强度被高估。
- **建议**：为每条攻击补“可失败判据 + 代码证据 + 失败时的后果”；把 26 章改名为“文档自洽复核”或补充实现层核验。

### A13-F07 [P2] 26.3 D 的攻击前提“当前 `Mutex<FnMut>`”在代码中字面不成立

- **结论**：26.3 D 称“现有 worker…在同一个 mutable gateway lock 下顺序执行”“当前 `Mutex<FnMut>` 会让云端内部操作串行”。代码中 `llm_gateway.rs` 与 `scheduler.rs` 均**无 Mutex**；`FnMut` 只作为 `auto_pipeline.rs` 的进度回调泛型约束存在（`&mut F: FnMut(...)`）。真实机制是 `&mut FnMut` 回调贯穿 pipeline 导致的顺序，而非 `Mutex<FnMut>`。
- **证据**：计划 4104–4112；`src-tauri/src/auto_pipeline.rs:388,413,1059`；全仓 grep `Mutex<...Fn` 无命中。
- **影响**：攻击针对了一个不存在的具体机制（稻草人），削弱该轮“对照当前代码”的可信度。
- **建议**：改为描述真实的 `&mut FnMut` 回调串行，并给出 `auto_pipeline.rs` 行号。

### A13-F08 [P2] 26.1 结论“三主表面已落实到 route”与代码存在第 4 路由、退休页未删的事实张力

- **结论**：26.1 结论称“三主表面已经落实到 route、组件、命令和退休文件清单，而不是视觉改名”。但 `src/app/router.ts:6` 的 `RouteName` 仍含第 4 值 `"legacy"`，§20.2 的退休文件（UnifiedPreview、StructuredAuthoringEditorV2、LibraryExamDetail、DocumentReview 等）**全部仍存在于源码树**（P10 延后）。
- **证据**：计划 4026、3689–3704；`src/app/router.ts:6,78-81`；`src/pages/*` 文件存在性核对；`src/app/legacyRoutes.tsx:11-17`（仍 import 退休页，但自身已成孤儿）。
- **影响**：“已落实”在“路由/组件”层面成立，在“退休文件删除”层面不成立；措辞略过强。
- **建议**：结论补注“退休文件删除归 P10，本轮仅冻结入口”。

### A13-F09 [P2] 26.4 D 的“真实 Tauri smoke 设为启动门”未接入 CI

- **结论**：26.4 D 声称“Phase 0 把真实 Tauri smoke 设为所有重构的启动门”。`npm run e2e:tauri` 脚本存在，但**未出现在任何 GitHub workflow** 中；CI 的 surface-gates 用的是浏览器 + devFallback 冒烟（`e2e:library-workspace`）。
- **证据**：计划 4167；`.github/workflows/product-convergence-gates.yml:56-57`；`windows-smoke.yml`；`package.json` scripts。
- **影响**：“门”目前是本地/人工步骤，不是强制门；与 §19.7“替代 dev fallback 假绿”的意图存在落差。
- **建议**：把 `e2e:tauri` 纳入 windows-smoke 或独立 workflow（可容忍允许失败并显式记录）。

### A13-F10 [P2] 第 27 章两行不可追溯（“全部 PR”“本文四轮”）

- **结论**：行 4204“深入代码并指出改哪些模块”的“实施任务”填“全部 PR”；行 4205“至少三轮对抗审计”的“实施任务”填“本文四轮”。二者均无任务编号、无验证绑定，不可追溯。
- **证据**：计划 4204–4205。
- **影响**：矩阵可追溯性在这些行失效；尤其 4205 是**自指**（用第 26 章证明第 26 章）。
- **建议**：4204 改为具体任务编号或标注“跨切面”；4205 改为外部验证来源（如独立审计报告路径）。

### A13-F11 [P3] §4.4 journal 上界正文写 100、代码写 200

- **结论**：§4.4 写“最近 100 条小型 edit journal”，代码 `DELETE ... ORDER BY id DESC LIMIT 200`。
- **证据**：计划 424；`src-tauri/src/library/repository.rs:382-383`。
- **影响**：轻微口径漂移，不影响正确性。
- **建议**：统一为 100 或更新正文。

### A13-F12 [P2] §28 的“最先启动的 PR”与 task_plan 已完成状态自相矛盾

- **结论**：§28 把 PR-01/02/03/04/05/06 作为“最先启动”的工作。但 `task_plan.md` 的 M0–M3 已标 complete（PR-01↔P0、PR-02↔P1、PR-03↔P2、PR-05/06↔P3、PR-04↔M1 五表+Repository）；同时 task_plan 的 Phase Map 又把 P4（PR-04/05）标为 “next”，**与 M1 complete 冲突**。
- **证据**：计划 4211–4218；`task_plan.md:27-30`（M0–M3 complete）对照 `:68`（P4 next）。
- **影响**：§28 的执行建议已过时；task_plan 内部状态口径不一致，会误导“从哪开始”。
- **建议**：§28 更新为“PR-07 之后 / M4 起”；统一 task_plan 的 M 与 Phase 两套状态。

---

## 对“四轮审计通过”的裁定

**裁定：第 26 章宣称的“四轮对抗审计通过”在当前代码现实下不可信——它至多成立为“文档自洽性复核通过”，不成立为“设计经代码验证通过”。**

理由：

1. **文档层基本成立**：29 个修订条目中 26 个确实写进了被指向的正文章节，仅 1 个完全落空（26.4 A 的 cross-repo parity）、2 个部分落空（26.2 D 的“回滚”、26.4 D 的“启动门”）。若“通过”仅指“正文里能查到修订文本”，多数轮次成立。
2. **实现层大面积落空**：26.3 全部 5 个修订、§8 的 reconcile 设计、后端 `ActionableIssueV1` 在代码中**零实现**（M5 pending、云端仍只产 `CloudReadingOutlineV1`）。第 26 章开头声称“对照当前代码”，却无一条 `file:line` 代码证据，第 26.3 轮尤其只核对了正文文本。
3. **攻击是自我确认式的**：26.1/26.3 的攻击针对的是计划已决定要做的方向，修订多为既定内容的重述，且**没有可失败判据**。这类“通过”不构成证伪。
4. **与追踪矩阵叠加放大**：§27 无完成状态列，把 §7/§8 的**未实现设计**与已实现章节并列呈现（A13-F04），使“设计存在”被读作“需求已接住”。
5. **存在可直接证伪的具体错误**：26.4 A 的 cross-repo parity 未写回正文（A13-F02）；26.3 D 的 `Mutex<FnMut>` 前提字面不成立（A13-F07）；26.4 D 的“启动门”未进 CI（A13-F09）。

因此，可信的表述应是：**“四轮文档自洽审计通过；其中第一、二轮对应的设计已在 M1–M3 部分落地，第三轮（云端）与 §8 合并设计尚未实现，第四轮 A 的跨仓 parity 修订未写回正文。”**

---

## 证据层级与局限

| 结论 | 层级 | 说明 |
|---|---|---|
| 26.1–26.4 修订是否写进正文 | `doc-only` | 逐行核对计划文档；行号可复核 |
| 26.3 五修订零实现、§8 未实现、后端 ActionableIssueV1 缺失 | `static` | 全仓 grep + 关键文件阅读；未编译、未运行 |
| 26.2 A/B/D 与 §4.4、§9.3、§10 的实现层判定 | `static` | `processing/`、`library/`、`recognition/`、前端组件静态阅读 |
| §28 冻结文件是否被改 | `static` | `git log -- <file>` + `git status --short`；未跑构建 |
| 冻结无强制手段 | `static` | 无 CODEOWNERS；CI workflow 全文阅读 |
| §27 任务编号是否存在 | `doc-only` | 与 §18 任务编号集合逐一比对（全部存在） |

**局限**：

- 未运行 `cargo build/test`、`npm run build`，代码“零实现”判定基于符号级 grep 与关键文件阅读；若存在宏/代码生成或运行时字符串拼接路径，可能有遗漏（已对云端契约做 `CloudReadingOutlineV1` 反向确认，未见完整候选）。
- `src-tauri/src/processing/`、`src-tauri/src/recognition/` 为未提交工作树；`recognition/local` 的 `QuestionLayoutGraphV1` 已存在但 `materialize_question_layout_graph` 只**写产物、无人消费**（`auto_pipeline.rs:289-312`，无读取方），故不计为 M4 主链落地。
- “计划编制后”以文档基线 `bb978be`（编制日期 2026-09-04）为界；`5daa04f` 等后续提交是否算“违反冻结”取决于口径，本报告按提交时间如实列出并注明改动性质。
- 未审计第 0–25 章正文自身的正确性（属 A1–A12 范围），本报告仅在核对修订落地时引用其行号。
