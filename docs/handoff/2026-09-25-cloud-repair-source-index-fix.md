# cloud-repair 分支头 3 条红灯：根因与修复任务（2026-09-25）

> 给执行 agent（cloud-repair / repair-context-packets 线）：这是质量把关方对 `wave3-listening` 分支头 `f6c49f0` 的复核。
> 工作约定沿用 `docs/handoff/2026-09-22-execution-prompt.md`：先写会失败的测试、亲眼看红再修；频繁提交；**不要 push**。

---

## 1. 现状（质量方本机实测）

- `cargo test --lib`：**1159 通过 / 3 失败 / 11 忽略**。
  1. `cloud_repair::tests::packets_mode_attaches_the_region_image_and_keeps_local_paths_out_of_the_prompt`：确定性失败；
  2. `cloud_repair::tests::an_edit_quoting_an_image_only_answer_page_lands_and_is_marked_unverifiable`：确定性失败（期望 `["A"]`，实际 `["B"]`）；
  3. `cloud_repair::tests::ten_packets_with_fixed_round_delay_finish_within_a_deadline_scaled_to_the_packet_count`：单跑能过（约 5.5 s，时限 7.5 s），全量跑时超时。
- **不是合并引入的**：在合并前的 `5748f45`（你们线自己的提交）上，1 和 2 同样失败。
  你们在 `5748f45` 的记录是「全量 1134/0/11」，这个结果在质量方机器上**复现不了**。

## 2. 根因（已定位，并已用最小改动反证）

`cloud_repair/mod.rs` 里的 `load_packet_source_index` 调用 `auto_pipeline::cloud_source_evidence`，目的只是拿 `sourceFileId` 和 `kind`。
但对 PDF 来说，这个函数会调用 `main_pdf_vision_extraction`，也就是**把整份 PDF 重新渲染一遍**（Python sidecar + pdfium，180 dpi），
然后**覆盖** `<job>/cache/vision/pdf-images.json`。

- 在测试里：`seed_page_images` 写好的页图缓存被一次失败的渲染结果覆盖掉
  （`"pages": []`、`"failureReason": "renderer_pdfium_failed"`）。于是 `page_images` 为空：
  - 测试 1 的区域图变成 `no page image available for this page`；
  - 测试 2 的 `read_page_region` 取不到页图，编辑不落库，答案停在 `B`。
- 测试能不能过，取决于本机的 Python/pdfium 在处理假 PDF 时会不会写出一份失败结果。所以**这些测试不是自包含的**，
  这也是你们的机器是绿、质量方的机器是红的原因。
- 反证：质量方在临时 worktree 里加了一个只读身份的函数（`load_job` + `main_source_for_cloud`，返回 `kind` / `sourceFileId`，
  不渲染），替换掉 `load_packet_source_index` 里的那一处调用后，1 和 2 **都转绿**，测试断言一行没改。

**产品层面的影响**（比测试红更重要）：
- 每次修复运行开头都会把整份 PDF 重新渲染一遍；`evidence_source_text` 里还有两处同类调用。
  真实的 40 页卷子上，这会实打实吃掉修复时限。
- 一旦某次渲染失败，它会把**之前成功生成的页图缓存覆盖成空的**，之后的包模式就再也拿不到区域图，而且没有任何提示。

## 3. 任务

### C1 修复循环不再为了拿来源身份去渲染 PDF
- 新增只读的来源身份函数（不渲染、不写盘），`load_packet_source_index` 改用它。
- 审计 `cloud_repair` 里所有调用 `cloud_source_evidence` 的地方（当前是 `mod.rs` 约 1250、1406、3486 行）。
  只拿身份的全部换成新函数；确实需要页图的地方，**优先复用已有缓存**，缓存缺失时才渲染。
- 测试 1、2 的断言**不许改**：它们表达的需求本来就对。

### C2 渲染失败绝不覆盖好的页图缓存
- `extract_pdf_images_for_vision` 及其兜底路径：先写临时文件，成功后再替换；失败时保留原缓存，并如实返回错误或警告。
- 先红测试：先种一份好的缓存，再让渲染失败（例如喂一份假 PDF），断言缓存原样保留。

### C3 让测试自包含
- 测试 1、2 的结果不能再取决于本机有没有 Python/pdfium，在两种情况下都要成立。
  如果需要，就注入渲染器，或者断言「修复循环根本不触发渲染」。
- 回报中写明：你是在哪种环境（sidecar 可用 / 不可用）下看到它们先红后绿的。

### C4 deadline 用例不再依赖机器负载
- 现在的余量只有约 2 s（11 轮 × 400 ms 纯睡眠 + 开销，时限 7.5 s），全量并行时必然被挤爆。
- 改成不看墙钟的判定，例如注入时钟，或者直接断言「按包数放宽后的时限值」加上轮数。
  **不要只调大数字**；如果坚持用墙钟，要写明理由，并连续 5 次全量跑都通过。
- C1 修好后先复测一次：如果每一轮都在重新渲染，那一部分开销也会消失。

## 4. 边界
- 不放宽网关校验，不改这 3 条用例的业务断言，不重写 golden 基线。
- 不碰听力/发布/门禁区域（`wave3-listening` 的另一条线），也不碰 `customer-delivery/`。
- 不 push。

## 5. 质量方收货标准
- 在质量方机器上连续 3 次 `cargo test --lib` 全量：**0 失败**；vitest、tsc 保持干净。
- C2 的先红测试：回退实现提交后，测试会失败。
- 在修复后的分支头上，质量方复跑阅读真实链（13/13）和听力真实链（12/12），两条都要通过；
  之后才考虑把 `wave3-listening` 合并进 `main`。
