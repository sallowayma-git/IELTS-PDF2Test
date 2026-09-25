# STATE — 校核链路上下文管理

> 唯一进度真源。每个 agent 开工先读、收工前更新并提交。只追加「日志」，阶段勾选按事实改。
> 状态取值：`todo` / `doing` / `done` / `blocked(原因)`。`done` 必须附提交 hash 与证据等级。

## 阶段

| 阶段 | 内容 | 状态 | 提交 / 证据 |
|---|---|---|---|
| P0 | 基线：环境探测、全量测试数字、核对 TASK.md §3 行号 | done | 原始基线 worktree `/tmp/pdf2test-baseline-34dfadd` @ `f13c75a`；本轮已在 HEAD `24eaa98` 复核平台、三项门禁、§3 行号、受控服务和 CDP 可用性，详见本日志；数字见「基线数字」 |
| P1 | `packets.rs` 切分 + 范围 + 包内容（TASK §4.1，测试 1-4） | done | `fe15917`（`85f5064` 修升级阶梯）。命令处理器层，`cargo test --lib cloud_repair::` 89 passed |
| P2 | `grab.rs` 抓取工具 + `report_insufficient_context` + 升级阶梯（§4.2，测试 6-7） | done | `fe15917`。同上 |
| P3 | 编排改造 + prompt + 请求体去整份 PDF（§4.3/4.4，测试 5、8、9） | done | `fe15917`、`85f5064`、复核修正 `1d79409`、`a70e3e0`。命令处理器层 + 真实 HTTP；全量 Rust `1111/0/11` |
| P4 | 可观测性 + 受控模型剧本 + 真实 HTTP 集成测试 + 两模式对比（§4.5/§6/§7） | done | `ee524f2`（§4.5 可观测性 + 文案）；`38ec89b`（§6 受控剧本 + 两模式对比）；`1b76667`（真实受控 HTTP 用例补请求体无整份 PDF 与编辑后 `finish_packet` 顺序断言）。命令处理器层 + 真实 HTTP；本机 CDP 未执行（Darwin 不支持 WebView2 通道） |
| P5 | 独立审计 #1（子代理，只读，对抗式） | done | 见「审计发现」；3 个只读子代理 + 3 处突变检查 + 逐条源码复核 |
| P6 | 修复审计 #1 发现 | done | A-1 `b821ae2`；A-3/A-12 `71814c2`；A-6/A-7/A-13 `3e4a5dd`；A-8/A-9/A-10/A-11/A-14/A-15/A-16 `c617f7a`；A-4/A-5 见本条日志。**A-2 未修**（越界，见「越界需求」1） |
| P7 | 最终验收 + 独立审计 #2 + 修复 | **blocked（原因见下）** | 干净状态重跑数字见「基线数字」；审计 #2 的发现见「P7 处置」；修复提交 `391294f` / `2a5dc64` / `208afd9` / `7430a71` / `4a578c5` / `c264ada` / `84325a1`。**未标 done**：① A-2 仍是未修的 P1（越界，见「越界需求」1）；② **§7.3 在包模式下仍差一步**：A-22 修掉了「包模式剧本从不裁定」这一半，但步骤 11b 要求的「至少一个包走了 L1」在这份 CDP 场景下**结构性不可满足**——题面类修复所需的原文行就在包内（`sourceEvidence.pages[]` 来自题组锚点页），模型没有升级的理由；要让它可满足必须改场景（把修复靶子换成答案类差异，答案页不在题组锚点页里），属**设计决策**，且本机无 CDP 通道无法验证；③ CDP 链在本机跑不了（macOS 无 WebView2 CDP 通道），§7.3 与 A-5 / A-18 / A-22 的实机验证全部缺失 |
| P8 | 收口报告 `REPORT.md` | done | 本目录 `REPORT.md`；复核同步提交 `6713720`（32 提交历史快照、最新请求体对比与全量门禁数字）。**注意**：P8 完成**不代表整条链收口**——P7 仍 `blocked`（A-2 越界 P1、§7.3 步骤 11b 结构性未达成、CDP 本机不可跑、§7.4 无额度） |
| P9 | 修复重切漏核风险、恢复包模式输入量优势并复核 | done | `1df51b7`。A-25/A-26 先红后绿；`cargo test --lib` 1104/0/11；真实 HTTP 两模式对比 226697 B vs 917173 B。命令处理器层；产品 UI / CDP / 真实模型仍未验收 |
| P9-Q | 引文必须能在完整原文里找到（本轮授权改 `tools.rs::validate_evidence` 及其调用处；TASK §2 描述同步修正） | done | `b6c0370`。7 条 tools 单测 + 3 条链路用例先红后绿（临时 stub 旧行为见到红）；命令处理器层 + 真实 HTTP |
| P10 | 新增「答案类」场景证明包模式真的会抓取（L1）+ 命令处理器层真实 HTTP 等价证据；CDP 本机未执行 | done（CDP 实机未执行） | `a8e1678`。抓取路径用例先红后绿（stash 受控服务分支见红）；命令处理器层 + 真实 HTTP；CDP 链步骤已写入但本机无法实跑（macOS 无 WebView2） |
| P11 | 解决多包顺序处理总时限（二选一：包间并行 ≤2 或总时限按包数放宽，理由写进 STATE） | done | `542e660`。选方案 b（+25%/包、封顶 3×）；10 包时限用例 + 跨包写冲突用例先红后绿；命令处理器层 |
| P12 | 收尾：全量重跑对比基线、全新只读子代理复核 P9-Q/P10/P11（含 2 处突变检查）、REPORT.md 追加 | done | 全量数字见「基线数字」；复核子代理（全新、只读、对抗式）3 处变异全部变红并还原，结论放行；REPORT.md 已追加 §9 |
| P12-Q | 修复第三方复核发现的引文核验缺陷：无文本层页上的真实引文被误拒；非主试卷 sourceFileId 的核验口径；TASK §2 两行描述对齐 | done | `2faa456` + 复核发现收紧 `b1cd8f3`。缺陷 1 两条单测 + 端到端用例先红后绿；缺陷 2 选方案 b（非主试卷一律 unverifiable + prompt 说明），理由见本条日志；全新复核子代理 2 处变异变红并还原、放行；命令处理器层 + 真实 HTTP |

## 基线数字（P0 填）

- **平台**：原始 P0 记录为 macOS（darwin，aarch64-apple-darwin）、Node 22.22.2；本轮复核仍是 macOS（Darwin 22.6.0 arm64），当前 Node **v24.21.0**。Rust 工具链在 `~/.cargo/bin`；前端命令由仓库依赖运行。
- **基线提交不是 `34dfadd`**：`34dfadd` 在 macOS 上**根本无法编译** —— `src-tauri/src/parser.rs:2184` `E0425: cannot find value '_asset_dir'`，只出现在 macOS 的 sips 渲染分支，仓库此前只在 Windows 构建过。本分支第一个提交 `f13c75a`（一行改动，把 `_asset_dir` 改回真实存在的 `asset_dir`）修掉了它，**无任何行为变更**。因此基线数字取 `f13c75a`。
  - 复现方式（worktree 已用完清理，需要时重建）：`git worktree add /tmp/base f13c75a`，把主工作区的 `src-tauri/lib`（pdfium，gitignore 里，worktree 拿不到）和 `node_modules` 软链进去，再 `cd /tmp/base/src-tauri && CARGO_TARGET_DIR=<主 target> cargo test --lib`（共享 target 可复用依赖产物，整轮约 1 分钟）。
- Rust `cargo test --lib`：基线（`f13c75a`）**1046 passed / 0 failed / 11 ignored** → P7 时 **1102 passed / 0 failed / 11 ignored**（净 +56）；P9 最新复测见下文
- Vitest：基线 **391 passed / 0 failed tests**（28 个测试文件里 1 个 suite 加载失败）→ P9 **392 passed / 0 failed tests**（27 suites passed，1 个 suite 因缺 `selenium-webdriver` 加载失败；净 +1 passed）
- tsc：基线 `exit 0` → 本分支 `exit 0`
- 已知基线失败（**非本任务引入，两提交一致**）：
  1. `scripts/e2e/lib/tauri-harness.mjs` 加载失败：`Failed to load url selenium-webdriver` —— 本机 `node_modules` 里没有 `selenium-webdriver`。属于环境缺依赖。
  2. CDP 通道只在 Windows（WebView2）可用，本机 macOS 跑不了 `scripts/e2e/tauri-cdp-*.mjs`。
