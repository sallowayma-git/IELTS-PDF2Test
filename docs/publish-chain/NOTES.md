# 发布链路收敛 — 本轮记录

- worktree：`F:\workspace\PDF2Test-publish`，分支 `feat-publish-chain`，基线 HEAD `fa18598`
- `CARGO_TARGET_DIR`：`F:/workspace/.cargo-target-publish`
- 隔离方式：分支名用 `feat-publish-chain`（**无斜杠**）。原因：本环境 `.git/refs/heads/` 下
  **嵌套目录无法持久** —— `git worktree add -b feat/publish-chain` 报 `fatal: invalid reference`；
  实测无斜杠分支可创建、直接写 ref 文件可行，但 `refs/heads/feat/` 目录会被回滚，
  导致分支成 unborn、worktree HEAD 无法解析（568 个文件全部显示未跟踪）。

标注约定：**[亲验]** = 本轮我直接读码/执行确认；**[转报]** = 由调研执行体给出、尚未亲验。

---

## 一、`5588412` 核对结论（不重做已完成部分）

提交：`feat(reconcile): 识别阻塞逐目标重判 + 发布判据三处统一`

改动面：`authoring_v2_commands.rs +235`、`cloud_repair/mod.rs +12`、`cloud_repair/tools.rs +10`、
`ielts_grammar/quality.rs +570`、`library/commands.rs +31`；自报 `809 passed; 0 failed; 11 ignored`。**[亲验]**

**它统一的是「未处理 blocking issue」这个谓词**，此前在**三处内联各写一份**：

| 三处 | 现状 |
|---|---|
| 预检 | 改为调用共享函数 |
| 导出 | 改为调用共享函数 |
| 修复终检 | `cloud_repair::remaining_tasks` 不再自己实现一份 |

落点（**修正我最初的推断**）：谓词在 `authoring_v2_commands.rs:432-440 blocking_issue_unresolved`
与 `:443-453 unresolved_blocking_issues`，**不在** `ielts_grammar/quality.rs`。
谓词本体：`severity == "blocking" && issue.details.resolution ∉ {resolved, ignored}`。

**它没有统一什么（关键）**：该提交 diff **不含 `runtime_validation.rs`，也不含 `export_pack.rs`**。

> 即：`5588412` 统一的是一个**谓词**，不是**那道门禁**。
> `runtime_validation.rs:360 publish_readiness_gate` 与
> `export_pack.rs:58 ExportValidationOptions::should_block` 均未被它触碰。

---

## 二、门禁判定点清单（当前事实）

### 2.1 遗留导出族：可被 policy 入参绕过

`export_pack.rs:20-95` **[亲验]**：

- `enum ValidationPolicy { Strict, Force }`（:21）
- `pub(crate) struct ExportValidationOptions { policy: ValidationPolicy }`（:26）—— **唯一字段就是 policy**
- `from_input`（:37）读 `input.get("validationPolicy")`
- `from_policy`（:41-49）：默认 `"strict"`，**接受 `"force"`**，其它值报 `invalid_validation_policy`
- `should_block`（:58）= `policy == Strict && !validation_report_passed(report)` ⇒ **Force 下永不阻塞**
- `validation_overridden`（:62）/ `ignored_issues`（:66-94）仅在 Force 且未通过时有值
- `persist_export_validation_report`（:104-113）确实**只写原始 report** 到 `validation-report.json`。
  **[更正 2026-09-19]** 但据此推出「绕过不留痕」是**错的**：`export_summary`
  （`:168-179` 单卷 / `:288-300` 批量）含 `validationOverridden` / `ignoredIssueCount` /
  `ignoredIssues`，并作为实参传给 `cleanup_transient_job_artifacts`（`:184`），后者经
  `write_authoring_project` 落到 **`authoring-project.json`**（`cleanup.rs:69,72`），
  开诊断时还会落到 `cleanup-summary.json`（`cleanup.rs:114,117`）。**留痕是有的，在 summary 里**。
- 真正的问题有两处（**[更正 2026-09-19]**）：
  1. `ignored_issues`（`:75-81`）只收 `severity == "error"` 的项 ⇒ **被 force 放行的 warning
     完全不留记录**；
  2. 批量导出 `:251` 的 `validation_overridden |= ...` 是**批级或运算** ⇒ 只要有一卷被 override，
     整批 summary 都被标记成 overridden（`ignoredIssues` 是逐卷 append 的，两者粒度不一致）。

构造点与可达性：**[亲验 `lib.rs` 与 `export_pack.rs`；`export_nas_library.rs` 为转报]**

| 位置 | policy 来源 | 可达性 |
|---|---|---|
| `export_pack.rs:126 ExportValidationOptions::strict()` | strict | 仅测试 |
| `export_pack.rs:202 from_input` ← cmd `export_reading_js`(`lib.rs:1419-1423`) | 调用方 `input.validationPolicy` | **产品路径** |
| `export_nas_library.rs:1451 from_input` ← cmd(`lib.rs:1425-1429`) | 调用方 `input.validationPolicy` | **产品路径** |
| `lib.rs:1415 from_policy` ← cmd `export_reading_assets`(`lib.rs:1407-1417`) | **前端传参 `validation_policy`** | **产品路径** |

`lib.rs:1407-1417` 原文 **[亲验]**：

```rust
#[tauri::command]
async fn export_reading_assets(
    job_id: String,
    export_dir: String,
    validation_policy: Option<String>,
    app: AppHandle,
) -> CommandResult<Value> {
    let root = app_root(&app)?;
    let options = ExportValidationOptions::from_policy(validation_policy.as_deref())?;
    export_reading_assets_with_options_core(&root, &job_id, &export_dir, true, options)
}
```

前端侧 **[亲验]**：

- `src/api/tauriCommands.ts:292` 把 `"force"` 作为**一等参数**暴露：
  `exportReadingAssets(jobId, exportDir = "local://exports", validationPolicy: "strict" | "force" = "strict")`
- `src/types/reading-source.ts:100` `export type ValidationPolicy = "strict" | "force"`
- 导出输入/结果类型均带 `validationPolicy?`（`reading-source.ts:69/88/105/112/126`）
- `src/services/devFallbackBackend.ts:2394` `normalizeValidationPolicy` 把 `"force"` 透传

**但在 `src/` 内未找到任何 UI 调用点实际传 `"force"`，也未找到调用 `exportReadingAssets` 的 UI 代码。**

⇒ 结论：绕过门禁的**能力**存在于 IPC（产品）面、可被 webview 调用；当前 UI 未实际使用它。
按任务书口径（「产品路径能绕过门禁就是必须修的缺陷，不是可配置项」）仍属必修。

#### 更正 2026-09-19：force 的可达范围（**我的原表述需要收窄，但收窄点不在 `export_reading_assets_core`**）

收到的更正意见是「force 只对 `export_reading_js` 成立，因为
`export_reading_assets_core:115-128` 硬编码 `strict()`」。**前半句经我再核对不成立**：

- `export_reading_assets_core`（`:115-128`）**确实**硬编码 `ExportValidationOptions::strict()`
  —— 这点更正意见是对的，它是个 4 参包装，**没有** force 入参。
