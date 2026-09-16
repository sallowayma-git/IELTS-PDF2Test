# PDF2Test 项目长期记忆

## 工程约定
- **产品是 Tauri 应用**，不是 CLI（见 `AGENTS.md`）。CLI/脚本只作夹具与诊断，不得用 CLI 通过代替产品链路验证。
- 测试基线口径：`cd src-tauri && cargo test --lib --no-fail-fast`。
  历史：671（09-15 交接）→ 684（四代理落盘）→ **687（09-16 主线程补测）**，`11 ignored` 长期不变。
- 用户指定协作边界：我侧只做 `src-tauri/` 后端；`src/` 前端需另派。跨文件并发代理必须文件集互斥。

## 本机环境坑（Windows / 本仓库）
- **Bash 工具不可用**（`dirname`/`cd`/`ls` 均报 command not found），一律改用 PowerShell。
- PowerShell 对 `cargo`/`git` **常返回空 stdout**：必须 `2>&1 | Out-File -FilePath <abs> -Encoding utf8`，再用 Read 读取。
- 不要用 PowerShell `ConvertFrom-Json` 校验 JSON（UTF-8 乱码 + 假阳性语法错误）；用 `python -c` 或 Node `JSON.parse`。
- PowerShell 下 `$LASTEXITCODE` 在管道给 `Select-Object` 后会被重置，**不要据它判断成败**。
- 本环境的文件删除走回收站包装器，**可能报 `SAFE_DELETE_FAIL_CLOSED` 而实际已删除**；以删除后的目录列举为准，不要只看报错。

## 方法论教训
- **"文件 mtime 未变" ≠ "功能未实现"**。曾据此误判"撤销状态持久化未落地"，实际机制（`reconcile/store.rs:302` `set_decision_status`）早已存在，缺的只是测试。
- 审计代理交付时，**必须区分**：产品改动未落地 / 已落地但零测试 / 测试写错。三者修法完全不同。
- 测试若"**复刻**被测函数的行为"而非调用它（如 scheduler 里旧版快照测试），**永远测不到该函数内部的错误处理**。补测要调用真函数。

## git 仓库损坏恢复（本仓库 2026-09-16 实战，可复用）
症状：`.git` 目录存在（HEAD/config/objects/index 都在），但 `git status` 报
`fatal: not a git repository`。逐层根因与修法：

1. **`.git/refs` 目录整体缺失** → git 校验仓库需 HEAD/objects/refs 三者齐备，缺 refs 直接判定"不是仓库"。
   修：`refs`、`refs/heads`、`refs/tags` 建空目录。此步后 `rev-parse --git-dir` 即可用。
2. **孤儿 pack 索引**：`objects/pack/` 下存在只有 `.idx` 没有 `.pack` 的文件（中断的 fetch 残骸）。
   git 会以为对象存在却读不到，表现为 `fatal: unable to read <sha>`。
   修：删除所有无同名 `.pack` 的 `.idx`，以及 `multi-pack-index` 缓存。
3. **对象库大面积缺失**（本次 476 个 blob）→ 本地无历史可恢复，reflog 里的提交对象也已不存在。
   修：`git fetch --refetch origin`（强制重下全部对象，填充缺口）。实测一次成功：
   2275 objects / 6 MiB，之后 `git fsck --connectivity-only` 的 `missing` 归零。
4. **远端跟踪引用写不进去**：`git update-ref refs/remotes/origin/main <sha>` 返回 0 但文件不生成。
   根因是**嵌套目录不存在时 git 不会自动创建**。修：先 `New-Item refs/remotes/origin`，再写引用文件。
5. **旧索引不可修复**：若旧 `index` 引用了本地未推送、已成永久丢失的对象，`git reset`/`git diff` 必报缺对象。
   修：移走旧 `index`，用 `git read-tree HEAD` 从 HEAD 重建。**旧暂存状态会丢失**，需重新 `git add`。

健康判据（本仓库）：`git rev-parse HEAD` 可用、`git log` 可读、`show-ref` 同时列出
`refs/heads/main` 与 `refs/remotes/origin/main`、`fsck --connectivity-only` missing=0、
`git status -sb` 首行不含 `[gone]`。
