# C1–C4 验收结论 + 阅读真实链在 Windows 上的红灯（2026-09-25）

> 给执行 agent（cloud-repair / repair-context-packets 线）。工作约定沿用 `docs/handoff/2026-09-22-execution-prompt.md`：
> 先红后修、频繁提交、**不要 push**。

---

## 1. C1–C4：通过（质量方独立复跑，HEAD `5a80786`）

- `cargo test --lib` 连跑 3 次：**1163 / 0 / 11**；vitest 412 / 28 个文件；tsc 干净。
- C2 先红抽查：只回退 `parser.rs` 的实现，`a_failed_render_keeps_the_existing_page_image_cache` 变红（好缓存被覆盖成失败结果）；恢复后转绿。
- 代码审查：
  - `pdf-images.json` 只有两个读取方，两者都只认「有页图」的缓存，所以失败时不写盘不会影响任何读取方；
  - Python sidecar 不会清空页图资产目录；
  - 包循环里已经没有墙钟判定（剩下的 `Instant::now()` 在 legacy 循环里，按任务书保持不动）。
- 真实 App（默认产品档）：听力链 **12/12**，单文件导入**通过**。

## 2. 新问题：阅读真实链在 Windows 上不是全绿

你们线的 `STATE.md` 记着：步骤 11b `packet-mode-asked-for-the-missing-page` 以及改写过的 10b / 11，**在 macOS 上从来没执行过**。
质量方在 Windows 默认档实跑（`--no-diagnostic-args`）的结果如下：

| 构建 | `packet-mode-asked-for-the-missing-page` | `remaining-tasks-match-backend-and-are-actionable` |
|---|---|---|
| `f6c49f0`（C1–C4 之前） | **FAILED** | passed |
| `5a80786` 第 1 次 | **FAILED** | FAILED |
| `5a80786` 第 2、3 次 | **FAILED** | passed |

其余场景全部通过，断线重连 0 次。`derive-answer-scenario` 仍然是 NOT_EXECUTABLE（fixture 里没有答案类错误），这一点和你们的记录一致。

### W1【高】`packet-mode-asked-for-the-missing-page`：确定性红灯，而且 C1 之前就存在
- 证据：`artifacts/e2e-cdp/run-cloud-repair-chain-2026-09-25T20-53-04-139Z/report.json`。3 个包的升级级别都是 0，没有任何包走到 L1：
  - 包 1：第一次请求 `firstCallPages: []`（一页原文都没带），调用 1 次，没有要材料就结束了；
  - 包 2、3：第一次请求就已经带着承载正确答案的第 4 页。
- 先回答一个问题：**包 1 一页原文都没有、却没有要材料就结束了，这算不算缺陷？**
  - 如果它的差异确实需要原文，那就是缺陷：包里缺了该有的页，或者受控模型的剧本在「材料不够」时没有走 `report_insufficient_context`。按缺陷修，先红后修。
  - 如果它的差异确实不需要原文，那就是这份卷子派生不出「非走 L1 不可」的场景。这时要改的是**场景派生**：
    比如构造一条修正页落在包范围之外的差异，或者换一份卷子。
- **不许**把断言改成恒过，也不许把它悄悄挪出 verdict。
  你们 `TASK.md` §134 写明的验收要求是「至少一个包走了 L1，且最终稿正确」，必须在真实 App 里被证明一次。
  如果确实派生不出来，就照 `derive-answer-scenario` 的做法标成 NOT_EXECUTABLE 并写清原因，同时在报告里写明这条验收**尚未达成**。

### W2【中】`remaining-tasks-match-backend-and-are-actionable`：偶发（4 次里挂 1 次）
- 证据：`artifacts/e2e-cdp/run-cloud-repair-chain-2026-09-25T20-44-46-996Z/report.json`，现象是「后端的 2 条剩余任务
  `cloud-question:group-1:0` 和 `cloud-question:group-1:1` 没有被清单接住」，当时后端共剩 17 条。
- 查清楚是 harness 读早了（后端又写了一批任务、界面还没刷新），还是产品的清单刷新丢了更新。
  是后者就按产品缺陷修；是前者就让脚本等后端写完再比较，**不要**放宽比较规则。
- 修完后连跑 5 次都要通过。

### W3 运行环境
- 你们在 macOS 上跑不了 CDP 链。所以凡是改动 `scripts/e2e/tauri-cdp-*.mjs` 的提交，回报里都要写明「未执行，交质量方实跑」，
  不能写成「保持 13/13」。

## 3. 边界
不放宽网关校验；不改其他场景的断言；不碰听力、发布、门禁区域和 `customer-delivery/`；不 push。

## 4. 质量方收货标准
- 阅读真实链在质量方 Windows 默认档连跑 3 次：除明确标注的 NOT_EXECUTABLE 外全部通过。W1 要么真正通过，要么如实标成「验收未达成」。
- `cargo test --lib` 0 失败；vitest、tsc 干净；听力链 12/12 保持。
