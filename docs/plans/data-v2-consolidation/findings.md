# 发现与取舍

作者端输入：origin/codex/backend-data-v2=94f85b3，origin/codex/backend-data-v2-macos-20261109=699c689；共同基线 a7fa8ca。主线8cc30fa，另有独立前端 PR #5（7a9de8e）。

已确认两种作者 V2 不是同一 schema：第一种试卷档案在 library 库，第二种在 analytics 库；事件字段 client_event_id/clock_id 与 event_id 不同；听力 score/max_score 与仅 weight 有差异；writing_grades 状态/约束不同。仅文件合并不能完成数据兼容。

需要按原任务书逐项验证并决定取舍，不能依赖历史完成报告的断言代替代码和真实快照证据。

## 初步主实现选择
A=原 codex/backend-data-v2（作者94f85b3，学生2be1275），B=macOS 分支（作者699c689，学生595631d）。学生 main=f685552。以A的模块化作者/学生实现为主：具有显式 clock_id、按 runtime hash 限定重判、library 归档恢复、听力 Part/题型统计。B贡献：原始事实重复导入冲突校验、听力逐slot score/max_score、连续离散改答即时采集、身份合并循环保护、真实 Electron 产物。

关键缺陷：A 对更新 exportedAt 的既有 attempt 删除重建子记录，缺乏事实不可变约束；B schema与A同称版本2但字段/约束不兼容。初读曾考虑双V2迁移（已被用户明确取消）。最终只支持所选V2结构与已有V1升级，保留批改、审计和未知得分，不以文件无冲突作为验收。
学生仓库已定位新路径；AGENTS要求默认先确认，本轮用户已有明确对比/收敛/合入授权，不重复确认。依赖只安装在新worktree真实目录，未触碰旧main或共享node_modules。

用户确认两种V2均未上线，取消双V2兼容要求。主实现采用A；不搬运B的事件方言或分析库档案表，只按业务价值移植事实冲突防护和快速离散改答采集。听力当前生产协议每slot一分，max_score/score是否冗余先按所选判分规则核对，不为假想部分分数扩张模型。

实际回归发现A漏提运行时常量提升：阅读/听力共享reading-runtime-version.ts但仍为0.2.0，导致1.0.0听力包被拒；已采用B的1.0.0值（沿用唯一定义）。Electron配置归一化也会丢弃sessionId/studentUuid，已采用B的两字段透传。新增timeline测试首次误用非公开内部字段，改为通过buildSnapshot验证外部行为。旧analytics测试允许修改原始分，与任务书矛盾，已改为拒绝并验证整份回滚。
