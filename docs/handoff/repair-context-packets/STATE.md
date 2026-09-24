# STATE — 校核链路上下文管理

> 唯一进度真源。每个 agent 开工先读、收工前更新并提交。只追加「日志」，阶段勾选按事实改。
> 状态取值：`todo` / `doing` / `done` / `blocked(原因)`。`done` 必须附提交 hash 与证据等级。

## 阶段

| 阶段 | 内容 | 状态 | 提交 / 证据 |
|---|---|---|---|
| P0 | 基线：环境探测、全量测试数字、核对 TASK.md §3 行号 | done | worktree `/tmp/pdf2test-baseline-34dfadd` @ `f13c75a`（已清理，重建方法见下）；数字见「基线数字」 |
| P1 | `packets.rs` 切分 + 范围 + 包内容（TASK §4.1，测试 1-4） | done | `fe15917`（`85f5064` 修升级阶梯）。命令处理器层，`cargo test --lib cloud_repair::` 89 passed |
| P2 | `grab.rs` 抓取工具 + `report_insufficient_context` + 升级阶梯（§4.2，测试 6-7） | done | `fe15917`。同上 |
| P3 | 编排改造 + prompt + 请求体去整份 PDF（§4.3/4.4，测试 5、8、9） | done | `fe15917`、`85f5064`。含真实 HTTP 集成用例 `packets_mode_requests_carry_no_whole_pdf_...` |
| P4 | 可观测性 + 受控模型剧本 + 真实 HTTP 集成测试 + 两模式对比（§4.5/§6/§7） | done | `ee524f2`（§4.5 可观测性 + 文案）；`38ec89b`（§6 受控剧本 + 两模式对比）；`P6` 补 §7 的题面类剧本分支与 CDP 步骤（见 A-4 / A-5）。**CDP 步骤已写入但本机未执行**（macOS 无 CDP 通道，如实记） |
| P5 | 独立审计 #1（子代理，只读，对抗式） | done | 见「审计发现」；3 个只读子代理 + 3 处突变检查 + 逐条源码复核 |
| P6 | 修复审计 #1 发现 | done | A-1 `b821ae2`；A-3/A-12 `71814c2`；A-6/A-7/A-13 `3e4a5dd`；A-8/A-9/A-10/A-11/A-14/A-15/A-16 `c617f7a`；A-4/A-5 见本条日志。**A-2 未修**（越界，见「越界需求」1） |
| P7 | 最终验收 + 独立审计 #2 + 修复 | todo | |
| P8 | 收口报告 `REPORT.md` | todo | |

## 基线数字（P0 填）

- **平台**：macOS（darwin，aarch64-apple-darwin）。Rust 工具链在 `~/.cargo/bin`（需显式 export PATH）；node 22.22.2；前端依赖已在 `node_modules/.bin`。
- **基线提交不是 `34dfadd`**：`34dfadd` 在 macOS 上**根本无法编译** —— `src-tauri/src/parser.rs:2184` `E0425: cannot find value '_asset_dir'`，只出现在 macOS 的 sips 渲染分支，仓库此前只在 Windows 构建过。本分支第一个提交 `f13c75a`（一行改动，把 `_asset_dir` 改回真实存在的 `asset_dir`）修掉了它，**无任何行为变更**。因此基线数字取 `f13c75a`。
  - 复现方式（worktree 已用完清理，需要时重建）：`git worktree add /tmp/base f13c75a`，把主工作区的 `src-tauri/lib`（pdfium，gitignore 里，worktree 拿不到）和 `node_modules` 软链进去，再 `cd /tmp/base/src-tauri && CARGO_TARGET_DIR=<主 target> cargo test --lib`（共享 target 可复用依赖产物，整轮约 1 分钟）。
- Rust `cargo test --lib`：基线（`f13c75a`）**1046 passed / 0 failed / 11 ignored** → 本分支 **1095 passed / 0 failed / 11 ignored**（净 +49）
- Vitest：基线 **391 passed**（28 个测试文件里 1 个加载失败）→ 本分支 **392 passed**（同样 1 个文件加载失败；净 +1）
- tsc：基线 `exit 0` → 本分支 `exit 0`
- 已知基线失败（**非本任务引入，两提交一致**）：
  1. `scripts/e2e/lib/tauri-harness.mjs` 加载失败：`Failed to load url selenium-webdriver` —— 本机 `node_modules` 里没有 `selenium-webdriver`。属于环境缺依赖。
  2. CDP 通道只在 Windows（WebView2）可用，本机 macOS 跑不了 `scripts/e2e/tauri-cdp-*.mjs`。
