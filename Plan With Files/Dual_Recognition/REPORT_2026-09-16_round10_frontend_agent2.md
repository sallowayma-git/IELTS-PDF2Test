# 前端 agent2 第十轮报告 —— 验收判定、去重身份、持久化撤销

日期：2026-09-16　范围：前端 + E2E（不自行修改后端门禁）

---

## 0. 一句话结论

本轮六项任务里 **1/2/3/4 已完成并有真实运行证据**；
**5 的原因未能查明**（不编解释），但已重建并重新验证去重代码，两个夹具逐键计数**完全一致**；
**6 仍被后端阻塞**（无真实候选、发布仍被 `WORD_LIMIT_UNPARSED` 误判拦下）。

本轮最重要的一件事：**我上一轮自己的修法是错的，而且方向相反**。
上一轮按门禁 `code` 去重，会**整条吞掉**一个阻断问题；改成按「根因 + 目标」去重后，
又**吞掉了 group-2 的第二条事实**。两次都是「用了一个不够细的键，并把它当成权威身份」。
现在改成显式的 `sameFact()` 判据，并明确写成**权宜之计**（等后端稳定事实 id）。

---

## 一、任务 1：完整链每个必需步骤必须明确 passed —— 已完成

### 洞在哪里

`computeChainVerdict` 原先只查三样：`missing`（步骤名不在数组里）、
`failed`（`status === "failed"`）、`blocked`（`status === "blocked"`）。
状态是 `skipped` / `not-executable` / `pending`，或者干脆**拼错**的必需步骤，
三样都不占，**直接走到最后的 `passed`**。

最坏的一条路径：发布被门禁正确拦下 + 另一步 `skipped` → `otherBlocked.length === 0`
成立 → 判 `passed-negative-case`（**退出码 0**）。负例模式因此会变成一条永远绿色的通道。

### 修法

- 新增 `notOutcome`：必需步骤里状态**不在** `{passed, failed, blocked}` 的。
  必须排在 `blocked` / 负例**之前**判。
- `not-executable` → `not-executable`(5)；其余（`skipped`/未知/`undefined`）→ `incomplete`(2)。
- 最后加一条自检 `passed.length === required.length`：将来规则被改坏也会被拦住。
- **不是白名单枚举**：列出的是「有结论」的三个状态，其余一律按「没有明确结论」处理，
  免得将来冒出第四个状态又漏过去。

### 证据

| 项 | 值 |
| --- | --- |
| 单测 | `scripts/e2e/lib/chain-verdict.test.mjs` **26 → 33**（+7，每条钉一个漏状态入口） |
| 真实链（默认） | `run-chain-2026-09-16T09-59-15-045Z` → `verdict=blocked exit=4`（**不是 passed**） |
| 真实链（负例） | `run-chain-2026-09-16T09-58-42-267Z` → `verdict=passed-negative-case exit=0` |

默认模式 11 步里 10 步 `passed`、发布 `blocked`；负例模式同样，判定词与退出码都对。
真实 recorder 只产出 `passed/blocked/failed`，所以这条是**纯护栏**，不改变现有行为。

---

## 二、任务 2：过期候选必须验证实际 outcome、用户内容及重开持久化 —— 已完成

### 洞在哪里

`stale-suggestion-protected` 原先在「接受按钮不存在」时**直接 return 成功**：

```js
if (!hasAccept) {
  return { ..., acceptAvailable: false, note: "旧建议已不可接受（过期/已处理），用户修改未被覆盖" };
}
```

既没读值、也没看决策状态、更没重开。按钮消失可能只是面板渲染问题，
而权威稿里用户的值**可能已经被覆盖** —— 那样这个场景会给出**假绿**。

### 修法：三条都必须成立，缺一条判失败

1. **用户内容**：读权威稿比对（接受尝试之后、**重开之后**各比一次）；
2. **实际 outcome**：后端 `stale` 必须为真 —— 它由 `baseEditVersion` 与当前版本比较得出，
   是**后端事实**，不是按钮可见性；且该决策有确定的 `resolution`（查不到 = 失败）；
