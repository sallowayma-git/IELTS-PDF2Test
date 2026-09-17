# PDF2Test 项目长期记忆

## 工程约定
- **产品是 Tauri 应用**，不是 CLI（见 `AGENTS.md`）。CLI/脚本只作夹具与诊断，不得用 CLI 通过代替产品链路验证。
- 测试基线口径：`cd src-tauri && cargo test --lib --no-fail-fast`。
  历史：671（09-15 交接）→ 684（四代理落盘）→ 687（09-16 主线程补测）→ 693（09-16 R1/R2/R3/R5 落地）
  → 696（09-16 撤销原子性 / 基线保护 / 受控模型服务场景）→ 730（09-17 首稿初始化提前）
  → **757（09-17 云端修复写入入口）**，`11 ignored` 长期不变。
- 用户指定协作边界：我侧只做 `src-tauri/` 后端；`src/` 前端需另派。跨文件并发代理必须文件集互斥。
- **工作区是多 agent 共享的**：`scripts/e2e/*`、`package.json` 等常有其他 agent 的在途改动。
  提交前必须 `git status --porcelain` 逐项确认归属，**只 `git add` 自己改的文件**，不要用 `git add -A`。

## 写测试的夹具约定（重要）
- **会写库的用例（接受 / 撤销 / 自动应用）必须用真实 golden 稿**：
  `fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json`（槽位 q14/q15，选项库 A–E）。
  手写精简权威稿会被 `apply_editor_commands_tx` → `refresh_quality_report` + `validate_authoring`
  拒掉（`AUTHORING_SCHEMA_INVALID:missing field displayLabel`），失败原因伪装成「夹具不合格」，
  把真正要验证的逻辑掩盖掉。
- 只读用例（如无云路径判链路状态）才可用精简稿。
- `mod tests` 里 **`super::rules::…` 无效**（`super` 指向 `commands`，不是 `reconcile`）；
  要用 `crate::reconcile::rules::…`。
- `store::read_decision_file` / `read_candidate` / `read_current_batch`（`reconcile/store.rs`）是
  验证「是否真的落盘」的标准入口，优于断言内存结构。

## 待办语义的唯一判据
- `DecisionItemV1::is_actionable()`（`schema/recognition_v1.rs`）= `resolution != AutoFixed
  && status ∈ {Open, Failed}`。`build_view` 与调度器 `actionableCount` **都必须用它**。
- 「仍生效的自动修正」判据：`resolution == AutoFixed && status == Accepted`
  （只排除 `Undone` 会漏掉 `Superseded`，导致失效规则对 AutoFixed 项完全无效）。
- `Failed` 的 `severity` **不**强制为 Blocker：severity 表达「是否阻断发布」，
  Failed 表达「自动写入是否成功」，强行升级会污染发布质量门。

## 内容变更事务与基线保护（09-16 新增）
- **决策状态必须与题稿同一事务**：`library::repository::apply_editor_commands_tx_with` 的
  `on_content_committed(&Connection, i64)` 在 `commit()` 前执行，返回 Err 则整体回滚。
  原 4 参版本委托给它并传空回调。**不要在事务外另外用一条连接写状态**——那会留下
  「题稿已改、状态未写」的窗口，重试会被前提复核误判为「用户改过」而拒绝，重启也不能自愈。
- 事务回调类型是 `&dyn Fn`：**不能在里面改外部 `Vec`**。做法是先把待落库的项算好，
  回调只读；内存副本等写入成功后再更新（否则写失败时 view 与 outcomes 自相矛盾）。
- **基线可信判据唯一来源**：`engine::frozen_snapshot_is_reusable`（`resolve_local_snapshot` 也用它）。
  `ReconcileBatchOutcome.local_baseline_frozen == false` ⇒ `auto_apply_candidates` 必为空。
  原因：`auto_apply_eligible` 的守卫「权威稿当前值 == 本地识别结果」只在本地基线是**冻结快照**时成立；
  快照缺失时基线退化为「当前稿现场重投影」，两边恒等、守卫等价于不存在。
- 新原因码 `BASELINE_NOT_FROZEN`（`schema/recognition_v1.rs::reason`）：项留在 `NeedsReview` 转人工，
  **不静默丢弃**。
