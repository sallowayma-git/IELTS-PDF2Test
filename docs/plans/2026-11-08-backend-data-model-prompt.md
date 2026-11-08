# 后端数据结构全量改造 —— 实施 Prompt（交给后端 agent）

> 本文件是给后端实现 agent 的完整任务书。你要做的是**后端数据层**：表结构、迁移、跨仓库契约、导入、查询与命令接口、测试。
> 前端页面不在本轮范围（只需保证每个页面需要的数据都有对应的后端命令且字段齐全）。

---

## 0. 角色、边界与工作方式

- 两个仓库：
  - 作者端（Tauri：Rust + React）`F:\workspace\PDF2Test`，远端 `sallowayma-git/IELTS-PDF2Test`，`main` 已含组卷发布 V1.1、导入/题库拆分、「数据」模块。
  - 学生端（Electron + Fastify TypeScript）`F:\workspace\IELTS-NASfor-WenDao`，远端 `sallowayma-git/IELTS-NASfor-WenDao`，`main` 已含当前试卷锁卷、终端握手、写回加固、交卷快照与状态信标。
- 先读两仓库各自的 `AGENTS.md`。作者端规则：产品是 Tauri 应用，CLI/schema 检查不能替代产品路径验证；报告要区分「端到端产品验证 / 命令处理层验证 / 仅 CLI 或 schema」。
- 现行契约（必须兼容，改动必须先改契约文档再改代码）：
  - `docs/plans/2026-11-07-current-paper-contract.md`（当前试卷 V1 + V1.1 修订 1~8 条）
  - `docs/plans/2026-11-07-submission-snapshot-contract.md`（交卷快照与状态信标 V1 + 1.1 节）
- 在各自仓库新建 worktree + 分支工作（例如作者端 `feat/backend-data-v2`、学生端 `feat/backend-data-v2`），**不要在 main 上直接改**，不要 push，按里程碑分组提交，提交信息结尾加 `Co-Authored-By: sallowayma-git <sallowayma@gmail.com>`。
- 学生端注意：worktree 的根 `node_modules` 若用目录链接共享，**绝不能**在根执行 `npm ci/install`；`npm run build:student-exam` 会对 `apps/student-exam` 执行 `npm ci`，该目录必须是 worktree 自己的真实目录。
- 不可破坏的产品原则（来自已确认的需求）：
  1. 题库不放机构服务器，全部经内网 NAS；学生机**不在本地留任何作答数据**（`persistAnswerDraft:false` 等策略不变）。
  2. 学生端每台机器只写自己的 `terminals/<terminalId>.sqlite`（DELETE 日志模式，进程内单连接）；作者端**只读** NAS 上的学生数据，只导入带 `.ready` 的快照，绝不打开学生正在写的库。
  3. 每次发布 = 替换「当前试卷」，全体学生同一套卷；考试中途重新发布不影响已锁卷的考生（按锁定批次判分）。
  4. 原始作答**永不修改**；判分分层：学生端自动判分（事实）→ 教师复核/改判（另存）→ 发布成绩（另存）。
  5. 只显示原始分与正确率，**不显示 IELTS Band**；所有统计带样本量 N；所有导出与页面注明「机构模拟测试原始分，非 IELTS 官方成绩」。
  6. 学生名册、班级权限、AI 评分本轮**不做**，但数据结构要为它们预留位置（字段/表可以先建，不接 UI）。

---

## 1. 现状盘点（已存在的数据，后端 agent 先逐项核对代码确认）

### 1.1 作者端题库（`src-tauri/src/library/schema.rs`，本地 SQLite）

| 表 | 用途 | 关键字段 |
|---|---|---|
| `library_items_v2` | 唯一权威稿 | id、modality(reading/listening/writing)、title、status(processing/action_required/ready/publishing/published/failed/archived/migration_required)、current_edit_version、canonical_ds_json、source_asset_id、created/updated/deleted_at |
| `processing_jobs_v2` | 识别任务 | 阶段、local/cloud/source 状态 |
| `actionable_issues_v1` | 待检查问题 | library_item_id、title、status(open/resolved/ignored) |
| `editor_journal_v1` | 编辑日志（CAS 与幂等） | |
| `publish_records_v2` | 每次发布一行 | library_item_id、edit_version、batch_id、destination、forced、verdict_json、reasons_json、student_loadable、status、created_at |
| `library_final_versions_v2` | 发布冻结的最终版证据 | edit_version、published_at、publish_record_id、evidence_json、source_purged_at |
| `recognition_*_v1`、`library_item_recovery_v1` | 识别闭环与恢复 | |