- **但命令 `export_reading_assets` 调的不是它**。`lib.rs:1407-1417` 原文 **[亲验]**：

  ```rust
  #[tauri::command]
  async fn export_reading_assets(
      job_id: String,
      export_dir: String,
      validation_policy: Option<String>,
      app: AppHandle,
  ) -> CommandResult<Value> {
      let root = app_root(&app)?;
      let options = ExportValidationOptions::from_policy(validation_policy.as_deref())?;
      export_reading_assets_with_options_core(&root, &job_id, &export_dir, true, options)   // ← 不是 _core
  }
  ```

  `validation_policy` 经 `from_policy`（`export_pack.rs:41-49`）**接受 `"force"`** ⇒
  **该命令上 force 可达**。`export_reading_assets_core` 是给测试与内部调用用的 strict 包装
  （调用点：`lib.rs:5287/7573/7672/7861`，全部在 `#[cfg(test)]` 之外仍是 strict 语义）。

⇒ **在 HEAD `fa18598` 上，产品 IPC 面共有 3 个 force 入口，不是 1 个**：

| 命令 | 位置 | force 来源 |
|---|---|---|
| `export_reading_assets` | `lib.rs:1407-1417` → `from_policy(validation_policy)` | 命令形参 `validation_policy` |
| `export_reading_js` | `lib.rs:1419-1423` → `export_pack.rs:202 from_input(input)` | `input.validationPolicy` |
| `export_nas_library` | `lib.rs:1425-1429` → `export_nas_library.rs:1451 from_input(input)` | `input.validationPolicy` |

（三个命令都在 `lib.rs:1726` 附近的 `invoke_handler` 注册表里，均可被 webview 调用。）

**范围问题在本轮已消解**：实现顺序第 4 项把 force 能力整体删除（后端入口全关），
所以"几个入口"不再影响结论。但记录必须准确，故保留此更正。

### 2.2 V2 发布族：硬拦、无 force 通道 **[转报]**

- 命令：`export_authoring_v2`(`lib.rs:1255`)、`publish_items`(`lib.rs:1090`)、
  `publish_nas_package_v2`(`lib.rs:1435`)
- 判定在 `authoring_v2_commands.rs:737-757`：`quality_state != "ready"` → Err；
  db_direct → `check_publish_preflight` → `!passed` → Err；否则 `validate_authoring_v2_publish_readiness`(:226-383)

### 2.3 仍然并存的判据实现（至少四处）**[转报 + 亲验摘录]**

1. `authoring_v2_commands.rs:285-306` 类型化就绪度
2. `ielts_grammar/quality.rs:223-233` 决定 `QualityReportV2` state（ready/review_required/blocked）
3. `authoring_v2_commands.rs:432-453` 共享谓词（预检 + cloud-repair 使用）
4. `export_pack.rs:97-102 validation_report_passed` = `report.passed`，
   而 `report.passed` 由 `authoring_validation.rs:196-213 merge_validation_issues` 置为
   `!has_error_issues(issues)`

⇒ 「能不能发布」在代码里**至少有 4 个独立来源**。

### 2.4 env 变量 **[转报，待亲验]**

| 变量 | 默认 | 作用 |
|---|---|---|
| `EPIC8_RUNTIME_GATE_STRICT` | **true** | 关闭时**只追加 warning**（`runtime_validation.rs:296-308`）⇒ 今天**不削弱任何阻塞**，形同装饰 |
| `EPIC8_NODE_VALIDATOR_DIAGNOSTICS` | **false** | 侧车诊断开关；关闭时函数直接 `return`，**连"未启用"这件事本身都不落痕**（§3.3） |

---

## 三、校验器不可用 ≠ 通过（本轮最高优先级）

### 3.1 侧车当前能不能起来：**能** **[转报，含真实探针]**

- `node --version` → `v22.22.2`
- 两个脚本均存在；import **全部是 Node 内置模块**（`node:fs` / `node:os` / `node:path` /
  `node:vm` / `node:child_process`）。根目录 `node_modules` **不存在**，但**不影响运行**
  ⇒ 侧车在 npm 离线下**可正常执行**，不存在缺依赖导致的离线失败。
- 真实探针（临时目录输入，未写入两个 worktree）：
  - 最小输入 `{}` → **exit 1**，stdout 为合法 JSON（`passed:false`，5 个 error），stderr 为空
  - 合法最小 source → exit 0、`passed:true`、stderr 空
  - 脚本末行 `process.exit(report.passed ? 0 : 1)`

### 3.2 守卫真值表（`runtime_validation.rs:209` 与 `:246` 同构）

```rust
if !output.status.success() && parsed.get("passed").and_then(Value::as_bool) != Some(false) {
    return Err(command_failure("node-validator", &output));
}
```

| exit | `passed` | 结果 | 备注 |
|---|---|---|---|
| 0 | true | `Ok` | 正常通过 |
| 0 | false | `Ok` | 校验失败以 `Ok` 返回（下游按 issues 重算） |
| 0 | 缺失/非 bool | `Ok` | **异常格**：畸形输出可被当作有效报告 |
| ≠0 | true | `Err` | 进程失败与结论矛盾 → 报错 |
| ≠0 | false | `Ok` | **异常格**：进程失败却返回 `Ok` |
| ≠0 | 缺失/非 bool | `Err` | 无法解析 → 报错 |

下游 `merge_sidecar_validation`（`authoring_validation.rs:135-194`）**不信** sidecar 自报的 `passed`，
而是用合并后的 issues 重算：`:184-190` `insert("passed", json!(!has_error_issues(&merged_issues)))`。

⇒ 失败结论得以传播，但「**是否执行成功**」与「**校验是否通过**」被压进同一个 `passed` 布尔。
**全仓不存在区分三态（通过 / 失败 / 未能执行）的类型** —— 这是需要新增的东西。

### 3.3 现状结论 **[更正 2026-09-19]**

- `run_node_validator_diagnostic`（`runtime_validation.rs:328-358`）仅在
  `node_validator_diagnostics_enabled()` 为真时执行；默认 false ⇒ 直接 `return`，
  **这个开关关闭时不产生任何记录**（这一点仍然成立）。
- **更正**：`Err` 分支**不是零记录**。`runtime_validation.rs:344-356` 明确
  `merge_validation_issues(...)` 写了一条 issue，但 `"severity": "warning"`（`:349`）。
  而 `has_error_issues`（`validator.rs:440-442`）经 `is_error_issue`（`:432-438`）
  **只认 `"error"`** ⇒ 该 issue 计入 layer 的 `warningCount`，**永不阻塞** `passed`。
  ⇒ 比"零记录"更隐蔽：**产物里有一条"校验器不可用"的记录，却对结论毫无影响**，
  读者会以为已被处理。
- `Err` 时只注入 warning，不翻转 `passed`（同上一句，这是同一个事实的两种说法）。
- 侧车**只**被两处调用：`runtime_validation.rs:337`、`preview_commands.rs:86`，**均为信息性**。
- 发布/导出路径（`export_pack.rs:139,248`、`export_nas_library.rs:1109`、
  `auto_pipeline.rs:2756,3202`）**完全不调侧车**。

**重要澄清（修正任务书的初始假设）**：真正的强制门禁
`validate_reading_source_contract`（`validator.rs:205`，由 `validate_authoring` 调用，
**纯 Rust 内建**）**始终执行**。被跳过的只是补充 / 对等诊断，而非全部校验。