- **§6 P7 历史两模式输入量对比（本机实测，`--nocapture`）**：同一份作业、同一条真实 HTTP 网关、同一份 212 KB 真实 PDF 样本 ——
  - legacy 总 `requestBytes` = **917173**（每轮附整份 PDF base64）
  - packets 总 `requestBytes` = **83738**（≈ 9.1%，**缩小 10.95 倍**）
  - packets 逐包 `estimatedInputTokens` 合计 = **12956**；legacy 侧该字段为 0（没有「包」这个概念）
  - 用例：`cloud_repair::tests::packets_mode_sends_much_less_input_than_legacy_for_the_same_paper`
  - 与 P4 记录（83531 / 12908）的差异**来自 A-8 的修复**：`paperMap` 每页多了一行 `paragraphLabels`（+207 字节 / +48 token）。legacy 侧一字未动，缩小倍数从 10.98 降到 10.95。
  - **P7 重跑：数字一字未变**（917173 / 83738 / 12956）。`region_requests` 的 A-17 修复在这份样本上没有改变产出——它只在「同一个包里两个题组、其中一组在某页缺 bbox」时才生效（见 P7 处置 A-17）。
- **P7 干净状态重跑（HEAD `7430a71`）**：
  - Rust `cargo test --lib`：**1099 passed / 0 failed / 11 ignored**（P6 收尾是 1095；+4 = A-17 用例、§5.1 用例、A-19 用例、§7.2 真实受控服务用例）
  - Vitest：**392 passed**（28 个文件里 1 个加载失败，与基线一致：缺 `selenium-webdriver`）
  - tsc：`exit 0`
  - §6 两模式对比：见上一条，未变
  - §7.2 真实 HTTP + **真实受控服务**：`the_real_controlled_service_drives_the_packet_loop_through_l0_l1_and_finish` **passed**（本机真起 node 跑 `scripts/controlled-llm-service.mjs`）
  - CDP 链：`node scripts/e2e/tauri-cdp-cloud-repair-chain.mjs` → `[scenario] FAILED harness :: ENOENT ... src-tauri/target/debug/ielts-author-studio.exe`，即 **未执行**（macOS 没有 WebView2 CDP 通道）。改动前后失败形态一致，说明新改的步骤 10b / 11 没有引入脚本级错误（另做 `node --check` 通过）
- **P7 审计 #2 处置后的重跑（HEAD `84325a1`）**：
  - Rust `cargo test --lib`：**1102 passed / 0 failed / 11 ignored**（P7 上一轮是 1099；+3 = A-23 的 L2 note 用例、A-22 的两条包模式裁定用例）。A-24 不新增用例（改的是缺 node 时的行为）
  - Vitest：**392 passed**（28 个文件里 1 个加载失败，与基线一致：缺 `selenium-webdriver`）
  - tsc：`exit 0`
  - §6 两模式对比：**917173 / 83738 / 12956，一字未变**（A-23 只影响「有 region 条目但无图」这条升级路径，本样本走不到）
  - A-22 的两条用例真的起 node 跑 `scripts/controlled-llm-service.mjs`（真 HTTP）：`the_real_controlled_service_rules_on_a_ruling_type_difference_in_packet_mode`、`the_controlled_service_stops_ruling_once_it_already_has`，均 passed
  - A-24 验证方式：把 PATH 换成 `$HOME/.cargo/bin:/usr/bin:/bin`（`which node` = not found）后跑 §7.2 用例 → **失败**（`本机 PATH 里没有 node…`），修复前同样条件下报的是 `1 passed`
- **P9 最新复测（代码提交 `1df51b7`）**：
  - Rust `cargo test --lib`：**1104 passed / 0 failed / 11 ignored**；A-25 / A-26 共新增 2 个用例。
  - Vitest：**392 tests passed**，27 suites passed；1 suite 加载失败，因为本机缺 `selenium-webdriver`（`scripts/e2e/lib/tauri-harness.mjs`）。
  - `npx tsc --noEmit`：`exit 0`。
  - §6 同卷真实 HTTP 对比：legacy **917173 B**，packets **226697 B**（约 **4.05×** 缩小）；packets `estimatedInputTokens` 合计 **12480**。当前 PDF 路径附带 **420174 B** 整页 PNG；未压缩包模式曾量到 **1191137 B**，故新增近灰度图压缩。
  - A-25 旧实现先红：第二个包编辑 q14 后，重切出来的新值仍命中旧 packet ID，实际只处理 3 包而用例要求 4 包；修复后新内容包重排，未完成的 q14 留在 `remaining_tasks`，整次状态也没有误报 `completed`。
  - A-26 旧图像路径先红：packet 总请求 **1191137 B > 917173 B**；灰度 PNG 压缩后两模式对比通过。图像单测确认长边不超过 600 px、彩色页图保持原样。
  - `cargo fmt --check` 全仓不通过，原因是仓库既有格式漂移；没有运行全仓格式化以免引入无关改动。
  - 产品端到端未执行：macOS 无 WebView2，CDP 脚本依旧因缺 `ielts-author-studio.exe` 无法启动；真实模型验收仍无额度。
- **P12 收尾全量重跑（HEAD `542e660`，与 P0 基线对比）**：
  - Rust `cargo test --lib`：**1125 passed / 0 failed / 11 ignored**（P0 复核基线 1111 → 净 +14：P9-Q 的 7 条 tools 单测 + 3 条链路用例、P10 的 1 条真实 HTTP 用例、P11 的 2 条用例、prompt 契约测试 1 条）。
  - `cargo test --lib cloud_repair`：**130 passed / 0 failed**（P0 时 117）。
  - Vitest：**392 tests passed**、27 suites passed；1 suite 因缺 `selenium-webdriver` 加载失败（已知环境基线问题，不计为通过）。
  - `npx tsc --noEmit`：`exit 0`。
  - §6 两模式对比重跑：legacy **919691 B**，packets **88447 B**（约 **10.4×** 缩小），packets `estimatedInputTokens` 合计 **12998**。本轮本机未生成整页页图（packets 无图可附），与历史轮次（227766 B，带 420174 B 整页 PNG）的差异属环境差异，两模式对比结论不变。
  - 全新只读对抗式复核子代理：结论**放行**；3 处变异（连字符规范化、页一致性判断、时限比例）全部变红并逐条还原，`git diff --name-only -- src-tauri scripts` 复空；确认无断言删除（diff 中 `^-.*assert` 为 0）、CAS/人工保护/三态/调度器顺序/cloud_permits/包内串行零改动。

## 审计发现（P5/P7/P9 填，逐条带状态）

> P5 方法：3 个只读子代理分别查「边界与正确性」「上下文质量」「测试可信度」，每条发现都要求 `file:line` + 可复现失败场景。汇总时**逐条回源码复核**，能复现的才写下来；不能复现的标「未证实」并写明原因。另做 3 处临时突变检查（见 M1-M3）。
> 严重级别口径：**P0** = 触碰 §2 不可动摇边界，或产生「假完成 / 假核对」的对外结论；**P1** = 产品行为可观测地错误或退化，或阻断验收；**P2** = 质量 / 覆盖 / 可维护性。

### P9 追加发现与处置

| # | 级别 | 状态 | 提交 / 证据 |
|---|---|---|---|
| A-25 | P1 | **fixed** | `1df51b7`。packet ID 纳入每条差异完整 JSON 与阻断问题内容，再用 SHA-256 生成稳定 ID。`a_replanned_packet_with_changed_difference_values_is_not_skipped_as_done` 旧行为红（3 包 vs 预期 4 包），修复后绿。 |
| A-26 | P1 | **fixed** | `1df51b7`。当前 PDF 的无 bbox 整页 PNG 让 packets 请求体达到 1191137 B，高于 legacy 917173 B；包模式近灰度图转灰度并缩到最长边 600 px 后为 226697 B。彩色图保持原字节；真实 HTTP 对比及 PNG 单测通过。 |

**A-25 复现细节**：已完成的 q14 包之后，另一个文档包把 q14 canonical 值从 A 改为 E。旧 ID 只含差异键，重切会把新值仍在的 q14 差异误认为已完成并跳过。修复后 ID 由题组、完整差异内容、阻断问题完整内容和文档包标记共同派生；内容未变的包保持稳定，内容变化时会重新排队。回归用例还断言 run 状态不是 `completed` 且 q14 出现在 `remaining_tasks`。

**A-26 复现细节**：输入量两模式用例先因 packets **1191137 B** 大于 legacy **917173 B** 变红。根因是当前环境生成的 **420174 B** 无 bbox 整页 PNG 抵消了文本上下文节省。修复只作用于包模式近灰度 PNG；彩色页图原样发送，细节不足可通过 `read_page_region` 请求原尺寸区域图。压缩后的真实 HTTP 请求体为 **226697 B**，比 legacy 小约 **4.05×**。

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