Part 标签（P1/P2/P3、Part 1–4、Task 1/2）由 `part_detection` 判定，前端行模型 `part`；题型来自 canonical 稿任务组。

### 1.2 发布到 NAS（作者端写，学生端读）

- `manifest.js`：只含当前试卷的条目（替换语义）。条目字段：examId、dataKey、script、title、category、frequency、schemaVersion、modality、assetManifest、resourcesBase、minimumRuntimeVersion、checksums{scriptSha256, assetManifestSha256, runtimeSha256}、publishOverride（可选）；`_meta`{schemaVersion, assetCount, generatedAt, batchId, forcedItems}。
- `current-paper.json`（考试路径唯一提交点）：schemaVersion、paperId、publishedAt、batchId、reading[3]{part, examId, title, script, scriptSha256, runtimeSha256, entry(完整条目)}、listening[]。
- `releases/<batchId>/`：脚本与 `resources/<examId>/`（含确定性 `asset-manifest.json`），不可变。
- `.publish-control/`：`export.lock`、`papers/<paperId>.json`（试卷历史）、`superseded/<batchId>.json`（撤下时间，24h 宽限清理依据）、`backups/`。

### 1.3 学生端终端库（`server/src/lib/exam/MachineSubmissionStore.ts`，NAS 上每终端一个 SQLite）

| 表 | 关键字段 |
|---|---|
| `exam_attempts` | attempt_id、batch_id、terminal_id、candidate_id、candidate_name、checkin_at、started_at、submitted_at、status(checked_in/in_progress/submitted/failed)、app_version、runtime_json |
| `reading_suite_attempts` | suite_attempt_id、attempt_id、status(active/completed/submitted)、started/completed/submitted_at、total_passages、aggregate_json（锁定序列；交卷后为 SuiteAggregateV1{passageCount, paperId, lockedSequence}） |
| `reading_passages` | suite_attempt_id、passage_index、asset_id、exam_id、title、category、started/ended_at、duration_sec、score_info_json、raw_submission_json（含答案、高亮、笔记、计时快照、滚动位置） |
| `reading_answers` | suite_attempt_id、passage_index、question_id、display_label、question_kind、control、source、match_mode、user/correct_answer_json、normalized_*、is_correct、weight、first/last_answered_at、change_count、visit_count、elapsed_ms、duration_ms |
| `writing_submissions` | writing_submission_id、attempt_id、task_type(task1/task2)、prompt_text、content、word_count、started/submitted_at、duration_sec、metadata_json |
| `submission_receipts` | receipt_id、attempt_id（UNIQUE）、sink、database_file_name、relative_path、written_at、payload_hash |
| `exam_kv` | `mid:<suite>:<part>`（阅读中途草稿）、`listening:<attemptId>`（听力进度与作答）、`paper_id:<attemptId>`、`db_schema_version`、`scorer_version:<attemptId>`、`snapshot_pending:<attemptId>` |

其他 NAS 产物：`snapshots/<terminalId>/<attemptId>.{sqlite,meta.json,ready}`、`status/<terminalId>/<attemptId>.json`、`terminals/<terminalId>.owner.json`；本机 `%ProgramData%\IELTS Practice\terminal-identity.json`。

**需要后端 agent 核实的已知疑点**：听力交卷（`ExamListeningService.submit`）计算出的分数似乎只返回给前端，没有落成表；逐题听力作答只在 `exam_kv listening:<attemptId>` 的 JSON 里。

### 1.4 作者端分析库（`src-tauri/src/analytics/mod.rs`，本地 `analytics.sqlite`）

`import_snapshots`、`attempts`、`suites`、`passages`（只有分数汇总 correct/total/percentage）、`answers`（逐题，与学生端同字段）、`writings`（**只有字数与时间，没有正文**）。

---

## 2. 需求：老师判卷 / 点开每个学生时要看到什么（逐项标注 现有 / 缺失）

### 2.1 仪表盘（批次与试卷层）

