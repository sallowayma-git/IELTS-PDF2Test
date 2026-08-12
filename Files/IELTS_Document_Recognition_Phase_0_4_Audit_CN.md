# IELTS 文档识别重构 Phase 0-4 审计报告

审计日期：2026-08-10

审计基线：`main` / `06f2ddf feat: complete phase 1 artifact and compatibility skeleton`

审计范围：Phase 0、1、2、3、4，含 PR-06 Shared slots vertical slice、PR-07 QualityReportV2、PDF2TEST 与 `E:\NAS` 的测试边界

审计方式：完整阅读总任务书和阶段完成记录；并发只读代码审计；运行专项门禁、构建、Rust 测试及跨仓库 vertical slice；核对未提交工作树

> 本报告只新增审计结论。未修改、重置、检出或清理 Phase 2-4 的实现、fixture 和完成记录；V1 仍保持 authoritative，V2 flags 仍默认关闭。

## 1. 总结论

**Phase 0-4 当前不能整体判定完成，也不应进入 Phase 5。**

最直接的阻断是 PDF `DocumentIRV2` producer 与 Rust contract 的 bbox 字段命名不一致，导致 Phase 2 shadow 在启用后无法生成；与此同时，Rust、TypeScript、JSON Schema 三份 V2 contract 已经分叉。Phase 0 frozen corpus、Phase 1 manifest 和 NAS peer contract 也都发生漂移。Phase 3 专项门禁通过，但不能抵消跨阶段 contract 和 corpus 门禁失败。Phase 4 的合成语义单测有实际价值，不过 8 份真实 PDF 的原始验收没有被自动测试证明，PR-06 的评分语义和 PR-07 的 hard invariants 仍不满足总任务书。

阶段判定：

| 范围 | 判定 | 说明 |
|---|---|---|
| Phase 0 | **不通过** | strict golden gate 为 35 errors；shared-response 真值无法被 metadata schema 结构化表达 |
| Phase 1 | **不通过** | schema manifest hash 漂移；三方 contract 分叉；NAS peer proof 缺失；revision store 仍有覆盖风险 |
| Phase 2 | **不通过 / shadow 当前不可用** | 专项门 1 passed / 11 failed；10 项由 bbox serde contract mismatch 直接触发 |
| Phase 3 | **专项通过，阶段总门未通过** | package 与 DOCX 专项门通过；真实 render-assisted 两栏 provider 的端到端证明仍缺 |
| Phase 4 | **部分实现，不通过验收** | grammar/quality 单测 29 passed；聚合 gate 接线陈旧；真实 8 PDF 验收未证明 |
| PR-06 | **prototype 存在，architecture proof 未完成** | fixture、compiler、NAS test-only loader/renderer 可运行，但评分语义、schema、双端 validator parity 不达标 |
| PR-07 | **未完成** | 尚未覆盖全部文档级/slot 归属/compiler hard invariants，QualityReport 也没有结构化 coverage ledger |

## 2. Findings

### P0-01 PDF V2 shadow 在启用后无法生成

**证据**

- PDF producer 写入 `nativeBBox` / `displayBBox`：`src-tauri/src/pdf_facts_shadow.rs:191-192`。
- Rust `SourceAnchorV2` 使用 `#[serde(rename_all = "camelCase")]`，字段 `native_bbox` / `display_bbox` 实际接受 `nativeBbox` / `displayBbox`：`src-tauri/src/schema/common.rs:67-76`。
- TypeScript contract 又声明 `nativeBBox` / `displayBBox`：`src/types/schema-common-v2.ts:37-43`。
- `npm run verify:phase2:shadow` 的 12 项测试只有 1 项通过、11 项失败；其中 10 项报 `unknown field nativeBBox/displayBBox`，覆盖 glyph、line/region/order、table、OCR、asset、widget、overlay 和 compare。

**影响**

`documentIrV2Shadow` 默认关闭只能限制暴露面，不能把功能判定为完成。显式启用后，PDF 路径只能产生 error artifact，Phase 2 主交付不可用，所有依赖 physical shadow 的 Phase 4 source evidence/coverage 也无法形成可信端到端证明。

**要求**

先选定唯一 wire name，统一 Rust serde、TS 和 JSON Schema；用真实 producer output 做 schema validation 和 Rust round-trip，而不是只验证各自内部类型。

### P0-02 Rust、TypeScript、JSON Schema 三份 DocumentIRV2 contract 已分叉