### P7 处置（审计 #2 的发现，方法同 P5）

> P7 方法：2 个**全新**只读子代理，各自只拿到 TASK.md 路径、审计范围（`git diff 34dfadd..HEAD` / `git diff 6149443..HEAD`）与「只读、每条发现必须带 `file:line` 与可复现失败场景」，**不给任何上一轮结论**。A 查 §5/§7 逐条验收（通过/失败/未执行 + 证据），B 专查「P6 的修复有没有引入回归、或是不是只把测试改绿了」。汇总后逐条回源码复核。

| # | 级别 | 状态 | 提交 / 证据 |
|---|---|---|---|
| A-17 | P1 | **fixed** | `391294f`。审计 B 用临时探针证出「旧 2 条请求 / 新 1 条」，P7 自己重写失败断言看到红（`left: [(1, true)] / right: [(1, true), (1, false)]`）再修 |
| A-18 | P1 | **fixed（本机未执行）** | `2a5dc64`。步骤 10b / 11 改成按模式分流。macOS 无 CDP 通道，只做了 `node --check` 与整脚本冒烟；实机判定要等 Windows |
| A-19 | P2 | **fixed** | `208afd9`。先写失败断言看到红（note 真的是「attached whole-page images for 1 page(s)」），再修 |
| A-20 | P2 | **fixed** | `208afd9`。三条弱证据用例（crop / 文档包 / paperMap）各自加固，并各做一次突变确认能变红（M6 / M7 / M8） |
| A-21 | P2 | **fixed** | `7430a71`。§7.2 原来打的是测试内 TCP stub，不是任务书点名的受控服务脚本。新用例真起 node 跑 `scripts/controlled-llm-service.mjs`；突变 M9 确认它真的在驱动 L1 |

**P7 审计 #2 的复核轮（第 2 个只读子代理的回归复核，产出 A-22/A-23/A-24）**：

> 复核范围：`git diff 6149443..HEAD`（即 A-17 / A-18 / A-19 / A-20 / A-21 四个提交），只给范围不给结论。结论：**P0 无**；**这四个提交本身没有引入 P1**；三处「被加固的用例」各自做临时突变全部确认变红（未只改绿）。

| # | 级别 | 状态 | 提交 / 证据 |
|---|---|---|---|
| A-22 | P1 | **fixed（链路级未执行）** | `84325a1`。包模式剧本从不 `record_ruling` ⇒ CDP 链 `cloud-fixed-content-on-its-own` 的 `adjudicatedCount >= 1` 在默认（包）模式下**恒红**。先写失败用例看到红（返回 `finish_packet` + 「找不到剧本指定的题面行」），再给剧本加裁定分支。**链路级未执行**：① 本机无 CDP 通道；② 步骤 11b 仍是结构性阻断，见下 |
| A-23 | P2 | **fixed** | `4a578c5`。`escalate_packet` 的 L2 `existing` 只按 `pageIndex` 收集、不看 `image` 是否为 `null` ⇒ 「有 region 条目但无图」时写出假 note。先写失败用例看到红（note 真的是「every page in scope already had an image…」），再修 |
| A-24 | P2 | **fixed** | `c264ada`。§7.2 用例缺 node 时只 `eprintln!` 后 `return` ⇒ 恒绿但什么都没证，且注释自称「不静默当通过」。改成硬失败；实测把 node 从 PATH 移除后由 `1 passed` 变成失败 |

**A-17 是什么（P1，P6 的修复自己带进来的回归）**：`packets.rs::region_requests` 在 A-7 的修法里引入 `pages_requested`，却把它声明在 `for task_id` **外面**，判据于是从「整组」滑到「整包」：包内任一题组在某一页有 bbox，其他题组在这一页的「缺 bbox 退整页图」就被吞掉。两条 bbox 覆盖的版面未必重叠，本组要核的内容可能整块落在对方裁剪范围之外。顺带修掉一处顺序依赖：`draft.task_ids` 是 `BTreeSet`，谁先遍历取决于 id 字典序，同一个包可能给 1 张图、也可能给 2 张（旧代码下两种顺序结果不同）。修法：判据下沉到组内（`covered`），去重（`seen`）仍跨组。

**A-18 是什么（P1，CDP 链在默认模式下必红）**：`REPAIR_CONTEXT_MODE` 的默认值已经是 `Packets`，而 `scripts/` 里**没有任何一处**设 `IELTS_REPAIR_CONTEXT_MODE`（已全仓 grep 确认）—— 也就是说 CDP 链默认跑在包模式下。可是步骤 10b 要求「整条回合里至少一次 `read_source`」（包模式的原文是**随包**发来的，这个计数天然是 0），步骤 11 要求回合形状是 `read_draft → read_source → 被拒的 apply_edits → 带 baseVersion 重交 → record_ruling → finish`（包模式没有 `read_draft`，收工是 `finish_packet`）。两条都改成了按模式分流，且两种模式证明**同一件事**：写进稿子的 `baseVersion` 是模型从**真实请求**里读到的、不是剧本常量（包模式读 `draftSlice.editVersion`，legacy 读顶层 `editVersion`，两个值都来自请求体落盘文件）。步骤 11b 的 L1 断言保留（§7 的硬要求），但失败载荷补上逐包诊断（每个包的 `pagesIncluded`、升级级别序列、「承载正确答案的那一页在不在这一包的首次请求里」）——原来那句「全是 0」看不出是「这一卷的包恰好自足」还是「模型没敢要」，两者处置完全不同。

**A-21 是什么（P2，§7.2 的缺口）**：任务书 §7.2 点名「用真实网关代码打 `scripts/controlled-llm-service.mjs` 起 HTTP」。仓库里那条真实 HTTP 用例的对端是**测试内的 TCP stub**，只够量输入量与断言请求体形状；受控服务自己的包模式分支（A-4 补的）此前只有一份**未提交**的临时脚本验证过，CDP 链又只跑 Windows —— 这段代码在仓库里没有任何可执行证据。新用例真的起 node 跑那个脚本，剧本里**不给正确答案**（只给「改哪个槽、答案在哪一页」），断言编辑落库的值是包里那一行读出来的、`insufficientContext == 1`、`escalationLevel >= 1`、以及逐包记录里第一轮 L0 不含答案页 / 第二轮 L1 带着取回的答案页。

**A-22 是什么（P1，包模式剧本缺一条腿）**：`cloud-fixed-content-on-its-own` 要求 `adjudicatedCount >= 1`，而这个数字只数**裁定**（`effective_adjudicated_count`）——被编辑改掉的差异进的是 `appliedCount`，两者刻意不重叠。CDP 场景里恰好有一条「当前稿对、候选错」的差异（`cloud-repair-scenario.mjs:235-246`，`task_group` + `instructions`），只能靠 `record_ruling` 了结；可包模式剧本（`repairPacketStepReply`）只会 `report_insufficient_context` / `apply_edits` / `finish_packet`，**从不** `record_ruling`。修法：加 `packetRulingCall`，把 `plan.rulings` 与**本包的** `context.differences` 对上、引文逐字取自 `sourceEvidence.pages[].lines[]`，并用 `observations[].result.recorded[]` 去重。去重不是可选项：裁定**不会**让差异从 `context.differences` 里消失（`build_repair_context` 返回的是原始 `candidate_differences`），少了它包会每轮重复同一次调用 → `REPEAT_LIMIT` → 整个 run 报成 `budget_exhausted`。两条用例都真起 node 打真 HTTP，去重那条用突变 M10 确认有效。

**A-22 仍未达成的那一半（必须写清楚，否则「已修」是误导）**：步骤 11b 要求「至少一个包走了 L1」。包模式下 `plan_packets` 会把题组的**锚点页原文**随包发出（`sourceEvidence.pages`，见 `packets.rs:1243-1246`），而 CDP 场景的修复靶子是**题面类**差异（`setResponseGroup` 改写 prompt）——题面行就在题组自己的锚点页上，模型第一轮就拿到了，**没有升级的理由**。也就是说 11b 在这份场景下不是「模型没敢要」，而是**这一卷的包天然自足**；要让它可满足，只能把场景的修复靶子换成**答案类**差异（答案页不在题组锚点页里，正是 §7.2 那条用例的情形）——这属于改验收场景的设计决策，且本机无 CDP 通道无法验证。审计子代理与我都只能做到源码级判定，**未实测**。因此 §7.3 仍记「未执行」，P7 不标 done。

