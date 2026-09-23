# PDF2Test 项目长期记忆

## 工程约定
- **产品是 Tauri 应用**（见 `AGENTS.md`），CLI/脚本只作夹具与诊断，**不得用 CLI 通过代替产品链路验证**。
- 测试基线：`cd src-tauri && cargo test --lib --no-fail-fast`。
  671(09-15) → 693 → 730 → 757 → 837(09-19) → **1023(09-23, wave3-listening)**；`11 ignored` 长期不变。
  跨轮对比前先确认**没有别人的在途改动**（多 agent 共享 worktree 会互相污染数字）。
- 用户指定协作边界：我侧只做 `src-tauri/`；`src/` 前端另派。跨文件并发代理必须**文件集互斥**。
- 工作区多 agent 共享：提交前 `git status --porcelain` 逐项确认归属，**只 add 自己改的文件**，不要 `add -A`。

## 写测试的夹具约定
- **会写库的用例（接受/撤销/自动应用）必须用真实 golden 稿**
  `fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json`（槽位 q14/q15，选项库 A–E）。
  精简稿会被 `apply_editor_commands_tx` → `validate_authoring` 拒掉，报
  `AUTHORING_SCHEMA_INVALID:missing field displayLabel`，失败原因伪装成「夹具不合格」，掩盖真正要验证的逻辑。
  只读用例才可用精简稿。
- `mod tests` 里 **`super::rules::…` 无效**（`super` 指向 `commands`）；要用 `crate::reconcile::rules::…`。
- 验「是否真的落盘」用 `store::read_decision_file` / `read_candidate` / `read_current_batch`，优于断言内存结构。

## 发布判据（唯一入口）
- `authoring_validation::publish_verdict(root, job_id, authoring, edit_version, scope)
  -> PublishVerdict{Ready/Blocked/Undetermined}`：**入参只有稿件 + 版本**，不接受 report、不接受 policy。
- `PublishScope{CanonicalDirect, FullDerived}` 由**来源**决定，只决定「哪些判据适用」，**不提供跳过判据的能力**。
- `apply_publish_verdict(report, verdict, scope)` 把结论写进报告，`passed` **由结论派生**。
- `ielts_grammar::quality::quality_readiness(authoring)` 是就绪度唯一规则，**重新推导**，从不读库里的 `state`。
- **`force` 能力已从后端整体删除**（09-19）：`from_policy` 只接受缺省/strict，其它值返回
  `invalid_validation_policy:<v>`（明确报错，**不静默降级成 strict**）；前端 `tauriCommands.ts` 的
  `"strict" | "force"` **未同步**。
- `merge_sidecar_validation` 缺省 = 合并；`layers` 空 = 不替换；只有显式 `replaceExistingLayers:true` 才替换。
- 09-19 起划给我的文件：`authoring_v2_commands.rs`、`ielts_grammar/quality.rs`、`instruction_signature.rs`。
- **包发布状态必须同时反映「门禁结论」与「学生端能否打开」**（09-23 R1 新增）：
  `student_loadable == false` 的条目**永远不是 `published`**。旧实现只看 `forced` 是缺陷。

## 待办语义（唯一判据）
- `DecisionItemV1::is_actionable()`（`schema/recognition_v1.rs`）= `resolution != AutoFixed
  && status ∈ {Open, Failed}`；`build_view` 与调度器 `actionableCount` **都必须用它**。
- 「仍生效的自动修正」= `resolution == AutoFixed && status == Accepted`
  （只排除 `Undone` 会漏 `Superseded`，导致失效规则对 AutoFixed 项完全无效）。
- `Failed` 的 `severity` **不**强制为 Blocker：severity 表达「是否阻断发布」，Failed 表达「自动写入是否成功」。

## 内容变更事务与基线保护
- **决策状态必须与题稿同一事务**：`library::repository::apply_editor_commands_tx_with` 的
  `on_content_committed(&Connection, i64)` 在 `commit()` 前执行，返回 Err 整体回滚。
  事务外另开连接写状态会留下「稿已改、状态未写」窗口，重试被前提复核误判「用户改过」，重启也不能自愈。