**证据**

- `contracts/common-v2.schema.json:59` 的 `sourceAnchor` 未声明新增 bbox、`pdfToDisplay`、`variants` 等字段，且 schema 使用 `additionalProperties: false`。
- `contracts/document-ir-v2.schema.json` 不接受 producer/Rust 已加入的 `visibilityObserved`、`geometryBasis`、`pageTransform`、`imagePlacements`、`markedContent`、`annotations`、`readingOrderGraph`，同时要求 producer 没有稳定提供的旧字段。
- Phase 1 verifier 只检查文件 hash、版本和 `$ref` 可达性，不验证 Rust/TS 与真实实例，因此没有拦住这次分叉：`scripts/verify-schema-contract.mjs:63-120`。

**影响**

Phase 1 的“schema round-trip 100%”验收不成立。即使修复 bbox 一个字段，后续仍会因 schema 属性集合差异继续失败；跨语言 consumer 也无法判断哪一份 contract authoritative。

**要求**

建立单一 contract source 或严格生成链，并增加四类测试：producer JSON -> JSON Schema、producer JSON -> Rust、canonical fixture -> TS、PDF2TEST/NAS 同 fixture hash + loader。任何一类漂移均应阻断 PR。

### P1-01 Phase 0 frozen corpus 已漂移，strict gate 为红

**证据**

- 5 个原有合成 DOCX 被改写，但 manifest、metadata、V1 baseline 保留旧 hash/size：`docx-external-image`、`docx-floating-text-box`、`docx-section-columns`、`docx-smartart`、`docx-table-merged-cells`。
- 漂移从 `fixtures/golden/manifest.json:56` 起可见。
- `npm run verify:phase0:strict`：`fixtureCount=39`、`errorCount=35`、`readyForPhase1=false`。

**影响**

Phase 0 的 frozen baseline 不再是可复现基线，阶段完成记录中“0 errors”的状态不适用于当前工作树。后续 parser 变化无法区分预期更新和无意回归。

**要求**

先明确这些 DOCX 是否为有意替换；若是，按受控 fixture revision 同步 manifest、metadata、baseline 和理由；若否，恢复正确生成逻辑而不是绕过 hash gate。更新后 strict gate 必须归零。

### P1-02 Phase 1 manifest 与 NAS peer contract proof 未完成

**证据**

- 当前 `contracts/ielts-authoring-ir-v2.schema.json` SHA-256 为 `0620c23a...`，`contracts/contract-manifest.json:17` 仍固定 `c249bb2b...`。
- `npm run verify:phase1:schema` 因 `hash_mismatch:IeltsAuthoringIRV2` 失败。
- 默认命令不传 `--peer-root`：`package.json:15`；verifier 在缺少参数时记录 `peerRepository: not_checked` 仍允许退出成功：`scripts/verify-schema-contract.mjs:104-120`。
- 计划指定的 `E:\NAS\developer\contracts\authoring` 当前不存在；显式 peer gate 返回 6 errors。

**影响**

schema bundle 不可复现，且所谓 cross-repo hash test 不是默认 hard gate。NAS test-only loader 当前靠本地手工 fixture 流转，不能证明两个仓库消费同一合同。

**要求**

修复并冻结本地 manifest 后，把 peer mirror 和显式 peer-root 校验纳入默认/CI gate；缺 peer contract 应失败或明确区分 local-only gate，不能以 `not_checked` 冒充 contract proof。

### P1-03 Phase 0 annotation schema 无法表达 shared-response 真值

**证据**

- `fixtures/golden/schema/golden-fixture-metadata-v1.schema.json:23-76` 只有 task 的 `displayRange/kind/slotIds` 与 slot 的 `responseType`。
- schema 没有 response group、cardinality、assignment、reuse policy、option-bank binding 或 scoring policy。
- `fixtures/golden/metadata/organisational-design.json:196` 只能把“共享题干、选二、无序计分”写进不可执行的 `knownIssues` 文本。
- 总任务书要求 case 标注 `responseGroups/cardinality/orderSemantics`，并计算 `Response Group Exact Match`。

**影响**

golden corpus 无法自动区分一个正确的 shared group 和两个错误的独立 checkbox 题，也无法作为 PR-06/Phase 4 的验收 oracle。

**要求**

升级 metadata schema 和 8 份真实样例标注，至少纳入 response group、option bank、cardinality、assignment、reuse/scoring policy、关键 source evidence 与预期 runtime 语义。

