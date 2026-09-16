# 交接（前端 → 识别/云端后端）：当前工作树无法编译，产品链与全部 E2E 被挡住

> 时间：2026-09-16 11:07（本地）
> 面向：`src-tauri/src/auto_pipeline.rs`、`src-tauri/src/ielts_grammar/**`、`src-tauri/src/processing/**` 的写入方
> 来源：前端执行 agent（按契约 §1.1 对该面**只读**，未改动任何后端文件）

## 一句话

后端在 `auto_pipeline.rs` 新增的 DOCX 纯文本抽取（446 行未提交新增）**有 4 个编译错误**，
`ielts-author-studio` lib 编译失败 → **无法产出可验收二进制** → 完整链、问题列表校验、
候选按钮流程**全部无法运行**（harness 会因 `staleBuild` 判 CANNOT-RUN）。

任务书第 6 条（真实候选按钮流程 + PDF/DOCX 发布到学生端计分）因此**无法开始**。

## 证据（两次独立重试完全一致，不是偶发）

命令按「完整日志落盘 + 显式退出码」执行（上一轮的 `| tail` 缺陷已修）：

```text
npx vite build                                                     → VITE_EXIT=0
npx tauri build --debug --no-bundle --config '{"build":{"beforeBuildCommand":""}}'
                                                                   → TAURI_EXIT=1

error: could not compile `ielts-author-studio` (lib) due to 4 previous errors; 6 warnings emitted
```

| 错误 | 位置 | 相关代码 |
|---|---|---|
| `E0596` cannot borrow `entry` as mutable | `src/auto_pipeline.rs:564:9` | `entry.read_to_end(&mut bytes).ok()?;` |
| `E0716` temporary value dropped while borrowed | `src/auto_pipeline.rs:595:43` | `let local = local_name_of(event.name().as_ref());` |
| `E0716` 同上 | `src/auto_pipeline.rs:607:43` | 同上 |
| `E0716` 同上 | `src/auto_pipeline.rs:615:43` | 同上 |

日志：`artifacts/tauri-build.log`、`artifacts/tauri-build-retry.log`。

**归属**：`git diff --stat src-tauri/src/auto_pipeline.rs` = `446 insertions(+), 0 deletions(-)`，
全部为**未提交新增**（`extract_docx_plain_text` / `docx_part_plain_text`）。不是前端改动。

## 最小修复（供参考，我按所有权约定**未**改动）

1. `auto_pipeline.rs:552`：`let entry = archive.by_index(index).ok()?;`
   → `let mut entry = archive.by_index(index).ok()?;`（编译器已给出该 help）。
2. `:595` / `:607` / `:615`：把临时值绑到 `let`，例如
   ```rust
   let name = event.name();
   let local = local_name_of(name.as_ref());
   ```

## 另请注意：工作树在我验证期间被并发修改

用 `find -printf '%T@ %s %p'` 在每次构建前后取指纹对比，两次构建期间各有后端写入：

| 构建 | 期间被改的文件 | 大小变化 |
|---|---|---|
| 第 1 次 | `processing/scheduler.rs` | 41873 → 42620 |
| 第 2 次 | `ielts_grammar/instruction_signature.rs` | 20018 → 20047 |
| 第 2 次 | `ielts_grammar/quality.rs` | 162185 → 164972 |

结论：**当前任何构建/E2E 结果都无法归因到某个确定的后端版本**。
请在收工（或宣布交付）时留一个稳定点（源码 mtime 不再变化）；我会在该点重建并跑完整链。

（看到 `quality.rs` / `instruction_signature.rs` 在被改，推测正在处理 R9 的
`WORD_LIMIT_UNPARSED` 误判 —— 那正是 R9 交接单里的第 2 条。方向正确，只是此刻树不编译。）

## 我这边已就绪（编译一通过即可跑，不需要再写代码）

- `npm run e2e:issue-list` —— 问题列表真实界面校验（PDF + `complex-reading.pdf` 负控）；
- `node scripts/e2e/tauri-cdp-recognition-buttons.mjs` —— 6 场景真实按钮流程；
- `node scripts/e2e/tauri-cdp-product-chain.mjs` —— PDF/DOCX 完整链到发布；
- 前端独立验证（与 Rust 构建无关，**已通过**）：`tsc --noEmit` 干净；
  `vitest run` → **186 passed / 12 files**。

## 未完成声明

本轮**没有任何**完整链通过、没有发布产物、没有进入学生端。
构建失败使全部真实链路验证不可执行；不以此前的历史结果替代。
