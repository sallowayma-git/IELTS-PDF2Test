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
## V1.1 修订（2026-11-07 审计后，**覆盖上文冲突条款**）

目的：让 `current-paper.json` 成为考试路径**唯一的提交点**，消除"清单与指针两个文件不一致"的窗口；让每个批次的全部文件不可变。

1. **资源按批次归档、不可变**：作者端把每题资源写到 `releases/<batchId>/resources/<examId>/`（含 `asset-manifest.json`），
   不再写根级 `resources/<examId>/`，也不再原地替换任何已发布文件。manifest 条目的 `resourcesBase` = `./releases/<batchId>/resources/<examId>`，
   `assetManifest` 为该目录下的 `asset-manifest.json` 相对路径。资产清单内容必须确定性生成（不含 `generated_at` 等随时间变化的字段），
   相同内容重复发布字节相同。
2. **指针自包含**：`reading[i]` 与 `listening[i]` 增加 `entry` 字段 = 写进 manifest 的那条完整条目对象（含 `script`、`resourcesBase`、
   `assetManifest`、`checksums{scriptSha256,assetManifestSha256,runtimeSha256}`、`schemaVersion`、`dataKey`、`title`、`category`、
   `modality`、`minimumRuntimeVersion` 等，原样拷贝）。顶层的 `script`/`scriptSha256`/`runtimeSha256` 保留且必须等于 `entry` 中对应值。
3. **学生端考试路径只读指针**：锁卷与判分只用 `current-paper.json` 的 `entry`（经 `safeJoinNasRoot`、限定 `releases/<batchId>/` 前缀，
   校验 script 与 asset-manifest 的 sha256），**不再要求 examId 在 manifest 中存在、也不与 manifest 比对**。manifest 只服务练习/目录展示。
   `resourcesBase` 缺失或为根级 `resources/<examId>` 的旧条目按原路径兼容读取。
4. **提交顺序与失败语义**：release 目录就位 → manifest 原子替换 → 指针原子替换（考试路径的提交点）。指针写失败或在其前崩溃：
   旧指针仍引用旧批次（旧批次文件未被清理，清理只在指针成功后执行），学生端照常按旧卷考试；作者端顶层结果必须是"未完成"状态，
   文案"学生端仍是旧试卷"。清理永不删除当前指针（无论新旧）引用的批次。
6. **宽限期从"被撤下"起算**：批次不再被当前指针引用的那一刻（指针切换时）记录 `supersededAt`
   （`.publish-control/superseded/<batchId>.json`）；清理按 `now - supersededAt >= 24h` 判定。无记录的历史批次在首次被发现
   "不被当前指针引用"时补记 `supersededAt = now`，不立即删除。
7. **带 `entry` 的部件必须完整**：学生端对带 `entry` 的部件强制要求 `schemaVersion`、`checksums.assetManifestSha256`、
   位于 `releases/<batchId>/` 下的 `resourcesBase` 与 `assetManifest`；缺任一项即拒绝，不得回退到根级 `resources/`。
   回退仅适用于 `entry` 缺失的 V1 指针。
8. **运行时版本**：manifest/entry 的 `minimumRuntimeVersion` 与学生端"阅读运行时版本"常量比较（当前 0.2.0），不与安装包版本比较。
5. **单题发布取消**：生产命令 `publish_items` 必须带 `paper`；不带 → `PUBLISH_REQUIRES_PAPER`。编辑器不再有单题发布。

- 学生端锁定套题时把每篇的 `examId`、`script`、`scriptSha256`、`runtimeSha256` 和 `paperId` 记进 lockedSequence；
  交卷判分按锁定的 `script` 读取并校验 `scriptSha256`（脚本在 `releases/<batchId>/` 下不可变），不按当时的 manifest。

## V1.2 数据归档补充

见 [后端数据层 V2](2026-11-08-backend-data-v2-contract.md)：指针格式不变，作者端本地冻结发布时的运行时 JSON 并存试卷档案；听力运行时门禁与阅读共用运行时版本常量。