- **版本读取不得折成 0**：`0` 是合法版本，用 `unwrap_or(0)` 顶替「读取失败」会让批次 id
  （job_id + source_sha256 + 版本）指向不存在的批次。`current_edit_version` 返回
  `Ok(None)`（行不存在）与 `Err`（查询失败）都是「不可用」，不是 0。

## 受控模型服务场景（前端可执行）
- 样本 `fixtures/controlled-llm/reading-outline.json`、预期 `fixtures/controlled-llm/expected-decisions.json`、
  服务 `scripts/controlled-llm-service.mjs`、说明
  `Plan With Files/Dual_Recognition/CONTROLLED_LLM_SCENARIO_2026-09-16.md`。
- profile 存于 `<appData>/config/llm-profiles.json`（`llm_profiles.rs::profiles_path`）；
  网关拼 `{baseUrl}/chat/completions`，且对明文 http 有白名单（回环/localhost/私有地址）。
- **样本里 `evidence.quotes[].pageIndex` 必须 ≥ 1**，`0` 判 `cloud_outline_group_quote_invalid`。
- 测试内的 stub HTTP 服务：**不要 `join` 服务线程**（网关正常只发一次请求，accept 循环会一直等），
  且必须先按 `Content-Length` 读完请求体再回写，否则客户端收到 RST。

## 测试里做故障注入的正确姿势
- **不要在产品代码里加测试专用开关**。用 SQLite 触发器注入：
  `CREATE TRIGGER ... BEFORE UPDATE ON recognition_decisions_v1 BEGIN SELECT RAISE(ABORT, '...'); END;`
  被测函数自己按 root 开连接，看到同一库文件里的触发器 ⇒ 注入点落在真实产品路径上，产品代码零污染。
  撤除用 `DROP TRIGGER IF EXISTS`。**不能**用内存库替代——那样测不到跨连接的真实行为。

## 云端修复写入入口（09-17 Stage 2，权威稿的唯一机器写入面）

- 唯一入口：`cloud_repair::tools::apply_cloud_edits`。模型只能「提交一批领域命令 + 说明依据」：
  不能执行代码、不能改源码、不能直接写导出 JS、不能碰质量/审计/来源字段、不能标问题已解决。
- **`EditOrigin` 由调用入口决定，绝不从模型输出/请求体推断**。
  `writes_human_protection()` 只对 `Human|Undo` 为真；`enforces_protection()` 只对带 run 的 `CloudRepair` 为真。
- `MODEL_ALLOWED_OPS` 14 个（有意不含 `resolveIssue` / `bindSource` / `deleteAnswerSlot`）；
  `setNodeAttrs` 收敛到 `MODEL_ALLOWED_NODE_ATTRS`；`FORBIDDEN_COMMAND_KEYS` **剥离而非报错**
  （报错只白烧一个模型回合，模型也学不到）并如实回报 `stripped_keys`。
- `requestId` 由 `run/round/toolCall` 后端派生，**不让模型提供**：否则它能用同一 id 提交不同内容，
  或换 id 把同一补丁重复写进去。
- 保护模型**故意不过度锁定**：`setAnswer` 的影响范围 = 该槽位 + `answerKey:<slotId>`，
  **不含同组其它槽位**——一个槽位被人工编辑不该锁住整卷。人工保护存 `protected_edits_json`（schema v5）。
- 撤销**逐目标**（首个 before + 最后 after），已被本轮之外写过的目标一律跳过，**绝不整卷回滚**；
  journal 被裁剪的轮次返回 `EDIT_REPAIR_UNDO_UNAVAILABLE`，不给点了没用的假按钮。

### 两个必须记住的坑

- **`answerKey:<slotId>` 不能用 `replace_object_by_id` 回填**：它在稿件里是「以 slotId 为键的
  对象条目」，条目内部没有任何身份字段，按 id 找对象永远找不到 ⇒ 槽位对象放回去了、
  答案值还是模型写的那一份。界面报「已撤销」而内容没回来，比不能撤销更坏。
  回填统一走 `write_change_value`（它自己分派该前缀），记账也要单独记一条。
- **硬失败基线必须自洽**：「本次是否引入新机械错误」的基线要用同一套质量管线在**未施加本次编辑的
  clone** 上重算，**不要读库里那份 `quality.hardFailures`**——它可能是播种/迁移留下的、与当前内容
  不同步的旧结果，判据会退化成「库里那份质量块新不新」，一批无关旧差异就能顶掉一次有效修复
  （任务书明确要避免的「有一个问题就整卷修不动」）。

