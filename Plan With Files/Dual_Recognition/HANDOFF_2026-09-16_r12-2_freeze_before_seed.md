# HANDOFF — 2026-09-16 R12-2：冻结快照早于权威稿播种，无云/有云两条路径都无法产出批次与候选

## 一句话

`processing/scheduler.rs` 在 `set_item_status_ready(...)`（它内部才调 `migrate_single_item`
播种权威稿）**之前**调用 `freeze_local_candidate_snapshot(...)`。冻结要求 `get_canonical_ds`
已存在，因此**首次导入必然**得到 `canonical_not_seeded:<jobId>`；按你们 14:04:47 的新逻辑，
冻结失败即拒绝裁决 → `reconcile_status=failed` → **没有批次** → 四条链全部 `not_run`。

这解释了任务书第 3 项「不能四条链全部 not_run」为何仍未达成，也解释了第 5 项
「候选按钮流程不可达」：有云路径同样在冻结处失败，随后按设计丢弃云端结果、跳过裁决。

## 证据（真实二进制 + 真实 IPC + 真实 SQLite）

二进制：`src-tauri/target/debug/ielts-author-studio.exe`
`sha256 = 5961c9cf6a8a6b5f75b138fd7bcd522a09d497be62c2cdc17509e24727711118`
（由 `node scripts/e2e/build-app.mjs` 产出，清单自陈 `inputsDriftedDuringBuild: false`，
即该 exe 与当前工作树**逐字节对应**。）

无云导入 `fixtures/parser/demanding-reading-passage-3.pdf` 后，`processing_jobs_v2` 的行：

```
stage            = ready_for_review
local_status     = succeeded
cloud_status     = not_run
reconcile_status = failed
actionable_count = 0
last_error_code  = canonical_not_seeded:import-20260916131334-d7ab7039
```

`recognition_batches_v1` / `recognition_decisions_v1` / `recognition_decision_journal_v1`
**均为 0 行**。同一时刻 `get_recognition_decision` 返回：

```
batchId = null
chains  = { local: not_run, cloud: not_run, source: not_run, adjudication: not_run }
```

复现命令：`node scripts/e2e/tauri-cdp-local-chain.mjs`
（结果目录 `artifacts/e2e-cdp/run-local-chain-*/report.json`；
数据库只读诊断：`node --experimental-sqlite scripts/e2e/inspect-run-db.mjs <runDir>`）

## 代码定位

`src-tauri/src/processing/scheduler.rs`：

| 行 | 内容 |
| --- | --- |
| 464 | `if let Err(error) = freeze_local_candidate_snapshot(&root, &job_id, base_edit_version)` ← **太早** |
| 495 | `set_item_status_ready(&app, &job_id).await;` ← 它内部才 `migrate_single_item`（见下） |
| 508 | `if freeze_error.is_some() { ... Some("failed") ... }` ← 冻结失败即拒绝裁决（14:04:47 新增） |
| 529 | `run_local_only_recognition_cycle(...)` ← 因此永远到不了这里 |

`set_item_status_ready`（同文件 981–1012）第 996 行：

```rust
crate::library::migration::migrate_single_item(&root, &job_id)?;
```

`freeze_local_candidate_snapshot`（同文件 791–816）第 798–801 行：

```rust
let (canonical, _current_version) =
    crate::library::repository::get_canonical_ds(&conn, job_id)
        .map_err(|error| format!("read_canonical_failed:{error}"))?
        .ok_or_else(|| format!("canonical_not_seeded:{job_id}"))?;   // ← 这里失败
```

`migrate_single_item`（`library/migration.rs:155`）是**按需播种**：它从
`candidate_authoring(root, job_id)`（本地流水线写出的 artifact）取稿并 `seed_canonical_ds`。
生产路径上只有三个调用点：`get_workspace_item_core`（`library/commands.rs:26`）、
`get_publish_preflight_core`（`authoring_v2_commands.rs:83`）、`set_item_status_ready`
（`scheduler.rs:996`）——**没有一个早于第 464 行的冻结**。

## 最小修法

在 `freeze_local_candidate_snapshot` 之前播种，例如把第 464 行前改为：