**A-23 是什么（P2，A-19 的修复自己带进来的一处倒退）**：A-19 把 L2 的 `attached` 从「整包带图的 region」改成「这一次新加的」，方向是对的；但同一个分支里 `existing`（判断「这一页是不是已经有图」）仍然只按 `pageIndex` 收集。`grab::materialize_regions` 在这一卷没有页图产物时会给每条 region 写 `"image": null`，而 `enforce_packet_budget` 只删**带图**的条目 —— null 条目原样留着。于是 `scope.pages ⊆ region 页` 时 `wanted == 0`，note 写「every page in scope already had an image in this packet」，而包里**一张图都没有**；A-19 之前这条路径说的是诚实的「no page image was available」。修法：`existing` 加 `!image.is_null()` 过滤。这条路径原本零覆盖（那句话只出现在生产代码里）。

**A-24 是什么（P2，一条恒绿但什么都没证的验收用例）**：§7.2 那条真实受控服务用例在 `node_binary()` 返回 `None` 时只 `eprintln!` 一句就 `return`。`cargo test` 默认捕获**通过**用例的 stderr，所以在没有 node 的机器上它是 `1 passed` —— 而它的注释自称「不静默当成通过」，代码与注释不符。与仓库里 pdfium 用例的区别是有意的：pdfium 是**可选**渲染器，node 是这个仓库的**必需**工具链（vitest、全部 `scripts/e2e/*.mjs`、`npm run check`）。改成 `panic!`，并同步改掉 `node_binary` 的说明。A-22 的两条新用例沿用同一处理。

**审计 A 的 §5/§7 逐条结论（修复后）**：

| 条目 | 结论 | 证据 |
|---|---|---|
| §5.1 切分规则 | 通过 | `packets.rs::tests::complex_reading_splits_into_packets_that_obey_the_grouping_rules`（P6 补，P7 用突变 M5 确认有效） |
| §5.2 交叉切分 | 通过 | `packets.rs::tests::a_different_cloud_split_merges_the_overlapping_groups` |
| §5.3 答案页 / `answerPagesUnknown` | 通过 | `answer_pages_fall_back_to_the_text_layer_search` 等 |
| §5.4 页号 0-based / 1-based | 通过 | `anchor_pages_are_reported_one_based_and_stay_inside_the_scope`（突变 M1） |
| §5.5 请求体无整份 PDF、有区域图 | 通过 | `packets_mode_requests_carry_no_whole_pdf_and_the_fetched_page_reaches_the_model`（突变 M4） |
| §5.6 上下文不足 → 用户任务 | 通过 | `a_packet_that_never_gets_enough_context_hands_the_difference_to_the_user_honestly`（突变 M2） |
| §5.7 抓取边界 | 通过 | `grab::tests::*`（突变 M3） |
| §5.8 编辑后重切 | 通过 | `packets.rs::tests::packet_ids_are_derived_from_identity_and_stay_stable_across_replanning` |
| §5.9 既有测试全绿 | 通过 | P9 最新 `cargo test --lib` 1104/0/11 |
| §7.1 命令处理器层三项 | 通过 | 见「基线数字」 |
| §7.2 真实 HTTP + 受控服务 | **通过（P7 补齐）** | `the_real_controlled_service_drives_the_packet_loop_through_l0_l1_and_finish`；A-22 又补两条裁定用例 |
| §7.3 CDP 13/13 + 新增一步 | **未执行（且包模式下仍差一步，见 A-22 的「未达成的那一半」）** | macOS 无 WebView2 CDP 通道；脚本报 `ENOENT ... ielts-author-studio.exe`。步骤已按模式分流（A-18）、包模式剧本已能裁定（A-22），但步骤 11b 的 L1 断言在这份场景下结构性不可满足，实机判定与场景调整留待 Windows |
| §7.4 真实模型 | **未执行** | 无额度 |

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

P7 追加（同样：临时改坏 → 确认变红 → 还原，`grep -c "TEMP-M"` 在四个文件上均为 0）：

| # | 改坏的点 | 变红的用例 | 结果 |
|---|---|---|---|
| M5 | `packets.rs::merge_components` 强行把所有题组并成一个 component | `tests::complex_reading_splits_into_packets_that_obey_the_grouping_rules` | 1 条变红（`left: 1 / right: 2`），已还原 |
| M6 | `packets.rs::paragraph_labels_on_page` 去掉「只有一个字符且是大写」的判据 | `packets::tests::the_paper_map_names_the_paragraph_labels_on_each_page` | 1 条变红（`left: ["A","B","THE"]`），已还原 |
| M7 | `packets.rs::plan_packets` 让文档包带上全部题组 | `tests::a_document_packet_never_hands_out_a_draft_slice` | 1 条变红（`left: ["early-approaches-q14-15"]`），已还原 |
| M8 | `grab.rs::crop_page_image` 还原 A-13 的混算（像素 `top` + 页单位 `height*3`） | `grab::tests::a_region_crop_keeps_the_bbox_height_it_promised` | 1 条变红（`裁剪高度 62 点…`），**由新加的「需求」断言**抓住而不是靠那个具体像素值，已还原 |
| M9 | `tests.rs` 的 §7.2 剧本把答案页指到**范围内**的页 | `tests::the_real_controlled_service_drives_the_packet_loop_through_l0_l1_and_finish` | 1 条变红（`left: ["B"] / right: ["A"]`，服务找不到答案行、什么也没改），已还原 |

审计 #2 复核轮追加（临时改坏 → 确认变红 → 还原；`grep -c "TEMP-M" scripts/controlled-llm-service.mjs` = 0）：

| # | 改坏的点 | 变红的用例 | 结果 |
|---|---|---|---|
| M10 | `controlled-llm-service.mjs::packetRulingCall` 删掉 `rulingAlreadyRecorded(...)` 去重 | `tests::the_controlled_service_stops_ruling_once_it_already_has` | 1 条变红（第 2 轮仍回 `record_ruling`，即真实存在的原地打转），已还原 |

P6 另有两处「对着旧行为跑红」的验证（不是独立突变，而是修 A-3 / A-12 时临时还原旧实现）：A-3 探针 `left: 0 / right: 2`、A-12 探针打印出 `TEMP-A12-PROBE-L2` 并变红，两处均已还原、**未提交**。

**历史审计中的未证实项（写了原因，不算发现）**：

- **A-25：`packetId` 不含内容指纹的风险，P5/P7 时未证实，P9 已证实并修复**。详见上方「P9 追加发现与处置」；集成用例构造「已完成的 q14 包、其他包改写 q14 值、重切后 q14 仍有差异」并先红后绿。此项不再是当前未证实问题。
- **`anchor_pages_of(input.candidate, &draft.task_ids)` 按本地 taskId 匹配候选锚点会漏**（`packets.rs:1028`）。反证：`reconcile/candidate.rs:4815` 的断言「14-15 必须接到权威稿题组」说明归一化后的候选题组 `task_id` **就是**本地 id；临时探针也打印出 `candidateSlice.taskGroups[].taskId = Some("early-approaches-q15-15"→本地 id)`。**未证实**（在本仓库的归一化下不成立）。
- **`crop_page_image` 的 `bottom-left` 分支可达**（见 A-13 的「未证实部分」）。

**P6/P7 对其余两条的复核结论**：
- 第 2 条（候选锚点按本地 taskId 匹配会漏）**已证伪**：`reconcile/candidate.rs:4815` 的断言与临时探针都表明归一化后的候选题组 `task_id` 就是本地 id。不成立，不修。
- 第 3 条（`bottom-left` 分支可达）**仍为未证实**：`bbox` 由 `pdf_ingest/coordinates.rs:137-155 display_rect` 产出、`origin` 恒为 `"top-left"`，`bottom-left` 只出现在 `nativeBBox`，而 `collect_bboxes` / `crop_page_image` 只读 `bbox`。P6 没找到能走到该分支的输入，因此按 A-13 只修了**确定可达**的算式错误。

## 越界需求（需要改非授权文件时写在这里，不要直接改）

1. **`src-tauri/src/cloud_repair/tools.rs`（§8 未授权）** —— 修 A-2 需要在这里把 evidence 的 `quote` 与**完整原文文本层**比对（`validate_evidence` 目前只校验结构）。§2 同时明写「不改 `tools.rs` 的校验与写入逻辑」，两条要求互相矛盾：**要么**放宽 §2 的那句「不改」，**要么**承认「引文校验只到结构层」并把 §2 的描述改准。P5 未动手，P6 同样未动手（证据见「P6 处置」A-2 条），等指示。
   - **P12 更新：已解除**。收尾任务明确授权「修改 `cloud_repair/tools.rs`，只限 validate_evidence 和它的调用处」，A-2 在 `b6c0370` 落地（CAS、人工保护、事务写入逻辑一字未动），TASK.md §2 的描述同步修正。A-2 状态从「未修（越界）」改为 **fixed**。
