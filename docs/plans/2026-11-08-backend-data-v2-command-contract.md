# 后端数据 V2 命令接口

本文件对应 `codex/backend-data-v2` 实现。前端页面留给后续任务；所有调用均可经真实 Tauri IPC 使用。错误为 `CODE: 中文消息` 字符串。设置的操作人姓名是审计身份，不是登录或权限认证。

## 参数与返回

Tauri 顶层参数使用 camelCase。写操作除设置外要求先调用 `save_review_settings({input:{operatorName:"教师姓名"}})`。返回的数据库事实字段保留 snake_case；统计视图沿用原有 camelCase。列表/统计用 `n` 表示对应范围数量，分页审计的 `n` 为过滤后总数。

| 命令 | 顶层参数 | 主要返回 |
| --- | --- | --- |
| list_papers | 无 | n、papers（档案状态、pointer）、recovery |
| get_paper | paperId | n、paper、items；缺档案明确错误 |
| get_question_context | examId、runtimeSha256、questionId（slot） | n、contentUnavailable、passage、taskGroups、questionText、options、explanation |
| get_attempt_detail | attemptId | n、candidate、paper、passages、answerEvents、activityComplete、writings、listening、listeningAnswers、overrides、anomalies、audit |
| list_overrides | attemptId | n、items（含撤销历史） |
| list_writing_queue | paperId?、status? | n、items；status 可为 ungraded/draft/reviewed/published/superseded |
| list_audit_log | filter:{targetId?,action?,limit?,offset?} | n、items；limit 1–500 |
| get_review_settings | 无 | operatorName |
| save_review_settings | input:{operatorName} | operatorName；设置变更本身也审计 |
| analytics_exam / analytics_student / analytics_question | 原有参数 + original? | 兼容既有视图；默认有效分，original=true 看原始口径 |
| analytics_listening_exam | batchId、paperId、original? | n、score、distribution、parts、kinds、questions、hardQuestions、matrix、students、disclaimer |
| analytics_export_xlsx | batchId、paperId | 保存对话框；取消为 null；成功含 n/path/sheets，8 张表 |

`get_attempt_detail` 每题包含原始/有效分、原始/有效判对、锁定内容与改答事件。缺失历史源返回 contentUnavailable，绝不借当前稿补题。活动区间只在同一 clock_id 且 monotonic 时间合法时计算；断档用 activityComplete=false 表达。原始作答改变后，旧改判不再生效，并在 anomalies 提示证据冲突。

## 改判与统计

写命令统一接收 `{input:{...}}`。

```json
{"attemptId":"attempt-id","modality":"reading","passageIndex":1,"responseSlotId":"q1","newIsCorrect":true,"newScore":1,"reason":"核对标准答案","expectedVersion":0}
```

以上用于 `create_score_override`；modality 默认 reading，listening 不传 passageIndex。满分来自该学生原始 slot，分数不得越界，判对须满分。返回 overrideId。`expectedVersion` 取详情 candidate.review_version；每次改判、撤销、统计选择或批量重判增加对应 attempt 的版本。

`revoke_score_override`：overrideId、expectedVersion、reason。`set_attempt_counted`：attemptId、counted、expectedVersion、reason。人工选择不能把未交卷/无有效原始分的记录变成有效成绩；不更改学生 status。原始数据不会被这些命令覆盖。

`preview_rejudge({rule})` 的 rule：scope=paper/exam/slot、modality?、paperId?、examId?、runtimeSha256?、responseSlotId?、acceptedAnswers（非空字符串数组）、reason。paper 范围必须有 paperId；exam/slot 必须指定 examId 与 runtimeSha256；slot 还须 slot ID。只支持明确的文本可接受答案扩展；不推测复杂多选的替代判分规则。

`apply_rejudge({input:{rule,previewToken}})` 必须使用预览 token。预览不写库；应用在事务内重验范围、原始证据和版本，新增 override/run/audit，不更新学生原始判分。预览后数据变化返回 REJUDGE_CONFLICT。

## 写作

`save_writing_grade` input：attemptId、writingSubmissionId、gradeId?、expectedVersion、taTr、cc、lr、gra、overallComment?、reason?。新版本 expectedVersion=0；修改必须指定已有 draft 的 gradeId 与 version。四维 0–9、步长 0.5，task_band 为四项精确均值，不取整。对外名称为“写作人工评分（0–9）”。

`review_writing_grade` / `publish_writing_grade`：gradeId、expectedVersion、reason?。状态顺序 draft→reviewed→published；新版发布后旧版标记 superseded，保留完整历史。同作文只能有一个 published。

`save_annotation`：gradeId、expectedVersion、annotationId?、startOffset、endOffset、quotedText、category、comment、reason?。偏移为 UTF-16 半开区间，与浏览器字符串一致；必须精确匹配原文，包括首尾空白。category 为语法/词汇/衔接/任务回应/其他。只能修改 draft，批注的 version 使用父 grade 的版本。

`delete_annotation`：gradeId、expectedVersion、annotationId、reason?；拒绝跨 grade 删除。成功修改批注后父 grade version 增加。

## 一致性和边界

教师写入与审计共用 analytics 事务，CAS 或审计失败时全部回滚。常见错误：VERSION_CONFLICT、OVERRIDE_CONFLICT、SLOT_NOT_FOUND、INVALID_SCORE、GRADE_TARGET_MISMATCH、GRADE_STATE_CONFLICT、INVALID_WRITING_SCORE、ANNOTATION_TARGET_MISMATCH、ANNOTATION_QUOTE_MISMATCH、OPERATOR_REQUIRED。

导出保留五张原始数据表，新增“改判后成绩”“写作成绩”“听力”。写作只导出 published 人工评分；听力明确原始与有效分。导出记录 started/completed/failed，保存取消不记录成功。各表带 N 和“机构模拟测试原始分，非 IELTS 官方成绩。”。

听力统计按 exam/runtime/slot 隔离。Part 来自学生锁定任务归属，旧数据无归属显示 unknown；分布按正确率十个百分点分箱，100% 包含在最后一箱。难题结论遵循原有最小 N 门槛。阅读/听力的独立成绩发布与冻结本轮未实现，依用户确认留待后续。