3. **重开持久化**：① ② 与 `resolution` 重开后一致。

`acceptAvailable` 降级为**线索记录**，不参与判定；返回值里加
`protectedEvidence: "user-value-intact + backend-stale-flag + survives-reopen"`，
把「通过依据是什么」写在报告里。

### 证据

`run-recog-buttons-2026-09-16T09-59-46-527Z` → 5 个场景全部 `not-executable`，
`verdict=not-executable exit=5`。**场景本身仍未执行**（无真实候选），
所以本节交付的是**断言与操作流程**，不是场景通过 —— 任务 6 交付后才能真跑。

---

## 三、任务 3：问题去重必须保留不同事实 —— 已完成（含对我自己上一轮的更正）

### 两个相反方向的错误

| # | 去重键 | 后果 | 实测 |
| --- | --- | --- | --- |
| F-R9-12 | `门禁 code + 目标` | 门禁把**所有**质量码塞进 `ISSUE_UNRESOLVED` 一个 code，两个不同根因共用 `ISSUE_UNRESOLVED:document` → **整条吞掉一个根因** | `complex-reading.pdf`：门禁 8 条 blocker，界面只剩 **3 行**，`SIGNIFICANT_REGION_UNASSIGNED` **完全不可见** |
| F-R9-13 | `根因 + 目标` | group-2 上两条**不同事实**共用同一个 `issueId` → **吞掉第二条事实** | 上一轮我改出来的新错误 |

### 最终判据 `sameFact()`

- **跨来源**（本地闭包 vs 门禁）同根因同目标 = 同一件事。两个子系统各写一句文案是
  **设计如此**（本地「第 27 题还没有答案。」vs 门禁「这道题还有答案没有填写。」），
  文案不同**不能**当两条事实 —— 否则同一道题白占两行（实测 14 道题多 14 行）。
- **同来源**还要 `userMessage` 也相同才算同一条；文案不同就是两条事实，**两条都留**。

**按任务书要求，明确写成权宜之计**：真正的身份应由后端给出稳定的「事实 id」
（现在 `issueId` 拼成 `phase4-{code}-{target}`，**不是唯一键**），
`resolution` 按它继承也是不安全的。代码注释与 `findings.md` F-R9-11/F-R9-13 都写明了这一点。

### 验收同时检查两个方向

校验脚本的**核心断言**改成**逐「根因 + 目标」的渲染行数 = 独立算法期望**：

- 行数**多**了 = 同一问题重复显示；
- 行数**少**了 = 不同问题被隐藏。

比的是逐键计数而不是总行数 —— 总数对得上也可能是「吞掉一条、同时多算一条」。
另加一条「本地来源的行数 = 期望」防止用**多删**满足上面那条。

### 证据（重建后二进制，`exeSha256 a66905a7ad9f…`）

**PDF 夹具** `run-issue-list-2026-09-16T09-57-25-608Z` → **7/7 断言通过，exit=0**

```text
gate raw=34 warnings=1 expectedLocal=14 expectedTotal=32 pairs=32
rendered=32
每个 (根因, 目标) 的渲染行数 = 独立算法期望（同时抓重复与隐藏） :: {"mismatches":[],"pairs":32}
本地来源的行数 = 独立算法算出的期望（没有被多删） :: {"rendered":14,"expected":14}
```

逐键计数**期望与实测完全一致**：14 条 `ANSWER_KEY_MISSING_SLOT:q*` +
14 条 `ANSWER_MISSING:q*`（跨来源合并后只剩本地那一行）+ 4 条其他 = 32。

**complex-reading 夹具（负控）** `run-issue-list-2026-09-16T09-58-09-856Z` → **7/7 通过，exit=0**

```text
gate raw=8 warnings=0 expectedLocal=0 expectedTotal=5 pairs=4
rendered=5
expected == actual == [QUALITY_NOT_READY:QUALITY_NOT_READY,1]
                        [RUNTIME_COMPILER_FAILED:document,1]
                        [SIGNIFICANT_REGION_UNASSIGNED:document,1]
                        [SLOT_HOST_MISSING:group-2,2]
```

