# 后端数据模型 V2：契约与迁移设计

状态：实施基准（2026-11-08）。现行契约已先追加 Snapshot V2 与 CurrentPaper V1.2 增量；代码按里程碑启用，完成状态见 progress。用户确认写作对外称“写作人工评分”，保留非官方声明；阅读/听力本轮不新增独立成绩发布，保留后续扩展。本轮“权限外”指终端/对象归属、状态、分值范围与 CAS，不引入账号体系。counted 仅控制可判分记录是否参与统计，不能把无效/失败考试变成有效成绩。

## 不变量

学生只写自己的 NAS `terminals/<terminalId>.sqlite`，DELETE journal、进程内单连接；作者不打开活跃终端库，仅消费 `.ready` 快照，NAS 源只读。作答与事件不在学生本机持久化。练习模式现有行为不能被考试模式的指针约束误伤。

考试唯一提交点仍为 `current-paper.json` 原子替换；manifest 为目录/练习用途。阅读与听力都应以锁定版本取内容/判分，重新发布不影响在途考生。任何历史回填不允许把当前 canonical 稿当成锁定时原文。

学生原始分、教师复核、已发布结果是不同事实层。教师命令不修改学生答案/事件/判分原始行；重导入学生快照不删除教师事实。原始分与正确率带 N，沿用“机构模拟测试原始分，非 IELTS 官方成绩。”。

## SubmissionSnapshotV2（生效增量）

meta 沿用 V1 的完整字段，仅变更：`schemaVersion="SubmissionSnapshotV2"`、`dbSchemaVersion=2`。整库快照、三件套原子可见、ready 最后、hash/size、补发 pending、terminal/install 冲突、paperId 优先级均不变。不能只改常量就声称迁移完成。

| schemaVersion | dbSchemaVersion | 作者消费规则 |
| --- | --- | --- |
| SubmissionSnapshotV1 | 1 或历史缺省 | 保持旧解析与缺省规则；V2 增量为空/回填，不报成数据丢失 |
| SubmissionSnapshotV2 | 2 | 要求必要新列/新表存在，字段和版本一致 |
| 已知 schema | 高于 2 | 异常队列，提示升级作者端，整份不导入 |
| 未知 schema / V2 与 1 混搭 / 畸形版本 | 任意 | 明确异常，不静默降级猜测结构 |

只在 `.ready` 候选经过文件安全检查、hash/size、一致性/身份校验之后导入；仍先复制到作者本地临时目录再只读开库。整库内其他 attempt 的身份与 paperId 仍按各行处理，不能把 meta.attemptId 的属性套给全库。

## 学生 schema v2

版本所有权从快照导出器的“缺失则标 1”改为连接初始化的事务迁移。新建空库先有基础表；旧库先探测版本/实际列；DDL、回填、版本键必须在一个事务成功后才可见。不能在 exam_kv 尚未建立时直接读取它。过新版本拒绝写入。迁移失败关闭未缓存的连接，避免半升级连接被复用。

| 表 | 增量 / 键 | 兼容与约束 |
| --- | --- | --- |
| reading_answers | response_slot_id、max_score、score | 旧行 slot=question_id、max_score=weight；is_correct=1→score=weight，0→0，null→null，不把未判分冒充零分 |
| exam_attempts | abandon_reason、session_id、student_uuid | 均可空；student_uuid 不接名册、不生成伪身份 |
| answer_events | attempt_id、event_seq、event_type、passage_index、question_id、old/new_answer_json、wall_time、monotonic_ms | attempt 内事件序号唯一、重试去重；事件种类为任务书枚举 |
| listening_attempts | attempt_id、exam_id、source_revision、started/submitted_at、score_correct/total、playback_snapshot_json | attempt 的听力事实；源 hash/pin 关联必须可追溯 |
| listening_answers | attempt_id、slot_id、question_kind、user/correct_answer_json、is_correct、weight | 按 slot 唯一，不信任客户端提供的标准答案或分值 |

