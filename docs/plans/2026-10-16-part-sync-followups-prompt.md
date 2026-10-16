# 任务：Part 归属同步的 3 个收尾问题

背景：上一轮已在工作区实现阅读 Part 同步（未提交），报告见 `docs/qa/2026-10-16-part-assignment-sync.md`。验收通过，留下 3 个问题。请在**现有未提交改动的基础上**修复，不要回退或重写已验收的部分。先读 `AGENTS.md`。

## 1.（必须修）懒对齐出错会拖垮整个题库列表或让条目打不开

`src-tauri/src/library/commands.rs`：

- `list_library_items_core` 遍历 `mismatched` 时调用 `align_reading_part(&mut conn, &id)?`
- `get_workspace_item_core` 和 `get_library_item_summary_core` 也直接 `align_reading_part(...)?`

`align_reading_part` 只吞掉 `CLOUD_REVIEW_IN_PROGRESS` 和 `EDIT_VERSION_CONFLICT` 两种错误。其他错误会一路传上去，比如某条存量稿件过不了 `validate_authoring`（迁移/种子写入的旧稿件不一定满足当前校验，上一轮测试夹具就因为缺 `passage.sourceAnchors` 而失败）、`part_projection` 的 SQL 错误、JSON 解析失败。一旦出错：

- 只要有一条坏数据，**整个题库列表加载失败**；
- 该条目在编辑器里**打不开**。

要求：

- 读路径（列表、打开工作区、单行摘要）上的懒对齐改为**尽力而为**：失败时记录日志（`eprintln!("[library] part alignment skipped for {id}: {error}")`，风格与同文件的 on-demand migration 一致），然后继续返回数据。该行保持原状，下次读取再重试。
- 用户主动操作的路径（`set_library_item_part_core`、编辑器 `setExamCategory` 保存）仍要把错误如实返回给前端，不能吞掉。
- 加 Rust 测试：构造一条手动标签与稿件不一致、且稿件过不了 `validate_authoring` 的阅读条目，加上一条正常条目。断言 `list_library_items_core` 成功并返回两条，`get_workspace_item_core` 能打开坏条目，坏条目的稿件和版本都没变。

## 2.（建议修）旧条目的自动判定标签被稿件里默认的 P1 覆盖

`align_reading_part` 处理非手动行时，会把稿件 `exam.category` 写回题库标签（`UPDATE ... SET part_label=category ...`）。`cas_write_canonical` 中的投影也一样。旧条目导入时稿件被无条件写成 `P1`，但题库标签之前是从标题行/题号范围/文件名判定出来的（如 P2），现在会被改回 P1，这是**退化**：题库原本正确的显示变成错的。

要求（推荐方案，如有更好的做法请说明理由）：

- 对于 `part_source` 是自动来源（`title`/`range`/`filename` 等，不是 `manual`、`none`、`canonical`）、判定结果是 P1–P3 且与稿件不一致、**而且稿件的 category 从未被人工设置过**（journal 里没有 `setExamCategory` 的人工记录）的阅读条目：把判定结果写进稿件。走带版本的写入路径，origin 用**非 Human** 的系统来源（参考 `EditOrigin` 现有变体，必要时新增 `PartBackfill` 一类），避免把 `exam` 登记成人工保护范围。
- 其余情况保持现在的「以稿件为准投影到题库」。
- 必须幂等：重复列出不再递增版本。同样遵守第 1 条，读路径失败不能拖垮列表。
- 加 Rust 测试覆盖：自动 P2 vs 稿件 P1 → 稿件改成 P2，版本只增一次；人工设置过的不会被自动判定覆盖。

## 3.（小问题）编辑器标题旁 Part 重复显示

`src/features/editor/ExamWorkspacePage.tsx:578` 的 `part-badge`（纯文本 "P2"）紧挨着 `:589` 的 P1/P2/P3 下拉，界面上显示成「P2 [P2 ▾]」。

要求：可编辑时只显示下拉；云端校核锁定或其他不可编辑状态下，显示只读徽章（或禁用的下拉），二者不要同时出现。保留可访问名称（例如 `aria-label="Part 归属"`）。同步更新 `ExamWorkspacePage.cloudReviewLock.test.tsx` 等相关测试，以及 `scripts/e2e/tauri-cdp-part-assignment-sync.mjs` 里依赖 `workspace-part-badge` 的断言（改断言要基于新的界面需求，不是为了让测试通过）。

## 不要做

- 不提交 git，不改发布门禁和 golden baseline，不直接改写真实用户库 `%APPDATA%\com.ielts.author.studio\authoring_hub.db`。

## 交付

1. 改动文件清单。
2. 运行并贴出结果：`npm run check`、`npm test`、`cargo test --manifest-path src-tauri/Cargo.toml --lib -- library:: authoring_v2 export cloud_direct:: processing::scheduler::tests`。
3. 重跑 `node scripts/e2e/tauri-cdp-part-assignment-sync.mjs`，贴出报告路径和断言结果，并附编辑器标题区截图（可编辑状态，最好再有一张锁定状态）。
4. 按 AGENTS.md 区分：端到端产品验证 / 命令层验证 / 仅 schema 证据，写明仍存在的缺口。
