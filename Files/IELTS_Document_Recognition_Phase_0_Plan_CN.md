# IELTS 文档识别重构：第一个计划（Phase 0）

> 来源：[IELTS_Document_Recognition_Overhaul_Plan_CN.md](IELTS_Document_Recognition_Overhaul_Plan_CN.md)
>
> 状态：修复完成，等待全局 Rust build 与 strict 复验（2026-08-12 最终审计）
>
> 本计划只冻结事实和测试契约，不修改生产解析链，也不启用 PDF 逐题 LLM repair。

## 目标

把总计划的第一个实施阶段拆成一个可复现的基线迭代：冻结当前代码版本，建立 golden corpus 清单和标注契约，捕获当前 V1 输出，并准备默认关闭的 V2 feature flags。没有源文件的私有语料必须显式标记为缺失，不能用推测补齐题组、答案或资源。

## 范围与交付物

| 编号 | 工作项 | 交付物 | 状态 |
|---|---|---|---|
| P0-01 | 冻结仓库基线 | PDF2Test 与 NAS 两仓库分支、commit、fixture SHA-256 记录 | 已完成 |
| P0-02 | 建立 golden corpus 契约 | `fixtures/golden/manifest.json`、metadata schema、metrics contract、README | 已完成 |
| P0-03 | 纳入可用 fixture | 3 个 parser PDF + 8 个固定种子随机私有 PDF 的 page role/task/slot/asset 标注 | 已完成 |
| P0-04 | 捕获 V1 基线 | `fixtures/golden/baseline/v1/*.json` | 已完成 |
| P0-05 | 建立合成 fixture 目录 | 15 个 PDF + 5 个 DOCX 合成场景、metadata 与 V1 baseline | 已完成 |
| P0-06 | 建立 feature flags | `documentIrV2`、`authoringV2`、`runtimeSourceV2`、`nasPackageV2`、`listeningV1` 默认关闭 | 已完成 |
| P0-07 | 可重复验证 | `npm run verify:phase0` | 已完成 |

## 本次迭代的验收标准

- 每个已纳入 fixture 都有源文件相对路径、SHA-256、字节数、预期页面角色、题组、答案位、资源和已知问题。
- V1 基线保存的是当前实际 CLI 输出的规范化快照；随机 job id、时间戳和机器绝对路径不进入比较事实。
- 8 份私有 IELTS PDF 使用固定种子随机抽样；每份都有本地源文件、原始文件名、SHA-256、字节数、metadata 和 V1 baseline。若来源缺失，仍必须以 `missing-source` 登记，并使 `readyForPhase1` 保持为 `false`。
- 20 个合成场景都有实际源文件、SHA-256、metadata 和 V1 baseline；它们是测试夹具，不是授权的真实 IELTS 题源。
- 所有 V2 feature flag 默认 `false`；PDF 逐题 LLM repair 保持关闭。
- 验证命令能发现文件缺失、hash 漂移、metadata/baseline 缺失和重复 fixture id。
- `metrics.json` 覆盖物理文字、题目结构、富版式、发布运行时四组指标，并声明 7 个 hard gates。
- 对总计划点名的 8 个旧版题源已建立独立的 `reference-only` JS 索引；它只作为历史结构证据，不把旧版 JS 当作本次随机样本的 PDF 源文件。

## 当前状态

8 份私有 IELTS PDF 已从用户提供的目录随机抽取并复制到被 Git 忽略的 `fixtures/golden/private-real/`。抽样池共 272 份，算法为按文件名排序后执行 seeded Fisher–Yates，种子为 `20260809`；本次样本和 hash 记录在 `manifest.json`。第二个 NAS 仓库已发现于 `E:\NAS`，且已纳入冻结清单。

重新登记或复现本次样本时，运行：

```text
npm run register:phase0:private -- --source-root "C:\Users\lenovo\Desktop\working space\0.3.1 working\ReadingPractice\PDF" --seed 20260809 --count 8
npm run capture:phase0-baseline
npm run annotate:phase0:private
cargo build --manifest-path src-tauri/Cargo.toml --bin ielts-author-studio
npm run verify:phase0:strict
```

严格门会先确认用于 V1 baseline 对比的 debug CLI 不早于任何 Rust/Cargo 输入；CLI
缺失或陈旧时直接失败，避免用旧二进制对当前源码产生假绿。CI contract 模式只验证
tracked corpus 契约，不依赖未入库私有 PDF 或本地 debug CLI。

合成 fixture 可重复生成：

```text
npm run generate:phase0-synthetic
npm run register:phase0-synthetic
```

## 执行记录

- 2026-08-09：冻结当前仓库 `main` 分支和基线 commit `78811e384b792ffae481192df7a52f0a4526e110`。
- 2026-08-09：发现并冻结 NAS 仓库 `E:\NAS` 的 `feat/exam-terminal-security-lifecycle` 分支和基线 commit `84b38bc71564ace01168a34bedc2401d6dd8c0f7`。
- 2026-08-09：确认可用 PDF 为 `complex-reading.pdf`、`image-only-reading.pdf`、`no-text.pdf`。
- 2026-08-09：建立 golden corpus manifest、metadata 和 V1 baseline 工具链。
- 2026-08-09：捕获 3 份当前 V1 baseline；在真实私有语料补齐前，严格门因缺失外部语料保持失败。
- 2026-08-09：生成并登记 15 个 PDF + 5 个 DOCX 合成 fixture；重复生成 hash 保持一致。
- 2026-08-09：捕获 20 个合成 fixture 的 V1 baseline，工作区 fixture 总数达到 23。
- 2026-08-09：补齐 `metrics.json` 与 JSON Schema；普通验证报告 4 组、35 项指标和 7 个 hard gates。
- 2026-08-09：从 `E:\reading-exams` 建立 8 个旧版题源的 reference-only 索引，全部匹配但不替代 PDF。
- 2026-08-09：`npm run check` 和 `npm run build` 通过；Rust 全量测试 116/120 非忽略测试通过，4 项依赖缺失的 `Files/*.pdf`/解析器环境而失败。
- 2026-08-09：从 `C:\Users\lenovo\Desktop\working space\0.3.1 working\ReadingPractice\PDF` 的 272 份 PDF 中以种子 `20260809` 随机抽取 8 份，复制到本地私有目录并登记 SHA-256/字节数。
- 2026-08-09：捕获 8 份随机私有 PDF 的 V1 baseline；工作区 fixture 总数达到 31。
- 2026-08-09：根据实际 V1 输出初始化随机样本的页面、题组、答案位和资源标注，并保留人工复核已知问题。
- 2026-08-09：`npm run verify:phase0:strict` 通过，报告 31 个 fixture、0 个缺失私有样本、0 个待生成合成样本、0 个错误。
- 2026-08-12：最终审计修复 strict gate 的 CLI 新鲜度缺口；严格门现在 fail-closed
  拒绝缺失或早于 Rust/Cargo 输入的 V1 比较二进制。
- 2026-08-12：Phase 0/1 feature flag 门改为读取 TypeScript AST，断言全部必需默认
  项存在且为 `false` 字面量；缺失、`true`、间接表达式、重复属性和未知 spread 均
  fail-closed。`pdfPerQuestionLlmRepair` 仍不可被 caller 覆盖为开启。
