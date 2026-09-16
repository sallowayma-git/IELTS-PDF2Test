# Dual Recognition 下一阶段执行计划（2026-09-12）

## 1. 总目标

UI 审计修复之后，项目主目标从“继续找样式问题”切换为：

> 在当前修复提交上建立可复现的真实 Tauri 产品证据，先消除导入/取消/恢复/迟到云结果的数据风险，再完成 M4 direct canonical 主链、M5 cloud candidate/reconciliation 与真实 NAS/student 发布闭环。

最终退出标准对应权威计划 DoD：真实 Tauri import/edit/reopen/publish 通过；新题 direct DocumentIRV2；迟到 cloud 不覆盖用户编辑；真实 NAS student loader 能读取发布包；100-PDF 与 50-file batch 达标。

## 2. 当前事实基线

- 规划时 HEAD：`3dfd6c0`，工作树除本规划目录外无产品代码改动。
- `npm run check` / `npm run build` 已有通过声明，但它们不是产品验收。
- 当前没有新鲜、完整成功的真实 Tauri E2E 报告；最新持久化报告仍是 2026-09-05 的 `verdict=failed`。
- M4 已实现 Question Layout Graph、题型分类、硬闭包、physical table/visual 与 quality blocker 消费，但 V1 authoring 仍是结构权威，识别 gate 默认关闭。
- M5 尚未开始；因此 `user_edited`、取消/迟到结果、canonical proposal-only 护栏必须先完成。
- A4-F01/F02/F03/F05、A3-F04、A7-F02/F03 仍开放；A7-F04 为部分完成。
- F3/F8 是非阻断样式架构债；但返回按钮存在确定性的 CSS 级联风险，需要在当前 UI 修复轮补掉。

## 3. 依赖主线

```text
当前修复 agent 交付并冻结 HEAD
        ↓
G0 真实产品基线 + G1 数据安全护栏
        ↓
G2 M4 量化验收 + direct canonical
        ↓
G3 M5 cloud candidate
        ↓
G4 deterministic reconciliation
        ↓
G5 NAS/student 真实发布闭环
        ↓
G6 100-PDF / 50-file release gate
        ↓
G7 V1 退出与大模块拆分
```

其中，M5 contract/fixture 设计可以与 M4 语料测量并行；任何 cloud 结果写入 canonical 的实现必须等待 G1 完成。

## 4. 推荐执行顺序

### G-1：收口当前修复 agent（立即，阻塞所有后续验收）

目标：获得唯一、可复现的修复基线，避免再次在漂移工作树上审计或跑 E2E。

任务：

- 要求当前修复 agent 返回 commit SHA、改动文件、已跑命令、未跑项和已知风险。
- 确认工作树只包含明确归属的规划文件或已提交代码；不要把并行 agent 的半成品混入基线。
- 对返回按钮做 computed-style 核验：当前 `.workspace-header button` 比 `.workspace-back-button` 特异性更高，会覆盖 `height:40px`、`min-width`、`border-radius:8px` 和 `align-self`。修复后必须断言实际 40×40、8px、可见 focus、点击前 `flush()`。
- 将 F3/F8 记录为独立债务，不扩大成第二轮 UI 改版。

完成定义：

- 新 HEAD 固定；`git status` 可解释。
- 返回按钮功能与 computed style 均通过，而不仅是源码声明存在。
- `npm run check && npm run build && npm test` 通过；产品 baseline 漂移已解释或按流程重录。

证据层级：static + browser/Tauri computed style。仅静态 CSS 阅读不能关闭该项。

### G0：建立当前 HEAD 的真实产品基线（最高优先级，1 个短迭代）

目标：把“脚本存在”变成“当前源码对应的真实应用运行成功”。

任务 ID：

