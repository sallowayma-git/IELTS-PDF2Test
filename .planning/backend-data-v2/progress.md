# 2026-11-08 准备记录

- 完整读取任务书、两仓库 AGENTS、两份生效跨仓契约；使用 planning-with-files 管理本轮独立记录，保留历史 root planning。
- 发现工具不支持 luna max，已询问并收到“必须 luna max，暂缓派发子代理”；没有启动任何子代理。
- 建立两个 codex/backend-data-v2 隔离 worktree；从作者原工作区复制未跟踪的任务书到作者实施 worktree，原文件保留。
- 当前仅做准备与只读入口核对；没有产品源码或数据库迁移变更，没有执行产品 E2E。
- 后续用户允许 terra 并提供精确模型名；model=terra 被拒绝，model=gpt-6.1-terra 成功。并发派发 E01–E06（只读），六卡均返回，主线程按关键出处点验后全部关闭。
- 新增 execution-plan（22 张探索/核验卡和主线程实施工作包）、design（DRAFT）、preparation-report；主线程纠正代理“ensureSchema 前迁移”对空库不适用、清单生效不等于考试指针生效等建议。
- 准备校验 Python 退出 0：任务 ID 无重复，22 张探索/核验卡、48 个主线程工作包齐全；学生入口 6 个链接存在；任务书副本 sha256=13770a945cde480199d9f5935c21e56879699e9943cd92f82d4a895c51abc180；两个 scoped planning 三文件齐全。
- 两端原 main `git status --short` 与起始相同；两实施分支均可解析；未构建的作者 exe 与学生 Electron 均不存在。
- 学生 /docs/ 被忽略，已将学生引用入口放到 developer/contracts/，不修改忽略策略。
- 迁址后复核 6 个链接通过；两端 git diff --cached --check 退出 0；暂存范围只有 Markdown 与 .planning/.active_plan，活动指针为无 BOM UTF-8。准备阶段验证完成，提交结果以 git log 为准。
