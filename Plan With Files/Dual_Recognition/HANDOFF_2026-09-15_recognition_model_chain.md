# 识别闭环后端交接 — 模型链路核实与修复

日期：2026-09-15
范围：`src-tauri/src/reconcile/**`、识别调度、模型网关、后端类型与后端测试
未触及：`src/**`（前端由另一 agent 负责）

---

## 0. 一句话结论

上一轮交付的「识别闭环后端」在**类型、规则引擎、状态字段**层面是完整的，但**模型链路实际没有接通**：云端识别的真实输出与消费它的解析器形状完全不兼容，导致云端候选项 **100% 在进入裁决前被丢弃**。本轮修好了这个桥接，并已用端到端测试证实。

**A3（原文件核验接入模型）与 A4（分歧时真实模型裁决）仍未接通**，见 §5。任何"函数名含 source/verification/adjudicate"或"存在调用次数常量"都不构成已实现的证据。

---

## 1. 实际调用链核实表（任务 A 的交付）

| 阶段 | 生产入口 | 真实模型调用 | 实际输入 | 实际输出 | 启动时机 |
|---|---|---|---|---|---|
| 本地识别 | `processing/scheduler.rs::run_job_inner` → `auto_pipeline::run_auto_pipeline_core`（`execution_mode=localOnly`） | **否**（确定性本地管道） | 原文件 + `DocumentIRV2` | `IeltsAuthoringIRV2` 草稿（可编辑） | 与云端**并发**起飞（本轮修复） |
| 云端识别 | 同上 → `auto_pipeline::generate_cloud_reading_outline` → `llm_gateway::run_openai_compatible_cloud_outline_llm` | **是**（全链路唯一真实模型调用） | PDF → base64 `type:"file"`（失败回退页图）；非 PDF → `DocumentIRV2` 抽出的 `sourceText` | 原始 JSON：`{title, groups[{kind, range[], layoutHint, questionIds, notesText, confidence, evidence.quotes, instructionsText, stimulusText, optionBank, slots[]}], answerKey, confidence, warnings}` | 与本地**并发**（本轮修复，此前为「本地跑完才启动」） |
| 原文件核验 | `reconcile/source.rs::verify_against_source` | **否** | 仅 `DocumentIRV2` | `SourceVerificationV1`（纯确定性） | 云端之后 |
| 统一裁决 | `reconcile/adjudicate.rs::adjudicate` | **否**（纯函数，无 IO） | canonical / local / cloud / source | `RecognitionDecisionV1` | 核验之后 |
| 安全自动应用 | `reconcile/commands.rs::apply_patches_with_recheck` | **否** | 决策项 | patch + 撤销信息 | 裁决之后 |

**局部性结论**：三路事实面中，只有 `cloud` 一路来自模型；`source` 一路**完全没有模型参与**。因此当前"三路核验"实质是「模型识别（cloud）+ 确定性解析（source）+ 本地管道（local）」，不是「两路模型互校」。

---

## 2. 本轮修复与证据

### 2.1 A2 云端「完整识别」真正进入裁决（最高优先级缺陷）

**缺陷**（已亲自核实）：`llm_gateway.rs:1249-1263` 的校验器强制 `groups[].range` 为**二元数组**，而 `reconcile/candidate.rs:390-395` 的解析器要求 `range` 为**对象** `{kind,start,end}`，且要求 `taskId`/`taskType`/`slots[]`——模型从不产出这些内部标识。结果：每个题组以 `cloud_group_task_id_missing` / `cloud_group_range_invalid` 被丢弃，`kept_groups=0`、状态 `Unusable`、`reason_code=MODEL_INVALID_OUTPUT`。**云端答案从未参与任何比较**。

**修复**：
- `llm_suggestions.rs::make_cloud_paper_generation_input`：`outputContract` 从「仅出 outline」升级为**完整识别**（新增 `instructionsText`/`stimulusText`/`optionBank`/`slots[]`），并明确「只出 outline 不可接受」。
- `reconcile/candidate.rs::expand_natural_cloud_shape`：把模型真实形态**确定性**映射为内部形态（`kind`→`TaskTypeV2`、合成 `taskId`/`responseGroupId`/`slotId`、`range`→内部对象、答案按题型包装）。设**无操作短路**（任一题组同时含 `taskId`+`taskType` 即原样透传），故既有内部形态测试零改动。
- `reconcile/candidate.rs::align_cloud_answer_shapes`（经 `engine.rs:262` 调用）：把云端答案重编码为**本地答案的形状**。必要原因：`rules.rs:85-126` 的 `answer_compare_key` 是**形状敏感**的（`text`→`values.join("|")`；`option`→`opt:LABELS:assignment`），形状不同会把同一答案判成"实质分歧"，从而在每个题上制造噪声。只对齐形状、保留云端自有值与标签，本地无答案则跳过。

