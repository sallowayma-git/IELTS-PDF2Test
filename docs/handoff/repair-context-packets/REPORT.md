# 收口报告 — 校核链路上下文管理（分支 `feat/repair-context-packets`）

> 分支基线 `wave3-listening` @ `34dfadd`，**未 push**。进度真源是同目录 `STATE.md`；本文件是它的收口摘要，冲突时以 `STATE.md` 为准。
> 报告里的每个数字都标了**证据等级**。等级低的证据**不**用来支持等级高的结论（`AGENTS.md` 的硬要求）。

---

## 1. 用户能感知到的变化

### 1.1 一次校核少发了多少内容

同一份作业、同一条真实 HTTP 网关路径、同一份 212 KB 真实 PDF 样本，跑两种上下文模式：

| 指标 | legacy（改造前） | packets（现在，默认） | 变化 |
|---|---|---|---|
| 整轮 `requestBytes` 合计 | **917857** | **227766** | **≈ 24.8%，缩小 4.03 倍** |
| 逐包 `estimatedInputTokens` 合计 | 0（没有「包」这个概念） | **12480** | — |
| 每轮是否附整份 PDF base64 | 是（每轮都附） | **否**；只有升到 L3（每次运行最多 1 次）才退回整份附件 | — |

用例：`cloud_repair::tests::packets_mode_sends_much_less_input_than_legacy_for_the_same_paper`（本机实测，`--nocapture` 打印）。本次复核复跑值为 **917857 B / 227766 B / 12480 tokens**；P9 当时记录的 917173 B / 226697 B 是此前一轮的历史实测，不覆盖本次最新值。没有足够证据解释两轮字节数的细小差异，因此不推断其原因。

P9 的复测请求路径为缺少 bbox 的页面附加了一张 **420174 B** 的整页 PNG；未压缩时包模式请求体合计达到 **1191137 B**，反而超过 legacy。于是包模式现在会将近灰度整页 PNG 转灰度并缩到最长边 600 px，彩色页图仍保留原样；图片细节不足时可再通过 `read_page_region` 按需读取原尺寸区域。P9 当时压缩后的真实请求合计为 **226697 B**；本次复核复跑合计为 **227766 B**。

P4/P7 曾记录的 **83738 B / 12956 tokens** 在本轮请求路径无法复现，因为本轮实际附带了整页 PNG。旧数值保留为历史记录，不再作为最新输入量结论。

**证据等级：命令处理器层（含真实 HTTP）**。它走真实网关代码、真实请求体落盘记录，但没有经过 Tauri UI，也没有真实模型。

### 1.2 上下文不够时，系统现在怎么做

改造前：模型每轮都拿到整份 PDF + 整卷上下文 + 全部历史 observation，于是「材料不够」这件事在结构上不会发生——它只会**猜**，而且猜得毫无痕迹。

现在：

1. **L0 本地预切**。按差异把要核的部分切成「校核包」，每包只带本包的稿件切片（含 `editVersion`）、云端候选切片、范围内原文（逐行文本，行 id 形如 `p4:l12`）与锚点区域图。范围外的内容不进请求。
2. **不够就说（L1 入口）**。包内材料不足以判断时，模型必须回结构化的 `report_insufficient_context`，点名要哪几页 / 哪段引文 / 哪个段落 / 哪个切片。**不猜**。
3. **自己去取（L1）**。模型也可以用受限只读工具主动取：`read_source`（必须给页范围或引文，单次 ≤ 3 页）、`search_source`、`read_page_region`、`read_passage`、`read_candidate`、`read_draft`（默认限本包）。每包抓取预算 ≤ 3 次、≤ 6 页、≤ 160 KB。
4. **逐级升级**。仍不够则 L2（后端补该范围整页图）→ L3（本包附一次整份原文件，每次运行最多 1 次）→ L4（后端**代记** `cannot_resolve` + 理由码 `CONTEXT_INSUFFICIENT`）。
5. **如实交给用户**。L4 的差异会进用户清单，文案「云端没能拿到足够的原文来判断第 N 题，请对照原文确认」，**不会**被写成「已核对」。三态（`not_executed` / `insufficient_context` / `passed`）在数据层与文案层都是分开的。

用户侧的直接感受：修复循环跑的轮数变少、每次请求小一个数量级；**「云端说它改好了」和「云端说它没材料」不再混成同一句话**——前端把「材料没到手」与「查过但定不了」拆成两句（`src/features/editor/userTasks.ts`，仅文案）。

**证据等级：命令处理器层（含真实 HTTP）**。L0→L1→编辑→收工的完整往返由 `the_real_controlled_service_drives_the_packet_loop_through_l0_l1_and_finish` 驱动**仓库里那个真实受控服务脚本**（真起 node、真 HTTP），断言「第一轮请求里没有答案页、第二轮带着取回的答案页」；不是靠测试内的 stub 自证。

---

## 2. 提交列表

`git log --oneline 34dfadd..a5300ad --reverse`（本次复核的 HEAD 快照共 **32 个提交**；未 push）：