| 信息 | 状态 | 来源 / 需要做什么 |
|---|---|---|
| 试卷列表：paperId、发布时间、撤下时间、三篇标题与 examId、批次 | **缺**（只在 NAS 历史 JSON 里） | 作者端新增本地「试卷档案」表，发布成功时写入 |
| 已签到 / 进行中 / 已交卷 / 作废 人数 | 有（状态信标） | — |
| 已导入 / 有效计分 人数、导入异常队列 | 有 | — |
| 成绩分布、P1/P2/P3 正确率、题型正确率（分母=得分点 weight 之和）、难题 Top10（含 N）、学生×题目矩阵 | 有（阅读） | 听力缺，见 2.4 |
| 考场场次（同一套卷不同班/不同时间） | **缺** | 预留 `session_id` 字段（学生端 runtime 配置可选项），本轮只落库不做 UI |

### 2.2 学生详情页（判卷主视图）

| 信息 | 状态 | 需要做什么 |
|---|---|---|
| 考生：姓名、卡号、终端、installId、签到/开始/交卷时间、应用版本、状态 | 有 | — |
| 稳定学生身份 student_uuid | **缺** | 预留列（名册本轮不做）；同卡号不同名进「待核对」已有 |
| 每篇：标题、Part、用时、得分/满分 | 有 | — |
| 每篇**原文**（段落） | **缺** | 作者端按 (examId, scriptSha256) 从试卷档案取锁定版本的内容，见 3.2 |
| 每题：题号、题型、学生答案、标准答案、对错、权重、首次/最后作答时间、改答次数、访问次数、停留时长 | 有 | — |
| 每题**题干文本、选项文本、解析** | **缺** | 同上，作者端题目内容快照 |
| 计分单元 response_slot（多选/矩阵一题多个得分点） | **缺** | 学生端答案行增加 slot 粒度或明确 question_id 即 slot 的约定；作者端按 slot 统计 |
| 高亮、笔记、计时快照 | 学生端有（raw_submission_json），**作者端没导入** | 导入到分析库 |
| Part 实际活动区间（切回切走后的真实用时） | **缺** | 学生端新增活动事件表 |
| 改答历史（旧答案→新答案，用于净改答收益） | **缺** | 学生端新增答案事件表 |
| 写作：题目、**全文**、字数、用时 | 学生端有，作者端**没导入正文** | 导入正文 |
| 写作人工批改：四维评分（TA/TR、CC、LR、GRA）、总评、批注、复核、发布 | **缺** | 作者端新增批改表 |
| 听力：逐题作答、对错、得分、播放快照 | **缺**（见 1.3 疑点） | 学生端落表 + 快照 + 作者端导入 |
| 教师改判（某题判错/判对的人工修正）及理由 | **缺** | 作者端新增改判表，不改原始答案 |
| 判分版本与重判记录 | 部分（scorer_version） | 作者端新增重判记录 |
| 作废原因、是否计入统计 | **缺**（只有 status=failed） | 学生端作废时记录原因；作者端「计入统计」标记 |

### 2.3 学生端读取（考试运行时）

| 信息 | 状态 |
|---|---|
| 当前试卷指针、锁定批次 pin、按 pin 读题取图、按锁定脚本判分 | 有 |
| 阅读运行时版本门禁 | 有（0.2.0） |
| **听力运行时版本门禁** | **错**：仍用安装包版本比较，需改为统一的运行时版本常量 |
| 听力 examId 来源 | 部分：current-paper.listening 或环境变量，需统一为只认指针 |

### 2.4 状态与审计

| 信息 | 状态 |
|---|---|
| 考试状态信标 | 有 |
| 终端握手状态 | 有（管理面板） |
| 作者端操作审计（改判、批改发布、作废、重判、导出、身份合并：谁、何时、改了什么、理由） | **缺** |
| 操作人身份 | **缺**（无账号体系）：本轮用设置里的「操作人姓名」字符串记录 |

---

## 3. 要建立的数据结构（目标设计，实现前先在契约/设计文档里定稿）

### 3.1 学生端终端库 schema v2（`db_schema_version` 1 → 2，就地迁移，旧库可升级）

