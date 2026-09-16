# 前端 agent2 第九轮报告 —— 先解除真实发布阻塞

日期：2026-09-15
范围：`src/**`、`scripts/e2e/**`、`src/api/*Client.ts`（未改后端门禁任何一行）
二进制：`src-tauri/target/debug/ielts-author-studio.exe`
sha256 `4a7eebd511bad742efe75648bd388e43483b96f1f3f975cad2e9c30a1ef7a52a`，mtime `23:10:15`
构建新鲜度：`buildFresh.ok=true`，`srcNewest=23:06:13`，**`toleratedConcurrentEdits=[]`**（无需容忍）

---

## 0. 本轮最重要的一句话

任务书把优先级改成「**先解除真实发布阻塞**」。为此我先做了归因，结论是：

> **发布阻塞不是「数据没改对」，而是门禁把一份好题判成了坏题。**

走真实界面把 `demanding-reading-passage-3.pdf` 的 14 个答案填上之后，
`hardFailures` 从 `["WORD_LIMIT_UNPARSED","ANSWER_KEY_MISSING_SLOT","RUNTIME_COMPILER_FAILED"]`
**降到 `["WORD_LIMIT_UNPARSED"]`** —— 其余阻塞**确实**被真实数据修复清掉了。
剩下的那一条是误判：它出现在一个按字母选的 summary completion
（"Complete the summary using the list of words and phrases, A-H"）上，该题型本就没有 word limit。
而它给出的两个补救动作 `edit_text` / `confirm_table` **都是死路**。

**因此第三节与第四节必须记为「未完成 / 未通过」** —— 发布没有发生。

详细证据见 `HANDOFF_2026-09-15_publish_blocker_attribution.md`。

---

## 一、编辑/预览专项

| 项 | 值 |
| --- | --- |
| 判定 | **`passed-specialty`**，`exitCode=0` |
| 范围 | 11 步，**明确不含发布**（`publish-via-workspace-button` 不注册） |
| 证据 | `artifacts/e2e-cdp/run-chain-2026-09-15T22-40-40-645Z/report.json` |
| 运行档案 | `runProfile=cdp-diagnostic` |

**它不叫「完整链通过」**，判定词是独立的 `passed-specialty`，退出码与完整链通过同为 0 但名称不同，
避免被误读成发布链成功。本轮 PDF 与 DOCX 两条完整链里这 11 步同样全部 `passed`（见三、四）。

覆盖：题库加载 → 导入 → 后台管道到稳定态 → 打开工作区 → 编辑正文并保存 →
重开仍保留 → 学生预览渲染 → 预览内作答相互隔离 → 预览后再编辑仍保留 →
识别面板读到真实后端 → 预览与门禁结论一致。

---

## 二、候选按钮交互

| 项 | 值 |
| --- | --- |
| 判定 | **`not-executable`**，`exitCode=5`（**不是 passed，也不是 failed**） |
| 证据 | `artifacts/e2e-cdp/run-recog-buttons-2026-09-15T22-46-09-621Z/report.json` |
| 脚本 | `scripts/e2e/tauri-cdp-recognition-buttons.mjs`（真实 DOM 点击，不是 IPC 探针） |

**场景前提不成立**，实测：

```text
candidates: batchId=null, actionableCount=0, needsReviewCount=0, autoFixedCount=0, chains.*=not_run
accept-suggestion / reject-suggestion / undo-auto-fix / stale-suggestion-protected / idempotent-retry
  → 全部 NOT-EXECUTABLE
verdict=not-executable exit=5
```

**为什么无候选**：后端本轮交接（`HANDOFF_2026-09-15_recognition_model_chain.md` §5）明确列出
**任务 D「8 个可直接用于 UI 的真实场景」= 未做**，未产出可复现产物与预期字段。
本轮我确认了这一点，没有用手写 DB 候选或伪造批次来把场景「做成」。

**已就绪的部分**（等真实候选一到即可执行，无需再写代码）：

| 场景 | 断言 |
| --- | --- |
| `accept-suggestion` | 内容确实改变、版本递增、重开后仍保留 |
| `reject-suggestion` | 内容**不变**、决策持久化（重开 `status != open`） |
| `undo-auto-fix` | 恢复正确原值、重开不再显示未撤销 |
| `stale-suggestion-protected` | 用户先改目标再接受旧建议 → 用户修改受保护 |
| `idempotent-retry` | 同一事件循环点两次 → 版本只递增 **1** 次 |
| （无候选） | 记 `not-executable`，**绝不跳过判 passed** |