3 行 → **5 行**：两个 `document` 根因各自成行，group-2 的两条事实**都留下了**。
**这一跑同时证明了两件事**：没有重复显示，也没有隐藏不同问题。

---

## 四、任务 4：接入后端持久化撤销 —— 已完成（并如实记录后端契约缺口）

### 洞在哪里

`RecognitionPanel` 用一个**会话内**的 `Set<decisionId>` 记「我点过撤销」并据此显示「已撤销」。
刷新页面/重开/换会话，这个 Set 就没了 —— 界面又把「撤销」按钮放回来，
而用户以为已经撤销完了。

### 后端现状（不能靠状态码判，也不能凭空发明一个）

- `RecognitionResolutionV1` 只有 `agreed | auto_fixed | needs_review | unverifiable` —— **没有**「已撤销」；
- `build_view` 还会把 `auto_fixed` 的项**无条件**留在 `autoApplied` 里。

### 修法：改用可观测的持久化事实

撤销的语义本身就是可观测的：`undo` 补丁带着「改回哪个值」，
只要**权威稿**里那个答案位已经等于它，撤销就已生效（无论是刚点的、上次会话点的、
还是用户自己手改回去的）。新增纯函数：

```ts
isUndoAlreadyApplied(undo, answerKey)  // 按 {kind, values|labels} 形状比，未知形状返回 false
```

面板改成按 `answerKey` **现算**，删掉会话 `Set`。重开/刷新后判定不变。

### 证据

- 单测：`recognitionDecisions.test.ts` **25 → 32**（+7：文本/选项/kind 不同/位缺失/补丁不可解析…）。
- `tsc --noEmit` 干净；全量 **186 passed / 12 files**。
- 真实界面验证需要真实候选 → 见任务 6。

**仍需后端补的**：持久化的「已撤销」状态（或让 `build_view` 把已撤销项移出 `autoApplied`），
这样界面才不必依赖「值比对」这个等价判据。已记入 `findings.md` F-R10-3。

---

## 五、任务 5：查明构建失败原因 —— **原因未查明**（不编解释），已重建并重新验证

### 现象

一次 `npx vite build` 失败（只留一帧栈 `at async Object.defaultBuildApp`），
随后 `npx tauri build` 因为 `dist/` 没更新而在 **1.05s** 内「成功」返回，
于是 exe 相对源码变旧 —— 被新鲜度护栏正确拦下为 `cannot-run`(3)，**没有产生假绿**。

### 为什么没查明

- **未能复现**：同一目录、同一命令随后连续成功（`vite exit=0` / `tauri exit=0`）。
- 排除了「应用占着 dist」：`tasklist` 确认**没有**残留的 `ielts-author-studio.exe` 进程。
- **我自己的工具缺陷把原因截掉了**：当时把输出接进了 `| tail -2`，
  **管道把退出码换成了 `tail` 的 0**，于是 `&&` 继续执行、真正的原因行被截断。

### 已做的处置

- 构建改为：**完整日志落盘**（`artifacts/vite-build.log`、`artifacts/tauri-build.log`）
  + **显式检查退出码** + `set -o pipefail`，失败即停并打印全文；
- 重建结果：`vite exit=0`、`tauri exit=0`，产出
  `src-tauri/target/debug/ielts-author-studio.exe`（`sha256 a66905a7ad9f…`）；
- 用重建后的二进制**重新验证了最新去重代码**（见第三节，两个夹具全绿）。

**诚实边界**：原因未查明。可依赖的是护栏 —— 构建失败不会被当成通过，而是 `cannot-run`。

---

## 六、任务 6：真实候选按钮流程 + PDF/DOCX 发布到学生端计分 —— **仍被后端阻塞**

| 项 | 状态 |
| --- | --- |
| 真实候选按钮流程 | **未执行** —— `run-recog-buttons-2026-09-16T09-59-46-527Z` 五个场景全部 `not-executable`，`exit=5`（`candidates: batchId=null, actionable=0`） |
| PDF 完整发布消费链 | **未通过** —— 发布步骤 `blocked`，`manifestExists=false` |
| DOCX 完整发布消费链 | **未通过** —— 同上 |
| 学生端提交计分 | **无证据** —— 没有任何本次发布的产物 |