### P1-04 PR-06 共享多选当前按 all-or-nothing 计分

**证据**

- Rust 对 `unordered_set` 仅在提交集合与答案集合完全相等时给满分，否则 0 分：`src-tauri/src/reading_source_v2.rs:253-269`。
- NAS loader 同样返回 0 或满分：`E:\NAS\server\src\lib\library\reading\reading-generated-loader-v2.ts:281-303`。
- cross-repo test 把“一项正确、一项错误 = 0/2”固定成 expected：`E:\NAS\developer\tests\cross-repo\reading-v2-vertical-slice.cjs:121-125`。
- 总任务书明确规定 IELTS 默认每个答案位 1 分，`Choose TWO` 不能因共享 checkbox UI 改成整组全有或全无。

**影响**

UI/结构表达虽然是 shared response，计分行为却与目标考试语义不一致；错误测试会阻止未来正确实现。

**要求**

引入明确 scoring policy，IELTS 默认将唯一正确 label 一对一匹配到 slot、每 slot 0/1；duplicate/extra selection 行为由 policy 定义。Rust 与 NAS 必须共享同一组正反例，包括 2/2、1/2、0/2、重复值、额外值和顺序无关。

### P1-05 PR-06 schema 仍缺少长期所需的核心语义

**证据**

- 当前 `OptionBankV2` 只有 id/title/options/allowReuse/anchors：`src-tauri/src/schema/ielts_authoring_v2.rs:200-209`，没有 task/document scope。
- `AnswerSlotV2` 没有 example/non-scoring/participation 语义：`src-tauri/src/schema/ielts_authoring_v2.rs:259-278`。
- `ResponseGroupV2` 只有 cardinality/assignment/reuse，没有独立 duplicate policy 和 scoring policy：`src-tauri/src/schema/ielts_authoring_v2.rs:212-228`。

**影响**

Early Approaches 的最小 fixture 能编译，不代表模型能无歧义覆盖计划中的示例号、共享 bank 范围、重复选项和每-slot/整组/部分分策略。若现在进入更多题型，后续会再次改 contract。

**要求**

在 PR-06 关闭前补齐这些语义并写进 Rust、TS、JSON Schema 和 golden annotation；不要让 renderer 控件类型隐式决定评分规则。

### P1-06 PDF2TEST compiler 与 NAS loader 的 hard invariant 不一致

**证据**

- Rust compiler 能发现一个 slot 被重复分配，但遍历结束后没有检查每个 `answerSlots` entry 必须出现在一个 response group：`src-tauri/src/reading_source_v2.rs:151-194`。
- NAS loader 明确拒绝未分配 slot：`E:\NAS\server\src\lib\library\reading\reading-generated-loader-v2.ts:240-242`。

**影响**

同一 artifact 可能在 author repo 编译成功、到 NAS 才失败。PR-06 的 architecture proof 要求 producer 和 consumer 对 hard invariants 一致，当前只证明了 happy path。

**要求**

整理一份共享 invalid-fixture matrix，在两端逐项断言相同的 pass/fail 结果和稳定 issue code，至少覆盖 orphan/duplicate/missing slot、bank resolution、cardinality、answer key、display map 和 scoring policy。

### P1-07 PR-07 的 QualityReportV2 尚未实现总任务书规定的 hard gate

**证据**

- `evaluate_quality()` 从 `taskGroups/answerSlots/answerKey` 开始评估，但未验证合法唯一的 `examId`、passage 有效文本/fallback、V2/V1 compiler schema validation，以及每个 slot 恰好属于一个 response group：`src-tauri/src/ielts_grammar/quality.rs:12-153`。
- 当前 `contracts/quality-report-v2.schema.json:7-17` 只有 aggregate `sourceCoverage` 和开放 numeric `metrics`，没有可审计的 coverage ledger 或每个 significant source node 的 disposition。
- `calculate_source_coverage()` 可以读取 physical shadow 的 ledger，但报告只保留汇总比例，无法从 QualityReport artifact 追踪未解释区域。
- 总任务书 8.2 明确把上述条目列为文档级/题组级硬不变量，并规定任何 hard failure 都必须 `blocked`。

**影响**

当前 `ready` 仍可能遗漏 orphan slot、空 passage、非法 exam identity 或 compiler 不可消费的 artifact。分数和汇总 coverage 不能替代 hard invariants。

