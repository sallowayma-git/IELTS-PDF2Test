# 云端直出主链：精简优化草稿（2026-10-14）

## 0. 一句话目标

**一份 PDF 进来，在线时由云端一次直接产出学生端可用的整份题稿（原文 + 题组 + 答案）；离线时由本地解析兜底产出可编辑题稿。** 两条链是"二选一"，不再互相比对、裁决、合并。

最终产物不变：学生端（`IELTS-NASfor-WenDao/apps/student-exam`）可识别的 `ReadingExamSourceV2`（`<examId>.js` 包装 + manifest）。V1（IELTS Atlas 现有 241 份）不在本轮改造范围，只要不破坏现有 V1 导出即可。

## 1. 现状诊断（证据摘要）

| 问题 | 证据 |
|---|---|
| 云端被本地"卡住起跑" | `processing/scheduler.rs:557` 云端任务先 `local_draft_ready_rx.await`，本地首稿写完才发请求 |
| 云端被禁止转录正文 | `llm_suggestions.rs:476` 契约："Do NOT transcribe the reading passage"；云端只给题组，正文永远来自本地 |
| 云端被要求复用本地节点 ID | 契约 `localNodeTargets` / `sourceParagraphs` 规则；直接导致 heading 节点 ID 撞车、云端新段落被白名单拒收 |
| 云端结果过约 20 道拒绝关 | 候选校验器 `llm_gateway.rs:2702-3041` 约 40 个 `return Err`；整份退回本地：题号重叠 <50%、句子匹配 <20%（`cloud_adoption.rs:67-114`）|
| 采纳后还有一串昂贵云端调用 | 答案页（`scheduler.rs:1135`）、修复循环最多 6 轮/10 分钟（`cloud_repair/mod.rs:45-47`）、裁决、来源核验；Pearls 实测后续校核 53,820 token 仍未完成 |
| 门禁多且最终被绕过 | 约 5 个独立"能否发布"判定源、约 54 个 `RUNTIME_*` 码；但前端发布永远带 `force`（`src/api/publishClient.ts:92`），门禁只挡预览/审阅不挡产物 |
| 同一编译器写两份 | 预览走 TS `readingRuntimeV2.ts:663`，发布走 Rust `reading_source_v2.rs:82`；Codex 修了 Rust 漏了 TS → 用户仍见 `RUNTIME_OPTION_BANK_MISSING` |
| 过渡工程残留 | v1/v2 双写（`document-ir.json` + `*.shadow.json`）、"shadow" 实为必需输入、两个发布器、`tauriCommands.ts` 60 个导出 36 个 UI 无调用 |
| 体量 | 后端 Rust 193k 行：本地解析/启发式 ~62k，云端相关 ~62k（`cloud_repair` 21k、`reconcile` 19k、`llm_gateway` 7.4k），一份 PDF 经过 13 种中间表示 |

## 2. 目标架构

```
                ┌──────────── 在线（有可用云端配置）────────────┐
PDF ─► 摄取(本地) ─► 云端直出(1 次) ─► 归一化补默认值 ─► 编译校验 ─┬─► 写入正式稿 ─► 编辑/预览 ─► 发布
      页图+文字层       整份 V2 题稿      (只补不拒)        (唯一编译器) │
                                                                    └─ 有结构错误 → 带错误清单修 1 次 → 仍失败：保留并标注问题（或回退本地）
                ┌──────────── 离线 / 云端失败 ─────────────┐
PDF ─► 现有本地流水线 ─► 本地题稿 ─► 同一编译校验 ─► 写入正式稿 ─► 编辑/预览 ─► 发布
```

### 2.1 主链（在线）

1. **摄取**（本地、便宜、复用现有代码）：落盘原文件；pdfium 渲染页图；抽取每页文字层（仅作云端提示和保真度对照，不做识别）。
2. **云端直出**（1 次调用；页数很多时按页段分块，但每块都直出完整结构）：
   - 输入：页图（扫描页必须）+ 每页文字层 + 输出契约 + 2–3 份标准范例 + 题型规则表（复用 `taskPresentationRules`）。
   - 输出：完整题稿 —— `passage.content`（含 `paragraphLabel`）+ `paragraphMap` + `taskGroups` + `answerSlots` + `answerKey`（PDF 里有答案页就一起读出，不再单独发答案页请求）。
   - ID 全部由云端自取（`passage-paragraph-A`、`group-1`、`q1`…），**不再传 `localNodeTargets`**。
   - DeepSeek：识别类请求关闭思考（Codex 已加），JSON 模式，超时 240s。
3. **归一化**（确定性、只补不拒）：补 `cardinality`/`scoringPolicy`/`duplicatePolicy`/`allowOptionReuse`/`sourceAnchors: []`/`quality`/`instructionSignature`/`displayRange`/`questionOrder`/`questionDisplayMap` 等可由题型推导的字段；TFNG/YNNG 固定标签；段落信息匹配从 `paragraphMap` 生成选项。
4. **编译校验**：唯一的 Rust 编译器 `compile_reading_source_v2`。
   - 通过 → 写入正式稿。
   - 有结构错误 → 把错误清单 + 当前 JSON 发回云端修 **1 次**（最多 2 次）。
   - 仍失败 → 结构可用就保留并在编辑器中标注问题；完全不可用（无题组/无正文）才回退本地链。
