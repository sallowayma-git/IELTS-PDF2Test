# A3 / A4 实施方案：把真实模型接进「原文件核验」与「分歧裁决」

> 状态：**设计稿，尚未实现**。本文所有论断都给出 `文件:行号`，可逐条复核。
> 目的：让实施者不必重做代码分析，直接按本文落地。
> 作者：主线程（子代理 `a34-plan` 与其重试 `a34-plan-r2` 均被账号级 429 限流中断，两次都在 1 秒内失败）。

---

## 0. 先修正一处事实（上一份交接单写错了）

上一份交接单写的是「`MAX_ADJUDICATION_MODEL_CALLS` 是**死常量，只在注释里**」。
实测（全仓 `src-tauri/src` 检索）**不成立**：

- `MAX_ADJUDICATION_MODEL_CALLS` 是**真实定义的常量**，值 `2`，位于
  `src-tauri/src/schema/recognition_v1.rs:23`；`MAX_CONSTRAINED_REPAIRS = 1` 在 `:25`。
- 但**全仓没有任何执行路径读取这两个常量**——仅出现在
  `engine.rs:10-11`（模块文档）与 `adjudicate.rs:351`（函数文档）两处注释里。
- `needs_model_adjudication`（`adjudicate.rs:352-358`）是**已实现**的判定函数
  （"存在 `NeedsReview` 且无 `proposed_patch` 且非 `DEPENDENCY_BLOCKED` 的项"），
  **同样没有任何调用方**。

⇒ 准确表述：常量与判定函数都在，缺的是**调用它们的地方**。这不是"死代码"，
而是**接线未完成的脚手架**。A4 的实施面比原先估计的小。

---

## 1. 注入点

### 1.1 现有可复用的先例（必须沿用它，不要另造机制）

`reconcile_batch` 与 `adjudicate` 目前标注为"纯函数、无 IO"（`engine.rs:216`、
`adjudicate.rs:201`），但它们**已经接受一个注入的闭包**：

- `ReconcileBatchInput::validate_batch`（`engine.rs:112`），类型
  `&'a dyn Fn(&[Value]) -> Result<(), String>`；
- 由 `run_recognition_cycle_core` 构造（`commands.rs:353-360`），在 `commands.rs:373` 传入，
  再转发给 `AdjudicateInput::validate_batch`（`adjudicate.rs:36`），
  最终在 `adjudicate.rs:198` 被调用。

同一个函数还已经注入了模型闭包：`CloudOutlineRunner<'a> = &'a dyn Fn(&Path, &str, Option<&str>) -> CommandResult<Value>`
（`commands.rs:39-40`），作为参数 `cloud_runner` 传给 `run_recognition_cycle_core`（`commands.rs:315`）。
`cloud_natural_shape_drives_full_reconcile_cycle` 这个测试正是靠注入桩函数跑通全链路的。

⇒ **结论**：沿用例行注射（`Option<&dyn Fn>`），不要给 `engine.rs` / `adjudicate.rs` 引入
`AppHandle`、异步运行时或全局状态——那样会一次性摧毁 671 个测试的确定性。

### 1.2 具体签名建议

在 `ReconcileBatchInput`（`engine.rs:92-113`）新增两个字段，均为 `Option`：

```rust
/// A3：原文件核验的模型通道。`None` = 无可用模型，只做确定性核验。
pub source_verifier: Option<SourceVerifyRunner<'a>>,

/// A4：分歧裁决的模型通道。`None` = 无可用模型，分歧全部留在 `NeedsReview`。
pub adjudicator: Option<AdjudicationRunner<'a>>,
```

```rust
// commands.rs（与 CloudOutlineRunner 并列）
pub type SourceVerifyRunner<'a> =
    &'a dyn Fn(Option<&Value>, &[(String, u32, Option<Value>, bool, String)]) -> CommandResult<Value>;
pub type AdjudicationRunner<'a> = &'a dyn Fn(&[Value]) -> CommandResult<Value>;
```

