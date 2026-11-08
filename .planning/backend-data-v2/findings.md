# 本轮事实与待核对项

## 已确认
- 作者基线 a7fa8ca；学生基线 f685552。原作者工作区有未跟踪任务书；原学生工作区有未跟踪 .workbuddy、planning、dist-prev-r14、tmp，均保留。
- 工作区分别 F:/workspace/PDF2Test-backend-data-v2 与 F:/workspace/IELTS-NASfor-WenDao-backend-data-v2；分支各为 codex/backend-data-v2。
- 当前指针 V1.1 是考试唯一提交点；不可变 release、锁卷、撤下后 24h 宽限不能改。
- 快照现有契约 V1；实际学生导出器的 SNAPSHOT_SCHEMA_VERSION=SubmissionSnapshotV1、DB_SCHEMA_VERSION=1，ensureMeta 只在键缺失时设置版本。
- 作者 analytics 模块已拆成 import/queries/export/status/tests；mod.rs 负责 schema 与启动，当前迁移只有 attempts.install_id 补列。

## 约束与未决
- luna max 不可用，最初暂停；用户后续授权的 gpt-6.1-terra 已成功派发六卡，关键出处点验完毕，详见 preparation-report。
- 全局“不显示 IELTS Band”与 writing_grades.task_band/0–9 四维的对外呈现边界需产品确认；草案不擅自决定对外写作评分口径。
- 旧 NAS 历史 JSON 不保证包含完整 runtime source；缺源必须显式标记不可恢复，不能用当前题库稿冒充锁定版本。
- “准备”阶段不运行完整产品验收；后续必须区分产品 E2E、命令处理层、单测/schema。

## 六卡返回后主线程点验
- 听力 submit 只返回分数，provider 按 manifest 与 appVersion 加载；锁卷当前只持久阅读序列，M2 需同时补完整听力 pin 与落库。
- 阅读自动保存 questionTimelineLite 显式为空；历史改答不能仅靠累计 change_count 恢复。
- NAS 历史在指针替换前写入；孤立历史不证明已成功发布，补建需恢复态。
- 作者子表整组刷新不可连带删除教师层，原手写 SQLite 测试不是学生真实快照。
- 完整证据 file:line 与对代理简述的纠正见 docs/plans/2026-11-08-backend-data-v2-preparation-report.md。