⇒ 「校验器不可用被当成通过」**不成立于主门禁**（纯 Rust 契约校验始终执行）；
但**成立于诊断链路**：诊断失败只留一条不阻塞的 warning（§3.3），
**在结论层面「没能执行」与「执行了且通过」不可区分**（`passed: bool` 两态）。
⇒ 不设第三态的后果：失败被降格为 warning 后，**任何只看 `passed` 的调用方都无法感知它**。

### 3.4 可复现的「非零退出却显示 ok」形态 **[转报]**

`run_preview_e2e_core` 把可能 `passed:false` 的 `diagnostic_report` 写盘为 `validation-report.json`，
却用 `static_report` 置 `JobStatus::ExportReady`（`preview_commands.rs:138-142`；
`lib.rs:5390` 有测试刻意锁定该行为）。

⇒ **落盘的报告与 job 状态来自两份不同 report** —— 属「同稿不同判」类缺陷，正是本轮要收敛的对象。

### 3.5 无「按词判定」 **[转报]**

grep `contains("passed"/"ok"/"success")`、`status.success()`、`status.code()` 全 `src-tauri`：
仅 `environment.rs` preflight 与 `parser.rs` 用退出码；两处侧车守卫用解析后的布尔。
**没有任何代码拿 stdout 里出现的词判通过。**

---

## 四、听力卷发布路径

**结论：听力卷在发布链上完全缺席 —— 不是被挡住，而是根本没有接线。** [转报，证据链完整]

### 4.1 `ExamCompiler` 抽象存在，但**是死代码**

`runtime_compiler.rs:4-9`：

```rust
pub(crate) trait ExamCompiler {
    type Output;
    fn compile(&self, source: &IeltsAuthoringIRV2) -> Result<Self::Output, Vec<CompilerIssueV2>>;
    fn validate(&self, compiled: &Self::Output) -> Vec<CompilerIssueV2>;
}
```

- 唯一实现 `ReadingExamCompilerV2`（:11-23），`type Output = ReadingExamSourceV2`
- **该 trait 生产代码从未被调用**：`grep ExamCompiler` 只命中本文件，唯一引用是 `#[cfg(test)]` 的 :31-32
- 真实编译直呼 free function `compile_reading_source_v2`
  （`authoring_v2_commands.rs:758`、`nas_package_v2.rs:193`、`quality.rs:827`）

⇒ 问题**不是**「`ExamCompiler` 缺一个听力实现」，而是**这个抽象层根本没被使用**。
补一个 `impl ExamCompiler for ListeningExamCompiler` 不会有任何效果 —— 这正是「文件存在 ≠ 接线存在」。

### 4.2 无 modality 调度

全仓**无**按 modality 选编译器的 match / factory / registry。
`ExamModalityV2 { Reading, Listening }` 见 `schema/ielts_authoring_v2.rs:11-14`，
唯一 modality 分叉是 `ielts_grammar/quality.rs:393`：

```rust
if authoring.get("modality").and_then(Value::as_str) == Some("listening") { return; }
```

只让听力**跳过** passage 校验。没有分支、没有 `unreachable!()`、没有 fallback。

### 4.3 听力侧只有 schema，无构造者也无消费者

- `schema/listening_runtime_v1.rs`：`ListeningExamSourceV1`(:92)、`ListeningAttemptV1`(:155)、
  `validate_listening_exam_source_v1`(:187)、`validate_listening_attempt_v1`(:420) —— 纯契约校验
- `schema/listening_audio_probe_v1.rs`：`probe_listening_audio_v1`(:133)，音频探测（symphonia）
- `schema/ielts_authoring_v2.rs`：`ListeningStructureV2`(:192)、`IeltsAuthoringIRV2.listening`(:543)
- **`src-tauri/src/listening_runtime_v2.rs` 不存在**（任务书列出的该文件在仓库里没有）
- 除 `schema/mod.rs:19` re-export 外，这些符号在 `src/` 内**无任何其它引用**

### 4.4 听力稿能「流进」gate，但随即被当 Reading 处理

两个 gate 的入参都是未分型 `ir: &Value`，**无 modality 分支**；
`validate_for_runtime_gate`（`runtime_validation.rs:288`）无条件 `let source = reading_source(ir);`。
V2 路径在 `reading_source_v2.rs:83-89` 直接失败：

```rust
let Some(passage) = source.passage.as_ref() else {
    return Err(vec![compiler_issue("RUNTIME_PASSAGE_MISSING", ...)]);
};
```

### 4.5 导出 / 运行时全部硬编码 Reading

`export_pack.rs:153-154`、`export_nas_library.rs:1130`、`preview_commands.rs:54` 均为 `reading_source(&ir)`；
`nas_package_v2.rs:193/199/446-448` 硬类型 `ReadingExamSourceV2`；
manifest `:655-656` 写死 `"schemaVersion":"ReadingExamSourceV2","modality":"reading"`；
`reading_runtime_v2.rs:64/72/80` 入参 `&ReadingExamSourceV2`。**无任何听力专属导出或渲染路径。**

### 4.6 判定

| 问 | 答 |
|---|---|
| (a) 自有 compile 实现 | **否** |
| (b) publish readiness gate | **否** —— 现有 gate 是 Reading gate，无听力分支 |
| (c) export 路径 | **否** —— 全部硬编码 Reading |

### 4.7 缺口规模（约 12–13 个集成点，建议独立一轮）

本轮只出事实与方案，不开写听力编译器：

1. `compile_listening_source_v1`（IR → `ListeningExamSourceV1`，含 audio/transcript）
2. 真正的 compiler 工厂 + 按 `ExamModalityV2` 派发
3. `reading_source()` 分型，或新增 `listening_source()`
4. 把 `probe_listening_audio_v1` 接入编译 / 校验 / 资产清单
5. 两个 gate 增加 modality 分支
6. `export_pack.rs` V1 导出分支与 wrapper kind
7. `export_nas_library.rs` direct-write 分支
8. `nas_package_v2.rs` 去掉 `ReadingExamSourceV2` 硬类型（193/199/446）
9. `authoring_v2_commands.rs` V2 导出的编译与 manifest kind
10. `preview_commands.rs` 预览渲染（现 unified HTML 为 Reading 专用）
11. 学生端 runtime（音频 / playback policy）
12. job/library 状态与 quality probes
13. 前端发布界面

## 五、canonical 版本一致性（预检 / 预览 / 导出）

### 5.1 访问器

| 访问器 | 位置 | 原子性 |
|---|---|---|
| `get_canonical_ds` → `Option<(Value, i64)>` | `library/repository.rs:137-157` | **原子**。单条语句（:143）同时选两列：`SELECT canonical_ds_json, current_edit_version FROM library_items_v2 WHERE id = ?1 AND canonical_ds_json IS NOT NULL` |
| `current_edit_version` | `repository.rs:167-178` | 只取版本，单独语句 |
| `apply_editor_commands_tx_with` | `repository.rs:1312-1319` | `TransactionBehavior::Immediate` 内单语句取文档+版本 |
| `cas_write_canonical` | `repository.rs:1153-1171` | 写侧 CAS：`WHERE id = ?1 AND current_edit_version = ?5` |

**写侧有 CAS，读侧全仓没有任何 CAS。**

### 5.2 两查询不一致：确实存在 **[转报，已给出行号]**

- `library/commands.rs get_workspace_item_core`：:33 `get_item` → :37 `get_canonical_ds`，
  却在 :54/:59 返回 `item.current_edit_version`。两次独立语句、无事务
  ⇒ **文档在版本 N、却报出版本 N+1 是可能的**。