`undo-auto-fix` 内含**空断言防护**：若撤销前后的值本来就相同，直接抛错拒绝执行
（这条防护是被上一轮的一次真实空断言事故逼出来的）。

**诚实边界**：现有 `setAnswer` IPC 探针只证明编辑通道可用，**不替代**这些按钮流程。
它是在**上一轮**（R7）运行的（`run-recog-write-2026-09-15T22-06-53-635Z`，8 步全通过），
**本轮没有重跑**，因此不作为本轮证据引用。

---

## 三、PDF 完整发布消费链

| 项 | 值 |
| --- | --- |
| 判定 | **`blocked`，`exitCode=4`** —— **未完成，未通过** |
| 证据 | `artifacts/e2e-cdp/run-chain-2026-09-15T23-10-30-734Z/report.json` |
| 夹具 | `fixtures/parser/demanding-reading-passage-3.pdf`（`importEntry=pick-folder`） |
| 步骤 | 11 `passed` / 1 `blocked` / 0 `failed` / 0 `missing` |

```text
verdict      = blocked   exitCode = 4
verdictReason= 必需步骤被质量门禁阻断：publish-via-workspace-button。发布未发生，不得计为通过。
publication  = { manifestExists:false, scriptFiles:[], resourceManifestExists:false, preflightPassed:false }
publicationFailures:
  - manifest.js 未落盘（manifestExists !== true）
  - 发布预检未通过（preflight.passed=false）
  - 发布目录里没有题目 JS（v2-p*.js）
  - 发布目录里没有资源清单（resources/<examId>/asset-manifest.json）
```

**没有发布、没有 manifest、没有题目 JS、没有资源清单、没有进入学生端。**
按本轮退出标准，这一节**不得**报告为完整链通过。

---

## 四、DOCX 完整发布消费链

| 项 | 值 |
| --- | --- |
| 判定 | **`blocked`，`exitCode=4`** —— **未完成，未通过** |
| 证据 | `artifacts/e2e-cdp/run-chain-2026-09-15T23-11-34-492Z/report.json` |
| 夹具 | `fixtures/parser/demanding-reading-passage-1.docx`（`importEntry=pick-files`） |
| 步骤 | 11 `passed` / 1 `blocked` / 0 `failed` / 0 `missing` |

结果与第三节逐字段一致：`manifestExists=false`、`scriptFiles=[]`、
`resourceManifestExists=false`、`preflightPassed=false`，四条 `publicationFailures` 全中。

DOCX 走的是 `source-files` hook（与 PDF 的 `pick-folder` 不同），
说明非 PDF 导入路径本身是通的；卡住的是**同一处**质量门禁。

**未完成**：没有发布产物，因此**没有**完成「manifest/题目 JS/资源落盘 → Electron 加载 →
作答 → 提交 → 回执与计分」，也**没有**独立人工答案表支撑的得分预期核对。
本轮**不**声称任何 PDF 或 DOCX 的完整消费链通过。

---

## 五、诊断参数运行与默认运行

| 档案 | 参数 | 本轮证据 |
| --- | --- | --- |
| `cdp-diagnostic` | `--no-sandbox --disable-gpu` | 本轮**全部**运行都是这个档案 |
| `cdp-default` | 无测试专用安全参数 | **本沙箱不可运行**（见下） |

受控 A/B（同一夹具、同一二进制，唯一变量是这两个参数）：

| 运行 | 安全参数 | 结果 |
| --- | --- | --- |
| A | `[]` | 12 步里 **11 步失败**：`CDP 连接已关闭`；应用输出停在 `[library] v2 migration: scanned=0 …`；`appProcessExitCode=null` |
| B | `--no-sandbox --disable-gpu` | 11 passed / 1 blocked |

**结论**：本环境**不存在** `cdp-default` 的通过证据。
因此本报告全部结论都只能声明为**诊断档案下**的结论，**不得**当作默认路径通过。
默认档案的运行入口已备好（`npm run e2e:chain:default-profile`），一旦环境允许即可取证据。
报告里 `runProfile` 与 `securityArgs` 两个字段逐条落盘，可随时区分。

---

## 六、受控云服务与真实云服务

| 项 | 状态 |
| --- | --- |
| 真实云服务 | **未完成** —— 无凭据，`cloudEnabled=false`；本轮未写入任何密钥 |
| 受控云服务完成候选交互链 | **未完成** —— 候选持久化与真实场景未交付（后端任务 D 未做），没有可供受控云服务驱动的候选 |

