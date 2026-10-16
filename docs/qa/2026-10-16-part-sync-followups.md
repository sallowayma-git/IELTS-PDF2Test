# Part 同步收尾验收（2026-10-16）

在上一轮已验收、未提交的改动上补齐三项收尾问题。未提交 git，未修改发布门禁、golden baseline 或真实用户数据库。上一轮报告保留；本报告更新懒对齐与标题区控件的行为说明。

## 修复结果

1. **读路径尽力对齐**：列表、工作区打开、单行摘要统一经 `align_reading_part_best_effort`。错误记录为 `[library] part alignment skipped for {id}: {error}` 后继续读取，下一次读取仍会重试；列表的候选扫描失败也只记录日志。对齐写入失败会回滚。题库主动设置及编辑器保存仍返回错误。
2. **保留旧自动判断**：自动来源（非 manual/none/canonical）的 P1–P3 标签与稿件不一致，且 journal 无人工 `setExamCategory` 记录时，以缓存的自动判定值修复旧稿件。新增 `EditOrigin::PartBackfill`，保留 CAS、版本递增、journal 和云端锁，不登记人工保护。普通内容保存不会提前把这份旧自动标签抹成默认 P1；其余情况仍投影稿件 category。
3. **人工历史持久性**：日志裁剪保留最新一条人工 Part 设置记录作为回填依据，在原有最近 200 条人工日志之外最多多留 1 条。避免发生“以前设过 Part，但日志已裁剪，于是误认为从未设置”的覆盖。
4. **标题旁仅一个控件**：删除重复的纯文本徽章，只显示 `aria-label="Part 归属"` 的下拉；云端校核、加载和冲突恢复时禁用。组件测试与真实应用脚本改为验证 select.value、可访问名称和禁用状态，不再把徽章当作存在条件。

## 命令层与组件测试

