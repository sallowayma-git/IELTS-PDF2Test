# 当前试卷（Current Paper）跨仓库契约 V1

作者端（PDF2Test，Tauri）写；学生端（IELTS-NASfor-WenDao，Electron）读。

## 位置

`<NAS 阅读根>/current-paper.json` —— 与 `manifest.js` 同一目录（作者端 `nas_reading_exams_dir(library_root)`，
学生端 `papersRoot/readingExamsRelative`）。

历史留档：`<library_root>/.publish-control/papers/<paperId>.json`（同内容，学生端不读）。

## 内容

```json
{
  "schemaVersion": "CurrentPaperV1",
  "paperId": "32位小写hex（uuid simple）",
  "publishedAt": "RFC3339",
  "batchId": "本次发布的 batchId（与 manifest._meta.batchId 一致）",
  "reading": [
    { "part": "P1", "examId": "...", "title": "...", "script": "./releases/<batchId>/<examId>.js",
      "scriptSha256": "64hex", "runtimeSha256": "64hex" },
    { "part": "P2", ... },
    { "part": "P3", ... }
  ],
  "listening": [
    { "part": "Part 1", "examId": "...", "title": "...", "script": "...", "scriptSha256": "...", "runtimeSha256": "..." }
  ]
}
```

规则：

- `reading` 必须恰好 3 条，`part` 依次为 `P1`、`P2`、`P3`，examId 互不相同。
- `listening` 可为空数组；非空时每条同上字段。
- `script`/`scriptSha256`/`runtimeSha256` 取自**同一次提交后**的 `manifest.js` 对应条目（`entry.script`、`entry.checksums.*`）。
- 作者端：在 `publish_items_core` 持有的 `export.lock` 内、`manifest.js` 原子替换**之后**，用临时文件 + `atomic_replace_file`
  写入。任一试卷题目发布失败或 `studentLoadable=false` → **整次不提交**：不替换 manifest、不写 current-paper.json，
  返回 `PAPER_INCOMPLETE`，NAS 上旧的当前试卷与清单保持原样（替换语义下部分提交会让旧指针失效）。
- 学生端：没有文件 / 解析失败 / 不满足上述规则 / examId 不在 manifest / 校验和与 manifest 条目不一致 → 报错
  `exam_paper_not_published`（HTTP 409，"未发布试卷，请联系监考老师"）。**禁止**回退到随机抽题。
- **发布即替换（V1 起）**：每次「发布为当前试卷」后，`manifest.js` 只包含本套试卷的题（reading 3 篇 + listening），
  不再累积旧题。旧题从清单撤下后，学生端新开考不可能再抽到它们。
- **撤下的旧文件保留宽限期**：`releases/<batchId>/` 与根级 `resources/<examId>/` 若不被当前试卷引用，且该批次发布时间早于
  24 小时，则在下一次发布时删除；24 小时内的保留，保证正在考试的学生（锁定了旧脚本）仍能交卷判分。
  批次发布时间以 `.publish-control/papers/*.json` 的 `publishedAt` + `batchId` 为准。
- 学生端锁定套题时把每篇的 `examId`、`script`、`scriptSha256`、`runtimeSha256` 和 `paperId` 记进 lockedSequence；
  交卷判分按锁定的 `script` 读取并校验 `scriptSha256`（脚本在 `releases/<batchId>/` 下不可变），不按当时的 manifest。