- `reconcile/commands.rs:268` + `:271` 同一形态。

### 5.3 三条路径各读什么

| 路径 | 入口链 | 读的源 | edit_version |
|---|---|---|---|
| **预检** | `get_publish_preflight`(`lib.rs:1234`) → `authoring_v2_commands.rs:78` | **canonical**（:84-85） | **捕获并传递**：:86 取 `version` → :103 `check_publish_preflight(job_id, version, &authoring)` |
| **预览** | `generate_preview_assets`(`lib.rs:1396`) → `preview_commands.rs:37` | **`authoring-ir.json` 文件**（:38） | **忽略**（根本不存在） |
| **导出 A（canonical，绑定）** | `export_authoring_v2`(`lib.rs:1256`) → `authoring_v2_commands.rs:670` | canonical：:675 读、:677 CAS `expected != version`、:680-681 冻结 | **捕获并传递**；门禁(:743) 与编译(:758) 用**同一个 Value**，**无二次读库** |
| **导出 B（遗留，文件）** | `export_reading_assets`(`lib.rs:1408`)→`export_pack.rs:138`；`export_reading_js`→`:247`；`export_nas_library`→`export_nas_library.rs:1108` | **`authoring-ir.json` 文件** | **忽略** |

- `publish_items_core`（`nas_package_v2.rs:262`）：事务内 :270 读 canonical → :271-272 冻结 `(ds, version)`
  → :273 提交 → :302-306 把 `authoring: Some(ds)` / `edit_version: Some(version)` 交给
  `export_authoring_snapshot`；提交后状态 CAS（:414）。
- `export_authoring_v2_core` 的遗留兜底：未给 authoring/edit_version 时 :713 → `load_current_authoring`
  → `authoring_v2_commands.rs:1016-1020` **读了 canonical 却丢弃版本**（`_version`）、返回 revision 0
  ⇒ **版本被忽略**。

### 5.4 漂移判定

- **canonical 导出路径内部：不可能 (a)**。冻结（:710-714）、门禁（:743）、编译（:758）
  全在同一份内存 `Value` 上。
- **跨路径：可能且静默接受 (c)**。预检读 canonical，而预览 / 遗留导出读 `authoring-ir.json` **文件**；
  而 **DB 编辑从不重写该文件**（`library/repository.rs:4-5`、`authoring_v2_commands.rs:7`）
  ⇒ 一个被 V2 编辑过的 job，其**文件是陈旧或缺失的** ⇒ 预览 / 遗留导出**可能校验并产出与预检不同的文档，且全程无任何版本检查**。
- **另一处 (c)**：`publish_items_core` 发布的是冻结版本 N；并发编辑推进到 N+1 只被提交后 CAS（:414）
  捕获，而**该失败被 `eprintln!`（:415）吞掉，publish 仍返回 OK**。

### 5.5 单一入口：**不存在**

`get_canonical_ds` 原子返回文档+版本，但**没有**「按版本 N 读取可发布文档」的单一入口。
需要保持同步的读点：

- `repository.rs:137 get_canonical_ds`（原子）—— 调用方：
  `authoring_v2_commands.rs:85,675,1017`、`nas_package_v2.rs:270`、`cloud_repair/mod.rs:531,1022,1056`、
  `cloud_repair/tools.rs:320,418`、`reconcile/commands.rs:271,658,1046`、`processing/scheduler.rs:1332,1601`
- `authoring_v2_commands.rs:1007 load_current_authoring`（revision 文件 → canonical 但丢弃版本 → shadow）
- `library/commands.rs:37`（`get_item` + `get_canonical_ds` 两查询）
- **文件读**：`preview_commands.rs:21,38,73`；`export_pack.rs:138,247`；`export_nas_library.rs:1108`

---

## 六、收敛设计（**本轮只出方案，未改任何代码**）

### 6.1 唯一判据（建议放在 `authoring_validation.rs`，属我的文件集）

```rust
/// 发布结论的**唯一来源**。调用方只能读它，不得自行重算，也不得通过任何入参绕过。
pub(crate) enum PublishVerdict {
    /// 可以发布。
    Ready { edit_version: i64 },
    /// 有阻断项，不可发布。
    Blocked { reasons: Vec<PublishBlocker> },
    /// **未能判定**：校验 / 编译无法执行。绝不允许折叠进 `Ready`。
    Undetermined { reasons: Vec<PublishBlocker> },
}

pub(crate) struct PublishBlocker {
    pub code: String,
    pub layer: String,
    pub target: Option<String>,   // 稳定目标，便于模型/人定位
    pub message: String,
}

pub(crate) fn publish_verdict(canonical: &Value, edit_version: i64) -> PublishVerdict;
```

设计要点与你的三条要求一一对应：

1. **入参只有稿件与版本，没有 report / policy / options** ⇒ 无法通过参数绕过。
2. **版本必须由调用方显式传入** ⇒ 强迫调用方先原子取到 (文档, 版本)，从类型上消除「校验 A 版本、导出 B 版本」。
3. **返回枚举而非布尔** ⇒ 「未能判定」在类型上无法被折叠进「通过」。

### 6.2 关闭 `force` 绕过（**完全在我文件集内，最小改动**）

- `export_pack.rs` 是 policy 的**唯一**解析点（`from_input:37` → `from_policy:41`）。
- 改 `from_policy`：`"force"` 不再返回 `Ok`，改为明确拒绝的错误码；并删除
  `ValidationPolicy::Force` 变体、`validation_overridden`、`ignored_issues`。
- 删除后 `should_block` 退化为 `!validation_report_passed(report)`（恒严格）。
- **不需要改 `lib.rs`**：命令仍接收 `validation_policy`，只是任何非 `"strict"` 值都被拒。
- 前端契约影响（**需主线处理，`src/**` 我禁止触碰**）：
  `src/api/tauriCommands.ts:292` 的 `"force"` 参数、`src/types/reading-source.ts:100` 的 `ValidationPolicy`、
  `src/services/devFallbackBackend.ts:2394` 的模拟实现。
- 若产品**确实需要**保留人工强制导出：按你的要求必须 ①把 `validationOverridden` +
  `ignoredIssues` 明细写进导出产物**与**持久化记录（`persist_export_validation_report` 现在只写原始 report），
  ②且**不得由入参控制**，只能由后端在明确、可审计的人工动作下开启。
  **我的判断倾向直接删除该能力** —— 现有代码与调研中都没有任何证据表明产品需要它。

### 6.3 三条路径读同一份 canonical 同一版本

新增唯一读入口（`authoring_validation.rs` 或 `product_chain.rs`，均在我文件集内）：

```rust
/// 读「可发布文档」的唯一入口：文档与其版本必须原子取得。禁止任何文件读。
pub(crate) fn read_publishable_document(
    conn: &Connection,
    item_id: &str,
) -> CommandResult<Option<(Value, i64)>>;
```

- 内部只调 `get_canonical_ds`（已确认单语句原子），**禁止** `read_json(authoring-ir.json)`。
- 让 `preview_commands.rs`、`export_pack.rs`、`export_nas_library.rs` 改经它取稿，
  并把拿到的 `edit_version` 一路传到 `publish_verdict`。