5. **保真度提示**（非阻断）：正文与 PDF 文字层做字符级 diff，算段落覆盖率；低于阈值的段落在编辑器里高亮"请核对"。扫描 PDF 无文字层则跳过。
6. **写入正式稿**：`revision source = cloud_direct`，直接进编辑器；预览和发布走同一个编译器。

### 2.2 离线链（降级，不删除）

- 无云端配置、网络失败、云端输出完全不可用 → 走现有本地流水线（`auto_pipeline` 本地部分），产出可编辑题稿，过同一个编译校验。
- 本地链**不再作为云端的比对基线**；云端成功时本地链不运行（或仅在后台作为"离线备份"运行，结果不参与合并）。
- 已有本地题稿时，用户可手动"用云端重新生成"：整份替换（带确认），不做逐段合并。

### 2.3 从主链移除（先旁路，验收后删除）

- reconcile 周期、冻结本地基线、`plan_adoption` 阈值、`adopt_cloud_primary` 合并
- 云端修复循环（6 轮）、裁决（adjudicate）、来源核验（verify_source_answers）
- 独立答案页请求（并入主调用；仅当主调用明确报告答案页不可读时才单独补一次）
- 云端校核锁、蓝绿选择
- token 预算的"用量未知即锁死"：改为软上限 + 按请求体估算记账

### 2.4 门禁收敛

- **唯一硬门禁**：编译校验通过（学生端能加载）。
- 其余（答案未填、低置信、保真度低、未人工确认）一律降为**警告**，在编辑器和发布对话框里展示，不阻断预览、不阻断发布。
- 预览改为调用 Rust 编译命令，删掉 TS 那份编译/校验的重复实现（或仅保留类型）。

## 3. 分阶段实施

| 阶段 | 内容 | 退出标准 |
|---|---|---|
| P0 检查点 | 当前未提交改动（Codex 10-14 修复）提交为检查点；新建分支；加开关 `pipelineMode: direct \| legacy`（默认 direct，可在设置里切回） | 现有测试不回退 |
| P1 直出主链 | 新增直出调用 + 输出契约 + 范例 + 归一化 + 1 次修复；调度器在线时直接走直出链，离线走本地链；直出结果直接写正式稿 | 见 §4 验收，真实 Tauri 产品路径 |
| P2 单一编译与门禁 | 预览调用 Rust 编译；发布判定收敛为"编译通过 + 警告列表"；去掉 `force` 伪装 | 预览与发布对同一份稿结论一致 |
| P3 删除 | 直出链在 ≥10 份真实 PDF 验收后，删除 reconcile/adoption/repair/adjudicate/blue-green/review-lock、v1 双写、无调用命令、`devFallbackBackend` 对应部分 | 统计删除行数；全量测试绿 |

## 4. 验收标准（产品路径，遵守 AGENTS.md）

样本：`F:\workspace\IELTS Atlas\ReadingPractice\PDF` 中至少 5 份，覆盖 heading、TFNG/YNNG、单选/多选、摘要填空（含词库）、表格/流程图填空、扫描答案页。建议含：`24. P1 - The Pearls`、`107. P2 - A study of western celebrity`、`100. P3 - What makes a musical expert_`、`10. P1 - Maori Fish Hooks`、`1. P1 - A Brief History of Tea`。

在线（真实 Tauri exe + DeepSeek flash）每份需满足：
- 导入到学生预览无阻断错误；发布生成 NAS 包，学生端加载器探针通过。
- 正文完整：文字层 PDF 段落覆盖率 ≥ 0.95，段落标签 A–X 齐全。
- 题号与题数和原卷一致；题干、选项无截断。
- 答案与 IELTS Atlas 对应 V1 文件（`assets/generated/reading-exams/*.js` 的 `answerKey`，用 `manifest.js` 的 `pdfFilename` 对应）一致率 ≥ 95%。
- 云端调用 ≤ 3 次/份；总耗时 ≤ 90s/份；token ≤ 约 60k/份。

离线（关闭云端配置）：同样 5 份都能产出可编辑、可预览的题稿（质量不作硬性要求）。

报告中必须区分：真实产品端到端验证 / 命令层验证 / 仅 CLI 或 schema 验证。

## 5. 风险与对策

| 风险 | 对策 |
|---|---|
| 模型编造或改写正文 | 文字层 diff 高亮 + 人工预览；不做自动拒收 |
| 长 PDF 超输出上限 | 按页段分块直出（每块完整结构），再按题号拼接；或正文与题组分两次调用 |
| 扫描件无文字层 | 只发页图；保真度检查跳过，编辑器提示"扫描件请核对" |
| 删除旧链路引起回归 | P1/P2 只旁路不删除，开关可切回；P3 才删 |
| DeepSeek 思考吃光输出 | 识别类请求关闭思考（已有）；`finish_reason=length` 时自动分块重试一次 |

## 6. 配套材料

- 低成本直出验证（主线程并行执行）：结果写入 `docs/plans/cloud-direct/validation.md`，可用的直出 prompt 与范例写入 `docs/plans/cloud-direct/prompt/`。开发时以此为直出契约的起点。