1. `reading_answers` 增加 `response_slot_id`（默认 = question_id）、`max_score`（= weight）、`score`（该 slot 得分，部分得分题用）。
2. 新表 `answer_events`：attempt_id、event_seq（单调递增）、event_type(answer_changed/part_enter/part_leave/question_focus/question_blur/app_blur/app_focus)、passage_index、question_id、old_answer_json、new_answer_json、wall_time、monotonic_ms。只记有教学/诊断用途的事件，不记鼠标轨迹与逐字键盘。写入走现有自动保存/保存通道批量提交，失败不影响作答。
3. 新表 `listening_attempts`（attempt_id、exam_id、source_revision、started/submitted_at、score_correct、score_total、playback_snapshot_json）与 `listening_answers`（attempt_id、slot_id、question_kind、user_answer_json、correct_answer_json、is_correct、weight）。听力交卷时在事务内写入（与阅读交卷同样的幂等与回执规则）。
4. `exam_attempts` 增加 `abandon_reason`、`session_id`（可空）、`student_uuid`（可空，预留）。
5. 快照契约升 **SubmissionSnapshotV2**：meta 增加 `dbSchemaVersion: 2`；作者端同时支持 V1 与 V2 快照。

### 3.2 作者端本地库

1. **试卷档案**：`papers`（paper_id PK、batch_id、published_at、superseded_at、destination、pointer_json、status current/superseded）与 `paper_items`（paper_id、part、exam_id、library_item_id、edit_version、title、script_sha256、runtime_sha256、asset_manifest_sha256）。发布提交成功（指针写入）时在同一流程写入；指针切换时更新 superseded_at。启动时可从 NAS 的 `.publish-control/papers/*.json` 补建（幂等）。
2. **题目内容快照**：`item_content_snapshots`（exam_id、runtime_sha256 PK、runtime_source_json，即发布时编译出的学生端运行时源，含原文段落、题干、选项、标准答案、解析）。发布时写入；学生详情页按学生锁定的 runtime_sha256 取，保证老师看到的是学生当时做的那一版。提供只读查询：按 (exam_id, runtime_sha256, question_id) 返回题干/选项/答案/解析、按 passage 返回原文。
3. **分析库扩展**（`analytics.sqlite`，带迁移）：
   - `passages` 增加 `highlights_json`、`note_text`、`timer_snapshot_json`、`runtime_sha256`、`script_sha256`。
   - `answers` 增加 `response_slot_id`、`max_score`、`score`。
   - `writings` 增加 `prompt_text`、`content`、`metadata_json`。
   - 新表 `answer_events`、`listening_attempts`、`listening_answers`（镜像学生端 v2）。
   - `attempts` 增加 `session_id`、`student_uuid`、`abandon_reason`、`counted`（是否计入统计，默认按有效计分规则）、`exclude_reason`。
4. **判卷与改判**（全部只增不改原始数据，带版本与审计）：
   - `writing_grades`：grade_id、attempt_id、writing_submission_id、task_type、grader、ta_tr、cc、lr、gra（0–9，允许 .5）、task_band（按四项等权计算，保留计算值）、overall_comment、status(draft/reviewed/published)、version、created/updated_at、reviewed_by、published_at。同一作文可有多版，`published` 只能有一版。
   - `writing_annotations`：annotation_id、grade_id、start_offset、end_offset、quoted_text、category（语法/词汇/衔接/任务回应/其他）、comment。
   - `score_overrides`：override_id、attempt_id、passage_index 或 listening、response_slot_id、original_is_correct、original_score、new_is_correct、new_score、reason、operator、created_at、revoked_at。统计查询默认使用「原始 + 有效改判」后的结果，并能切换看原始。
   - `rejudge_runs`：run_id、scope（paper/exam/slot）、reason、old_scorer_version、new_rule_json（例如新增可接受拼写变体）、affected_count、operator、created_at；重判结果写成批量 override，不改原始答案。
5. **审计**：`audit_log`（id、at、operator、action、target_type、target_id、before_json、after_json、reason）。改判、批改发布、作废/恢复计入、重判、导出、身份合并都必须记。
6. **身份核对**：`candidate_aliases`（candidate_id、candidate_name、student_uuid 可空、merged_into、operator、at）——名册本轮不做，先把人工合并结果存下来。

---

## 4. 需要提供的后端命令（Tauri）

保留现有 `analytics_*` 命令并按需扩字段；新增：