- **§6 两模式输入量对比（本机实测，`--nocapture`）**：同一份作业、同一条真实 HTTP 网关、同一份 212 KB 真实 PDF 样本 ——
  - legacy 总 `requestBytes` = **917173**（每轮附整份 PDF base64）
  - packets 总 `requestBytes` = **83738**（≈ 9.1%，**缩小 10.95 倍**）
  - packets 逐包 `estimatedInputTokens` 合计 = **12956**；legacy 侧该字段为 0（没有「包」这个概念）
  - 用例：`cloud_repair::tests::packets_mode_sends_much_less_input_than_legacy_for_the_same_paper`
  - 与 P4 记录（83531 / 12908）的差异**来自 A-8 的修复**：`paperMap` 每页多了一行 `paragraphLabels`（+207 字节 / +48 token）。legacy 侧一字未动，缩小倍数从 10.98 降到 10.95。

## 审计发现（P5/P7 填，逐条带状态）

> P5 方法：3 个只读子代理分别查「边界与正确性」「上下文质量」「测试可信度」，每条发现都要求 `file:line` + 可复现失败场景。汇总时**逐条回源码复核**，能复现的才写下来；不能复现的标「未证实」并写明原因。另做 3 处临时突变检查（见 M1-M3）。
> 严重级别口径：**P0** = 触碰 §2 不可动摇边界，或产生「假完成 / 假核对」的对外结论；**P1** = 产品行为可观测地错误或退化，或阻断验收；**P2** = 质量 / 覆盖 / 可维护性。

### P6 处置（每条发现的状态）

| # | 级别 | 状态 | 提交 / 证据 |
|---|---|---|---|
| A-1 | P0 | **fixed** | `b821ae2`。先写失败断言（`adjudicated_count` 期望 0、实测 1），再修 `effective_adjudicated_count` |
| A-2 | P1 | **未修（越界）** | 不是本分支引入的回归；修它要动 `tools.rs`（§8 不允许）而 §2 又禁止改 `tools.rs` 的校验逻辑。证据见该条，需求见「越界需求」1 |
| A-3 | P1 | **fixed** | `71814c2`。测试先对旧行为跑红（`left: 0 / right: 2`，临时还原旧行为验证过用例有效） |
| A-4 | P1 | **fixed** | `scripts/controlled-llm-service.mjs` 补「题面类」分支。**已用真实 HTTP 逐分支验证 10/10**（命令与输出见本条日志） |
| A-5 | P1 | **已写入，本机未执行** | `scripts/e2e/tauri-cdp-cloud-repair-chain.mjs` 新增步骤 11b `packet-mode-asked-for-the-missing-page`。macOS 无 CDP 通道，如实记「未执行」 |
| A-6 | P2 | **fixed** | `3e4a5dd`。测试先红（`[2, 3, 4]` vs `[2, 3]`） |
| A-7 | P2 | **fixed** | `3e4a5dd`。测试先红（`[(1,true),(3,true)]` vs `[(1,true),(2,false),(3,true)]`） |
| A-8 | P2 | **fixed** | `c617f7a`。`paperMap` 每页补 `paragraphLabels`，判据与 `read_passage` 对齐 |
| A-9 | P2 | **fixed** | `c617f7a`。丢光仍超预算时 `budgetNote` 明说「still not enough」 |
| A-10 | P2 | **不改（写明口径）** | `c617f7a`。改常量会同时移动 §6 基线，收益只是更早丢图；改为把「乐观下界」写进常量注释 |
| A-11 | P2 | **fixed** | `c617f7a`。补注释 + 用例固定「文档包读不到稿」是**有意**的 |
| A-12 | P2 | **fixed** | `71814c2`。测试先对旧行为跑红（临时探针已还原、未提交） |
| A-13 | P2 | **fixed** | `3e4a5dd`。测试先红（`88x180` vs `88x88`）。`bottom-left` 那半条仍为未证实 |
| A-14 | P2 | **fixed** | `c617f7a`。两处恒真断言分别改为「出口说明 + 不许猜」与「先断言非空」 |
| A-15 | P2 | **fixed** | `c617f7a`。新增走真实 HTTP 网关的用例，断言 `regions` 非空且带图 |
| A-16 | P2 | **fixed** | `c617f7a`。`enforce_packet_budget` / L2 / L3 由 A-3 / A-12 用例覆盖；`strip_packet_image_paths` 由新集成用例覆盖（突变检查 M4 确认用例有效） |

