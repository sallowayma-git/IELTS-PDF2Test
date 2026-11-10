# 后端数据模型 V2：准备阶段报告

日期：2026-11-08。准备完成不等于 M1–M4 实现。本轮没有改产品源码、运行数据库迁移或产品验收。

## 产出

- 两仓库隔离 worktree 与 codex/backend-data-v2 分支（见 execution-plan）。
- 保存原任务书；新增 execution-plan、design 草案和本报告。
- 22 张探索/独立核验卡、48 个主线程工作包；每项列出范围、依赖、验收点。
- 学生端 developer/contracts/2026-11-08-backend-data-v2-entry.md 引用作者端唯一计划/设计，不复制两份会漂移的契约；学生仓库 /docs/ 整体被忽略，不在该处交付未纳管文件。
- 独立 `.planning/backend-data-v2` 记录，不改原有长期 planning 历史。

## 模型与调度记录

最初工具元数据只列固定 luna low，用户要求 luna max 后明确暂缓；后续用户允许 terra 并给出完整模型名。`model="terra"` 调用被拒绝；`model="gpt-6.1-terra"` 成功。E01–E06 均用后者成功派发，未静默替换模型；六张卡均返回并已关闭。模型参数接受是调度证据，不额外声称已审计服务供应商内部实际路由。

| 卡 | 子代理 ID | 范围 |
| --- | --- | --- |
| E01 | 01a1bcc4-a43d-78d2-8ab5-3382ac47a9ec | 迁移与版本所有者 |
| E02 | 01a1bcc5-0d5b-7411-bc8a-21a16dd37c7b | 重导入与夹具 |
| E03 | 01a1bcc5-1043-76b2-8bd4-5d1c85087197 | 发布档案挂点 |
| E04 | 01a1bcc5-0eb4-7d13-b82f-7f98b344924c | 听力提交/锁定 |
| E05 | 01a1bcc5-11be-7cc0-9bb0-4ff9dde47e8f | 产品验收入口 |
| E06 | 01a1bcc5-1365-7fa0-9de7-ba43dc60d9db | slot 与保存通道 |

## 当前源码证据（关键结论主线程已按出处点验）

| 事实 | 出处（路径均相对对应 worktree，行号为准备基线） | 影响 |
| --- | --- | --- |
| 学生 connect 在 configureDatabase 后调用 ensureSchema，随后缓存连接 | 学生 server/src/lib/exam/MachineSubmissionStore.ts:176 | 首次打开适合挂迁移；空库先建基础表，不能机械照搬子代理“ensureSchema 之前迁移”的建议 |
| 版本键由导出器 ensureMeta 缺失时写 1 | 学生 server/src/lib/exam/SubmissionSnapshotService.ts:74 | 迁移与 meta 必须共享真实版本所有权 |
| 作者仅接受 SubmissionSnapshotV1 且 dbSchemaVersion 至多 1 | 作者 src-tauri/src/analytics/import.rs:210 | 必须双版本兼容，不仅改常量 |
| 作者重导入先删除 answers/passages/writings/suites | 作者 src-tauri/src/analytics/import.rs:419 | 教师层独立保存，外键不能级联删除批改 |
| 作者现有快照测试手写学生 schema | 作者 src-tauri/src/analytics/tests.rs:1、21 | 新增真实服务生成夹具，旧测试不作为真实兼容证明 |
| runtime 内容已由 receipt.runtimePath 读入 source_value | 作者 src-tauri/src/nas_package_v2.rs:637 | 可在发布路径固化锁定内容 |
| NAS 历史在 current-paper 指针替换前写入 | 作者 src-tauri/src/nas_package_v2.rs:447 | 历史存在不能直接证明成功发布 |
| manifest 先替换，考试唯一提交点仍是之后的指针 | 作者 src-tauri/src/nas_package_v2.rs:861、868 | 不能按子代理“清单提交后对学生可见”的简述推导考试生效 |
| 听力 submit 仅校验计算并 return，无落表 | 学生 server/src/lib/exam/ExamListeningService.ts:239 | 听力事务/最终交卷衔接是 M2 必做 |
| 听力 provider 从 manifest 查 examId，门禁用 appVersion | 学生 server/src/lib/library/listening/listening-v1-loader.ts:415 | 需切考试 pin 与统一 runtime 常量 |
| 锁卷事务只写阅读 lockedSequence，摘要返回听力 examId | 学生 server/src/lib/exam/ExamSuiteService.ts:146、180 | 听力需完整持久 pin，examId 本身不能锁版本 |
| reading 答案按 questionOrder/qid 一行一 slot | 学生 server/src/lib/exam/ExamReadingService.ts:269 | 兼容 response_slot_id 同 qid；不凭显示号拆分 |
| 阅读自动保存显式写 questionTimelineLite:[] | 学生 apps/student-exam/src/pages/ReadingExamPage.vue:680 | 事件采集需要修保存载荷，已有累计计数不等于改答历史 |
| 现有学生自建 E2E 卷 listening=[] | 学生 developer/tests/e2e/student_exam_altu_electron_flow.py:260 | 扩听力真实链，不能把阅读通过算听力覆盖 |
| 双产品链先检查 exe/夹具/学生脚本和构建新鲜度 | 作者 scripts/e2e/cross-repo-full-chain.mjs:403 | 实施 worktree 尚无构建，当前不能宣称 E2E 可运行 |

未逐行复核的子代理外围结论不升级为事实：例如整个仓库完全不存在环境变量 examId 来源、所有模式写作覆盖、Python Playwright 包安装状态、旧卷资源完整性。本轮只确认有限范围的入口；E07–E16 留作后续探索。

## 验证与迁移状态

准备检查：两次 `git worktree add -b codex/backend-data-v2 ... main` 均退出 0；两端源码基础文档读取与关键出处抽查完成。文档引用/任务 ID/分支/任务书 hash、`git diff --check` 与提交结果见本轮 progress 记录。

迁移：仅有草案，没有执行、没有改库。M1 迁移幂等/回滚/V1-V2 真实快照兼容均待实现和测试。

证据层级：本轮只有源码静态核对及准备材料检查；服务与命令处理层没有运行；真实 Tauri/Electron 没有运行；tsc/vitest/cargo/student 测试套件均未运行。新 worktree 未安装依赖、未构建，不复用旧 exe/dist 伪装当前源码验证。

## 下一步与未决

先完成 C01–C04 契约定稿，再进入 M1；其中无争议的版本迁移和原始数据保全可先继续细化。产品待确认集中在 execution-plan，不擅自替产品决定显示成绩或新增认证规则。本轮不把未来完整验收标为已通过，不 push。