- `authoring-ir.json` 的文件读只保留作**诊断 / 迁移**用途，且必须在报告里标注来源是文件而非 canonical。
- 未收敛点：`export_authoring_v2_core:713` 的遗留兜底在 `authoring_v2_commands.rs:1016-1020` 丢弃版本 —— 该文件不在我文件集内。

### 6.4 补齐三态（校验未能执行）

- 引入 `PublishVerdict::Undetermined`。
- 侧车：把 `validate_with_node_sidecar` / `validate_preview_with_node_sidecar` 的返回值改为能表达三态的类型。
  `Err`（脚本缺失 / spawn 失败 / 非 JSON）映射为 **Undetermined**，既不是 warning，也**不是** pass。
- **根因修复（守卫真值表两个异常格）**：`:209` / `:246` 的
  `!output.status.success() && parsed.get("passed")... != Some(false)` 应改为**先判执行是否成功**：
  - 非零退出 ⇒ 必须 Err / Undetermined，**不得**因为 payload 自报 `passed:false` 就返回 `Ok`；
  - `passed` 缺失或非 bool ⇒ **不得**当作有效报告，应为 Undetermined。
- `EPIC8_NODE_VALIDATOR_DIAGNOSTICS`（默认 false）关闭时，必须在产物里留下「未执行」记录，而不是静默 `return`。
- `preview_commands.rs:138-142`：落盘的 report 与用于置 `JobStatus::ExportReady` 的 report
  **必须合并为同一份**。（锁定旧行为的测试在 `lib.rs:5390` → 未收敛点）
- `publish_items_core:414-415` 提交后 CAS 失败被 `eprintln!` 吞掉且仍返回 OK：必须改为返回失败。
  该处在 `nas_package_v2.rs`，**在我文件集内**，可直接修。

### 6.5 收敛后的判定来源

目标 **1 处**。落地后各调用方：

| 调用方 | 现状 | 目标 |
|---|---|---|
| 遗留导出三命令 | `should_block`(policy) | `publish_verdict` ✅ 文件集内 |
| 预览 | 文件 + `validate_for_runtime_gate` | `publish_verdict` ✅ 文件集内 |
| 预检 | `check_publish_preflight` | `publish_verdict` ⛔ 未收敛点 |
| V2 导出 `:737-757` | quality_state + preflight + readiness 三判 | `publish_verdict` ⛔ 未收敛点 |
| `QualityReportV2.state` | `quality.rs:223-233` | 由 `publish_verdict` 派生 ⛔ **明确禁止** |

### 6.6 未收敛点（需你或主线决定）

| 未收敛点 | 文件 | 原因 |
|---|---|---|
| 共享 blocking 谓词 + V2 门禁 | `authoring_v2_commands.rs:432-453, 737-757` | 不在我的独占文件集 |
| 质量 state 判据 | `ielts_grammar/quality.rs:223-233` | **明确禁止** |
| 命令签名（若要移除 `validation_policy`） | `lib.rs:1407-1417` | 不在我的独占文件集 |
| 锁定旧预览行为的测试 | `lib.rs:5390` | 同上 |
| 前端 `force` 契约 | `src/api/tauriCommands.ts:292`、`src/types/reading-source.ts:100` | **明确禁止** |

### 6.7 反例清单（按你的要求：先构造能复现的反例，再修）

| # | 反例 | 现状（应复现的错） | 修后 |
|---|---|---|---|
| 1 | `export_reading_assets` 传 `validationPolicy:"force"`，稿含 `severity=error` | 仍导出成功；`validation-report.json` 不含绕过记录，绕过明细只在 `authoring-project.json` 的 `exportSummary` 里，且**只收 error、漏掉全部 warning** | 拒绝（force 能力删除） |
| 2 | 改名/删除 sidecar 脚本 + `EPIC8_NODE_VALIDATOR_DIAGNOSTICS=1` | 只留 warning，`passed` 不受影响，**无「未执行」记录** | `Undetermined` |
| 3 | sidecar 非零退出但 payload 带 `passed:false` | 返回 **`Ok`** | `Err` / `Undetermined` |
| 4 | 编辑 canonical（不重写 `authoring-ir.json`）后跑预览 | 校验的是**旧文件**，与预检结论可能不一致 | 读 canonical 同一版本 |
| 5 | `publish_items_core` 发布期间并发推进版本 | CAS 失败被 `eprintln!` 吞，**仍返回 OK** | 返回失败 |
| 6 | `get_workspace_item_core` 两次独立读之间并发写入 | 文档版本 N、报出版本 N+1 | 单次原子读 |

---

## 七、实现轮（2026-09-19）

文件边界本轮放开：`authoring_v2_commands.rs`、`ielts_grammar/quality.rs` 归我；
`lib.rs` 只改命令签名相关最小片段（改动点见 §7.9）。
仍然禁止：`cloud_repair/**`、`processing/**`、`reconcile/**`、`schema/**`、`src/**`、
`scripts/e2e/**`、`scripts/controlled-llm-service.mjs`、`llm_*`、`auto_pipeline.rs`、
`instruction_signature.rs`。

### 7.1 新的头号缺陷：`merge_sidecar_validation` 默认会**擦掉** Rust 自查结果

`authoring_validation.rs:158-164` 原文 **[亲验]**：

```rust
if layers_to_replace.is_empty() {
    layers_to_replace.extend(["ReadingExamSourceV1".to_string(), "DomProtocol".to_string()]);
}
let replace_existing_layers = sidecar
    .get("replaceExistingLayers")
    .and_then(Value::as_bool)
    .unwrap_or(true);        // ← 缺省就是「替换」
```

两处缺省叠加的结果：**一份"什么都不说"的侧车输出（`{}`）会让 base 上
`ReadingExamSourceV1` 与 `DomProtocol` 两层的 error 被整层删除**，
然后 `passed` 由 `!has_error_issues(剩余)` 重算 ⇒ **翻成 true**。

为什么它比 policy 入参那条更严重：
1. **不需要任何人主动传参** —— 缺省路径就成立；
2. 触发条件是「侧车输出不含 `layers`」，而 §3.2 的守卫异常格
   （`exit 0` + 无 `passed` 字段 → `Ok`）正好让**畸形输出也能走到 merge**；
3. `preview_commands.rs:95` **直传原始侧车输出**，没有像
   `runtime_validation.rs:340` 那样显式置 `replaceExistingLayers=false`。

⇒ 这两条串起来就是一条端到端的"静默改判"通道：
**空输出 → 守卫放行 → merge 删层 → `passed: true`**。

### 7.2 修法：默认不得替换，缺省必须是合并

- `replaceExistingLayers` 缺省改为 **`false`**（合并）。
- 删掉"`layers` 为空 ⇒ 替换默认两层"的回退 —— 空列表就是"不替换任何层"。
- 显式 `replaceExistingLayers: true` 的能力**保留**（协议未变），但必须由调用方写明。
- `preview_commands.rs:95` 不再依赖缺省，改为显式表达"这是合并"。

### 7.3 守卫真值表：按脚本**自身契约**修正，而不是按"非零即失败"

侧车脚本的契约（`validate-reading-source.mjs:141`、`preview-e2e.mjs:734`）是

```js
process.exit(report.passed ? 0 : 1);
```

⇒ **`exit 1` 正是脚本表达「校验失败」的正常方式**，不是"没能执行"。
因此正确的守卫不是"非零退出一律 Err"，而是**校验产物与退出码是否自洽**：