按任务书要求，两类结果**分别报告**：本节两项都是未完成，因此
**本轮不宣称任何云链路通过**，也不把本地链路的结论外推到云端。

---

## 七、本轮交付的改动与验证

| 文件 | 改动 |
| --- | --- |
| `scripts/e2e/tauri-cdp-publish-unblock-probe.mjs` | **新增**归因探针（三阶段：改数据 / 标 ignored / 按建议改文字），`isAcceptanceEvidence: false` |
| `scripts/e2e/tauri-cdp-issue-list.mjs` | **新增**问题列表真实界面校验（8 条断言，含「同一根因只占一行」的 DOM 不变量） |
| `Plan With Files/Dual_Recognition/HANDOFF_2026-09-15_publish_blocker_attribution.md` | **新增**给后端的交接（含三条需要他们决定/修的事项） |
| `src/features/editor/actionableIssues.ts` | `rootCauseOf()`；去掉泛化 `QUALITY_HARD_FAILURE`；`ROOT_CAUSE_ALIASES` 去掉跨来源同根因重复 |
| `src/features/editor/ExamWorkspacePage.tsx` | 定位失败时如实说明；行加 `data-issue-code`/`data-issue-source`；`locateTarget` 兼按 `hostNodeId` 找 |
| `src/features/editor/publishGateIssues.test.ts` | 11 → 16 单测 |
| `package.json` | 新增 `e2e:issue-list`（**注意**：改 `package.json` 会让已构建二进制在新鲜度规则下变 stale，下次运行前需重建） |
| `findings.md` / `progress.md` / `task_plan.md` | 记录 F-R9-1…F-R9-10 与本轮状态 |

验证：

```text
tsc --noEmit                     → 干净
vitest run                       → 12 文件 / 167 测试全部通过（上一轮 162）
契约漂移检查                     → 0 处破坏性不一致，1 处需要留意
真实产物回放（临时测试，已删）    → blockers 34 → 31（两次独立运行一致）
问题列表真实界面校验             → 8/8 断言通过，verdict=passed exit=0
```

---

## 八、补做：问题列表的真实界面证据（R9-8）

第七节的两处改动当时**只有单测 + 真实产物回放，没有在真实应用里看过一眼**。
任务书要求「用户能完成修复，而不只是看到错误」，因此补了
`scripts/e2e/tauri-cdp-issue-list.mjs`（WebView2 CDP 真实通道，非 IPC 探针）。

| 项 | 值 |
| --- | --- |
| 判定 | **`passed`**，`exitCode=0`，8/8 断言 |
| 证据 | `artifacts/e2e-cdp/run-issue-list-2026-09-15T23-31-41-068Z/report.json` |
| 运行档案 | `runProfile=cdp-diagnostic`（`--no-sandbox --disable-gpu`，见第五节 F-R8-2） |

```text
gate raw=34 warnings=1 expectedGate=18 expectedLocal=14
rendered=32                     # 修之前是 46
同一根因（归一后）在界面上只占一行 :: {"duplicated":[],"total":32}
门禁来源的行数 = 独立算法算出的期望 :: {"rendered":18,"expected":18}
本地来源的行数 = 独立算法算出的期望（没有被多删）:: {"rendered":14,"expected":14}
```

### 9.1 校验过程中查出的真实产品缺陷（已修）

第一次跑出 `rendered=46`，比期望多 14 行。**没有直接记成产品缺陷**，回原始产物逐行核对后
确认那 14 行是**真实重复**：同一道题渲染两行，因为两个子系统给同一件事起了两个码 ——
本地闭包记 `ANSWER_UNRESOLVED`（warning，「第 27 题还没有答案。」），
门禁记 `ANSWER_MISSING`（blocker，「这道题还有答案没有填写。」）。
去重键是 `code:targetId`，两个码撞不上。

**别名是可证明的，不是猜的**：后端 `authoring_v2_commands.rs:439-446` 用
`answerKey[slot].kind == "unresolved"` 筛出这些 slot，本地闭包用的是**同一个谓词**。
同谓词、同目标 → 同事实、同用户动作。于是登记一条别名
`ANSWER_UNRESOLVED → ANSWER_MISSING`（**只登记这一族**），合并时保留本地更具体的文案、
把级别提到 blocker（发布确实被它拦下）。46 → 32 行。

同时加了「本地那 14 行一条没少」的**反向断言**，堵住「靠多删满足去重」的假绿。

