# REPORT 2026-09-18 — 云端自主识别与修复循环的接通现状

本轮授权：**提高云端自主性，云端可以纠正本地「确定性结论」**；旧 A3/A4「模型只能建议、
不能覆盖本地」的政策作废。核心要求：没有对应的本地节点，不等于云端不能创建正确的新节点。

本文只写**可复现的证据**，并按 `AGENTS.md` 要求把三类证据分开标注：

- **产品行为（端到端）**：真实应用进程 + 真实 IPC + 真实 DOM。
- **服务/命令处理器行为（UI 之下）**：真实网关 / 真实 HTTP / 真实 SQLite，但不经过界面。
- **仅 CLI / 仅 schema**：不驱动产品路径。

---

## 1. 云端现在能自主做什么

一次导入之后，**不需要用户点任何按钮**，后端会自己跑完：

1. 本地快速出稿（用户此时已可编辑，这是产品的核心承诺）；
2. 云端**独立**识别原文件，产出一份完整候选（正文 / 题组 / 富内容题干 / 选项库 /
   作答位置 / 答案），**候选不写权威稿**；
3. 后端给候选接身份、重算质量、独立落盘；
4. **自主修复循环**：模型每回合只回一个应用层 JSON 工具消息
   （`read_draft` / `read_source` / `apply_edits` / `finish`），由 Rust **真的执行**
   编辑并把握稿、版本、拒绝原因回传下一回合。上限 6 回合 / 10 分钟，可取消，
   重复且无进展会停。
5. 循环结束后，**剩余问题由后端按当前权威稿重算**——`finish` 不是产品完成。

对应用户的收益（本轮消掉的操作）：

| 以前用户要做 | 现在 |
| --- | --- |
| 逐条读云端建议，自己判断哪条对、再手改 | 云端直接改，用户只看**还剩什么** |
| 内容改过之后，旧问题仍挂在待办里，得手动划掉 | 逐目标重判，条件已被满足的阻塞项自动消失 |
| 云端失败了被显示成「本次没有云端参与」 | 如实报 `failed` / `partial`，错误码保留 |
| 想撤销整轮云端修改只能自己一条条改回去 | 一键撤销整轮修复（走 Rust 批次回滚） |
| 别的窗口/云端改了稿，编辑器不知道 | 权威稿版本推进时通知编辑器，保存后自动加载最新版本 |

## 2. 仍然需要用户处理的部分

- 修复循环**明确没解决**的阻塞问题（`blocking: true`）——这是设计意图，不是缺陷：
  模型不能通过 `resolveIssue` 把问题标成已解决，`MODEL_ALLOWED_OPS` 刻意不含该操作。
- 模型自报的、无法从原文件确证的差异（`blocking: false`，`action: review_difference`）。
- 修复预算耗尽（6 回合 / 10 分钟）后剩下的内容。
- 版本冲突：本地未保存修改与云端修改撞在同一目标上时，用户需要选「重试保存」或
  「放弃本地修改」。

## 3. 本轮提交

| 提交 | 内容 |
| --- | --- |
| `a3d8ca7` | 调度接入新主链（完整候选 → 云端修复循环） |
| `fbe603c` | 云端拉取失败不再被报成 `not_run` |
| `5588412` | 识别阻塞逐目标重判 + 发布判据三处统一 |
| `af81c3f` | 批次记录承载云端修复摘要（`repair_json` + 前端视图） |
| `fcf71f7` | 修复摘要接线 + 整轮修复撤销入口 |
| `acf9ee6` | 视图契约补 `repair`（与 Rust 真源对齐） |

本轮工作区改动（已提交，见上）：事件载荷携带 `editVersion`、内容提交推高
`stateVersion` 并发事件、前端 `pendingRemoteVersion` 延迟刷新。

## 4. 验证（真实退出码）

命令逐条单独执行并捕获**真实退出码**（不经管道，避免 `$?` 读到 `tail` 的退出码）。

| 命令 | 退出码 | 结果 |
| --- | --- | --- |
| `cargo test --lib` | 0 | 814 passed / 0 failed / 11 ignored |
| `cargo check --all-targets` | 0 | 仅既有 warning，无 error |
| `npm run check`（`tsc --noEmit`） | 0 | — |
| `npm test`（`vitest run`） | 0 | 283 passed / 16 files |
| `node scripts/recognition/contract-drift.mjs` | 1 | 5 处破坏性不一致、1 处需留意 |