| # | 提交 | 内容 |
|---|---|---|
| 1 | `f13c75a` | 修 `parser.rs` macOS sips 分支的 `asset_dir` —— **基线提交不是 `34dfadd`**，原因见 §7 |
| 2 | `9f390cc` | 任务书与 STATE 落仓库 |
| 3 | `fe15917` | `packets.rs` + `grab.rs` + 编排 + prompt + 去掉整份 PDF 附件（P1-P3） |
| 4 | `85f5064` | 让升级阶梯能走到 L4；收尾包不再被误报 `budget_exhausted` |
| 5 | `ee524f2` | §4.5 逐包可观测性；「材料没到手」文案与「查过但定不了」分开 |
| 6 | `38ec89b` | §6 受控服务包模式剧本 + 两模式对比用例 + 审计记录（P4） |
| 7 | `62bda85` | 修正审计记录的提交号与措辞 |
| 8 | `c00526c` | 记下基线 worktree 的重建方法 |
| 9 | `b821ae2` | **A-1（P0）**：「上下文不足」不再被算进「已了结」 |
| 10 | `71814c2` | A-3 / A-12：预算裁剪与升级说明不再互相矛盾 |
| 11 | `3e4a5dd` | A-6 / A-7 / A-13：不再编造页号、不再丢证据页、不再超额裁剪 |
| 12 | `c617f7a` | A-8 / A-9 / A-10 / A-11 / A-14 / A-15 / A-16：段落标号、预算口径、测试缺口 |
| 13 | `6149443` | A-4 / A-5：受控服务补「题面类」分支；CDP 链新增步骤 11b |
| 14 | `391294f` | **A-17（P1）**：某组的整页图回退不再被别组的区域裁剪吞掉 |
| 15 | `2a5dc64` | **A-18（P1）**：CDP 步骤 10b / 11 改成按模式分流 |
| 16 | `208afd9` | A-19 / A-20：L2 只数它自己附的图；三条弱证据用例加固 |
| 17 | `7430a71` | A-21：§7.2 改成真起 node 跑真实受控服务（真 HTTP） |
| 18 | `db0e051` | 记 P7 重跑数字、审计 #2 发现与修复 |
| 19 | `4a578c5` | **A-23（P2）**：L2 不再把「有 region 条目但无图」的页说成已覆盖 |
| 20 | `c264ada` | **A-24（P2）**：缺 node 时用例硬失败，不再恒绿 |
| 21 | `84325a1` | **A-22（P1）**：包模式剧本能裁定「当前稿对、候选错」的差异 |
| 22 | `a6c75a0` | 记审计 #2 复核轮的 A-22/A-23/A-24 |
| 23 | `ffb055e` | 本报告（P8 收尾） |
| 24 | `1df51b7` | A-25 包内容变化时重建稳定 ID；A-26 压缩包模式近灰度页图 |
| 25 | `24eaa98` | 复核 P0 基线证据 |
| 26 | `b1b91d1` | 补记 P0 基线复核 |
| 27 | `1d79409` | 对齐 packet prompt 信封与分发协议 |
| 28 | `a70e3e0` | 强制工具调用的 `arguments` 为对象 |
| 29 | `8e09749` | 记录 P3 协议复核证据 |
| 30 | `1b76667` | 真实请求断言不带整份 PDF，并验证编辑后收尾包 |
| 31 | `a19c378` | 记录 P4 复核和 P7 阻塞 |
| 32 | `a5300ad` | 记录 P3 证据复核 |

---

## 3. 证据等级

| 结论 | 等级 | 具体是什么 | **不是**什么 |
|---|---|---|---|
| 一次校核少发 4.03 倍内容 | 命令处理器层（含真实 HTTP） | 真实网关代码 + 真实请求体落盘 + 212 KB 真实 PDF 样本；本次复核压缩页图后 227766 B vs 917857 B | 不是产品端到端；没有 UI，没有真实模型 |
| 包模式走完 L0→L1→编辑→收工 | 命令处理器层（含真实 HTTP） | 真起 node 跑 `scripts/controlled-llm-service.mjs`，真 HTTP 往返 | 不是产品端到端（无 WebView2） |
| 「不够就说」有出口、L4 如实交给用户 | 命令处理器层 | `cargo test --lib` 的包模式用例 | 不是真实模型的行为证据 |
| 切分规则、页号换算、抓取边界、预算退让 | **仅单元测试** | `packets::tests::*` / `grab::tests::*` 纯函数用例 | 不是端到端；这些用例**不**驱动 Tauri、WebView2、SQLite 真实写路径 |
| CDP 链 14/14 | **未执行** | 本机 macOS 无 WebView2 CDP 通道 | 不是通过，也不是失败——是没跑 |
| 真实模型收敛 / token / 是否主动报「不够」 | **未执行** | 无额度 | 同上 |
| 证据引文与完整原文比对 | **未实现（越界）** | 见 §6 A-2 | §2 的那句话目前**没有**实现支撑 |

---

## 4. 亲眼看到「先红后绿」的测试

### 4.1 先写失败断言、看到红、再修

| 发现 | 用例 | 看到红时的实际输出 |
|---|---|---|
| A-1（P0） | `a_packet_that_never_gets_enough_context_hands_the_difference_to_the_user_honestly` 末尾临时加 `assert_eq!(report.adjudicated_count, 0)` | `left: 1 / right: 0`（一条云端从未拿到材料的差异被算成已了结） |
| A-3 | 临时还原旧行为 | `left: 0 / right: 2` |
| A-6 | 页号上界用例 | `[2, 3, 4]` vs `[2, 3]` |
| A-7 | 逐页 bbox 用例 | `[(1,true),(3,true)]` vs `[(1,true),(2,false),(3,true)]` |
| A-12 | 临时探针 | 打印 `TEMP-A12-PROBE-L2` 后变红 |
| A-13 | `a_region_crop_keeps_the_bbox_height_it_promised` | `88x180` vs `88x88` |
| A-17（P1） | 重写失败断言 | `left: [(1, true)] / right: [(1, true), (1, false)]` |
| A-19 | `the_l2_note_counts_only_the_images_it_actually_attached` | note 真的是「attached whole-page images for 1 page(s)」 |
| A-22（P1） | `the_real_controlled_service_rules_on_a_ruling_type_difference_in_packet_mode` | 返回 `finish_packet` + 「找不到剧本指定的题面行」 |
| A-23 | `the_l2_note_does_not_call_a_page_without_an_image_covered` | note 真的是「every page in scope already had an image in this packet」 |
| A-24 | §7.2 用例 + 把 node 从 PATH 移除 | 修复前 `1 passed`（恒绿）；修复后 `本机 PATH 里没有 node…` 失败 |
| A-25（P1） | `a_replanned_packet_with_changed_difference_values_is_not_skipped_as_done` | 修复前只处理 3 包，期望 4 包；q14 的新 canonical 值仍被旧 ID 过滤 |
| A-26（P1） | `packets_mode_sends_much_less_input_than_legacy_for_the_same_paper` | 修复前 packets **1191137 B** > legacy **917173 B**；压缩后 **226697 B**。PNG 单测另验证灰度缩放和彩色保持原样 |