**证据**：`reconcile::commands::tests::cloud_natural_shape_drives_full_reconcile_cycle ... ok` —— 用**真实校验形态**（`range` 数组 + `answerKey`）驱动 `run_recognition_cycle_core`，断言云端链不再为 unusable 且云端答案确实产生带 `cloud_value` 的裁决项。另有 `natural_cloud_shape_is_no_longer_discarded`、`non_contiguous_natural_group_yields_matching_slots`、`unknown_natural_kind_fails_closed_with_reason_code`（未知 kind 仍 fail-closed，无静默成功）。

**顺带确认**：`compare_groups`（`rules.rs:511-521`）在云端 `taskId` 与本地不一致时**已**退化为按题号集合匹配，因此适配器合成的云端 task id 不会造成题组级误报。

### 2.2 A1 本地识别与云端识别真正并行

**缺陷**：原实现**先 `await` 完整本地识别，发布草稿后才启动云端**，云端模型调用在本地跑完前根本不存在。

**修复**（`processing/scheduler.rs::run_job_inner`）：
- 立即并发拉起本地阻塞任务与云端 async 任务；
- **只 await 本地** → 失败/取消检查 → `advance(local="succeeded")` → `set_item_status_ready` → **冻结 `base_edit_version`**；
- **之后**才 await 云端句柄 → 再取消检查 → 进入裁决（复用已拉取结果，不二次联网）；
- 云端 permit 与 `advance(running)` **移入云端任务内部**，本地识别不再被云端并发容量卡住；
- 云端任务在被取消 / lease 丢失时返回 `None`，调用方跳过裁决并如实报 `cloud=failed, reconcile=skipped`，不伪造结论；
- 新增 `set_cloud_status_only`，避免本地仍在跑时把 stage 提前报成 `cloud_recognition`。

**中途纠错记录**：第一版实现（由 `sched-dev` 提交，当时 666 测试通过）引入了两个产品可见回归——① 本地识别被云端 permit 卡住（云端 permit 饱和时本地不启动）；② 草稿发布开始等云端模型调用返回（破坏"本地先出稿、立刻可编辑、不等云端"）。两项均由代码审读发现并已修正。

**证据**：`processing::scheduler::tests::cloud_fetch_starts_before_local_recognition_finishes_and_draft_published_first ... ok` —— 同时断言「云端在本地完成前已起飞」与「草稿发布早于云端完成」。

### 2.3 DOCX 云端链路打通

**缺陷**：`auto_pipeline.rs:433` 的 `main_source_is_not_pdf` 直接阻断 DOCX；且 `llm_gateway.rs:718-732` 的 `sourceText` 分支在全仓**没有任何生产者**（死代码）。

**修复**（`auto_pipeline.rs`）：
- 新增 `main_source_for_cloud`：只拒绝"没有源文件"，**不**拒绝"非 PDF"；
- 新增 `document_ir_source_text`：从 `DocumentIRV2` 的 `pages[].lines[].text`（回退 `spans[].text`）抽取原文，作为非 PDF 的证据面；
- `generate_cloud_reading_outline` 对非 PDF **摘掉错误的 `pdfPath`**（否则会以 `data:application/pdf` 声明发送 DOCX）、改填 `sourceText`；抽不到文本时如实返回 `cloud_source_text_unavailable`，不假装识别过。

**行为测试已补齐（2026-09-15 补记）**：原计划由 `bridge-verifier` 补齐，该 agent 被限流中断；但同批次的 `auto_pipeline.rs` 中实际已存在并通过以下三项测试，故本条"仅编译级保证"的表述**已过时**：
- `auto_pipeline::tests::main_source_for_cloud_accepts_docx` —— 目标 3(a)：DOCX 主源被云端链接受，绝不返回 `main_source_is_not_pdf`；
- `auto_pipeline::tests::main_source_for_cloud_rejects_missing_file_not_wrong_type` —— 只拒绝"缺文件"，拒绝原因不含 `main_source_is_not_pdf`；
- `auto_pipeline::tests::document_ir_source_text_flattens_pages_and_lines` —— 目标 3(c)：`DocumentIRV2` 无文本时返回 `None`，有文本时拼接返回。