关于契约漂移检查的退出码 1：在 `fbe603c` 的临时 `git worktree` 里实测**同数同项**
（基线也是 1）。本次改动**未新增漂移**；那 5 处是既有问题，不在本次授权范围内。

### 4.1 产品行为（端到端，真实应用进程）

| 脚本 | 退出码 | 结果 |
| --- | --- | --- |
| `node scripts/e2e/tauri-cdp-smoke.mjs` | 0 | 5/5 步骤通过 |

步骤：`app-launch-and-page-load` / `dom-read-app-shell` / `tauri-command-roundtrip` /
`real-ui-click-settings-tab` / `page-survives-after-interaction`，全部 passed。

> 标注：本次为**诊断参数运行**（`--no-sandbox --disable-gpu`，本机 WebView2 必需），
> 不得写成「默认产品路径通过」。

### 4.2 服务/命令处理器行为（UI 之下，真实 HTTP + 真实 SQLite）

`cloud_repair::tests::controlled_model_service_drives_a_real_repair_round_through_the_real_gateway`

这条用例起一个**有剧本的受控 HTTP 模型服务**，让 `run_repair_loop` 用**真实网关**
（`repair_authoring_step_through_gateway`）作为 `step`，断言：

- 请求**真的**到了服务，且首轮请求带 `application/pdf` 附件（证据面是原文件，
  不是只发一句 prompt）；
- 第二轮请求带上了第一轮工具的真实结果（「读稿 → 改稿」闭环成立）；
- 模型提交的编辑**真的落到权威稿**（读回 `answerKey.q14` 逐字比对）；
- 编辑版本**真的推进**；
- `repair_run_id` 原样带出（前端撤销依赖它）。

这条用例是**交接文档点名的那条缝**：此前所有修复循环用例都把模型输出直接塞进注入点，
证明不了「模型服务那一端真的接上了」。

### 4.3 未验证 / 明确缺口

- **`run_job_inner` 的云端分支编排没有 UI 级端到端覆盖**。调度器里
  「候选 → 修复 → 写库 → advance」这一段需要 `AppHandle`，单元测试构造不出。
  目前它的正确性由三部分间接保证：映射函数的单测（`fbe603c`）、
  本条真实网关用例（4.2）、以及 `local_only_cycle_runs_reconcile_and_reports_cloud_not_run`
  （无云路径真的跑完核心）。
- **没有跑「导入 → 云端修复 → 编辑 → 重开 → 预览 → 导出 → 学生端」的完整 CDP 链路。**
  `scripts/e2e/tauri-cdp-*.mjs` 这套夹具已存在（含受控模型服务），但受控服务目前
  只应答 outline / A3 / A4 三类任务，**还没有 `repair_authoring_step` 的剧本**，
  所以那条链路要跑起来还需要给 `scripts/controlled-llm-service.mjs` 加一个修复模式。
- `repair_json` 没有 `repair = running` 的增量写入：修复进行中，前端看到的仍是上一次
  的摘要（或没有摘要），只有循环结束才落库。
- 「云端可以纠正本地确定性结论」的 Phase-B 复核**没有做**。
- 取消（`cancel`）在修复循环里有实现，但没有在界面上暴露入口。
- 本地/生产库从旧 schema 升级到 v7（新增 `repair_json`）的迁移路径只在内存库上测过。
- 前端 `useCanonicalEditor` 的延迟刷新逻辑由纯函数单测覆盖（`remoteVersion.test.ts`），
  但**没有在真实应用里驱动过一次「云端改了稿 + 本地有未保存修改」的时序**。

## 5. 职责与文件

- `src-tauri/src/processing/scheduler.rs`：事件载荷构造（`item_updated_payload`）、
  内容变更通知（`prepare_content_change_notification` / `notify_item_content_changed`）、
  云端状态映射（`cloud_status_for_job`）。
- `src-tauri/src/processing/queue.rs`：`bump_event_seq`、`get_job_by_library_item`。
- `src-tauri/src/library/repository.rs`：`current_edit_version`（只读版本号，不解析稿）。
- `src-tauri/src/lib.rs`：三条改写权威稿的命令（`apply_editor_commands` /
  `apply_recognition_decisions` / `undo_cloud_repair`）在成功后统一发通知。
- `src/api/processingClient.ts`：载荷收窄 + 单调去重（`acceptProcessingUpdate`）。
- `src/features/editor/remoteVersion.ts`：远端版本处置的纯函数判定。
- `src/features/editor/useCanonicalEditor.ts`：`pendingRemoteVersion` 延迟刷新。
