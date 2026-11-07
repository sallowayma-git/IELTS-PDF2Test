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
  写入。任一试卷题目发布失败或 `studentLoadable=false` → **不写** current-paper.json，并把整次结果报告为 `PAPER_INCOMPLETE`
  （manifest 里已成功的题照常保留，学生端继续用旧的当前试卷）。
- 学生端：没有文件 / 解析失败 / 不满足上述规则 / examId 不在 manifest / 校验和与 manifest 条目不一致 → 报错
  `exam_paper_not_published`（HTTP 409，"未发布试卷，请联系监考老师"）。**禁止**回退到随机抽题。
- 学生端锁定套题时把每篇的 `examId`、`script`、`scriptSha256`、`runtimeSha256` 和 `paperId` 记进 lockedSequence；
  交卷判分按锁定的 `script` 读取并校验 `scriptSha256`（脚本在 `releases/<batchId>/` 下不可变），不按当时的 manifest。