2. **`src/api/recognitionClient.ts` / `src/features/editor/recognitionDecisions.ts`（§8 未授权）** —— 若 A-1 的修法选择在**前端**把「已了结」与「上下文不足」分开计数，需要动这两个文件。P5 建议改后端（`mod.rs` 在授权内），P6 按此执行，因此**不需要**越界。
3. **`scripts/e2e/tauri-cdp-cloud-repair-chain.mjs` 的调用记录映射（超出「仅新增步骤」）** —— §8 允许该文件「仅新增步骤」，但新增的步骤 11b 需要读 `llm-calls.jsonl` 的 `packetId` / `escalationLevel` / `pagesIncluded` / `imageCount` / `estimatedInputTokens` / `requestBytes`，而原来的映射只保留 `{commandName, ok, errorClass, latencyMs}`。改动是**只加字段、不删不改**既有四个字段（`report.modelTraces.llm.callRecords` 的既有消费者不受影响）。这是新增步骤的最小前置，如认为仍越界请指示回退。
4. **A-4 的真实 HTTP 验证脚本不落仓库** —— §8 不允许在 `scripts/` 下新增文件，所以 A-4 的验证脚本放在 `/tmp/verify-packet-prompt-rewrite.mjs`（临时，未提交）。命令与 10/10 输出记在本条日志里，可据此复现。若希望它成为常驻用例，需要授权在 `scripts/` 下新增文件（或并入 `scripts/e2e/` 既有文件的某个步骤）。
   - **P7 更新**：这条**已不需要**了。§7.2 的真实受控服务用例（`7430a71`）现在真的起 node 跑 `scripts/controlled-llm-service.mjs`，A-4 补的答案类分支由它常驻覆盖；题面类分支由 CDP 链的 `packetPromptRewrite` 走（实机判定留待 Windows）。`/tmp` 那份脚本可以丢掉。
5. **`scripts/e2e/tauri-cdp-cloud-repair-chain.mjs` 的步骤 11 被**改写**（超出「仅新增步骤」）** —— §8 只允许该文件「仅新增步骤」，但审计 #2 查明：默认模式已是 `packets`，而步骤 10b / 11 写死的是 legacy 的回合形状，硬断言在包模式下必红（见 A-18）。要让这条链在默认模式下有意义，只能改这两步（步骤 10b 也动了：包模式下的前提换成「至少一轮请求带着承载那一页的原文文本」）。改动**没有删除任何原有断言**：legacy 分支逐条保留，包模式分支是新写的等价物。另外 `llmTraces` 的 `repairRounds` 补了 `draftEditVersion` / `packetId` / `escalationLevel` / `packetMode` / `packetPages` 五个**新增**字段（旧字段一个没删）。如认为仍越界请指示回退。
6. **`scripts/e2e/lib/cloud-repair-scenario.mjs`（§8 未授权）** —— 要让 §7.3 在**包模式**下真正可满足，必须改这份场景本身：把修复靶子从**题面类**差异换成**答案类**差异（答案页不在题组锚点页里 ⇒ 包天然缺材料 ⇒ 模型必须走 L1）。理由见「A-22 仍未达成的那一半」。不改场景的话，步骤 11b 的「至少一个包走了 L1」在这份卷子上永远为假——而 §2 又禁止把这条断言放宽。这是**设计决策**（换靶子会同时改变 legacy 分支的回合形状），所以 P7 没有擅自动手：**等指示**。可选方案：① 换靶子（改 `cloud-repair-scenario.mjs`）；② 保留现有场景，另加一份「答案类」场景专门喂包模式的 L1 断言。

## 日志（只追加：时间、agent 做了什么、提交、下一步）