### 4.2 突变检查（改坏 → 确认变红 → 还原）

| # | 改坏的点 | 变红的用例 |
|---|---|---|
| M1 | `packets.rs` 页号 `+ 1` 去掉 | `anchor_pages_are_reported_one_based_and_stay_inside_the_scope` 等 2 条 |
| M2 | `mod.rs` 理由码 `CONTEXT_INSUFFICIENT` 改成别的 | `a_packet_that_never_gets_enough_context_...` |
| M3 | `grab.rs` 删掉「无选择器就拒绝」 | `read_source_without_a_selector_is_rejected_with_a_specific_reason` 等 2 条 |
| M4 | `llm_gateway.rs` 停掉 `strip_packet_image_paths` | `packets_mode_attaches_the_region_image_and_keeps_local_paths_out_of_the_prompt` |
| M5 | `merge_components` 把所有题组并成一个 | `complex_reading_splits_into_packets_that_obey_the_grouping_rules`（`left: 1 / right: 2`） |
| M6 | `paragraph_labels_on_page` 去掉「单个大写字母」判据 | `the_paper_map_names_the_paragraph_labels_on_each_page`（`left: ["A","B","THE"]`） |
| M7 | `plan_packets` 让文档包带上全部题组 | `a_document_packet_never_hands_out_a_draft_slice` |
| M8 | `crop_page_image` 还原 A-13 的混算 | `a_region_crop_keeps_the_bbox_height_it_promised`（被**需求**断言抓住，不是靠那个具体像素值） |
| M9 | §7.2 剧本把答案页指到范围内的页 | `the_real_controlled_service_drives_the_packet_loop_through_l0_l1_and_finish`（`left: ["B"] / right: ["A"]`） |
| M10 | `packetRulingCall` 删掉去重 | `the_controlled_service_stops_ruling_once_it_already_has`（第 2 轮仍回 `record_ruling`） |

全部已还原：`grep -c "TEMP-M"` 在 `grab.rs` / `packets.rs` / `mod.rs` / `tests.rs` / `controlled-llm-service.mjs` 上均为 0。

**审计 #2 复核轮特别确认**：A-20 那三条「被加固」的用例不是「只把测试改绿」——M6 / M7 / M8 证明它们在实现改坏时确实会红。

---

## 5. 全量数字与基线对比

| 门禁 | 基线（`f13c75a`） | 本分支最新复测 | 变化 |
|---|---|---|---|
| `cd src-tauri && cargo test --lib` | 1046 passed / 0 failed / 11 ignored | **1111 passed / 0 failed / 11 ignored** | 净 **+65** |
| `npx vitest run` | 391 passed（28 文件里 1 个加载失败） | **392 passed**（27 suites passed，1 suite 因缺 `selenium-webdriver` 加载失败） | 净 **+1** |
| `npx tsc --noEmit` | `exit 0` | **`exit 0`** | 无变化 |

最新全量门禁结果来自 `1b76667` 后的复核：Rust **1111/0/11**；Vitest **392 tests passed**、27 suites passed，另 1 suite 因缺 `selenium-webdriver` 无法加载；`npx tsc --noEmit` 为 `exit 0`。Vitest 的加载失败仍是已知环境基线问题，不计为通过。

**已知基线失败（非本任务引入，两提交一致）**：
1. `scripts/e2e/lib/tauri-harness.mjs` 加载失败：`Failed to load url selenium-webdriver` —— 本机 `node_modules` 缺这个依赖，属环境问题。
2. CDP 通道只在 Windows（WebView2）可用。

**基线提交为什么不是 `34dfadd`**：`34dfadd` 在 macOS 上**根本无法编译** —— `src-tauri/src/parser.rs:2184` `E0425: cannot find value '_asset_dir'`，只出现在 macOS 的 sips 渲染分支（仓库此前只在 Windows 构建过）。本分支第一个提交 `f13c75a` 把 `_asset_dir` 改回真实存在的 `asset_dir`，**无任何行为变更**，基线数字因此取 `f13c75a`。

---

## 6. 审计发现的最终状态

两轮独立审计（P5 三个只读子代理；P7 两个全新只读子代理 + 一轮回归复核），每条都要求 `file:line` + 可复现失败场景，汇总时逐条回源码复核。

| # | 级别 | 最终状态 | 提交 / 证据 |
|---|---|---|---|
| A-1 | P0 | **fixed** | `b821ae2`。`effective_adjudicated_count` 排除 `CONTEXT_INSUFFICIENT` |
| A-2 | P1 | **未修（越界）** | 证据引文从不与完整原文比对。修它必须改 `tools.rs`，§2 明写不改 → 越界需求 1 |
| A-3 | P1 | **fixed** | `71814c2` |
| A-4 | P1 | **fixed** | `6149443`。受控服务补「题面类」分支，真实 HTTP 逐分支验证 10/10 |
| A-5 | P1 | **已写入，本机未执行** | `6149443` 新增 CDP 步骤 11b。macOS 无 CDP 通道 |
| A-6 | P2 | **fixed** | `3e4a5dd` |
| A-7 | P2 | **fixed** | `3e4a5dd` |
| A-8 | P2 | **fixed** | `c617f7a` |
| A-9 | P2 | **fixed** | `c617f7a` |
| A-10 | P2 | **不改（写明口径）** | `c617f7a`。改常量会同时移动 §6 基线，收益只是更早丢图；改为把「乐观下界」写进常量注释 |
| A-11 | P2 | **fixed** | `c617f7a` |
| A-12 | P2 | **fixed** | `71814c2` |
| A-13 | P2 | **fixed** | `3e4a5dd`。`bottom-left` 那半条仍为**未证实**（找不到可达输入） |
| A-14 | P2 | **fixed** | `c617f7a` |
| A-15 | P2 | **fixed** | `c617f7a` |
| A-16 | P2 | **fixed** | `c617f7a` |
| A-17 | P1 | **fixed** | `391294f` |
| A-18 | P1 | **fixed（本机未执行）** | `2a5dc64`。只做了 `node --check` 与整脚本冒烟 |
| A-19 | P2 | **fixed** | `208afd9` |
| A-20 | P2 | **fixed** | `208afd9`。三条弱证据用例加固，M6/M7/M8 确认能变红 |
| A-21 | P2 | **fixed** | `7430a71` |
| A-22 | P1 | **fixed（链路级未执行）** | `84325a1`。包模式剧本能裁定了；但步骤 11b 仍差一半，见 §7 |
| A-23 | P2 | **fixed** | `4a578c5` |
| A-24 | P2 | **fixed** | `c264ada` |
| A-25 | P1 | **fixed** | `1df51b7`。完整差异内容进入 packet ID；集成用例先红后绿，确认重切后的 q14 新值重新排队 |
| A-26 | P1 | **fixed** | `1df51b7`。近灰度包页图转灰度并缩至最长边 600 px；真实 HTTP 两模式对比回到显著低于 legacy |