现有 reading `questionOrder` 已按评分 slot 遍历；V2 明确 `question_id` 保留兼容 slot ID，新增 response_slot_id 显式同值。旧 scorer 的数组 slot 可能 weight>1，不能无证据拆行、改变旧得分。若遇真正一逻辑题多 slot 的新形态，使用 runtime 的 slot ID，不用显示题号重建键。

事件沿现有保存通道批量提交，renderer 只保存内存队列。去重 ID、服务端 event_seq 分配、重启后 monotonic 原点标识须在 C02 定稿，不能以 wall_time 排序替代单调序号。事件保存失败不使答案保存失败；缺事件时报告活动区间不完整，不能把缺失区间推定为无活动或零用时。旧库无法回填改答历史。

听力当前链存在 manifest 加载与无持久锁定问题，故 M2 必须新增 attempt 的听力 pin（与阅读同次锁卷持久化完整 entry/script/hash，不只存 examId）。考试 API 拒绝请求路径中不属于已锁卷的 examId；恢复考试读已存 pin，不重新读取当前指针。无听力旧卷保持兼容。统一 runtime 版本常量，不再比较安装包版本。

听力提交与最终交卷当前是两个独立服务。先定义听力完成事实的幂等事务，再明确最终交卷是否要求听力完成并在一致性快照中包含它；不能为听力新增第二张冲突的 attempt 最终回执。最终提交重试、事务失败回滚与 NAS 快照补发沿用现有语义。锁卷在签到后、进入听力前调用 suite.start；恢复从该 attempt 的持久 pin 及 NAS 进度继续。

## 作者本地数据归属（C03 定稿）

| 所有者 | 数据 | 迁移策略 |
| --- | --- | --- |
| 本地 library SQLite | papers、paper_items、item_content_snapshots | 接现有 library/schema.rs 只追加迁移，当前基线版本 10，不重用历史编号 |
| analytics.sqlite 原始镜像 | attempts/suites/passages/answers/writings 与事件/听力镜像 | 新增明确的本地 schema 版本，保留已有 install_id 的升级能力 |
| analytics.sqlite 教师层 | writing_grades/annotations、score_overrides、rejudge_runs、audit_log、candidate_aliases、counted/exclude_reason | 独立表/教师字段所有权，学生重导入 SQL 不覆盖 |

NAS 文件与本地数据库无法用同一个 SQLite 事务原子提交。发布时先持久化内容证据及待提交批次；指针成功后更新本地 current/superseded 状态。指针已成功而本地失败时不能假报“未发布”或删除线上 release；报告“已发布、档案待修复”，启动恢复必须幂等。历史 JSON 可补建为 archive_state=unconfirmed 的非当前档案；只有实际指针或本地已确认事实能使其成为 confirmed，撤下时间记录本身不证明过去已提交。

现有 NAS 历史是在指针前写入，因此“发现历史 JSON”不等于该卷曾发布成功。补建时对照实际指针、发布记录和撤下证据，不能将孤立历史都标为 current/superseded；无法证明的记录带异常/未确认状态。采用独立 archive_state=confirmed/unconfirmed；status 仍保留 current/superseded，不混入学生考试状态。

`item_content_snapshots` 主键 `(exam_id,runtime_sha256)`；保留编译出的完整 runtime source，不另算一个不同序列化方式的 hash。`paper_items` 保存 script/runtime/asset hashes 及 modality/part 归属；part 字符串不能充当唯一身份（如阅读和听力都可能出现 Part 1）。

内容查询只按学生 pin 的 examId/runtimeSha256，scriptSha256 作关联校验；原文、题干、选项、标准答案、解析从锁定 runtime 的实际结构提取。历史 release 内容已清理时显式 `contentUnavailable` 和原因，不能从当前题库猜补。NAS 历史回填只读文件，不写学生 submissions 或活跃库。

## 分析导入与教师层保护

