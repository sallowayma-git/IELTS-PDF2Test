# 2026-09-24 Wave3 · 质量方复核收口（音频丢行根因 + 门禁收窄核对）

分支 `wave3-listening`。本文回应质量方本轮复核的 1)–4)，收尾提交后工作区干净、未 push（任务书约定）。

## 1) 「4 段音频只落库 3 行」：根因与先红后修证据

根因链在 `70a32a1` 已定位并修复，本轮补齐了评审指定的**命令处理器层**可复现反例：

- **确切位置（按丢的是什么）**：
  - 丢的是「稿里那个 part 的 media」：`sync_item_audio_media` 旧实现**先读台账、后读稿**，
    台账快照过期时把「快照里没有的 part」补 `media: null` 删掉；镜像输掉
    `EDIT_VERSION_CONFLICT` 后错误被调用方丢掉（当时无重试）。
    这正是两次 E2E 各缺一段（part-4 / part-2）的机制。
  - 丢的是「台账行」：`bind_audio` 旧用 deferred 事务，WAL 下「先 SELECT 后 UPSERT」
    撞 `SQLITE_BUSY_SNAPSHOT`（不受 busy_timeout 保护），该段 INSERT 失败。
  - 错误被吞的两处：前端导入循环吞掉 bind 错误；播种后的对账 `let _ =` 吞掉。
- **命令处理器层反例（本轮新增）**：
  `src-tauri/src/listening_audio/canonical_media.rs` ::
  `the_bind_command_path_never_loses_a_row_while_recognition_writes_run`。
  导入听力卷（壳 + shadow 候选稿）→ 绑定线程逐字跑 `bind_listening_audio` 的写路径
  （`bind_audio_command_path`：bind_audio → 镜像 → 读回确认，从
  `commands.rs` 抽取，命令本身现在也调用它）→ 识别线程并发做
  `ensure_initial_canonical` + 3 次条目状态写（调度器同构）→ 重复 30 次，
  每次统计 `listening_audio_assets_v1` 行数、稿内 media 数、绑定失败清单。
  含反空转断言（播种与绑定必须真的交错过）。
- **先红（对照实验，本轮实测）**：把 `70a32a1` 的三处修复临时回退（单次尝试、先台账后稿、
  deferred 事务）后，该测试 30 次内命中：
  `#13 稿里只镜像了 [part-1, part-2, part-3]（台账 4 行）`——绑定日志里 part-4 还报过
  `ok updated=[part-4]`，随后被并发写入抹掉，与真实缺陷特征一致。恢复修复后回到绿
  （listening_audio 模块 37/37）。
- **界面如实报错**：Rust 侧读回确认（`ensure_part_media_matches`，镜像没落盘时命令报
  `LISTENING_AUDIO_MEDIA_NOT_MIRRORED:{part}`，不返回 Ok）；前端抽取为纯函数
  `bindListeningAssignments`（`listeningAudioPlan.ts`）并补 3 条单测：失败段落进 rejected
  清单、不中断后续段、每段失败各自有条目；`LibraryPage` 在抽屉卸载后仍渲染
  `library-import-rejected`（role=alert）。

## 2) 门禁收窄（`SIGNIFICANT_REGION_UNASSIGNED`）逐条核对

`70a32a1` 已实现，本轮逐条对过评审要求，全部在 `quality.rs` 带测试：

| 评审要求 | 实现 | 测试 |
|---|---|---|
| 封面三前提（无题号声明 + 无锚点 + 须知词表）才 `exam_front_matter` | `physical_ignored_reasons` 4091–4118；词表含 Candidate Number / INSTRUCTIONS·INFORMATION FOR CANDIDATES / IELTS 全称等 | `a_candidate_notice_cover_is_explained_as_exam_front_matter` |
| 封面出现 `Questions 1-5` 不得忽略 | `declares_question_numbers` 判「声明」而非出现 question 一词 | `a_cover_that_declares_question_numbers_is_never_ignored_as_front_matter` |
| 页上有任何锚点即非封面 | `anchored_page_indices` 闸 | `a_page_that_carries_a_source_anchor_is_not_treated_as_a_cover` |
| `【VOL7-T9】` → `paper_label` | `is_volume_label`（整块仅一对书名号短标签） | `a_volume_label_on_a_content_page_is_explained_as_a_paper_label` |
| `Read the text and answer questions 1-10` 不忽略、挂到 Part | `listening_draft.rs::part_source_anchors` 把指令行挂进 Part 的 sourceAnchors | `a_section_instruction_is_never_ignored_by_these_rules` |
| 阅读卷封面同规则 | 同一套按页判定，与科目无关 | `the_same_cover_rule_applies_to_a_reading_paper` |

## 3) 阅读 golden 与复跑交接

- golden 逐字节不变（git 证据）：`git log 8f378d7..HEAD -- fixtures/golden/` 为空，
  本轮全部提交未触碰 golden fixtures；Rust 全量含 golden 对照测试全绿。
- 本会话 `spawnSync` 全程不可用（环境约束，见 35c9afe），**阅读链未在本会话复跑**，
  由质量方在默认产品档执行。当前磁盘上的 exe 是 `70a32a1` 之前构建的（已过时），
  复跑前需重建 App。
- 全量数字（本轮收口时）：cargo lib **1074 passed / 0 failed / 11 ignored**；
  vitest **411 passed**（28 文件）；`tsc --noEmit` 干净。
- 质量方复跑清单：① 阅读链（默认产品档，须 13/13）；② 听力链（默认产品档，须全绿、
  第 10 步干净 `published`）。第 10 步预期：门禁对封面/卷标/分节说明按上表处置后，
  `SIGNIFICANT_REGION_UNASSIGNED` 不再 blocking。

## 4) push 状态

按任务书约定本轮**未 push**；`70a32a1` 与收口提交都在本地 `wave3-listening` 上，
此前已推送的那次由用户决定是否处理。