`source_verifier` 的参数与 `verify_against_source`（`source.rs:248-252`）**逐字一致**
（`document_ir` + `local_slots`），返回值是模型原始 JSON——解析交给
`source.rs` 里的新适配器（对应 `candidate.rs` 的 `cloud_candidate_from_value`），
保持"边界上是 JSON、内部才是类型"的既有约定。

### 1.3 数据流与调用时机

```
run_recognition_cycle_core (commands.rs:309)   ← 唯一的 IO 层，构造闭包、持预算计数器
   └─ reconcile_batch (engine.rs:217)
        ├─ 本地快照 (219) / 云端候选 (231-257)
        ├─ align_cloud_answer_shapes (262)
        ├─ verify_against_source (283)  ──► 【A3 插入点】确定性结论 + 可选模型结论
        └─ adjudicate (289)
             ├─ compare (202)
             ├─ merge_duplicates (208)
             ├─ assign_dependency_groups (209)
             ├─ 【A4 插入点】对 SUBSTANTIVE_DIVERGENCE 项批量裁决
             └─ auto_apply_eligible (212)   ← 必须在裁决之后，否则裁决影响不到自动应用
```

**A4 必须插在 `assign_dependency_groups`（209）之后、资格计算（212）之前**：
裁决会改写 `proposed_patch`，而 `auto_apply_eligible`（`adjudicate.rs:120-182`）用
`answer_compare_key(canonical) != answer_compare_key(local)`（`:169`）与
`answer_compare_key(inferred) == answer_compare_key(proposed)`（`:179`）判定，
只有先裁决才能让"原本待确认"的项变成可自动应用——这正是 A4 的产品价值所在。

**A3 插在 `verify_against_source` 内部**（`source.rs:248`）：确定性结论先产出，
模型结论作为**追加的证据**合并进 `SourceVerificationV1`。

### 1.4 为什么闭包放在最外层而不是 `engine.rs`

`engine.rs:10-11` 的文档承诺"有界、不回退"，预算是 IO 策略而非判定逻辑。
把计数器放在 `run_recognition_cycle_core` 构造的闭包里（用 `Cell<u32>` 配合
`Fn` 的内部可变性），`reconcile_batch` / `adjudicate` 依旧无状态、可确定性测试。

---

## 2. A4 的预算：如何让它真正生效

### 2.1 计数器位置与语义

- 计数器活在 `run_recognition_cycle_core` 构造的闭包里（`Cell<u32>`），
  `adjudicate` 侧只管调用与处理结果，不持有状态。
- **按批次调用，不按项调用**：一份 40 题的卷子，把当批所有分歧项**打包进一次请求**。
  这样"预算 2"是合理的：1 次主裁决 + 1 次受约束修复（对应 `MAX_CONSTRAINED_REPAIRS = 1`）。
  若按项调用，8 个分歧就会超预算 4 倍。
- 单批项数建议再设一个软上限（如 12 项/次），超出部分**留在 `NeedsReview`**，
  不静默丢弃、也不静默超支。

### 2.2 耗尽 / 失败时怎么办（关键：绝不把"没裁"写成"裁过了"）

预算耗尽或调用失败时，被影响的分歧项必须：

| 情形 | `resolution` | `reason_code` |
|---|---|---|
| 预算耗尽 | `NeedsReview`（**不是** `Agreed`/`AutoFixed`） | `ADJUDICATION_BUDGET_EXHAUSTED`（新增） |
| 模型超时/非法输出/不支持 | `NeedsReview` | 复用 `classify_cloud_error`（`engine.rs:68-88`）产出的 `MODEL_TIMEOUT` / `MODEL_INVALID_OUTPUT` / `MODEL_UNSUPPORTED_INPUT` |
| 模型明示无法裁定（`unresolved`） | `NeedsReview` | `ADJUDICATION_DECLINED`（新增），并在 `user_message` 追加"模型未能裁定" |
| 未配置模型（闭包为 `None`） | 与今天完全一致 | 保持 `SUBSTANTIVE_DIVERGENCE` |

