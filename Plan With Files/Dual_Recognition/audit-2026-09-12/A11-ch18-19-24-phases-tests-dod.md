# A11 — 第 18 / 19 / 24 章对抗审计：阶段计划、测试体系、Definition of Done

- 审计日期：2026-09-12
- 工作树：HEAD `47a3806`（M1）+ 43 项未提交改动（其中 `src-tauri/src/processing/`、`src-tauri/src/recognition/` 为未跟踪新目录）
- 审计对象：`IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md` 第 18 章（3061–3472）、第 19 章（3474–3667）、第 24 章（3898–3952）
- 证据分层（AGENTS.md）：`product` / `command` / `cli` / `schema` / `browser-dev-fallback` / `doc-only`
- 只读审计，未修改任何产品代码；未运行 `cargo build/test`、`npm run build` 或任何 e2e 脚本。

## 章节范围

| 章节 | 内容 | 计划规模 |
|---|---|---|
| 第 18 章 | Phase 0–8 分阶段实施计划，9 个阶段，共 44 个任务（P0-T01…P8-T05） | 计划自称 71–112 工程日 |
| 第 19 章 | 测试金字塔 7 层 + Golden Corpus + Cloud Contract 17 例 + Reconciliation 9 例 + Editor 14 项 + Parity + 3 条 E2E + 12 故障场景 + 7 项性能目标 | — |
| 第 24 章 | Definition of Done，6 大类共 30 个 `[ ]` 勾选项 | — |

## Phase 完成度表

> 判定口径：`complete` = 出口条件在当前工作树有对应实现**且**证据达到该条件要求的层级；`partial` = 有实现但证据层级不足或出口条件部分满足；`not started` = 无实现；`over-claimed` = 计划/追踪文档宣称完成但出口条件未满足。

| Phase | 计划出口条件 | 实现证据 | 证据层级 | 判定 |
|---|---|---|---|---|
| **P0** 冻结基线（3–5 日） | P0-T01 固定 commit SHA/schema hash/8 份 Reading PDF/批量 fixture；P0-T02 真实 Tauri smoke 三个脚本；P0-T03 溢出矩阵 | `scripts/verify-product-baseline.mjs`（14 KB）✅、`fixtures/product-baseline.json` ✅（含 `commitSha`/`schemaHash`）；`fixtures/ui/long-content.json` ✅；`scripts/ui/layout-matrix.mjs` ✅；真实 Tauri 脚本**只有 1 个** `scripts/e2e/tauri-import-edit-publish.mjs`（计划要求 3 个） | cli / browser-dev-fallback / product（未通过） | **partial** |
| **P1** 外壳简化（5–8 日） | 三路由、题库统一列表、Import Drawer、CSS 整改；1100×760 无溢出 | `router.ts` RouteName = `library\|workspace\|settings\|legacy`；`AppShell.tsx` NAV 仅 2 项；`styles/` 分层 10 文件；`LibraryPage`/`features/import/*` 存在。溢出证据仅来自浏览器 CDP 矩阵 | command + browser-dev-fallback | **partial**（task_plan 标 complete，产品级无证据） |
| **P2** Canonical DS（8–12 日） | 新表/repository/migration/Workspace API；停止双写；迁移幂等 | `library/{schema,repository,migration,commands}.rs`；`library_items_v2` 等表；`get_workspace_item`/`apply_editor_commands`/`list_library_items` 已注册；library 测试 7 个 | command | **partial**（M1 宣称 complete，但仅命令级） |
| **P3** 唯一 WYSIWYG 工作区（10–15 日） | ExamWorkspacePage、Canvas 拆分、稳定文本编辑、题型 renderer、SourceDrawer、退休重复页面 | `features/editor/ExamWorkspacePage.tsx` 存在；`exam-canvas/` 4 模块。**renderer 按题型拆分未做**（task_plan M3 自认）、**NAS parity 未做**、退休页面**未从主 bundle 移除** | command + browser-dev-fallback | **partial / over-claimed** |
| **P4** 本地 DocumentIRV2 直通（12–20 日） | P4-T01…T06 | P4-T01 仅完成 DOCX 物理入口统一；**P4-T02 `recognition/local/{mod.rs,question_blocks.rs}` 于本次审计期间（11:37–11:39）新落盘、未跟踪、0 测试、未接入主链**；P4-T03–T06 无实现 | command（半成品） | **not started（仅 T01 增量）** |
| **P5** 云端完整识别（10–15 日） | versioned skill bundle、CloudRecognitionCandidateV1、repair/salvage | **全仓无 `CloudRecognitionCandidateV1` 标识符**；无 `recognition/skills/`；云端仍只产出 `CloudReadingOutlineV1` | — | **not started** |
| **P6** 双路并发与 Reconciliation（8–12 日） | Processing Queue、local/cloud 并发、Reconciliation Engine、live update、删前端 queue | `processing/{queue,scheduler,commands}.rs` 存在（queue 3 个测试）；`publish_items` 原子批发布；**Reconciliation Engine 不存在（无 `*reconcil*` 文件）**；P6-T05 前端 queue 删除状态未验证 | command | **partial** |
| **P7** 发布/设置/清理收敛（7–10 日） | 发布并入 Library/Workspace、typed preflight、batch NAS、设置简化、artifact cleanup | `publish_items_core` 整批原子 + 故障注入测试；typed preflight 存在；`cleanup.rs` 存在但**成功路径有、启动/退出无 hook**（`lib.rs` 无 `RunEvent::Exit`/startup cleanup） | command | **partial** |
| **P8** 旧链删除与真实回归（8–15 日） | 删退休前端、删 dev fallback、停 V1 写入、拆超大 Rust 文件、真实发布验收 | `src/pages/` 11 个退休页面**全部仍在**；`legacyRoutes.tsx` 仍在（仅未被 import）；`devFallbackBackend.ts` 仍在；`auto_pipeline.rs` 主链仍调 `make_dynamic_split_candidates`→`build_authoring_v2_shadow`；`lib.rs` **11151 行未拆分**；无 Windows 安装包/100 PDF/50 batch 报告 | doc-only（仅计划） | **not started** |