- 试卷：`list_papers`、`get_paper(paper_id)`（含三篇内容快照引用）。
- 学生详情：`get_attempt_detail(attempt_id)` 一次返回：考生信息、试卷、每篇（原文段落、用时、活动区间、高亮、笔记、得分）、每题（题干、选项、学生答案、标准答案、原始对错、改判后对错、改答历史、停留）、写作（题目、全文、字数、批改版本列表）、听力（逐题）、异常与审计摘要。
- 题目上下文：`get_question_context(exam_id, runtime_sha256, question_id)`。
- 改判：`create_score_override`、`revoke_score_override`、`list_overrides(attempt_id)`。
- 重判：`preview_rejudge(rule)`（只返回影响范围，不写）、`apply_rejudge(rule)`。
- 写作批改：`list_writing_queue(paper_id, status)`、`save_writing_grade`（draft，带版本 CAS）、`review_writing_grade`、`publish_writing_grade`、`save_annotation`/`delete_annotation`。
- 统计口径：`set_attempt_counted(attempt_id, counted, reason)`。
- 审计：`list_audit_log(filter)`。
- 导出：现有 Excel 增加「改判后成绩」「写作成绩」「听力」工作表，并标明哪些是原始、哪些是改判后。

所有命令：错误码 + 中文消息；写操作必须记审计；涉及版本的写操作做 CAS；查询返回 N。

---

## 5. 同时要修的遗留问题（与本轮数据改造直接相关）

1. 学生端听力运行时门禁改用统一运行时版本常量（与阅读同一处定义）。
2. 听力 examId 只认 `current-paper.json` 的 listening，不再读环境变量（保留环境变量仅供测试，且在考试模式下忽略）。
3. 作者端 10 个标注 DEPRECATED 的 e2e 脚本：本轮不迁移，但新增命令的测试不得依赖它们。

---

## 6. 实施顺序与验收

**里程碑 M1：契约与迁移**
- 先写/更新契约文档：快照 V2、学生端 schema v2、作者端新表的设计说明（放 `docs/plans/`，两仓库共用一份的放作者端并在学生端引用路径）。
- 学生端 schema v1→v2 迁移、作者端分析库迁移、新表建立。旧数据可升级、迁移幂等、重复启动不出错。
- 测试：v1 库升级后旧数据完整；v1 与 v2 快照都能被作者端导入。

**里程碑 M2：学生端采集**
- 答案事件、Part 活动区间、听力落表、作废原因、slot 粒度；快照 V2。
- 测试：服务层用例 + 真 Electron 流程（`py developer/tests/e2e/suite_practice_flow.py`）跑通并检查快照里新表有数据。

**里程碑 M3：作者端档案与导入**
- 试卷档案、题目内容快照（发布时写入，旧试卷可从 NAS 历史补建）、分析库导入新字段与新表。
- 测试：Rust 夹具必须使用**学生端真实代码生成的快照**（不得手写学生端从不产出的形态——上一轮正是这样出现了假阳性）。

**里程碑 M4：判卷与审计命令**
- 改判、重判、写作批改、计入统计、审计、`get_attempt_detail`、导出扩展。
- 测试：每个命令的成功/冲突/权限外路径；改判后统计变化正确、撤销后恢复；写作四维计算；审计必记。

**最终验收**
- `npx tsc --noEmit`、`npx vitest run`、`cargo test`（列出因私有语料跳过的用例）；学生端 `npm run build:server`、`node developer/tests/exam/run-all.cjs`、`apps/student-exam` 的 vitest、`node --test developer/tests/js/*.test.js`、`IELTS_PDF2TEST_REPO=<作者端 worktree> py developer/tests/ci/run_static_suite.py`。
- 产品级：扩展 `scripts/e2e/cross-repo-full-chain.mjs`：作者端发布 → 学生端真实考试（含听力与写作）→ 作者端导入 → 断言 `get_attempt_detail` 返回原文、题干、改答历史、写作全文与听力逐题；做一次改判与一次写作批改发布，断言统计与导出随之变化且审计有记录。workarounds 必须为空。

## 7. 报告要求（每个里程碑结束都要给）

中文；改动文件与命令；迁移说明；每条验证命令的真实结果；证据层级（产品端到端 / 命令处理层 / 仅单测或 schema）；未完成与不确定项如实列出；需要产品拍板的问题单独列出（不要自己替产品决定口径）。
