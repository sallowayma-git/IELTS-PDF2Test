# 学生预览反馈修复验收（2026-10-31）

工作基线 PDF2Test `0880eba`。两个仓库均只修改工作区，没有提交，没有修改发布门禁、golden baseline 或真实用户数据库。最初未跟踪的任务说明及 NAS 用户文件均保留。

## 修复与根因

1. 作者端学生预览新增顶栏 `Note`、`Notes` 面板和 `Close`；选区工具条新增 `Note`，把文字标为 `kind=note` 并按 NAS 的 `> 引文` 格式追加笔记。点击已有高亮可以转为 note。普通高亮使用 NAS 的梅红色、note 使用蓝色。状态仅存在于预览组件和 DOM，退出预览清空，不修改权威稿或答案控件。
2. 两端采用同一份结构判定：所有 response 都为 matching/per_slot/单槽，交互明确为 radio 或 checkbox，选项集合一致才显示表格。以第一行选项顺序为列顺序；不满足条件保留逐组列表。显式 inline 选项严格比较 optionId 和 label。旧编译器生成的段落选项 ID 含 response ID，每行天然不同；只对匹配生成格式、内容等于字母且 paragraphMap 可确认的选项按段落目标统一身份，保证旧包无需重新导出。作者模式保留真实选项的文字编辑、增删和排序入口；paragraphMap 派生选项继续受原有 optionSourceEditable 限制。两端支持字号继承和横向滚动；NAS 使用现有深色主题令牌。
3. Part 有三条经过验证的缺陷：
   - **自身保存回声**：后端提交后、返回保存结果前通知；前端误记 deferred，且保存排空后只处理“仍需重载”的记录，遗漏清理已被保存吸收的记录。现在在途 N+1 事件先暂存，收到成功保存结果才确认是自身回声；如果 CAS 失败，仍把该事件作为真正远端更新处理。保存完成后清理已吸收的 deferred。选择前端处理是为了不依赖后端通知与 IPC 返回的调度顺序；同版本真实远端写入不会被无条件忽略。
   - **云端直出覆盖人工 Part**：write_direct_canonical_inner 以 cloud_candidate_adoption 来源整稿替换，没有合并 setExamCategory 的 `$top:exam` 人工保护。修复前真实调度器重新识别将 P3→P1，版本 8→9，journal 明确记录该来源，part_source 却仍是 manual。现在每次 CAS 尝试都重新读取并保留受保护的 exam 范围；兼容仅有 manual Part 投影的旧数据。未受保护的机器稿仍允许自动更新 Part。
   - **停止处理后的重试竞态**：工作区先本地解锁再异步取消，解锁效果触发的 flush 可以早于后端取消提交，再次撞 CLOUD_REVIEW_IN_PROGRESS。现在等待取消成功落库后才解锁；取消失败保留锁并显示错误。

编辑/学生预览共用同一个 Part 下拉和 setExamCategory 路径。保存失败时保留未保存命令和选择；Part 因云端锁被拒绝时明确提示“修改尚未保存、选择已保留”。人工冲突和校验失败继续显示现有重试/放弃入口。

## 命令与结果

日志均在 `F:/workspace/PDF2Test/artifacts/`。