```rust
// 权威稿必须先就位：冻结要求 `get_canonical_ds` 存在，而播种此前只发生在
// `set_item_status_ready`（在冻结之后）。不先播种则首次导入必然 canonical_not_seeded。
if let Err(error) = crate::library::migration::migrate_single_item(&root, &job_id) {
    eprintln!("[processing] seed canonical before freeze failed for {job_id}: {error}");
}
```

（`migrate_single_item` 幂等且不覆盖用户已编辑的稿，重复调用安全；也可选择把
`set_item_status_ready` 整体提到冻结之前，但那样会把「发布可编辑」也提前，改变
「冻结早于可编辑」的既有承诺，故不推荐。）

修完后应当观察到：无云导入的 `batchId` 非空、`local/source/adjudication` 三链有终态、
`cloud` 为 `not_run` 且带原因码；有云路径则应能走到裁决并产出真实候选。

## 一个附带提醒（同一处顺序问题的另一半）

有云路径（第 543–546 行）在 `freeze_error.is_some()` 时**丢弃云端结果并跳过裁决**。
在这条顺序修好之前，即使接上可用的云端 profile 也不会产生任何候选——所以第 5 项
「候选接受/拒绝/撤销的真实按钮验收」在修复前不可能通过，不必再去排查前端按钮。

## 前端侧本轮已做、与本缺陷无关的改动（均在 `scripts/**`，不碰后端文件）

- `scripts/e2e/lib/tauri-cdp-harness.mjs`：`assertBuildFresh` 改为**内容哈希清单优先**
  （`artifacts/build-manifests/<exeSha>.json`），mtime 三段链兜底；新增 `buildFreshReport`。
- `scripts/e2e/build-app.mjs`：可归因构建（前端 → dist → exe + 清单，构建期间输入漂移则退出 4）。
- `scripts/e2e/lib/build-manifest.mjs`：清单实现。
- `scripts/e2e/lib/build-freshness.test.mjs`：14 条回归（含「旧 dist 被新 exe 包入」）。
- `scripts/e2e/tauri-cdp-local-chain.mjs`：无云四链验收（当前**正确地失败**并报出本缺陷）。
- `scripts/e2e/tauri-cdp-freeze-order-proof.mjs`：本缺陷的因果实验（导入无批次 → 打开工作区播种 → 重试后有批次）。
- `scripts/e2e/lib/fake-llm-gateway.mjs`：隔离 localhost 协议测试服务（任务书第 4 项）。
- `scripts/e2e/inspect-run-db.mjs`：运行目录数据库只读诊断。

---

## 追加验证（R13，2026-09-16 晚）：**确定性**复现，`ee36033` 未触及本顺序问题

上一轮曾出现「同一 exe、同样步骤、一次无批次、一次有批次」的观测矛盾，本轮已收敛。

### 1. 「竞态」假设已排除 —— 那次「有批次」是自己播种造成的

`tauri-cdp-freeze-order-proof.mjs` 的**步骤 B 刻意调用 `get_workspace_item`**，
而它会走 `migrate_single_item` 播种权威稿；步骤 C 再 `retry_processing`，冻结自然成功。
所以「有批次」是**被实验设计引入**的结果，不是同一路径下的随机性。
也就是说：**播种 → 有批次；不播种 → 无批次**，这是确定性的因果，不是竞态。

### 2. 用新二进制（含 `ee36033` / `99f769c`）重测，仍然无批次

二进制：`sha256 = ad934efa59236f2ffe3fd11e6fe28465bf47b076ed00e7dddb4783c6a5868aa7`
（`build-app.mjs` 产出，`frontendInputs 6f104c05… / dist 69ca780e… / backendInputs c02a5a59…`，
构建期间无输入漂移。）

无云导入 `demanding-reading-passage-3.pdf` 后，`processing_jobs_v2`：

```
stage            = ready_for_review
local_status     = succeeded
cloud_status     = not_run
reconcile_status = failed
actionable_count = 0
last_error_code  = canonical_not_seeded:import-20260916132912-318c0dc6
```

`recognition_batches_v1` / `recognition_decisions_v1` / `recognition_decision_journal_v1` **均为 0 行**。

### 3. `ee36033` 修的是**同一处的另一个问题**，不是本顺序问题