**要求**

让 PR-07 以 compiler validator 和结构化 coverage ledger 为事实来源；对总任务书 8.2 的每项 hard invariant 建唯一 issue code、阻断状态和正反 fixture。

### P1-08 Phase 4 聚合 gate 检查的是旧接线

**证据**

- `scripts/verify-phase4-grammar.mjs:59` 要求 `authoringV2Shadow` 默认关闭，并在 `auto_pipeline.rs` 中查找 `authoring_v2_shadow_enabled()`。
- 当前 reliability gate 已迁移为 `qualityGateV2` / `quality_gate_v2_enabled()`：`src/config/featureFlags.ts:32-42`、`src-tauri/src/environment.rs:227-228`、`src-tauri/src/auto_pipeline.rs:184-188`。
- 因此 `npm run verify:phase4:grammar` 失败于陈旧的静态字符串断言，而非 grammar 单测。

**影响**

Phase 4 完成记录中的“门禁通过”不可由当前工作树复现；同时，静态源码字符串 gate 容易在正确重构后误报。

**要求**

改为行为测试：flag off 时走 V1 weak reliability 且输出不变；`qualityGateV2` 显式开启时只读取 V2 quality state；缺/坏 shadow 时不得误判 Ready。保留默认关闭断言，但不要把函数名当 contract。

### P1-09 Phase 4 的 8 份真实 PDF 验收没有被证明

**证据**

- Phase 4 测试主要覆盖合成 JSON matrix 和一个手工 `early-approaches-authoring-v2.json`。
- 没有自动测试对总任务书点名的 8 份真实 PDF 逐份断言 task/slot 数、prompt boundary、option bank、asset、answer key 和 quality state。
- Organisational Design、Petri Dish、Western Celebrity 等关键验收目前依赖 metadata/完成记录描述，而 metadata 又无法表达完整 response-group 真值。

**影响**

29 个 grammar/quality 单测证明了纯函数和部分合成情形，不证明原始 Phase 4 目标“8 份 PDF 全部形成正确结构”。

**要求**

建立 8 case executable acceptance suite，串联 physical shadow -> authoring shadow -> QualityReportV2，并对关键结构做局部 golden 断言；完整 GUI/NAS 发布仍可保持关闭。

### P1-10 Phase 2 失败会遗留孤立 PDF shadow asset

**证据**

- PDF 提取先写 `assets/shadow/*`，之后才执行 `serde_json::from_value::<DocumentIRV2>`：`src-tauri/src/pdf_facts_shadow.rs:466-598`、`:862-997`。
- 失败清理仅在 DOCX 时删除 asset 目录；PDF 调用传入 `false`：`src-tauri/src/authoring_commands.rs:45-77`、`:219-221`。
- 本轮失败复现中，临时 job 留下 `assets/shadow/pdf-image-p001-8-0.png`，没有成功 shadow JSON 引用。

**影响**

反复 shadow 提取会留下孤立/过期资产，后续完整性校验可能误认旧文件存在，磁盘占用也会累积。

**要求**

写入 job-scoped staging 目录，完成 schema validation 与 manifest closure 后原子提交；所有失败路径统一清理 staging。不要直接清理共享 production asset 目录。

### P1-11 Phase 2 多项里程碑仍是占位或浅层实现

**证据**

- glyph bbox 主要由 text matrix 推导，style 只有有限字号事实，`visibilityObserved` / Unicode mapping observation 没有形成真实 provider evidence。
- `OutputDev::end_line` 被忽略：`src-tauri/src/pdf_facts_shadow.rs:232`；line test 反而断言 `hardBreakAfter` 不存在：`src-tauri/src/pdf_ingest/line_builder.rs:568`。
- OCR 初始 region 主要在整页无 glyph 时建立，未覆盖“native text + 含字图片”的 mixed image-text；`ocr_merge::merge_tokens()` 只被模块自身单测使用，没有接入 extractor。
- reading-order 的 `line_to_region` 参数未使用：`src-tauri/src/pdf_ingest/reading_order.rs:31`；graph 实际是线性排序的相邻边，cycle resolution 没有真实约束冲突输入。
- marked-content structure path 为空，widget appearance stream 只记 warning，不形成 appearance asset。
- PDF 路径直接整文件读入并尝试 repair，缺页数、对象数、图像像素、加密、脚本/embedded file 与资源上限 preflight。

**影响**

