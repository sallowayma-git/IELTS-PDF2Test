# 后端数据层 V2（2026-11-08）

继承 CurrentPaperV1.1 与 SubmissionSnapshotV1.1 的全部提交点、锁卷、NAS 只读与身份冲突规则。

## 迁移与快照

学生端 schema 1 → 2：仅在本终端 NAS 库执行事务内增表/增列；读取版本 >2 时拒绝打开，不降级。重复启动幂等。新增列不删除旧值；reading_answers.question_id 约定为运行时计分 slot ID（response_slot_id 默认回填 question_id），max_score=weight，score=is_correct*weight；未知判分保持 NULL。旧快照仍可导入。

SubmissionSnapshotV2 的布局、哈希与 ready 顺序保持 V1。schemaVersion=SubmissionSnapshotV2、dbSchemaVersion=2；meta 其余字段不变。作者端接受 (V1,1)/(V2,2)，过新版本进入异常队列，不读 NAS 活库。新表 answer_events、listening_attempts、listening_answers 随整库快照导出。写作正文与 raw_submission_json 作为事实导入，不改写。V2 覆盖 V1 的“新 exportedAt 覆盖”中与不可变事实冲突的部分：更新导入来源与补充缺失字段/记录可以，已导入原始答案、原始判分或全文发生冲突则进入异常队列，保留已有事实，不因 exportedAt 更新而接受篡改。

## 采集

answer_events 按 (attempt_id,event_seq) 唯一，seq 服务端单调分配；event_id 可选幂等键，用于客户端批次重试；事件类型 answer_changed/part_enter/part_leave/question_focus/question_blur/app_blur/app_focus。passage_index 使用学生库原有的 1-based Part 序号（旧导入快照保留其原序号，不重编号）。wall_time 为客户端 RFC3339，monotonic_ms 为同次运行 performance.now，不跨重启相减。仅采集粗粒度事件，失败不阻止保存/答题；无本机草稿。旧快照无事件不得伪造历史。

听力提交在 NAS 事务内冻结 exam_id/source_revision、逐 slot 答案、标准答案、分数与播放快照。重复相同提交返回原有结果，不改原始行；不同 payload 返回冲突。整个考试的 final-submit 保持唯一回执，听力阶段提交不提前结束整场考试。考试模式只认锁定 current-paper 的 listening entry；运行时能力版本统一 1.0.0（阅读原有 0.2.0 题包继续兼容；作者端现有听力题包 minimumRuntimeVersion=1.0.0，不降级包门禁）。

作废同样在本终端状态事务提交后导出 ready 快照（失败只记 pending），其 meta.submittedAt 可为 null；作者端只导入快照内 submitted/failed 关闭状态，failed 默认不计分。

exam_attempts 增加 session_id/student_uuid（可空、runtime 可选项）与 abandon_reason；学生身份名册与权限不实现。

## 作者端本地数据

analytics.sqlite 通过幂等事务迁移，包含 papers、paper_items、item_content_snapshots、原始导入表、判卷版本、改判、重判、审计与候选身份别名。papers 保存自包含指针，paper_items 保存 (paper,modality,part) 与锁定哈希；item_content_snapshots 的联合主键是 (exam_id,runtime_sha256)，只存发布 wrapper 中实际投影的 JSON，绝不根据当前权威稿代替历史。

先准备本地内容快照，再发布；指针成功后归档并切换当前状态。归档失败是发布后的 warning，不能声称 NAS 发布失败；NAS 历史可恢复，失败历史文件必须与实际指针/撤下记录区分。历史脚本已清理时返回 contentUnavailable，绝不借用新版。查询缺失内容需显式给异常。

passages 保存 raw_submission_json、highlights_json、note_text、timer_snapshot_json 与 lockedSequence 对应哈希；writings 保存题目/全文/metadata；V2 镜像事件与听力表。attempts.counted 初值按现有阅读有效性计算，教师选择以 counted_override 标记，重复导入保留教师决定、改判和批改。

## 判卷与审计

写入命令必须传非空 operator（来自设置的操作人姓名，当前无账号鉴权），返回稳定错误码与中文消息。版本写入使用 expectedVersion CAS；不存在/目标不属于 attempt/已结束版本操作拒绝。无账号不宣称角色权限检查。

原始 answers/passages/writings 永不由判卷命令修改。有效改判按 (attempt,modality,passage,slot) 至多一条；已有改判需先撤销，撤销保留审计。new_score 在 [0,max_score]，支持部分分；原始 is_correct 为未知时不允许改判。统计默认有效改判，可切换原始视图；分母为 max_score 之和，N 为参与样本数。

写作：四项 0..9、0.5 步长，task_band 只作为存储计算值=四项算术平均，不显示 IELTS Band。draft 可 CAS 修改，reviewed/published 冻结；新版本从新 draft 开始。同作文同一时刻最多一版 published，发布新版本时旧版归 reviewed，全部变更记审计。批注采用 UTF-16 offset（对应 JS 编辑器），引用文字必须匹配全文；已审核版禁止改批注。

重判规则本轮支持显式 paper/exam/slot 范围的阅读/听力可接受答案变体，preview 与 apply 共用匹配器；apply 接受 previewToken（受影响原始/有效分数/版本及规则哈希），漂移则冲突。结果批量新增 override，不改原始作答。

所有查询返回 N（对象集合附 n）；所有报表附「机构模拟测试原始分，非 IELTS 官方成绩」。Excel 分开原始、改判后、写作维度与听力，不输出 Band。
