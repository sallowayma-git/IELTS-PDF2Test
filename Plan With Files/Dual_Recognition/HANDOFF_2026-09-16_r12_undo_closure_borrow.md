# HANDOFF — 2026-09-16 R12：`reconcile/commands.rs` 在途改动不编译，阻断全部真实 E2E

## 结论（一句话）

`src-tauri/src/reconcile/commands.rs` 当前工作树版本有 **2 个 `E0596`**，`cargo build` 失败，
因此**无法产出任何新 exe**；无云导入四条链、候选按钮、PDF/DOCX→发布→学生端计分等
全部真实 E2E 都卡在这一步。文件属后端独占区（`RECOGNITION_LOOP_CONTRACT.md` §1.1），
前端侧未做任何修改，仅出本交接单。

## 复现

```
cd F:/workspace/PDF2Test
npx tauri build --debug --no-bundle      # 或 cargo check --manifest-path src-tauri/Cargo.toml
```

## 错误原文（`--message-format=short` 等价）

```
error[E0596]: cannot borrow `items` as mutable, as it is a captured variable in a `Fn` closure
   --> src\reconcile\commands.rs:801:41
    |
584 |     let mut items = store::load_decision_items(&conn, &batch.batch_id)?;
    |         --------- `items` declared here, outside the closure
...
799 |                 &|tx, _version| {
    |                  -------------- in this closure
800 |                     for index in &runnable {
801 |                         let item = &mut items[*index];
    |                                         ^^^^^ cannot borrow as mutable

error[E0596]: cannot borrow `items` as mutable, as it is a captured variable in a `Fn` closure
   --> src\reconcile\commands.rs:901:37
    |
584 |     let mut items = store::load_decision_items(&conn, &batch.batch_id)?;
...
899 |             &|tx, _version| {
900 |                 for index in &undo_indices {
901 |                     let item = &mut items[*index];
    |                                     ^^^^^ cannot borrow as mutable
```

## 最小修法（两个方向，任选其一）

两处都是「闭包按 `Fn` 捕获 `items`，但闭包里要 `&mut items[i]`」。`Fn` 只给 `&self`，
所以拿不到可变借用。

1. **把闭包签名放宽到 `FnMut`**（改动最小）。前提是调用方接受 `FnMut`；若调用方
   （`apply_editor_commands` 的事务执行器）按 `Fn` 约束，则需要同步放宽它的约束，
   或改用下面第 2 种。
2. **不捕获 `items`，改为在闭包内构造待写数据、在事务外用返回值合并**。把
   「哪个 index 改成什么状态」先算成 `Vec<(usize, DecisionStatusV1, ...)>`，闭包只捕获
   这份只读计划，事务成功后再回到外层 `&mut items` 应用。这条路径不依赖调用方的约束，
   也顺带把「状态写入与回滚同一事务」的意图表达得更清楚。

注意两处（第 799–801 的 accept 分支、第 899–901 的 undo 分支）是**同一类**问题，
修完一处另一处会继续报错；建议一次改完。

## 影响面（为什么必须先修）

- 这两个分支恰好是 **accept 写入**与 **undo 状态写入**。R12 任务书的第 1、2、5 项
  （撤销对齐、撤销后权威稿/revision/决策状态验证、候选接受/拒绝/撤销真实按钮验收）
  都直接依赖它们，属于当前最高优先级路径。
- 在编译通过之前，任何「无云导入有 batch」「候选按钮通过」的报告都**不可能**成立——
  只能拿到一个更旧的 exe，而该 exe 与当前源码不一致。

## 时间线（供归属判断）

| 时刻（本地 +01:00） | 事件 |
| --- | --- |
| 14:02:52 | 前端侧发起 `tauri build --debug --no-bundle`（含 `npm run build`） |
| 14:04:20 | `reconcile/commands.rs` 被写入（构建进行中） |
| 14:04:34 | 该次构建**成功**产出 exe（`sha256 db268c43b4fee62fa17d520500d34881fc6012027266471558c83907d3160745`） |
| 14:04:38 | `processing/scheduler.rs` 被再次写入 |
| 14:04:47 | `processing/scheduler.rs` 再次写入 |
| 14:05:24 | 第二次构建的 `npm run build` 产出新 dist，随后 `cargo build` **失败**（即上面 2 个 E0596） |

因此现存 exe 只对应「14:04:34 之前被 rustc 读到的源码」，**不能**代表当前工作树。

## 前端侧已做的、与本缺陷无关的改动

- `scripts/e2e/lib/tauri-cdp-harness.mjs`：`assertBuildFresh` 改为三段链
  （前端输入 → dist → exe，后端输入 → exe），并新增 `buildFreshReport`。
- `scripts/e2e/lib/tauri-harness.mjs`：`buildFreshness` 同步纳入 dist 段。
- `scripts/e2e/lib/build-freshness.test.mjs`：新增 7 条回归（含「旧 dist 被新 exe 包入」）。
- `scripts/e2e/tauri-cdp-local-chain.mjs`：新增无云导入四条链验收脚本。

这些都在 `scripts/**`（前端独占），与后端文件不冲突。