**合计**：fixed 23 条（其中 A-18 / A-22 的实机判定未执行）、未修 1 条（A-2，越界）、不改但写明口径 1 条（A-10）。A-5 已写入 CDP 步骤，但实机仍未执行。

---

## 7. 未执行项、偏离与越界需求

### 7.1 未执行（**没跑 ≠ 通过**）

| 项 | 状态 | 原因 |
|---|---|---|
| §7.3 CDP 链 13/13 + 新增一步 | **未执行** | macOS 无 WebView2 CDP 通道；脚本报 `ENOENT ... ielts-author-studio.exe` |
| §7.3 的步骤 11b「至少一个包走了 L1」 | **结构性未达成** | 见下 |
| §7.4 真实模型验收 | **未执行** | 无额度 |
| A-18 / A-22 的实机判定 | **未执行** | 同 CDP |

**步骤 11b 为什么在包模式下不可满足**（这是本轮最重要的留白）：包模式下 `plan_packets` 会把题组的**锚点页原文**随包发出（`packets.rs:1243-1246`），而 CDP 场景的修复靶子是**题面类**差异（`setResponseGroup` 改写 prompt）——题面行就在题组自己的锚点页上，模型第一轮就拿到了，**没有升级的理由**。所以 11b 在这份场景下不是「模型没敢要」，而是「这一卷的包天然自足」。要让它可满足，只能把场景的修复靶子换成**答案类**差异（答案页不在题组锚点页里，正是 §7.2 那条用例的情形）——属改验收场景的**设计决策**，且本机无 CDP 通道无法验证。审计子代理与我都只做到**源码级判定，未实测**。

### 7.2 偏离 TASK 的地方

1. **基线提交用 `f13c75a` 而不是 `34dfadd`** —— `34dfadd` 在 macOS 上编译不过（`parser.rs:2184` E0425）。第一个提交只改一行、无行为变更。
2. **`REPAIR_CONTEXT_MODE` 默认已是 `packets`**，而 `scripts/` 里没有任何一处设 `IELTS_REPAIR_CONTEXT_MODE`（全仓 grep 确认）。这意味着 CDP 链默认就跑在包模式下——这是 A-18 的根因，也是本报告多处结论的前提。
3. **A-10 不改常量**（只写口径）：改 `PACKET_CHARS_PER_TOKEN` 会同时移动 §6 基线，收益只是更早丢图。

### 7.3 越界需求（等指示，未动手）

1. **`src-tauri/src/cloud_repair/tools.rs`（§8 未授权）** —— 修 A-2 需要把 evidence 的 `quote` 与**完整原文文本层**比对。§2 同时明写「不改 `tools.rs` 的校验与写入逻辑」，两条要求互相矛盾：要么放宽 §2 那句，要么承认「引文校验只到结构层」并把 §2 的描述改准。
2. **`src/api/recognitionClient.ts` / `src/features/editor/recognitionDecisions.ts`** —— 若 A-1 选择在**前端**分开计数才需要；P6 按「改后端」执行，因此**不需要**越界。
3. **`scripts/e2e/tauri-cdp-cloud-repair-chain.mjs` 的调用记录映射（超出「仅新增步骤」）** —— 新增步骤需要读 `packetId` / `escalationLevel` / `pagesIncluded` / `imageCount` / `estimatedInputTokens` / `requestBytes`，而原映射只保留 `{commandName, ok, errorClass, latencyMs}`。改动是**只加字段、不删不改**既有四个。
4. ~~A-4 的真实 HTTP 验证脚本不落仓库~~ —— **已不需要**：§7.2 的真实受控服务用例（`7430a71`）已常驻覆盖。
5. **`tauri-cdp-cloud-repair-chain.mjs` 的步骤 10b / 11 被**改写**（超出「仅新增步骤」）** —— 默认模式已是 `packets`，而这两步写死 legacy 的回合形状，硬断言在包模式下必红（A-18）。改动**没有删除任何原有断言**：legacy 分支逐条保留，包模式分支是新写的等价物；`llmTraces` 只**新增** 5 个字段。
6. **`scripts/e2e/lib/cloud-repair-scenario.mjs`（§8 未授权）** —— 要让 §7.3 在包模式下真正可满足，必须改这份场景：把修复靶子从**题面类**换成**答案类**差异。不改场景的话，步骤 11b 在这份卷子上永远为假，而 §2 又禁止放宽这条断言。**等指示**。可选方案：① 换靶子；② 保留现有场景，另加一份「答案类」场景专门喂包模式的 L1 断言。

### 7.4 仍未证实（写了原因，不算发现）

- **`crop_page_image` 的 `bottom-left` 分支可达**。`bbox` 由 `pdf_ingest/coordinates.rs:137-155 display_rect` 产出、`origin` 恒为 `"top-left"`，`bottom-left` 只出现在 `nativeBBox`，而 `collect_bboxes` / `crop_page_image` 只读 `bbox`。没找到可达输入。

---

## 8. 给下一轮的建议

