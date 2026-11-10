# 后端数据模型 V2 实施准备

## 目标与本轮边界
按 docs/plans/2026-11-08-backend-data-model-prompt.md 准备两仓库实施，交付隔离工作区、可派发任务卡、契约草案、风险与验收矩阵。本轮不把准备完成表述为 M1–M4 已实现。

## 状态
- [complete] 主线程读取任务书、两仓库 AGENTS 和当前试卷/交卷快照契约。
- [complete] 建立两仓库 codex/backend-data-v2 worktree，保留原工作区现场。
- [complete] 六个 gpt-6.1-terra 探索任务返回；主线程按出处核对关键入口，明确未核实项。
- [complete] 编写 22 张探索/核验卡与细粒度主线程实施工作包及依赖。
- [complete] 编写 V2 契约/本地库设计草案，明确未决产品问题。
- [complete] 准备材料检查与暂存范围检查通过；两仓库准备提交以 git log 为准，不 push。
- [complete] 用户后续允许 terra 并明确 gpt-6.1-terra；精确模型名调用成功，E01–E06 返回并关闭；无模型阻塞。

## 实施里程碑
- [in_progress] M1 契约定稿和两端迁移。
- [pending] M2 学生采集。
- [pending] M3 作者档案/真实快照导入。
- [pending] M4 判卷命令/统计/审计。
- [pending] 最终真实产品验收。

用户已授权完成整个实施计划。写作“人工评分”可对外导出，保留声明；阅读/听力成绩发布推迟。新 spawn 报 agent type unavailable，恢复原 terra agent 后继续只读探索。

## 规则
子代理仅探索/检索/独立核验；主线程完整读取基础文档和将修改的确切代码，负责代码修改、设计决策与最终验收。所有派发使用用户已授权的模型参数，不能用提示词声称模型已升级；本轮为 gpt-6.1-terra。

## 错误与纠正
- 初始工具元数据过滤过宽、旧 planning 文件合并读取输出被截断：改为工具名前缀精确过滤、按文件/区间读取。没有据截断内容作完成判断。
- 学生端子目录 rg --files -g AGENTS.md 返回 1：未找到额外 AGENTS，根 AGENTS 已完整读取。
- `model=terra` 返回 Unknown model；用户给出精确 gpt-6.1-terra 后成功。没有使用未授权替代模型。
- 检查不存在的新 worktree exe 时 Get-Item 返回非零：证据为未构建，不能报告产品通过；改用 Test-Path 显式登记。