`classify_cloud_error`（`engine.rs:68-88`）目前只服务云端链，但它输入是错误字符串、
输出是 `CloudFailure{status, reason_code, message}`，**原样复用于 A3/A4**，不要重写一套分类。

### 2.3 用户如何看见"没裁"

`chains.adjudication` 现在是**硬编码** `StageStatusV1::new(StageStateV1::Succeeded)`
（`engine.rs:305`）——这是必须修的缺陷：无论模型有没有跑过，UI 都显示"裁决成功"。
改为：全部项都有裁定 → `Succeeded`；部分 → `Partial`；无模型/未调用 → `NotRun` 并带原因码。

---

## 3. 输出契约（两个新 LLM 任务）

派发点：`llm_gateway.rs:36-48` 的 `match command_name`，落空返回
`unsupported_llm_gateway_command:{name}`。新增一个任务 = 1 个 match 分支 + 1 个 runner +
1 个 prompt 构造器（惯例见 `llm_suggestions.rs`：`make_llm_input:108`、
`make_vision_transcription_input:166`、`make_vision_answer_extraction_input:181`、
`make_cloud_paper_generation_input:212`）。
参考实现 `run_openai_compatible_cloud_outline_llm`（`llm_gateway.rs:706-788`）：
PDF 走 base64 `type:"file"` 附件（`data_url_for_pdf:559`），回退渲染页图
（`append_pdf_images_to_content:578`），非 PDF 走 `sourceText`（`:718-732`）。

### 3.1 A3：`verify_source_answers`

模型返回：

```json
{
  "findings": [
    { "questionNumber": 14,
      "slotId": "slot-14",
      "verdict": "confirmed",            // confirmed | contradicted | not_verifiable
      "quote": "...原文摘句...",
      "pageIndex": 3,
      "observedValue": {"kind":"text","values":["stencilling"]},
      "confidence": 0.9 }
  ]
}
```

校验器 `validate_verify_source_output`（镜像 `validate_cloud_outline_output`，
`llm_gateway.rs:1191-1330`）：

- `findings` 必须是数组；
- `questionNumber` 必须是 u32；
- `verdict` 必须落在三值枚举内；
- **`verdict == "confirmed"` 必须同时具备非空 `quote` 与 `pageIndex`**——
  没有原文出处的"确认"一律拒绝，这是"不把未核验写成已核验"的第一道闸；
- `observedValue` 若存在，必须是合法 `AnswerValueV2`（`kind` 在已知集合内且具备该 kind 的必需字段）；
- **未知字段一律拒绝**（`deny_unknown_fields` 语义）。既有教训：
  `CloudReadingOutlineV1` 没有 Rust 结构体、也没有 `contracts/` schema，
  事实规范只活在手写的 `validate_cloud_outline_output` 里，是契约漂移的高发区。

### 3.2 A4：`adjudicate_divergence`

输入：JSON 数组，每项 `{decisionId, questionNumber, local, cloud, source}`。

模型返回：

```json
{
  "rulings": [
    { "decisionId": "d:slot:slot-14:answer",
      "chosen": "cloud",                 // local | cloud | source | unresolved
      "value": {"kind":"text","values":["stencilling"]},
      "confidence": 0.82,
      "rationale": "..." }
  ]
}
```

校验器 `validate_adjudication_output`：

- `rulings` 必须是数组；
- **`decisionId` 必须属于本次提交的 id 集合**——模型幻觉出的 id 一律拒绝
  （否则会去改一个根本不存在的决策项）；
- `chosen` 必须落在四值枚举内；`chosen != "unresolved"` 时 `value` 必填且必须是合法
  `AnswerValueV2`；`rationale` 非空。

### 3.3 为什么这个形状能被既有比较代码直接消费

裁决返回的是**完整的 `AnswerValueV2`**，而 `auto_apply_eligible`（`adjudicate.rs:120-182`）
已经通过 `answer_compare_key` 做比较，写入路径也是既有的 `answer_patch`。
因此**不需要第二套适配器**——唯一前提是 §5 的形状对齐。

---

## 4. 如实上报（reason code）

复用为主，新增为辅。现有常量见 `recognition_v1.rs:64-87`。

