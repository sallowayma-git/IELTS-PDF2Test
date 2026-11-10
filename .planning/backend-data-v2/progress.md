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

## 实施进行中
- 用户授权完成 M1–M4；确认写作对外“人工评分”且有非官方声明，阅读/听力独立成绩发布留后续。
- 新 spawn 报 agent type unavailable；resume 后 E07 代理自报模型为 sol，已停止使用恢复代理并尝试关闭（返回 not found）；主线程继续，不宣称当前子代理满足 terra。初轮准备六卡仍按其原调度记录报告。
- 先更新生效契约 Snapshot V2 / CurrentPaper V1.2，再实现终端库事务迁移、作者分析迁移与 library v11 档案表。
- 学生 build:server 通过；迁移新增 3/3、连接模型 8/8 通过；快照测试旧 V1 断言需要更新到实际 V2，已逐项修正，待复跑。
- 作者首次 cargo 测试缺 pdfium.dll（worktree未带忽略的资源），已从原目录复制运行库；后续 Rust 编译成功，原 analytics 10/11，通过前10项，过新版本用例的未来值由2改3，待复跑。新增迁移3例随后纳入。
- 当前在实现听力pin/事务落库、事件保存、档案与教师命令；源码尚未提交，不把任何里程碑宣称完成。
- 学生静态套件首跑失败：私有 apps/student-exam 缺 vite，正在通过 build:student-exam 安装该私有子目录依赖。根 node_modules 是junction，未在根执行npm ci/install。