- 2026-09-24 P0：`34dfadd` 在 macOS 上编译不过（`parser.rs:2184` E0425），基线改在 `f13c75a` 取。worktree `/tmp/pdf2test-baseline-34dfadd`（`CARGO_TARGET_DIR` 指向主 target 复用依赖；`lib/` 软链到主工作区）。数字：cargo 1046/0/11、vitest 391、tsc 0。下一步：P1。
- 2026-09-24 P1-P3：`fe15917`（packets.rs + grab.rs + 编排 + prompt + 去整份 PDF）、`85f5064`（修「无进展指纹不含升级级别」导致阶梯卡在 L1；修「收尾包被误报 budget_exhausted」）。`cargo test --lib` 1085/0/11。
- 2026-09-24 P4：`ee524f2`（§4.5 逐包 `packetId`/`escalationLevel`/`pagesIncluded`/`estimatedInputTokens`/`imageCount`/`requestBytes` 落 `llm-calls.jsonl`；`userTasks.ts` 把「材料没到手」与「查过但定不了」拆成两句）。`38ec89b`（§6 受控服务包模式剧本 + §6 两模式对比用例：legacy 917173 B → packets 83531 B）。**受控服务剧本已用真实 HTTP 逐分支验证**：缺答案页 → `report_insufficient_context{needs:[pages 3..3]}`；页到手 → `apply_edits{baseVersion:7, quote:"14 A"}`；差异清空 → `finish_packet`；**反自证**：页在包里但答案行不在 → 只 `finish_packet`，不编答案。
- 2026-09-24 P5：3 个只读子代理并行审计（边界与正确性 / 上下文质量 / 测试可信度）+ 3 处突变检查（M1-M3 均确认变红后还原）+ 逐条源码复核 + A-1 实测复现（临时断言变红 `left: 1 / right: 0`，已删除）。结果见「审计发现」：P0 ×1、P1 ×4、P2 ×11、未证实 ×3。下一步：P6 先修 A-1 / A-3 / A-4（A-4 是 CDP 验收的前置阻断），再补 A-5 的 CDP 步骤。
- 2026-09-24 P6：逐条处置「审计发现」。**fixed 14 条**（A-1 `b821ae2`；A-3/A-12 `71814c2`；A-6/A-7/A-13 `3e4a5dd`；A-8/A-9/A-10/A-11/A-14/A-15/A-16 `c617f7a`），**未修 1 条**（A-2：越界，见「越界需求」1，已写出源码证据），**不改但写明口径 1 条**（A-10），**已写入未执行 1 条**（A-5）。每条都先写失败测试并**看到红**（A-1/A-6/A-7/A-13 实测红；A-3/A-12 用临时还原旧行为的方式确认用例有效；M4 确认 `strip_packet_image_paths` 用例有效）。A-4 的题面类分支用**真实 HTTP** 验证：`/tmp/verify-packet-prompt-rewrite.mjs` 起 `scripts/controlled-llm-service.mjs`（`--plan` 用 CDP 剧本形状：无答案行），POST 真实 `repair_authoring_step` 请求体，**10/10 checks passed** —— 题面取自包内那一行、`sourceAnchors` 原样带回、`baseVersion` 取自包的 `editVersion`、引文逐字来自本轮请求；反自证分支（包内无题面行）只 `finish_packet` 并带上 `unresolved`；答案类差异仍走 `setAnswer`（回归）。**全量重跑**：`cargo test --lib` **1095/0/11**（基线 1046）、`vitest` **392**（基线 391，仍 1 个文件因缺 `selenium-webdriver` 加载失败）、`tsc exit 0`。§6 两模式对比重跑：legacy 917173 → packets **83738**（10.95×），差异来自 A-8（`paperMap` 多了一行 `paragraphLabels`）。下一步：P7（独立审计 #2 + Windows 上跑 CDP 13/13 → 14/14）。
- 2026-09-24 P7：**干净状态重跑**（`cargo test --lib` 1099/0/11、`vitest` 392、`tsc exit 0`、§6 对比 917173→83738 未变、CDP 未执行）+ **2 个全新只读子代理**（A 按 §5/§7 逐条验收；B 专查「P6 的修复有没有引入回归 / 是不是只把测试改绿了」，只给范围不给结论）。发现 **P1 ×2**（A-17 `region_requests` 的判据被 A-7 的修法从「整组」滑到「整包」；A-18 CDP 步骤 10b/11 写死 legacy 形状而默认模式已是 `packets`）、**P2 ×3**（A-19 L2 note 的 `attached` 数了整包带图的 region；A-20 三条弱证据用例；A-21 §7.2 打的是测试内 stub 而不是任务书点名的受控服务）。逐条按 P6 的方式修：先写失败断言**看到红**（A-17 `left: [(1, true)] / right: [(1, true), (1, false)]`；A-19 note 真的是「attached whole-page images for 1 page(s)」），再修，然后重跑受影响的测试。提交 `391294f`（A-17 + §5.1 用例）、`2a5dc64`（A-18）、`208afd9`（A-19/A-20）、`7430a71`（A-21）。**5 处突变检查 M5-M9 全部确认变红后还原**（`grep -c "TEMP-M"` 在四个文件上均为 0）。**P7 不标 done**：A-2 仍是未修的 P1（越界，见「越界需求」1），且 CDP 链在本机跑不了（§7.3 未执行）。另有一条 P0 级的**未证实**项仍未证实（`packetId` 不含内容指纹 → 重切后可能漏核），P7 没有真实模型额度，无法推进，见「未证实」。下一步：P8 收口报告 `REPORT.md`；拿到 Windows 环境或额度后补 §7.3 / §7.4。
- 2026-09-24 P7（审计 #2 复核轮）：第 2 个只读子代理的回归复核回来了，产出 **A-22（P1）/ A-23（P2）/ A-24（P2）**，并逐条复核了 A-17/A-18/A-19/A-20/A-21 四个提交：**P0 无、这四个提交本身没引入 P1**，三处被加固的用例各自临时突变全部变红（**不是只改绿**）。逐条按 P6 的方式修：A-23 先写失败用例看到红（note 真的是「every page in scope already had an image…」，而包里一张图都没有）→ `4a578c5`；A-24 把「缺 node 静默报绿」改成硬失败 → `c264ada`；A-22 先写失败用例看到红（包模式剧本回的是 `finish_packet` + 「找不到剧本指定的题面行」）→ 给 `repairPacketStepReply` 加 `packetRulingCall`（含 `observations` 去重，突变 M10 确认有效）→ `84325a1`。**重跑**：`cargo test --lib` **1102/0/11**、`vitest` 392、`tsc exit 0`、§6 对比未变。**仍未标 done**：① A-2（越界 P1）；② **A-22 只修掉了一半**——步骤 11b 的 L1 断言在这份 CDP 场景下结构性不可满足（题面类修复所需的原文行就在包内），要让 §7.3 在包模式下可满足必须改场景，属越界 + 设计决策，已写进「越界需求」6 等指示；③ CDP 链本机跑不了。下一步：P8 收口报告；等指示后再动场景。
- 2026-09-24 P8：写本目录 `REPORT.md`（八节：用户可感知变化 / 22 个提交列表 / 逐项证据等级 / 先红后绿与 M1-M10 突变 / 全量数字与基线对比 / 审计 24 条最终状态 / 未执行项 + 偏离 + 越界需求 / 下一轮建议：真实模型验收怎么跑、云端优先级与原生 tool call 与 prompt cache 各从哪个接口入手）。写报告时**没有重跑门禁**（上一轮 `a6c75a0` 的全量数字就是当前 HEAD 的数字，报告里如实注明取自该轮）。**整条链未收口**：P7 仍 `blocked`，三条原因（A-2 越界 P1 / §7.3 步骤 11b 结构性未达成 / CDP 本机不可跑 / §7.4 无额度）全部需要外部输入才能推进，因此**不输出 `ALL_DONE`**。下一步：等指示——要么授权越界需求 1（`tools.rs` 引文校验）与 6（场景换靶子），要么提供 Windows 环境 / 真实模型额度。
- 2026-09-24 P9：继续修复分支问题并提交 `1df51b7`（packet ID 纳入完整差异与阻断问题内容，避免差异值变化后重切包被 `done_packets` 跳过；包模式近灰度整页 PNG 转灰度并缩至最长边 600 px，彩色图保持原字节）。A-25/A-26 都先看到旧行为变红，再修复：重切集成用例从旧行为 3 包/预期 4 包变绿；两模式请求体从 packets 1191137 B > legacy 917173 B，压缩后 226697 B。最新门禁：`cargo test --lib` 1104/0/11；Vitest 392 tests passed、27 suites passed，1 suite 因缺 `selenium-webdriver` 加载失败；`npx tsc --noEmit` exit 0。`cargo fmt --check` 全仓仍受既有格式漂移影响，不通过；没有运行全仓格式化。证据为命令处理器/真实 HTTP 与单测，**不是产品端到端**；CDP 仍因本机 macOS 缺 WebView2/`ielts-author-studio.exe` 未执行，真实模型仍无额度。报告已更新 A-25/A-26 与本机最新输入量；P7 的原有阻塞未解除。
- 2026-09-24 P0 复核（HEAD `24eaa98`）：无未提交改动。平台为 Darwin 22.6.0 arm64 / Node v24.21.0。重跑 `cd src-tauri && cargo test --lib`：**1104 passed / 0 failed / 11 ignored**；`npx vitest run`：**392 passed / 0 failed tests**，27 suites passed，1 suite 加载失败（缺 `selenium-webdriver`，属环境问题）；`npx tsc --noEmit`：exit 0。对照 `34dfadd` 核验 TASK §3：`repair_authoring_step_through_gateway` 2866、`make_repair_authoring_step_input` 573、`run_openai_compatible_repair_step_llm` 1651、`build_repair_context` 843；12 条 observation / 6 轮、锚点结构、页号口径均符合快照。基线 `read_source` 无 `pageIndex` 会返回 PDF 全部页；当前包模式改为强制页范围或引文，legacy 保留旧行为。TASK §3 行号与描述在基线均无事实偏差，**未加 P0 更正**；实现后的行号变化（网关 1856、上下文 912）是新增代码造成的漂移。
- 2026-09-24 P0 复核：group-6 缺少选项 E-G（以及 group-5 少六行）是 `7b644b2` 在基线 `34dfadd` 之前已修复的历史问题（该提交是基线祖先），故不记作当前测试失败或本任务引入。当前真实听力探针 `real_listening_split_evidence_covers_every_claimed_block` 依赖私有 fixture `fixtures/golden/private-real/listening-vol7-t9.pdf`；本机 fixture 不存在，用例提前返回，因此本轮**未复核真实卷**，不能把它的绿灯当成该问题的当前端到端证据。
- 2026-09-24 P0 复核：已实际运行 `node scripts/controlled-llm-service.mjs`，默认监听 `127.0.0.1:11435`，base URL 为 `http://127.0.0.1:11435/v1`；`GET /health` 返回 `ok: true`，结束后确认服务已停止。P4 修复剧本使用 `--candidate <file> --plan <file>`。本机不是 Windows 且没有 `src-tauri/target/debug/ielts-author-studio.exe`，所以 WebView2 CDP 产品链**不能执行**；不报通过。下一未完成阶段 P7 仍受既有阻塞：A-2 要改 `tools.rs`，场景要改 TASK §8 未列出的 `scripts/e2e/lib/cloud-repair-scenario.mjs`，且本机无 Windows/WebView2 与真实模型额度；本轮未越界改动。
- 2026-09-24 P3 复核/修正（`1d79409`）：STATE 原标 P3 done；源码复核发现 prompt 的空参数 `read_draft` 示例会被包范围执行器拒绝、包模式还误称上下文是整份文档、`apply_edits` 没说可用包内 `draftSlice.editVersion`、L0-L2 input 仍携带 PDF 路径、`report_insufficient_context` 表列了但校验器不要求 `packetId`/`reason`，且旧 packetId 请求仍会被实际取材/算作 L1。先加回归测试并看到红，再修正这些协议点；既有测试/断言未删减或放宽，旧调用已提供新增必填字段。全量 `cd src-tauri && cargo test --lib`：**1110 passed / 0 failed / 11 ignored**。证据等级：命令处理器 + 真实 HTTP 受控服务测试；**不是** Tauri UI/CDP 端到端。P7 仍 blocked，外部条件未变化。

  工具信封核对表（prompt `tools` 表 ↔ `CloudRepairToolCallV1` / `validate_repair_tool_arguments` / `execute_tool` / `grab`）：反序列化后 gateway 要求顶层 `callId` 非空、`tool` 属 `CLOUD_REPAIR_TOOLS`、显式 `arguments` 对象（缺失/null 拒绝）；以下工具参数键逐项对照，分模式约束保留。

  | 工具 | prompt 参数键 | 解析/校验/分发核对 |
  |---|---|---|
  | `read_draft` | `taskGroupIds`, `questionNumbers` | 包模式至少一个本包选择器；`PacketTools::scope_error` 拒绝空范围/越界。prompt 现在明确说明，示例用包内 id。 |
  | `read_source` | `pageIndex`, `pageTo`, `quote` | 包模式页范围或 quote 必填，最多 3 页；legacy 保留旧全页行为。 |
  | `search_source` | `query` | validator 要求非空字符串。 |
  | `read_page_region` | `pageIndex`, `bbox` | validator 要求 1-based `pageIndex`；`bbox` 可选，grab 按对象裁剪/否则整页。 |
  | `read_passage` | `paragraphLabels`, `questionNumbers` | 至少一个非空数组；validator 与 grab 一致。 |
  | `read_candidate` | `taskIds`, `questionNumbers` | 至少一个非空数组；分发器再限制在本包范围。 |
  | `apply_edits` | `baseVersion`（number）, `commands`, `evidence` | prompt 现以数字示例；分发器要求整数版本和 commands 数组，evidence 继续走既有校验。 |
  | `record_ruling` | `rulings` | 分发器要求差异存在且 ruling 值合法；表内 reason/evidence 为裁定内容。 |
  | `report_insufficient_context` | `packetId`, `reason`, `needs` | validator 要非空 packetId/reason 与非空且按 kind 校验的 needs；分发器要求 packetId 等于当前包。 |
  | `finish_packet` | `note`, `unresolved` | 两项可选；循环结束当前包，后端仍重算剩余任务。 |
  | `finish` | `note`, `unresolved` | 两项可选；legacy/提前结束整次运行，后端仍重算剩余任务。 |

  P3 红绿证据：`packet_repair_prompt_does_not_show_an_unscoped_read_draft_call`、`packet_repair_rules_explain_the_read_draft_scope_selector`、`packet_repair_input_carries_pdf_path_only_for_the_l3_fallback`、`insufficient_context_validator_requires_the_declared_packet_and_reason_fields`、`insufficient_context_report_must_name_the_active_packet` 均先红后绿；工具名/参数键表一致性由 `repair_tools_table_names_and_argument_keys_match_the_dispatch_contract` 固定。测试 5/6/8/9 与 cloud_repair/tools 回归在上述全量 Rust 测试中通过。下一未完成阶段 P7 仍需越界事项裁定、Windows CDP 环境与真实模型额度，未执行项不记通过。