### 9.2 本轮自查出的第 4 个假结论（我的工具，已修）

期望算法**只对门禁那半建模**，而界面的行是 `mergePublishGateIssues(本地闭包, 门禁)`。
`46 vs 32` 因此被稳定地呈现成「产品多渲染」，实际是**我算漏了一半**。
修法：期望拆成门禁/本地两半（本地那半的键要**占位**，否则被吸收的门禁行会重复计入）；
断言按 `data-issue-source` 分开比；为此给行加了 `data-issue-code` / `data-issue-source`
（**光看 code 分不出来源**：本地与门禁都可能写 `ANSWER_MISSING`）。

### 9.3 更正我自己上一轮的 F-R9-4

上一轮写「`complex-reading.pdf` group-2 有两条 `SLOT_HOST_MISSING`（同码同目标、**两条不同事实**）」，
并据此说「按 target 合并是错的」。回原始产物逐字段核对后**不成立**：那两条**逐字节相同**
（`internal` 也相同），是**同一个问题被后端推了两遍**。前端收成一行是对的；
真正的缺陷是后端重复推送（它抬高 `len()`，更早触发截断警告）。
**教训：计数相同不等于内容不同，写「两条不同事实」前必须看条目本身。**

### 9.4 新发现的后端矛盾（未修，已交接）

`authoring_v2_commands.rs:480` 在 `blockers.len() >= 20` 时发
`BLOCKER_LIST_TRUNCATED`「问题较多，仅显示前 20 条。」，但**返回的是完整数组、界面也全部渲染**。
本夹具三类各 3/14/16 都没到 20（什么都没截），警告照样发；界面于是出现
「仅显示前 20 条」旁边列着 46 行的自相矛盾。截断本身是**按类** `take(20)` 的，
文案却按整表说、也不说是哪一类被砍 —— 真被砍时用户不知道少看的是什么。
该文件是后端领域（共享文件我只 append），**未修**。

---

## 九、未完成清单（不得当作已交付）

1. **任务二的核心部分**：可确认的疑点需要「确认」动作，但门禁无视 `resolution`，
   现在加按钮就是**死按钮**（违反「不提供能忽略…的通用按钮」）→ **依赖后端修门禁**。
   已做的是不需要后端配合的那半：去掉泛化重复、去掉跨来源同根因重复、定位失败时如实反馈
   —— 且这三处现在都有**真实界面证据**（第八节，8/8 断言通过）。
2. **任务三的 5 个场景本身**：无真实候选项，无法执行（框架已就绪，判定 `not-executable`）。
3. **任务四**：发布未发生（`manifestExists=false`）→ 没有任何完整链通过可报告，
   也没有任何学生端提交计分证据。
4. **探针阶段 C 的运行时补证**：**已完成**。说明文字改后确实保存（版本 16→17），
   但 `instructionSignature.normalizedText` 逐字节不变、`wordLimit` 仍为 `null`、
   `WORD_LIMIT_UNPARSED` 仍 blocking —— `edit_text` 是死路已从「代码推论」升为「实测结论」
   （`run-publish-attribution-2026-09-15T23-14-01-281Z`）。
   同一次调查还抓到我自己一个假阳性：探针点偏了（长行内 span 的包围盒中心落在空白处），
   一度误以为说明文字不可编辑；改为点首行左边缘后编辑器正常打开，**该点不构成产品缺陷**。
5. **`cdp-default` 通过证据**：本沙箱不可得（第八节的运行档案是 `cdp-diagnostic`）。
6. **真实云服务 / 受控云服务链路**：均未完成。
7. **上一轮 F-R9-4 的一个错误结论已更正**：我曾写 `complex-reading.pdf` group-2 上有
   「同码同目标的两条不同事实」，并据此反对按 target 合并。回原始产物核对后，那两条
   **逐字节相同**，是后端重复推送同一个问题 —— 不是两条事实。相关结论见第八节 9.3。
8. **后端待修（已交接，不在我的写入范围）**：
   - 门禁无视 `resolution`（`quality.rs:211` + `authoring_v2_commands.rs:430-438`）→ 阻塞任务二核心；
   - 同一份预检里同一个 issueId 出现两次（group-2 的 `SLOT_HOST_MISSING`）；
   - `BLOCKER_LIST_TRUNCATED` 说「仅显示前 20 条」但返回完整数组（第八节 9.4）。

> 本轮不把「本轮修复已完成」当作「整个产品目标已完成」。
