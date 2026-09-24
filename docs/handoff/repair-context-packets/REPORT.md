# 收口报告 — 校核链路上下文管理（分支 `feat/repair-context-packets`）

> 分支基线 `wave3-listening` @ `34dfadd`，**未 push**。进度真源是同目录 `STATE.md`；本文件是它的收口摘要，冲突时以 `STATE.md` 为准。
> 报告里的每个数字都标了**证据等级**。等级低的证据**不**用来支持等级高的结论（`AGENTS.md` 的硬要求）。

---

## 1. 用户能感知到的变化

### 1.1 一次校核少发了多少内容

同一份作业、同一条真实 HTTP 网关路径、同一份 212 KB 真实 PDF 样本，跑两种上下文模式：

| 指标 | legacy（改造前） | packets（现在，默认） | 变化 |
|---|---|---|---|
| 整轮 `requestBytes` 合计 | **917173** | **83738** | **≈ 9.1%，缩小 10.95 倍** |
| 逐包 `estimatedInputTokens` 合计 | 0（没有「包」这个概念） | **12956** | — |
| 每轮是否附整份 PDF base64 | 是（每轮都附） | **否**；只有升到 L3（每次运行最多 1 次）才退回整份附件 | — |

用例：`cloud_repair::tests::packets_mode_sends_much_less_input_than_legacy_for_the_same_paper`（本机实测，`--nocapture` 打印）。

同一份样本在 P4 首次量到的是 83531 B，现在是 83738 B，差的 207 B 来自 A-8 的修复（`paperMap` 每页多一行 `paragraphLabels`）——是**有意**增加的信息，不是回归。

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

`git log --oneline 34dfadd..HEAD --reverse`（22 个提交，**未 push**）：

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

---

## 3. 证据等级

| 结论 | 等级 | 具体是什么 | **不是**什么 |
|---|---|---|---|
| 一次校核少发 10.95 倍内容 | 命令处理器层（含真实 HTTP） | 真实网关代码 + 真实请求体落盘 + 212 KB 真实 PDF 样本 | 不是产品端到端；没有 UI，没有真实模型 |
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

| 门禁 | 基线（`f13c75a`） | 本分支（`a6c75a0`） | 变化 |
|---|---|---|---|
| `cd src-tauri && cargo test --lib` | 1046 passed / 0 failed / 11 ignored | **1102 passed / 0 failed / 11 ignored** | 净 **+56** |
| `npx vitest run` | 391 passed（28 文件里 1 个加载失败） | **392 passed**（同样 1 个文件加载失败） | 净 **+1** |
| `npx tsc --noEmit` | `exit 0` | **`exit 0`** | 无变化 |

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

**合计**：fixed 21 条（其中 A-18 / A-22 的实机判定未执行）、未修 1 条（A-2，越界）、不改但写明口径 1 条（A-10）。

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

- **`packetId` 不含内容指纹，重切后可能命中 `done_packets` 而被跳过**。id 只由「本地题组 + 差异键 + 阻断问题 id + 是否文档包」派生，而差异键 `(targetType, targetId, field)` 在**值变化时不变**。构造「键不变而值变、且该包已在 `done_packets` 里」需要一次真实 `apply_edits` 造成候选侧值变化——两轮都没稳定复现。**如果成立，后果是「重切后漏核」，属 P0 级**；它没有被证伪，只是没有被证实。
- **`crop_page_image` 的 `bottom-left` 分支可达**。`bbox` 由 `pdf_ingest/coordinates.rs:137-155 display_rect` 产出、`origin` 恒为 `"top-left"`，`bottom-left` 只出现在 `nativeBBox`，而 `collect_bboxes` / `crop_page_image` 只读 `bbox`。没找到可达输入。

---

## 8. 给下一轮的建议

### 8.1 真实模型验收怎么跑（§7.4）

**素材**：`complex-reading.pdf` + 一份多题组长文（题组 ≥ 2 个、跨页、含答案区，且**至少一条差异的答案页不在题组锚点页上**——否则包天然自足，测不出 L1）。

**怎么接**：在应用的「LLM 配置」里建一个真实 provider profile，把 `IELTS_REPAIR_CONTEXT_MODE` 显式设为 `packets`（不要靠默认值，否则以后默认一改，这次验收的语义就漂了），跑一次导入 → 云端候选 → 校核。

**要记录什么**（全部从 `llm-calls.jsonl` 与 `repair_json` 的诊断区读，不要靠日志猜）：
1. **逐包级别序列**（`packetId` + `escalationLevel`）：确认没有包一步跳到 L3/L4；
2. **`requestBytes` / `estimatedInputTokens` 逐包合计**，与 legacy 同卷对比，确认倍数关系在本机复现（参考值 10.95×）；
3. **是否主动调用 `report_insufficient_context`**，以及它点名的页是不是真的不在包里——这是「回退真的在传内容」的唯一硬证据；
4. **收敛**：总轮数、是否撞上 `PACKET_MAX_ROUNDS`、终态是 `completed` / `needs_attention` / `budget_exhausted`（三态必须分开看，`needs_attention` 不等于失败）；
5. **重点观察 §7.4 的那条未证实项**：一次真实 `apply_edits` 之后，重切出的包 id 有没有因为「差异键不变」而落在 `done_packets` 里被跳过。做法是在 `plan_repair_packets` 的过滤处临时打印 `done_packets` 与 `next` 的 id 集合做差，比对「差异仍存在但包不再排队」的包。

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