### 提交数与成本估算对照（成本低估论证）

- `git log --oneline 5daa04f..HEAD` = **2 个提交**（`401ca76` M0、`47a3806` M1）。
- 计划 Phase 0–8 合计自称 **71–112 工程日**（2–3 名工程师）。
- 即使把 43 项未提交改动（`32 files changed, 1082 insertions(+), 678 deletions(-)`，另有未跟踪的 `processing/`、`recognition/`、若干脚本）全部计入，实际可归因到该计划的已落盘规模，与 71–112 工程日的计划体量存在**数量级不匹配**：Phase 4–8 的核心内容（本地直通识别、云端完整候选、reconciliation、旧链删除、真实发布验收）几乎未落盘。
- 结论：计划把「基础设施 + 表面收敛」估成「全链路产品重构」的工程量，Phase 4–8 的估算缺乏依据。

## 测试体系覆盖表

| §19 小节 | 计划要求 | 实际覆盖 | 缺口 | 判定 |
|---|---|---|---|---|
| §19.1 层 1 Pure unit | geometry/token/stem/option/schema/merge/editor command | Rust 侧 `pdf_geometry`(32)、`ielts_grammar`(多)、`schema` 有；**merge / editor command 无单元测试** | **无任何前端测试基础设施**：`package.json` 无 vitest/jest，`src/**` 无 `*.test.*`/`*.spec.*` | 部分 |
| §19.1 层 2 Contract | Rust↔TS↔JSON Schema↔NAS | `verify-schema-contract.mjs`、`nas-student-contract.mjs` 存在 | 仅 CLI/schema 级，无运行时契约 | 部分 |
| §19.1 层 3 Fixture integration | PDF/DOCX→DocumentIRV2→candidates→DS→runtime | `phase0-golden.mjs` + `product_chain.rs`（9 测试，命令级） | 无「candidates→canonical DS→runtime」端到端断言 | 部分 |
| §19.1 层 4 Real Tauri E2E | import→processing→edit→publish | 1 个脚本 `tauri-import-edit-publish.mjs`（真 tauri-driver+selenium）；最新报告 **verdict=failed**（7 通过 / 2 失败），且时间为 2026-09-05，**早于 HEAD `47a3806`** | 报告过期，不能证明当前代码 | 部分 |
| §19.1 层 5 Visual regression | author/student canvas + overflow matrix | 仅 `layout-matrix.mjs`（浏览器 CDP） | 无 canvas 截图 diff、无 student canvas | **browser-dev-fallback** |
| §19.1 层 6 Fault injection | exit/network/disk/malformed cloud/conflicting edits | 见 §19.8 | 大面积缺失 | 部分 |
| §19.2 Golden Corpus | 标注含 `taskGroups/range/taskType/questions/options/prompt` + `unassignedAllowed`；10 项指标有计算实现 | 39 份 metadata 存在，但字段为 `displayRange`（非 `range`）、`kind`（非 `taskType`）、`slotIds`，**无 questions/options/prompt**；`unassignedAllowed` **0 份**；`metrics.json` 只有 `phase0-golden.mjs` 做**结构/schema 校验**，无任何指标计算消费者 | 标注格式与计划不一致；10 项指标**无计算实现**（与 A5 线索一致，已独立验证） | **not met** |
| §19.3 Cloud Contract 17 例 | 17 个具体用例 | **无 `CloudRecognitionCandidateV1` 类型**；仅有 `CloudReadingOutlineV1` 的 `validate_cloud_outline_output` | 17 例中 0 例有对应 fixture/测试 | **not met**（0/17） |
| §19.4 Reconciliation 9 case | 9 个 case | **无 reconciliation 模块/文件/测试** | 0/9 | **not met** |
| §19.5 Editor Command 14 项 | IME/emoji/删除/替换/debounce/flush/stale/并发/undo/刷新等 | 无前端测试框架；仅 `workspace-save-regressions.mjs` 以**受控 IPC 替身**覆盖 3 项保存链 | 0/14 有独立测试实现 | **not met** |
| §19.6 Author/Student Parity | 同 fixture 渲染两模式并比对 | 无 parity 测试 | 0 | **not met** |
| §19.7 三条真实 E2E | E2E-1/2/3 三个脚本 | 只有 1 个脚本，非计划中的 3 个场景；verdict failed | 缺 E2E-2（local+cloud 分歧）、E2E-3（batch 20 + restart） | 部分/未通过 |
| §19.8 故障注入 12 场景 | 12 场景 × 4 问 | 约 3–4 个有单元级覆盖（见下） | 见下 | 部分（≈30%） |
| §19.9 性能目标 7 项 | P50/P95/fps/搜索延迟等 | **无 benchmark、无基线数据、无度量实现**（`bench/p50/p95/fps` 仅出现在无关字符串） | 7/7 无实现 | **not met** |