### 8.1 真实模型验收怎么跑（§7.4）

**素材**：`complex-reading.pdf` + 一份多题组长文（题组 ≥ 2 个、跨页、含答案区，且**至少一条差异的答案页不在题组锚点页上**——否则包天然自足，测不出 L1）。

**怎么接**：在应用的「LLM 配置」里建一个真实 provider profile，把 `IELTS_REPAIR_CONTEXT_MODE` 显式设为 `packets`（不要靠默认值，否则以后默认一改，这次验收的语义就漂了），跑一次导入 → 云端候选 → 校核。

**要记录什么**（全部从 `llm-calls.jsonl` 与 `repair_json` 的诊断区读，不要靠日志猜）：
1. **逐包级别序列**（`packetId` + `escalationLevel`）：确认没有包一步跳到 L3/L4；
2. **`requestBytes` / `estimatedInputTokens` 逐包合计**，与 legacy 同卷对比；本次复核值约为 **4.03×**，其他页图产物与环境需重新测量；
3. **是否主动调用 `report_insufficient_context`**，以及它点名的页是不是真的不在包里——这是「回退真的在传内容」的唯一硬证据；
4. **收敛**：总轮数、是否撞上 `PACKET_MAX_ROUNDS`、终态是 `completed` / `needs_attention` / `budget_exhausted`（三态必须分开看，`needs_attention` 不等于失败）；
5. **重点观察重切语义**：代码回归已覆盖「已完成包中的差异值被其他包改动」这一情形；真实模型验收时仍记录包 ID、重切结果与剩余任务，确认实际裁定和重排行为一致。

**通过标准**：裁定质量不降（对照人工确认的 golden 标注）、每次「上下文不足」都有 `packetId` + 页号 + 理由码可解释、总输入量显著低于 legacy。

### 8.2 以后接「云端优先级」要从哪里入手

**决策层在 `reconcile/`，不在 `cloud_repair/`**——这一点必须先说清，否则容易改错地方：

- `reconcile/rules.rs`：确定性规则层（题目 / 段落 / 选项 / 答案位 / 来源覆盖），规则不足以结论时给 `Unverifiable`，**不猜**。
- `reconcile/adjudicate.rs`：本地与云端分歧的裁决。改「云端优先还是本地优先」的**策略**应该落在这里，而不是散在调用点。
- `reconcile/engine.rs`：三路结果 → 一份统一裁决 → 持久化的编排（只做编排，不做判断）。
- `processing/scheduler.rs`：**调度器阶段顺序**与模型调用预算在这里（`MAX_ADJUDICATION_MODEL_CALLS` / `MAX_CONSTRAINED_REPAIRS` 定义在 `schema/recognition_v1.rs:28,30`，由 scheduler 消费）。
- `reconcile/candidate.rs`：`normalize_cloud_authoring`（:2588） / `align_cloud_answer_shapes`（:392）。**注意**：`cloud_repair` 的差异比较（`candidate_differences`）读的就是这份归一化后的候选，所以归一化语义一改，校核包的切分也会跟着变——两条链路共享同一个前提。

**接口建议**：把「优先级」做成 `adjudicate` 的**显式入参**（而不是读全局配置），这样 `cloud_repair` 的裁定语义与 reconcile 的裁决语义可以分别测试、分别演进。

### 8.3 接原生 tool call 要从哪里入手

现在整条修复链是**应用层 JSON 工具消息**：模型回一段 JSON → Rust 派发 → 真实结果下一轮回填。没有用 provider 的原生 tools。

要换的话，改动点集中在四处（顺序即依赖顺序）：

1. `schema/cloud_repair_v1.rs::CLOUD_REPAIR_TOOLS`（11 个工具）——变成 provider 的 tool schema；
2. `llm_gateway.rs::run_openai_compatible_repair_step_llm`（~1714）——请求里带上 `tools`，响应里解析 `tool_calls` 而不是从文本里抠 JSON；
3. `cloud_repair/mod.rs::parse_tool_call`（~1287）——同一个入口，但输入从「文本里的 JSON」换成「原生 tool_call 结构」；
4. `llm_gateway.rs::repair_step_prompt`（~1611）——把「信封形状」那几段说明换成工具描述。

**不要动的**：`execute_tool` 的派发与校验、`apply_cloud_edits` 的事务与 CAS、`report_insufficient_context` 的语义。原生 tool call 只换**传输形态**，不换纪律——尤其「上下文不足不得变成猜一个」这条必须在换完之后仍然成立。

**风险提示**：`findings.md` 的 F-REAL-MODEL 节记过「prompt 没写信封键」的亏。换成原生 tools 之后，这类坑会从「prompt 漏写」变成「schema 与校验器不一致」——**照旧要逐项对照校验器核对**，别只看模型能不能调通。

### 8.4 接 prompt cache 要从哪里入手

TASK §4.3 已经把前提写好了：每包请求 = **固定前缀**（规则 + 工具表 + 本包证据，稳定顺序）+ **包内** observation，换包清空、不跨包累积。这是为 cache 准备的。

要真正吃到 cache 收益：

1. **`llm_gateway.rs::repair_step_prompt`** 的前缀部分必须**逐字节稳定**——任何随包变化的内容（包 id、页号、计数）都不能进前缀，只能进后缀；
2. **`cloud_repair/packets.rs` 的包内字段顺序**要固定（现在 `serde_json::Value` 的插入顺序基本稳定，但 `BTreeMap` / `BTreeSet` 的迭代顺序要显式确认）；
3. **provider 侧**：OpenAI 兼容接口的 cache 命中依赖前缀完全一致，`system` / 第一条 `user` 消息放固定前缀最稳；
4. **量测**：在 `llm-calls.jsonl` 的调用记录里加 `cachedPromptTokens`（如果有），否则「以为吃到了 cache」无法证伪——这正是本分支吃过的那类亏（**没有数字就没有证据**）。

### 8.5 其他