**A-2 的证据（为什么标「未修」而不是「不成立」）**：这条发现**成立**，只是动不了。
- `tools.rs:271-303 validate_evidence` 只校验结构（`sourceFileId` 非空、`pageIndex >= 1`、`quote` 非空），函数自己的注释就写明「不校验内容正确性」。
- `tools.rs:309-403 apply_cloud_edits` 写库前只做四件事：`sanitize_commands` → `validate_evidence` → 人工保护目标预检 → 事务内 `refresh_quality_report_for_targets` + `validate_authoring` + 阻断诊断增量比对（`tools.rs:381-401`）。**全流程没有任何一处把 `quote` 与原文文本层比对。**
- 仓库里唯一做这件事的是 `llm_suggestions.rs:887 llm_suggestion_quote_mismatches`，它服务 outline 建议链，**不在云端修复的写入路径上**。
- 结论：`apply_edits` 带一条凭空编造的 `quote` 也会被判 `Applied`。这是 §2 的描述（「校验仍对照完整原文」）与实现的差距，**不是本分支引入的回归**；修它必须改 `tools.rs`，与 §2「不改 `tools.rs` 的校验与写入逻辑」直接冲突。按纪律不动手，写进「越界需求」等指示。

### P0

**A-1｜「上下文不足」被算进「已了结」，对外结论与 §2 边界相反。**
- 位置：`src-tauri/src/cloud_repair/mod.rs:608-615`（`effective_adjudicated_count`）、`:3105-3111` / `:2441` / `:2541` / `:2665` / `:2816`（写进报告）、前端 `src/api/recognitionClient.ts:392-395`。
- 事实：`effective_adjudicated_count` 只要求 `fresh_ruling_for_difference(...).is_some()`，**不看裁定值**。而 L4 由后端代记的裁定（`mod.rs:3495-3535`）是 `ruling = cannot_resolve`、`reason = CONTEXT_INSUFFICIENT`。于是同一批差异会同时：① 出现在 `remaining_tasks` 里且带 `contextInsufficient: true`（`mod.rs:2112-2116`，文案「云端没能拿到足够的原文来判断第 N 题，请对照原文确认」）；② 被计入 `adjudicatedCount`，前端渲染成「已了结 N 处差异」。
- 复现（**已实测**）：在 `tests.rs:3677 a_packet_that_never_gets_enough_context_hands_the_difference_to_the_user_honestly` 末尾临时加 `assert_eq!(report.adjudicated_count, 0)` → 变红，`left: 1 / right: 0`，即「一条云端从未拿到材料的差异被算成已了结」。临时断言已删除，未提交。
- 为什么是 P0：TASK §2 明写「三态不坍缩……上下文不足不得算作已核对」。现在用户看到的是「已了结 1 处差异」，而那一处恰恰是云端**根本没拿到材料**的。
- 归属：`mod.rs` 在 §8 允许清单内 → P6 直接修（`effective_adjudicated_count` 排除 `reason == CONTEXT_INSUFFICIENT`）。

### P1

**A-2｜证据引文从不与原文比对，§2「校验仍对照完整原文」这句话没有实现。**
- 位置：`src-tauri/src/cloud_repair/tools.rs:271-303`（`validate_evidence`）。函数自己的注释也写明「只校验**结构**……不校验内容正确性」。
- 事实：`apply_cloud_edits`（`tools.rs:309-403`）在写库前只做四件事 —— `sanitize_commands`、`validate_evidence`（结构）、人工保护目标预检、事务内质量重算 + `validate_authoring`（schema / ID / 引用闭合）。**全流程没有任何一处把 `quote` 与原文文本层比对**。仓库里唯一做这件事的函数是 `llm_suggestions.rs:887 llm_suggestion_quote_mismatches`，它服务的是 outline 建议链，**不在云端修复的写入路径上**。因此 `apply_edits` 带一条凭空编造的 `quote`（只要 `sourceFileId` 非空、`pageIndex >= 1`、`quote` 非空）也会被判 `Applied`。
- 归属：`tools.rs` 在 §8 **不允许**改（§2 也明写「不改 tools.rs 的校验与写入逻辑」）→ 见「越界需求」。**不是本分支引入的回归**，但任务书 §2 的描述与实现不符，报告里必须如实说明。

