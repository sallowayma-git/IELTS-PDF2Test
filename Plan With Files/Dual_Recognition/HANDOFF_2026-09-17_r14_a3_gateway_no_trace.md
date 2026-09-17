# 交接单：A3 的网关调用只留下输入缓存，既没有结果也无法诊断（R14）

日期：2026-09-17　　提出方：前端 agent　　归属：**后端独占区**（`llm_gateway.rs`、`auto_pipeline.rs`、`processing/scheduler.rs`）
对应 findings：`F-R14-4`　　构建：`937dda5`（A3）+ `d353e9d`（A4），exe 由 `scripts/e2e/build-app.mjs` 产出
复现目录：`artifacts/e2e-cdp/run-controlled-service-2026-09-17T16-12-34-283Z`
复现命令：`node scripts/e2e/tauri-cdp-controlled-service.mjs --tolerate-concurrent-edits --diagnostic-args`

## 0. 一句话

受控服务在线、A3 请求也**确实被构造出来了**（14 个槽位、真实 `pdfPath`），
但这次调用**既没有到达服务，也没有留下调用记录，更没有输出** ——
所以 A3 在真实产品路径上拿不到任何结果，而且失败得不可诊断。

## 1. 三处互斥的事实（同一作业目录）

作业目录：`…/appdata/data/jobs/import-20260917161240-5b4b2e32/`

### 事实 A：输入缓存存在，且请求是真实的

`cache/llm/verify_source_answers-input-1789661600016.json`（mtime 17:13:20）内容要点：

```json
{
  "mode": "verify_source_answers",
  "job": { "jobId": "import-20260917161240-5b4b2e32", "title": "demanding reading passage 3" },
  "sourceFile": { "fileType": "pdf", "originalName": "demanding-reading-passage-3.pdf",
                  "sha256": "f13bd65cb5f5c76a…", "sizeBytes": 212305 },
  "pdfPath": "…\\uploads\\f13bd65c-demanding-reading-passage-3.pdf",
  "profile": { "profileId": "controlled-outline", "baseUrl": "http://127.0.0.1:11435/v1",
               "model": "controlled-outline-v1", "temperature": 0, "timeoutMs": 60000 },
  "slots": [ { "slotId": "q27", "questionNumber": 27, "localValue": { "kind": "unresolved" } }, … 共 14 项 ],
  "outputContract": { "schema": "SourceVerificationV1", "jsonOnly": true, "rules": [ … 7 条 … ] }
}
```

即：`run_llm_gateway("verify_source_answers", …)` 已被进入（输入缓存是它写的，
全仓只有 `llm_gateway.rs:32` 会写这个文件名），请求体完整、对象真实、指向受控服务。

### 事实 B：受控服务**零 POST**

`scripts/controlled-llm-service.mjs:276` 对每个 `/chat/completions` 都打一行
`[controlled-llm] … POST …`。本次运行服务端只收到 **1** 个请求，且是 outline：

```
taskCounts = { "total": 1, "outline": 1, "a3": 0, "a4": 0 }
```

### 事实 C：没有调用记录、没有输出

同目录下**不存在** `llm-calls.jsonl`，也**不存在** `verify_source_answers-output-*.json`。
（`cache/llm/` 里只有那一个 `-input-` 文件。）

## 2. 为什么 C 与代码相矛盾

`llm_gateway.rs:30-79` 的顺序是：

```rust
write_json(&input_path, &redact_llm_input_for_cache(input))?;   // 34 行：写输入
let started = std::time::Instant::now();
let output = match command_name {
    "verify_source_answers" => run_openai_compatible_source_verification_llm(root, job_id, input, api_key),
    …
};
let call_record = json!({ … "ok": output.is_ok(), "errorClass": … });   // 63 行
let _ = append_llm_call_record(root, job_id, &call_record);            // 76 行：**无论成败都记**
let output = output?;                                                  // 77 行
write_json(&output_path, &output)?;                                    // 78 行：仅成功写输出
```

`append_llm_call_record`（76 行）**在** `output?`（77 行）**之前**，且 `append_text`
（`util.rs:297`）会自行 `create_dir_all`，不存在「父目录不存在」这条失败路径。
所以「输入缓存写了、调用记录却没有」只能说明：**执行分支在 76 行之前就没有返回。**

## 3. 两个候选机制（都**未证实**，请你们判定）

1. **`run_openai_compatible_source_verification_llm` panic 掉了所在线程。**
   该分支若在后台线程上执行，panic 只终止该线程，不会崩溃应用，
   也不会执行 76 行 —— 与观察完全吻合。日志目录 `appdata/data/logs/` 是空的，
   所以我这边拿不到 panic 痕迹。
2. **该函数在发出 HTTP 之前永久阻塞**（例如等一把已被别处持有的锁，或读源文件时死等）。
   阻塞会让线程永不返回，同样跳过 76 行。

判据（如果你们要一次区分）：给这个分支加一层 `catch_unwind` 或把
`append_llm_call_record` 移到 `match` **之前**（改成「先记 attempted=true，成功后再补结果」）。
两者任一都能让下一次的痕迹自证。

## 4. 阻塞影响

1. **A3 从未产生结果**：本次运行 `chains.source` 恒为 `not_run`（前端归一成 `not_started`）。
2. **A3 的失败不可诊断**：唯一能说明原因的调用记录缺失。
3. **任务书第 6 条大部分场景不可达**：`a3-partial-not-reported-as-complete`、
   `a3-model-failure-not-reported-as-complete` 两个场景都断言 `chains.source == partial`，
   实测是 `not_started`；`late-model-result-does-not-overwrite-user-edit` 需要批次，也没有。
4. **另有更上游的阻塞**：本次导入**完全没有产出识别批次**（`batchId=null`，四路链全 `not_run`），
   而且 F-R12-2 的既有绕行「播种 + 重试」在 `937dda5` 上**已经失效**——
   重试后仍 `batchId=null`，换派生样本重跑一次仍 `batchId=null`。
   这一条见 `HANDOFF_2026-09-16_r12-2_freeze_before_seed.md`，本轮只是确认它没有好转。

## 5. 前端侧本轮做了什么（供对照，不含后端改动）

- 受控服务已按 prompt 标记分派 A3/A4，并支持 `normal|decline|partial|fail|garbage` 五种模式
  （curl 冒烟已确认分派正确）——**服务侧不是瓶颈**。
- 前端已按线上形状适配四路链状态与「部分完成」文案，并由独立实现的状态断言钉住
  （`verification-status-matches-chains` 是本次唯一通过的场景）。
- 验收脚本原先会把「发起了但没到」误判成「前提不成立」，已改为三态判定并把网关痕迹
  写进 `report.service.gatewayTraces`（见 `F-R14-5`）。