### §19.8 故障注入逐条核对

| # | 场景 | 覆盖情况 | 证据 |
|---|---|---|---|
| 1 | PDF page render 失败 | ❌ 无 | — |
| 2 | 某一页 text layer 乱码 | ❌ 无 | — |
| 3 | cloud timeout | ⚠️ 仅有 `llm_timeout()` 实现，无测试 | `llm_gateway.rs:121` |
| 4 | cloud malformed JSON | ❌ 无（无 candidate 校验） | — |
| 5 | cloud repair timeout | ❌ 无 | — |
| 6 | 本地 worker panic | ❌ 无 | — |
| 7 | 应用强制退出 | ⚠️ 单元级：stale worker/expired lease 恢复 | `queue.rs:309,335` |
| 8 | 磁盘剩余空间不足 | ❌ 无 | — |
| 9 | SQLite busy/locked | ✅ `package_publish_rejects_busy_lock_before_commit` | `nas_package_v2.rs:1911` |
| 10 | source 文件导入中被删除 | ❌ 无 | — |
| 11 | 发布目标断开 | ❌ 无（无网络级故障） | — |
| 12 | 发布 staging 后进程退出 | ✅ `publish_items_is_all_or_nothing_across_a_batch` + manifest 中断恢复 | `product_chain.rs:1046`、`nas_package_v2.rs:1960/2038/2105` |

覆盖：约 4/12（33%），且全部为命令/单元级，无真实 Tauri/NAS 进程级故障注入。

## DoD 逐项核对表

> 「满足」= 当前工作树有实现且证据层级足够；「部分」= 有实现但证据层级不足；「不满足」= 无实现；「无法验证」= 存在实现但缺少可执行证据。