事务顺序：`prepare_ds`（质量重算 + schema 校验）→ 写入 → journal → `merge_protected_edits`
（仅 Human/Undo）→ `on_content_committed`（提交前，返回 Err 整体回滚）。
校验必须在**克隆**上跑完再回写，失败时权威稿一字不改。

## 本机环境坑（Windows / 本仓库）
- **Bash 工具不可用**（`dirname`/`cd`/`ls` 均报 command not found），一律改用 PowerShell。
- PowerShell 对 `cargo`/`git` **常返回空 stdout**：必须 `2>&1 | Out-File -FilePath <abs> -Encoding utf8`，再用 Read 读取。
- 不要用 PowerShell `ConvertFrom-Json` 校验 JSON（UTF-8 乱码 + 假阳性语法错误）；用 `python -c` 或 Node `JSON.parse`。
- PowerShell 下 `$LASTEXITCODE` 在管道给 `Select-Object` 后会被重置，**不要据它判断成败**。
- 本环境的文件删除走回收站包装器，**可能报 `SAFE_DELETE_FAIL_CLOSED` 而实际已删除**；以删除后的目录列举为准，不要只看报错。
  （09-17 补充：另有一种相反形态——`Remove-Item` 命令**整体 exit 1 且什么都没删**，这是沙箱直接拒绝执行，
  连 `Out-File` 都不会落地。仓库根的 `_*.log` / `_*.txt` 临时日志因此清不掉；它们被 gitignore，不影响提交。）
- PowerShell 读 git 输出会把 UTF-8 中文显示成 GBK 乱码，但那只是**显示**问题，提交内容是对的；
  命令前加 `[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()` 即可正确读回。

## 方法论教训
- **"文件 mtime 未变" ≠ "功能未实现"**。曾据此误判"撤销状态持久化未落地"，实际机制（`reconcile/store.rs:302` `set_decision_status`）早已存在，缺的只是测试。
- 审计代理交付时，**必须区分**：产品改动未落地 / 已落地但零测试 / 测试写错。三者修法完全不同。
- 测试若"**复刻**被测函数的行为"而非调用它（如 scheduler 里旧版快照测试），**永远测不到该函数内部的错误处理**。补测要调用真函数。

## git 仓库损坏恢复（本仓库 2026-09-16 实战，可复用）
症状：`.git` 目录存在（HEAD/config/objects/index 都在），但 `git status` 报
`fatal: not a git repository`。逐层根因与修法：

1. **`.git/refs` 目录整体缺失** → git 校验仓库需 HEAD/objects/refs 三者齐备，缺 refs 直接判定"不是仓库"。
   修：`refs`、`refs/heads`、`refs/tags` 建空目录。此步后 `rev-parse --git-dir` 即可用。
2. **孤儿 pack 索引**：`objects/pack/` 下存在只有 `.idx` 没有 `.pack` 的文件（中断的 fetch 残骸）。
   git 会以为对象存在却读不到，表现为 `fatal: unable to read <sha>`。
   修：删除所有无同名 `.pack` 的 `.idx`，以及 `multi-pack-index` 缓存。
3. **对象库大面积缺失**（本次 476 个 blob）→ 本地无历史可恢复，reflog 里的提交对象也已不存在。
   修：`git fetch --refetch origin`（强制重下全部对象，填充缺口）。实测一次成功：
   2275 objects / 6 MiB，之后 `git fsck --connectivity-only` 的 `missing` 归零。
4. **远端跟踪引用写不进去**：`git update-ref refs/remotes/origin/main <sha>` 返回 0 但文件不生成。
   根因是**嵌套目录不存在时 git 不会自动创建**。修：先 `New-Item refs/remotes/origin`，再写引用文件。
5. **旧索引不可修复**：若旧 `index` 引用了本地未推送、已成永久丢失的对象，`git reset`/`git diff` 必报缺对象。
   修：移走旧 `index`，用 `git read-tree HEAD` 从 HEAD 重建。**旧暂存状态会丢失**，需重新 `git add`。

健康判据（本仓库）：`git rev-parse HEAD` 可用、`git log` 可读、`show-ref` 同时列出
`refs/heads/main` 与 `refs/remotes/origin/main`、`fsck --connectivity-only` missing=0、
`git status -sb` 首行不含 `[gone]`。