**卡在哪**：发布仍被 `WORD_LIMIT_UNPARSED` 的**误判**拦下（上一轮已归因，`edit_text` 与
`confirm_table` 两个补救动作都是死路）。候选侧等后端任务 D 的 8 个可复现场景。

**已就绪、等交付即可跑**：按钮流程 5 个场景的断言（含本轮重写的场景 4）、
逐键计数校验、完整链必需步骤判定。**不需要再写代码。**

---

## 七、本轮交付的改动

| 文件 | 改动 |
| --- | --- |
| `src/features/editor/actionableIssues.ts` | 新增 `sameFact()` 取代 `根因+目标` 唯一身份；导出 `issueRootCause()`；行上带 `rootCause` |
| `src/features/editor/ExamWorkspacePage.tsx` | 行输出 `data-issue-code` / `data-issue-source` / `data-issue-root-cause`；`RecognitionPanel` 传 `answerKey` |
| `src/features/editor/RecognitionPanel.tsx` | **删掉会话内 `undoneIds` Set**，改为按权威稿现算 `isUndoAlreadyApplied` |
| `src/features/editor/recognitionDecisions.ts` | 新增 `isUndoAlreadyApplied()` + `sameAnswerValue()` |
| `src/features/editor/publishGateIssues.test.ts` | 16 → 21（含「同来源同文案才算同一条」「两条不同事实都留」） |
| `src/features/editor/recognitionDecisions.test.ts` | 25 → 32 |
| `scripts/e2e/lib/chain-verdict.mjs` | 新增 `STEP_STATUS` / `notOutcome` 判定 + 通过数自检 |
| `scripts/e2e/lib/chain-verdict.test.mjs` | 26 → 33 |
| `scripts/e2e/tauri-cdp-issue-list.mjs` | 期望算法改为**逐「根因+目标」计数**（一条断言同时抓重复与隐藏） |
| `scripts/e2e/tauri-cdp-recognition-buttons.mjs` | 场景 4 重写：值 + 后端 `stale` + 重开三者都要验；`readDecision` 补 `stale`/`baseEditVersion` |
| `findings.md` / `progress.md` / `task_plan.md` | 记录 F-R9-12/13、F-R10-1…4 |

### 验证汇总

```text
tsc --noEmit                     → 干净
vitest run                       → 12 文件 / 186 测试全部通过（上一轮 167）
问题列表校验（PDF 夹具）          → 7/7 断言，verdict=passed exit=0
问题列表校验（complex-reading）   → 7/7 断言，verdict=passed exit=0（负控：5 行，两条事实都在）
完整链（默认）                    → verdict=blocked exit=4
完整链（负例）                    → verdict=passed-negative-case exit=0
候选按钮流程                      → verdict=not-executable exit=5（无候选）
构建                              → vite exit=0 / tauri exit=0
```

---

## 八、未完成清单（不得当作已交付）

1. **任务 6 全部内容**：真实候选按钮流程、PDF/DOCX 发布到学生端提交计分 ——
   被后端阻塞（无候选 + 发布被误判拦下）。**本轮不宣称任何完整链通过。**
2. **任务 5 的根因**：未查明。只做了处置与护栏，**没有解释**。
3. **后端待修（已交接，不在我的写入范围）**：
   - `issueId` 必须唯一（现在会撞键）→ 去重与 `resolution` 继承都受影响；
   - 门禁无视 `resolution`（`quality.rs:211` + `authoring_v2_commands.rs:430-438`）→ 阻塞「确认」动作；
   - `BLOCKER_LIST_TRUNCATED` 说「仅显示前 20 条」但返回完整数组；
   - 无持久化「已撤销」状态（现用值比对等价判据）。
4. **`cdp-default` 通过证据**：本沙箱不可得（F-R8-2）。
5. **真实云服务调用**：未完成（无凭据）。

> 本轮不把「本轮修复已完成」当作「整个产品目标已完成」。