| 类别 | 勾选项 | 当前状态 | 证据 | 证据层级 |
|---|---|---|---|---|
| 产品面 | 普通导航只有题库和设置；打开题目进入工作区 | **部分** | `AppShell.tsx:9-12` NAV 仅 2 项；`App.tsx:31` 仍渲染 legacy `WritingStudio` | command |
| 产品面 | 导入在题库完成，不存在独立 wizard 主流程 | **部分** | `LibraryPage` 用 ImportDrawer；`pages/ImportWizard.tsx` 未删，仍在 `legacyRoutes.tsx:77` | command |
| 产品面 | 编辑界面即最终学生题面，不需要预览切换 | **无法验证** | workspace 存在；无 parity/学生题面证据 | — |
| 产品面 | 批量导入每题有独立实时状态 | **部分** | `processing/scheduler.rs` + `processing://item-updated`（command 级） | command |
| 产品面 | 发布在题库/工作区完成 | **部分** | `ExamWorkspacePage` 发布按钮 + `publish_items` | command |
| 数据面 | 新题只有一个 Canonical DS 权威稿 | **部分** | M1 repository（command 级）；产品级未验证 | command |
| 数据面 | job.json/legacy exams 不再参与新题写入 | **不满足** | 主链仍写 V1 shadow（见下） | doc-only |
| 数据面 | 运行时和 JS 只由 Canonical DS 编译 | **无法验证** | 无端到端编译证据 | — |
| 数据面 | 临时 artifact 能在成功/取消/启动/退出后清理 | **部分** | `cleanup.rs` 被 `authoring_commands/auto_pipeline/export_*` 调用（成功路径）；`lib.rs` **无 startup/exit hook** | command |
| 本地识别 | 新题 direct DocumentIRV2，不经 V1 authoring 主链 | **不满足** | `auto_pipeline.rs:187/1692/2449` 仍 `make_dynamic_split_candidates`→`build_authoring_v2_shadow` | command（证伪） |
| 本地识别 | 基础题型 Ready 时题干和选项完整 | **不满足** | 无 P4-T03/T04 实现 | — |
| 本地识别 | significant unassigned evidence 阻止错误 Ready | **不满足** | 无 P4-T06 实现 | — |
| 本地识别 | 表格/流程图/diagram 至少有一种完整呈现 | **不满足** | 无 P4-T05 实现 | — |
| 云端 | 使用 versioned skill bundle | **不满足** | 无 `recognition/skills/` | — |
| 云端 | 返回完整 CloudRecognitionCandidateV1 | **不满足** | 全仓无该标识符 | — |
| 云端 | malformed JSON 经过 validate/repair/salvage | **不满足** | 无 candidate/repair/salvage 实现 | — |
| 云端 | 第二次校对只产生 proposal | **不满足** | 无实现 | — |
| 云端 | cloud 失败不阻止打开本地稿 | **无法验证** | 无真实 E2E 证据 | — |
| 编辑器 | 中文 IME、粘贴、撤销、刷新恢复通过 | **部分** | AutoSizeTextarea 实现存在；仅浏览器冒烟覆盖「删一字符刷新仍在」 | browser-dev-fallback |
| 编辑器 | 用户改一个字符只更新对应 DS 节点 | **部分** | `EditorCommandV1` + 事务编辑（command 级） | command |
| 编辑器 | 迟到 cloud 不覆盖 user_edited 节点 | **不满足** | 无 reconciliation/user_edited 保护实现 | — |
| 编辑器 | author/student parity 100% | **不满足** | 无 parity 测试 | — |
| UI | 1100×760、Windows 125%/150% 无横向溢出 | **部分** | `layout-matrix.mjs` 报告 0 溢出，但为浏览器 CDP | browser-dev-fallback |
| UI | 长文本、长文件名、宽表格有可用策略 | **部分** | `fixtures/ui/long-content.json` + 溢出矩阵 | browser-dev-fallback |
| UI | 普通 UI 不展示技术 hash/schema/manifest | **无法验证** | 无截图/断言证据 | — |
| 测试 | Real Tauri import/edit/publish E2E 通过 | **不满足** | 最新 `artifacts/e2e-tauri/run-2026-09-05T11-34-16-269Z/report.json` **verdict=failed**（title-persists、publish 两步失败），且早于 HEAD | product（失败） |
| 测试 | 100-PDF corpus 报告通过正式门槛 | **不满足** | 无报告；`privateCorpusReady=false` | — |
| 测试 | 50 文件 batch + restart 通过 | **不满足** | 无执行 | — |
| 测试 | NAS student loader 读取发布包通过 | **不满足** | 仅跨仓 contract 脚本，无 loader 运行时刻 | cli |
| 测试 | 旧题迁移幂等、可回滚 | **部分** | `library` 迁移测试 2 个（command 级）；无回滚证据 | command |