- 2026-09-24 P3 协议严格性补正 / P7 本机门禁复核（`a70e3e0`）：顶层 prompt 要求 `arguments` 对象，但 gateway 原来把缺失/null 当空参数；`repair_tool_envelope_requires_arguments_to_be_an_object` 先红后绿，空对象仍通过。最终全量 Rust：**1111/0/11**；`npx vitest run`：**392 tests passed**，27 suites passed、1 suite 因缺 `selenium-webdriver` 无法加载（与 P0 已知问题一致）；`npx tsc --noEmit` exit 0。产品端 CDP/真实模型未执行；P7 仍 blocked，未把不可执行事项计为通过。
- 2026-09-24 P4 复核并推进到下一未完成阶段 P7：核对 §4.5 的 `llm-calls.jsonl` 六项包字段和 `repair_json` 逐包 `rounds/escalationLevel/rulings/edits/insufficientContext` 诊断，确认「上下文不足」文案映射仍在 `userTasks.ts`。核对 `scripts/controlled-llm-service.mjs`：仅从请求里的 `scope.pages` / `sourceEvidence.pages[].lines[].text` 决定缺页与答案，引文和编辑值从实际行文本提取；剧本 plan 没有答案值。真实 HTTP 命令处理器测试起该服务，通过真实 `run_llm_gateway` 驱动 L0 缺页 → `report_insufficient_context` → L1 含第 3 页 → `apply_edits` → 后续包 `finish_packet`，最终 q14 为 A。先加 `finish_packet` 顺序和真实请求体断言；请求体记录能力未实现时测试先红（缺少 request log 文件），在受控服务加入可选 `--request-log` 后转绿；断言覆盖至少 3 个 HTTP 请求、编辑包之后有 `finished` 包、任何请求都不含 `data:application/pdf;base64,`。代码提交 `1b76667`。同一真实 PDF fixture 的真实 HTTP 模式比较本次重测为：legacy `estimatedInputTokens=0` / `requestBytes=917857`；packets `estimatedInputTokens=12480` / `requestBytes=227766`（旧记录保留其当时测量值）。全量 Rust `cargo test --lib`：**1111 passed / 0 failed / 11 ignored**；定向真实 HTTP 测试通过；`npx vitest run`：**392 passed**、27 suites passed、1 suite 因缺 `selenium-webdriver` 加载失败（已知基线环境问题）；`npx tsc --noEmit` exit 0。当前为 Darwin，CDP 结论：**未执行：平台不支持 CDP 通道**。证据等级为命令处理器 + 真实 HTTP，非产品端到端。P4 保持 done；P7 仍 blocked：A-2 要求改 §8 未授权的 `tools.rs`，步骤 11b 需要针对 §8 外的 CDP 场景更换差异类型，另需 Windows/WebView2 与真实模型额度；本轮未越界修改。
- 2026-09-24 P3 完成证据复核 / 推进 P7：当前 HEAD `a19c378` 无未提交改动。复核包提示将工具名从 `CLOUD_REPAIR_TOOLS` 单一常量取值、示例信封和包/legacy 规则；`repair_tools_table_names_and_argument_keys_match_the_dispatch_contract` 按工具逐项比对参数键，`validate_repair_tool_arguments` 与分发器保持约束。测试 5 `packets_mode_requests_carry_no_whole_pdf_and_the_fetched_page_reaches_the_model`、测试 6 `a_packet_that_lacks_the_answer_page_says_so_and_gets_it_next_round` / `a_packet_that_never_gets_enough_context_hands_the_difference_to_the_user_honestly`、测试 8 `an_applied_edit_reslices_the_packet_with_a_fresh_version_and_fewer_differences`，以及测试 9 `tools.rs` 的人工保护目标、旧版本和证据页校验均由全量 Rust 集覆盖。重跑 `cd src-tauri && cargo test --lib`：**1111 passed / 0 failed / 11 ignored**；`npx vitest run`：**392 passed**、27 suites passed，1 suite 因缺 `selenium-webdriver` 加载失败；`npx tsc --noEmit` exit 0。平台复核为 Darwin，WebView2 CDP 未执行，按 §7.3 记 **未执行：平台不支持 CDP 通道**。证据等级为源码/命令处理器与单测，非 Tauri 产品端到端；P3 维持 done，P7 仍 blocked，原因及所需外部决策见「P7 处置」与「越界需求」，本轮无越界改动。
- 2026-09-25 P12-Q（`2faa456` + `b1cd8f3`）：修复第三方复核发现的两个引文核验缺陷。
  - **缺陷 1（无文本层页误拒）**：`EvidenceSourceText::Paged` 只要文档有任何一页有文本就要求引文必须找到；扫描答案页 / 图片答案表（L2 整页图与 `read_page_region` 交给模型的页）的文本层缺席，模型从页图读来的真实引文被误拒。修法：引文**全文都找不到**且声明页存在、其 ±1 相邻页中有任何一页没有文本层（缺席或规范化文本 < `MIN_TEXT_LAYER_CHARS=8`，理由见常量注释：扫描页文本层常只剩 1–4 字符噪声，真实可引用原文明显超过它）⇒ 标 `unverifiable` 放行；声明页与相邻页都有真实文本层时照拒；声明页超出文档末尾（编造页号）照拒。`EvidenceSourceText::Paged` 变体新增 `source_file_id` 与 `existing_pages`（有文本页 ∪ 有页图页，用于区分扫描页与超界页号）。先红后绿：`a_quote_read_from_a_textless_page_is_marked_unverifiable_not_rejected`（两变体：全缺席 / 只剩页码噪声）红 → 绿；护栏 `a_missing_quote_on_a_page_with_a_real_text_layer_is_still_rejected`、`a_declared_page_beyond_the_document_is_rejected_even_without_text_layers` 常绿钉住不放宽。
  - **缺陷 2（非主试卷 sourceFileId）**：先查链路——修复链的证据面**只来自主试卷**：`cloud_source_evidence`/`main_source_for_cloud` 只选 `role == "MainQuestion"`；`grab::load_source_index` 只读主 `document-ir.json`；legacy 只附主 PDF/文本。答案文件（`role == "AnswerKey"`）只进 `parse_answer_source_candidates` 的本地答案识别管线，修复循环任何一环都看不到它。**选方案 b**：非主试卷 sourceFileId 的证据一律标 `unverifiable`，不拒绝，prompt 两种模式都写明。理由：把答案文件文本接入核验，等于后端拿「模型从未在修复链里见过的内容」做核验——核验面与证据面脱节；且接线要在每次核验时加载并解析第二个来源的 document-ir，引入第二份真源。方案 b 保住任务书的硬要求：真实存在于答案文件里的引文不会被误拒。测试 `evidence_from_a_non_main_source_file_is_marked_unverifiable_not_rejected` 先红后绿。
  - **端到端（真实受控服务 + 真实 HTTP）**：`an_edit_quoting_an_image_only_answer_page_lands_and_is_marked_unverifiable`——答案页只有页图零文本行，受控服务走 `read_page_region` 抓页（新分支受 `answerFetch` 键门控，答案值取自请求内 `candidateSlice.answerKey`，反自证成立：第一轮请求文本里没有答案行），引用后修改落库、`evidenceUnverifiable` 在工具结果 / 逐包诊断 / `unverified_evidence` 摘要三处如实呈现、L1 且 `insufficientContext==0`、全程无整份 PDF。对旧行为（临时禁用退让分支）红：`left: ["B"] / right: ["A"]`。
  - **复核**：全新只读对抗式子代理放行，2 处变异（阈值比较方向翻转 → 护栏用例红；`same_source_file_id` 恒真 → 非主试卷用例红）均变红并精确还原。复核指出一条「实现宽于声明」：退让此前在「引文于别页找得到、页号差 >1」时也触发，宽于「全文都找不到」→ `b1cd8f3` 收紧（`found.is_empty()` 才退让）+ 钉住用例先红后绿 + u32 截断加固。CAS / 人工保护 / 三态 / 既有测试零改动。
  - 全量：`cargo test --lib` **1131/0/11**（上一轮 1125，+6：4 条 tools 单测 + 1 条端到端 + 1 条收紧钉住）、`cargo test --lib cloud_repair` **136**（130）、vitest **392**（同一 suite 因缺 `selenium-webdriver` 加载失败）、tsc `exit 0`。CDP 与真实模型仍未执行（平台 / 额度）。