- `G0-T01` 在 Windows 上重建 debug executable；freshness guard 必须为 `staleBuild=false`。
- `G0-T02` 运行 `e2e:tauri:workspace-edit`：打开真实工作区、改正文、flush、重开、内容仍在、返回题库。
- `G0-T03` 运行 `e2e:tauri:publish`：使用可发布 fixture，必须真正发布成功；`blocked` 不是通过。
- `G0-T04` 运行完整 `e2e:tauri`：真实进程 + WebView2 + SQLite + filesystem，覆盖 import → ready/action-required → edit title/body → reopen → library → publish。
- `G0-T05` 报告写入 `artifacts/e2e-tauri/`，记录 HEAD SHA、exe/source mtime、fixture hash、每步状态与截图。
- `G0-T06` 将最小 Windows Tauri smoke 纳入 CI；环境不可用时必须返回 CANNOT-RUN，不得记 passed。

完成定义：三条真实 Tauri driver 均产生当前 HEAD 的新鲜报告；完整链 `verdict=passed`；publish 产生可检查的 NAS 包。任何 `passed-with-publish-gate-blocked` 不计完成。

### G1：数据正确性与云写入前置护栏（与 G0 可并行，最高优先级）

目标：先消灭会留下孤儿、误报状态或覆盖用户编辑的数据风险。

任务 ID：

- `G1-T01` 修 A4-F01：把 job/staged source 与 library item/queue 建立为可恢复协议；queue 失败后不能出现“磁盘有 job、DB 无可见行”的孤儿。可选设计是补偿删除，或保留可见 failed item + retry；必须只选一种并测试。
- `G1-T02` 修 A4-F02：local/cloud 返回后再次检查 durable cancel/lease；迟到结果不得把 cancelled job 推到 ready。
- `G1-T03` 修 A4-F03：启动恢复区分 `requeued` 与 `action_required`，UI 不得对超限任务谎称“已自动排队重试”。
- `G1-T04` 修 A4-F05：reclaim lease 时保留已经成功的 `local_status`，只推进当前需要恢复的 stage。
- `G1-T05` 处理 A3-F04：要么真正把 `keepSourceFiles` 接到 Rust 清理策略与首次确认，要么删除 UI 承诺；禁止保留空操作设置。
- `G1-T06` 修 A7-F02：定义 proposal-only apply protocol；reconcile 读取 `user_edited`，拒绝迟到 cloud 覆盖；保留候选与用户决策审计。
- `G1-T07` 建故障注入矩阵：staging 成功/queue 失败、local running cancel、cloud running cancel、lease expiry、restart 达重试上限、user edit + late cloud。

完成定义：上述 6 个场景都有 Rust command/handler 测试；至少导入失败、运行中取消、重启恢复、迟到结果 4 条通过真实 Tauri UI 验证；canonical 与 revision 不丢失、不被迟到结果覆盖。

不得接受：只改文案、只测 CLI、只检查数据库 schema。

### G2：M4 从“附加证据”升级为本地识别主链（下一里程碑）

目标：新题由 DocumentIRV2 / Question Layout Graph 直接产出 canonical authoring，不再由 V1 authoring 结构主导。

任务 ID：

- `G2-T01` 恢复/提供 8 份私有 Reading PDF，并使 runner 记录 fresh run token、fixture SHA、当前 HEAD。
- `G2-T02` 实现并固定 Phase 4 指标计算：empty prompt、option label recall、statement completeness、matching exact structure、complex visual fallback。
- `G2-T03` 先在 report-only 模式跑 8-PDF，再扩至标注 corpus；逐项审查 false positive。
- `G2-T04` 让 QLG/task groups/table stimuli 直接构建 IeltsAuthoringIRV2，停止 `make_dynamic_split_candidates → V1 authoring → V2 shadow` 作为新题权威路径。
- `G2-T05` 只有在指标达标后才默认开启 recognition blocker gate；保留明确 rollback flag。
- `G2-T06` 补 PDF/DOCX 真实产品路径回归，确保 editable draft、issue target、source overlay 和发布门消费同一判断。

完成定义：