**复用**：`MODEL_UNSUPPORTED_INPUT`（`:67`）、`MODEL_TIMEOUT`（`:68`）、
`MODEL_INVALID_OUTPUT`（`:69`）、`NO_SOURCE_EVIDENCE`（`:77`，"结论一致但缺原文证据，
不得视为已验证"）、`EVIDENCE_MISSING`（`:79`）、`SOURCE_FILE_UNREADABLE`（`:86`）。

**新增**（追加到 `recognition_v1.rs:87` 之后）：

| 常量 | 语义 |
|---|---|
| `ADJUDICATION_BUDGET_EXHAUSTED` | 裁决预算耗尽，该项**未经**模型裁定 |
| `ADJUDICATION_DECLINED` | 模型明示无法裁定 |

**硬规则**：模型只能把 `Unverifiable` 提升为有证据的结论，**永远不能**把
"没跑模型"折叠成"没问题"。任何 `resolution` 从 `NeedsReview` 变为 `Agreed`/`AutoFixed`
的路径，都必须能追溯到一条模型裁定或一条确定性证据——二者皆无则不得变更。

---

## 5. 形状对齐（本方案最大的技术风险）

`answer_compare_key`（`rules.rs:85-126`）是**形状敏感**的：
`text` → `values.join("|")`；`option` → `opt:LABELS:assignment`。
任何新增的答案来源若不先对齐形状，会在**每一道题**上制造假分歧。

既有实现：`align_cloud_answer_shapes`（`candidate.rs:376-427`），由 `engine.rs:262` 调用。
**已实测有效**：本次定性中，`d:slot:slot-14:answer` 项的 `localValue` 与 `cloudValue`
字节完全一致（均为 `option[B]/per_slot`），而云端原始形态是 `text[B]`——证明重编码生效。

**建议（A3/A4 的前置重构）**：从 `candidate.rs:376-427` 抽出一个纯函数内核

```rust
pub(crate) fn align_answer_value(value: &Value, target_shape: &Value) -> Option<Value>
```

让 `align_cloud_answer_shapes` 调用它，A3 的 `observedValue` 与 A4 的 `rulings[].value`
也调用它。**不要让三个调用点各写一份**——那正是假分歧的温床。

对齐落点（两处，缺一不可）：
- A3 的 `observedValue`：在 `source.rs` 的新适配器里、写入 `SourceVerificationV1` **之前**；
- A4 的 `rulings[].value`：写回 `proposed_patch` **之前**，且以
  `canonical_answer`（`adjudicate.rs:161`）取到的权威稿答案形状为目标形状。

---

## 6. 确定性与可测性

- **现有 671 个测试零改动**：两个新字段是 `Option`，`None` 时走今天的代码路径。
  建议在 `ReconcileBatchInput` 上加构造器或让所有构造点显式传 `None`
  （当前构造点：`commands.rs:362-374`，以及任何测试内构造处）。
- 需要新增的测试：
  1. `align_answer_value` 内核：双向形状转换（text↔option），值相同则对齐后
     `answer_compare_key` 必须相等（单元）。
  2. A3 适配器拒绝畸形输出：`confirmed` 无 `quote`、非法 `questionNumber`、
     非法 `AnswerValueV2`（单元）。
  3. A4 适配器拒绝幻觉 `decisionId`（单元）。
  4. 命令层（沿用 `cloud_natural_shape_drives_full_reconcile_cycle` 的注入写法）：
     - 注入桩裁决器 → 分歧项变为可自动应用且取到裁定值；
     - 预算耗尽 → 该项 `NeedsReview` + `ADJUDICATION_BUDGET_EXHAUSTED`，
       **断言其 `resolution` 绝不为 `Agreed`/`AutoFixed`**；
     - 不注入（`None`）→ 结果与今天逐字节一致（回归护栏）。
  5. 调度层：模型裁决发生在云端完成之后，不得阻塞草稿发布
     （已有护栏 `cloud_fetch_starts_before_local_recognition_finishes_and_draft_published_first`）。

---

