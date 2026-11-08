你在 `F:\workspace\PDF2Test`（Tauri 题稿制作应用：Rust 后端 `src-tauri/src`，React 前端 `src`）上做一次架构精简开发。先完整阅读：

1. `AGENTS.md`（产品路径优先，验证必须走真实 Tauri 应用）
2. `docs/plans/2026-10-14-cloud-direct-pipeline.md`（本次方案，奠基性文档，必须自己读完）
3. `docs/qa/user-bugs-2026-10-14.md`（上一轮修复背景）

## 背景（一句话）
现在云端识别被设计成"本地解析的附属候选"：要等本地首稿完成才起跑（`processing/scheduler.rs:557`），契约禁止它转录正文（`llm_suggestions.rs:476`），要复用本地节点 ID，然后过约 20 道拒绝关、采纳阈值、6 轮修复循环、裁决、来源核验。结果是又贵又慢、经常拿不到能用的题稿。目标是改成：**在线时云端一次直出整份 `IeltsAuthoringIRV2` 题稿（含原文、题组、答案）→ 确定性归一化（只补不拒）→ 唯一 Rust 编译校验 → 直接写正式稿；离线时走现有本地流水线兜底。两条链二选一，不再比对合并。**

## 本次任务范围：方案中的 P0 + P1 + P2

**P0 检查点**
- 工作树里有上一轮（Codex，10-14）未提交的修复。先 `git status` 看清楚，把这些改动作为一个检查点提交（提交信息写明"checkpoint: user-bugs 2026-10-14 fixes"）；不要丢弃、不要 reset。然后从这里新建分支 `feat/cloud-direct`。
- 加配置开关 `pipelineMode: "direct" | "legacy"`（默认 direct，设置页可切回 legacy；legacy = 现有行为不变）。

**P1 直出主链**
- 新增云端调用（例如 `generate_exam_direct`），走现有 `llm_gateway` 的传输/重试/记账，但用全新的输出契约：
  - 输入：每页渲染图（扫描页必须）+ 每页文字层文本 + 契约 + 范例 + `taskPresentationRules` 表。**不传 `localNodeTargets` / `sourceParagraphs`**。
  - 输出：完整题稿，必须包含 `passage.content`（段落带 `paragraphLabel`）、`paragraphMap`、`taskGroups`、`answerSlots`、`answerKey`；PDF 含答案页时在同一次调用里读出答案。ID 由模型自取。
  - DeepSeek 识别请求保持关闭思考；超时默认 240s；`finish_reason=length` 时按页段分块重试一次。
- 归一化：能由题型推导的字段一律补齐（cardinality、scoringPolicy、duplicatePolicy、allowOptionReuse、sourceAnchors、quality、instructionSignature、displayRange、questionOrder、questionDisplayMap、TFNG/YNNG 固定选项、段落信息匹配选项来自 paragraphMap），不因缺这些字段拒收。
- 校验：调用 `reading_source_v2.rs` 的 `compile_reading_source_v2`。有错误 → 把错误清单和当前 JSON 发回云端修 1 次（最多 2 次）→ 仍有错但结构可用：保留并在编辑器标注；完全不可用（无题组或无正文）才回退本地链。
- 保真度提示（非阻断）：正文与 PDF 文字层做 diff，低覆盖段落在编辑器高亮。
- 调度器：`pipelineMode=direct` 且有可用云端配置时，**不等本地首稿**，直接走直出链；成功则直接写正式稿（revision source 标 `cloud_direct`），并跳过 reconcile/冻结基线/plan_adoption/adopt_cloud_primary/修复循环/裁决/来源核验/独立答案页请求/云端校核锁。无配置或网络失败 → 现有本地链。
- token 预算：去掉"用量未知即锁死任务"，改为软上限 + 按请求体估算记账。

**P2 单一编译与门禁收敛**
- 学生预览改为调用 Rust 编译命令，不再使用 TS 的 `buildReadingSourceV2FromAuthoring` / `validateReadingExamSourceV2` 作为判定（TS 类型可保留）。
- 发布判定收敛为：编译通过 = 可发布；其它（答案未填、低置信、保真度低、未人工确认）全部降为警告，在编辑器和发布对话框展示。去掉前端永远带 `force` 的伪装（`src/api/publishClient.ts`）。

**本次不要做**：P3 的大规模删除（旧模块只旁路，不删除）；V1 导出改造；听力链路。

## 直出契约的起点
主线程正在并行做低成本直出验证，完成后会写入：
- `docs/plans/cloud-direct/validation.md`（3 份真实 PDF 直出结果、校验结果、答案准确率、耗时与 token）
- `docs/plans/cloud-direct/prompt/`（验证过的 system prompt、输出契约、范例）
开始写直出调用前先检查这两个路径；存在就以它为契约起点，不存在就先做调度器和归一化部分，稍后再接。不要修改 `docs/plans/cloud-direct/` 下的文件（那是主线程的输出）。

## 验收（必须走真实产品路径）
按方案 §4：真实 Tauri exe + 本机 DeepSeek 配置（设置页里已有，不要读取或打印 API key），样本目录 `F:\workspace\IELTS Atlas\ReadingPractice\PDF`，至少 5 份（Pearls、western celebrity、musical expert、Maori fish hooks、Brief History of Tea）。每份：导入 → 学生预览无阻断 → 发布 NAS 包 → 加载器探针通过；正文覆盖率、题数、与 IELTS Atlas V1 `answerKey` 的答案一致率、云端调用次数、耗时、token 全部记录。再关闭云端配置跑同样 5 份，确认离线能产出可编辑题稿。可复用 `scripts/e2e/tauri-cdp-*.mjs` 的 CDP 驱动方式。

## 工作约束
- 子代理只用于宽读取/检索，模型用 haiku 或 sonnet；方案取舍、代码修改、最终验证由你负责。
- 不要为了让测试变绿去改 golden 基线；旧链路相关测试在 legacy 模式下应保持通过，direct 模式新增独立测试。
- 不要改用户已有题库、服务配置或已安装的应用；验证用独立应用数据目录。
- 每完成一个阶段提交一次（中文提交信息即可），不要 push。
- 最终报告写到 `docs/qa/cloud-direct-2026-10-14.md`：区分"真实产品端到端验证 / 命令层验证 / 仅 CLI 或 schema 验证"，列出未覆盖的缺口，并统计本次旁路掉的模块与代码行数。