当前 helper 测试通过不能证明总任务书中的“无损物理层”、mixed OCR、tagged/widget 显示证据、DAG order 和安全边界已经完成。

**要求**

将这些能力按 B-001 等 backlog 拆成可观察 fixture + 指标；每项必须有 extractor 接线测试，不能以 provider-independent helper 存在替代端到端交付。

### P1-12 revision store 的 immutable revision 仍可能被覆盖

**证据**

- current pointer 缺失时 `read_current_revision()` 返回 revision 0：`src-tauri/src/artifact_store.rs:196-207`。
- `append_revision()` 直接基于该值计算下一 revision，未先恢复 pointer 或持有跨进程锁：`src-tauri/src/artifact_store.rs:235-260`。
- 底层 target 已存在时走备份后替换：`src-tauri/src/artifact_store.rs:525-568`。
- 现有测试覆盖显式 recovery 和顺序 optimistic conflict，没有覆盖 pointer 丢失后直接 append，或两个并发 base-0 writer。

**影响**

这两类路径都可能重写 `1.json/1.meta.json`，与 Phase 1 “immutable revision / atomic recovery”目标冲突。

**要求**

append 前在锁内恢复最大 revision，并以 create-new/不可覆盖方式提交 revision pair；增加 pointer 丢失、并发 writer、半写 meta/artifact 和 crash recovery 测试。

### P2-01 private corpus 边界不一致

`fixtures/golden/private-real` 当前有 16 份 PDF：8 个旧 `private-random-*` 和 8 个总任务书点名样例。Phase 0 verifier只要求 required 集合存在，不拒绝额外 private fixture；Rust 测试却硬断言目录恰好 8 份：`src-tauri/src/pdf_facts_shadow.rs:2531`。应明确 authoritative 8 case 集合，以 manifest 选择而不是目录数量作为 consumer contract。

### P2-02 Phase 3 专项实现较完整，但 render-assisted 验收仍缺一段端到端证据

`verify:phase3:docx-package` 和 `verify:phase3:docx` 均通过，恶劣 Word fixtures、package safety、表格/编号/图片/外链阻断测试质量整体良好。剩余主要缺口是总任务书要求的“spaces/tab 模拟两列选项通过 render-assisted 模式恢复”没有一个真实 renderer/provider output fixture 串到最终结构；当前更多证明 fallback 协议和合成几何逻辑。

### P2-03 CI 没有覆盖最容易漂移的 contract/corpus gate

`.github/workflows/windows-smoke.yml:5-14` 的 path filter 不包含 `contracts/**`、`fixtures/**`，workflow 也不运行 Phase 0 strict、Phase 1 schema/peer、Phase 2-4 专项门。正因如此，schema hash 和 frozen fixture 漂移不会由 PR CI 直接阻断。应增加快速 contract/corpus job，并让 full/nightly gate 覆盖真实 corpus。

### P2-04 工程卫生门当前不是全绿

- `cargo check` 通过但报告 94 warnings。
- `cargo fmt --all -- --check` 失败于既有 `job_commands.rs`、`job_store.rs`。
- live/sample30 脚本要求显式 corpus/参数，不能作为零配置 gate 运行；应在 gate runner 中明确配置、skip 条件和结果状态。

这些不等同于 Phase 4 新回归，但“全部门禁通过”的完成表述在当前工作树不成立。

## 3. 门禁结果

| 命令/检查 | 结果 | 审计解释 |
|---|---:|---|
| `npm run verify:phase0:strict` | **FAIL** | 39 fixtures，35 errors，`readyForPhase1=false` |
| `npm run verify:phase1:schema` | **FAIL** | `IeltsAuthoringIRV2` manifest hash mismatch |
| Phase 1 显式 NAS peer schema gate | **FAIL** | 6 errors；peer contract mirror 缺失 |
| `npm run verify:phase2:shadow` | **FAIL** | 1 passed / 11 failed；10 个 bbox serde mismatch，另有 private corpus 数量断言 |
| `npm run verify:phase3:docx-package` | PASS | package safety 专项通过 |
| `npm run verify:phase3:docx` | PASS | DOCX 富结构专项通过 |
| `npm run verify:phase4:grammar` | **FAIL** | gate 脚本仍检查旧 `authoringV2Shadow` reliability 接线 |
| Phase 4 grammar/quality Rust 单测 | PASS | 29 passed |
| PR-06 跨仓库 vertical slice | PASS（局部） | fixture、NAS test-only loader/renderer happy path 可运行；不代表评分/contract 已验收 |
| PDF smoke | PASS | 3/3 passed |
| `npm run check` | PASS | TypeScript/Svelte check 通过 |
| `npm run build` | PASS | 前端构建通过 |
| `cargo check` | PASS with warnings | 94 warnings |
| `cargo test --lib` | **FAIL** | 216 项：198 passed、15 failed、3 ignored；15 项由 11 个 Phase 2 失败加 4 个既有环境失败构成 |
| `cargo fmt --all -- --check` | **FAIL** | 既有 `job_commands.rs`、`job_store.rs` 格式差异 |
| live/sample30 | 未作为 gate 执行 | 缺必需 corpus/命令参数，当前不是零配置门禁 |