| exit | `passed` | 修前 | 修后 | 依据 |
|---|---|---|---|---|
| 0 | true | Ok | Ok | 正常通过 |
| 1 | false | Ok | Ok | 正常失败（脚本的既定表达） |
| **0** | **缺失/非 bool** | **Ok** | **Err** | 畸形产物，不可当报告 |
| 0 | false | Ok | **Err** | 与脚本自身契约矛盾 |
| ≠0 | true | Err | Err | 矛盾 |
| 2（用法错误，无 stdout） | — | Err | Err | JSON 解析失败 |

修前两个异常格（`runtime_validation.rs:209` / `:246`）**都是"进程没说清楚也放行"**。

### 7.4 `PublishVerdict`：发布判据的唯一来源

`PublishVerdict { Ready{edit_version} / Blocked{reasons} / Undetermined{reasons} }`，
`publish_verdict(root, job_id, authoring, edit_version) -> PublishVerdict`。

- 入参只有**稿件 + 版本**（`edit_version` 为 `Option`，`None` = 来源不是 canonical）；
  **不接受 report，也不接受 policy** ⇒ 没有参数可以绕开判据。
- 返回**枚举** ⇒ "未能判定"在类型上无法被折叠进"通过"。
- `Blocked` 与 `Undetermined` 都必须被调用方当成"不放行"，但**语义不同**：
  前者是"查过了，不能发"，后者是"没能查清楚，不许当通过"。

收敛后的判据来源 **1 处**；`quality::state`、预检 `passed`、V2 导出门禁、
`export_pack` 的 `report.passed` 全部改为**读同一个结论**（§7.6）。

### 7.5 一处需要说明的"偏离字面要求"

任务书写「进程失败不得返回 Ok」。按脚本自身契约，`exit 1 + passed:false`
是**正常的失败表达**，若把它判成 `Err`/`Undetermined`，后果是
**每一份真的有问题的稿子都从"确凿被拦"降级成"没能判定"**——
结论方向仍然是不放行，但可操作性变差（用户看不到具体问题）。
因此我按"自洽性"而不是"退出码非零"来修，并在 §7.8 明确标注这一条。

### 7.6 收敛映射

| 原判据 | 位置 | 收敛后 |
|---|---|---|
| 类型化就绪度 | `authoring_v2_commands.rs:285-306` | 读 `publish_verdict` |
| 质量 state | `ielts_grammar/quality.rs:223-244` | 抽出唯一规则 `quality_readiness()`，`publish_verdict` 与 `state` 共用 |
| 共享 blocking 谓词 | `authoring_v2_commands.rs:432-453` | 下沉为唯一实现，`authoring_v2_commands` 保持同名 re-export（`cloud_repair` 引用路径不变） |
| `report.passed` | `export_pack.rs:97-102` | 删除；`should_block` 一并删除 |

### 7.7 `option_alphabet` 是否参与发布门禁（第 5 项评估）

**是。而且这是本轮评估里唯一一个"可能由别人的改动引入新阻断"的点。**

`option_alphabet` 进入两条**阻塞**判据：

1. `ielts_grammar/quality.rs:1565-1582`：`selection_type = optionAlphabet != null`；
   不是选择型且又没有 `wordLimit` ⇒ `WORD_LIMIT_UNPARSED`（**blocking**）。
   方向：alphabet 变 `Some` ⇒ **减少**阻塞。
2. `ielts_grammar/quality.rs:1692-1695` + `:1761-1767` / `:1789-1800`：
   `expected_labels = optionAlphabet → A..X 标签集`；
   `labels_match = expected.is_none_or(|e| labels == e)`，不匹配 ⇒
   `OPTION_ALPHABET_MISMATCH`（**blocking**）。
   方向：`None → Some("A-F")` 把该处从"无约束"变成"必须恰好 A..F" ⇒ **可能新增**阻塞。

再加 `ielts_grammar/mod.rs:875-882` `run_matches_alphabet` 用它挑选项 run，
挑不中会退到 `fixed_options_from_v1`，仍拿不到就报 `OPTION_RUN_INCOMPLETE`（blocking）。

**决策（2026-09-19，已定）**：**严格增量**。
`instruction_signature.rs` 自本轮起划归我，主链 agent 仍禁止修改。

### 7.7.1 做法：**拆表**，不是整段挪动（与任务书字面写法有出入，理由在下面）

任务书说"把区间循环挪到 `paragraphs`/`sections` 分支之后"。**照字面整段挪会违反它自己的判据**
（"让 a-f/a-h/a-j 的新增严格只影响原本返回 None 的输入"），反例：

```
"The reading passage has seven paragraphs, A-G. Write the correct letter A-G."
```

这句同时含 `paragraphs`（复数，真实 IELTS 高频写法）与 `A-G`。

| 写法 | 该句返回 | 后果 |
|---|---|---|
| 改动前（区间在前） | `Some("A-G")` | `expected_labels = A..G`，`OPTION_ALPHABET_MISMATCH` **有效** |
| **整段挪到分支之后** | `Some("paragraph_letters")` | `expected_option_labels` 因无 `-` 返回 `None` ⇒ **约束消失** ⇒ 真实标签错配不再被拦 |
| **拆表（本轮采用）** | `Some("A-G")` | 与改动前**逐字相同** |

⇒ 整段挪动会**移除一个既有的阻塞检查**（属于"悄悄改别处行为"的同一类病），
而拆表让 `a-d/a-e/a-i/a-g` 保持在分支之前、只把新增的 `a-f/a-h/a-j` 放到分支之后，
**真正做到"新增只影响原本返回 None 的输入"**。

拆表同时满足任务书列的两条硬约束：

- **A-H 原始 bug 仍然修好**：`"…the list of words and phrases, A-H, below. Write the correct
  letter, A-H…"` 不含 `paragraphs`/`sections` ⇒ 拆表后仍返回 `Some("A-H")` ⇒
  `selection_type = true` ⇒ 不再误报 `WORD_LIMIT_UNPARSED`。
- 即使某句**同时**含 `section` 与 `A-H`：拆表后返回 `paragraph_letters`，而
  `paragraph_letters` 是 `Some` 且非 null ⇒ `selection_type` 仍为 `true` ⇒
  **`WORD_LIMIT_UNPARSED` 同样不会误报**，且不会新增 `OPTION_ALPHABET_MISMATCH` 约束。
  即两条目标不冲突。

### 7.7.2 待办（独立一轮，不在本轮做）

"接受收紧、把 `paragraph_letters` 升级为真正的字母表"是**另一个方向**，
它会**新增** `OPTION_ALPHABET_MISMATCH` 阻塞。要做必须先拿数据：
跑一批真实样本，统计"新拦下多少卷、其中多少是真错"，再决定。
本轮不做，仅记录为待办。

### 7.7.3 配套测试（本轮必做）

1. `instruction_signature.rs:424` 那条既有测试（`"Write A-G."` / `"Write A-F."`）
   **必须补 `option_alphabet` 断言** —— 它现在只断言 `task_type`/`confidence`/`warnings`，
   所以行为变了也不会红。这正是这次问题被藏住的原因。
2. 新增用例题干**同时含 `paragraphs` 与 `A-F`** ⇒ 必须走 `paragraph_letters`，
   **不得**返回 `Some("A-F")`。
