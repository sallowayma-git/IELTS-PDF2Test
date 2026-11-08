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

### 1.1 套题归属（V1.1，审计后补）

- 学生端锁卷时在终端库 `exam_kv` 写 `paper_id:<attemptId>` = paperId（JSON 字符串；旧套题无 paperId 则不写）。
- 交卷时 `reading_suite_attempts.aggregate_json` 不再覆盖成 `{"passageCount":N}`，而是
  `{"schemaVersion":"SuiteAggregateV1","passageCount":N,"paperId":"... 或 null","lockedSequence":[锁定序列原样]}`。
- 作者端取 paperId 的优先级：`exam_kv paper_id:<attemptId>` → aggregate 对象的 `paperId` → 锁定序列数组元素的 `paperId`
  → 仅对 `meta.attemptId` 本身使用 `meta.paperId`。已存的非空 paperId 不得被空值覆盖；来源之间不一致记入导入异常。
- 同一 attemptId 出现在不同 terminalId 或不同 installId 的快照中 → 记入导入异常，不覆盖已导入记录。
- `dbSchemaVersion` 大于作者端支持的版本 → 记入导入异常（"快照版本过新，请升级作者端"），不导入。

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

## 3. V2 增量（2026-11-08）

新学生端输出 `schemaVersion: SubmissionSnapshotV2`、`dbSchemaVersion: 2`；作者端同时接受 V1/1（含历史缺省版本）与 V2/2，拒绝未知、畸形、不一致和过新版本。整库快照、只读源、ready 最后、pending 重试与身份冲突规则不变。

终端库连接初始化在事务内升级到 v2，版本存入 exam_kv.db_schema_version；导出器读取真实版本，不替旧库虚标版本。新增 reading_answers.response_slot_id/max_score/score（旧 slot=question_id、max_score=weight、score 按原始 is_correct 回填，未知为 null）；exam_attempts.session_id/student_uuid/abandon_reason 可空。

新增 answer_events，键 `(attempt_id,event_seq)`，含 client_event_id（attempt 内唯一、请求重试幂等）、event_type、passage_index、question_id、old/new_answer_json、wall_time、monotonic_ms、clock_id。event_seq 由服务端事务内分配；clock_id 区分 renderer 重启后的单调时钟。事件只经现有保存通道批量发送，保留内存队列；失败不使答案保存失败，不在本地持久化。只记录最终改答、Part/题目/应用焦点，不记录逐字按键或鼠标轨迹。

新增 listening_attempts 和 listening_answers：前者按 attempt 唯一，保存 exam_id/source_revision、源 hash、时间、汇总、播放快照；后者按 attempt+slot_id 唯一，保存逐题学生/标准答案、对错及权重。听力提交事实事务落库且幂等，最终交卷仍只有原 attempt 回执；快照包含新表。

作者端重导入仅刷新学生事实，不覆盖 counted/exclude_reason，不级联删除教师改判、写作批改、批注、重判及审计。教师操作另存，统计使用有效改判，可切原始分；原始学生行不被教师命令修改。

V2 管理员作废事务提交后也导出就绪快照，meta.submittedAt 可为 null，源 attempt.status=failed；用于保留作废原因与已有原始作答，不创建最终交卷回执，不计入有效计分。作者同时导入 submitted/failed 的已结束事实；已签到/进行中仍仅由信标统计。
### V2 听力统计补充

`listening_answers` 同时保存可空 `part_id`，取自锁定源的 `parts[].taskIds` 与 slot 所属 task；`question_kind` 取该 task 的 `taskType`。旧快照无 part 时归入“未知 Part”，不根据题号猜测。该字段为向后兼容扩展。