**A-3｜L2 刚补的整页图可能被紧随其后的预算裁剪清掉，而 `escalationNote` 仍声称「已附」。**
- 位置：`src-tauri/src/cloud_repair/mod.rs:3462-3472`（L2 分支写入 `regions` 并写 `scopeManifest.escalationNote = "L2: the backend attached whole-page images for every page in scope."`）→ `:3486` 立刻 `enforce_packet_budget(packet)` → `:3303-3327` 超预算时 `regions.clear()`。
- 事实：L2 附整页图后包体几乎必然超过 24k（`PACKET_IMAGE_TOKENS = 1200` / 张，`packets.rs:39`），`enforce_packet_budget` 会把**包括刚补的**整页图全部清掉，同时留下 `budgetNote`。于是包里同时存在「L2 已附整页图」和「N 张区域图被丢」两句互相矛盾的话，而模型看到的是一张图都没有。
- 复现：**未实测**（源码级判定）。需要构造 scope ≥ 6 页的包（6 张整页图 = 7200 token，加正文易过 24000），断言 `packet["sourceEvidence"]["regions"]` 非空即红。本机没有现成的 ≥ 6 页 packet fixture，P6 补用例时一并确认。
- 归属：`mod.rs` 在 §8 允许清单内 → P6 修（L2 之后不要立刻按同一预算裁剪，或裁剪时改写成诚实的 `escalationNote`）。

**A-4｜受控服务的「包模式」剧本只覆盖答案类修复，CDP 剧本是题面类修复 → 一旦跑 Windows CDP 很可能直接失败。**
- 位置：`scripts/controlled-llm-service.mjs:530-590`（`repairPacketStepReply`，判据是 `answerLineFromPacket`，只认 `^<题号>\s+(.+)$` 这种答案行）；对照 `scripts/e2e/lib/cloud-repair-scenario.mjs:225-249`（CDP 剧本是 `setResponseGroup` 改写 prompt / instructions，`plan` 里只有 `fixSlotIds` / `questionNumber` / `sourcePageOneBased` / `rulings` / `unresolved`，**没有**答案行）。
- 事实：包模式已是默认（`REPAIR_CONTEXT_MODE = Packets`），而 e2e 里没有任何地方设 `IELTS_REPAIR_CONTEXT_MODE`。所以 CDP 场景一跑，修复请求的 `context.contextMode` 就是 `packets`，会被路由到 `repairPacketStepReply`；它找不到答案行 → 每轮 `report_insufficient_context` → 升级到 L4 → 后端代记 `cannot_resolve` → 题面**根本没被改** → `cloud-fixed-content-on-its-own` 断言必然红。
- 影响：§7 要求的「CDP 链保持 13/13」在补齐之前不可能成立。这是 P6 的前置项，不是可选优化。
- 归属：`scripts/controlled-llm-service.mjs` 在 §8 允许清单内 → P6 补齐「题面类」修复分支（从包的 `sourceEvidence` 行文本取正确题面 + `setResponseGroup` 改写，与 legacy 剧本同源）。
- 本机状态：**未验证**（macOS 无 CDP 通道）。

**A-5｜§7 要求的 CDP 新步骤尚未写入。**
- 位置：`scripts/e2e/tauri-cdp-cloud-repair-chain.mjs`（`git diff 34dfadd..HEAD -- scripts/` 为空 → 本分支**一个字都没改**）。
- 事实：§7 要求「必须保持 13/13，并新增一步断言至少一个包走了 L1 且最终稿正确」。现在既没有新步骤，也没有在 Windows 上验证过 13/13。
- 归属：P6 补步骤；P7 在 Windows 上跑并如实记录。本机如实记「未执行：平台不支持 CDP 通道」。

### P2

**A-6｜`expand_edge_pages` 无上界，会把不存在的页号写进 `scope.pages`。**
- 位置：`src-tauri/src/cloud_repair/packets.rs:503-526`，第 523 行 `out.insert(anchor.page + 1);` 没有与总页数比较（上一行 `anchor.page - 1` 有 `> 1` 保护）。
- 事实：锚点贴最后一页页边时，`scope.pages` 会含 `最后一页 + 1`；该页在 `source.page_images` / 文本层里都不存在。`scopeManifest` 于是声明了一个取不到的页，模型按它去要只会拿到「不存在」。
- 归属：`packets.rs` 在 §8 允许清单内 → P6 修（按 `source` 的最大页号收敛）。

