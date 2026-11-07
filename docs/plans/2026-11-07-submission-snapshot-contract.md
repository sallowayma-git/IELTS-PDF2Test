# 作答快照与考试状态 跨仓库契约 V1

学生端（IELTS-NASfor-WenDao）写；作者端「数据」模块（PDF2Test）只读。

`<submissionsRoot>` = 学生端配置的考试作答存储目录（现有 `terminals/<terminalId>.sqlite` 所在根）。
作者端在「设置」里单独配置同一个目录（"作答数据目录"）。

## 1. 交卷快照

交卷事务（`ExamFinalSubmitService.submit`）**提交成功之后**，学生端把本终端整个 SQLite 导出一份一致性快照：

```
<submissionsRoot>/snapshots/<terminalId>/<attemptId>.sqlite        快照（VACUUM INTO 或 backup API 生成）
<submissionsRoot>/snapshots/<terminalId>/<attemptId>.meta.json     元数据
<submissionsRoot>/snapshots/<terminalId>/<attemptId>.ready         空文件，最后写；没有它的快照作者端一律忽略
```

写入顺序：先写 `.<name>.<rand>.partial` 临时文件 → 校验 → rename 为正式名；sqlite 与 meta 都就位后再创建 `.ready`。
同一 attempt 重新导出（补发重试）时整组覆盖（先删旧 `.ready`）。

`meta.json`：

```json
{
  "schemaVersion": "SubmissionSnapshotV1",
  "terminalId": "...",
  "installId": "... 或 null",
  "attemptId": "attempt_...",
  "batchId": "...",
  "paperId": "... 或 null（旧套题）",
  "candidateId": "...",
  "candidateName": "...",
  "submittedAt": "RFC3339",
  "exportedAt": "RFC3339",
  "appVersion": "... 或 null",
  "dbSchemaVersion": 1,
  "scorerVersion": "reading-scorer/1",
  "sizeBytes": 12345,
  "sha256": "64hex（快照 .sqlite 文件字节）"
}
```

- 快照是**整库**（含该终端的其他 attempt）。作者端按 `attempt_id` 去重合并，同一 attempt 以 `exportedAt` 最新的快照为准。
- 导出失败**不得**让交卷失败：在终端库 `exam_kv` 记 `snapshot_pending:<attemptId>`，下次启动、下次交卷时补导出，成功后删除该键。
- `dbSchemaVersion`：学生端在 `exam_kv` 存 `db_schema_version`；V1 = 当前表结构（exam_attempts / reading_suite_attempts /
  reading_passages / reading_answers / writing_submissions / submission_receipts / exam_kv）。
- `scorerVersion`：学生端判分逻辑版本常量，同时写进 `exam_kv` 的 `scorer_version:<attemptId>`。

## 2. 考试状态信标（用于"已签到 / 进行中"实时人数）

签到、锁卷开始、交卷、作废时，学生端覆盖写一个小 JSON（临时文件 + rename，失败只记日志不影响考试）：

```
<submissionsRoot>/status/<terminalId>/<attemptId>.json
```

```json
{
  "schemaVersion": "AttemptStatusV1",
  "attemptId": "...", "terminalId": "...", "batchId": "...", "paperId": "... 或 null",
  "candidateId": "...", "candidateName": "...",
  "status": "checked_in | in_progress | submitted | failed",
  "updatedAt": "RFC3339", "appVersion": "... 或 null"
}
```

作者端用信标统计 已签到 / 进行中 / 已交卷 / 作废；用快照统计 已导入 / 有效计分。两者分开显示，不互相推断。