- **A-2 是个真问题**，不是理论问题：`apply_edits` 带一条凭空编造的 `quote` 会被判 `Applied`。§2 说「校验仍对照完整原文」，实现里没有。要么改实现（越界需求 1），要么把 §2 那句话改准——**两选一，不能两头都不动**。
- **§7.3 的场景调整**（越界需求 6）建议选方案 ②：保留现有题面类场景，**另加**一份答案类场景专门喂包模式的 L1 断言。理由是换靶子会同时改变 legacy 分支的回合形状，而 legacy 分支现在逐条保留着改造前的断言，是重要的回归对照。
- **`PACKET_CHARS_PER_TOKEN = 4` 对中文低估 3–4 倍**（A-10，未改）。接真实模型时如果遇到「单包超 24k 预算」的告警，先怀疑这个系数，而不是怀疑预算本身。

---

## 9. 收尾轮（P9-Q / P10 / P11，HEAD `542e660`）

> 本轮在 P8 报告之后执行三项收尾：A-2 的授权落地（引文核验）、答案类场景（L1 抓取路径）、多包总时限。进度真源仍是 `STATE.md`。

### 9.1 用户能感知到的变化

1. **「云端说它改对了」现在必须拿原文对得上号的证据**（A-2 落地）。以前 `apply_edits` 带一句凭空编造的 `quote` 也会被判 `Applied`；现在每条证据引文都要在**完整原文**的文本层里逐字找到（规范化空白/弯引号/连字符、忽略大小写；声明页与实际页允许 ±1 的跨页差），找不到整批拒绝、错误码点名是哪一条（`CLOUD_EDIT_EVIDENCE_QUOTE_NOT_IN_SOURCE:<index>`）。裁定（record_ruling）走同一套校验——编造引文的裁定不予记录。原文没有文本层（扫描件）时不拒绝，但每条证据被如实标为 `unverifiable`：裁定标记随裁定记录落盘，编辑侧计入运行摘要（`repair_json.unverifiedEvidence`），**不算已核验**。
2. **「模型自己去取材料」这条路有了可执行的证明**。新增答案类场景：某题答案错了、正确答案印在题组锚点页之外的答案页上——包第一轮拿不到它，模型用 `read_source` 抓取之后才改对。真实 HTTP 用例硬断言：第一轮请求里没有答案行（模型此时不可能知道答案）、后续轮带着抓取结果、最终答案正确、全程无整份 PDF。与既有 `report_insufficient_context` 路径互补，L1 的两条腿都有了链路级证据。
3. **多包校核不再轻易撞上十分钟墙**。包模式总时限按包数放宽（每包 +25% 的基础时限，封顶 3×——基础 10 分钟 ⇒ 最多 30 分钟）。10 个包的受控用例：旧时限下跑到第 5 个包就 `budget_exhausted`，放宽后 11 轮全部完成、状态 `completed`。包内仍严格串行；CAS 串行写入不变——后到的包用过期版本提交会被拒（`EDIT_VERSION_CONFLICT`），用重切后刷新的版本重试才能落地，先到包的修改不会被覆盖。

### 9.2 提交列表

| # | 提交 | 内容 |
|---|---|---|
| 33 | `b6c0370` | **P9-Q（A-2 落地）**：引文对照完整原文核验（apply_edits + record_ruling）；无文本层标 unverifiable；prompt 与剧本引文同步；TASK.md §2 描述修正 |
| 34 | `a8e1678` | **P10**：受控服务 `answerFetch: read_source` 分支；场景库 `deriveAnswerRepairScenario`（纯新增）；CDP 链答案场景派生步骤（独立 `answerScenario` 三态段）；Mac 端真实 HTTP 抓取路径用例 |
| 35 | `542e660` | **P11（方案 b）**：包模式总时限按包数线性放宽（+25%/包、封顶 3×）；10 包时限用例；跨包写冲突回归用例 |

### 9.3 逐项结论的证据等级

| 结论 | 等级 | 具体是什么 | **不是**什么 |
|---|---|---|---|
| 编造引文整批拒绝、真实引文（含空白/弯引号/连字符/大小写差异）通过、页差 ±1 放行 / 差 2 页拒绝、无文本层标 unverifiable | 命令处理器层（含真实 HTTP） | tools.rs 写入层 7 条单测 + 3 条循环级用例（其中真实 HTTP 链路用例覆盖 apply_edits 拒绝→重试落地）；record_ruling 同一套校验由链路用例覆盖 | 不是产品 UI 端到端；「引文能证明答案正确」的**语义**判断仍不存在——核验只保证引文真实存在于原文 |
| 答案类场景走 L1 抓取路径、最终答案正确、无整份 PDF | 命令处理器层（含真实 HTTP） | 真起 node 跑 `scripts/controlled-llm-service.mjs`（真 HTTP），逐包 `llm-calls.jsonl` 与受控服务请求日志双重对账 | 不是 CDP 产品端到端（Windows only） |
| CDP 链答案场景步骤 | **未执行** | macOS 无 WebView2 CDP 通道；且当前唯一 golden 无答案页标注（`answerKeyAbsence`），答案场景在该 fixture 上结构性 not-executable | 步骤已写入并做派生冒烟（node 4 分支全过），实机判定留待 Windows + 带 `answerErrors` 标注的 fixture |
| 多包总时限放宽后 10 包能完成；跨包写冲突被 CAS 裁决 | 命令处理器层 | 受控（脚本内）循环 + 真实 SQLite 写路径；时限缩短模拟真实延迟 | 不是真实模型时长的实测（16–50s/轮的换算见 8.1 的真实模型验收清单） |
| 全量门禁数字 | 命令处理器层 + 单测 | 见 9.5 | — |

### 9.4 亲眼看到「先红后绿」的测试