**A-7｜`region_requests` 按**整组**判断有没有 bbox，一页缺 bbox 会让整组的缺页都拿不到区域图。**
- 位置：`src-tauri/src/cloud_repair/packets.rs:1221-1249`：`collect_bboxes(group, &mut bboxes)` 收集整组，`:1235` 是 `if bboxes.is_empty()` —— 整组级判据。
- 事实：某题组 3 页里 2 页有 bbox、1 页没有 → `bboxes` 非空 → 那一页既没有区域图、也没有退化成整页图（退整页图的分支只在**整组都没有** bbox 时走）。
- 归属：P6 修（判据下沉到「每页有没有 bbox」）。

**A-8｜`paper_map` / 包内容不给段落标号，模型只能靠题号去取段落。**
- 位置：`packets.rs:583-657`（`paper_map` 的每页标签只有 `q<n>` / `answers` / `passage`）、`:1290-1331`（`paragraph_windows` 返回 `{id, text}`，用的是段落 **id** 不是标号）。
- 事实：`read_passage`（`grab.rs:645-658`）收的是 `paragraphLabels`（"A"/"B"）或 `questionNumbers`。题号那条路可用，所以不是死路；但「这一页有哪几个段落标号」在包里无从得知。
- 归属：P6 修（`paper_map` 每页补段落标号，或 `paragraph_windows` 带 label）。

**A-9｜`enforce_packet_budget` 只会丢区域图；单题组超预算时既不拆也不裁文字。**
- 位置：`mod.rs:3303-3327`（只 `regions.clear()`）；`packets.rs:825` `if size <= PACKET_TOKEN_BUDGET || draft.task_ids.len() <= 1 { split.push(draft); continue; }` —— 单题组**永不拆分**。
- 事实：一个内容极多的单题组包会带着超过 24k 的正文发出去，「单包上限 24k」这条约束在最坏情况下不成立。相对整份 PDF 仍小得多，属退化不属错误。
- 归属：P6 评估（可记录 `budgetNote` 说明未再退让，而不是静默超限）。

**A-10｜`PACKET_CHARS_PER_TOKEN = 4` 对中文严重低估。**
- 位置：`packets.rs:37`、`:579`（`chars / PACKET_CHARS_PER_TOKEN + images * PACKET_IMAGE_TOKENS`）。
- 事实：中文大致 1 token ≈ 1–1.5 字符，`chars/4` 会低估 3–4 倍。题面/说明里中文不少，`estimatedInputTokens` 因此偏乐观，`enforce_packet_budget` 的触发点比真实值晚。
- 归属：P6 评估（至少把口径写清；不影响正确性）。

**A-11｜文档包 `task_ids` / `question_numbers` 全空 ⇒ 该包内任何 `read_draft` 必被拒。**
- 位置：`mod.rs:1320-1340`（`scope_error`）：空选择器 → `CLOUD_DRAFT_SCOPE_REQUIRED`；有选择器但不在包内 → `CLOUD_DRAFT_OUTSIDE_PACKET`。文档包由 `packets.rs:811-819` 造出，`task_ids` 为空。
- 事实：文档包只带索引与诊断（`differences` / `blocking_issues` 都空），所以实际影响有限；但「文档包永远读不到稿」这件事没有任何注释或测试说明是**有意**的。
- 归属：P6 补注释或补测试固定该行为。

**A-12｜第二个包要 L3 时静默跳过，但级别仍记成 3。**
- 位置：`mod.rs:3474-3482`（`if !*used_full_source { ... }`，`else` 什么都不做、**不写** `escalationNote`）→ `:3485` `packet["escalationLevel"] = json!(next);` 无条件设成 3。
- 事实：诊断里出现 `escalationLevel = 3` 的包，但它既没有 `attachFullSource`，也没有任何说明。§4.2「每次运行最多 1 次」是对的，错的只是**把没发生的事记成发生了**。
- 归属：P6 修（跳过时不抬级别，或写一句诚实的 `escalationNote`）。