`ee36033`（14:10:59，「refuse auto-apply without a frozen baseline」）在冻结失败时
**新增了拒绝自动应用的守卫**，并让 `current_edit_version_of` 的读取失败不再被
`unwrap_or(0)` 折成版本 0。这是**加固**（宁可不产出候选，也不产出不可信候选），
方向正确；但它**没有**把播种挪到冻结之前，所以 `canonical_not_seeded` 依旧发生。
注意 `last_error_code` 仍是 `canonical_not_seeded:*` 而**不是** `FREEZE_SNAPSHOT_FAILED` ——
说明走的还是原路径。

### 4. 新发现的第二个症状：本地链**明明跑成功了，视图却报 `not_run`**

同一时刻 `get_recognition_decision` 返回
`chains = { local: not_run, cloud: not_run, source: not_run, adjudication: not_run }`，
而数据库里 `local_status = succeeded`。

原因：`chains` 取自**批次行**（`reconcile/commands.rs:226-229`），
`load_latest_batch` 为 `None` 时直接返回四条 `not_run`（同文件 241-259）。
所以只要 reconcile 没产出批次，**本地识别实际成功的状态就完全不可见**。

这使任务书第 3 项的两个要求同时不成立，且是**两个不同的问题**：
- 「有 batch」——本顺序问题；
- 「本地核验状态准确」——批次缺失时链状态**无处可读**，视图只能用 `not_run` 兜底，
  把一个成功的本地识别报成「从未运行」。这一条即使顺序修好也值得复核：
  建议 `load_latest_batch` 为 `None` 时，本地链状态改为从
  `processing_jobs_v2.local_status` 投影，而不是一律 `not_run`。

---

## 追加更正（R13，同一晚）：这是**竞态**，不是「确定性失败」——我上一节的说法需要修正

上一节我写「『竞态』假设已排除」。**这个结论下错了**，必须更正，因为它直接影响这个缺陷的
严重性与复现方式。以下是决定性证据。

### 反例：同一份代码、同一条导入路径，**有批次**

`tauri-cdp-controlled-service.mjs` 第 4 步会轮询 `get_workspace_item`（它内部会
`migrate_single_item` 播种），随后才等云端链。结果：

```
batchId = rec-import-20260916135224-9c373987-v1-f13bd65cb5f5
chains  = { local: succeeded, cloud: succeeded, adjudication: succeeded, source: not_run(EVIDENCE_MISSING) }
候选 29 条，受控服务收到 POST /v1/chat/completions（parts=[text,file]，content=640B）
```

`usedSeedRetryWorkaround` **未被触发** —— 批次是**第一次**就出现的，没有重试。

### 差别只在「谁先到」：播种 vs 冻结

| 脚本 | 导入后是否调 `get_workspace_item` | 结果 |
| --- | --- | --- |
| `tauri-cdp-local-chain.mjs` | **否**（`waitForDraft` 在第 313 行，位于 `waitForDecision` 之后） | 无批次 |
| `tauri-cdp-controlled-service.mjs` | **是**（第 4 步立刻轮询） | 有批次 |

本地识别要跑约 40–50 秒，而冻结发生在其**之后**。第 4 步的轮询在导入后 2 秒内就开始，
于是播种**先于**冻结完成 → 冻结成功。
`local-chain` 脚本从不提前播种，所以冻结必然失败。

### 因此准确的表述是

- **因果规则**是确定的：**播种在前 → 冻结成功；播种在后（或没有播种）→ `canonical_not_seeded`**。
  这一点上一节没说错。
- 但**产品结果是竞态**：生产代码里没有任何一处「在冻结前主动播种」，
  播种只是**用户打开工作区**（`get_workspace_item_core`）的副作用。
  于是导入之后，「用户点多快」与「本地识别跑多快」在赛跑：

  - 用户在本地识别结束前点开该行 → 有候选；
  - 没点（或点晚了）→ `reconcile_status=failed`、无批次、四条链全 `not_run`，
    用户必须**手动重试**才可能拿到候选。

- 这比「稳定失败」更糟：它**间歇**，且失败时的界面表现（四条链全 `not_run`）
  与「什么都没跑」无法区分，用户不会知道该重试。

### 修法不变，但优先级应提高

最小修法仍是在 `freeze_local_candidate_snapshot` 之前主动播种
（`migrate_single_item` 幂等、不覆盖用户已编辑的稿）。修好之后
「导入即可产出候选」才不依赖用户手速；`local-chain` 验收也应随之从 failed 转 passed。