passages 新增 highlights_json/note_text/timer_snapshot_json/runtime_sha256/script_sha256；answers 新增 slot/max/score；writings 新增 prompt/content/metadata；镜像 answer_events/listening 两表；attempts 新增 session/student_uuid/abandon_reason 与作者自己的 counted/exclude_reason。

重导入只刷新学生所有的镜像字段；不得级联删除 grade/annotation/override/audit/rejudge/alias，不得重置 counted。教师表引用稳定 attempt/slot 身份，不能通过 FK cascade 绑到会被整组重建的子行。若原始同 slot 内容变化，显式记录证据冲突，不能把旧 override 无条件应用到新原始答案。

有效计分与人工排除分开：`eligible` 为原始数据可判分资格，`counted` 为教师选择；统计参与条件为二者均满足。新记录按任务书现行有效规则初始化，后续学生快照更新不得无条件重设教师选择。缺有效原始数据的 attempt 不能靠 counted=true 制造分数。

V1/V2 的 Rust 夹具必须经真实学生服务/导出器生成，记录学生 HEAD、schema、生成命令及 hash。允许保留手写异常/损坏库测试，但它们只能证明异常处理，不能作为真实兼容证据。

## 教师命令与并发（C04 定稿）

所有新命令采用稳定错误码与中文消息，保持现有 analytics_* 兼容；列表/统计显式 N。操作人来自设置字符串，记审计但不当作认证。禁止客户端指定任意 SQLite/NAS 写入路径。

| 命令族 | 写入与一致性要求 |
| --- | --- |
| list_papers/get_paper/get_question_context/get_attempt_detail | 只读；缺历史内容独立报告；保留原始与有效分、上下文/事件/批改版本/听力/审计摘要 |
| create/revoke/list override | 有效 override 按 attempt+modality+passage/听力项+slot 唯一；理由、操作者必填；新分数在 0..max_score；撤销保留历史 |
| preview/apply rejudge | preview 无写入；apply 重验范围/原始证据/版本，批量新增 override 与 run/audit 同事务，不改原始答案 |
| save/review/publish writing grade | draft 写入带 expectedVersion CAS；四维有限值 0–9、步长 .5；task_band 保留精确等权均值、不擅自取整；同作文最多一版 published |
| save/delete annotation | 引用属于指定作文/grade；UTF-16 起止偏移需匹配 quoted_text，中文与 emoji 不按 UTF-8 字节截断；CAS/审计要求同属父 grade |
| set_attempt_counted | 原因审计，与 eligible 分开；不更改学生原始 status |
| list_writing_queue/list_audit_log | 稳定过滤、分页及 N，限定对象范围 |
| analytics_export_xlsx 扩展 | 原始与有效分区分；加改判后/写作/听力表；导出事实记录成功/失败边界，不把取消保存报为成功 |

写操作与其 audit_log 在同一 analytics 事务提交；CAS 冲突不留下半个操作或成功审计。单一 published 写作版本使用数据库唯一约束兜底；发布后的正文与批改事实保留历史，后续改稿创建新版本。用户确认本轮仅复核与统计，公开阅读/听力成绩的独立发布实体留待后续。

有效成绩查询集中实现，再供详情、分布、题型、难题、学生矩阵与 xlsx 复用；部分分以 score 求和，不再以 is_correct×weight 代替所有新题型。分母为 max_score/weight 的得分点和，N 是对应范围内参与统计的有效考生数量；跨 paper/runtime 的同题号不得混组。

## 实施状态与接口

C01–C04 已定稿并实现；命令 DTO、错误、CAS、N、审计和写作发布规则见 [命令接口](2026-11-08-backend-data-v2-command-contract.md)。教师操作身份来自独立 review_settings；本轮没有账号权限、名册 UI 或新增判卷页面。现有导出需要先通过设置命令填写操作人，页面接入属于后续前端任务。

M1–M4 进度和本轮实际验证分层见 [里程碑报告](2026-11-08-backend-data-v2-milestones.md)。历史准备报告与原任务卡保留，不能当作当前实现状态。