3. `a-j` 的 `to` / en-dash（`–`）变体补覆盖。
4. 用一条**不含** `paragraphs`/`sections` 的 A-H 摘要题干，钉死 `Some("A-H")`
   （即"原始 bug 仍修好"）。

### 7.9 force 能力删除的执行条件（2026-09-19，已定）

`lib.rs` 允许改动的最小片段（三处）：

1. `lib.rs:1415` 命令体里的 `from_policy` 调用点；
2. `lib.rs:7708` `force_export_bypasses_publish_validation_and_records_override`；
3. `lib.rs:7827` `force_export_does_not_bypass_unsafe_exam_id`。

第 2、3 条**不得只把断言反过来**。改写后必须同时覆盖：

- 传 `"force"` **被明确拒绝**（返回错误）——**不能静默降级成 strict**，
  否则调用方会以为绕过生效了；
- 传 `"strict"` 或**不传**时，正常导出路径**仍然工作**；
- 第 3 条原本保护的不变量（**unsafe exam id 无论如何都要拦**）在新路径下**仍然成立**
  —— 这是真实的安全断言，不能随 force 一起被删。

> **记录**：这两条测试原本是**故意锁定 force 行为**的，说明当初有人**有意**引入它。
> 因此删除该能力是**产品决策**，不是清理死代码。

### 7.8 结果与证据等级

**测试基线**：`cd src-tauri && cargo test --lib --no-fail-fast`
（worktree `F:\workspace\PDF2Test-publish`，分支 `feat-publish-chain`，`CARGO_TARGET_DIR=F:/workspace/.cargo-target-publish`）

| 时点 | `test result:`（passed / failed / ignored） | 说明 |
|---|---|---|
| 本轮起点 `fa18598` | 757 / 0 / 11 | 交接记录里的数字 |
| `#8`(`eea4501`) + `#9`(`d679e39`) | — | 两条提交各自跑过定向模块；随后整包运行**编译失败**（`#10` 改到一半），所以**没有**干净的中间整包数字 |
| `#10`–`#13`、`#15` 首轮 | 835 / 1 / 11 | 唯一失败项是本轮**有意**改掉的错误串形状（§7.13.1），断言口径修正后转绿 |
| `#10`–`#15` 全量 | **837 / 0 / 11** | 以 `_test3.txt` 为准，exit 0 |

> ⚠️ `757` 来自交接记录，**不是**本轮实测：本轮第一次整包运行已经带着 `#10` 的在途改动，
> 因此不存在一个干净的"改动前"观测点。请以 **837 / 0 / 11** 作为本轮唯一实测结论。
> （我**不能**排除中间数字里混入了其他 agent 对非我文件集的新增测试。）

**证据等级**（按要求逐条标注）：

- **command-handler 级** —— 直接调用 `*_core(...)` / 被测函数，断言返回值 + **落盘产物**：
  `#8`、`#9`、`#10`、`#11`、`#12`、`#13`、`#14`、`#15`，**全部**是这一级。
- **UI / 产品 e2e 级** —— **本轮一条都没有**。改动全在 `src-tauri`，前端 `src/` 不在我的文件集内，
  我跑不通"界面点击 → IPC 命令 → 产物"的链路。因此本记录**不声称**产品层行为已验证；
  需要主线补的 UI 侧证据列在 §7.13.2。

逐条（反例 → 修前 → 修后）：

| # | 缺陷 | 反例（先写） | 修前行为（实测/静态推定） | 修后行为 |
|---|---|---|---|---|
| 8 | 侧车合并缺省=**覆盖** | `sidecar_report_without_layers_must_not_erase_rust_errors`、`sidecar_issues_are_merged_not_substituted` | 实测：`merge_sidecar_validation(&mut base, json!({}))` ⇒ `passed` false→**true**、issues 2→**0**；`layers` 缺失时退化为替换 `[ReadingExamSourceV1, DomProtocol]` 且 `replaceExistingLayers` 缺省 `true` | 缺省=**合并**；`layers` 为空 = **不替换任何层**；替换能力保留但必须显式 `replaceExistingLayers:true`；`preview_commands.rs` 显式置 `false` |
| 9 | 守卫真值表两个异常格 | `zero_exit_without_a_passed_field_is_not_a_report`、`zero_exit_claiming_failure_is_contradictory`、`nonzero_exit_claiming_success_is_contradictory`、`consistent_sidecar_reports_stay_trustworthy` | 实测：`(exit 0, 无 passed 字段)` 判 **Ok**；`(exit≠0, passed=true)` 判 Err 但 `(exit=0, passed=false)` 与 `(exit≠0, passed=false)` 混为一谈 | `interpret_sidecar_report` 三分：`Trustworthy{passed}` / `Malformed` / `Contradictory`；异常格一律 `Err` |
| 10 | 四处判据各自产出布尔 | （见 §7.6 收敛映射）`export_reading_js` 的批级 `\|=` 污染、`ignored_issues` 只记 `severity=="error"` | 判据散在 4 处；`report.passed` 自己就是第 4 个来源 ⇒ 同稿不同判 | 唯一入口 `authoring_validation::publish_verdict`（`Ready`/`Blocked`/`Undetermined`）；`passed` 由结论派生；入参只有稿件+版本，无 report/policy |
| 11 | `force` 可关掉发布门禁 | `force_policy_is_rejected_explicitly_not_downgraded`、`strict_export_still_works_after_force_removal`、`unsafe_exam_id_invariant_survives_force_removal` | 传 `validationPolicy:"force"` ⇒ 门禁关闭、产物写 `validationOverridden:true`/`ignoredIssues:[…]` | `invalid_validation_policy:force`（**明确报错，不静默降级**）；strict/缺省照常；`unsafe examId` 不变量保留 |
| 12 | 预览 E2E 有**两份** report | `preview_e2e_records_one_report_and_keeps_diagnostics_non_binding`（改成调**真函数**） | 就绪度与 job 状态用 `static_report`，落盘/返回用 `diagnostic_report` ⇒ 诊断不可用时产物 `passed=false` 而 status `ExportReady` | 一份产物：`passed` 与 job 状态同源；诊断结论进 `previewDiagnostics.passed`（`binding:false`）；`publishBasis` 说明 `passed` 由哪两项构成 |
| 13 | 提交后状态 CAS 失败被 `eprintln!` 吞 | `post_publish_status_cas_reports_edit_version_drift_instead_of_swallowing_it` | `if let Err(..) = conn.execute("UPDATE … WHERE id=?1 AND current_edit_version=?2")`：CAS 未匹配返回 **`Ok(0)`，不是 `Err`** ⇒ 静默；publish 返回 `Ok` 而库里 `status` 未改 | 抽成 `commit_published_status`：`Err` / 未匹配 / 正常三分；未确认一律 `PUBLISH_BATCH_STATUS_DRIFT:manifest_committed:{…}`（**不回滚**，清单已提交） |
| 14 | 跨路径版本漂移**全程无检查** | `version_alignment_detects_the_file_source_diverging_from_canonical` | 预检读 canonical、预览/遗留导出读 `authoring-ir.json` 文件；两者可不同，且产物里没有任何痕迹 | 四条读文件入口的产物新增 `versionAlignment`（三态：`checked:false` / `aligned:true` / `aligned:false` + `canonicalEditVersion`）；**只检出，不阻塞** |
| 15 | `option_alphabet` 表位置 | `which_paragraph_and_section...` 补 `option_alphabet` 断言；新增 `paragraph_letter_cues_take_precedence_only_for_the_newly_added_ranges`、`newly_added_ranges_accept_space_and_en_dash_spellings` | 单字母区间表含新增的 `a-f/a-h/a-j`，且整表在 `paragraphs`/`sections` 分支**之前** ⇒ 含 `paragraphs`+`A-F` 的题干返回 `Some("A-F")`（**新增**约束） | **拆表**：`a-d/a-e/a-i/a-g` 留在分支前，`a-f/a-h/a-j` 移到分支后。`paragraphs`+`A-F` ⇒ `paragraph_letters`；`paragraphs`+`A-G` ⇒ `Some("A-G")`（与改动前逐字相同）；A-H 原始 bug 仍修好 |