- 回调类型是 `&dyn Fn`：**不能在里面改外部 `Vec`**；先算好待落库项，内存副本等写入成功后再更新。
- 基线可信唯一来源 `engine::frozen_snapshot_is_reusable`。`local_baseline_frozen == false`
  ⇒ `auto_apply_candidates` 必空（快照缺失时基线退化为「现场重投影」，守卫恒等 = 不存在）。
  原因码 `BASELINE_NOT_FROZEN`：项留在 `NeedsReview`，**不静默丢弃**。
- **版本读取不得折成 0**：`0` 是合法版本；`Ok(None)`（行不存在）与 `Err`（查询失败）都是「不可用」。

## 云端修复写入入口（权威稿的唯一机器写入面）
- 唯一入口 `cloud_repair::tools::apply_cloud_edits`：模型只能「提交一批领域命令 + 说明依据」，
  不能执行代码、改源码、直接写导出 JS、碰质量/审计/来源字段、标问题已解决。
- **`EditOrigin` 由调用入口决定**，绝不从模型输出/请求体推断。`writes_human_protection()` 只对
  `Human|Undo` 真；`enforces_protection()` 只对带 run 的 `CloudRepair` 真。
- `MODEL_ALLOWED_OPS` 14 个（有意不含 `resolveIssue`/`bindSource`/`deleteAnswerSlot`）；
  `FORBIDDEN_COMMAND_KEYS` **剥离而非报错**（报错只白烧一个模型回合）并回报 `stripped_keys`。
- `requestId` 由 `run/round/toolCall` 后端派生，**不让模型提供**。
- 保护模型**故意不过度锁定**：`setAnswer` 影响范围 = 该槽位 + `answerKey:<slotId>`，**不含同组其它槽位**。
  人工保护存 `protected_edits_json`（schema v5）。
- 撤销**逐目标**（首个 before + 最后 after），本轮之外写过的目标一律跳过，**绝不整卷回滚**；
  journal 被裁剪的轮次返回 `EDIT_REPAIR_UNDO_UNAVAILABLE`，不给点了没用的假按钮。
- 事务顺序：`prepare_ds`（质量重算 + schema 校验）→ 写入 → journal → `merge_protected_edits`（仅 Human/Undo）
  → `on_content_committed`。校验必须在**克隆**上跑完再回写，失败时权威稿一字不改。

### 两个必须记住的坑
- **`answerKey:<slotId>` 不能用 `replace_object_by_id` 回填**：它是「以 slotId 为键的对象条目」，
  条目内无任何身份字段，按 id 永远找不到 ⇒ 槽位放回去了、答案值还是模型写的，界面报「已撤销」而内容没回来。
  回填统一走 `write_change_value`（自己分派该前缀），记账也要单独记一条。
- **硬失败基线必须自洽**：用同一套质量管线在**未施加本次编辑的 clone** 上重算，
  **不要读库里的 `quality.hardFailures`**（可能不同步，一批无关旧差异就能顶掉一次有效修复）。

## 测试里做故障注入
- **不要在产品代码加测试专用开关**。用 SQLite 触发器：
  `CREATE TRIGGER ... BEFORE UPDATE ON recognition_decisions_v1 BEGIN SELECT RAISE(ABORT,'...'); END;`
  被测函数自己按 root 开连接、看到同一库文件里的触发器 ⇒ 注入点落在真实产品路径上，产品零污染。
  撤除 `DROP TRIGGER IF EXISTS`。**不能用内存库替代**（测不到跨连接的真实行为）。

## 受控模型服务场景（前端可执行）
- 样本 `fixtures/controlled-llm/reading-outline.json`、预期 `expected-decisions.json`、服务
  `scripts/controlled-llm-service.mjs`。profile 存 `<appData>/config/llm-profiles.json`；
  网关拼 `{baseUrl}/chat/completions`，明文 http 有回环/localhost/私有地址白名单。
- 样本里 `evidence.quotes[].pageIndex` **必须 ≥ 1**（`0` 判 `cloud_outline_group_quote_invalid`）。
- 测试内 stub HTTP 服务：**不要 `join` 服务线程**（accept 循环会一直等）；
  必须先按 `Content-Length` 读完请求体再回写，否则客户端收到 RST。