**统计：30 项中「满足」= 0 项；「部分」= 10 项；「不满足」= 15 项；「无法验证」= 5 项。** 按 AGENTS.md 证据分层，无任何一项达到 `product` 级满足。

## 发现清单

### A11-F01 [P0] 真实 Tauri E2E 最新报告为 failed，且早于当前 HEAD——产品级验收从未成立

- 结论：`§19.7` / `§18 Phase 0 P0-T02` / `§24 测试类第 1 项` 声称的「Real Tauri import/edit/publish E2E 通过」不成立。
- 证据：`artifacts/e2e-tauri/run-2026-09-05T11-34-16-269Z/report.json`：`verdict = "failed"`，`coverage = "real-tauri-process+webview2+sqlite+filesystem"`，9 步中 `title-persists-in-library`、`publish-via-workspace-button` **failed**。该报告时间 2026-09-05，早于 M1 提交 `47a3806`，而工作树还有 43 项未提交改动。`repair-2026-09-07/progress.md:63` 亦自认「真实 Tauri … 未在本环境执行，不计为验收通过」。
- 影响：DoD「测试」类 5 项全部无法满足；Phase 0 出口「能用自动测试证明产品链」未达成。
- 建议：在冻结 SHA 上重跑 `npm run e2e:tauri` 并要求 verdict=passed，报告须绑定当前 commitSha 与 schemaHash。

### A11-F02 [P0] 第 18 章文件清单与实际产物不符：P0-T02 要求 3 个真实 Tauri 脚本，实际只有 1 个

- 结论：`§18 P0-T02` 列出的 `scripts/e2e/tauri-library-import.mjs`、`tauri-workspace-edit.mjs`、`tauri-publish.mjs` **三个文件均不存在**；实际只有 `scripts/e2e/tauri-import-edit-publish.mjs`。
- 证据：`ls scripts/e2e/` 仅 5 个文件；Grep `tauri-library-import|tauri-workspace-edit|tauri-publish` 仅命中计划文档与 backend-map。
- 影响：P0-T02 出口条件「导入→打开→改一个字符→保存→发布」仅由单脚本承载，且该脚本最新 verdict=failed；计划与仓库的事实一致性缺失。
- 建议：要么按计划补三脚本，要么修正计划文件清单并说明单脚本等价性。

### A11-F03 [P0] P0-T01「8 份 Reading PDF fixture」在本地缺失——语料不可校验

- 结论：`fixtures/golden/manifest.json` 声明 8 份 `requiredPrivateCorpus` 为 `available`，但源 PDF 不在工作树。
- 证据：`fixtures/golden/private-real/` 只有 `.gitignore` 与 `README.md`，**无任何 `.pdf`**；`fixtures/product-baseline.json` 中 `corpus.privateCorpusReady = false`。8 份 metadata（chili-peppers 等）存在但对应源文件缺失，README 自述「若来源缺失必须保持 missing-source」。
- 影响：Phase 4 的「Golden corpus Ready 指标」「100-PDF corpus」等验收前置语料在本环境不可用；以 metadata 冒充「fixture 已就位」属 over-claim。
- 建议：明确区分「metadata 已标注」与「源文件可用」，并在验收报告中标注语料缺失。

### A11-F04 [P1] Golden Corpus 标注格式与 §19.2 不一致，10 项指标无计算实现

- 结论：`§19.2` 要求的标注结构（`range`/`taskType`/`questions`/`options`/`prompt`/`unassignedAllowed`）与 10 项指标计算均未落地。
- 证据：39 份 metadata 中 `range`=0、`taskType`=0、`questions`=0、`options`=0、`prompt`=0、`unassignedAllowed`=0 份；实际字段为 `displayRange`、`kind`、`slotIds`（见 `fixtures/golden/metadata/pdf-two-column.json:20-35`）。`fixtures/golden/metrics.json` 仅被 `scripts/phase0-golden.mjs` 做 **schema 校验 + 计数**（`:713-740`），全仓无任何指标数值计算代码。A5 报告线索经独立验证成立。
- 影响：`§19.2` 的 Prompt/Non-empty/Option Text 等指标因缺少标注**在原理上无法计算**；`§18 Phase 4` 的验收指标（option label recall ≥99.5% 等）不可度量。
- 建议：补齐标注 schema 与指标计算器，或下调计划指标并声明依赖。

### A11-F05 [P1] 云端/合并链完全缺失，`§19.3` 17 例与 `§19.4` 9 例覆盖率为 0