**仍未覆盖**：目标 3(b) 的**连接线**断言——"非 PDF 时网关入参带 `sourceText` 且不带 `pdfPath`"目前由源码审读确认（`auto_pipeline.rs:1270-1295`），无独立测试；驱动 `generate_cloud_reading_outline` 需已配置的 LLM profile（`config/llm-profiles.json`），未在测试中伪造。

### 2.4 任务 B 反序列化边界

- `schema/recognition_v1.rs`：`ApplyRecognitionDecisionsRequestV1` 加 `#[serde(deny_unknown_fields)]`（**防旧格式 `decisions` 被静默当成空操作**）；新增 `validate()`（空请求 / 同一 id 同时 accept+reject / 空 id / 负版本一律拒绝，且**不写 journal**）与 `normalized()`（去重）。
- `reconcile/commands.rs::apply_recognition_decisions_core`：在写 journal **之前**调用 `validate()` + `normalized()`；接受分支对**非答案字段**改用 `is_user_edited` 复核，避免"用户只改了别处"导致整批作废。

---

## 3. 测试证据

| 状态 | 结果行 |
|---|---|
| 本轮起点（基线） | `test result: ok. 660 passed; 0 failed; 11 ignored` |
| 本轮结束 | `test result: ok. 670 passed; 0 failed; 12 ignored; finished in 26.18s` |
| **收尾补记（同日晚）** | `test result: ok. 671 passed; 0 failed; 11 ignored; finished in 23.52s` —— 隔离测试定性后解除 `#[ignore]` 并通过（+1 passed / −1 ignored） |

任务书要求的「测试是否真断言了？还是只断言 mocks？」：本轮新增测试断言的是**真实校验形态经真实解析/裁决产生的真实结构**（`status`、`salvage`、题组/槽位数量、题号、`answer_compare_key` 两端相等、`cloud_value` 非空、cancel 语义），不是 mock 调用次数。**仍属单元/命令处理层**，未驱动 Tauri UI（见 §5）。

---

## 4. 已知未结项（必须补齐）

1. ~~**被隔离的测试**：`reconcile::commands::tests::cloud_shape_mismatch_does_not_spawn_false_divergence` 当前为 `#[ignore]`~~ → **已定性并解除隔离（2026-09-15 补记）**。

   **定性方法**：把该测试产生的 `decision.items` 落盘后逐项读明细（`tmp/_divergence_dump.json`，读毕已删）。明细**只有 2 项**，足以定性：

   | decision_id | reason_code | 判读 |
   |---|---|---|
   | `d:slot:slot-14:answer` | `NO_SOURCE_EVIDENCE`（`unverifiable`） | `localValue` 与 `cloudValue` **字节完全一致**（均 `option[B]/per_slot`）⇒ 反证 `align_cloud_answer_shapes` **工作正常**：云端原始 `text[B]` 已被重编码为本地形状。不是分歧项。 |
   | `d:slot:slot-14:slot_interaction` | `SLOT_INTERACTION_ANSWER_MISMATCH` | 消息为"作答方式是「text」，答案却是「option」"。**这才是 `SUBSTANTIVE_DIVERGENCE` 的真正来源** |

   **结论：夹具自相矛盾，不是产品缺陷。** 分歧与"形状敏感比较"无关，而是测试自己的夹具声明了 `interaction:"text"` 却给了 `option` 答案。

   **修复**：
   - 夹具改为自洽：`taskType: "single_choice"` + `interaction: "radio"` + `responseGroups[0].kind: "choice"`，答案保持 `option[B]/per_slot`；`stub_cloud_runner_answer_b` 的题组 kind 同步改 `single_choice`。
   - 断言**收窄**：只断言 `target.target_type == Slot && target_id == "slot-14" && field == Answer` 的项不携带 `SUBSTANTIVE_DIVERGENCE`（从 13 处产出点中排除 groups/assets/source-coverage 等无关维度）。
   - 断言**强化**（新增正向证据）：定位 slot-14 的 Answer 项，断言 `local_value == cloud_value`。这比"不存在分歧"更有力——直接证明云端确实被重编码为本形状，而非"两边恰好都没有值"。
   - 移除临时诊断落盘代码。
   - 验证：`cloud_shape_mismatch_does_not_spawn_false_divergence ... ok`，且已不在 ignored 集合中。
2. 云端 payload 的真实形态**未做离线回放**验证（无录制的真实模型输出作为夹具）；当前桥接的正确性由"契约形状 + 校验器语义"共同定义。
3. `sched-fix` 的并行改造**未在真实 `run_job_inner` 上端到端驱动**（该函数需要 Tauri `AppHandle` + 库 DB + 真实管道，且两条链无注入点）；当前证据是并发原语 + 排序 helper 的单元测试，以及代码结构审读。

