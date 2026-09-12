# A4 对抗审计：第 5 章（后端目标架构 / 持久化任务调度）与第 12 章（批量导入与实时状态）

审计日期：2026-09-12。审计者：对抗审计子代理 A4（只读）。
审计对象：`Plan With Files/IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md` §5（466–687 行）、§12（2127–2231 行）。
代码基线：HEAD `47a3806`（计划冻结 `bb978be` 已落后）；`src-tauri/src/processing/` 为**未提交**新模块。
方法：Read/Grep/Glob/只读 git；**未运行** `cargo build`/`cargo test`/`npm run build`；未修改任何产品代码。

---

## 章节范围

- **§5 后端目标架构：持久化任务调度**（466–687 行）
  - §5.1 新模块布局（468–528）
  - §5.2 任务状态模型（530–570）
  - §5.3 Tauri 命令收敛（572–597）
  - §5.4 后端事件（599–614）
  - §5.5 并发调度伪代码（616–664）
  - §5.6 为什么不能继续让 `UnifiedPreview.tsx` 管云端队列（666–686）
- **§12 批量导入和实时状态**（2127–2231 行）
  - §12.1 Import Drawer（2129–2150）
  - §12.2 批量创建必须先落库（2152–2180）
  - §12.3 进度不是虚假百分比（2182–2197）
  - §12.4 任务取消与重试（2199–2204）
  - §12.5 应用重启恢复（2206–2219）
  - §12.6 前端订阅（2221–2230）

对抗立场：默认每条论断错误/过时/未完成，用当前代码证伪。修复轮（`repair-2026-09-07/progress.md`）声称修了 F07/F08/F09 —— 本轮**回到代码逐条验证**：F07/F08/F09 确已修复，但修复引入/遗留了新的并发与状态机缺陷（见 A4-F01…A4-F06）。

---

## 断言核对表

证据层级：`product`（真实 Tauri 产品路径）/`command`（命令处理层）/`static`（静态代码/SQL 阅读）/`doc-only`（仅文档声明，代码无对应物）。

### §5.1 新模块布局

