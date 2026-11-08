# 数据 V2 命令接口

统一错误格式沿用现有 Tauri `CommandResult`：`CODE:中文消息`。无账号体系，不声称角色鉴权；操作人必填（或从作者端本地操作人设置读取）。本轮不改作者端页面。

读取：
- `list_papers({destination?: string})`：`{n,papers,recovery}`。传 NAS 发布目录时只读补建历史档案；启动读取当前试卷时也补建。
- `get_paper({paperId})`：`{n,paper,items}`，items 含冻结哈希和 content_available。
- `get_attempt_detail({attemptId})`：`{n,attempt,paper,passages,events,writings,listening,overrides,anomalies,importExceptions,audit,identityAliases,disclaimer}`。原始导入字段使用 SQLite snake_case；补充层字段见返回值，包含 effectiveScore / effectiveIsCorrect / context / activityIntervals。
- `get_question_context({examId,runtimeSha256,questionId})`：slot、promptText、promptHtml、options、optionBanks、完整 taskGroups、answer、explanation、explanationAvailable。历史运行时没有解析内容时返回 null/false，绝不生成解析。
- `list_overrides({attemptId})`、`list_writing_queue({paperId?:string,status?:draft|reviewed|published|ungraded})`、`list_audit_log({filter:{targetType?,targetId?,action?,limit?,offset?}})` 均返回 n。
- `analytics_exam` 保留原参数，增加 `original?:boolean`，默认使用有效改判；`get_attempt_detail` 同时返回两层，原始行不变。
- `analytics_listening({paperId,original?:boolean})`：分布/学生矩阵、题型和逐 slot 加权统计、hardTop10；每项带 N，小样本不下难题结论。

写命令统一参数 `{input:{...}}`，可传 `operator`，未传时使用 `set_analytics_operator` 保存的姓名。所有写入与审计同事务；版本冲突整组回滚。

- `set_analytics_operator`：`{operator}`。
- `create_score_override`：`{attemptId,expectedVersion,modality:reading|listening,passageIndex?,responseSlotId,newIsCorrect:boolean,newScore:number,reason}`。expectedVersion 来自 attempt.version；听力 index 固定 -1。
- `revoke_score_override`：`{overrideId,expectedVersion,reason}`，版本来自 override.version。
- `set_attempt_counted`：`{attemptId,expectedVersion,counted:boolean,reason}`。不能把未交卷记录计分；恢复仍须满足基础数据有效性。
- `preview_rejudge({rule})`：rule=`{scope:paper|exam|slot,modality,paperId?,examId?,responseSlotId?,acceptedAnswers:string[]}`。只支持新增可接受答案变体，返回 `{n,affectedCount,affected,blocked,previewToken}`，不写入。
- `apply_rejudge`：`{rule,previewToken,reason}`。目标漂移或已有有效改判时拒绝；输出 runId 与 affectedCount。保留原 scorerVersion，另存重判规则与原判分版本。
- `save_writing_grade`：`{attemptId,writingSubmissionId,gradeId?,expectedVersion,taTr,cc,lr,gra,overallComment?}`。新 grade expectedVersion=0；草稿修改用其 version。grade_id 区分多版草稿，version 是该版的 CAS 修订号。四项 0–9、步长 .5；task_band 为内部等权平均计算值，不作为 IELTS Band 输出。
- `review_writing_grade` / `publish_writing_grade`：`{gradeId,expectedVersion,reason?}`。draft→reviewed→published；审核后不可改批注；新版本发布会将旧 published 退为 reviewed 并保留审计。
- `save_annotation`：`{gradeId,annotationId?,expectedVersion,startOffset,endOffset,quotedText,category,comment}`。offset 为 UTF-16，文本必须与原文一致；category 为语法/词汇/衔接/任务回应/其他；返回 gradeVersion。
- `delete_annotation`：`{gradeId,annotationId,expectedVersion}`，同样推进 gradeVersion。
- `merge_candidate_identity`：`{candidateId,candidateName,mergedInto,studentUuid?,expectedVersion,reason}`。新别名 expectedVersion=0；禁止循环；只保存别名，不改原始考生记录。
- `analytics_export_xlsx`：原 batchId/paperId 参数，新增 `operator?:string`。姓名可从本地设置取；输出 8 张表，包括原始成绩、改判后成绩、写作四维及听力。

发布新题先冻结确实嵌入 wrapper 的 payload，指针提交后写本地档案；本地档案写失败仅返回 archiveWarning，NAS 发布结果不倒退。NAS 历史脚本已经清理且本地无快照时，返回 CONTENT_UNAVAILABLE，不借用新版内容。
