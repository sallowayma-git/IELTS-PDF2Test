# 受控模型服务场景（前端可执行）

日期：2026-09-16　　归属：A3/受控候选场景（交接单「Task 27」）

## 1. 这一层的证据是什么，不是什么

| 层面 | 本场景覆盖 | 说明 |
| --- | --- | --- |
| 服务测试 | 是 | `cargo test --lib`，见 `reconcile::commands::tests::controlled_model_service_drives_candidates_through_the_real_gateway` |
| 受控模型服务 | **是（本节主项）** | 真起 HTTP 服务、真发请求，走完整网关解析/校验；结果经比较、裁决、持久化 |
| 真实 UI | 否 | 前端按第 3 节执行即可，但**结果要由前端侧回报**，不能拿本节的机器断言代替 |
| 真实云服务 | 否 | 本服务是固定应答的替身，**不验证**任何真实供应商的鉴权、限流、超时、模型行为差异 |
| 发布到学生端 | 否 | 与本场景无关 |

**边界（有意为之，不是省略）**：本场景不覆盖 `make_cloud_paper_generation_input` 的取原文与拼 prompt 那一段（它依赖真实文件的抽取产物，属于导入链路的职责）。网关收到的输入里 `sourceText` 由调用方直接给出。

## 2. 组成

| 文件 | 作用 |
| --- | --- |
| `fixtures/controlled-llm/reading-outline.json` | 模型的**自然形态**输出（唯一真源）。字段形状对齐 `llm_gateway.rs::validate_cloud_outline_output`：`title` / `groups[]`（`kind`、`layoutHint`、`notesText`、`range`、`questionIds`、`confidence`、`evidence.quotes[]`、`slots[]`）/ `answerKey` / `confidence` / `warnings` |
| `fixtures/controlled-llm/expected-decisions.json` | 期望的裁决结果（投影形态）。实际结果与它不一致时，Rust 用例会失败并提示同步 |
| `scripts/controlled-llm-service.mjs` | 独立可运行的受控服务。把上面的样本包进标准 chat-completions 信封返回 |

样本里 `evidence.quotes[].pageIndex` **必须 ≥ 1**：网关的校验把 `pageIndex = 0` 视为非法（`cloud_outline_group_quote_invalid`）。这是踩过的坑。

## 3. 前端执行步骤

```bash
node scripts/controlled-llm-service.mjs            # 默认 127.0.0.1:11435
```

启动后终端会打印可直接粘贴的 profile 配置。要点：

- `provider = OpenAiCompatible`
- `baseUrl = http://127.0.0.1:11435/v1`　（网关拼 `{baseUrl}/chat/completions`）
- `model = controlled-outline-v1`
- `apiKey` 任意非空字符串，或留空（网关只做 bearer 透传）
- `forceJson = true`

**为什么必须是回环/私有地址**：`llm_gateway.rs::openai_chat_completions_endpoint` 对明文 `http` 有白名单，只允许 `localhost`、`*.local`、回环与私有/链路本地地址，其余一律 `llm_profile_base_url_unsafe_http` 拒绝。服务脚本因此绑定 `127.0.0.1`。

然后：

1. 在应用的 LLM 配置里新增该 profile（或直接写 `<appData>/config/llm-profiles.json`）。
2. 导入一份测试件并选该 profile。受控服务**忽略请求内容**、固定返回样本，所以任何输入都会得到同一结果。
3. 等「本地识别完成 / 待复核」，打开识别面板。

## 4. 预期结果

1 条待确认项：

- 目标 `q14`（`answer` 字段），decisionId `d:slot:slot-14:answer`
- `resolution = needs_review`，`reasonCode = SUBSTANTIVE_DIVERGENCE`，`isActionable = true`
- 本地值 `{"kind":"unresolved"}`（题稿空），云端值 `{"kind":"text","values":["stencilling"]}`
- 来源值为 `null`（夹具稿没有 `sourceAnchors`，原文核验给不出建议）
- `autoApplyEligible = false`：自动补空要求「原文断言的值 == 建议值」，本场景刻意不提供原文证据，因此**不会自动写入**，正好让「接受 / 撤销」按钮可验证

按钮验证：

- 点**接受** → 题稿 `q14` 写入 `stencilling`，该项离开待办，撤销入口出现
- 点**撤销** → 题稿 `q14` 回到空，该项离开「已自动修正」区
- 重要：撤销对**手工接受**的项也应按同一闭环退出

## 5. 与后端 seed 用例的分工

`seed_applied_batch` / `seed_bridge_job` 那类用例把云端结果**直接塞进注入点**，验证的是裁决与状态机；本场景验证的是**网关这一段**（HTTP + `openai_chat_content` + `parse_llm_json_content` + `validate_cloud_outline_output`）。两层证据不可互相代替——这正是 `AGENTS.md` 要求区分「产品行为端到端」与「服务层验证」的落点。