**已核对**：当前 11 个 `#[ignore]` **全部是有意隔离**（live-LLM 探针需环境变量、夹具写出器、需真实语料/凭据），**不存在其它被隔离的失败测试**。

---

## 5. 未完成的任务项（本轮**没有**做，不得视为已交付）

| 项 | 状态 | 说明 |
|---|---|---|
| A3 原文件核验接通真实模型 | **未实现** | `verify_against_source` 仍纯确定性，只看 `DocumentIRV2`，无图像/模型参与 |
| A4 分歧时真实模型裁决 | **未实现**（但脚手架已就位） | `adjudicate` 仍是确定性规则。**更正此前表述**：`MAX_ADJUDICATION_MODEL_CALLS = 2`（`schema/recognition_v1.rs:23`）与 `MAX_CONSTRAINED_REPAIRS = 1`（`:25`）**是真实定义的常量**（此前误记为"只在注释里"，且行号误引 `engine.rs:10-11`，那里只是文档注释）；`needs_model_adjudication`（`adjudicate.rs:352-358`）也是**已实现**的判定函数。三者共同点：**没有任何执行路径调用/读取它们** ⇒ A4 缺的不是设计，是**调用方与预算执行**。实施方案见 `PLAN_2026-09-15_model_verification_and_adjudication.md` |
| 任务 C：8 个并发/持久化窗口 | **未做** | 用户中途改稿、状态持久化失败、进程中断、重试、并发请求、单云任务失败/取消、重启恢复 |
| 任务 D：8 个可直接用于 UI 的真实场景 | **未做** | 未产出可复现产物与预期字段 |
| 任务七：真实云服务验证（PDF + DOCX） | **未做** | 需凭据；本轮未写入任何密钥 |
| 任务八：退出标准 9 问逐项汇报 | 见本文件 §1–§5 | 按「已实现 / 已验证 / 待集成 / 受阻」分类 |

---

## 6. 后续接手注意事项

- `base_edit_version` 必须冻结在**草稿可编辑那一刻**（本地识别完成后）。这是"迟到结果不覆盖用户修改"的机制本身——提前冻结会误判用户修改，延后冻结会漏判。
- 云端 permit 只应覆盖**模型调用**，不要覆盖 reconcile。
- `answer_compare_key` 是形状敏感的。任何新增的答案来源（A3/A4 会新增）都必须在比较前做形状对齐，否则会在每个题上制造假分歧。
- 云端契约同时被两条消费链读取：`outline_group_summary_from_cloud`（读 `range/kind/layoutHint/questionIds/notesText`）与 `cloud_candidate_from_value`（读内部形态）。改动契约时必须同时满足两者。
- `CloudReadingOutlineV1` **没有 Rust 结构体、也没有 `contracts/` 下的 JSON Schema**，事实规范由 `llm_gateway.rs::validate_cloud_outline_output` 手写校验定义。若要真正确立契约，应补一份 schema 与契约漂移检查。

---

## 7. 本轮验证方式与局限（诚实声明）

- 全部结论基于**源码逐行审读 + `cargo test --lib`**（收尾补记后为 671/0/11）。环境限制：本会话 Bash 工具不可用，改用 PowerShell 并将输出落盘后读取。
- **未**进行 Tauri 应用级/浏览器级端到端验证；因此"产品行为端到端已验证"这一档，本轮只有 `run_recognition_cycle_core` 层级（命令处理以下）的证据。
- 三个实现/验证 agent（`reconcile-dev`、`bridge-verifier`、`sched-fix`）先后被账号级限流（429）中断，其中 `bridge-verifier` 留下**无法编译**的半成品测试（`commands.rs` 私有导入 + 高阶生命周期错误），已由本文档作者修复至可编译；`sched-fix` 的 scheduler 改造已完成但**未自证**，其正确性由本文档作者的代码审读 + 测试结果共同确认。
- **同日晚的补记工作（定性隔离测试、收窄断言、核对全部 ignored 项）由本文档作者亲自完成**：重派子代理的**三次**尝试（`triage-dev`、`a34-plan`、`a34-plan-r2`）**全部在 1 秒内以同一 429 终止**，证明账号级频率限制尚未解除（重置时间 2026-09-16 10:03 UTC+8），故不再重试。
  **注意**：`Agent` 的 spawn **返回成功不等于子代理可用**——本次 `a34-plan-r2` 的 spawn 正常返回，失败是异步到达的，不可据 spawn 返回值判断限流是否解除。
- **本轮全部改动尚未提交**（工作树含 19 个已跟踪文件改动 + 多个新增未跟踪文件）。
