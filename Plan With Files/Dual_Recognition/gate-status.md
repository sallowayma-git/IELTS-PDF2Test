# 门禁状态（Gate Status）

日期：2026-09-05 · 基线 commit `bb978be`

## 结论先说

**本轮新增的门禁全绿；仓库里 7 个历史 phase 门禁在本轮开始之前就是红的，与本轮改动无关（下方给出证明）。**
唯一被本轮改动打破的历史门禁是 `verify:phase5:editor`，已按新产品要求更新契约（不是重写基线）。

## 本轮新增门禁

| 门禁 | 命令 | 状态 | 说明 |
|---|---|---|---|
| TypeScript | `npm run check` | 通过 | `tsc --noEmit` |
| 生产构建 | `npx vite build` | 通过 | 142 modules |
| 产品面基线 | `npm run verify:product-baseline` | 通过 | routes 4 / pages 10 / commands 59 / tables 8，无漂移 |
| 横向溢出矩阵 | `npm run test:layout:legacy` | 通过 | 84 项检查 0 溢出 |
| 三表面主链冒烟 | `npm run e2e:library-workspace` | 通过 | 13 步 |
| Rust fmt | `cargo fmt --check` | 通过 | |
| Rust check | `cargo check` | 通过 | exit 0，83 个既有 dead_code 警告 |
| Rust tests | `cargo test` | 通过（见下） | |

CI 装配：`.github/workflows/product-convergence-gates.yml`（surface-gates + backend-gates 两个 job）。

## 历史 phase 门禁：本轮开始前即为红

证明方式：`git diff HEAD --name-only` 显示本轮**没有修改**这些门禁读取的任何输入文件
（`src/config/featureFlags.ts`、`contracts/`、`fixtures/golden/`、`src-tauri/src/schema/`）。

| 门禁 | 失败原因 | 是否本轮引入 | 处置 |
|---|---|---|---|
| `verify:phase0:ci` | 追踪语料校验 `readyForPhase1: false` | 否 | 不动。属于历史 Phase 0 语料状态 |
| `verify:phase1:schema:local` | `hash_mismatch:ListeningAttemptV1` 等 schema 哈希漂移 | 否 | 不动。Listening 契约与本轮无关 |
| `verify:phase2:shadow` | 断言 `documentIrV2Shadow must remain disabled by default`，实际默认 `true` | 否 | 不动。门禁编码的是历史 Phase 2 要求；当前默认开启是计划 §1.1 确认的既有状态 |
| `verify:phase3:docx` | 断言 `documentIrV2 must remain disabled by default` | 否 | 同上 |
| `verify:phase4:grammar` | 断言 `documentIrV2 must remain disabled by default` | 否 | 同上 |
| `verify:phase6:runtime` | `ReadingExamSourceV2 contract hash is stale` | 否 | 不动。契约哈希漂移，需要单独决策是否重录 |
| `verify:phase7:listening-contract` | 缺少同级仓库 `../NAS/server/src/lib/...` | 否 | 环境限制，本机没有 NAS 仓库 |

按 `AGENTS.md`：「Golden-baseline drift is diagnostic evidence, not an automatic instruction to restore old
behavior or rewrite baselines.」这 7 个门禁不应该被顺手改绿 —— 它们要么编码了已被产品决策取代的旧要求
（documentIrV2 默认关闭），要么反映真实的契约漂移，都需要独立决策。本轮只如实记录，不重写。

## 被本轮打破并已按新要求更新的门禁

`verify:phase5:editor`

两处失败与对应处置：

1. `src/components/ExamCanvasV2.tsx` 不存在
   —— ExamCanvas 已按计划 §16.7 迁到 `src/exam-canvas/ExamCanvas.tsx`，门禁路径同步更新。
2. `Phase 5 shared ExamCanvas contract is missing contentEditable`
   —— 计划 §9.3 明确要求**移除**裸 `contentEditable` 与 `document.execCommand`。
   门禁原来断言它必须存在，这是被产品决策取代的旧要求。

更新后的契约（不是放宽，是换成更严的新要求）：

```js
// 正向：新的原位编辑契约必须存在
"InlineTextEditor", "onTextCommand", "expectedText"
// 负向：旧实现不得被无意恢复
for (const banned of ["contentEditable", "document.execCommand"]) { ... throw }
// 新增：IME / 纯文本粘贴 / 无障碍标签
"onCompositionStart", "onCompositionEnd", 'clipboardData.getData("text/plain")', "aria-label"
// 新增：EditorCommandV1 的 code point 计数正确性
"set_text", "expectedText", "codePointLength", "EditorCommandConflictError", "Array.from(text).length"
```

`verify:phase5:editor` 现在通过。