| 运行命令 | 最终结果 | 日志 |
| --- | --- | --- |
| `npm run check` | 通过，exit 0 | `artifacts/part-followups-check.log` |
| `npm test` | **55 files / 558 passed**，exit 0 | `artifacts/part-followups-vitest.log` |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib -- library:: authoring_v2 export cloud_direct:: processing::scheduler::tests` | **189 passed / 0 failed / 3 ignored / 1351 filtered out**，exit 0 | `artifacts/part-followups-rust.log` |
| `npm run build:app` | 通过，输入哈希无漂移 | `artifacts/part-followups-build.log` |
| `node --check scripts/e2e/tauri-cdp-part-assignment-sync.mjs` | 通过 | 命令输出无错误 |
| `git diff --check` | 通过 | 无空白错误 |

新增 Rust 回归：

- 缺 `passage.sourceAnchors` 的旧阅读稿，manual P3/category P1，加一条正常条目：重复 list 返回两条，get_workspace 和摘要成功；坏稿完整 JSON、版本、标签保持不变。主动题库设置/编辑器保存仍返回错误。
- title/content/range/filename 四种旧自动来源 P2/category P1：稿件改为 P2，版本仅 1→2 一次，来源仍是自动，journal origin 为 part_backfill，人工保护值不变。
- 人工设置 P3 后进行 202 次无关保存：人工 Part 历史在日志裁剪后仍存在；再出现旧自动 P2 标签时，列表以稿件 P3 为准，不改稿、不增版。
- 普通标题保存不会抹掉旧自动 P2；云端校核期间系统回填推迟，解锁后成功，以 part_backfill 来源增版一次。

组件测试覆盖可编辑时只有下拉、云端锁定时只有禁用下拉，以及加载/冲突恢复期间禁止 Part 修改。

首次 Rust 尝试有 1 个新增测试夹具失败：该夹具携带旧 V1 compatibility quality 标记，打开时触发了既有的独立质量兼容迁移。最终夹具使用当前 skipped 标记，隔离本次 Part 对齐失败行为；**完整 JSON 相等、版本不变的断言保留**。没有修改既有质量迁移或发布规则。最终完整运行上述指定命令通过。

## 端到端产品验证

命令：`node scripts/e2e/tauri-cdp-part-assignment-sync.mjs`。

最终 **passed，12/12 assertions**。驱动当前源码构建出的真实 Tauri exe、Rust、SQLite、学生预览和 NAS 文件输出，使用隔离的真实阅读导入稿副本。

[完整报告](F:/workspace/PDF2Test/artifacts/e2e-cdp/part-assignment-sync-2026-10-16T21-59-27-424Z/report.json)

| 断言 | 结果 |
| --- | --- |
| 题库 Part 下拉存在且可操作 | PASS |
| 题库改 P2 → 权威稿 P2，版本 1→2 | PASS |
| 无 processing row，打开中的编辑器跟随远端 P1→P2 通知 | PASS |
| 可编辑标题区仅 1 个 Part 下拉、0 个徽章，名称 Part 归属 | PASS |
| 锁定标题区仅 1 个禁用下拉、0 个徽章，P2 与锁定横幅可见 | PASS |
| 学生预览无编译错误，副标题和底部导航 Part 2 | PASS |
| 真实 NAS 发布结果 published | PASS |
| runtime.meta.category=P2，examId 保持 p1-d35bb0b9 | PASS |
| manifest.category=P2 | PASS |
| 编辑器 Part 下拉可改 P3 | PASS |
| 编辑器改 P3、撤销/重做、返回题库后 P3 持久化 | PASS |
| The Pearls 副本 P1→P3，版本 5→6，重复读取不增版，examId 不变 | PASS |

截图已实际查看，标题旁不再显示重复 Part：

- [可编辑状态：只有 P2 下拉](F:/workspace/PDF2Test/artifacts/e2e-cdp/part-assignment-sync-2026-10-16T21-59-27-424Z/editor-P2.png)
- [锁定状态：只有禁用的 P2 下拉](F:/workspace/PDF2Test/artifacts/e2e-cdp/part-assignment-sync-2026-10-16T21-59-27-424Z/editor-P2-locked-controlled-state.png)
- [学生预览 P2](F:/workspace/PDF2Test/artifacts/e2e-cdp/part-assignment-sync-2026-10-16T21-59-27-424Z/student-preview-P2.png)

构建 exe SHA256：`5baa51f7a8126517fc51d9fb7df87ce68bbd6412046addc55a9e10952dff61fe`。

## 证据边界与缺口

- **端到端产品**：Part 修改、远端通知、预览、真实 NAS 导出、撤销/重做与题库同步、The Pearls 副本对齐均在真实 Tauri 应用执行。
- **受控产品状态**：锁定截图在隔离测试库注入 cloud_recognition/running 行，再经真实导航重新读取而得到；没有改 DOM，也没有真实发起云端模型调用。锁定解除后移除该测试行，再继续正常预览/导出。
- **命令层证据**：坏稿不阻断读取、自动标签系统回填、人工历史防覆盖及日志裁剪由 Rust 临时库回归验证。本次未另做坏稿/自动回填的完整 UI 场景。
- **通道限定**：CDP 使用 `--no-sandbox --disable-gpu`，报告 `diagnosticRun=true`；未验证默认浏览器参数。
- **未覆盖**：没有重跑付费云端识别，也未启动外部 NAS 学生端。NAS 消费方源码审计沿用上一轮；没有把本次证据表述为外部 NAS UI 已通过。
- **仅 schema 证据**：本次没有以 schema-only 或 CLI-only 检查替代产品链路；Rust 测试为上述相关模块，未声称全量 Rust 测试全部通过。

## 本轮改动文件

- [library/commands.rs](F:/workspace/PDF2Test/src-tauri/src/library/commands.rs)：尽力对齐读路径、系统回填调用和 4 项新增 Rust 回归。
- [library/repository.rs](F:/workspace/PDF2Test/src-tauri/src/library/repository.rs)：PartBackfill、人工 category 历史判断、旧自动标签 CAS 保护、日志标记保留及云端锁。
- [ExamWorkspacePage.tsx](F:/workspace/PDF2Test/src/features/editor/ExamWorkspacePage.tsx)：单一可访问 Part 下拉及禁用条件。
- [ExamWorkspacePage.cloudReviewLock.test.tsx](F:/workspace/PDF2Test/src/features/editor/ExamWorkspacePage.cloudReviewLock.test.tsx)：新的单控件要求与不可编辑状态测试。
- [tauri-cdp-part-assignment-sync.mjs](F:/workspace/PDF2Test/scripts/e2e/tauri-cdp-part-assignment-sync.mjs)：以 select.value 断言实际 Part、单控件检查及受控锁定截图。
- 本报告及 `.planning/part-assignment-sync/{task_plan,findings,progress}.md` 跟踪记录。

上一轮其余未提交实现均保留；用户任务文档未改动，NAS 仓库没有改动。