| 项 | 用例 | 看到红时的实际输出 |
|---|---|---|
| P9-Q | `a_fabricated_quote_rejects_the_whole_batch_and_names_the_entry`、`a_quote_two_pages_away_from_the_declared_page_is_rejected`、`evidence_without_a_text_layer_is_marked_unverifiable_not_rejected_or_verified` | 把核验临时 stub 成旧行为（只查结构）后 3 条全红：编造引文被判 Applied、差 2 页被判 Applied、`evidence_unverifiable` 为空（`left: [] / right: [0]`）；还原后全绿 |
| P9-Q（链路） | `an_edit_with_a_fabricated_quote_is_rejected_and_a_real_quote_lands`、`a_ruling_with_a_fabricated_quote_is_rejected_and_a_grounded_one_is_recorded`、`rulings_recorded_without_a_text_layer_carry_the_unverifiable_mark` | 随实现一并落地后绿；对旧行为的红由上面 3 条 stub 运行代表（同一核验函数） |
| P10 | `an_answer_difference_fetches_the_answer_page_through_read_source_before_fixing` | stash 掉受控服务的 `answerFetch` 分支后红：观察里没有任何 `read_source` 结果（服务走了 `report_insufficient_context` 老路径）；恢复后绿 |
| P11 | `ten_packets_with_fixed_round_delay_finish_within_a_deadline_scaled_to_the_packet_count` | 临时禁用换算（ratio=1）后红：2.5s 内只处理 5 包即 `budget_exhausted`（`left: 5 / right: 11`；复核轮变异 M3 独立复现 `left: 4 / right: 11`）；放宽后 11 轮全部完成 |
| P11（回归） | `a_conflicting_edit_from_a_later_packet_is_rejected_and_recovers_without_overwriting` | 锁定既有 CAS 行为（预期常绿）：确认后到包收到 `EDIT_VERSION_CONFLICT:current=2:base=1`、重试落地、先到包修改保留 |

复核子代理的变异检查（独立于上表，全新执行）：M-新1 删连字符规范化 → `a_real_quote_survives_...` 红；M-新2 把页一致性判断放宽成「任何页出现即放行」 → `a_quote_two_pages_away_...` 红；M-新3 时限比例置 1 → `ten_packets_...` 红。三处全部还原（`git diff --name-only -- src-tauri scripts` 复空）。

### 9.5 全量数字与基线对比

| 门禁 | P0 复核基线（`24eaa98` 前，即 1111 那轮） | 本轮（HEAD `542e660`） | 变化 |
|---|---|---|---|
| `cd src-tauri && cargo test --lib` | 1111 passed / 0 failed / 11 ignored | **1125 passed / 0 failed / 11 ignored** | 净 **+14** |
| `cargo test --lib cloud_repair` | 117 passed | **130 passed** | 净 **+13** |
| `npx vitest run` | 392 passed（1 suite 因缺 `selenium-webdriver` 加载失败） | **392 passed**（27 suites passed，同一 suite 加载失败） | 无变化（已知环境基线问题） |
| `npx tsc --noEmit` | `exit 0` | **`exit 0`** | 无变化 |
| §6 两模式对比（本轮重跑） | 917857 B / 227766 B / 12480 tokens（历史轮） | legacy **919691 B**，packets **88447 B**（≈ **10.4×**），packets tokens **12998** | 结论不变；本轮本机未生成整页页图，与历史 227766 B 的差异属环境差异 |

### 9.6 未执行项及原因

| 项 | 状态 | 原因 |
|---|---|---|
| CDP 链实机（含答案场景步骤） | **未执行** | macOS 无 WebView2 CDP 通道 |
| CDP 答案场景的端到端判定 | **结构性未达成** | 当前唯一 golden 明确标注 `answerKeyAbsence`（原文件没有答案页）；需要一份带答案页的原文件 + 对应 `answerErrors` 人工标注 golden。派生函数与链路步骤已就绪（冒烟 4 分支全过），fixture 落库后即可切换为判定步骤 |
| 真实模型验收 | **未执行** | 无额度 |
| `editor_journal_v1` 携带证据核验标记 | **未做（越界）** | `library/repository.rs` 不在本轮授权文件内；edits 的 unverifiable 标记如实落在工具结果、逐包诊断与 `repair_json.unverifiedEvidence`，rulings 的随裁定 artifact 落盘 |

### 9.7 偏离本任务之处及理由

1. **STATE 阶段编号**：STATE 表里已有一个前任的 P9（A-25/A-26），为避免编号冲突，本轮引文核验记作 **P9-Q**，其余沿用 P10/P11，收尾记 P12。
2. **P10 的 Mac 等价证据选了「抓取工具路径」**：既有 `the_real_controlled_service_..._l0_l1_and_finish` 已覆盖 `report_insufficient_context` 路径的同一组断言；新用例专门钉住 `read_source` 抓取路径（任务书「report_insufficient_context **或** 抓取工具」的另一条腿），并额外断言 `insufficientContext == 0` 以区分两条路径。
3. **CDP 答案场景判定段的落位**：写进独立 `report.answerScenario`（三态）而非 `report.scenarios`——当前 fixture 上该场景必然 not-executable，若计入 verdict 会把题面类验收一起拖成 not-executable（等于用一个缺失的 fixture 否掉整条链）。切换条件写在脚本里。
4. **P11 二选一选了方案 b**（时限放宽）而非方案 a（不相交包并行 ≤2）：并行会让重切、done_packets、升级预算、L3 计数这些单线程循环状态进入并发路径，CAS 之外的不变量风险大；方案 b 改动集中一处、直接命中真实瓶颈，且保持「包内串行 + CAS 串行写入」的全部既有语义。选择理由已按任务书要求记入 STATE。
5. **A-2 状态更新**：REPORT §6 的 A-2 从「未修（越界）」改为 **fixed（`b6c0370`）**，越界需求 1 解除；TASK.md §2 的描述同步修正为已实现语义并注明历史差距。

---

## 10. 缺陷修复轮（P12-Q，HEAD `b1cd8f3`）

> 第三方复核发现 P9-Q 的引文核验有两处缺陷；本轮修复并经全新对抗式复核放行。

### 10.1 用户能感知到的变化