## 7. 涟漪效应与风险

**必须改动的文件**：

| 文件 | 改动 |
|---|---|
| `schema/recognition_v1.rs` | 新增 2 个 reason code（`:87` 后） |
| `reconcile/engine.rs` | `ReconcileBatchInput` +2 字段；`chains.adjudication` 不再硬编码（`:305`） |
| `reconcile/source.rs` | A3：接收可选 runner，新增适配器，合并模型证据 |
| `reconcile/adjudicate.rs` | `AdjudicateInput` +1 字段；在 `:209` 与 `:212` 之间插入裁决 |
| `reconcile/candidate.rs` | 抽出 `align_answer_value` 内核（重构 `:376-427`） |
| `reconcile/commands.rs` | 构造闭包、持预算计数器、串联 |
| `llm_gateway.rs` | 2 个 match 分支 + 2 个 runner + 2 个校验器 |
| `llm_suggestions.rs` | 2 个 prompt 构造器 |
| 前端 | 若 `chains.adjudication` 新增 `Partial`/`NotRun`，前端映射必须同步覆盖（既有教训：`CHAIN_STATE_TO_STATUS` 覆盖不全曾把状态渲染成英文原文） |

**风险**：

1. **最高**：形状未对齐 → 每题假分歧。缓解：先做 §5 的内核并配测试，再接模型。
2. 成本：40 题卷子分歧多时费用上升。缓解：按批打包 + 预算 2 + 单项数软上限 12。
3. `adjudicate` 引入闭包后不再是严格纯函数。缓解：文档改为"除注入闭包外无 IO"，
   与既有 `validate_batch`（`adjudicate.rs:36`）一致。
4. **A3 的图像证据面尚不存在**：`verify_against_source` 只有 `DocumentIRV2` 文本，
   无文本时直接返回 `NotRun`（`source.rs:262-285`）。扫描版 PDF 目前**无法**做 A3。
   网关侧有 `append_pdf_images_to_content`（`llm_gateway.rs:578`）可走图像，但那是**额外工程量**。
   建议 A3 首版只做文本，图像留作后续。
5. 不可行项：`adjudicate` / `reconcile_batch` **不能**自己发 HTTP——它们拿不到 profile 与 IO 句柄。
   任何"在函数内部直接调模型"的设计都是错的。

---

## 8. 推荐增量顺序

**Step 0（先做，独立有价值）**：从 `candidate.rs:376-427` 抽出 `align_answer_value` 内核并补测试。
它同时是 A3 与 A4 的前置，且能单独提交、单独验证。

**Step 1（A4，价值密度最高）**：新增 2 个 reason code → `AdjudicateInput` 加 `adjudicator`
→ 网关加 `adjudicate_divergence`（含 `validate_adjudication_output`）
→ `commands.rs` 构造闭包与预算 → 修 `chains.adjudication` 硬编码（`:305`）→ 补 §6 的测试。
只对 `SUBSTANTIVE_DIVERGENCE` 项生效，不动调度、不动 UI 时序，直接兑现"用户只处理少量疑问"。

**Step 2（A3，文本版）**：`source.rs` 接收可选 runner，新增
`verify_source_answers` 与 `validate_verify_source_output`，模型证据**追加**在确定性结论之上，
无文本时如实返回 `NotRun` + `SOURCE_FILE_UNREADABLE`。

**Step 3（可选，较大）**：A3 走图像证据（`append_pdf_images_to_content`），覆盖扫描版 PDF。

---

## 9. 本文的不确定性（照实声明）

- Step 1/2 的**具体代码量**未估算，本文只给落点与契约，未写代码。
- A3 对扫描版 PDF 的可行性依赖图像证据面的新建，本文**未展开**该面的数据结构设计。
- 两个新契约**建议**补 `contracts/` 下的 JSON Schema；现有 `CloudReadingOutlineV1`
  没有 schema，是否同步补齐属于额外决策。
- 全部结论基于源码审读与 `cargo test --lib`（671/0/11），**未**做真实模型往返验证（需凭据），
  **未**做 Tauri UI 级验证。
