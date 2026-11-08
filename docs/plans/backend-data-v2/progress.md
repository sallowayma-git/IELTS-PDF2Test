# 实施日志

2026-11-08：已读取任务书、两仓库规则与现行契约；创建两个 codex/backend-data-v2 worktree。尚未修改实现。

## M1
- 两库已建立 codex/backend-data-v2 worktree。作者端 1a43569，学生端 a8be840，均含指定 co-author；未 push。
- 迁移旧数据完整、重复执行、未来版本拒绝：学生 schema-v2.test.cjs 通过；作者 local_v1_migration 测试通过。
- V1/V2 实际学生服务生成快照导入通过。作者真实快照夹具位于 fixtures/golden/synthetic/backend-data-v2。
- 证据层级：迁移/命令处理层，非作者端 UI。

## M2
- npm run build:server：通过。
- node developer/tests/exam/run-all.cjs：21/21 文件通过（含迁移、实际 Fastify 保存 handler、真实听力/写作/交卷服务）。
- npm --prefix apps/student-exam test：7 文件、51 测试通过。
- npm run build:student-exam：通过，apps/student-exam/node_modules 为自己的真实目录；根仅链接依赖，未运行根 npm install/ci。
- python3 developer/tests/e2e/suite_practice_flow.py：macOS 真实 Electron 通过（20261108-231955，阅读/高亮/笔记/写作/提交/快照）。
- python3 developer/tests/e2e/listening_v1_student_flow.py：真实 Electron 听力→阅读→写作→最终提交通过（20261108-232103），快照 9 条听力答案、14 条事件、2 篇作文。
- static suite 与 node --test JS：Windows 两项依赖 pwsh，macOS ENOENT 失败；其余静态/跨仓库契约通过，JS 13/15 通过。没有安装 PowerShell 或伪造通过。

## M3/M4 中间验收
- npx tsc --noEmit：通过；npx vitest run：65 文件/648 测试通过。
- Rust analytics：16/16 通过（V1/V2真实快照、冻结内容、改判撤销/统计、CAS、写作/批注/发布、重判、计入统计、8表导出）；新增边界测试待最终一轮。
- 全量 cargo test --lib：1584 passed / 0 failed / 19 ignored；最终 cargo test（含bin/doctest）收尾中。

## 环境与修正记录
- 首次 Rust build 缺 worktree PDFium 本地资源，建立未跟踪资源链接后成功。
- 第一次学生全套测试仅旧快照版本断言失败，按 V2 契约更新后通过。
- 第一次 Electron 主链停在 Windows 托管模式断言；按实际平台能力验证并显式报告 windowsKioskVerified=false 后，完整考试链通过。
- 一次自动文本编辑未找到 rustfmt 后的片段，未修改文件；按实际代码边界重新应用。

## 最终验收
- cargo test（含 bin/doc）：1587 passed / 0 failed / 19 ignored；其中 9 个私有语料早退项在验收报告另列，不能算实际验证。
- 最终 analytics 边界：20 passed / 0 failed；发布核心归档复证：38 passed / 0 failed。
- 真实 Electron 留存快照已加入 v2-electron，作者端实际导入与内容查询通过。
- 最终静态18/20、JS13/15，均只有 pwsh 缺失导致两项 Windows kiosk 检查失败。
- 完整作者端 UI 链退出3（缺 Windows exe），没有冒充通过。
- 最终细节、命令、文件与未验收项见 acceptance-report.md。