| # | 断言 | 判定 | 证据（绝对路径:行号） | 层级 |
|---|---|---|---|---|
| 1 | `processing/{mod,queue,scheduler,worker,state,events,recovery}.rs` 全部存在 | **PARTIAL** | 实际仅 `mod.rs`/`queue.rs`/`scheduler.rs`/`commands.rs`；`worker.rs`/`state.rs`/`events.rs`/`recovery.rs` 不存在（功能被折进 `scheduler.rs`/`queue.rs`）`F:\workspace\PDF2Test\src-tauri\src\processing\`（ls 实测） | static |
| 2 | `recognition/{mod,candidate,local/*,cloud/*}` 存在 | **FALSE** | 目录不存在：`ls: cannot access 'src-tauri/src/recognition/'` | static |
| 3 | `reconcile/{mod,alignment,field_compare,merge,report}` 存在 | **FALSE** | 目录不存在 | static |
| 4 | `library/{mod,repository,migration,assets,cleanup}.rs` 存在 | **PARTIAL** | 有 `mod.rs`/`repository.rs`/`migration.rs`/`schema.rs`/`commands.rs`；**无 `assets.rs`/`cleanup.rs`**（`cleanup` 是顶层 `src\cleanup.rs`，非 `library/cleanup.rs`） | static |
| 5 | `editor/{mod,commands,apply,validation}` 存在 | **FALSE** | 目录不存在 | static |
| 6 | `publish/{mod,compile,preflight,nas}` 存在 | **FALSE** | 目录不存在（发布仍在顶层 `nas_package_v2.rs`） | static |
| 7 | §17.1 目标 `mod recognition; mod editor; mod publish;` 与 §5.1 一致 | **FALSE（同为纸面）** | §17.1（2748–2754）与 §5.1 一致，但两者都不匹配代码：`lib.rs` 无这些 `mod`；`lib.rs` 仍 455 KB 巨文件 | doc-only |

### §5.2 任务状态模型

| # | 断言 | 判定 | 证据 | 层级 |
|---|---|---|---|---|
| 8 | `enum StageStatus{NotStarted,Queued,Running,Succeeded,ActionRequired,Failed,Cancelled,Interrupted}` | **FALSE** | 全仓 grep `StageStatus` 零命中（仅 `schema/listening_runtime_v1.rs` 有无关同名变体）；`ActionRequired`/`Interrupted` 未持久化 | static |
| 9 | `struct ProcessingItemV1{...}` | **FALSE** | 全仓 grep `ProcessingItemV1` 零命中 | static |
| 10 | `enum ProcessingStage{Queued,PreparingSource,LocalRecognition,CloudRecognition,Reconciling,ReadyForReview,ReadyToPublish,Failed}` | **FALSE** | 全仓无 `ProcessingStage`；实际只有 `queue.rs:14-20` 的 7 个字符串常量（`queued/running/local_recognition/cloud_recognition/ready_for_review/failed/cancelled`），缺 `preparing_source`/`ready_to_publish`/`reconciling` 常量，且 `StageResult`/`ProcessingProgress` 也不存在 | static |
| 11 | 实际权威类型 | **（事实）** | `ProcessingJobRow`（`queue.rs:26-41`）：`id/library_item_id/source_asset_id/stage/local_status/cloud_status/reconcile_status/progress/last_error_code/retry_count/lease_owner/lease_expires_at/event_seq`；`progress` 是裸 `Value` 而非 `ProcessingProgress` | static |

### §5.3 Tauri 命令收敛

注册表：`lib.rs:1528-1537`（`invoke_handler`）。

| # | 计划命令 | 判定 | 证据 | 层级 |
|---|---|---|---|---|
| 12 | `import_files(ImportFilesInput) -> Vec<LibraryItemSummaryV2>` | **PARTIAL** | 已注册 `lib.rs:976,1533`；但返回 `{created:[{itemId,title}],rejected:[...]}`（`commands.rs:52-57`），非 `Vec<LibraryItemSummaryV2>` | command |
| 13 | `list_library_items(LibraryFilterV2)` | **PARTIAL** | 已注册 `lib.rs:947,1532`；签名是 `include_deleted: Option<bool>`，非 `LibraryFilterV2`；但确实附带 `processing` 状态（`library\commands.rs:82-93`） | command |
| 14 | `get_workspace_item(item_id: String)` | **HOLDS** | `lib.rs:925,1530`；返回 `WorkspaceItemV1` JSON（`library\commands.rs:44-59`） | command |
| 15 | `apply_editor_commands(ApplyEditorCommandsInput)` | **HOLDS** | `lib.rs:935,1531` | command |
| 16 | `retry_processing(item_id, stage: Option<ProcessingStage>)` | **PARTIAL** | `lib.rs:994,1537` 只有 `item_id`，**无 `stage` 参数**（计划 §12.4「只重跑失败阶段」因此无接口支撑） | command |
| 17 | `publish_library_items(PublishLibraryItemsInput)` | **FALSE** | 无此命令；实际注册的是 `publish_items`（`lib.rs:969,1534`），名称/入参均不同 | static |
| 18 | `save_app_settings(AppSettingsV2)` | **FALSE** | 全仓无该 Tauri 命令；`appSettings.ts:5-7` 自认「后端目前没有对应的设置命令」，设置只落 localStorage | static |
| 18b | §17.1 另列 `get_app_settings` | **FALSE** | `grep get_app_settings src-tauri/src/` 零命中；§5.3（7 条）与 §17.1（8 条含 `get_app_settings`）本身不一致 | doc-only |
| 19 | 新 UI 不再直接调用 V1 `split`/`document review`/`generate preview assets`/`apply LLM suggestion` | **PARTIAL** | 新 UI（`features/library`、`features/editor`）确无调用；但 `run_rule_split`/`generate_preview_assets`/`apply_llm_suggestion` 仍**注册且仍被调用**，调用点全在**已孤立**的 legacy 页面：`src\pages\DocumentReview.tsx:2,40`、`src\pages\UnifiedPreview.tsx:3,14,309,827`（经 `src\app\legacyRoutes.tsx:11,13,17` 引用，而 `App.tsx` 未 import `legacyRoutes`） | static |

### §5.4 后端事件

| # | 断言 | 判定 | 证据 | 层级 |
|---|---|---|---|---|
| 20 | emit `processing://item-updated` | **HOLDS** | `scheduler.rs:27`（常量）、`scheduler.rs:108-124`（emit） | static |
| 21 | 事件字段 `library_item_id/stage/local_status/cloud_status/reconcile_status/progress_percent/actionable_count/display_message` | **PARTIAL** | 实际为 camelCase 且多两个字段：`libraryItemId/jobId/stage/localStatus/cloudStatus/reconcileStatus/progressPercent/actionableCount/displayMessage/stateVersion`（`scheduler.rs:109-120`） | static |
| 22 | 前端只订阅这一种事件，不在页面里自维护队列 | **PARTIAL** | 活跃前端确只订阅：`processingClient.ts:19-28` → `libraryStore.ts:103-111`（`subscribeProcessing(refresh)`），**无 2 秒轮询**（`grep setInterval` 在 `features/library` 零命中）。但 `UnifiedPreview.tsx` 的旧队列代码**未删除**（见 #27） | static |
| 23 | 前端 `libraryStore.patch(payload.libraryItemId, payload)` | **PARTIAL** | 实际是「事件→丢弃旧 `stateVersion`→触发整表 `refresh()` 重拉」（`processingClient.ts:22-27`、`libraryStore.ts:106`），不是 `patch` 局部更新；与 §12.6 伪代码语义不符但结果一致 | static |

### §5.5 并发调度伪代码

实现点：`scheduler.rs:239-380`。

| # | 断言 | 判定 | 证据 | 层级 |
|---|---|---|---|---|
| 24 | CPU 密集 PDF 解析在 `spawn_blocking` 中 | **HOLDS** | 本地链 `run_auto_pipeline_core` 包在 `spawn_blocking`：`scheduler.rs:267-283`；云端同样 `scheduler.rs:334-339` | static |
| 25 | local/cloud 独立 semaphore | **PARTIAL** | 两个 semaphore 存在（`scheduler.rs:57-58`），但**不并发**：本地 permit 在 `scheduler.rs:284` 显式 `drop` 后才进云端；`cloud_permit` 于 `scheduler.rs:317` 获取。二者从不同时持有 | static |
| 26 | `tokio::join!(local_future, cloud_future)` 两链并发 | **FALSE** | 实现是**串行 await**：先本地（`scheduler.rs:267-292`），成功后才云端（`scheduler.rs:311-360`）；全仓无 `tokio::join!` | static |
| 27 | 「云端失败不能取消本地链」 | **HOLDS（结果成立，原因不同）** | 云端在本地之后运行，故云端失败不会取消本地；`cloud_status` 记 `failed` 但仍推进 `ready_for_review`（`scheduler.rs:340-359`） | static |
| 27b | 「一路失败不取消另一路」（本地失败时云端仍作为对照稿跑） | **FALSE** | 本地失败即 `fail_job` 并 `return`（`scheduler.rs:289-292`），云端**永不运行**；计划 `(Err(local),Ok(cloud))=>canonical_from_cloud_with_review` 分支不存在 | static |
| 28 | `persist_candidate` | **FALSE** | 全仓零命中；候选由 `run_auto_pipeline_core`/`run_cloud_review_core` 内部写 artifact，非调度器持久化 | static |
| 29 | `reconcile(local, cloud)` / `canonical_from_local` | **FALSE** | 全仓零命中；本地+云端结果**从不合并**，直接进 `ready_for_review`（`scheduler.rs:344-359`） | static |
| 30 | `complete_job(job_id, readiness)` | **FALSE** | 无此函数；以 `advance_stage(...ready_for_review...)` + `set_item_status_ready` 替代（`scheduler.rs:344-359,487-499`） | static |
| 31 | `cleanup.schedule(job_id, CleanupReason::CandidateMerged)` | **FALSE** | 全仓无 `CleanupReason`/`cleanup.schedule`；`cleanup.rs` 顶层有 `cleanup_transient_job_artifacts` 但调度器未调用 | static |
| 32 | `ctx.job_semaphore.acquire()` | **FALSE** | 无 `job_semaphore`；改用 `local_permits.available_permits()==0` 的 TOCTOU 门控（`scheduler.rs:143-145`），permit 在 spawn 后异步获取（`scheduler.rs:256`）→ 可能超发认领（见 A4-F06） | static |

### §5.6 前端口内队列是否删除

| # | 断言 | 判定 | 证据 | 层级 |
|---|---|---|---|---|
| 33 | 删除 localStorage queue/lease/`window.__IELTS_CLOUD_REVIEW_WORKER__` | **FALSE（代码仍在）/ PARTIAL（已不可达）** | `src\pages\UnifiedPreview.tsx` 仍存在 66 KB；`__IELTS_CLOUD_REVIEW_WORKER__` 在 `:38,209,212,243`，localStorage 读写 `:84`，云端轮询 `setInterval` 在 `:573`。可达性：`App.tsx` 不 import `legacyRoutes`，且 `router.ts:47-49` 把 `#/legacy/preview/<id>` 重定向到 `/items/<id>` → 页面**实际不可达**，但**未删除**（`task_plan.md:29` 却称 M2「删前端队列」complete） | static |

### §12.1 Import Drawer 默认值

| # | 断言 | 判定 | 证据 | 层级 |
|---|---|---|---|---|
| 34 | `title = filename stem` | **PARTIAL** | 主路径成立：picker 传 `titleHint=cleanFileStem(name)`（`desktopDialogs.ts:94,153,169-175`），后端优先用 `title_hint`（`commands.rs:72-76`）。但后端 fallback 有 bug：`file.name.trim_end_matches(['.',' '])` 保留扩展名，仅当为空才取 stem（`commands.rs:76-80`）→ 无 hint 时 title 带 `.pdf` | static |
| 35 | `modality = auto detect / Reading default` | **PARTIAL/FALSE** | 无自动检测：`upsert_item_shell` 硬编码 `modality:"reading"`（`commands.rs:151`），schema 有 `CHECK (modality IN ...)`（`schema.rs:52`） | static |
| 36 | `parseMode = auto` | **FALSE** | 后端无 `parseMode` 字段；调度器恒以 `execution_mode:"localOnly"` 跑本地（`scheduler.rs:274-279`） | static |
| 37 | `cloud = app setting` | **FALSE** | 后端 `input.cloud_enabled.unwrap_or(false)`（`commands.rs:67`），默认 false；调度器默认 `cloud_enabled_default=false`、`cloud_concurrency=2` 硬编码（`scheduler.rs:42-51`）；设置页的 `cloudConcurrency`/开发者开关只存 localStorage（`appSettings.ts:26-32`），**从不被后端读取** | static |
| 38 | `category = unknown（识别后填）` | **PARTIAL** | 建 job 时 `category: None`（`commands.rs:93`）；但「识别后填」无实现：`set_item_status_ready`（`scheduler.rs:487-499`）只写 status，不写 category | static |
| 39 | `frequency = unset` | **HOLDS** | `commands.rs:94`（`frequency: None`） | static |

### §12.2 批量创建必须先落库（两阶段）

| # | 断言 | 判定 | 证据 | 层级 |
|---|---|---|---|---|
| 40 | `stage_import_batch` 先写 batch staging | **FALSE** | 无 staging 表/批概念；`import_files_at_root` 逐文件处理（`commands.rs:71-129`），`stage_source_file` 直接 hash+rename 落 `uploads/`（`job_commands.rs:186-205`） | static |
| 41 | 单事务建 asset→item→job | **PARTIAL** | 无 asset 注册（`source_asset_id` 直接塞 `job_id`，`commands.rs:159-161`）；item+queue 在同一 `Immediate` 事务（`commands.rs:144-168`），但 job.json 在事务外先写（`commands.rs:91-98`）→ 非「一个事务」 | static |
| 42 | `promote_staged_files_after_commit` 提交后 promote | **FALSE** | 函数不存在；staging 在事务前已完成 promote | static |
| 43 | 单文件 promote 失败 → 标记 `failed/source_commit_failed` | **FALSE** | `source_commit_failed` 全仓零命中。实际失败路径：`queue_import` 失败→事务回滚（item 行不存在）→`set_item_status(&conn,&job_id,"failed")` 是 **UPDATE**（`repository.rs:168-176`），命中 0 行 → **无 item 行、无 queue 行**，只剩 job.json + uploads 文件，成为孤儿（`commands.rs:120-127`） | static |
| 44 | 单文件失败不阻止其他文件建行 | **HOLDS** | `continue` 语义（`commands.rs:103-109,120-127`） | static |

### §12.3 进度不是虚假百分比

| # | 断言 | 判定 | 证据 | 层级 |
|---|---|---|---|---|
| 45 | 8 段权重表（prepare_source 5 / render 20 / local_layout 20 / local_semantics 15 / cloud_upload 5 / cloud_inference 20 / validate_repair 5 / reconcile 10） | **FALSE** | `stage_percent` 是**另一套累计检查点**：`queued=0 / preparing_source=5 / local_recognition=45 / cloud_recognition=70 / reconciling=85 / ready_for_review=100`（`scheduler.rs:75-86`）。且 `preparing_source`/`reconciling` 从未被 `advance_stage` 设置（grep 仅命中 map 自身）→ 实际只出现 0→45→70→100，8 段表无实现 | static |
| 46 | 「云端不可用但本地成功→待检查，不卡 80%」 | **HOLDS** | 无云端/云端失败均推进 `ready_for_review`（`scheduler.rs:344-379`） | static |

### §12.4 取消与重试

| # | 断言 | 判定 | 证据 | 层级 |
|---|---|---|---|---|
| 47 | 取消未开始 job：立即 cancelled + 清理 temp | **PARTIAL** | `request_cancel` 仅对 `stage='queued'` 生效（`queue.rs:274-286`）；**无 temp 清理** | static |
| 48 | 取消本地正在运行：阶段边界检查取消 token | **PARTIAL** | 取消标记仅存内存 `HashSet`（`scheduler.rs:60,171`）；检查点仅两处：本地前（`scheduler.rs:261`）、云端前（`scheduler.rs:312`）。**云端禁用时本地跑完后无检查**，取消被吞（直接进 `ready_for_review`，`scheduler.rs:364-379`） | static |
| 49 | 取消 HTTP 云端调用 / 迟到结果不覆盖用户稿 | **FALSE** | 云端 `spawn_blocking` 不可取消；云端返回后**无取消检查**、无迟到结果守卫，直接 `advance(ready_for_review)`（`scheduler.rs:334-360`）→ 取消发生在云端期间会被忽略 | static |
| 50 | 重试只重跑失败阶段，已有 local candidate 不重复解析 | **FALSE** | `queue::retry` 把所有状态重置为 `not_started` 并 `stage='queued'`（`queue.rs:257-271`）→ 全链重跑（含重新解析）；且命令无 `stage` 入参（#16） | static |

### §12.5 应用重启恢复

| # | 断言 | 判定 | 证据 | 层级 |
|---|---|---|---|---|
| 51 | `jobs_with_status(Running)` 遍历 | **PARTIAL（实现更强）** | 无 `jobs_with_status`，但 `recover_on_startup` 用 SQL 覆盖 `running/local_recognition/cloud_recognition/reconciling` 四阶段（`queue.rs:232-254`），优于计划 | static |
| 52 | `mark_interrupted` 持久化 interrupted 状态 | **FALSE** | 无 `interrupted` stage/status；仅写 `last_error_code='interrupted'`（`queue.rs:238,247`） | static |
| 53 | `retry_count < MAX_AUTO_RECOVERY` → enqueue | **HOLDS** | `queue.rs:234-243`；`MAX_AUTO_RECOVERY=3`（`scheduler.rs:29`），`start` 时调用（`scheduler.rs:135`） | static |
| 54 | 超限 → `require_action("处理被中断，请点击重试")` | **PARTIAL/FALSE** | 无 `require_action`；改为 `stage='ready_for_review', local_status='action_required'`（`queue.rs:244-252`）。**文案错误**：两分支都置 `last_error_code='interrupted'`，而 `display_message` 把 `interrupted` 映射为「上次运行被中断，**已自动排队重试**」（`scheduler.rs:90-93`）→ 超限未重排队的 job 也谎称已重试 | static |

### §12.6 前端订阅

| # | 断言 | 判定 | 证据 | 层级 |
|---|---|---|---|---|
| 55 | `listen("processing://item-updated")` 单订阅 | **HOLDS** | `processingClient.ts:19-28` | static |
| 56 | 不轮询、每个详情页不拥有 worker | **HOLDS（活跃 UI）** | `libraryStore.ts:103-111` 无 `setInterval`；无页面级 worker | static |

### 交叉验证历史缺陷（F07/F08/F09）

| # | 历史缺陷 | 判定 | 证据 | 层级 |
|---|---|---|---|---|
| 57 | **F07** 认领 SQL 把 job_id 与时间戳绑混 | **已修复（HOLDS）** | `claim_next` 绑定分离：`params![now, worker_id, lease_expires, job_id]` ↔ `SET ... updated_at=?1 ... lease_owner=?2, lease_expires_at=?3 WHERE id=?4`（`queue.rs:114-122`）；select 的 `job_id` 独立变量（`queue.rs:99-113`） | static |
| 58 | **F08** `stage_source_file` 未登记 `SourceFile` | **已修复（HOLDS）** | `stage_source_file` 构造 `SourceFile` 并 `job.source_files.push(...)` + `update_job`（`job_commands.rs:206-221`）；导入路径确实调用：`commands.rs:100`；识别器按 `main_source_file(&job)` 解析（`auto_pipeline.rs:1441-1444`） | static |
| 59 | **F09** 恢复/过期 lease 只覆盖 running | **已修复（HOLDS）** | 认领谓词含 `running/local_recognition/cloud_recognition/reconciling`（`queue.rs:101-105`）；续租同集合（`queue.rs:157`）；恢复同集合（`queue.rs:240,249`） | static |
| 60 | **F01** 应用无法编译 | **UNVERIFIABLE（本轮禁跑 cargo）** | 修复轮声称 `cargo check --locked` 通过（`repair-2026-09-07/progress.md:24`）。静态看原先 6 处错误点已改（`ProcessingSettings::defaults` 现为 `pub(crate)` `scheduler.rs:42`；`scheduler.rs:220/433/460` 的闭包移动已重写为 `job_id.clone()`）；但本轮不运行构建，不能独立确认 | doc-only |

---

## 并发与状态机核查

### 1. 本地/云端并非并发（§5.5 核心承诺落空）
计划把「两条主识别链并发 + 一路失败不取消另一路」当作 P0 卖点。实现是**严格串行**：本地 `await` 完成后才进云端（`scheduler.rs:267-360`）。本地 permit 在 `scheduler.rs:284` 提前释放，云端 permit 于 `scheduler.rs:317` 获取，二者无重叠窗口。后果：
- 单题总时长 = 本地 + 云端（计划 §19.9 只承诺本地首稿 <3s，但工作区可用性依赖本地先完成，实际未劣化；然而云端对照稿的产出被本地阻塞）。
- 本地失败 → 云端永不运行，计划中「本地失败时云端只作对照稿」的分支（`canonical_from_cloud_with_review`）不存在。

### 2. 取消状态机在云端窗口内失效（新缺陷 A4-F02）
取消标记只在「本地前」「云端前」检查。取消发生在云端 `spawn_blocking` 期间时，云端返回后直接推进 `ready_for_review`（`scheduler.rs:344-359`），既不检查 `state.cancelled` 也无迟到结果守卫。云端禁用路径同样没有本地后检查（`scheduler.rs:364-379`）。§12.4「取消本地正在运行：在页/阶段边界退出」只在云端启用时部分成立。

### 3. 恢复分支的用户文案自相矛盾（新缺陷 A4-F03）
`recover_on_startup` 两个分支都写 `last_error_code='interrupted'`，而 `display_message` 对 `interrupted` 唯一映射是「已自动排队重试」。超限（`retry_count>=3`）转 `ready_for_review/action_required` 的 job 实际未重排队，却向用户谎称已重试。状态与文案不一致，且没有真正的 `action_required` 阶段（`ready_for_review` 语义是「可打开检查」）。

### 4. 认领门控存在 TOCTOU / 超发风险（新缺陷 A4-F06）
调度循环以 `local_permits.available_permits()==0` 作为「是否继续认领」的门（`scheduler.rs:143-145`），但 permit 是在 `run_job_inner` 里**异步**获取的（`scheduler.rs:256`）。claim（`scheduler.rs:147-154`）与 acquire 之间无原子性：在 800ms tick 内若 run_job 尚未被调度，可连续 claim 超过 `local_concurrency` 的 job。计划 §5.5 的 `ctx.job_semaphore.acquire()` 先拿许可再认领的顺序被颠倒。

### 5. 过期 lease 认领会污染 `local_status`（新缺陷 A4-F05）
`claim_next` 的 UPDATE 对非 `running` 阶段一律 `local_status='running'`（`queue.rs:118`）。reclaim 一个 `cloud_recognition`（本地已 `succeeded`）的过期 job 时，会把 `local_status` 从 `succeeded` 覆盖为 `running`，丢失「本地已完成」的事实，并使重跑从零开始。计划 §12.4「已有 valid local candidate 不重复解析」在此路径上不可能实现。

### 6. 导入失败路径留下孤儿（新缺陷 A4-F01）
见断言 #43：`queue_import` 的事务一旦失败即回滚，item 行不存在；随后的 `set_item_status(...,"failed")` 是 `UPDATE`（`repository.rs:168-176`），命中 0 行。结果：磁盘留下 `job.json` + `uploads/<hash>-<name>`，DB 既无 `library_items_v2` 行也无 `processing_jobs_v2` 行。这与 §12.2「不留『Processing 但永远找不到源文件』的悬挂任务」的表面结论不同——它不留悬挂任务，但留**不可见孤儿文件**，且用户只看到 `rejected` 文案。计划的 `failed/source_commit_failed` 标记与 `promote_staged_files_after_commit` 完全未实现。

### 7. 设置页与调度器脱节（新缺陷 A4-F04）
`appSettings.ts` 的 `cloudConcurrency`/`localConcurrency`/开发者开关只写 localStorage（`:59-73`），后端 `ProcessingSettings::defaults()` 硬编码（`scheduler.rs:42-51`），`import_files` 的 `cloud_enabled` 默认 false（`commands.rs:67`）。§5.3 的 `save_app_settings`/`get_app_settings` 命令不存在 → 用户改并发/云端开关**不影响**真实处理行为。§12.1「cloud = app setting」不成立。

### 8. 阶段常量与实际推进不匹配
`preparing_source`/`reconciling`/`ready_to_publish` 只出现在 `stage_percent`/`display_message` 的 map 中，从无 `advance_stage` 写入；`STAGE_RECONCILING` 常量根本不存在（SQL 用字面量 `'reconciling'`）。因此「reconcile」既无阶段也无进度，§12.3 的 10% reconcile 权重为空。

---

## 发现清单

### A4-F01 [P0] 导入队列失败留下不可见孤儿文件（§12.2 核心承诺落空）
- **结论**：`queue_import` 失败时事务回滚 → item 行不存在 → `set_item_status("failed")` 命中 0 行。磁盘留 `job.json`+staging 后的 `uploads/` 文件，DB 无 item/无 job。计划声明的 `failed/source_commit_failed` 标记与 `promote_staged_files_after_commit` 均不存在。
- **证据**：`F:\workspace\PDF2Test\src-tauri\src\processing\commands.rs:112-127`（失败处理）、`commands.rs:135-170`（queue_import 事务）、`F:\workspace\PDF2Test\src-tauri\src\library\repository.rs:168-176`（`set_item_status` 为 UPDATE）。grep `source_commit_failed`/`promote_staged` 零命中。
- **影响**：用户看不到条目却占用磁盘；重试无从下手；与 §12.2「任何单文件失败都形成明确结果」矛盾。
- **建议**：在事务内 upsert 失败行（`status='failed'`）后再提交，或失败时把已落盘文件移到 quarantine 并写一条 failed item；补 `source_commit_failed` 错误码。

### A4-F02 [P1] 云端执行窗口内的取消被吞掉（§12.4）
- **结论**：取消标记仅在本地前/云端前检查；云端返回后无取消检查、无迟到结果守卫，直接 `ready_for_review`。云端禁用时本地后亦无检查。
- **证据**：`scheduler.rs:312-316`（云端前检查）、`scheduler.rs:334-360`（云端后无检查）、`scheduler.rs:364-379`（无云端路径无检查）、`queue.rs:274-286`（`request_cancel` 只处理 queued）。
- **影响**：用户取消后任务仍标记完成，违反 §12.4。
- **建议**：云端 `await` 返回后与每次 `advance` 前统一检查 `cancelled`（含 no-cloud 路径）；取消走 `finish_cancelled` 并丢弃迟到结果。

### A4-F03 [P1] 启动恢复的 action_required 分支向用户谎称已重试（§12.5）
- **结论**：两个恢复分支都写 `last_error_code='interrupted'`；`display_message` 对 `interrupted` 只输出「已自动排队重试」，超限转 `ready_for_review/action_required` 的 job 实际未重排队。
- **证据**：`queue.rs:232-254`、`scheduler.rs:88-93`。
- **影响**：用户等待一个永远不会自己重试的任务；与计划 `require_action("处理被中断，请点击重试")` 文案不符。
- **建议**：区分 `interrupted_requeued` 与 `interrupted_action_required` 两个错误码，或在 action_required 分支写独立 code 并映射正确文案；最好引入真正的 `action_required` 阶段。

### A4-F04 [P1] 设置页的并发/云端开关与调度器完全脱节（§5.3/§12.1）
- **结论**：无 `get_app_settings`/`save_app_settings` 命令；`appSettings.ts` 只落 localStorage；调度器 `ProcessingSettings::defaults()` 硬编码 `cloud_enabled_default=false`、`cloud_concurrency=2`；`import_files` 云端默认 false。
- **证据**：`src\features\settings\appSettings.ts:5-7,26-32,59-73`、`src-tauri\src\processing\scheduler.rs:42-51`、`src-tauri\src\processing\commands.rs:67`。
- **影响**：§12.1「cloud = app setting」不成立；用户在设置页的并发/云端选择无任何效果。
- **建议**：新增 `get_app_settings`/`save_app_settings`（§17.1 已列），调度器从设置读取并发与云端开关；或明确把该能力降级标注为未实现。

### A4-F05 [P1] 过期 lease 认领覆盖 `local_status`，破坏「本地已完成」事实（§12.4）
- **结论**：reclaim 非 `running` 阶段时无条件 `local_status='running'`，覆盖 `cloud_recognition` 阶段的 `succeeded`。
- **证据**：`src-tauri\src\processing\queue.rs:114-122`（`CASE WHEN stage != 'running' THEN 'running'`）。
- **影响**：重试/恢复路径无法复用已完成的本地候选，与 §12.4「不重复解析」直接冲突；进度回退。
- **建议**：reclaim 时只重置被中断的阶段状态，保留已 `succeeded` 的 `local_status`。

### A4-F06 [P2] 调度循环认领门控 TOCTOU，可能超发并发（§5.5）
- **结论**：以 `available_permits()==0` 决定是否认领，permit 在 spawn 后异步获取，claim 与 acquire 非原子；计划要求先 `job_semaphore.acquire()` 再认领。
- **证据**：`scheduler.rs:141-166`、`scheduler.rs:256`。
- **影响**：短时超过 `local_concurrency` 的阻塞任务并发，CPU/内存尖峰。
- **建议**：在 claim 前同步获取 permit（或 `try_acquire` 失败即跳过），持有至 job 结束。

### A4-F07 [P1] §5.1 目标模块布局绝大部分不存在（纸面设计）
- **结论**：`recognition/`、`reconcile/`、`editor/`、`publish/` 四目录**完全不存在**；`processing/` 缺 `worker/state/events/recovery` 四文件；`library/` 缺 `assets.rs`/`cleanup.rs`。
- **证据**：ls 实测（`src-tauri/src/` 下无 `recognition`/`reconcile`/`editor`/`publish`）；`src-tauri\src\processing\` 仅 4 文件。
- **影响**：§5.1 与 §17 的逐文件清单不能作为已完成度证据；M2「complete」仅覆盖 `processing` 的调度子集。
- **建议**：把 §5.1/§17 明确标注为「目标态，M4/M5/M6 交付」，并在 `task_plan.md` 的 M2 行补注实际交付范围。

### A4-F08 [P1] §5.2 状态模型类型全部缺失
- **结论**：`StageStatus`/`ProcessingItemV1`/`ProcessingStage`/`StageResult`/`ProcessingProgress` 全仓零命中；实际用字符串列 + `ProcessingJobRow`。
- **证据**：grep 零命中；`queue.rs:14-20,26-41`。
- **影响**：§5.2 无法作为类型契约核对；`ActionRequired`/`Interrupted`/`ReadyToPublish` 无持久化落点（与 A4-F03 同源）。
- **建议**：要么实现枚举并加迁移，要么把 §5.2 改为「当前字符串状态机」的如实描述。

### A4-F09 [P1] §5.3 七条命令只落地 5 条且签名不符
- **结论**：`publish_library_items`→实际 `publish_items`；`save_app_settings`（及 §17.1 的 `get_app_settings`）不存在；`retry_processing` 无 `stage` 入参；`import_files` 返回结构与 `list_library_items` 入参均不符计划。
- **证据**：`lib.rs:925-1000,1528-1537`；`commands.rs:52-57`；`library\commands.rs:82-93`。
- **影响**：§5.3 命令收敛未完成；§12.4「只重跑失败阶段」无接口。
- **建议**：补 `stage` 入参与设置命令，或修正 §5.3 清单。

### A4-F10 [P2] §5.5 的持久化/合并/清理环节整体缺失
- **结论**：`persist_candidate`/`reconcile`/`canonical_from_local`/`complete_job`/`cleanup.schedule`/`CleanupReason`/`job_semaphore` 全部不存在；本地与云端结果从不合并。
- **证据**：grep 零命中；`scheduler.rs:344-379` 直接推进 `ready_for_review`。
- **影响**：§24 DoD「cloud 失败不阻止打开本地稿」成立，但「三方合并 / 迟到 cloud 不覆盖 user_edited」无落点（与 M5 pending 一致）。
- **建议**：标注为 M5 交付；调度器补齐 `cleanup.schedule` 调用（`cleanup.rs:99` 已有可复用函数）。

### A4-F11 [P2] §12.3 进度权重表未实现，且部分阶段永不出现
- **结论**：实际为 0/5/45/70/85/100 检查点；`preparing_source`/`reconciling`/`ready_to_publish` 从不被写入；8 段权重（render/local_layout/local_semantics/cloud_upload/cloud_inference/validate_repair）无任何对应物。
- **证据**：`scheduler.rs:75-86`；grep `preparing_source`/`reconciling` 仅命中 map 自身与 SQL 字面量。
- **影响**：进度条跳变（0→45），计划「不是虚假百分比」的粒度承诺未兑现（虽非虚假，但粗粒度且不匹配文档）。
- **建议**：或实现阶段化权重，或把 §12.3 改为实际检查点表。

### A4-F12 [P2] §12.1 默认值与实现不符（modality/parseMode/cloud/category）
- **结论**：modality 硬编码 `reading`（无 auto detect）；无 `parseMode`；cloud 默认 false 且不读设置；category 识别后不填；title 后端 fallback 保留扩展名。
- **证据**：`commands.rs:67,76-80,93,151`；`scheduler.rs:274-279`。
- **影响**：§12.1 的字段默认值表大部分为纸面。
- **建议**：按表逐项实现或修正文档。

### A4-F13 [P3] 前端未真正删除旧队列，M2 声明过度（§5.6 / task_plan M2）
- **结论**：`UnifiedPreview.tsx` 仍在（含 `__IELTS_CLOUD_REVIEW_WORKER__`、localStorage lease、`setInterval` 轮询），仅因 `App.tsx` 不再 import `legacyRoutes` 而不可达；`task_plan.md:29` 却称 M2「删前端队列与 2s 轮询」complete。
- **证据**：`src\pages\UnifiedPreview.tsx:38,84,209,212,243,573`；`src\app\legacyRoutes.tsx:17,81`；`src\app\App.tsx`（无 legacyRoutes import）；`router.ts:47-49`（重定向）。
- **影响**：审计/维护者若 grep 到该文件会误判队列未迁移；M2 完成度被高估。
- **建议**：按 P10 计划整体删除 legacy 页面集合，或在 task_plan M2 行注明「代码保留、已不可达，待 P10 删除」。

### A4-F14 [P3] 死代码与文档漂移
- **结论**：`enqueue_processing_job`（`scheduler.rs:502-512`）定义后全仓无调用；`§5.3`（7 命令）与 `§17.1`（8 命令含 `get_app_settings`）清单不一致。
- **证据**：grep `enqueue_processing_job` 仅定义点；计划 576-594 vs 2758-2767。
- **影响**：误导后续实现者。
- **建议**：删除死代码；对齐两处命令清单。

---

## 与历史审计的关系

| 历史发现 | 上轮状态 | 本轮核验 | 证据 |
|---|---|---|---|
| F01 应用无法编译 | P1 阻断 | **UNVERIFIABLE（禁跑构建）**；静态看 6 处错误点已重写，但无法独立确认 | `scheduler.rs:42,208-237`；`repair-2026-09-07/progress.md:24` |
| F07 认领 SQL 绑参错误 | P1 阻断 | **已修复** | `queue.rs:99-122` |
| F08 源文件未登记 | P1 | **已修复** | `job_commands.rs:206-221`；`commands.rs:100` |
| F09 恢复只覆盖 running | P1 | **已修复**（四阶段全覆盖） | `queue.rs:101-105,157,240,249` |
| M2「complete」 | 上轮判 in-progress | **过度声明**：调度子集可用，但 §5.1 模块布局、设置命令、前端队列删除均未完成；新增 A4-F01/F02/F03/F05 | 见发现清单 |
| 上轮「2s 轮询」 | 存在 | **已删除**（活跃 UI）；仅孤立文件残留 | `libraryStore.ts:103-111`；`UnifiedPreview.tsx:573` |

**修复轮确实关闭了 F07/F08/F09**（本轮以当前代码逐条确认）。但修复引入/遗留了 6 条新的并发与状态机缺陷（A4-F01…A4-F06），其中 A4-F01 与 A4-F02 直接违反 §12.2/§12.4 的明文承诺。计划 §5.1/§5.2/§5.3/§5.5 的相当部分仍是**纸面设计**，不应被 M2 的「complete」标签覆盖。

---

## 证据层级与局限

**证据层级分布**
- `product`（真实 Tauri 端到端）：**0 条**。本轮未运行 Tauri，未做真实导入/取消/重启。
- `command`（命令处理层）：`import_files`/`list_library_items`/`retry_processing` 注册与核心逻辑（#12–#19）。
- `static`（静态代码/SQL/目录实测）：绝大多数断言（#1–#11、#20–#33、#40–#59）。
- `doc-only`（仅文档声明）：#7、#18b、F01 的构建结论。

**局限**
1. **未运行 `cargo check`/`cargo test`/`npm run build`**（任务约束）→ F01 编译状态、并发行为、`advance_stage` 的租约竞态均只能静态推断，无法动态证伪。
2. 未启动 Tauri → 取消/重启恢复/云端迟到结果等时序缺陷（A4-F02/F03/F05/F06）为**静态状态机推演**，非运行时复现。
3. 未运行 SQLite → 断言 #43 的「`set_item_status` 命中 0 行」基于 `UPDATE` 语义与事务回滚的静态推断，未实测。
4. 前端「不可达」结论基于 `App.tsx` 导入图 + `router.ts` 重定向的静态分析；若存在动态 `import()` 或测试 harness 直接挂载 `legacyRoutes`，可达性结论需修正。
5. 计划 §5.5 伪代码中的 `CandidateKind`/`JobId`/`AppContext` 等类型未逐一核验（因核心函数 `persist_candidate`/`reconcile` 已确认不存在，未再展开）。
6. 未审计 §5 之外的 M4/M5 交付物（`recognition/*`、`reconcile/*` 属 M4/M5 范围），本轮只判定「§5.1 声称的布局当前不存在」，不评判其设计合理性。