### 7.10 `#12` 的取舍：为什么不是"让状态改用诊断报告"

`preview_commands.rs:138-142` 的两份 report 有**两个**收敛方向，我只取了其中一个：

- ❌ **让 job 状态改用 `diagnostic_report`** —— 直接违反既有产品决策 **E8-26**
  （`Plan With Files/task_plan.md:506`、`findings.md:615`："诊断失败可见，但不再把 static
  已就绪的 job 从 `ExportReady` 降级"）。在 CI / 开发机上侧车通常不可用，这一改会让
  绝大多数预览把 job 打成 `NeedsReview`。
- ✅ **让产物改用状态的那份判据** —— 不动判据归属，只让**同一份产物**把两件事分别命名：
  `passed`（发布判据，与状态同源）+ `previewDiagnostics.passed`（诊断，`binding:false`）。
  `issues` / `layers` / `runtime` 的字段位置**一个没动**，前端读的地方不受影响。

顺带把那条**复刻式**测试换成了调真函数 —— 它原来自己造报告、自己算 readiness、自己调
状态函数，`run_preview_e2e_core` 内部错成什么样它都不会红。这正是"测试复刻被测函数就永远
测不到它"的样本。

### 7.11 `#13` 的判据为什么用"读回校验"而不是受影响行数

`execute` 返回的受影响行数属于**驱动层语义**（"匹配到"还是"真的改了"）。它是什么行为，
不该成为发布判据的一部分 —— 所以 `commit_published_status` 先执行 CAS，再
`SELECT status, current_edit_version` 读回确认后置条件。这样无论驱动层怎么算，
"条目是否真的被标成已发布且版本仍是发布时那一版"都有唯一、可读的答案。

**同类问题（已发现，本轮未处理 → §7.13.3）**：`library/repository.rs:1153-1171`
`cas_write_canonical` 也只 `map_err` 不查行数，同样是"CAS 未匹配 = 静默成功"。它不在我的
文件集内，且不在本轮缺陷清单上，故仅记录。

### 7.12 `#14` 为什么只检出、不阻塞

- **不阻塞**：阻塞等价于**强制同步** —— 现网所有"被 V2 编辑过的 job 走遗留导出/预览"
  会立刻失败。那是产品行为变更（`authoring-ir.json` 不再被 DB 编辑重写是既有设计），
  必须单独决定，不能夹在一次判据收敛里顺手做掉。
- **不做同步修复**：让三条路径都按版本读同一份 canonical，需要"按版本 N 读取可发布文档"
  的单一入口 —— 目前**不存在**（§5.5）。这是独立一轮的工作量。
- 所以本轮只把"两份稿是不是同一份"变成**可观察**：`versionAlignment` 三态字段。
  三态是必要的 —— "查不了"（`checked:false`）绝不能和"查了且一致"（`aligned:true`）混为
  一谈，否则这个字段一上线就会用"库里没记录"冒充"没问题"。

### 7.13 本轮发现但**未处理**的项（转报 / 待你决定）

1. **§7.13.1 就绪门 issues 仍不落盘（`#12` 的残留）**：`run_preview_e2e_core` 里
   `publish_readiness_gate` 的结果仍只用来取 `passed`，它新加的 issues（humanVerified /
   source review / authoring review）没有进产物。原因：把诊断合并结果喂给就绪门会让
   `merge_validation_issues` 按合并后的 issues 重算 `passed`，从而让诊断错误**影响状态**
   —— 又回到 E8-26 的反面。要彻底修得先决定"就绪门跑在哪一份 report 上"，属独立一轮。
   **据此可以查到"job 没到 ExportReady"，但产物里查不到是哪一条就绪度判据拦的。**
2. **§7.13.2 需要主线补的 UI 侧证据**（我做不到的部分）：
   - 删 `force` 后，前端 `src/api/tauriCommands.ts:292` 的 `"strict" | "force"` 类型**未改**。
     现网若仍传 `"force"`，用户会看到 `invalid_validation_policy:force` 报错。**后端入口已关闭，
     前端契约需主线另行安排**（是你指定的分工）。
   - 预览 E2E 产物新增 `previewDiagnostics` / `publishBasis` / `versionAlignment` 三个字段。
     纯新增，但**界面上要不要显示**、显示在哪，属 UI 决定。
3. **§7.13.3 同族缺陷（不在本轮清单）**：`library/repository.rs:1153-1171` `cas_write_canonical`
   与 `#13` 同形（CAS 未匹配不报）。文件不在我的文件集内。
4. **§7.13.4 一条因改错误串而修正的测试**：`authoring_v2_commands.rs`
   `export_rejects_non_ready_quality_state_before_materialization` 原来断言精确串
   `authoring_v2_export_blocked:quality_state=review_required`。判据合并后该串由 `publish_verdict`
   统一产出（一次列出**全部**阻断项，而不是只报第一个命中的）。断言已改为钉判据本身
   （`starts_with(blocked)` + `contains(QUALITY_NOT_READY)` + `contains(quality_state=review_required)`），
   保留"判据没过 ⇒ 一个产物都不落地"这条实体断言。**实质行为未变，只有消息形状变了。**
   顺带发现该测试的夹具 `jobId` 与请求的 `job_id` 不一致（`early-approaches-architecture-proof`
   vs `quality-gate`），修前因为质量判据先返回所以没暴露；现在 verdict 会把 `JOB_ID_MISMATCH`
   一并列出。**没有**改动夹具（属别人的测试意图），仅记录。

### 7.14 `#15` 拆表的对照实验（为什么不能整段挪）

| 题干 | 拆表后 | 整段挪到分支后 | 后果 |
|---|---|---|---|
| `…has seven paragraphs, A-G. Write the correct letter A-G.` | `Some("A-G")` | `Some("paragraph_letters")` | `expected_option_labels` 对无 `-` 的名字返回 `None` ⇒ **既有阻塞约束消失** |
| `…has six paragraphs, A-F. Write the correct letter A-F.` | `Some("paragraph_letters")` | `Some("paragraph_letters")` | 一致（都不新增约束） |
| `…list of words and phrases, A-H, below. …`（无 `paragraphs`/`sections`） | `Some("A-H")` | `Some("A-H")` | 原始 bug 两种写法都修好 |

⇒ 拆表是唯一同时满足"新增只影响原本返回 `None` 的输入"与"不移除任何既有约束"的写法。
`paragraph_letters` 仍是 `Some`/非 null ⇒ `selection_type = true` ⇒ `WORD_LIMIT_UNPARSED`
在两条路线上都不会误报，目标不冲突。