## 判据收敛的方法论
- 「两份 report」类缺陷的**收敛方向不能随手选**：`run_preview_e2e_core` 里「状态用静态稿、产物用诊断稿」，
  正确解法是让**产物改用状态的那份判据**并给诊断结论显式命名（`previewDiagnostics.binding=false`）；
  反过来会推翻产品决策 E8-26（诊断失败可见但不降级 `ExportReady`）。
- 动「看起来不一致」的地方前先查 `Plan With Files/task_plan.md` 的 E8-xx 与 `findings.md`。
- **复刻式测试是缺陷的藏身处**：断言里自己重建被测逻辑，被测函数坏成什么样都不红。发现一条改成调用真函数。
- CAS 判据不要用受影响行数：`execute` 返回 `Ok(0)`（未匹配）不是 `Err`，`if let Err` 会静默放过。用读回确认后置条件。
- **「文件 mtime 未变」≠「功能未实现」**。审计交付必须区分：产品改动未落地 / 已落地但零测试 / 测试写错。
- **「两边都能过」的断言等于没测**（如 `status == "published" || status == "published_forced"`）。
  写断言前先自问：这个断言在实现坏掉时会红吗？

## 本机环境坑（Windows / 本仓库）
- Bash 工具**时好时坏**：stderr 常出现 `dirname: command not found` 噪音但命令其实已生效；
  不稳定时改用 PowerShell。带绝对路径调用 `.exe`（python/node）仍可用。
- PowerShell 对 `cargo`/`git` **常返回空 stdout**：`... 2>&1 | Out-File -FilePath <abs> -Encoding utf8` 再用 Read。
- **不要用 `ConvertFrom-Json` 校验 JSON**（UTF-8 乱码 + 假阳性语法错误）；用 `python -c` 或 Node `JSON.parse`。
- `$LASTEXITCODE` 在管道给 `Select-Object` 后会被重置，**不要据它判断成败**。
- 文件删除走回收站包装器：可能报 `SAFE_DELETE_FAIL_CLOSED` 而**实际已删除**；另一种形态是命令整体
  exit 1 且什么都没删（沙箱直接拒绝，连 `Out-File` 都不落地）。以删除后目录列举为准。
- PowerShell 读 git 输出把中文显示成 GBK 乱码，只是**显示**问题；前置
  `[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()`。
- **构建瞬时锁三连**：`LNK1104` / 构建脚本 `os error 32` / rustc ICE（query stack 为空）。
  反复失败时先查**是不是有旧应用实例在跑**：cargo 把 `target/debug/<crate>.exe` 与
  `target/debug/deps/<crate_>.exe` 做**硬链接**，一个实例同时锁两个路径（还锁 `pdfium.dll`）。
  `Get-Process | ? { $_.Path -like "*<repo>*" }`；进程名可能与 crate 名不同。重试无效的那种先关进程（**先问用户**）。

## git 仓库损坏恢复（09-16 实战）
症状：`.git` 在但 `git status` 报 `fatal: not a git repository`。根因链与修法：
1. `.git/refs` 缺失 → 建 `refs`、`refs/heads`、`refs/tags` 空目录。
2. 孤儿 pack 索引（只有 `.idx` 无 `.pack`）→ 删除它们与 `multi-pack-index`。
3. 对象库大面积缺失 → `git fetch --refetch origin`。
4. 远端跟踪引用写不进去 → **嵌套目录不存在时 git 不自动创建**；先 `New-Item refs/remotes/origin` 再写引用文件。
5. 旧 `index` 引用已永久丢失的对象 → 移走旧 index，`git read-tree HEAD` 重建（暂存状态会丢失）。
健康判据：`fsck --connectivity-only` missing=0、`show-ref` 同时有本地与远端 main 引用。

## 网络 / Git（代理）
- 本机代理 `http://127.0.0.1:7897`，拉大包常 `RPC failed; curl 56 ... early EOF`。修法**两条都要**：
  `git config --local http.proxy ""`（+ `https.proxy ""`）**且** `http.version HTTP/1.1`
  （只绕代理仍失败，根因是 HTTP/2 over schannel 传大包被掐断），配合 `http.postBuffer 1048576000`。
- fetch 中断会清空 `.git/refs/remotes/origin/`；对象其实已落地，从 `.git/FETCH_HEAD` 取 sha 重建引用。