- 结论：`CloudRecognitionCandidateV1`、versioned skill bundle、repair/salvage、Reconciliation Engine 在代码中均不存在。
- 证据：Grep `CloudRecognitionCandidateV1` 仅命中计划与 backend-map；全仓无 `*reconcil*` 文件；`llm_gateway.rs` 仅 `validate_cloud_outline_output`（`CloudReadingOutlineV1`）。task_plan M5 自认 pending。
- 影响：DoD「云端」5 项、「编辑器」迟到 cloud 保护 1 项全部不满足；`§19.3/§19.4` 26 个用例 0 覆盖。
- 建议：按 P5/P6 推进，或从本轮 DoD 中移除并显式降级为后续里程碑。

### A11-F06 [P1] 无前端测试基础设施，`§19.5` 14 项与 `§19.6` parity 无独立实现

- 结论：仓库没有任何前端单元测试框架。
- 证据：`package.json` devDependencies 无 vitest/jest；`src/**` 无 `*.test.*`/`*.spec.*`；无 `vitest.config.*`/`jest.config.*`。`workspace-save-regressions.mjs` 头部自述 `coverage = "current-react-ui-with-controlled-ipc-adapter"`，`editor-repro.json` 亦标注 `current-react-ui-with-controlled-ipc-adapter`——是**替身 IPC 的浏览器测试，不是 Rust/SQLite/NAS**。
- 影响：`§19.5` 编辑器 14 项、`§19.6` parity 无实现；DoD「author/student parity 100%」不满足。
- 建议：引入前端测试框架；parity 需真实 render 两模式并比对 DOM/几何。

### A11-F07 [P1] 以 CLI/schema/browser-dev 证据冒充产品验收（over-claim 模式）

- 结论：多处把低层级证据表述为产品级完成。
- 证据：
  - `task_plan.md:27` M0 标 `complete`，但其「真实 Tauri E2E」最新 verdict 为 failed 且早于 HEAD（A11-F01）。
  - `repair-2026-09-07/progress.md:26-27` 把 `npm run e2e:library-workspace`（**浏览器 + devFallback**）与 `workspace-save-regressions.mjs`（**受控 IPC 替身**）列为验证，与同文档 `:63` 「真实 Tauri … 不计为验收通过」自相矛盾。
  - `§19.1` 层 5 Visual regression 仅有浏览器 `layout-matrix.mjs`。
- 影响：AGENTS.md 明令「CLI/schema 检查不得替代产品工作流验证」；当前追踪文档违反该分层纪律。
- 建议：所有状态表增加「证据层级」列，`product` 级验收必须由真实 Tauri 报告且绑定当前 SHA。

### A11-F08 [P1] Phase 8 的删除项几乎全未执行，V1 主链仍在写

- 结论：`§18 Phase 8`（P8-T01…T04）无实质进展。
- 证据：`src/pages/` 仍含 Dashboard/DocumentReview/ExportPage/ImportWizard/JobList/LibraryExamDetail/StructuredAuthoringEditorV2/UnifiedPreview 等 11 个文件；`legacyRoutes.tsx` 仍在（虽未被 import）；`devFallbackBackend.ts` 仍在（仅加 `import.meta.env.DEV` 守卫）；`auto_pipeline.rs:187/1692/2449` 仍走 `make_dynamic_split_candidates`→`build_authoring_v2_shadow`；`lib.rs` 11151 行、`authoring_pipeline.rs` 13848 行**未拆分**。
- 影响：DoD「新题 direct DocumentIRV2，不经 V1 authoring 主链」不满足；「主产品只有三个表面」不成立。
- 建议：按 P8 顺序执行，删除需与 P10 计划一致地整组移除。

### A11-F09 [P2] 成本估算与实际投入严重不匹配（71–112 工程日 vs 2 个提交）

- 结论：`§18` 的阶段工期估算缺乏依据。
- 证据：`git log --oneline 5daa04f..HEAD` 仅 2 个提交（M0/M1）；未提交改动 `32 files changed, +1082/-678`。而 Phase 0–8 合计 71–112 工程日，其中 Phase 4–8（占 57–92 日）几乎未落盘。
- 影响：以「工程日」为单位的阶段划分会误导排期与验收预期。
- 建议：按可交付物而非人日重排，并标注每个 Phase 的当前证据层级。

### A11-F10 [P2] 部分 Phase 出口条件不可度量