1. **扫描答案页 / 图片答案表不再制造假拒绝**。这些页（正是 L2 整页图与 `read_page_region` 交给模型的页）在文本层里缺席或只剩页码噪声；模型从**页图**里读到的真实引文，以前会被当成编造整批拒掉。现在：引文全文都找不到、而声明页（±1 相邻页中任一页）没有有效文本层（规范化文本薄于 8 字符，理由见常量注释）⇒ 标 `unverifiable` 放行，如实计入 `evidenceUnverifiable` / 运行摘要 `unverifiedEvidence`，不算已核验。声明页与相邻页都有真实文本层时照拒；声明页超出文档末尾（编造页号）照拒；引文在别页找得到、只是页号归属不对时**照拒**（复核指出的「实现宽于声明」已收紧，退让只属于「全文都找不到」）。
2. **单独上传的答案文件的证据不再被误拒**。链路调查确认：修复链的证据面只来自主试卷（`MainQuestion`），答案文件（`AnswerKey` role）只进本地识别管线，修复循环任何一环都看不到它。因此非主试卷 sourceFileId 的证据核验不了也不该拒——一律标 `unverifiable`（方案 b，理由记入 STATE）；真实存在于答案文件里的引文不会被误拒。prompt 两种模式都写明这两类「不拒绝、标 unverifiable」的情形。

### 10.2 提交列表

| # | 提交 | 内容 |
|---|---|---|
| 36 | `2faa456` | **P12-Q**：无文本层页退让（阈值 + existing_pages）；非主试卷 sourceFileId 标 unverifiable；受控服务 `answerFetch: read_page_region` 分支；端到端用例；prompt 契约扩展；TASK §2 两行描述对齐 |
| 37 | `b1cd8f3` | **复核收紧**：无文本层退让仅限「引文全文都找不到」；钉住用例先红后绿；u32 截断加固 |

### 10.3 证据等级

| 结论 | 等级 | 具体是什么 | **不是**什么 |
|---|---|---|---|
| 无文本层页上的引文标 unverifiable 放行；有文本层照拒；超界页号照拒；别页找得到照拒 | 命令处理器层（含真实 HTTP） | tools 写入层 5 条用例（两变体无文本层 / 阈值护栏 / 超界页 / 别页找得到 / 非主试卷）+ 端到端用例 | 不是产品 UI 端到端 |
| 非主试卷 sourceFileId 标 unverifiable | 仅单元测试（写入层） | `evidence_from_a_non_main_source_file_is_marked_unverifiable_not_rejected`；链路「证据面只来自主试卷」为源码级判定（`main_source_for_cloud` / `load_source_index` / legacy 附件路径核对） | 没有真实多源作业的端到端样本 |
| 页图抓取路径端到端 | 命令处理器层（含真实 HTTP） | 真起 node 走 `read_page_region` → 引用 → 落库 + 三处 unverifiable 呈现 | 受控服务读不了像素：答案值取自请求内候选切片（请求文本），真实模型「从图读值」的环节无法模拟，如实记录 |
| 全量门禁 | 见 10.5 | — | — |

### 10.4 先红后绿

| 用例 | 红的形态 |
|---|---|
| `a_quote_read_from_a_textless_page_is_marked_unverifiable_not_rejected` | 旧实现把两变体（无文本层 / 只剩页码噪声）都判 `Rejected` |
| `evidence_from_a_non_main_source_file_is_marked_unverifiable_not_rejected` | 旧实现拿主试卷文本核验非主试卷证据 → 误拒 |
| `an_edit_quoting_an_image_only_answer_page_lands_and_is_marked_unverifiable` | 临时禁用退让分支：`left: ["B"] / right: ["A"]`（页图引文被拒、编辑没落库） |
| `a_quote_found_on_another_page_is_rejected_even_when_the_declared_page_is_textless` | 收紧前：`Applied` vs 期望 `Rejected`（复核发现的「宽于声明」） |

复核子代理变异（2 处，独立执行、精确还原）：M1 阈值比较方向翻转 → `a_missing_quote_on_a_page_with_a_real_text_layer_is_still_rejected` 红（`Applied vs Rejected`）；M2 `same_source_file_id` 恒真 → `evidence_from_a_non_main_source_file_...` 红（`[] vs [0]`，被误判「已核验」）。

### 10.5 全量数字与上一轮对比

| 门禁 | P9-Q/P10/P11 收尾轮 | 本轮（HEAD `b1cd8f3`） | 变化 |
|---|---|---|---|
| `cd src-tauri && cargo test --lib` | 1125 / 0 / 11 | **1131 / 0 / 11** | +6 |
| `cargo test --lib cloud_repair` | 130 | **136** | +6 |
| `npx vitest run` | 392 passed（1 suite 缺 `selenium-webdriver` 加载失败） | **392 passed**（同一 suite 同因失败） | 无变化（环境基线） |
| `npx tsc --noEmit` | `exit 0` | **`exit 0`** | 无变化 |

### 10.6 未执行项及原因

- CDP 实机（含页图路径的产品端验证）：macOS 无 WebView2 通道，未执行。
- 真实模型验收：无额度，未执行。
- 真实多源（试卷 + 答案文件）作业的端到端样本：仓库无此 fixture，缺陷 2 只有写入层证据 + 链路源码级判定，如实标「仅单元测试」。

### 10.7 偏离本任务之处及理由

1. **复核发现的「实现宽于声明」当场收紧**（`b1cd8f3`）：任务书规定退让条件是「引文在全文其他地方也找不到」，初版实现把「找得到但页号差 >1」也退让了；按「不放宽校验」纪律改为 `found.is_empty()` 才退让，并补钉住用例（先红后绿）。
2. **缺陷 2 选方案 b**（非主试卷一律 unverifiable + prompt 说明），理由与链路调查证据记入 STATE「P12-Q 日志」；方案 a（接入答案文件文本）会让核验面超出修复链实际交给模型的证据面。
3. 阈值常量 `MIN_TEXT_LAYER_CHARS = 8` 的语义边界：混合页（既有正文文本又有图片答案区）的文本层会超过阈值，其上的引文仍按「有文本层」核验——阈值只覆盖「整页没有有效文本层」的情形，与任务书口径一致。