- 2026-09-24 P9-Q（`b6c0370`）：A-2 落地。`validate_evidence` 保持结构校验，新增 `verify_evidence_quotes`：每条 `evidence.quote` 对照**完整原文**文本层（PDF 用 `grab::load_source_index` 的全量逐页行文本，与抓取工具同源；DOCX/TXT 用 `cloud_source_evidence` 抽取全文），规范化仅限空白折叠/弯引号/连字符统一/忽略大小写，不做模糊匹配；声明页与实际页 ±1 内放行，超出整批拒绝 `CLOUD_EDIT_EVIDENCE_QUOTE_NOT_IN_SOURCE:<index>`；原文无文本层（扫描件）标 `unverifiable` 不拒绝——裁定侧逐条盖 `verification` 标记随 `repair-rulings` artifact 落库，编辑侧经 `evidenceUnverifiable` 进工具结果、逐包诊断与 `repair_json.unverifiedEvidence` 摘要。record_ruling 走同一套校验（结构 + 引文），编造引文的裁定不得记录。`repair_step_prompt` 两种模式都写明拒绝码/±1 容差/unverifiable 语义（prompt 契约测试钉住）；受控剧本引文本就从请求行逐字取，未改行为；tests.rs 四个 legacy 剧本的编造引文改为真实原文行、`seed_job_with_source` 文本层补真实答案页（断言一字未删，overrule 用例「文本层无 14 C」保留）。TASK.md §2 描述修正。先红后绿：临时把核验 stub 成旧行为，3 条新用例变红（编造引文被放行 / 差 2 页被放行 / 无 unverifiable 标记），还原后全绿。编辑器 journal 表（repository.rs）未动——edits 的核验标记落不进 `editor_journal_v1`，如实落在工具结果与摘要里，rulings 的落在裁定 artifact。
- 2026-09-24 P10（`a8e1678`）：答案类场景。`controlled-llm-service.mjs` 新增 `answerFetch: 'read_source'` 分支（受 plan 键门控，旧剧本一字不变）：答案页不在包里时用 `read_source` 主动抓取，答案值与引文逐字取自上一轮抓取返回的行。`cloud-repair-scenario.mjs` 纯新增 `deriveAnswerRepairScenario`（答案错误真实存在 + 答案页不在题组锚点页上才成立；plan 含答案值或答案页在锚点页上 ⇒ 如实报前提不成立；node 冒烟 4 分支全过）。CDP 链新增派生步骤，结果写独立 `report.answerScenario` 三态段——当前 golden 标注 `answerKeyAbsence`（原文件无答案页），场景在这份 fixture 上必然 not-executable，刻意不进 verdict 以免拖垮题面类验收；等带 `answerErrors` 标注的 golden 落库后即可切换为判定步骤。Mac 等价证据：`an_answer_difference_fetches_the_answer_page_through_read_source_before_fixing` 真起受控服务走真实 HTTP，断言第一轮请求体无「14 A」且 pagesIncluded 不含答案页、后续轮带抓取结果（L1、insufficientContext==0 ⇒ 证明走的是抓取工具路径而非报告路径）、最终答案正确、全程无整份 PDF。先红后绿：stash 掉受控服务分支后用例红（无 read_source 发生），恢复后绿。CDP 实机：未执行（macOS 无 WebView2 通道）。
- 2026-09-24 P11（`542e660`）：**选方案 b**——总时限按包数线性放宽（+25%/包、封顶 3×，基础 10 分钟 ⇒ 最多 30 分钟）。理由：方案 a（不相交包并行 ≤2 请求）会让重切、done_packets、used_full_source、预算记账这些单线程循环状态进入并发路径，CAS 之外的不变量风险大；方案 b 改动集中在一处换算且直接命中「10 包 × 每轮 16–50 秒 ≫ 10 分钟」的真实瓶颈。实现：`scaled_packet_deadline` 在包循环开工时按初始队列长度换算一次，循环内三处截止判断改用它；重切新包不二次续期；legacy 循环与单包（ratio=1）行为与旧常数完全一致；调度器阶段顺序、cloud_permits、包内串行、取消/失败终态/进度上报零改动。先红后绿：`ten_packets_with_fixed_round_delay_finish_within_a_deadline_scaled_to_the_packet_count`（多组夹具 10 差异包、每轮睡 400ms、时限缩到 2.5s）对旧行为红——2.5s 只处理完 5 包即 `budget_exhausted`（临时禁用换算复现，`left: 5 / right: 11`）；放宽后 11 轮全部完成、`completed`。另加 `a_conflicting_edit_from_a_later_packet_is_rejected_and_recovers_without_overwriting`：后到包用过期 baseVersion 被 CAS 拒（`EDIT_VERSION_CONFLICT:current=2:base=1`），用重切后刷新的版本重试落地，先到包的修改不被覆盖。
- 2026-09-24 P12（收尾）：全量重跑 `cargo test --lib` **1125/0/11**（P0 复核基线 1111，净 +14）、`cargo test --lib cloud_repair` **130**（117）、vitest **392 passed**（27 suites，1 suite 因缺 `selenium-webdriver` 加载失败，基线问题）、`tsc exit 0`；两模式对比重跑 legacy **919691 B** / packets **88447 B**（≈10.4×）/ packets tokens **12998**（本机本轮未生成整页页图，与历史 227766 B 的差异属环境差异，结论不变）。派 1 个**全新**只读对抗式复核子代理（不给本轮结论）：放行；3 处变异检查（M-新1 连字符规范化、M-新2 页一致性判断放宽方向、M-新3 时限比例置 1）全部变红并逐条还原，还原后复跑绿、`git diff --name-only -- src-tauri scripts` 复空；确认 diff 无任何断言/测试函数删除、CAS/人工保护/三态/调度器顺序/cloud_permits/包内串行零改动、P10 剧本反自证成立。REPORT.md 追加 §9。未执行项不变：CDP 实机（macOS）、真实模型（无额度）。
- 2026-09-24 P8 证据复核与报告同步（报告提交 `6713720`）：确认 `REPORT.md` 原有数字取自 P9 历史复测；本轮重新运行 `cd src-tauri && cargo test --lib packets_mode_sends_much_less_input_than_legacy_for_the_same_paper -- --nocapture`，通过并打印 legacy `requestBytes=917857`、packets `requestBytes=227766`、packets `estimatedInputTokens=12480`。报告保留 P9 的 `917173/226697` 作为历史值，并将本次复跑明确列为最新值（约 4.03×）。复核 `34dfadd..a5300ad` 共 32 个提交，报告列表已补齐；最新全量门禁证据仍是 `1b76667` 后的复核：Rust **1111/0/11**、Vitest **392 tests passed**（27 suites passed，1 suite 因缺 `selenium-webdriver` 无法加载）、tsc `exit 0`，没有把加载失败计为通过。本轮只重跑了定向对比，没有重跑全量门禁。证据等级：真实 HTTP 命令处理器 + 既有全量门禁记录；不是 Tauri UI/CDP 产品端到端。P8 维持 done；P7 的剩余事项仍因 §8 越界/设计决策、Darwin 不支持 WebView2 CDP、没有真实模型额度而 blocked，本轮无越界修改。