说明：完成记录中的 Rust “196 项 / 190 passed / 4 environment failures / 2 ignored”是较早快照；当前工作树新增测试后实际枚举为 216 项，且 Phase 2 contract mismatch 引入了可复现的新失败。

## 4. 已满足且应保留的约束

1. V1 仍为 authoritative；V2 只生成 shadow artifact，没有进入正式 UI、export、runtime 或 NAS production 发布。
2. `documentIrV2Shadow`、`authoringV2Shadow`、`qualityGateV2` 以及其余 V2 flags 默认均为 `false`。
3. Rust shadow 还受 debug build + 显式 environment opt-in 限制。
4. PDF per-question LLM repair 在 TS resolver 与 OCR 计划中继续保持关闭。
5. Phase 3 的 package safety、富结构专项 fixture 和测试是当前 0-4 中完成度最高的一段，应避免在 contract 修复时破坏。
6. Phase 0 verifier 本身能发现 hash/size/metadata/baseline 漂移；当前红灯说明 detector 有效，不应绕过。
7. Phase 4 的题号表达式、instruction signature、completion、diagram、answer key、asset integrity 等纯函数/合成测试提供了可复用基础。

## 5. 修复顺序与重新放行条件

### Stop-the-line：先恢复可验证基线

1. 统一 Rust/TS/JSON Schema 的 DocumentIRV2 contract，修复 bbox 命名和全部属性集合。
2. 用 producer 实例建立 strict schema/serde/TS round-trip；恢复 `verify:phase2:shadow` 全绿。
3. 处理 5 个 DOCX corpus 漂移，恢复 `verify:phase0:strict` 0 errors。
4. 更新并冻结 contract manifest，落 NAS peer mirror，使默认 cross-repo gate 真正执行。

### 完成 PR-06 architecture proof

1. 扩展 golden annotation，使 shared response 成为可执行真值。
2. 补 OptionBank scope、slot participation/example、duplicate/scoring policy。
3. 把 IELTS shared multi-select 改为每 slot 0/1，并在 Rust/NAS 共用正反例。
4. 对齐 author compiler 与 NAS loader hard invariants，特别是每 slot 恰好归属一个 response group。

### 完成 PR-07 quality gate

1. 实现总任务书 8.2 的全部文档级和题组级 hard invariants。
2. 让 V2 compiler validation、asset closure 和 coverage ledger 成为 QualityReport 的可审计输入/输出。
3. 修复 Phase 4 聚合 gate，使用行为测试验证 `qualityGateV2` off/on 路径。
4. 建立 8 份真实 PDF 的结构化 acceptance suite，验证 task/slot/prompt/bank/asset/answer/quality。

### Go 条件

只有以下条件同时满足后，才建议开始 Phase 5：

- Phase 0 strict、Phase 1 local + peer、Phase 2、Phase 3、Phase 4 聚合门全部通过；
- `cargo test --lib` 无新增功能失败，环境依赖项被明确隔离而非混在 PASS 声明中；
- PR-06 的评分与双端 validator parity 通过；
- PR-07 的 hard invariants 和真实 8 PDF acceptance 通过；
- V1 authoritative 和所有 V2 默认关闭约束继续保持。

## 6. 工作树保全说明

审计开始与报告落盘前均核对了 `git status`。当前分支仍为 `main`，HEAD 仍为 `06f2ddf`。Phase 2、3、4 的 tracked 修改和 untracked 文件均完整保留；审计过程没有执行 `reset`、`checkout`、`clean`、提交或暂存操作。测试生成内容仅位于被忽略的 `tmp/`，本报告是审计新增的唯一预期文件。