- 结论：`§18` 多个出口条件缺少可执行判据。
- 证据：如 P1「批量选择后题库立即出现行」未定义「立即」的时限；P3「中文输入法、快速输入、撤销/重做通过」无具体 fixture/阈值；P6「50 文件批量时 UI 不冻结」未定义帧率/时限（`§19.9` 有 50–60fps 但无实现）；P7「用户从题库选 20 题一键发布」无脚本。
- 影响：无法客观判定 Phase 是否达标，易产生 over-claim。
- 建议：每个出口条件补充可执行命令 + 阈值 + 证据层级。

### A11-F11 [P2] `§19.9` 7 项性能目标零实现

- 结论：无任何性能度量或基线数据。
- 证据：Grep `bench|p50|p95|fps|performance.now` 在 `src-tauri/src`、`scripts` 仅命中无关字符串；无 benchmark 目录、无性能报告。
- 影响：DoD 与 `§18` 中与性能相关的出口（50 batch、1000 条目搜索）不可判定。
- 建议：建立性能基线脚本并纳入 CI。

### A11-F12 [P3] `product-baseline.json` 的 commitSha 已过期

- 结论：基线快照与当前 HEAD 不一致。
- 证据：`fixtures/product-baseline.json:4` `commitSha = 401ca76…`，而当前 HEAD = `47a3806…`（M1）；`recordedAt = 2026-09-05T11:42:30Z`。工作树另有 43 项未提交改动。
- 影响：`verify-product-baseline.mjs` 的漂移检测对 M1 之后的改动无约束力；「固定基线」名不副实。
- 建议：在冻结 SHA 上重跑 `--update` 并提交。

### A11-F13 [P3] `recognition/local/` 于审计期间新增，0 测试、未接入主链、追踪文档未同步

- 结论：P4-T02 出现半成品，但无证据、无状态更新。
- 证据：`src-tauri/src/recognition/local/{mod.rs,question_blocks.rs}`（共 43 KB）时间戳 2026-09-12 11:37–11:39，`git status` 显示 `?? src-tauri/src/recognition/` 未跟踪；`lib.rs:85` 仅 `mod recognition;`；两文件 `#[test]` 计数均为 0；task_plan M4 仍写「P4-T02~T06 未开始」。
- 影响：审计快照与实现并发变化，存在状态漂移；该模块当前无消费者、无测试，不能作为任何出口条件证据。
- 建议：明确其状态（未完成/实验），补测试与接线后再计入进度。

## 证据层级与局限

- **层级分布**：本组核对到的实现证据集中在 `command`（Rust 命令/服务层）与 `browser-dev-fallback`（CDP + devFallback/受控 IPC）；`cli`/`schema` 用于基线与契约；**`product` 级证据唯一来源是 `artifacts/e2e-tauri/`，而其最新报告 verdict=failed 且早于当前 HEAD**。
- **禁止的推断**：`scripts/e2e/library-workspace-smoke.mjs`（浏览器 + devFallback）、`scripts/e2e/workspace-save-regressions.mjs`（受控 IPC 替身）、`scripts/ui/layout-matrix.mjs`（浏览器 CDP）**均不得作为真实 Tauri/SQLite/文件系统/NAS 的验收证据**。
- **历史报告的时间戳与 commit**：`artifacts/e2e-tauri/run-2026-09-05T11-34-16-269Z` 记录于 2026-09-05，早于 M1 提交 `47a3806`；`artifacts/workspace-save-regressions/editor-repro.json` 记录于 2026-09-12 00:06，但 `coverage` 明确为受控 IPC 替身。两者都不能证明当前工作树的产品行为。
- **未执行项（本环境约束）**：未运行 `cargo build/test`、`npm run build`、任何 e2e 脚本；未访问真实 NAS 学生端；因此 Rust 测试「567 passed」等结论仅采信 `repair-2026-09-07/progress.md` 的自述，未独立复核。
- **并发变化**：`src-tauri/src/recognition/` 在审计进行期间落盘（11:37–11:39），本报告以其在读取时刻的状态为准；该目录为未跟踪文件，未被任何提交固定。
- **判定总览**：9 个 Phase 中 `partial` 6 个（P0/P1/P2/P3/P6/P7，其中 P3 兼 over-claimed）、`not started` 3 个（P4 仅 T01 增量、P5、P8）、`complete` 0 个；DoD 30 项中「满足」0 项。