| 命令 | 最终结果 | 日志 |
| --- | --- | --- |
| `npm run check` | 通过 | `student-feedback-check.log` |
| `npm test` | 56 files，583 passed | `student-feedback-vitest.log` |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib -- library:: authoring_v2 export cloud_direct:: processing::scheduler::tests` | 190 passed，3 ignored，0 failed | `student-feedback-rust.log` |
| `npm run build:app` | 通过，输入哈希无漂移 | `student-feedback-build.log` |
| `node scripts/e2e/tauri-cdp-student-feedback.mjs --before`（基线构建） | 按预期失败，21 项中 8 项失败 | `student-feedback-before.log` |
| `node scripts/e2e/tauri-cdp-student-feedback.mjs`（最终构建） | 46/46 passed | `student-feedback-after.log` |
| NAS `npx tsc --noEmit -p server/tsconfig.json` | 通过 | `student-feedback-nas-server-typecheck.log` |
| NAS `npm exec --yes --package=vue-tsc --package=typescript -- vue-tsc --noEmit -p developer/tests/student-exam/matching-matrix-typecheck.json` | 通过 | `student-feedback-nas-vue-typecheck.log` |
| NAS `npm --prefix apps/student-exam test` | 5 files，42 passed | `student-feedback-nas-test.log` |
| NAS `py developer/tests/ci/run_static_suite.py` | 通过 | `student-feedback-nas-static.log` |
| 随后 NAS `py developer/tests/e2e/suite_practice_flow.py` | 真实 Electron 主链通过 | `student-feedback-nas-practice.log` |
| NAS `py developer/tests/e2e/reading_matching_matrix_flow.py --package F:/workspace/PDF2Test/artifacts/e2e-cdp/student-feedback-2026-10-31T10-01-01-906Z/nas` | 14/14 passed，真实提交计分 | `student-feedback-nas-matrix.log` |
| 两仓 `git diff --check`，两个新验收脚本语法检查 | 通过 | 命令输出 |

NAS 原来没有前端 tsconfig/typecheck 命令；新增测试目录下的配置检查本次 renderer 及传递依赖，不声称检查整个 Vue 应用。类型检查暴露并修正了三处旧问题：select 分支不可能再是 dragdrop、align 的 unknown 类型未收窄、fetch headers 缺 HeadersInit 注解。未改有效题稿或接口语义，没有新增运行时依赖或修改 lockfile。

新增回归覆盖 Note 两入口、面板输入/关闭/重开、切回作者清空、note 不写稿；共享及一致 inline 选项、不同 ID/label 回退、dragdrop/select/多槽/无序组回退、答案写入与 checkbox 语义；自身回声与同版本真实外部冲突、高版本/未知远端更新、保存失败保留 Part 和提示；取消提交前不解锁、不重试，成功后重试及失败提示。Rust 测试覆盖人工 exam 保护、机器新正文仍采纳、版本与 journal、manual 投影、旧数据及未保护 Part 仍可更新。

## 产品证据与边界

**修复前真实 Tauri**：

[报告](F:/workspace/PDF2Test/artifacts/e2e-cdp/student-feedback-before-2026-10-31T09-37-09-459Z/report.json)，21 项中 8 项失败：两种模式自身保存提示残留，以及真实调度器直出覆盖人工 Part。基线 exe SHA256 `2455884d0a1c7d9d65625cfc5cdf58eddb9c9985906f4c293e9be4df622c9b01`。

验收输入是 `The Pearls.pdf`：A–G 段落、题号 1–13、Questions 1–4 信息匹配、没有题组 optionBank，文件名不含 P1/P2/P3。通过真实导入界面、实际 PDF 摄取、HTTP 网关、直出归一化、调度器、版本事务和 SQLite。HTTP 服务回放此前真实云端识别的完整响应，**没有重新调用付费云端模型**。题目 80 不在本机，没有声称复现原题。

**修复后真实 Tauri**：[最终报告](F:/workspace/PDF2Test/artifacts/e2e-cdp/student-feedback-2026-10-31T10-01-01-906Z/report.json)，**46/46 passed**，无 CDP 重连。脚本 `node scripts/e2e/tauri-cdp-student-feedback.mjs` 覆盖两种模式 P1→P3→P2、题库/重开、真实重新识别后保留人工 P3、Note 两入口和表格答题、字号与窄屏、权威稿完全不变、真实 NAS 发布。使用默认 WebView2 沙箱，只启用 CDP 自动化端口。最终 exe SHA256 `a91e1e37b479af38bc0e3b3663989b135119fa94a529a2bf0cefe6396dc7f284`，构建输入哈希无漂移。

截图已查看：[Notes 与选区蓝色标记](F:/workspace/PDF2Test/artifacts/e2e-cdp/student-feedback-2026-10-31T10-01-01-906Z/student-Notes-and-matrix.png)、[表格选中态](F:/workspace/PDF2Test/artifacts/e2e-cdp/student-feedback-2026-10-31T10-01-01-906Z/student-matrix-selected.png)、[作者预览窄屏](F:/workspace/PDF2Test/artifacts/e2e-cdp/student-feedback-2026-10-31T10-01-01-906Z/student-matrix-narrow.png)、[学生预览保存被锁时保留 P3 并提示](F:/workspace/PDF2Test/artifacts/e2e-cdp/student-feedback-2026-10-31T10-01-01-906Z/student-save-locked.png)。锁失败时 canonical=P1/UI=P3；停止并补存后两边 P3，再改 P2、重开仍 P2。两个模式均通过。

**受控 Tauri 并发状态**：为稳定复现“选了 Part 后才撞云端锁”，仅在隔离库写入受控 running/lease 状态并发出对应处理通知；自动保存仍调用真实 Rust 并被实际锁拒绝。通过实际“停止云端校核”按钮取消并补发保存，随后模拟该受控 worker 的取消终态。这部分不是新的真实模型调用，也不冒充自然时序必然发生；报告保留 UI、exam.category、part_label、part_source、版本和 journal。

**NAS Electron**：正式启动→登记→三 Part 套题→实际发布的 Pearls V2 包→A–G 表格→选 C→字号 16/18/20→深色模式→窄屏横向滚动→Part 导航后恢复→提交→数据库 q1=C/isCorrect=1 和回执。P1/P2 陪跑题使用旧的真实发布包，临时目录只组合原 manifest 条目，不改 payload 或重算校验和。使用 Electron CDP 和 `--no-sandbox --disable-gpu` 诊断参数，不能表述为默认沙箱也已验收。

[NAS 最终报告](F:/workspace/IELTS-NASfor-WenDao/developer/tests/e2e/reports/matching-matrix-20261031-100255/report.json)，**14/14 passed**，消费本次最终 Tauri 导出包。截图已查看：[浅色表格](F:/workspace/IELTS-NASfor-WenDao/developer/tests/e2e/reports/matching-matrix-20261031-100255/matrix-light-selected.png)、[深色表格](F:/workspace/IELTS-NASfor-WenDao/developer/tests/e2e/reports/matching-matrix-20261031-100255/matrix-dark-selected.png)、[窄屏](F:/workspace/IELTS-NASfor-WenDao/developer/tests/e2e/reports/matching-matrix-20261031-100255/matrix-narrow.png)、[提交成功](F:/workspace/IELTS-NASfor-WenDao/developer/tests/e2e/reports/matching-matrix-20261031-100255/submission-complete.png)。窄屏矩阵 scrollWidth=600、clientWidth=455，实际可横向滚动。

**单独的现有缺口**：NAS 管理员目录选择器在 `electron/main.js:931` 要求 manifest 同目录有题目 `.js`，拒绝 publisher 把脚本放在 `releases/` 的包。本次 NAS 验收使用支持的隔离 `exam-runtime.json` 预配置目录；管理员选择此类包的流程尚未修复。没有复制假根脚本来掩盖这个问题。

**证据分类**：上述 Tauri/NAS 操作是实际应用/运行时产品证据；锁拒绝是受控状态下实际 UI+命令证据；Vitest/Rust/typecheck/static suite 分别为组件、事务或静态证据。没有以 CLI/schema 通过替代产品操作。作者应用没有现成深色模式切换入口，本次作者侧核验浅色/字号/窄屏；NAS 深色模式有实际截图。

## 改动文件：PDF2Test

- `src/exam-canvas/StudentReadingTools.tsx`
- `src/exam-canvas/ExamCanvas.tsx`
- `src/exam-canvas/renderers/MatchingMatrix.tsx`
- `src/exam-canvas/renderers/matchingMatrixModel.ts`（新增）
- `src/styles/exam-canvas.css`
- `src/exam-canvas/studentReadingExperience.test.tsx`
- `src/exam-canvas/taskPresentation.test.tsx`
- `src/features/editor/useCanonicalEditor.ts`
- `src/features/editor/useCanonicalEditor.partAndRemote.test.tsx`（新增）
- `src/features/editor/ExamWorkspacePage.tsx`
- `src/features/editor/ExamWorkspacePage.cloudReviewLock.test.tsx`
- `src-tauri/src/cloud_direct.rs`
- `scripts/e2e/tauri-cdp-student-feedback.mjs`（新增）
- 本报告及 `.planning/student-preview-feedback/{task_plan,findings,progress}.md`

## 改动文件：IELTS-NASfor-WenDao

- `apps/student-exam/src/modules/reading-engine/renderers/ReadingExamV2Renderer.vue`
- `apps/student-exam/src/modules/reading-engine/renderers/ReadingExamV2Renderer.test.ts`
- `apps/student-exam/src/modules/reading-engine/renderers/MatchingMatrix.vue`（新增）
- `apps/student-exam/src/modules/reading-engine/matchingMatrixModel.ts`（新增）
- `apps/student-exam/src/modules/reading-engine/renderers/AnswerSlotNode.vue`
- `apps/student-exam/src/modules/reading-engine/renderers/ContentNodeRenderer.vue`
- `apps/student-exam/src/modules/exam/examApi.ts`
- `developer/tests/student-exam/matching-matrix-typecheck.json`（新增）
- `developer/tests/e2e/reading_matching_matrix_flow.py`（新增）