**A-13｜`grab.rs` 区域裁剪把像素 `top` 与页单位 `height` 混算，裁剪高度比意图大约 1.4 倍。**
- 位置：`src-tauri/src/cloud_repair/grab.rs:588-591`。`:588` 先把页单位的 `top` 换算成像素（`let top = ((top - height - margin_y) * scale_y)...`），`:589` 又用**已是像素**的 `top` 加**页单位**的 `height * 3.0 + margin_y` 再乘 `scale_y`。
- 事实：`top-left` 原点下，意图是 `(y-1.1h)` → `(y+1.1h)`（页单位高 2.2h），实得 `(y-1.1h)` → `(y+2h)`（高 3.1h），即**多给约 0.9h**。方向是「多给」不是「裁掉」，且末尾 `.min(pixel_height)` 会夹到页底，所以不会崩、也不会少给。**注意**：子代理报的「约 3 倍」不准确，按源码重算约 1.4 倍。
- 未证实部分：`bottom_left` 分支（`:563-564` `let top = if bottom_left { y + height } else { y }`）把「离底距离」当「离顶距离」用，对 `bottom-left` 锚点会裁到镜像位置。但 `bbox` 由 `pdf_ingest/coordinates.rs:137-155 display_rect` 产出，`origin` 是 `"top-left"`；`bottom-left` 只出现在 `nativeBBox`（`:125-135`），而 `collect_bboxes` / `crop_page_image` 只读 `bbox`。**未找到能真正走到 `bottom_left` 分支的输入**，故该半条标「未证实」。
- 归属：P6 修 `:589` 的算式（用页单位算完再一次性乘 `scale_y`）。

**A-14｜测试断言恒真 / 空集合上恒真。**
- `tests.rs:4043-4046`：`assert!(seen[0].contains("report_insufficient_context"))`。该字符串来自 prompt 里**无条件**拼接的工具清单（`llm_gateway.rs:1651-1653`），所以无论 prompt 有没有真的解释这条出口，断言都成立 —— 它证明不了任何事。
- `tests.rs:3460-3463`：`assert!(included.iter().all(|page| scope.contains(page)))`。`included` 为空时**空真**，而用例没有先断言 `!included.is_empty()`，所以「包里一页证据都没有」也能过。
- 归属：P6 加固（断言工具清单里同时出现**信封键**，或改成对包内页数下限的断言）。

**A-15｜§5.1 / §5.5 的覆盖缺口。**
- §5.1 要求「`complex-reading` canonical + 构造候选 → 包数与每包题组符合规则 2」：**没有**任何包模式用例用 `complex-reading`（`tests.rs:1056-1204` 用它的那条是 legacy 用例）。包模式全部用例走 `seed_packet_job` 手写的 3 页 `document-ir.json`。
- §5.5 要求断言「只有范围内页文本与**区域图**」：`tests.rs:3996-4105` 断言了「无 `application/pdf`」与「取回的页文本真的到了下一轮」，但**没有**断言 `sourceEvidence.regions` 非空；也**没有**任何 L3（`attachFullSource`）用例 —— 全仓库只有 `mod.rs:3477` 与 `llm_gateway.rs:1733` 两处实现引用，测试零引用。
- 归属：P6 补用例。

**A-16｜以下实现点被改坏不会有任何测试变红（源码级核对：全仓库零测试引用）。**
- `mod.rs:3303 enforce_packet_budget`、`:3428 escalate_packet`（L2/L3 分支体）、`llm_gateway.rs:1693 strip_packet_image_paths`、`mod.rs:3330 merge_fetched_evidence` 的 `page_region` 分支。
- 依据：`Grep PACKET_TOKEN_BUDGET|budgetNote|strip_packet_image_paths|enforce_packet_budget|escalate_packet` 在 `tests.rs` 里零命中（`PACKET_TOKEN_BUDGET` 只在 `packets.rs:825` 生产代码里出现）。
- 归属：P6 至少给 `enforce_packet_budget` 与 L2 各补一条用例（与 A-3 同一处）。

### 突变检查（测试有效性证据）

临时改坏 → 确认变红 → 还原（`git diff --stat` 已确认清空，`cloud_repair::` 回到 89 passed）：

| # | 改坏的点 | 变红的用例 | 结果 |
|---|---|---|---|
| M1 | `packets.rs` `page: zero_based as u32 + 1` → `zero_based as u32` | `packets::tests::anchor_pages_are_reported_one_based_and_stay_inside_the_scope`、`tests::packet_source_lines_carry_one_based_page_ids_matching_the_text_layer` | 2 条变红，已还原 |
| M2 | `mod.rs` `"reason": CLOUD_RULING_REASON_CONTEXT_INSUFFICIENT` → `"OTHER_REASON"` | `tests::a_packet_that_never_gets_enough_context_hands_the_difference_to_the_user_honestly` | 1 条变红，已还原 |
| M3 | `grab.rs` 删掉 `page_from.is_none() && quote.is_none()` 的拒绝 | `grab::tests::read_source_without_a_selector_is_rejected_with_a_specific_reason`、`tests::grab_tools_reject_out_of_bounds_requests_with_specific_reasons` | 2 条变红，已还原 |
| M4 | `llm_gateway.rs` 停掉 `strip_packet_image_paths(&mut prompt_input)` | `tests::packets_mode_attaches_the_region_image_and_keeps_local_paths_out_of_the_prompt` | 1 条变红（`imageAttached` 缺失），已还原 |