- Ready simple choice `empty prompt = 0`。
- option label recall ≥99.5%，statement completeness ≥99%，matching exact structure ≥98%，complex visual fallback =100%。
- 8-PDF strict 报告 8/8 新鲜；新题 direct DocumentIRV2，不经 V1 authoring 主链。
- gate 默认启用后真实 Tauri import/edit 仍通过。

### G3：M5A 云端完整候选与容错链

依赖：G1 全部完成；G2 canonical schema 稳定。可以提前设计 fixture，但不能提前写 canonical。

任务 ID：

- `G3-T01` 建 `recognition/skills/ielts-reading-v1` versioned skill bundle 和 8 类示例。
- `G3-T02` 同步 Rust / TS / JSON Schema 的 `CloudRecognitionCandidateV1`，绑定 source/skill/schema version。
- `G3-T03` 实现 PDF direct 与一次性分页图 fallback；补页覆盖与截断检测。
- `G3-T04` 单一 JSON extract → normalize → schema validate → semantic closure。
- `G3-T05` 最多一次 constrained repair；按 group salvage；返回 typed outcome。
- `G3-T06` 覆盖 malformed JSON、code fence、前后解释、字段别名、缺字段、超时、429、provider error 等至少 17 个 fixture。

完成定义：valid group 可独立持久化为候选；invalid group 不污染 canonical；原始 parse error 不进入普通 UI；输出不含 HTML/JS；失败不阻止打开本地稿。

### G4：M5B 确定性 Reconciliation 与 ActionableIssue 统一

依赖：G3；G1-T06。

任务 ID：

- `G4-T01` local/cloud/source 三方对齐，稳定 ID allocator 与 evidence resolver。
- `G4-T02` 只对本地空字段且证据成立的内容自动补全；非空冲突一律 proposal-only。
- `G4-T03` answer conflict、option bank、shared prompt、范围不明均成为 typed blocker。
- `G4-T04` 第二次校对只在 deterministic diff 存在时运行，只能产 proposal。
- `G4-T05` 统一后端 ActionableIssue CRUD、命名、传输和前端渲染，停止前后端各自派生不同事实。
- `G4-T06` 后端改用 typed error；普通 UI 只见用户文案，internal detail 进入日志/开发模式。

完成定义：计划规定的 9-case reconciliation matrix 全绿；真实 Tauri cloud-on disagreement 能在 Canvas 原位显示 diff/issue；接受产生 revision，拒绝不改 canonical；迟到 cloud 永不覆盖 `user_edited`。

### G5：发布、NAS 与学生端真实闭环

依赖：G2/G4 的 canonical 与 quality 稳定。

任务 ID：

- `G5-T01` 为 batch publish 加 durable journal 与 crash recovery，覆盖 release rename 后、manifest replace 前崩溃。
- `G5-T02` 返回每题/节点 typed failure，并支持“仅发布通过项”的显式二次动作。
- `G5-T03` 实现 release/asset 引用集合 GC，不能只保证 manifest 原子而无限堆垃圾。
- `G5-T04` 生成当前 HEAD 的 NAS contract 报告；脚本成功必须写报告和 fixture hash。
- `G5-T05` 在真实 NAS Electron/student loader 中加载至少一整套题，完成作答、判分、资源/热点交互。
- `G5-T06` 做 author/student semantic parity 与关键视口 screenshot parity。

完成定义：20 题批量发布成功；故障注入后可恢复；真实学生端能加载和作答；contract 检查与真实 loader 两层均通过。

### G6：发行级真实语料门

依赖：G5。

- `G6-T01` 100-PDF strict 报告，按题型/扫描/表格/图示分类，0 CLI invocation failure；该门放私有 runner/nightly，不要求普通 PR 消耗真实 LLM。
- `G6-T02` 50-file batch + restart，覆盖取消、断电式重启、低磁盘、网络失败与恢复。
- `G6-T03` Windows NSIS 安装后重跑最小真实 Tauri + student loader smoke。
- `G6-T04` 冻结 release evidence bundle：commit、installer hash、fixture manifest、Tauri/NAS/100-PDF/50-batch 报告。

