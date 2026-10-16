# 任务：题库 Part 归属与稿件/导出不一致（阅读 P1/P2/P3）

先读 `F:\workspace\PDF2Test\AGENTS.md`：以真实 Tauri 产品路径为准，CLI/schema 检查不能替代产品验证。

## 用户看到的现象

题库界面可以给题目选 Part 归属（P1/P2/P3）。改成 P2 或 P3 后，编辑界面、学生预览和导出包里仍然按识别时的 Part 1 显示，数据结构没有跟着变。

## 已确认的根因（请先复核）

同一份题目的 Part 存在三处，彼此不同步：

1. **题库表** `library_items_v2.part_label / part_source`：题库界面「选择 Part」只写这里。
   - 前端 `src/features/library/LibraryPage.tsx:250` → `libraryStore.setPart` → `setLibraryItemPart`（`src/api/tauriCommands.ts:401`）
   - Rust `set_library_item_part`（`src-tauri/src/lib.rs:1797`）→ `library::commands::set_library_item_part_core`（`src-tauri/src/library/commands.rs:200`）→ `repository::set_item_part`，**只 UPDATE 了这一列**
2. **权威稿** `canonical_ds_json` 里的 `exam.category`（`"P1"|"P2"|"P3"`，类型见 `src/types/ielts-authoring-v2.ts:27` 和 `src-tauri/src/schema/ielts_authoring_v2.rs:23`）。导入时由 `job.category` 生成，缺省硬编码为 `"P1"`（如 `src-tauri/src/cloud_direct.rs:784`、`authoring_pipeline.rs:4792/8967`），并不使用 `library/part_detection.rs` 的自动判定结果。
3. **examId** 带 Part 前缀（如 `p1-86fcf23e`）。

学生预览（`src/services/readingRuntimeV2.ts:672`）和 NAS 导出（`src-tauri/src/export_artifacts.rs:90`，缺省 `"P1"`）读的都是第 2 处，所以题库里怎么改都不影响学生端。

真实数据（`%APPDATA%\com.ielts.author.studio\authoring_hub.db`，只读查询）：条目 `import-20261014104038-…`「24. P1 The Pearls」，题库 `part_label=P3, part_source=manual`，稿件 `exam.category=P1`，`examId=p1-86fcf23e`。

## 要求

### 1. 阅读题以 `exam.category` 为准，题库设置必须写进稿件
- 阅读条目在题库里设置 P1/P2/P3 时，在**同一次操作**里同时更新权威稿的 `exam.category` 和 `library_items_v2.part_label`（`part_source=manual`）。必须走现有的权威稿写入路径：版本号递增、并发冲突保护、打开中的编辑器能感知远端变更（参考 `remoteVersion.ts` / `conflictRecovery.ts` 的现有机制）。不要绕开版本直接改 JSON。
- 清除手动值（label 为 null）时，回到自动判定，稿件的 `exam.category` 也要跟着改成判定结果。判定不出时的行为要定下来并写清楚（建议保留原值，不要清空）。
- `exam.category` 只接受 P1/P2/P3。阅读条目如果在题库里被设成别的值（如 "Part 1"），要规范化或拒绝。听力（Part 1–4）和写作（Task 1/2）这次不改数据结构，保持只写题库列，但要在代码注释和报告里说明。

### 2. 编辑界面能看到并修改 Part，且与题库双向一致
- 编辑界面左上角（`ExamWorkspacePage.tsx` 的 `workspace-header-left`，标题旁）显示当前稿件的 Part 徽章，数据来自 `editor.draft.exam.category`。
- 提供在编辑界面修改 Part 的入口（下拉即可）。走现有的 patch 路径（`authoringV2Patches.ts` + Rust 对应 patch），支持撤销/重做、自动保存。保存后题库列表的 Part 也要同步更新（Rust 侧写权威稿时同步更新 `part_label`，或者题库读取时以稿件为准，二选一并说明理由）。
- 云端校核锁定期间（`cloudReviewing`）不允许修改，和其他编辑入口保持一致。

### 3. 导入时用自动判定结果初始化
- 新导入的阅读题，权威稿的 `exam.category` 要用自动判定（`library/part_detection.rs`：文件名、标题行、题号范围）的结果初始化，判定不出时才缺省为 P1。不要再无条件写 "P1"。云端直出（cloud_direct）和本地管线两条路径都要覆盖。

### 4. 存量不一致数据
- 对于已有的 `part_source=manual` 且与稿件 `exam.category` 不一致的阅读条目，做一次性对齐：以手动值为准写入稿件（同样走带版本的写入路径）。时机自选（启动迁移，或打开/列出时懒对齐），要幂等。上面那条「The Pearls」就是验收用例。

### 5. examId 前缀
- `examId` 是 NAS 包的身份标识，**不要**因为改 Part 而改 examId，否则重新发布会在 NAS 上产生重复题目。
- 但必须核实没有任何消费方从 examId 前缀推断 Part：PDF2Test 的导出/编译，以及 `F:\workspace\IELTS-NASfor-WenDao` 学生端的题库目录、筛选、展示（注意 `assets/generated/reading-exams/p1-low-30.js` 这类文件是按 id 命名的）。如果发现有地方靠前缀判断 Part，改成读 `meta.category`，并在报告里列出来。
- 标题里自带的 "P1"（来自文件名，如「24. P1 The Pearls」）不要自动改写，只在报告里说明。

## 不要做
- 不改识别管线的其他逻辑，不改发布门禁，不重写 golden baseline（漂移先判断是否是本次预期改动）。
- 不为了让测试通过而改测试期望。
- 不提交 git，改动留在工作区由我验收。直接修改真实用户库 `authoring_hub.db` 之前先备份；测试用临时库。

## 交付时请给出
1. 改动文件清单（如果改了 NAS 仓库，分开列）。
2. 测试：
   - Rust：`set_library_item_part_core` 对阅读条目同时更新稿件 `exam.category` 并递增版本；清除手动值回到自动判定；非 P1–P3 值的处理；存量对齐幂等；导入时用判定结果初始化 category。
   - 前端 vitest：编辑器 Part 徽章与下拉、patch 往返、撤销/重做、云端校核锁定时禁用。
3. 已运行命令及结果：`npm run check`、`npm test`、`cargo test --manifest-path src-tauri/Cargo.toml`（至少 library / authoring_v2 / export 相关模块）。
4. 按 AGENTS.md 区分：端到端产品验证 / 命令层验证 / 仅 schema 证据。至少在 Tauri 应用里实际操作：题库把一份阅读题改成 P2 → 打开编辑界面，左上角显示 P2 → 学生预览 → 导出 NAS 包，包内 `meta.category` 为 P2；再在编辑界面改回 P3 → 返回题库显示 P3。可参考 `scripts/e2e/tauri-cdp-*.mjs` 的 CDP 驱动方式，附截图或报告。做不到的部分明确写出缺口。