P6 另有两处「对着旧行为跑红」的验证（不是独立突变，而是修 A-3 / A-12 时临时还原旧实现）：A-3 探针 `left: 0 / right: 2`、A-12 探针打印出 `TEMP-A12-PROBE-L2` 并变红，两处均已还原、**未提交**。

**未证实（写了原因，不算发现）**：

- **`packetId` 不含内容指纹，重切后可能命中 `done_packets` 而被跳过**。`packets.rs:852-894` 的 id 只由「本地题组 + 差异键 + 阻断问题 id + 是否文档包」派生，注释也明说「内容变了才换 id」，但差异键 `(targetType, targetId, field)` 在值变化时**不变**。构造「键不变而值变、且该包已在 `done_packets` 里」的场景需要一次真实 `apply_edits` 造成候选侧值变化 —— 试过一轮没能稳定复现，**未证实**。
- **`anchor_pages_of(input.candidate, &draft.task_ids)` 按本地 taskId 匹配候选锚点会漏**（`packets.rs:1028`）。反证：`reconcile/candidate.rs:4815` 的断言「14-15 必须接到权威稿题组」说明归一化后的候选题组 `task_id` **就是**本地 id；临时探针也打印出 `candidateSlice.taskGroups[].taskId = Some("early-approaches-q15-15"→本地 id)`。**未证实**（在本仓库的归一化下不成立）。
- **`crop_page_image` 的 `bottom-left` 分支可达**（见 A-13 的「未证实部分」）。

**P6 对以上三条的复核结论（都不是发现）**：
- 第 1 条（`packetId` 不含内容指纹）**仍为未证实**：差异键 `(targetType, targetId, field)` 在值变化时不变是**读源码就能确认**的事实，但「因此会被 `done_packets` 跳过」需要一次真实 `apply_edits` 造成候选侧值变化并命中 `done_packets`——P5 试过一轮没稳定复现，P6 也未复现，因此不写成发现。**注意**：这条如果成立，后果是「重切后漏核」，属于 P0 级；它没有被证伪，只是没有被证实。留待 P7 用真实模型跑一轮时重点观察。
- 第 2 条（候选锚点按本地 taskId 匹配会漏）**已证伪**：`reconcile/candidate.rs:4815` 的断言与临时探针都表明归一化后的候选题组 `task_id` 就是本地 id。不成立，不修。
- 第 3 条（`bottom-left` 分支可达）**仍为未证实**：`bbox` 由 `pdf_ingest/coordinates.rs:137-155 display_rect` 产出、`origin` 恒为 `"top-left"`，`bottom-left` 只出现在 `nativeBBox`，而 `collect_bboxes` / `crop_page_image` 只读 `bbox`。P6 没找到能走到该分支的输入，因此按 A-13 只修了**确定可达**的算式错误。

## 越界需求（需要改非授权文件时写在这里，不要直接改）

1. **`src-tauri/src/cloud_repair/tools.rs`（§8 未授权）** —— 修 A-2 需要在这里把 evidence 的 `quote` 与**完整原文文本层**比对（`validate_evidence` 目前只校验结构）。§2 同时明写「不改 `tools.rs` 的校验与写入逻辑」，两条要求互相矛盾：**要么**放宽 §2 的那句「不改」，**要么**承认「引文校验只到结构层」并把 §2 的描述改准。P5 未动手，P6 同样未动手（证据见「P6 处置」A-2 条），等指示。
2. **`src/api/recognitionClient.ts` / `src/features/editor/recognitionDecisions.ts`（§8 未授权）** —— 若 A-1 的修法选择在**前端**把「已了结」与「上下文不足」分开计数，需要动这两个文件。P5 建议改后端（`mod.rs` 在授权内），P6 按此执行，因此**不需要**越界。
3. **`scripts/e2e/tauri-cdp-cloud-repair-chain.mjs` 的调用记录映射（超出「仅新增步骤」）** —— §8 允许该文件「仅新增步骤」，但新增的步骤 11b 需要读 `llm-calls.jsonl` 的 `packetId` / `escalationLevel` / `pagesIncluded` / `imageCount` / `estimatedInputTokens` / `requestBytes`，而原来的映射只保留 `{commandName, ok, errorClass, latencyMs}`。改动是**只加字段、不删不改**既有四个字段（`report.modelTraces.llm.callRecords` 的既有消费者不受影响）。这是新增步骤的最小前置，如认为仍越界请指示回退。
4. **A-4 的真实 HTTP 验证脚本不落仓库** —— §8 不允许在 `scripts/` 下新增文件，所以 A-4 的验证脚本放在 `/tmp/verify-packet-prompt-rewrite.mjs`（临时，未提交）。命令与 10/10 输出记在本条日志里，可据此复现。若希望它成为常驻用例，需要授权在 `scripts/` 下新增文件（或并入 `scripts/e2e/` 既有文件的某个步骤）。