完成定义：权威计划 §24 测试 DoD 全部有新鲜产品证据，环境缺失不得写“通过”。

### G7：旧链退出与结构治理（最后）

依赖：G6 通过并观察至少一个发行周期。

- 新题停止 job.json/legacy exams/V1 authoring 双写；旧 V1 仅保留幂等、可回滚的读取迁移 adapter。
- 删除死 flag、dev fallback、旧预览/发布路径。
- 拆分 `auto_pipeline.rs`、`authoring_pipeline.rs`、`lib.rs`，按识别/合并/编辑/发布边界组织。
- 最后治理 F3/F8：建立共享 Button primitive/variant，删除 legacy class 分叉；补 focus/disabled/hit-target/a11y 回归。

完成定义：新题全程单一 canonical；旧题迁移可回滚；产品门全绿；删除旧链后不靠重录 golden 掩盖回归。

## 5. 两个迭代的建议切片

### 迭代 A：可信基线与数据安全

纳入：G-1、G0、G1。

不纳入：M5、泛 UI 优化、大文件拆分。

迭代目标：取得当前 HEAD 的真实 Tauri 成功报告；消除已知孤儿/取消/恢复/迟到覆盖风险。

### 迭代 B：M4 正式完成

纳入：G2；可并行准备 G3 的 schema/fixture，不允许 canonical apply。

迭代目标：8-PDF/标注指标达标，QLG 直接产 canonical，recognition gate 可以默认启用。

## 6. 当前 agent 之间的边界

- 正在工作的修复 agent：只负责其已领取的问题、返回按钮级联回归和已有 UI 修复验证；完成后提交并交接。
- 下一位实现 agent：从 G0/G1 领取，不应再次开始全仓 UI 审计。
- 规划/复审 agent：维护本任务书、核验报告新鲜度与证据层级，不直接把 CLI/schema 绿灯改写成产品完成。
- 若有多 agent 并发：按 `src-tauri/processing`、Tauri E2E/CI、M4 corpus 三个互斥写入面切分；主代理统一集成与产品验证。

## 7. 下一个 agent 的推荐任务描述

> 基于当前修复 agent 的最终 commit，先完成 G0 + G1，不推进 M5。重建 Windows debug Tauri executable，运行 workspace-edit、publish、完整 import-edit-publish，并保存带 HEAD/exe/source freshness 的报告。同时修复 A4-F01/F02/F03/F05、A3-F04 和 A7-F02，补故障注入与真实 Tauri 回归。任何 blocked/CANNOT-RUN/CLI-only 结果不得记为产品通过。不要重录 golden 来掩盖行为漂移；若私有 PDF、NAS repo 或 driver 缺失，明确记录环境阻塞与可复现命令。

## 8. 决策门

只有以下门通过，才进入下一层：

| 门 | 进入条件 | 阻止的下一步 |
|---|---|---|
| D0 修复基线 | 固定 SHA、干净/可解释工作树、返回按钮 computed style 正确 | 所有 E2E |
| D1 产品可信 | 当前 SHA 的真实 Tauri workspace/publish/full chain 全绿 | M4 默认开 gate |
| D2 数据安全 | 取消/迟到/user-edited/恢复故障矩阵全绿 | M5 canonical apply |
| D3 本地识别 | Phase 4 指标 + 8-PDF strict 达标、direct canonical | M5 reconciliation |
| D4 云端候选 | 17 fixtures + repair/salvage + typed outcome | 自动/提案合并 |
| D5 合并安全 | 9-case matrix + 真实 Tauri diff/accept/reject | release candidate |
| D6 学生端 | NAS contract + 真实 loader + parity | 100-PDF release gate |
| D7 发行证据 | 100-PDF、50-file restart、installer smoke | V1 删除 |