## 日志（只追加：时间、agent 做了什么、提交、下一步）

- 2026-09-24 P0：`34dfadd` 在 macOS 上编译不过（`parser.rs:2184` E0425），基线改在 `f13c75a` 取。worktree `/tmp/pdf2test-baseline-34dfadd`（`CARGO_TARGET_DIR` 指向主 target 复用依赖；`lib/` 软链到主工作区）。数字：cargo 1046/0/11、vitest 391、tsc 0。下一步：P1。
- 2026-09-24 P1-P3：`fe15917`（packets.rs + grab.rs + 编排 + prompt + 去整份 PDF）、`85f5064`（修「无进展指纹不含升级级别」导致阶梯卡在 L1；修「收尾包被误报 budget_exhausted」）。`cargo test --lib` 1085/0/11。
- 2026-09-24 P4：`ee524f2`（§4.5 逐包 `packetId`/`escalationLevel`/`pagesIncluded`/`estimatedInputTokens`/`imageCount`/`requestBytes` 落 `llm-calls.jsonl`；`userTasks.ts` 把「材料没到手」与「查过但定不了」拆成两句）。`38ec89b`（§6 受控服务包模式剧本 + §6 两模式对比用例：legacy 917173 B → packets 83531 B）。**受控服务剧本已用真实 HTTP 逐分支验证**：缺答案页 → `report_insufficient_context{needs:[pages 3..3]}`；页到手 → `apply_edits{baseVersion:7, quote:"14 A"}`；差异清空 → `finish_packet`；**反自证**：页在包里但答案行不在 → 只 `finish_packet`，不编答案。
- 2026-09-24 P5：3 个只读子代理并行审计（边界与正确性 / 上下文质量 / 测试可信度）+ 3 处突变检查（M1-M3 均确认变红后还原）+ 逐条源码复核 + A-1 实测复现（临时断言变红 `left: 1 / right: 0`，已删除）。结果见「审计发现」：P0 ×1、P1 ×4、P2 ×11、未证实 ×3。下一步：P6 先修 A-1 / A-3 / A-4（A-4 是 CDP 验收的前置阻断），再补 A-5 的 CDP 步骤。
- 2026-09-24 P6：逐条处置「审计发现」。**fixed 14 条**（A-1 `b821ae2`；A-3/A-12 `71814c2`；A-6/A-7/A-13 `3e4a5dd`；A-8/A-9/A-10/A-11/A-14/A-15/A-16 `c617f7a`），**未修 1 条**（A-2：越界，见「越界需求」1，已写出源码证据），**不改但写明口径 1 条**（A-10），**已写入未执行 1 条**（A-5）。每条都先写失败测试并**看到红**（A-1/A-6/A-7/A-13 实测红；A-3/A-12 用临时还原旧行为的方式确认用例有效；M4 确认 `strip_packet_image_paths` 用例有效）。A-4 的题面类分支用**真实 HTTP** 验证：`/tmp/verify-packet-prompt-rewrite.mjs` 起 `scripts/controlled-llm-service.mjs`（`--plan` 用 CDP 剧本形状：无答案行），POST 真实 `repair_authoring_step` 请求体，**10/10 checks passed** —— 题面取自包内那一行、`sourceAnchors` 原样带回、`baseVersion` 取自包的 `editVersion`、引文逐字来自本轮请求；反自证分支（包内无题面行）只 `finish_packet` 并带上 `unresolved`；答案类差异仍走 `setAnswer`（回归）。**全量重跑**：`cargo test --lib` **1095/0/11**（基线 1046）、`vitest` **392**（基线 391，仍 1 个文件因缺 `selenium-webdriver` 加载失败）、`tsc exit 0`。§6 两模式对比重跑：legacy 917173 → packets **83738**（10.95×），差异来自 A-8（`paperMap` 多了一行 `paragraphLabels`）。下一步：P7（独立审计 #2 + Windows 上跑 CDP 13/13 → 14/14）。
