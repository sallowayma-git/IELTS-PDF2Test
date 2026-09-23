# Findings

## 2026-09-20 source coverage（漏题检测）

- 现有 `recognition::local::task_groups::check_declared_range_present` 只在单个 instruction zone 内比较声明范围与已恢复的 question blocks；它依赖同一条本地识别结果，不能发现“声明题域和 canonical 同时漏掉同一道题”。
- `ielts_grammar::quality::evaluate_quality_with_gate` 已是导出/发布共用的质量事实入口，并接收独立的 `DocumentIRV2` physical shadow；source coverage 应在此处加入独立题域比较，不能从 `taskGroups` 反推 source declaration。
- `DocumentIRV2.pages[].lines[].text` 是当前产品路径可用的原文证据面。`question_number::parse_question_expression_detailed` 已支持普通 `Questions 1-13`、短横线、`to`、`and`/混合范围，但其边界检查会拒绝 `questions1-10`，需要先写红测再收紧修改。
- 规则设计：从原文行中收集 `Questions ...` 和 `Write your answers in boxes ...` 声明，union 后与 canonical `answerSlots[].questionNumber` 比较；无法可靠解析任何声明或遇到声明式文本无法解析时返回 `Undetermined`，绝不返回完整覆盖。声明可解析且存在缺题时返回 blocking 缺口。
- 实现落在 `ielts_grammar::quality::evaluate_quality_with_gate`：报告新增 `questionCoverage`，缺题写入 blocking issue，无法判定写入 warning；显式 `modality=listening` 暂不套用阅读规则。
- 红测先证明删除 canonical q15 会得到 `missing=[15]` / `SOURCE_QUESTION_COVERAGE_MISSING`，不可解析声明只能得到 `undetermined`；另覆盖 `questions1-10`、独立 answer-box 声明和空 `lines` 回退 `spans`。
- 真实 CDP 链第一次运行暴露了 pdfium 的实际形状：多位数字被抽成 `2 7` / `3 1`，导致误报缺题。新增反例先红后修，在 coverage 声明入口合并相邻数字间字形空格；修复后同一 PDF 的 `questionCoverage` 为 `complete`，题号 `27..40` 闭合，13/13 场景通过。
- 本轮全量 Rust `880 passed / 0 failed / 11 ignored` 是数字间空格修复前的结果；最终新增该反例后应以最新复跑数字为准，不能沿用这条旧数字。

## 2026-09-14 第三轮精简确认 verifier 的技术发现（跨仓）

第三轮派 3 个只读 verifier（发布/打包完整性、学生端提交校验、编辑器与云端 UX）复核第二轮
修复。**三条修复被证伪或部分证伪**，本轮已全部修复；以下记录技术机理与裁定依据。

### 1. 提交点判定用了一个「提交后就不存在」的文件（P0，`6b6ac8e`）

- `nas_package_v2.rs` 的 `atomic_replace_file(&release.join("manifest.js"), &paths.manifest_path)`
  是**移动**语义（`MoveFileExW`），提交成功后 `releases/<batchId>/manifest.js` 已不存在。
- 而 `manifest_matches_release` 正是拿 live manifest 与这个文件比对 → 恒为 `false`。
- 于是「清单已替换、`manifestCommitted` 尚未写盘」这个崩溃窗口被误判为**未提交**：
  `recover_interrupted_batches` 会 `remove_dir_all` 掉刚上线的 `resources/<examId>`、
  用备份还原、再 `restore_manifest_baseline` 退回旧清单——**毁掉一个已经生效的批次**。
- 修复：状态文件在破坏性步骤**之前**记录候选清单的 sha256（`pendingManifestSha256`），
  恢复端用**线上清单的内容哈希**判定提交点。
- 附带修掉一个测试缺陷：原测试用 `fs::write(release/manifest.js, candidate)` 造了一份**副本**，
  恰好让「提交后 release 里没有清单」这一真实情况消失，因此 bug 在测试里不可见。
  现改为复刻生产的移动语义，并断言移动后 release 里确实没有该文件。

### 2. `JSON.stringify` 对超过 2^53 的整数会舍入（P1，`84bc3f0`）

- JSON 只有 double 一种数字类型。`JSON.stringify(9007199254740993)` → `"9007199254740992"`。
- `js_number_to_string` 对 i64/u64 走了「精确位数」捷径，绕过了已经实现好的
  ECMAScript `Number::toString`，于是 Rust 侧算出的 `runtimeSha256` 与学生端复算值不同。
- 可达性：契约里没有接近 2^53 的整数字段（`byteLength` 等远小于），因此在当前真实包上
  **不可达**；但公开命令接受任意 `source_path`，且这是编码器语义错误，按正确性修复。
- 期望值取自真实 Node（`JSON.stringify(JSON.parse(<字面量>))`），不靠推断：
  `18446744073709551615` → `"18446744073709552000"`，
  `123456789012345678901234567890` → `"1.2345678901234568e+29"`。

### 3. 「槽位交互 ↔ 答案键类型」只检查了有内容热点的槽位（P1，`6741fb4`）

- 学生端 `reading-v2-loader.ts:495-496` 对**每个**槽位强制：文本槽位必须配文本答案，
  非文本槽位必须配选项答案；不一致时抛 500 → **整份提交失败**（整卷不可提交）。
- 生产端只在 `hotspot_issues` 里检查绑定到内容热点的槽位，`diagram_hotspot`/`composite`
  等没有内容热点的槽位漏检 → 可以发布一个学生端必然拒绝的包。
- 修复：在 `validate_reading_source_v2` 里对**全部** `answer_slots` 逐槽位比对，与学生端
  规则一一对应，并给出 `RUNTIME_TEXT_SLOT_ANSWER_NOT_TEXT` /
  `RUNTIME_CHOICE_SLOT_ANSWER_NOT_OPTION` 两个码。
- 顺带发现既有夹具 `text_entry_response_and_text_answers_do_not_require_an_option_bank`
  把响应组改成 `TextEntry` 并给了文本答案，却没改槽位的 `interaction`——它构造的正是
  学生端会拒绝的状态。夹具已修正为自洽。

### 4. 预检可以给一个永远发不出去的条目报「通过」（P1，`f8a48be`）

- `get_publish_preflight_core` 在条目没有权威稿时回退到 job 目录草稿，仍可能返回 `passed`。
- 但发布路径硬要求权威稿，否则 `ITEM_DS_NOT_SEEDED`。界面因此会出现「绿色 + 发布必失败」。
- 修复：预检在无权威稿时直接返回同一个 `ITEM_DS_NOT_SEEDED` 阻断。

### 5. 提示被保存循环吃掉 = 静默数据丢失（P0，`ed5e75a`）

- `recoverFromConflict` 先 `setSaveMessage("…另有 N 项未能应用…")`，紧接着 `await persist()`；
  而 `persist()` 的每次循环迭代都会 `setSaveMessage(undefined)`，最后 `setSaveState("saved")`。
- 只要 `applied.length > 0`（几乎总是），用户**只会看到绿色「已保存」**。
- 更严重：`checkpoint()` 写进 localStorage 的 `pending` 只有 `applied`，未能应用的补丁
  **连恢复草稿里都没有**，无法找回。
- 修复：提示改走独立的、保存循环不触碰的 `saveNotice` 状态（需用户手动关闭），
  并把「何时必须有提示」抽成纯函数 `conflictRecoveryNotice(applied, dropped)` 以便断言。
- 前端没有 React 测试环境（无 testing-library/jsdom），因此**没有** hook 级测试；
  证据层级是「纯函数单测 + 代码审查」，如实登记。

### 6. 真实 Tauri E2E 的两个真实根因（`4890606` + 未解决）

- **参数**：`--disable-gpu` 与 `--disable-software-rasterizer` 同时存在 = GPU 与软件光栅化
  全部关闭，渲染进程没有可用绘制后端，WebView2 会话建立即崩。只留
  `--no-sandbox --disable-gpu` 后会话存活。
- **构建模式**：`cargo build` 产出的是 **dev 模式**二进制，按 `tauri.conf.json` 的
  `devUrl` 加载 `http://localhost:1420`；dev server 未启动，页面根本没加载
  （探针观察到 `url=about:blank`、`bodyLen=0`）。产品 E2E 必须用
  `npx tauri build --debug --no-bundle`（内嵌 `dist`）。
- **仍未解决**：内嵌构建下应用进程与 2 个 WebView2 子进程独立运行时稳定存活，
  但被 tauri-driver 驱动时第一个命令即断。驱动/运行时版本匹配，`switchTo` 无关。
  这是 driver/WebView2 会话层问题，不是产品断言失败。

## 2026-09-14 六 verifier 复核后的裁定与修复（跨仓闭环）

第二轮 6 个只读 verifier 的结论已逐条裁定。以下记录「裁定 + 处置 + 证据层级」，避免把
verifier 的推断当成事实，也避免把环境阻塞当成产品缺陷。

### 已确认为真实产品缺陷并修复（product/service 级证据）

1. **V6 P1-3 `runtimeSha256` 哈希操作数不是学生读到的字节**（PDF2Test `958e7e3`）
   - 原实现 `serde_json::to_value(source)` 重序列化后哈希，而 wrapper 里嵌的是磁盘原始
     `source_value`；`skip_serializing_if` 字段显式为 `null` 时两者字节不同。
   - **可达性裁定**：默认 export→publish 链上 `runtime_value` 也是 `to_value(&runtime)`，
     因此二者字节相同，此缺陷在该链上**不可达**；但公开命令接受任意 `source_path`，
     该路径上可达。按防御性正确性修复：改为对 `source_value` 编码。
   - 证据层级：service/命令级（Rust 单元测试 + 探针门禁）。

2. **V6 P1-2 发布门禁不校验 checksum（假阳性）**（PDF2Test `958e7e3`）
   - `run_student_loader_probe` 只查资源与语义，从不复算 `runtimeSha256`/`scriptSha256`/
     `assetManifestSha256`，因此可以对一个学生端会以 `reading_source_integrity_failed`
     拒绝的包报 `passed` —— 正是本轮真实踩到的缺陷类型。
   - 修复：新增 `run_student_loader_probe_with_files`，从落盘 `<examId>.js` 中提取
     `__READING_EXAM_DATA__.register` 的 payload，用 ECMAScript 规范编码复算并比对；
     `scriptSha256`/`assetManifestSha256` 按文件字节复算。
   - 证据层级：service/命令级。

3. **V2 F1/F2/F3 批量发布崩溃窗口**（PDF2Test `958e7e3`）
   - F3（数据丢失）：状态文件在 `rename` **之后**才写，且恢复/回滚会**无条件**删除线上
     `resources/<examId>`；崩溃在两者之间就永久删掉旧资源。
   - F1：清单替换后状态写失败会触发回滚，删掉刚生效的资源，留下「新清单 + 无资源」坏包。
   - F2：从不备份 `manifest.js`。
   - 修复：状态先于破坏性移动写入；回滚与恢复都改为「备份存在才动线上目录」；提交点用
     live manifest 与 release 候选清单的**字节比对**判定（不依赖状态文件写入时机）；
     提交点之后不再回滚资源；还原失败时保留备份交给下次恢复重放；并保留清单字节基线。
   - 证据层级：service/命令级（4 个定向单元测试覆盖三个窗口 + 备份感知回滚）。

4. **V3 F1 多槽 `per_slot` 组被整组求和误判超限 → 整卷 500**（学生端 `0b7b3d4`）
   - 生产者为 `per_slot` 组按槽位写 `{min:1,max:1,exact:1}`；学生端把整组标签求和后与
     `max=1` 比较，任何多槽 per_slot 组都会超限。
   - 裁定：`unordered_set` 是整组共享标签池（生产者保证 `exact == 槽位数`），
     `per_slot`/`ordered_slots` 是逐槽约束。改为按 `assignment` 分派校验。
   - 证据层级：跨仓静态套件（phase6 + vertical-slice）。

5. **V3 F2 / V6 P2-4 热点槽位用文本答案 → 可点但永远交不上卷**（PDF2Test `958e7e3`）
   - 学生端对 `interaction != 'text'` 的槽位要求答案键是 `option`；生产者却接受 text
     答案键（`normalize_runtime_hotspots` 还会把 hotspotId 改写成文本值，使值匹配看似通过）。
   - 修复：编译期新增 `RUNTIME_HOTSPOT_ANSWER_NOT_OPTION`。
   - 证据层级：service/命令级。

6. **V3 F3 「同一字母可用多次」的匹配题被误判重复**（学生端 `0b7b3d4`）
   - 重复选项只在确实不允许复用时才算错（响应组 `allowOptionReuse` 或任务选项库
     `allowReuse`）。`buildReadingV2InteractionModel` 增补 `optionBankAllowsReuse`。
   - 证据层级：跨仓静态套件。

7. **V3 F10 / V6 P0-1 空槽位让整卷提交 500 且无回执**（学生端 `0b7b3d4`）
   - `createReadingV2PracticeSubmission` 传 `requireComplete: true`，任何未作答评分槽位都会
     让 `/api/exam/final-submit` 抛 500 且不写回执，学生被锁死。V1 路径一直是按答错计分。
   - 修复：终局提交改为 `requireComplete: false`（空槽计错、正常出回执）；学生输入类校验
     改为 HTTP 400（可被前端呈现），只有「已发布包自身不一致」保留 500。
   - 证据层级：跨仓静态套件；真实 Electron E2E 仍为全对作答，空槽路径**尚无产品级 E2E**。

8. **V4 P0 保存冲突死路**（PDF2Test `440ce94`）
   - `reload()` 先 `persist()`，失败被吞掉 → 刷新从未发生；恢复记录又把同一个过期 draft
     载回来重跑同一次失败保存；返回/发布都先 flush，静默失败；重启应用无效。
   - 修复：`rebasePendingPatches` 纯函数把未保存补丁按序重放到服务端最新版本（遇到无法
     应用的补丁就停下并报告条数）；`recoverFromConflict` / `discardLocalChanges` 两个明确
     出路常驻在保存状态为 conflict/failed 时。
   - 证据层级：pure unit（3 例）+ tsc；**真实 Tauri 双写者场景未做产品级 E2E**。

9. **V4 P1 编辑器问题列表 ≠ 发布门禁**（PDF2Test `d274162`）
   - 编辑器只有纯前端近似检查，会出现「界面 0 问题、点发布失败」且文案泛化。
   - 修复：新增只读命令 `get_publish_preflight`，复用发布路径同一个
     `check_publish_preflight`，并把 blocker/warning 并入同一问题列表（保留后端
     `userMessage`，按 code+targetId 去重）。
   - 证据层级：pure unit（4 例）+ service（命令复用同一函数）。

10. **V5 P1 云端候选无修订绑定 + V1/V2 分裂**（PDF2Test `baa0965`）
    - `apply_llm_suggestion` 只写 V1 `authoring-ir.json`；条目一旦有 V2 权威稿，
      `migrate_single_item` 直接返回、从不回读 V1 → 接受建议**静默丢失**。
    - 修复：新增 `apply_llm_suggestion_core_with_version`，写盘前守卫：
      版本不一致 → `LLM_SUGGESTION_STALE`；已有 V2 权威稿 → 显式拒绝
      `LLM_SUGGESTION_AUTHORITATIVE_STORE_IS_V2`（把静默丢失变成可诊断拒绝）。
    - **未交付**：候选审阅 UI（diff + 逐题选择）。云端 API 函数自审阅页删除后**没有 UI 调用方**，
      接受路径在产品界面上不可达；本次只把后端做成安全的，没有新增表面。
    - 证据层级：service/命令级（1 个 Rust 测试）+ pure unit（3 例文案）。

### 判定为「不需要修复」或「非产品缺陷」

- **V1 P2 键序排序差异**：Rust 按 UTF-8 字节序、JS 按 UTF-16 码元序。契约键全为 ASCII 时
  等价；出现非 BMP 键才会分歧。当前所有契约字段名均为 ASCII，判定为**暂不修复**，
  作为已知约束记录（若将来引入非 BMP 键需重新评估）。
- **V2 环境阻塞**：`npm run build:server` 与 vite `emptyOutDir` 会递归删除目录，被沙箱
  `safe-delete` 拦截。这是**环境限制不是产品缺陷**；已用 `tsc` 直出 + `vite --emptyOutDir=false`
  预构建，并给四个跨仓脚本加 `PHASE6_SKIP_BUILD=1` 预构建跳过开关（默认行为不变）。
- **V2 F2 清单备份**：`atomic_replace_file` 在 Windows 用 `MoveFileExW`
  （`REPLACE_EXISTING|WRITE_THROUGH`）、Unix 用 `fs::rename`，都是原子替换；加上提交点按字节
  比对判定后，清单不会被半写坏。仍保留一份字节基线作为恢复兜底。
- **`better-sqlite3`**：并非缺陷。Electron ABI 下正常建库与迁移成功；真正阻塞 E2E 的是
  Python 解释器选择、GPU 参数与沙箱删除限制。
- **V1 旧路径仍泄漏 `answerKey`（V6 P2-5/P3-9）**：V2 边界已确认无泄漏（payload 与
  `runtimeSourceV2` 均无 answerKey）。V1 legacy 路径的不透明性与 `schemaVersion` 判定
  属既有历史行为，本轮**未改**，如实列为残留风险。

### 复核后仍未闭环（如实登记）

- P4 云端候选审阅 UI（diff 呈现 + 逐题采纳）**未实现**，云端接受路径在产品界面上不可达。
- 编辑器「学生预览」表面（`ExamCanvas mode="student"` 无调用点、
  `buildReadingSourceV2FromAuthoring` 无导入方）**仍未接线**，因此「编辑器 == 学生」目前
  只能靠跨仓静态套件 + 真实 Electron E2E 间接证明，缺少「同一 UI 内对照」的直接证据。
- 热点重命名是破坏性两段补丁、资源预览缓存不失效、插入槽位用全局 max+1 等 P2 未处理。
- 真实 PDF/DOCX 从 Tauri UI 到发布的同一轮闭环仍是缺口（本轮 E2E 起点是真实发布产物）。

## 2026-09-13 产品可用性诊断 / WYSIWYG / 云端校验

- 用户目标不是增加更多“Phase 页面”，而是一个转换工具：导入 PDF/DOCX -> 自动识别为学生可作答 GS/考试题目形态 -> 同一真实练习界面中查看与编辑 -> 云端校验以可理解方式呈现差异 -> 用户确认后导出。
- 规划文档将目标架构收敛为 Library / Workspace / Settings 三个主表面，以 `IeltsAuthoringIRV2` 为基础的 Canonical Exam DS 为唯一权威内容；runtime/JS/NAS 仅在发布时编译。
- 规划文档 2026-09-12 自审计显示：产品壳层/三路由/题库任务态/工作区直接编辑已大体落地；仍未交付或只部分交付的核心是 DocumentIRV2 geometry-first 主识别链、完整 Cloud Recognition Candidate、cloud evidence resolver/group salvage/reconcile、EditorCommand 全量覆盖、过程文件清理和跨实际学生 renderer 的 parity 证明。
- 用户当前“看起来已经成型但可用性很差”的反馈，初步与计划文档自身识别出的未完成核心一致：UI 表面简化并不等于识别闭包、云端校验呈现、编辑反馈与最终学生端一致性已经闭环。
- 计划把真实产品体验定义为“本地草稿一旦可用即可打开最终题面；云端结果迟到时只增量呈现局部差异；用户离开工作区后后台任务继续”。这意味着可用性不能依赖一个阻塞式“全部生成完再进入编辑器”的流程。
- WYSIWYG 的关键不是直接编辑生成 JS，而是：前端内存 DS 即时更新，约 450ms debounce 发送 `EditorCommandV1`，Rust 事务更新 canonical DS/editVersion，发布时再编译 JS/NAS。需要重点核查当前实现是否真正遵守这条链，而不是只“看起来可编辑”。
- 云端目标 UX 已明确：新增建议绿底、删除建议红色删除线、结构冲突定位到题组 popover；绝不再建独立 LLM Review 页。用户编辑过的节点应进入 proposal-only 保护，迟到 cloud result 不能自动覆盖。
- 规划对发布门禁的产品定义很重要：只看当前 Canonical DS、当前 actionable blockers、当前 compiler 和资源闭包；不应因历史 fallback/partial 日志永久禁止发布，也不应把 schema/hash/CAS 暴露给用户。
- 规划 2026-09-12 校正还显示：ExamCanvas 拆分仅部分落地（当时 3/11 组件）、EditorCommand 仍不完整、后端逐文件重构几乎零推进。这提示“前端已收敛”可能掩盖底层仍由旧链驱动的产品割裂。


## 2026-06-06 Settings Preflight + 100-PDF Live Regression

- 设置页当前 `settings-grid` 把预检、模型列表、新建配置、高级诊断同时排成 3 列；预检项又复用 `.layer-list div` 大卡片样式，导致依赖排查信息在 warning 较多时显得臃肿。
- 预检报告已有足够结构化字段：`ok/errors/warnings/checks[]`，前端可以直接按 severity/ok 归类并默认展示需要处理的项。
- `/Users/maziheng/Downloads/0.3.1 working/ReadingPractice/PDF` 当前有 262 份 PDF，足够抽样 100 篇。
- `ReadingPractice` 目录没有 legacy generated JS 对照文件；现有 `scripts/pdf-regression-sample.mjs` 需要 `--legacy-dir`，不能直接用于这批 PDF 的 100 篇测试。
- CLI 已有 `--run-auto-pipeline`，会创建临时 app root、保存临时 LLM profile 并调用 `run_auto_pipeline_core`，适合作为 100 篇真实 PDF Live smoke 的调度目标。

## 2026-06-06 Windows Package and Compatibility

- `Files/Windows包体与兼容规划.md` 明确第一版 Windows 目标：TXT/MD、文字层 PDF、DOCX、LLM 辅助、导出和 Pack 构建不依赖 Node/Python/OCR；扫描 PDF 继续走云端 vision/人工确认，不打包 Tesseract 或 Python runtime。
- 当前根目录已有 `task_plan.md`、`findings.md`、`progress.md`，但内容属于上一轮 PDF/LLM 导入目标；本轮已在顶部追加当前 Windows 包体与兼容目标，保留旧历史。
- 仓库启动时已有 dirty worktree：多处 Rust、前端、脚本和原 Windows 规划文件已修改。本轮必须按文件范围隔离，不能 revert 既有用户/历史改动。
- 当前 `scripts/package-audit.mjs` 只审计 macOS `.app/.dmg`，且使用 `new URL(...).pathname`，Windows 会有 `/C:/...` 路径风险。
- 当前 `package.json` 的 `verify:release` 固定串联 macOS DMG repack，Windows 构建会失败。
- 当前 Rust `parser.rs` 写死 `python3`，`render_pdf_pages_with_adapter` 实际固定落到 macOS `sips`。Windows 下需要 Python resolver 和结构化 renderer unsupported/manual-review 降级。
- 当前 `environment.rs` 固定探测 `python3`、`pypdf` 和 `renderer:macos-sips`，Windows 预检文案会误导用户。
- 当前 `sidecars/ui-flow-e2e/ui-flow-e2e.mjs` 使用 `new URL(...).pathname`、直接 spawn `npm`，且 Chrome 查找缺 Windows 默认路径。
- 当前 `scripts/pdf-regression-sample.mjs` 默认数据路径绑定本机 macOS 目录，Windows/CI 需要显式传参或 fixture 默认。
- 实现完成后，release/audit 已拆成 macOS/Windows 独立入口；Windows audit 会输出 WebView2 模式、artifact size/SHA-256、git/lockfile 摘要，并阻止默认包体混入 Node/Python/Tesseract/OCR/PDFium。
- Rust runtime 已支持 `EPIC8_PYTHON`、平台 Python 候选、`EPIC8_PDF_RENDERER`、结构化 renderer unsupported/manual-review 降级，以及平台化环境预检。
- Windows 签名/分发已落地为说明文档、`audit-windows-signatures.ps1` 和 Windows smoke workflow；本机 macOS 无法执行 `pwsh` 或生成真实 NSIS/MSI。

## Current State
- Prior implementation added one-click editable draft generation, overwrite protection, inline completion layout hints, dev PDF/DOCX parsing, CLI generation, batch picking, and a random regression script.
- Known high-risk areas: no-space ellipsis blanks such as `11………`, answer-key extraction from scanned answer pages, sentence-ending matching classification, and post-import routing.
- Margaret Preston `Questions 8-13` now renders as one inline completion group with `q8` through `q13`; no `Questions 8-13 item 11` placeholders remain in the generated source.
- Random regression smoke sample structure passed, but answers failed when answer pages had no extractable text; report now classifies that as `answer_pages_have_no_extractable_text`.
- Latest 30-PDF regression report uses `comparison.groupShapeIssues`; high-frequency mismatches were dominated by: old JS table-rendered paragraph matching, old JS list-rendered completion layouts, single-choice groups misclassified by broad `according to`/`match`/`two` triggers, and `Questions 27 and 28` ranges dropping the second question.
- Production classifier should be semantic-first: paragraph/section matching stays matching even when old JS rendered it as a table; single-choice requires explicit A-D option evidence; multi-choice requires explicit `Choose TWO/THREE letters`.

## 2026-06-04 Silent LLM Chain
- User explicitly disallowed simulated API tests. Vision answer extraction and cloud comparison must be validated only through the real OpenAI-compatible API; if the API cannot run, mark that test as not run instead of substituting mocks.
- Current ordinary artifact minimization removes `pipeline-report.json`, so any user-visible cloud/answer warning must also be persisted into `authoring-ir.audit.issues` or the editable draft page will not see it after minimization.
- Existing `nextRoute=document` for source review explains why generated jobs can land on the first recognition page; the default route should remain `groups` once `authoring-ir.json` exists, while source-review warnings are surfaced inside the editable draft.
- Existing PDF vision fallback can degrade to macOS `sips`, which only renders one preview image. This is insufficient for answer pages at the end of IELTS PDFs; full-page rendering via PyMuPDF/Poppler should be attempted before `sips`.
- With the new API profile, the real Margaret Preston run confirms the intended split: local generation keeps `Questions 8-13` as one inline completion group and vision answer extraction fills the answer key. The cloud model instead labeled the range as `summary_completion` with list layout, so the quality gate correctly marked the job as needing confirmation without overwriting the local draft.
- The provider rejected direct PDF file input with HTTP 400 `file type is not supported`; image fallback is therefore required for this compatible endpoint.
- UI e2e still has an old assertion named `ocr source review route`; it should be updated to expect the editable draft page while preserving visible confirmation state.
- Latest random PDF sample has three structure mismatches; inspect `tmp/pdf-regression-sample/report.json` before deciding whether they are normalizer gaps or classifier bugs.

## 2026-06-06 Cloud/Vision UX Follow-up
- The observed "stuck at 正在生成 1/6" state is a missing user-facing phase indicator, not necessarily a deadlock. `runAutoPipeline` is a blocking call while it performs vision answer extraction and cloud whole-paper comparison, so the import page must show that the cloud model is running.
- `pipeline-report.json` is removed by ordinary process minimization. Any user-visible cloud/vision state must be copied into `authoring-ir.audit.issues` before minimization; otherwise the editable draft page loses the distinction between local output, vision answer extraction, and cloud comparison.
- The Rust LLM gateway already supports `extract_pdf_image_answers` and `generate_pdf_reading_outline`. The Node sidecar did not, so development/diagnostic parity needed explicit command support.
- The export failure pasted by the user is publish-readiness behavior: empty answers, source-review parser warnings, low-confidence questions/groups, and missing human verification are supposed to block export. The fix is clearer guidance plus surfacing vision/cloud evidence, not bypassing publish readiness.
- PDF image extraction now intentionally tries full-page renderers before falling back to `sips`, so tests should not assume a single rendered preview image.
# 2026-09-01 AI 能力完整性审计

- 审计范围：真实 Tauri UI、Rust command handler、PDF/DOCX ingestion、LLM gateway/sidecar、authoring artifacts、editor/student preview、publish gate、runtime 和相关测试。
- 重点能力：AI 生成、AI 核验、AI 补全，以及模型/工具调用的结构化约束、可观测性、失败降级和人工确认。
- 初始已知：历史记录提到本地生成、视觉答案补全和云端整卷对照已有实现，但需要验证它们是否覆盖 UI 到导出的完整路径，及 Node/Rust 工具调用是否一致。

## 2026-09-02 主线程复核与有效探针证据

有效的网关探针确认以下事实（其余五个探针因上游代理 502 终止，未作为证据）：

- `src-tauri/src/llm_gateway.rs:23-29` 将所有命令路由到 OpenAI-compatible 实现；`src-tauri/src/llm_gateway.rs:88-99` 固定拼接 `/chat/completions` 并只读取 `/choices/0/message/content`。设置中的 Anthropic/Ollama/Custom 类型没有对应协议分派。
- `src-tauri/src/llm_gateway.rs:102-114` 在严格 JSON 失败后截取首个 `{` 到末个 `}`；`src-tauri/src/llm_gateway.rs:462-505` 和 `:691-795` 对缺失字段默认补齐/归一化，未执行 required/additionalProperties、置信度范围或输出 schema 拒绝。
- `src-tauri/src/llm_gateway.rs:117-144` 只对少量字符串化 HTTP 错误重试三次，未覆盖 429/408/500、Retry-After 或模型 JSON/schema 失败；`:402-428` 会把所有 direct PDF 错误都转为图片 fallback。
- `src-tauri/src/llm_gateway.rs:304-318`、`:330-345`、`:348-374` 的请求没有 `tools`/`tool_choice`，响应没有 `tool_calls`/审批/执行；当前不存在受控工具调用闭环。
- `src-tauri/src/llm_gateway.rs:159-170` 把响应正文直接拼入错误；`src-tauri/src/llm_gateway.rs:18-33` 失败时没有统一 error/run record。`src-tauri/src/cleanup.rs:121-148` 和 `:180-207` 默认清除 `cache`、`llm-suggestions`、`llm-calls.jsonl`、视觉原始输出，AI 证据无法长期追溯。
- `src-tauri/src/llm_commands.rs:149-176` 手动题组调用失败时直接降为 deterministic suggestion，且没有把 fallback/失败作为结构化 run 状态；`:199-213` 只按置信度和有限白名单阻止应用。`src-tauri/src/auto_pipeline.rs:1626-1657` 只记录视觉答案摘要，需继续确认其候选是否写回和人工接受入口。
- `src-tauri/src/llm_suggestions.rs:545-574` 只验证 evidence block ID 属于题组且 quote 非空，不验证 quote 是否真实存在于源块；`:458-470` 对 layout template 仅检查非空字符串。

### 初步缺口矩阵

| 优先级 | 能力/缺口 | 影响 | 证据 |
|---|---|---|---|
| P0 | AI 工具调用尚未实现；没有工具 schema、白名单、参数校验、审批、执行结果和审计 | 无法安全接入“AI 调用核验/生成/补全工具” | `src-tauri/src/llm_gateway.rs:304-345` |
| P0 | 视觉答案候选的生成、接受/拒绝、写回、revision/audit 闭环不完整 | 扫描答案可能只停留在诊断文件，用户无法逐项确认 | `src-tauri/src/auto_pipeline.rs:1626-1657`，待主线程核验 |
| P1 | provider 配置与实际请求协议不一致 | Anthropic/Ollama 配置可保存但运行失败或误发请求 | `src-tauri/src/llm_gateway.rs:88-99` |
| P1 | AI 生成输出 fail-open，缺字段默认补齐，JSON 外围文本被接受 | 模型幻觉/畸形结果进入 suggestion 或比较结果 | `src-tauri/src/llm_gateway.rs:102-114`, `:462-505` |
| P1 | endpoint 没有 scheme/host/私网/重定向约束 | 文档内容和密钥可被发送到任意地址 | `src-tauri/src/llm_commands.rs:74-88`, `src-tauri/src/llm_gateway.rs:49-56` |
| P1 | AI 核验没有持久化任务/逐项 diff/证据审阅和导出门禁契约 | 核验状态重启丢失，用户难以判断是否可发布 | `src/pages/UnifiedPreview.tsx:333-499`, `src-tauri/src/cleanup.rs:121-148` |
| P1 | AI 补全没有统一候选状态和人工决策记录 | 无法区分生成、建议、接受、拒绝和最终权威答案 | `src/types/ielts-authoring-v2.ts:230-237` |
| P2 | 重试、错误分类、调用耗时/request id/输入输出 hash 不完整 | 失败不可诊断，限流/认证错误可能被误判为降级 | `src-tauri/src/llm_gateway.rs:117-171` |
| P2 | 测试主要覆盖 OpenAI happy path，缺 provider/工具/负例/真实 UI | 关键安全和产品闭环无回归保护 | `src-tauri/src/lib.rs:1736`, `:2218-2285`, `:3159` |

## 2026-09-02 主线程第一轮模块地图

- 6 个重新派发的只读探针均在约 10 分钟内未返回；已关闭，不能作为审计证据。主线程按关键源码和产品路径复核。
- AI 入口与产品面至少分布在导入向导、统一预览、V2 结构化编辑器、设置页和导出页；背景云端复核由 `UnifiedPreview.tsx` 的队列/调度器触发，需确认它是否覆盖真实 Tauri 导入路径以及结果是否回写持久化 artifact。
- AI 逻辑存在 Rust 网关、Rust command handler、Node sidecar 和历史/诊断脚本多套实现；必须检查协议、输出约束和错误语义是否一致，不能只依赖 CLI/sidecar 的通过结果。

## 2026-09-02 六路并发审计结论（网关/编排/视觉落盘/前端/测试/产品要求）

### 执行流与并发（最重要）
- 产品唯一 runAutoPipeline 调用点 `src/pages/ImportWizard.tsx:162` 硬编码 `executionMode:"localOnly"` 且不传 profileId → 真实 Tauri 后端下**从不发起任何 LLM/云端调用**（`auto_pipeline.rs:1178-1179` cloud_diagnostics_opted_in=false）；后台云复核队列因 profileId=null 永不入队（`UnifiedPreview.tsx:475-476`），前端文案「云端复核自动转入后台队列」在真实后端是空头支票；dev fallback 后端掩盖了这一点（浏览器 e2e 全绿）。
- full 模式内部纯顺序：解析→视觉识别→视觉答案→split→草稿→逐组 LLM 循环→云端 outline（`auto_pipeline.rs:1678-1705`，此刻才发网络调用）→比对→质量门禁，全部阻塞返回。云端 outline 只依赖 PDF 抽图+profile，**不依赖本地解析结果**（比对才依赖），具备并发条件。无任何 spawn/锁原语；`update_job`/`write_json` 非原子。
- `run_cloud_review_core`（`auto_pipeline.rs:1946-2274`）重跑 vision 抽图+outline，会重复计费；只做 outline 对照，不含视觉答案候选。

### 视觉答案落盘
- 候选 `vision_answer_candidate_for_job`（`auto_pipeline.rs:443-480`）只存活于内存，applied=false/diagnosticOnly=true 硬编码（:1406-1407）；唯一落盘 `vision-answer-output.json`（:1404）全仓零读者、且不在 cleanup 删除名单（孤儿）。候选不进 `split.answerKeyCandidates` 合并（:1452-1454 只并本地候选）。
- 审计摘要语义失真：`vision_answer_extraction_summary` 的 filled/missing 按本地 authoring-ir 现状计算（:1627-1630），把本地填的答案说成「视觉模型已补全」；attempted=true 就追加，不看 failure/applied。
- 命令面无 `apply_vision_answers`（对照已有 `apply_vision_transcription` `authoring_commands.rs:285-315`）；UnifiedPreview 只渲染 `vision_transcription_summary`（:298-306），`vision_answer_extraction_summary` 与逐题候选无任何 UI。
- 导出门禁（`runtime_validation.rs:360-389`）与导出链路 grep "vision" 零命中，视觉来源不可查。

### 网关
- **生产代码无任何 tool-calling**（全仓 grep tools/tool_calls 零命中）；实际模型是「JSON 建议 + 人工/半自动审批」，闸门密集但有三处 fail-open：
  - confidence 无 0..1 校验（`llm_gateway.rs:462-464` 只查 is_number 补 0.65；`llm_suggestions.rs:388-394` >=0.85 自动应用）→ confidence:87 可绕过人工审查。
  - evidence quote 只验 blockId 隶属+非空（`llm_suggestions.rs:559-574`），不验 quote 文本存在于源块 → 幻觉引文可通过自动应用。
  - provider 字段是摆设：网关无 provider 分支，全部 POST {base}/chat/completions（`llm_gateway.rs:88-92`）；AnthropicCompatible/Custom 选项误导。
- base_url 零校验（`llm_gateway.rs:49-56` 仅 trim）；429/408/Retry-After 不处理（:135-144）；JSON 校验失败不重试；单次最坏 3×300s 阻塞、逐组串行放大；无请求级耗时/错误分类落盘（llm-calls.jsonl 只记 suggestion 本体）。
- API key 存储良好：keyring 优先、明文回退需 EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK=1、缓存脱敏（`llm_profiles.rs:52-102`、`llm_gateway.rs:36-43`）。
- legacy sidecar gateway.mjs 与 Rust 网关行为漂移（缺 2 种 kind、接受明文 key、静默兜底），已标注 legacy 不在生产路径。

### 前端
- `llm_classify_group/llm_extract_group/apply_llm_suggestion` 已注册（`lib.rs:1431-1433`）且 API 已封装（`tauriCommands.ts:199-209`）但**零 UI 消费方**；`group.llmReview` 持久但无渲染。
- UnifiedPreview 预览编译双重吞错（:225-236 `catch {}`）；Settings testLlmProfile/removeProfile 无 catch（:190-204）；rerunOcr/applyManualTranscription 无 busy 无错误态。
- V2 导出门禁错误格式（`authoring_v2_commands.rs:201-262` 的 `authoring_v2_export_blocked:...`）与前端解析（`ExportPage.tsx:32-82` 只认旧 `nas_export_validation_failed:{json}`）不匹配 → 分类指导全部不可达。
- 导入不可取消、无后端进度事件（全仓 grep `.emit(` 零命中）；云复核队列存 sessionStorage 重启丢失。

### 测试
- Rust 536 测试；AI 全部证据在命令处理器级。零覆盖：JSON 容错解析、重试状态码、provider 分派、endpoint 安全、tool calling、并发顺序。mock server 只接受 1 个请求只回 200（`lib.rs:1736`），重试/负例在结构上不可测。前端零单测；ui-flow-e2e 走 dev-fallback 不经真实 Tauri 后端。
- 关键已有覆盖：云端 mismatch 不覆盖本地答案（`lib.rs:6281-6286`）、对照 summary 最小化后存活（:6287-6299）、localOnly 零云端调用（:9860）、LLM 失败不阻断草稿（:4292）。

### 产品要求（对并发目标的裁定）
- 文档要求：本地确定性解析是唯一可自动写入 AuthoringIR 的来源（总计划:334）；云端只能只读对照 DiagnosticComparison、不得覆盖题源（总计划:45、338）；不把 cloud LLM 作为必经阶段（:3799）；规则优先、规则不足再调 LLM（Tauri设计:1482）。
- 文档**没有**「LLM 与本地规则并发」的表述。用户本轮目标的合规落地方式：LLM 转化与本地解析**并发执行但结果只作只读比对**（缩短总时延），本地权威与「云端不回写」约束不变；云端调用必须显式 opt-in（R44），UI 需提供该开关。
- contracts 侧：confidence 全部 0..1；extractionMode 枚举无 vision/llm 取值（视觉产物溯源只能走 sourceVariant.provider）；document-ir 表格 detectionMode 含 vision_model。

### 修复优先级（本轮执行序）
1. [complete] P0-A 可达性：ImportWizard 增加「云端对照」opt-in（存在启用 profile 时可选），传 profileId + executionMode full；报告回传 profileId 使后台复核队列可用；去重避免双重云端调用。
2. [complete] P0-B 并发：full 模式下用 std::thread::scope 把「PDF 抽图→视觉答案候选→云端 outline」与本地解析并发；本地草稿落盘后 join，做比对+审计+门禁；LLM 失败不阻断草稿。
3. [complete] P0-C 视觉闭环：持久化 vision-answer-candidates.json（逐题 answer/confidence/evidence）；新增 apply_vision_answer_candidates 命令（接受→写入 question.answer+confidence+revision，拒绝→audit）；UnifiedPreview 渲染候选+逐题采用/忽略；修正摘要三态语义；cleanup 收编孤儿文件。
4. [complete] P1 校验收紧：confidence 越界 fail-closed 置 0；evidence quote 文本与源块比对；base_url scheme/host 校验（http 仅限本地/私网）；provider 枚举校验+UI 说明；429/408/Retry-After 重试；网关调用耗时/错误分类落盘 llm-calls.jsonl；profile.enabled 检查。
5. [complete] P1 前端补口：题组「获取 AI 建议」入口（classify+extract+diff 卡+应用/忽略）；V2 导出错误分类解析；预览编译/Settings 静默失败补错误态。
6. [complete] 测试：网关负例矩阵（JSON 容错/重试/429）、视觉候选闭环、并发顺序、endpoint 校验、quote 验真、confidence 越界。

### A7 后续增量改进（非阻断）
1. 导入取消/进度事件：当前导入过程阻塞 UI，无取消按钮；Rust 后端无 `.emit()` 进度事件推送。影响：长文档导入期间用户无法取消，也看不到详细进度。
2. 云复核队列持久化：当前队列存 sessionStorage，刷新/重启丢失。影响：后台队列任务在浏览器重启后需要手动重新触发。
3. Preflight LLM 连通性检查：Settings 环境预检目前只检查 Python/pypdf/renderer，不检查 LLM profile 网络连通性。影响：用户导入时才发现 LLM 配置无效。
4. V2 revision 链分歧：Phase 5 V2 编辑器使用独立 revision 命名空间，legacy `apply_llm_suggestion` 等命令仍写 V1 namespace。影响：系统性架构遗留，需 Phase 6 统一迁移规划。

## 2026-09-03 用户确认的设计修正

- durable audit 不按 HTTP 请求计数：一次 AI 阶段/run 归并网络重试；原始 prompt/response 仍按诊断设置选择性保存。必须保留能解释产品决策的候选生成、接受/拒绝、权威写回、阻断和失败摘要。
- 输出校验的“补默认字段”是 fail-open：例如缺 `confidence` 被写成 `0.65`、缺 `patch/questions/evidence` 被写成空容器；这会把协议违规结果伪装成合法候选。业务字段缺失/类型错/证据或 patch 不合法应拒绝并进入 needs-review。
- 目标 UI 是代码补全式候选审阅：原文为基线，AI 新增绿色、删除红色，接受后才写入 revision；拒绝不改变权威稿，视觉答案同样只是候选。
- 当前工作区已有未提交的并发/候选实现，后续审计以其实际代码为基线，不回滚或覆盖；需重点检查其 `Mutex` 是否把网络调用重新串行化，以及拒绝-only、重复接受、重启后的决策语义。

## 2026-09-03 对抗审计复核基线

- 两轮共 12 个只读探针均在延长等待窗口内没有返回可引用正文，已按异常规则停止；不得将代理空结果视为通过。
- 当前主线程读到的网关入口将 `classify_group`、`extract_group`、视觉转写、视觉答案和云端 outline 全部路由到 OpenAI-compatible `/chat/completions`；`tools`、`tool_choice`、`tool_calls` 尚未进入请求/响应契约。
- 当前 `run_llm_gateway` 的 `llm-calls.jsonl` 是调用级诊断记录，和用户确认的阶段级 durable audit 不是同一层；后续需保留诊断可选性，同时将候选生成/接受/拒绝/写回/阻断/失败归并为阶段事件。

## 2026-09-02 修复实施记录（第一轮优化）

### 已落地
- **P0-B 并发改造**（`src-tauri/src/auto_pipeline.rs`）：`run_auto_pipeline_core_with_gateway` 用 `std::thread::scope` 在本地解析前启动云转化工作线程（`run_cloud_conversion_worker`）：PDF 抽图一次 → 视觉答案候选 → 云端 outline 生成，经 mpsc 通道回传；主线程本地解析/split/草稿落盘/逐组 LLM 并行推进；草稿落盘后 recv 汇合做只读比对+审计。网关闭包经 `Mutex` 共享（`lock_gateway` 防 poison）。此前 full 模式下抽图最多跑 3 次，现在 1 次。`cloud_outline_check_for_job` 拆为 `cloud_outline_generate_with_gateway`（工作线程）+ `cloud_outline_report_from_output`（主线程比对）。
- **P0-C 视觉答案闭环**：候选结构化落盘 `vision-answer-candidates.json`（VisionAnswerCandidatesV1，逐题 questionNumber/questionId/answer/confidence/evidence）；新增 Tauri 命令 `apply_vision_answer_candidates`（`apply_vision_answer_candidates_core`，llm_commands.rs）：采用→写入 question.answer+confidence（verified 保持 false，人工确认门禁不变），忽略→audit；audit 追加 `vision_answer_adoption` issue；`get_job` 回传 `visionAnswerCandidates`；UnifiedPreview 渲染候选面板（逐题采用/忽略，`vision-answer-candidates` testid）+ `vision_answer_extraction_summary` 横幅；`run_cloud_review_core` 后台复核也产出候选；摘要消息三态化（失败/无候选/待确认），不再把本地答案说成视觉已补全；cleanup 把孤儿 `vision-answer-output.json` 加入删除名单，候选文件保留供重启后确认。dev fallback 同步实现（e2e 可用）。
- **P0-A 可达性**：ImportWizard 加载 `listLlmProfiles`，存在启用的非占位 profile 时显示「导入时并发运行云端对照（只读）」开关（默认勾选、可取消、发送内容范围说明），勾选时传 `executionMode:"full"+profileId`；full 模式报告含 cloudComparison.attempted，后台队列 guard 天然去重不重复调用。
- **P1 校验收紧**（llm_gateway.rs/llm_suggestions.rs/llm_commands.rs）：confidence 缺失/非数值/越界 fail-closed 置 0 + warning（suggestion/vision 转写/视觉答案）；cloud outline 置信度 clamp；evidence quote 文本与源块内容归一化比对（`llm_suggestion_quote_mismatches`，流水线与 apply 命令在 document-ir 可用时启用，最小化后人工采纳不受阻）；`save_llm_profile_core` 校验 provider 枚举 + base_url（http 仅限 localhost/私网、拒绝内嵌凭据）；`llm_run_group_core`/`test_llm_profile_core` 校验 profile.enabled；重试增加 429/408 + Retry-After（封顶 5s），非 2xx 先于 JSON 解析处理（HTML 错误页也能按状态重试，raw 截断 300 字符）；`run_llm_gateway` 每次调用写 `{recordType:"llm_call", ok, latencyMs, errorClass}` 到 llm-calls.jsonl（失败也落盘）。
- **前端补口**：Settings 测试连接/删除/预检补错误态与 busy（未保存配置测试给出明确提示）；学生端预览编译失败展示首条错误（`preview-compile-error`）。

### 验证结果
- `cargo test --lib`：**533 通过**（基线 525）；新增 7 个测试全过：429→200 重试、llm_call 观测记录、JSON 容错解析、confidence 越界 fail-closed、幻觉 quote 拦截、base_url/provider 校验、视觉候选采用闭环（含 verified 不被置真断言）。
- `npm run check`（tsc）：通过。
- **10 个失败全部为 Windows 本机预存环境问题**（stash 验证与 HEAD 相同）：8 个需要私有 PDF 语料或 Python pypdf（本机 pip 无网络）、2 个 reqwest 请求头断言（`authorization: Bearer` 在 HEAD 也失败）。
- `npm run e2e:ui-flow`：clear-text route 超时，**stash 验证 HEAD 基线同样失败**（疑与本机缺 pypdf/浏览器环境相关），非本轮改动引入。
- 有意的行为变更（golden drift 裁定）：`vision_answer_extraction_summary` 消息从「视觉模型已从 PDF 图片页补全答案」改为「视觉模型产出了答案候选，尚未写入题稿」，同步更新 lib.rs 测试断言——旧消息把本地解析填的答案错误归因给视觉模型（审计 P1-5）。

### 遗留（按优先级）
- P0-D：V2 导出门禁错误解析（`authoring_v2_export_blocked:*` 与前端分类指导不匹配）。
- P1：题组「获取 AI 建议」UI 入口（llm_classify_group/llm_extract_group/apply_llm_suggestion 已有命令与 API，无 UI 消费方）。
- P1：导入不可取消、无后端进度事件；云复核队列 sessionStorage 重启丢失。
- P2：preflight 无 LLM 连通性检查；test_profile 缓存写到 jobs/profile-test/；请求体大小上限。

## 2026-09-02 对抗审计轮（第 2 轮红队）与修复

两个对抗审计代理分别攻击并发改造与视觉/网关修复。裁定与处理：
- **[阻断→已修复] worker panic 挂死**：Sender 原由函数帧持有，worker panic 时 `recv()` 永久阻塞、scope join 重抛 panic。修复：Sender 所有权移入 worker 闭包 + worker 体 `catch_unwind`（panic 转为 `cloud_conversion_worker_panicked` Err 结果），「LLM 故障不阻断草稿」在 panic 路径也成立。worker spawn 移到本地解析之后（最常见的解析失败提前返回不再等待云端）。
- **[严重→已修复] 「忽略」按钮必然失败**：reject-only 决策被 `accepted.is_empty()` 误判为错误。修复：接受空 accepted（rejected/alreadyAnswered 非空即成功），拒绝项持久化为候选文件的 `dismissedAt`（重启后不再重现），错误仅在决策全为 unmatched 时返回。
- **[严重→已修复] base_url 私网前缀绕过**：`http://10.evil.com` 等公网子域名命中 `starts_with("10.")`。修复：改为 IPv4 字面量严格解析（`Ipv4Addr::parse` + is_loopback/is_private/is_link_local）+ 精确主机名（localhost/::1/.local/host.docker.internal）。
- **[严重→已修复] 候选可覆盖已确认答案**：apply 命令现检查目标题现有 answer 非空或 verified=true → 计入 `alreadyAnsweredQuestionIds` 不写入；decision.answer 覆盖参数限定 string/string[]；数字型 questionNumber 兼容。新增测试覆盖 reject-only 与防覆盖。
- [一般→已修复] 候选 TOCTOU：apply 支持 `generatedAt` 回传校验（`vision_answer_candidates_stale`）；ImportWizard renderModelReport 旧文案「已尝试，未安全写入」改为候选语义。
- [一般→已修复] quote 验真盲区：document 存在但 blockId 查不到时报告 `evidence_quote_block_missing`（不再静默跳过）；归一化增加连字符容错变体。
- [已知留档] `run_cloud_review_core` 每次复核都会刷新候选（设计如此，但无去重）；置信度默认填充顺序导致「missing→0」注释偏差已改注释；Phase5 V2 revision 链与 legacy apply 命令的分歧是系统性既有缺口（apply_llm_suggestion_core 同模式），列为后续项。
- 默认勾选云端对照为产品向决定（用户目标要求 PDF 导入即并发云端对照；勾选框明示发送范围、可取消；localOnly 路径有零云端调用测试守护）。

## 2026-09-02 第三轮补充（遗留 P0/P1 前端口）
- **P0-D 已修复**：ExportPage 新增 V2 发布门禁错误解析（`authoring_v2_export_blocked:quality_state|unresolved_answers|hard_failures|issues=*` 与 `authoring_v2_export_compile_blocked:{json}`），分类为可操作指导，V2 下 canForce=false（与隐藏的强制导出按钮一致）。
- **题组 AI 建议 UI 入口已落地**：UnifiedPreview 题组工作台新增「AI 题组建议」面板——「获取 AI 建议」按钮（llmExtractGroup，需已启用 profile）、建议卡片（建议题型 vs 当前题型、置信度、warnings）、「应用到题组」（applyLlmSuggestion，kind/layout/questions 三路径，置信度 <0.85 时禁用并说明）、持久化 `group.llmReview` 警告横幅渲染。新增样式。
- 最终回归：cargo test --lib 533 通过 / 10 失败（全部为预存环境问题：8 个需私有 PDF 语料或 Python pypdf（本机 pip 无网络）、2 个 reqwest 请求头断言在 HEAD 基线同样失败）；`npm run check` 通过；`npm run e2e:ui-flow` 在 HEAD 基线同样失败（本机环境）。

# 2026-09-13 跨仓库闭环第一轮侦察统一 findings（6 子代理 + 主代理抽查证实）

## 证据分层说明
- 下述每条标注：[产品E2E] / [service] / [schema] / [推断待复现]。

## P0（阻断交付，必须修复）

1. **hotspot 提交契约断裂** [service+schema，行号抽查证实]
   - 学生端 FigureNode.vue:65-67 点击热点提交 `hotspot.hotspotId`；AnswerSlotNode.vue:116-119 同。
   - 服务端 reading-v2-loader.ts:484-487 要求非文本提交值 ∈ 该 slot 解析出的 option bank label（大写化），否则 `reading_v2_submission_invalid`；reading-sessions.ts:317 `requireComplete:true` 使终审整体 500。
   - 生产端 hotspotId 来源：editorCommands.ts:56-62 `set_slot_placement` 生成 `${slotId}-hotspot`；SelectionInspector.tsx:48 `crypto.randomUUID()`。DiagramHotspotV2 无 label 关联字段（content-doc-v2.ts:100-105）。
   - PDF2Test 编译器无 hotspot 校验：reading_source_v2.rs:369 diagram_hotspot answer-kind 检查落在 `_ => true`。
   - 结论：热点题"能点但不可提交/计分"。修复方向：hotspotId ≡ 该 slot answerKey 的 option label（导出重写或编辑期约束），编译器+发布门拒绝无法映射的热点，PDF2Test 预览按学生语义渲染热点按钮。

2. **Electron V2 图片资源 URL** [推断待复现，代码链证实]
   - contracts-v2.ts:145-147 `readingAssetUrl` 返回相对 `/api/exam/reading/...`；FigureNode.vue:4 `:src="assetUrl"`、:47。
   - 所有 API 走 resolveApiUrl→`http://127.0.0.1:<port>`（examApi.ts:72-96），listening 资源显式用它（examApi.ts:227-229），reading 图片是唯一例外。
   - app:// 页面内相对路径 → `app://app/api/...` → protocol.js:89-120 只服务打包文件 → resolveBundledAsset 找不到 → 404（resourceOverlayManager.js:267-296）。
   - 复现步骤：Electron 打开含 figure 的 V2 考试 → Network 过滤 `reading/.*assets` → 确认 404；手动改 `http://127.0.0.1:<port>/api/...` 应出图。修复：readingAssetUrl 经 resolveApiUrl。

3. **batch 发布 `./releases/<batchId>/` 布局与学生端硬编码 `resources/${examId}` 不兼容** [service，行号抽查证实]
   - nas_package_v2.rs publish_items_core：script/resourcesBase/assetManifest 重写为 `./releases/{batch_id}/...`，staging 整目录 rename 到 releases/<batchId>。
   - 学生端 reading-asset-resolver.ts:107,122 硬编码 `safeJoinNasRoot(root, 'resources/${examId}')`；resourcesBase 只校验不参与解析（NasJs:120-122）。
   - 影响：batch 发布的资产学生端必然 404/integrity fail；单考发布布局吻合无此问题。修复（生产侧，不改学生端抽象）：batch 布局保持资源位于 reading 根 `resources/<examId>/`（manifest 的 script 可留在 releases/，assetManifest/resourcesBase 必须指向根级 resources/<examId>），并让 probe/nas-student-contract 对 batch 布局复验。

4. **EDIT_VERSION_CONFLICT 页面级死路** [service+E2E]
   - useCanonicalEditor.ts:118-126 保存失败仅置状态；ExamWorkspacePage.tsx:129-139 返回/发布/重识别均先 flush（抛错被 withBusy 吞，不导航）；reload() 先 persist 且失败被吞（useCanonicalEditor.ts:243-245）→ 冲突永久，只能重启。
   - 触发面：识别重跑 bump edit_version + processing 事件跳过 reload（ExamWorkspacePage.tsx:61-67）。
   - 修复：saveState failed/conflict 提供"重试保存/放弃本地修改并重载"；返回按钮 flush 失败给确认对话框而非静默拦截。

## P1

5. **共享契约漂移** [schema，SHA 实算证实]
   - PDF2Test contracts/ielts-authoring-ir-v2.schema.json = 61b05bf59dd094c2679e8fb2761a3ec7082c8842e1d2953a9e162665e345fa31（20070B，含 recognitionBlockers/recognitionBlockerTargets/recognitionBlockerTarget 定义）。
   - 学生镜像 developer/contracts/authoring/ = ee09788cc01074eef9dd79c6bf9dbd87036e11f137fd52ae945c885b4a648271（19553B）。
   - diff 恰为上述三段（19a20-27、422a431-439），全部 optional；其余 9 个 schema 与 contract-manifest 结构双仓一致。
   - 同步机制已存在：PDF2Test scripts/verify-schema-contract.mjs（--peer-root）+ 学生 developer/tests/cross-repo/authoring-schema-mirror.cjs（断言 manifest 逐字节+逐文件 hash）。
   - 决策：canonical owner = PDF2Test（producer）；把 2 个文件同步到学生镜像，方向 PDF2Test→学生，字段保留为 optional（用户任务书允许）。
   - 学生端 student-repo 的 verify-schema-contract.mjs 默认 peer 根 `../NAS` 不存在，须用 --peer-root 指向 IELTS-NASfor-WenDao\developer\contracts\authoring。

6. **编译器校验缺口** [service]
   - hotspot 有效性（hotspot.slotId↔answerSlots、figure_hotspot hostNode）全链无校验（同 P0-1）。
   - hostNodeId 悬空无校验；questionNumber 唯一性不查（reading_source_v2.rs:90 排序静默）。
   - unresolved answer 编译容忍（有意），靠发布门拦截；但学生 loader 更严（reading-v2-loader.ts:342 直接 fail）——发布门必须兜住。

7. **apply/采纳命令写 V1 不写 canonical** [service]
   - apply_vision_answer_candidates_core（llm_commands.rs:453-701）、apply_llm_suggestion_core 写 V1 authoring-ir.json；M1 权威是 library_items_v2 canonical DS → 采纳结果到不了新工作区。

8. **视觉候选无 revision 绑定** [service]：vision-answer-candidates.json 与生成时识别 revision 无绑定（llm_commands.rs:459-461 直接读文件）；仅"已答不覆盖"间接保护。

9. **issue 面分裂** [service]：用户看到的 deriveActionableIssues（actionableIssues.ts:105-162，6 code）≠ 发布门消费的 quality.issues/recognitionBlockers/SourceReview；recognitionBlockers 前端不渲染；DB actionable_issues_v1 表零读写。

10. **发布门双盲** [service]：validate_authoring_v2_publish_readiness（authoring_v2_commands.rs:187-344）不含 stale cloud candidate、不读 user_edited、recognitionBlockers 仅 flag-on 时经 quality 生效；无学生端加载探针（runtime_validation.rs:282-326 仅静态）。

11. **过期 localStorage 恢复稿覆盖新识别 ds** [service]：useCanonicalEditor.ts:145-157 整体覆盖+还原旧 version，无"放弃恢复"入口。

12. **删答案位遗留悬挂 hotspot** [service]：patchDeleteAnswerSlot（authoringV2Patches.ts:353-366）不清理 node.hotspots 中指向被删 slot 的热点；UI 无法修复。

13. **跨仓真实包 E2E 为零** [产品E2E]：学生端 Electron E2E（student_exam_altu_electron_flow.py）只消费手写 V1 fixture；PDF2Test 真实 publisher 包从未进真实学生 UI（nas-student-contract.mjs:10-11 自认 pending）。方案：dump:nas-fixture（真 publish 产物）→ 学生端 EXAM_RUNTIME_CONFIG 指向该目录 → Playwright CDP 驱动作答/提交/计分断言。

14. **计分 E2E 无 is_correct 断言** [产品E2E]：主链只数 sqlite 行数（altu L248-271）。

## P2/P3（择要）
- unordered_set 预览无"选满禁用/slice"（学生端 ReadingExamV2Renderer.vue:42,190-195 有）；PDF2Test student 预览 inline text onChange 不写状态（ExamCanvas.tsx:334）；热点在 student 预览 no-op（:263-266）。
- MatchingMatrix 矩阵版式 vs 学生平铺（值等价，有意分歧，记录即可）。
- crop 支持不对称：PDF2Test 显示裁剪图+重映射热点，学生显示全图+原始 rect（FigureNode.vue:46-57）。
- V1 payload answerKey 不剥离（ExamReadingService.ts:35-37，注释明示有意保留 V1 opaque payload）——legacy 行为，不破坏；重点保证 V2 边界。
- checksum canonical JSON 双端等价（serde_json vs JS canonicalJson）需真实 E2E 证实。
- direct canonical 现状：empty prompt 55/57、visual 5/5 失败、statement 0 覆盖、private-real=0；flags 全默认关；不宣布达标。
- per-keystroke patch+整文档 clone 性能；TS/Rust expandExpression 校验分叉；窄窗 issue 定位无效；OPTION_TEXT_MISSING（分组选项）无定位属性。
- 学生端 manifest 别名索引可被同名 dataKey 劫持（P3）；资源字节 SHA 读取时才校验（P3）。

## 覆盖矩阵（Agent F，按证据层级）
- 产品E2E 已覆盖：导入（PDF 钩子+DOCX 文件钩子）、编辑/保存/重开、返回按钮 12 步、取消/强杀恢复、publish-ready 发布链、direct canonical 工件断言（audit note）。
- 仅 cargo/service：product_chain 9 测试（导入/识别/编辑/导出/发布/批次原子性）、NAS probe、学生 provider/loader 校验、phase6 计分语义。
- 仅 schema/CLI：verify-schema-contract、nas-student-contract 镜像规则。
- 完全未覆盖：学生端 Electron 加载真实 V2 包、V2 交互 UI、资源 UI 显示、UI 计分断言、真实识别→ready→发布。

## 环境/命令速查
- PDF2Test E2E：`npx tauri build --debug --no-bundle` → npm run e2e:tauri:direct-canonical（--keep）等；env：PDF2TEST_AUTOMATION_*、QLG_DIRECT_CANONICAL、LOCAL_RECOGNITION_BLOCKERS_GATE。
- 学生端：npm run build:server && build:student-exam；py developer/tests/ci/run_static_suite.py；py developer/tests/e2e/suite_practice_flow.py；npm run verify:cross-repo-reading-v2；env：IELTS_PDF2TEST_REPO、EXAM_RUNTIME_CONFIG、IELTS_USER_DATA_PATH、EXAM_ADMIN_TEST_DATA_DIRECTORY。
- 契约校验：node scripts/verify-schema-contract.mjs --peer-root F:\workspace\IELTS-NASfor-WenDao\developer\contracts\authoring；学生侧 node developer/tests/cross-repo/authoring-schema-mirror.cjs（IELTS_PDF2TEST_REPO=F:\workspace\PDF2Test）。

## 2026-09-15 本轮 findings（三个缺口闭环）

标注约定：**FACT** = 本轮实测/代码可查；**INFERENCE** = 由事实推出的判断；**HISTORY** = 上一轮或更早的结论，不作本轮验收。

### F-R1-1（FACT，根因）`cargo build` 产出的是 dev 模式二进制
- `src-tauri/tauri.conf.json` 同时有 `devUrl: http://localhost:1420` 与 `frontendDist: ../dist`。
- 实测：`cargo build` 出的 exe 通过 CDP 读到的页面是 `ERR_CONNECTION_REFUSED / localhost 拒绝连接`（Microsoft Edge 错误页）——说明它在加载 `devUrl`，而无人监听 1420。
- 结论：可验收的内嵌构建必须来自 `npx tauri build --debug --no-bundle`。这是上一轮「应用单独跑得起来、driver 第一条命令就死」的另一半根因。

### F-R1-2（FACT）tauri-driver + msedgedriver 在本环境无法建立稳定会话
- 报错：`session not created / unable to connect to renderer`、`chrome not reachable`、`invalid session id: session deleted as the browser has closed the connection`。
- 参数矩阵（baseline / `--disable-gpu` / `--no-sandbox --disable-gpu` / `--disable-gpu-compositing`）只有 `--no-sandbox --disable-gpu` 能建立会话。
- 采用替代通道：WebView2 自带 CDP（`--remote-debugging-port`），封装在 `scripts/e2e/lib/tauri-cdp-harness.mjs`。驱动真实 exe / 真实后端 / 真实 SQLite / 真实文件系统。
- **FACT**：`--no-sandbox --disable-gpu` 放宽了渲染进程沙箱与 GPU 路径，属诊断参数；报告记 `diagnosticRun=true`，不得算作默认产品路径通过。

### F-R1-3（FACT，本 agent 自身缺陷）harness 判定 bug
- `tauri-cdp-smoke.mjs` / `tauri-cdp-product-chain.mjs` 都在 `try` 块里用 `report.steps`（此时仍是空数组）算 verdict，而 `report.steps` 要到 `finally` 才赋值 → 空数组让 `every()`/`filter()` 得出「通过」。
- 实测后果：DOCX 那次 9 个步骤全失败，报告却写 `verdict=passed`。已修：判定移到 `finally` 里 `report.steps = recorder.steps` 之后，并加「步骤数为 0 也算失败」。
- 教训：**harness 的判定逻辑本身必须被反向验证一次**，否则它会把失败洗成通过。

### F-R2-1（FACT）工作区学生预览已接通，走官方编译闸门
- `buildReadingSourceV2FromAuthoring` 之前**没有任何调用方**（全仓搜索为空）。现在由 `src/features/editor/studentPreview.ts` 调用：编译失败不渲染预览，只给可定位的 `code/targetId`；编译成功交给共享 `ExamCanvas` 的 `mode="student"` 渲染，不新增第三份渲染近似。
- 真实链路证据：`student-preview-renders` 断言 `isStudentMode=true`、`hasAuthorTextarea=0`、`hasAuthorTools=0`、预览正文含刚保存的 `E2E EDIT CHECK 42`、初始无任何预填作答。
- **FACT**：`student-preview-answering-isolated` 断言预览里作答后，作者答案 `["q1=TRUE","q2=FALSE","q3=NOT GIVEN"]` 前后完全一致，且无新修订、无保存触发。
- **INFERENCE（限制）**：该次预览选择的选项恰好与作者答案相同（都是 q1=TRUE），隔离性证明偏弱。已把脚本改为**刻意选择与作者答案不同的选项**再断言，但改动后的这次运行结果需另行记录。

### F-R2-2（FACT）共享选择上限与学生端不一致（已修）
- 真实 `ReadingExamV2Renderer.vue:42` 用 `cardinality.max` 作为 `unordered_set` 的禁用阈值，legend 才用 `exact`。
- `ExamCanvas.tsx` 原来用 `cardinality.exact ?? cardinality.max` → 作者预览会比学生端更早锁住选项。已改为 `cardinality.max`。

### F-R2-3（FACT，契约差异，非渲染差异）内联 answer_slot 的选项分支不可达
- 真实 `AnswerSlotNode.vue` 支持内联 `answer_slot` 的 radio/checkbox/select 选项（读 `node.options`）。
- PDF2Test 的 `AnswerSlotNodeV2`（`src/types/content-doc-v2.ts:157`）**没有** `options` 字段，本流水线无法产出这种节点 → 该分支不可达，故不实现。

### F-R3-1（FACT，需要后端对齐）`get_recognition_decision` 实际返回形状与契约不一致
- 契约 §2.3 约定：`cloudStatus` / `localStatus` / `cloudReasonCode` / `items` / `summary`。
- 实测后端返回：`{ chains: { local|cloud|source|adjudication: { state } }, actionable, autoApplied, editVersion, jobId, baseEditVersion, batchId, stale, summary, generatedAt }`——没有 `cloudStatus`，也没有 `items`。
- 实测后果：面板渲染出 `云端核验状态未知（undefined）`——这正是任务书禁止的「假信息」。
- 前端处理：`normalizeDecisionView` **两种形状都认**，优先契约字段，缺了回退 `chains` / `actionable` / `autoApplied`；缺字段一律降级成 `not_started`，绝不猜成完成。单测覆盖契约形状、实现形状、完全缺字段三种情况。
- **待办**：请后端按契约补齐 `cloudStatus` / `localStatus` / `items`（或双方确认以 `chains` 为准并改契约文档）。

### F-R4-1（FACT）真实样本的发布被质量门禁拦下，且前端无清除入口
- `fixtures/parser/complex-reading.pdf` 导入后 4 项阻断：`QUALITY_NOT_READY`、`QUALITY_HARD_FAILURE`、`completion slot 没有可渲染的宿主节点`（target `group-2`）、`仍有显著源区域未被题目、passage 或有理由的忽略记录解释`（target `document`）。
- 代码核查：`QUALITY_NOT_READY` / `QUALITY_HARD_FAILURE` 来自 `authoring_v2_commands.rs:418/432`，读的是 `quality.state` 与 `quality.hardFailures`；前端没有任何调用方触发质量重算或 review-state 变更（`refresh_quality_report` 无调用方）。
- 结论：这是识别质量问题，属识别/云端 agent 范围。按任务书「门禁拦下是正确负例」，本轮不伪装成发布成功；**学生端链路因此未完成**。

### F-R4-2（FACT）`complex-reading.md` 不是 `complex-reading.pdf` 的答案表
- 实际导入该 PDF 后题面为 "Complex Fixture Passage / museum that moved its archive into a renovated warehouse"，工作区答案 q1=TRUE、q2=FALSE、q3=NOT GIVEN。
- `.md` 写的是 1 TRUE / 2 FALSE / 3 TRUE / 4 diaries / 5 diaries → q3 与题面不一致。
- 结论：该 `.md` 不能当人工期望答案表；Task 4 的人工答案表必须按实际导入题面另行人工核对。

### F-R4-3（FACT）「选择文件夹」导入入口只收 PDF
- `job_commands.rs:377` 的 `pick_pdf_folder_sources_core` 走 `list_pdf_files_in_dir` → 只列 PDF。
- DOCX 必须走「选择文件」入口（`automation_source_files_from_env`，读 `PDF2TEST_AUTOMATION_SOURCE_FILES`，任意扩展名）。
- 实测后果：DOCX 用「选择文件夹」入口时文件被静默丢弃，`picked-files` 永不出现，后续 9 步连锁失败。已修脚本按扩展名选入口。

## 2026-09-15 补充轮 findings（门禁根因 + 预览覆盖 + 夹具裁定）

### F-R5-1（FACT，P0，需后端修复）质量门禁对 `resolution` 完全无视——任何出现过硬失败的题稿永久无法发布
这是本轮最重要的发现，它解释了 F-R4-1「发布被拦」的真正原因，并且**不是题稿质量问题，而是门禁逻辑缺陷**。

`resolution`（用户「采用修正」/「保留当前内容」的落点）在两个独立位置被彻底忽略：

1. **状态推导**（`src-tauri/src/ielts_grammar/quality.rs:211`）：
   ```rust
   let has_blocking = !hard_failures.is_empty();
   ...
   let state = if has_blocking { "blocked" } else if ... { "review_required" } else { "ready" };
   ```
   `has_blocking` 只看 `hard_failures` 是否为空。而 `hard_failures` 由 `push_issue`（同文件 `3717-3721`）在 `severity == "blocking"` 时**无条件**写入，从不读 `details.resolution`。
   同文件 `213-220` 的注释声称已经修掉「resolving or ignoring an issue changed nothing」，但该修复只作用于 `unresolved_blocking_issues` 这一项（`221-231`）；`has_blocking` 分支在前、优先级更高，仍然无视 resolution。**修复不完整。**
   更关键的是执行顺序：`refresh_quality_report`（`authoring_v2_commands.rs:977-992`）先调 `evaluate_quality` 生成 `hardFailures`，**之后**才 `preserve_issue_resolutions` 把 `resolution` 盖回 `issues`。`hardFailures` 早已定型，盖回 resolution 不可能再影响它。

2. **发布门禁**（`src-tauri/src/authoring_v2_commands.rs:425-438`）：对 `quality.hardFailures[]` 逐条 push `QUALITY_HARD_FAILURE`，**同样没有 resolution 判断**。对比同函数 `455-468` 的 `unresolved_blockers` 是有 resolution 判断的——同一函数里两套标准。

- 后果：只要一份题稿曾产生过任意一个 blocking issue，`state` 就恒为 `blocked`，`check_publish_preflight` 就恒返回 `QUALITY_HARD_FAILURE`。用户无论怎么修、怎么点「保留当前内容」，都无法发布。这不是「正确拦下坏题」，而是「好题也永远出不去」。
- 归属：`ielts_grammar/quality.rs` 与 `authoring_v2_commands.rs`（后者在双 agent 契约里是**共享 append-only** 文件）。属识别/校验后端范围，**本 agent 不改**，仅交接。
- 需要的修复方向（供后端参考，非本 agent 结论）：`has_blocking` 与 `hardFailures` 循环都应先过滤掉 `details.resolution ∈ {resolved, ignored}` 的 issue，或让 `hard_failures` 本身携带 issueId 以便按 resolution 过滤。修好后必须有回归测试覆盖「blocking issue 被 ignored 后 state 变 ready / preflight passed」。

### F-R5-2（FACT）夹具裁定：`complex-reading.docx` 才是可用答案表夹具；`demanding-reading-passage-1.docx` 只有正文没有题目
- `fixtures/parser/complex-reading.docx` 解压后 13 段，**含题干与答案表**：`Answers 1 TRUE 2 FALSE 3 NOT GIVEN 4 maps 5 diaries`。它是同族 `complex-reading.pdf/.txt/.md` 的兄弟，可作为 Task 5 的人工期望答案表来源（注意 F-R4-2：`.md` 的 q3 与 PDF 实际题面不一致，需以实际导入题面为准）。
- `fixtures/parser/demanding-reading-passage-1.docx` 解压后 16 段，**只有 READING PASSAGE 1 标题、instructions 与正文段落，完全没有 Questions 1-13 的题干和选项**。解析器仍造出 13 个答案位，但无题干无选项 → 全部落成 text 交互槽。
- 后果：该 DOCX 不是合格的端到端验收夹具（学生无从判断该填什么），本轮把它降级为「text 交互路径」的覆盖样本，不作为 Task 5 主夹具。

### F-R5-3（FACT，本 agent 自身缺陷，已修）预览作答隔离步骤的两处缺陷
1. **比较对象错误**：`student-preview-answering-isolated` 里为了「刻意选一个与作者答案不同的选项」，去查 `[data-testid="exam-canvas-v2-author"] input[type=radio]`。但作者画布在 `mode === "student"` 时已被卸载（这正是预览隔离的实现方式），查询恒为空 → `deliberatelyDiffersFromAuthor` 恒为 `false`，退化成「点第一个选项」。旧报告里的 `previewChecked: ["q1=TRUE"]` 与作者答案相同，隔离其实是**巧合**而非证明。
   已修：作者答案快照在切到预览**之前**采集，并把快照注入预览侧的求值表达式做比较。
2. **只覆盖 radio**：原步骤只找 `input[type=radio]`，遇到纯 text 交互的题稿直接失败（`no-candidate-radio`）。而真实学生端 `AnswerSlotNode.vue` 明确支持 text / radio / checkbox / select / dragdrop / hotspot 六种，只测一种等于没测全。
   已修：按 radio/checkbox → text → 兜底 的优先级选择目标；选项类走真实鼠标点击，文本框走真实键盘输入（CDP `Input.insertText`），并如实记录 `inputMethod`；若受控组件未登记真实键盘输入则退化为原生 setter + `input` 事件并记录该退化。
3. 同时给第 7 步加了「可作答性」断言：`radio+checkbox+text+hotspot == 0` 时直接失败——一份学生无法作答的题稿不该被判成「预览渲染通过」。

### F-R5-4（FACT，产品缺口）`resolveIssue` 补丁在数据层已通，但没有任何 UI 触发它
- 数据层齐全：`AuthoringPatchV2` 有 `{ op: "resolveIssue"; issueId; resolution: "resolved" | "ignored"; note? }`（`src/types/authoring-editor-v2.ts:74`）；前端 `src/services/authoringV2Patches.ts:395` 实现 `patchResolveIssue`；Rust `authoring_v2_commands.rs:1125 → resolve_issue` 处理它。
- 但全仓检索无任何组件调用该 op。也就是说用户界面里**没有**「这个问题我确认过了 / 忽略」的入口。
- 叠加 F-R5-1 后影响被放大：即便补上入口，当前门禁逻辑下点了也无效。两者必须一起修才能真正打通发布。
- 归属：UI 入口属本 agent 的 `src/**`；但因为它单独修无效、且与识别/校验语义强耦合，先交接，待后端确认 resolution 语义后再落地，避免做出一按就假的按钮。

### F-R5-5（FACT，已修）发布步骤证据过薄，无法交接「为什么发不出去」
- 旧实现只记录 `outcome` 与 `manifestExists`，`blockers` 全丢。
- 已修：第 10 步在点击发布后额外走真实 IPC 调 `get_publish_preflight`，把 `passed` / `editVersion` / 每条 blocker 的 `code` / `targetId` / `internal` 与 warnings 一并写进报告，作为门禁交接的原始证据。

### F-R5-6（FACT）`RUNTIME_COMPILER_FAILED` 的真实原因：选项型槽位的答案键被写成 text 型
从运行产物的 `quality.compilerProbes.v2Runtime` 直接读到（不再靠猜）：

```
RUNTIME_CHOICE_SLOT_ANSWER_NOT_OPTION:q1:Slot interaction and answer key kind must agree; the student runtime rejects a mismatched key and the whole submission fails.
RUNTIME_RESPONSE_ANSWER_KIND_MISMATCH:q1:response kind and answer key kind must agree.
（q2、q3 同）
```

对照题稿实体（`authoring-ir-v2.shadow.json`）：

| 实体 | 实际值 |
| --- | --- |
| `answerSlots.q1.interaction` | `radio` |
| `group-1.taskType` | `true_false_not_given` |
| `group-1-responses.kind` | `choice`（`optionBankRef` = `group-1-option-bank`，选项 TRUE/FALSE/NOT GIVEN 齐全） |
| `answerKey.q1` | **`{kind:"text", values:["TRUE"]}`** ← 应为 `{kind:"option", labels:["TRUE"]}` |

也就是说：题面与选项都对，**只有答案键的类型错了**。真实学生端要求「非 text 槽位的答案键必须是 option 型」，否则拒绝整份提交。这是识别/自动产出答案键的缺陷，属另一 agent 范围，本 agent 只交接、不改。

另注：`recognitionBlockers` 里还有 `QUESTION_NUMBER_MISSING`（targets `group-1`/`group-2`），而 `answerSlots` 实际都有 `questionNumber`，两者不一致，一并交后端确认。

### F-R5-7（FACT，本 agent 已修）TS 运行时校验缺同一条判定——预览因此显示假完成
- Rust 侧有 `RUNTIME_CHOICE_SLOT_ANSWER_NOT_OPTION`；TS 侧 `src/services/readingRuntimeV2.ts` 的 `validateReadingExamSourceV2` **没有**这条检查（它只查 `RUNTIME_OPTION_BANK_MISSING` 等结构问题）。
- 后果链：TS 校验通过 → `buildReadingSourceV2FromAuthoring` 不抛错 → 预览把题面完整渲染出来、学生能作答 → 作者看到「一切正常」→ 点发布只收到一句「这道题存在必须修复的内容缺陷」。这就是任务书禁止的**假完成**。
- 已修：新增导出 `validateReadingAnswerKeyKinds(source)`，与 Rust 同一条判定（`text` 槽位要 text 型答案键，其余要 option 型；`unresolved` 不算类型不匹配）。
  - **刻意不并入** `validateReadingExamSourceV2`：那条路径被 `assertReadingExamSourceV2` 直接 throw，并进去会把整份预览一起挡掉，作者连学生视图都看不到，反而无从判断。单独暴露 → 预览保留学生视图，同时明确报出「提交/计分会失败」。
- 预览侧接线：`compilePreviewSource` 的 `summary` 新增 `answerKeyIssues`；`describePreviewPublishLimitation` 新增 `runtimeIssueCount`；工作区新增 `workspace-preview-runtime-issues` 区块（可点击定位回编辑态的具体答案位）。
- 新增 5 个单测（选项型槽位+文本型答案键 → 报出且仍可渲染；正确配型 → 不报；`unresolved` → 不报；文本槽位正确配型 → 不报；限制说明含「提交」）。前端合计 **122 passed**，`tsc --noEmit` 干净。
- E2E 侧新增第 12 步 `preview-and-gate-agree`：只要门禁报出 `RUNTIME_COMPILER_FAILED`，预览就必须已报出答案键类型问题，否则直接判失败。把「预览不得假完成」变成可执行断言，而不是靠人工看截图。

### F-R5-8（FACT）一致性断言第一版过粗，被真实数据证伪后收紧
第 12 步第一版写的是「门禁报 `RUNTIME_COMPILER_FAILED` ⇒ 预览必须报出答案键类型问题」。它在 `demanding-reading-passage-1.docx` 上**失败**了：

```
[step] FAILED preview-and-gate-agree :: 预览与门禁不一致：门禁报 RUNTIME_COMPILER_FAILED，但预览没有报出任何答案键类型问题（预览显示假完成）
```

查探针明细后确认断言本身有错，不是产品有错：`RUNTIME_COMPILER_FAILED` 有多个来源。三次运行的真实探针码：

| 夹具 | 探针 issueCodes |
| --- | --- |
| `complex-reading.pdf` | `RUNTIME_CHOICE_SLOT_ANSWER_NOT_OPTION`, `RUNTIME_RESPONSE_ANSWER_KIND_MISMATCH` |
| `complex-reading.docx` | 同上 |
| `demanding-reading-passage-1.docx` | `RUNTIME_ANSWER_UNRESOLVED`, `RUNTIME_RESPONSE_KIND_OPTION_SOURCE_MISMATCH` |

第三个夹具的槽位全是 `text`、答案全是未填，预览的类型检查**本来就该**报 0 条。所以断言收紧为「只在该原因码确实属于预览已覆盖的那一类、而预览却没报出时才失败」，并新增 `previewUnmappedProbeCodes` 如实记录「门禁拦下但预览没有专门呈现区」的原因码（这些原因仍通过问题列表的 `ANSWER_MISSING`、预览的 `answeredSlots`、门禁阻断数暴露）。
**该断言已被观测到真实失败过一次**，因此不是「永不触发的装饰断言」。

### F-R5-9（FACT，小问题，未修）选项正文与标签重复，学生端会显示成「TRUE TRUE」
预览截图 `06-student-preview.png` 里 T/F/NG 三个选项渲染为 `TRUE TRUE` / `FALSE FALSE` / `NOT GIVEN NOT GIVEN`。
- 渲染侧与真实学生端一致（`AnswerSlotNode.vue` 也是 `<strong>{label}</strong> {contentText(content)}`），所以不是渲染缺陷。
- 根因在数据：`optionBank.options[].content` 的文本与 `label` 相同。属识别侧产出问题，本 agent 只记录。

### F-R6-1（FACT，P0，本 agent 自身缺陷，已修）写路径还有**第二层**漂移：没把请求包进 `input`
上一轮修掉了字段名漂移（`{itemId,decisions[]}` → `{requestId,batchId,baseEditVersion,accept[],reject[]}`），
契约漂移检查器随即报「0 处破坏性不一致」。但写路径**仍然是坏的**，因为还有一层：

- Tauri 命令签名是 `async fn apply_recognition_decisions(input: Value, app: AppHandle)`
  （`src-tauri/src/lib.rs:980`）→ IPC 参数必须整体包在 `input` 键里。
- 我原来的 `src/api/recognitionClient.ts` 把 `requestId/batchId/baseEditVersion/accept/reject` **平铺**在顶层，
  真实后端于是返回：
  ```
  invalid args `input` for command `apply_recognition_decisions`: command apply_recognition_decisions missing required key input
  ```
- **契约漂移检查器存在结构性盲区**：它只比对 Rust **结构体**的字段名（`TS_MAP` ↔ struct），
  看不到**命令包装层**（`input: Value`）。所以「0 处破坏性不一致」与「写路径坏了」可以同时成立。
  这类漂移**只能靠真实 IPC 调用**发现——静态检查永远看不到。
- 已修：请求包进 `input`；新增 2 个单测把「顶层只能有 `input` 一个键」钉死（`recognitionClient.test.ts` 11 → 13 项）。
- 教训（写入本轮方法论）：**契约检查通过 ≠ 接线可用**。凡是跨 IPC 边界的形状，必须至少有一次真实调用证据。

### F-R6-2（FACT）学生端存在真实跨仓产品级 E2E，是 Task 5 学生侧的正确验收通道
`IELTS-NASfor-WenDao/developer/tests/e2e/reading_v2_student_flow.py` 正是任务书第 5 项要的链路：

> 真实 Electron 学生端加载 PDF2Test 发布的 ReadingExamSourceV2 包 → NAS 目录 → manifest 发现 →
> V2 loader 校验 → V2 渲染 → 图片显示 → hotspot 作答 → 提交 → 服务端校验与计分（`reading_answers.is_correct`）。
> 同时断言学生端 HTTP payload 不含 answerKey（V2 安全边界）。

- 输入：作者仓 `artifacts/nas-e2e-fixture`（P1/P2/P3），由 `src-tauri/src/product_chain.rs` 的
  `#[ignore]` 测试 `dump_published_v2_visual_package_for_student_e2e` 产出。
- 该 dump **走真实 export + NAS publish**（`dump_v2_visual_package_for_part`），学生端消费的是 publisher 原样输出；
  `create_export_folder` 明确「不重写 payload、不重算校验和」，因此哈希校验是真校验。
- **但它的输入是预先做好的 `READY_AUTHORING_FIXTURE`（quality 已 ready 的手工题稿），不是本轮真实导入的 PDF/DOCX。**
  因此它能验收**学生侧**，不能替代「本轮导入 → 发布」的耦合验收。二者必须分开写，不得混为一谈。
- 环境前置（本沙箱实测）：`playwright`（python）需装；`server/dist` 与 `dist/student-exam` 必须先移出，
  否则 `build:server` / `vite build` 触发 `SAFE_DELETE_BULK_CONFIRM_REQUIRED`（沙箱批量删除保护，非产品缺陷）。

---

## 2026-09-15 续行 R7 findings（撤销语义 + 断言自检）

### F-R7-1（FACT，P0，本 agent 自身缺陷，已修）「撤销自动修正」是假完成：撤销被实现成 reject 决策

**现象**：面板「已自动修正 N 项」折叠组里的「撤销」按钮，点击后显示「已保持现状 1 项」，
但**权威稿里的自动修正原样留着**。用户以为撤销了，其实没有。

**根因（两层，缺一不可）**：

1. 面板实现与契约不符。契约 §4.4 写「显示「已自动修正」+ 撤销（用 `undo`）」，
   §6.1 第 6 条写「撤销按钮（提交 undo 作为 editor 命令）」。
   而 `RecognitionPanel.tsx` 实现的是 `submit(item.decisionId, [item], "reject")`。
2. 后端 reject 的语义**本来就不是撤销**。`src-tauri/src/reconcile/commands.rs` 拒绝分支：

   ```rust
   // ── 拒绝：只改状态，不碰权威稿 ──────────────────────────────────
   for decision_id in &request.reject {
       ...
       items[index].status = DecisionStatusV1::Rejected;
       store::set_decision_status(...)?;
       outcomes.push(DecisionOutcomeV1 {
           kind: DecisionOutcomeKindV1::Rejected,
           message: "已拒绝该建议，权威稿未改动。",   // ← 明确不回滚
           ...
       });
   }
   ```

   即：reject 对一条**已经写入过**的 `auto_fixed` 项，只把 decision 状态改成 rejected，
   自动修正写进 `answerKey` 的值**不会**被改回。两者叠加 = 界面说成功、稿子没变。

**为什么单测和契约检查器都没抓到**：这是**语义**错配，不是字段错配。
两边各自的字段都合法、调用都成功、`outcomes[].kind` 也是合法的 `rejected`，
没有任何一层会报错——只有把「按钮文案」和「权威稿是否真的变了」放在一起看才会暴露。

**修复**：撤销改为把 `undo` 当**编辑器命令**提交，经 `apply_editor_commands` 的版本化事务
（版本 CAS + 幂等 + journal）改回旧值。

- `recognitionDecisions.ts` 新增 `parseUndoPatch(undo)`：严格校验 `setAnswer` 形状，
  认不出来返回 `undefined`。
- `RecognitionPanel.tsx`：有可用补丁才给按钮；没有则显示「这条没有带可撤销的信息，
  请在题面上手动改回原值。」——**宁可让用户手动改，也不给一个按了不生效的「撤销」**。
- `ExamWorkspacePage.tsx`：`applyAuthoringV2Patches` 离线试算 → `editor.applyPatch` →
  `await editor.flush()`。离线试算这一步是必要的：`applyPatch` 内部 `catch` 掉本地应用失败
  只置 `saveState="failed"`、**不向调用方抛错**，直接 `flush()` 会在什么都没写的情况下
  正常 resolve，于是又变成一次假完成。

**残余（已交接，见 §6 of `HANDOFF_2026-09-15_frontend_write_path_and_undo_fixed.md`）**：
`build_view` 把**所有** `resolution == AutoFixed` 的项无条件推进 `auto_applied`（不看 `status`），
所以撤销后该项仍列在「已自动修正」组里。契约未规定撤销后该条状态（`DecisionStatusV1`
没有「已撤销」）。我这一侧只做了**纯展示层**标记（会话内记住已撤销的 decisionId），
没有发明后端语义、没有额外写 decision 状态。

### F-R7-2（FACT，本 agent 自身缺陷，已修）第 9 步第一版是**空断言**

「撤销通道」步骤第一版把探针值写成 `{kind:"unresolved"}`，而被选中的槽位 `q27`
**原值本来就是 `{kind:"unresolved"}`**。于是断言 `kind !== "unresolved"` 恒成立，
读回来相等什么也证明不了——**只有版本号 1→2 是真实证据**。

已修正为：写入值必须与原值不同（相同则直接抛错拒绝执行），并断言读回值等于写入值
**且**不等于原值。修正后证据：`valueActuallyChanged: true`，`versionBefore=1 → 2 → 3`，
`canonicalValueAfterUndo = {kind:"text",values:["E2E-UNDO-PROBE"]}`，最后还原成功。

**教训（与 F-R5-3 / F-R5-8 同类，第三次）**：断言「某值等于 X」时，必须先确认
「不等于 X」也是可能的，否则断言会退化成恒真。已固化为脚本里的显式守卫。

### F-R7-3（FACT）契约与交接文档的状态表已过期，且检查器的盲区需要补护栏

`HANDOFF_2026-09-15_frontend_contract_drift.md` §1 与 `RECOGNITION_LOOP_CONTRACT.md` §10.2
仍写「写入面至今未修，是当前唯一阻塞」「2 处破坏性不一致」，而实际已是
**0 处破坏性 / 1 处需要留意**，写入面也已用真实 IPC 验证通过。

更值得记录的是 **F-R6-1 的盲区还在**：`contract-drift.mjs` 只比对 Rust **结构体**字段名，
**不解析 `#[tauri::command]` 的参数名**，所以「字段名全对但没包 `input`」这种漂移
它永远报「0 处破坏性不一致」。`apply_editor_commands` 也是同一签名形态。
已在交接文档里向对方提出补护栏建议（解析命令参数名，对 `Value`/`input` 形态要求前端包 `input`）。
两个文件都在契约 §1.1 划给对方的独占写入区，**我没有改动**，只新建了一份回复交接。

---

## 2026-09-15 续行 R8 findings（验收判定假绿 + 运行档案）

### F-R8-1（FACT，P0，本 agent 自身缺陷，已修）完整链报告的最终判定是假绿：`blocked` 不参与 verdict

**现象**：`scripts/e2e/tauri-cdp-product-chain.mjs` 在「发布被质量门禁拦下、`manifest.js` 根本没生成、
学生端没有任何产物」的情况下，仍然 `verdict=passed`、`process.exit(0)`。

**根因**（`tauri-cdp-product-chain.mjs` 旧 finally 块）：

```js
const failed = report.steps.filter((s) => s.status === "failed");
const blocked = report.steps.filter((s) => s.status === "blocked");
if (report.verdict !== "cannot-run") {
  report.verdict = failed.length || report.steps.length === 0 ? "failed" : "passed";
}
report.summary = { failed: ..., blocked: blocked.map((s) => s.name) };
```

`blocked` 被算出来、写进 `summary`，**然后就不参与任何判定**。只有 `failed` 会否掉通过。

**受影响的历史报告：15 份**（全部是 `publish-via-workspace-button` = blocked 却 verdict=passed）。
清单与复核命令见 `artifacts/e2e-cdp/VERDICT_DEFECT_NOTE.md`；报告**未删除**，作为缺陷证据保留。

**为什么这个缺陷特别危险**：它不在产品里，而在**验收工具**里。
产品一直在诚实地说「发布被拦下了」（`outcome=blocked_by_quality_gate`、`preflight.passed=false`、
`manifestExists=false`），是**报告层**把这份诚实结论翻译成了「通过」。
于是「本轮真实导入 → 发布 → 学生端」明明一步没走完，却有一份绿色的证据可以引用。

**修复**：判定抽成纯函数 `scripts/e2e/lib/chain-verdict.mjs` + 19 条回归测试
（`scripts/e2e/lib/chain-verdict.test.mjs`，已并入 `npm test`）。规则：

| 情形 | verdict | 退出码 |
|---|---|---|
| 必需步骤缺失/未执行 | `incomplete` | 2 |
| 必需步骤 failed（**任何**步骤失败都算，不只必需步骤） | `failed` | 1 |
| 必需步骤 blocked（含发布被门禁拦下） | `blocked` | 4 |
| 步骤全过但产物不完整（manifest / 题目 JS / 资源清单） | `failed` | 1 |
| `--expect-blocked` 且门禁正确拦下 | `passed-negative-case` | 0 |
| `--scope=edit-preview-specialty` | `passed-specialty` | 0 |
| CANNOT-RUN | `cannot-run` | 3 |

**实测对照**（同一夹具、同一二进制，`demanding-reading-passage-3.pdf`）：

```text
修复前：  verdict=passed    exit=0     ← 假绿（publish blocked，manifest 未生成）
修复后：  verdict=blocked   exit=4     ← 同一个运行，如实报告
负例模式：verdict=passed-negative-case exit=0
```

**顺带修掉的第二个判定洞**：判定原本写在 `if (recorder) {...}` 里。
CANNOT-RUN 时 `recorder` 还是 `null`，于是这种运行**根本不执行判定**，
`verdict` 停在下方的初值、`exitCode` 是 `undefined`。
实测暴露：一次 `staleBuild` 的 CANNOT-RUN 报出 `verdict=failed exit=undefined`。
已改为**无条件判定**，并把 `verdictReason` 一并写进报告。

### F-R8-2（FACT）本沙箱下「默认档案」（不带测试专用安全参数）跑不起来

任务书要求「不含测试专用安全参数的运行证据，与 CDP 诊断运行分开记录」。
为此把 `--no-sandbox --disable-gpu` 从默认值改成**显式开启**（`--diagnostic-args`），
并新增 `runProfile`：`cdp-default`（无测试专用安全参数）/ `cdp-diagnostic`（有）。

**受控对照实验**（假设 → 运行 → 结果）：

- 假设：`--no-sandbox --disable-gpu` 在本沙箱是必需项，不是可选项。
- 运行 A：`--pdf demanding-reading-passage-3.pdf`（默认档案，`securityArgs: []`）
  → 12 步中 11 步 **failed**，全部报 `CDP 连接已关闭`；`library-page-loads` 就在等
  `library-page-after-reload` 时超时，app 输出停在 `[library] v2 migration: scanned=0 …`，
  `appProcessExitCode=null`。即 renderer 在第一步之前就死了。
- 运行 B：同一夹具 + `--diagnostic-args` → 11 passed / 1 blocked（正常）。
- 结果：假设成立。**本沙箱内所有可得证据都是 `runProfile=cdp-diagnostic`。**
- 停止条件：已确认差异可归因于这两个参数（唯一变量），不再重复试验。

**如实记录**：因此本轮**没有**任何 `cdp-default` 的通过证据；默认档案在本环境
只能得到 CANNOT-RUN 级别的失败。这一条不得被表述为「默认产品路径已验证」。

### F-R8-3（FACT，工具摩擦）`package.json` 的脚本级改动会触发 staleBuild

`assertBuildFresh` 按 mtime 比对 `package.json`，所以只加一条 npm script 也会判 staleBuild
（实测：加 `e2e:chain` 等脚本后，下一次运行 CANNOT-RUN 指向 `package.json @ 22:39:18 > exe @ 22:34:20`）。

`scripts` 不影响构建产物，只有 `dependencies`/`devDependencies` 影响。
但**没有**按「依赖指纹」收窄这个检查：判定构建是否新鲜需要构建时刻的指纹，
而构建由 `tauri build` 完成、不落这份状态，靠 mtime 是当前唯一可靠手段。
收窄会削弱「exe 与源码一致」这条安全属性，而绕过手段（`--tolerate-concurrent-edits`）
会把容忍项**逐文件记进报告**，本来就是透明的。

**因此采取的做法**：不改判定规则；需要引用证据时**先重建再运行**（保证 `tolerated` 为空），
非产物相关的容忍（如仅 `scripts` 变化）用 `--tolerate-concurrent-edits` 并在报告里留痕。

---

## F-R9-1（P0，产品）：发布阻塞的最后一公里是门禁误判，不是数据

**结论**：`WORD_LIMIT_UNPARSED` 在一个**按字母选的 summary completion** 上被误判为 blocking，
导致 `quality.state` 恒为 `blocked`、发布永远被拦。

**证据**（`artifacts/e2e-cdp/run-publish-attribution-2026-09-15T23-02-36-674Z`）：

```text
group-1.taskType        = summary_completion
group-1.wordLimit       = undefined                      ← 门禁因此判 blocking
group-1.normalizedText  = "Questions 27 - 31 Complete the summary using the list of words
                           and phrases, A-H, below. Write the correct letter, A-H, in boxes 27-31 …"
```

题目要求填**字母 A–H**，这类 summary completion 在 IELTS 里本就没有 word limit，
源文里也确实没有。而 `quality.rs:1548-1563` 对**所有** completion 类型都要求 `wordLimit` 存在。

**为什么这条最要紧**：它是本轮「先解除真实发布阻塞」的唯一残余障碍。
同一夹具上，走真实界面填完 14 个答案之后 `hardFailures` 已经从
`["WORD_LIMIT_UNPARSED","ANSWER_KEY_MISSING_SLOT","RUNTIME_COMPILER_FAILED"]` 降到
`["WORD_LIMIT_UNPARSED"]` —— 也就是说**其余阻塞都真的被数据修复清掉了**。

## F-R9-2（P0，产品）：「可见、可点、不可解决」的问题

`WORD_LIMIT_UNPARSED` 的 `suggestedActions = ["edit_text","confirm_table"]`，两个动作都是死路：

- `edit_text`：判定读 `instructionSignature.wordLimit`（`quality.rs:1362`），
  而 `instructionSignature` 的写入者只有 `set_task_type`（只改 `taskType`）与
  `set_question_expression`（只改 `expectedQuestionNumbers`/`expectedSlotCount`）。
  **`replace_text` 不在其中** —— 用户改多少文字都不会重算 `wordLimit`。
- `confirm_table`：门禁无视 `resolution`（见 F-R9-3），标成 `ignored` 也不会放行。

于是问题列表给出一个「点得动、但永远不会消失」的阻断项。
这正是任务书要求验证的「用户能完成修复，而不只是看到错误」——答案是**不能**。

## F-R9-6（P0，产品）：`edit_text` 是死路 —— 运行时已证实

F-R9-2 原先只有代码层结论（「没有写入路径，就不可能被改掉」）。现已补上运行时证据。

保存路径 `apply_patch` → `refresh_quality_report`（只读存储的 signature）→ `validate_authoring`
（纯 serde schema 校验），**没有一处重算 `instructionSignature`**。

实测（`artifacts/e2e-cdp/run-publish-attribution-2026-09-15T23-14-01-281Z`）：
把 `group-1-instructions-text` 改成 `原文 + " Write ONE WORD ONLY."`（正是 `edit_text` 建议的动作），
补丁成功、**版本 16 → 17（真的保存了）**，而：

```text
instructionSignature.normalizedText  unchanged = true   (670 字 → 670 字，逐字节相同)
instructionSignature.wordLimit       undefined → undefined
WORD_LIMIT_UNPARSED                  still blocking
wordLimitCleared                     = false
```

**说明文字改了、存了，签名一个字都没动。** 用户按问题给的建议动作做了，问题不会消失。

## F-R9-7（探针自身，已修）：点击点落在行内元素包围盒的空白处

第一次尝试编辑说明文字失败（`notes: ["group-1: 没能进入原位编辑器"]`），
看上去像「说明文字不可编辑」——**那是探针的假阳性，不是产品缺陷**。

原因：该节点是 670 字、跨多行的**行内** `<span>`。`clickSelector` 点的是**联合包围盒中心**，
而对跨行行内元素来说那个点可能落在空白区域（某行较短时），点击于是落在父元素上、进不了编辑。
改成点「首行靠左边缘」（`r.left + 6, r.top + 8`）后编辑器正常打开
（`run-…T23-16-59-183Z`，`path = real-ui-inline-editor`）。

**结论**：说明文字**可以**在真实 UI 里原位编辑。探针现在记录 `clickStrategy`
（`first-line-edge` / `bbox-center`），避免再把「点偏了」报成「产品不可用」。
**这正是本轮第三次由我自己的工具造成的假结论**（前两次见 F-R9-5）。

## F-R9-3（P0，门禁）：同一份预检、同一个动作、两个答案（运行时复现 ×2）

受控 A/B：唯一变量是把所有 blocking 问题标成 `ignored`。两个独立夹具、三次运行结果一致：

| blocker 码 | 标成 `ignored` 之后 | 对应代码 |
| --- | --- | --- |
| `ISSUE_UNRESOLVED` | **清掉** | `authoring_v2_commands.rs:455-468`（读了 resolution） |
| `QUALITY_HARD_FAILURE` | **仍在** | `authoring_v2_commands.rs:430-438`（不读 resolution） |
| `QUALITY_NOT_READY` | **仍在** | `quality.rs:211` `has_blocking = !hard_failures.is_empty()` |

实测（`run-…T22-55-55-165Z`）：

```text
resolution-phase applyResult     = ok
resolution-phase hardFailures    = ["SLOT_HOST_MISSING","SIGNIFICANT_REGION_UNASSIGNED","RUNTIME_COMPILER_FAILED"]
resolution-phase preflight.passed = false
resolution-phase blockerCodes    = ["QUALITY_NOT_READY","QUALITY_HARD_FAILURE"]   ← ISSUE_UNRESOLVED 消失
```

这为前一轮那份**静态审读**提供了运行时证据。语义后果比「完全没生效」更糟：
用户以为处理完了，发布仍然不可能。

## F-R9-4（前端，已修）：同一根因的泛化重复

实测一份缺 14 个答案的题稿，预检返回 **34 条** blocker，其中同一个根因被表达了三次：

```text
QUALITY_NOT_READY:1, QUALITY_HARD_FAILURE:3, ANSWER_MISSING:14, ISSUE_UNRESOLVED:16
```

`QUALITY_HARD_FAILURE`（`internal` 是质量码、targetId 为空、文案泛化）在已有带具体目标的记录时
是纯重复。已在前端去掉：**真实产物回放 34 → 31**（两次独立运行一致）。

**当时故意未合并**的部分：`ISSUE_UNRESOLVED`(16) 与 `ANSWER_MISSING`(14) 描述同一个事实，
但它们是两个子系统各起的码。合并需要一条跨子系统的「码别名」权威表（后端领域）。
这一条**在本轮被部分推翻**：跨来源的同一根因确实需要别名表，而且其中一族
（本地 `ANSWER_UNRESOLVED` ↔ 门禁 `ANSWER_MISSING`）的别名**是可证明的**，见 F-R9-8。

### F-R9-4 二次核对（FACT）：原结论成立，我中途那次「更正」才是错的

原文写「`complex-reading.pdf` 的 group-2 上有**两条** `SLOT_HOST_MISSING`（同 code、同 target、
两条不同事实）」。本轮我一度把它「更正」成「两条逐字节相同，是后端重复推送」——
**那次更正是错的**，因为我比的是 **preflight 的 blocker 条目**，而 preflight 只携带
`internal = issueId`，两条的 issueId **撞了**，所以看起来逐字节相同。

回**质量报告**（`issues` 数组，不是 preflight）核对，两条确实是**两个不同事实**：

```json
{"issueId":"phase4-SLOT_HOST_MISSING-group-2","code":"SLOT_HOST_MISSING","targetId":"group-2",
 "suggestedActions":["edit_text","confirm_table"],"message":"completion slot 没有可渲染的宿主节点。"}
{"issueId":"phase4-SLOT_HOST_MISSING-group-2","code":"SLOT_HOST_MISSING","targetId":"group-2",
 "suggestedActions":["confirm_table","edit_text"],"message":"table completion 没有可渲染的 table stimulus。"}
```

来源：`artifacts/e2e-cdp/run-publish-attribution-2026-09-15T22-55-55-165Z/report.json`
（探针保留的原始 `issues`，含 `raw`）。

**真正的缺陷比原来写的更严重，而且是两层的：**

1. **`issueId` 不是唯一键。** 后端 `issue()` 把 `issueId` 拼成 `phase4-{code}-{target_id}`，
   于是「同一个 code + 同一个 target 上的两个不同事实」得到**同一个 issueId**。
2. **preflight 因此丢信息。** `ISSUE_UNRESOLVED` 只带 `internal = issueId`，
   两条不同事实在 API 边界上变成**两条不可区分的记录**。前端拿到的东西里
   已经没有「这是两条」的信息了 —— 所以前端按 `code:targetId` 收成一行，
   **会静默吞掉其中一个事实**。

推论（对任务二/任务三都成立）：`resolution` 是按 `issueId` 存的。既然 issueId 会撞，
**按 issueId 继承 resolution 是不安全的** —— 这正好印证任务书那句
「`resolution` 只能继承到『仍然是同一个事实』的问题上（不能只按旧 `issueId`）」。

**前端这边的处置**：不去假装能分开（数据里已经分不开了），而是**把这个损失变成可见的**——
校验脚本会统计「门禁里同键但 `userMessage` 不同的条目」，把它们记进报告；
PDF 夹具上应为 0，`complex-reading.pdf` 上应为 2。真正的修法在后端（见 F-R9-11）。

**教训（这次是双重的）**：我先是**对**的，然后被自己一次未经核对的「更正」带偏。
两次都犯同一个错：**拿二手字段（计数 / 已被压平的 preflight）去推断一手事实**。
断言「A 与 B 相同/不同」之前，必须看 A、B 本身。

## F-R9-11（P0，后端，未修，已交接）：`issueId` 不唯一 → preflight 丢事实 + resolution 不安全

见上面 F-R9-4 二次核对。三件事需要后端定：

1. `issueId` 加判别位（例如带上一个稳定序号或 message hash），让它**真的是唯一键**；
2. `ISSUE_UNRESOLVED` 的 `internal` 别只放 issueId，至少能让前端看出「这是两条」；
3. `resolution` 的继承判定不能只看 issueId（撞键时会串）。

**未修**：`quality.rs` / `authoring_v2_commands.rs` 是后端领域（共享文件我只 append）。
已在交接文档 §5/§6 报给后端。

## F-R9-8（P0，前端，已修）：同一根因因「两个子系统各起一个码」而重复占行

**这是 F-R9-4 里那条「故意未合并」的同族问题，但发生在本地的码与门禁的码之间。**
实测 `demanding-reading-passage-3.pdf`：14 个答案位未解析，于是同一道题渲染**两行**：

| 来源 | code | 严重级 | 文案 |
| --- | --- | --- | --- |
| 本地闭包 `deriveActionableIssues` | `ANSWER_UNRESOLVED` | warning | 第 27 题还没有答案。 |
| 发布门禁 `check_publish_preflight` | `ANSWER_MISSING` | blocker | 这道题还有答案没有填写。 |

去重键是 `code:targetId`，两个码不同 → 撞不上 → 46 行里有 14 行是纯重复。

**为什么这次的别名是可证明的（而不是猜的）**：两个实现用的是**同一个谓词**。

```rust
// src-tauri/src/authoring_v2_commands.rs:439-446
.filter(|(_, value)| value.get("kind").and_then(Value::as_str) == Some("unresolved"))
```
```ts
// src/features/editor/actionableIssues.ts（本地闭包）
if (!answer || answer.kind === "unresolved") { ... }
```

两边都在筛 `answerKey[slot].kind === "unresolved"`，targetId 也都是那个 slotId。
同一个谓词、同一个目标 → 同一个事实、同一个用户动作。所以：
`ROOT_CAUSE_ALIASES = { ANSWER_UNRESOLVED: "ANSWER_MISSING" }`（**只登记这一族**，
别名表越短越安全）。合并时保留**本地**文案（带题号，比门禁的「这道题…」具体）
但把级别提到 blocker（发布确实被它拦下）。

**真实界面证据**（`run-issue-list-2026-09-15T23-31-41-068Z`，8 条断言全通过）：

```text
gate raw=34 warnings=1 expectedGate=18 expectedLocal=14
rendered=32
同一根因（归一后）在界面上只占一行 :: {"duplicated":[],"total":32}
门禁来源的行数 = 独立算法算出的期望 :: {"rendered":18,"expected":18}
本地来源的行数 = 独立算法算出的期望（没有被多删）:: {"rendered":14,"expected":14}
```

46 → 32 行，14 行纯重复消失，且**本地那 14 行一条没少**（防止用「多删」凑数）。

## F-R9-9（P1，后端，未修，已交接）：`BLOCKER_LIST_TRUNCATED` 说「仅显示前 20 条」，但什么都没截

```rust
// src-tauri/src/authoring_v2_commands.rs:430 / 447 / 469  每类各自 take(20)
for failure in hard_failures.iter().take(20) { ... }
for slot_id in unresolved_answers.iter().take(20) { ... }
for issue in unresolved_blockers.iter().take(20) { ... }
// :480  但触发条件是**总数** >= 20，且返回的是**完整**数组
if blockers.len() >= 20 {
    warnings.push(json!({ "code": "BLOCKER_LIST_TRUNCATED", "message": "问题较多，仅显示前 20 条。" }));
}
```

两个问题：
1. **本夹具上什么都没被截断**（hardFailures 3、未解析答案 14、unresolved blockers 16，
   三类都没到 20），但总数 34 ≥ 20 → 照样发警告。界面于是出现
   「仅显示前 20 条」旁边**列着 46 行**的自相矛盾（实测 `rendered=46` 那次运行）。
2. 截断是**按类**发生的，文案却按**整表**说，且不说是哪一类被砍。真出现 30 条
   hardFailures 时，用户会被砍掉 10 条而不知道少看的是什么。

**没有修**：`authoring_v2_commands.rs` 是后端领域文件（我只在共享文件上 append）。
已在交接文档里报给后端。

## F-R9-10（我自己的工具，已修）：期望行数算法只算了「门禁那半」

第四次自查出的假结论。第二版脚本的期望算法只对 `get_publish_preflight` 的返回建模，
而界面的行是 `mergePublishGateIssues(本地闭包, 门禁)`。于是：

```text
[assert] FAIL 渲染行数 = 独立算法算出的期望行数 :: {"rendered":46,"expected":32}
```

我一度准备把它记成「产品多渲染 14 行」。核对后：多出来的 14 行**正是**本地闭包对
14 个未解析答案位产生的 warning，产品行为是对的，**是我的算法少算了一半**。

修法（同时避免下一次再犯）：
- 期望拆成两半：`expectedGateRowCount(blockers, warnings, localKeys)` 与
  `expectedLocalIssues(ds)`（后者独立重写产品语义，且它的键**要占位**——被吸收的
  14 条门禁 `ANSWER_MISSING` 不能再算一次）；
- 断言按 `data-issue-source` 分开比，而不是拿总数比；
- 加「本地那半没有被多删」这条反向断言，堵住「靠多删满足去重」的假绿；
- 为了让脚本能分辨来源，给行加了 `data-issue-code` / `data-issue-source`
  两个机器可读属性（`ExamWorkspacePage.tsx`）。**光看 code 分不出来源**：
  本地与门禁都可能写 `ANSWER_MISSING`。

**教训**：独立算法要独立**完整**。只对一半输入建模，比没有断言更危险 ——
它会把「我算漏了」稳定地呈现成「产品错了」。

## F-R9-5（探针自身，已修）：字段名猜错会造成反向结论

第一版探针读 `issue.actions`，而后端 `issue()` 真正写出的字段是 **`suggestedActions`**
（`quality.rs:3668`）。结果是所有问题都显示成「没有建议动作」——那是探针的假阴性。
**教训**：诊断脚本读后端字段时，必须先从构造函数确认字段名，不能按直觉猜；
已在探针里两个名字都读并保留原始对象。

同一类错误在阶段 A 也发生过一次：选项位回退到「第一个 optionBank 的标签」，
而 `optionBanks` 里 group-1/group-3 是空数组，于是 9 个槽位被探针自己跳过、
却被记成了「没有 UI 入口」。真实情况是那 9 个槽位各有 3–4 个可见单选项控件。
已改为**直接问 DOM 要合法标签**，并把 `probeSkippedSlots` 与 `slotsWithoutUiEntry` 分开统计；
探针自跳过任何槽位时判定记为 `inconclusive-probe-incomplete`，不允许得出「改完也不行」的结论。







---

# 2026-09-16 续行 R10 findings（去重身份 + 验收判定 + 持久化撤销）

## F-R9-12（P0，前端，已修）：按门禁 code 去重会**整条吞掉**一个阻断问题

`mergePublishGateIssues` 原先用 `code:targetId` 当去重键。门禁把**所有**质量码都塞进
`ISSUE_UNRESOLVED` 这**一个** code 里，于是两个**完全不同的**阻断问题共用
`ISSUE_UNRESOLVED:document`，第二条被当成重复丢掉。

实测 `complex-reading.pdf`：门禁 **8 条** blocker，界面只剩 **3 行** ——
`SIGNIFICANT_REGION_UNASSIGNED`（「仍有显著源区域未被…解释」）**完全不可见**。
它不是被折叠，是**从来没有渲染过**。用户会看到「问题都处理完了」而发布仍被拦。

来源：`artifacts/e2e-cdp/run-issue-list-2026-09-15T23-36-44-422Z/report.json`。

**修**：去重键改成「根因 + 目标」，根因从 `internal`（`phase4-<质量码>-<目标>`）读出来
（`rootCauseOf`）。修复后同一夹具渲染 5 行，两个 `document` 根因各自成行。

**这条是「不同问题被隐藏」，与「同一问题重复显示」是相反方向的错误** ——
任务书明确要求两者同时检查，不能只看条数变少。

## F-R9-13（P0，前端，已修）：把「根因 + 目标」当唯一身份 → 隐藏 group-2 的第二条事实

**这是上一轮我自己的修法带来的新错误，方向相反。**

修完 F-R9-12 之后我把 `根因 + 目标` 当成了**唯一身份**。但实测 `complex-reading.pdf` 的
group-2 上有**两条不同事实**：

```json
{"issueId":"phase4-SLOT_HOST_MISSING-group-2","suggestedActions":["edit_text","confirm_table"],
 "message":"completion slot 没有可渲染的宿主节点。"}
{"issueId":"phase4-SLOT_HOST_MISSING-group-2","suggestedActions":["confirm_table","edit_text"],
 "message":"table completion 没有可渲染的 table stimulus。"}
```

两条的 `issueId` **撞了**（`issueId` 拼成 `phase4-{code}-{target}`，不是唯一键），
到了 preflight 只剩 `internal = issueId`，于是「根因 + 目标」也分不开这两条 ——
按它去重又是**吞掉一条**。

**修**：`sameFact()` 分两种情形判，都是可解释的：
- **跨来源**（本地闭包 vs 门禁）同根因同目标 = 同一件事。两个子系统各写一句文案是
  **设计如此**（本地「第 27 题还没有答案。」vs 门禁「这道题还有答案没有填写。」），
  文案不同**不能**当两条事实 —— 否则同一道题白占两行（实测 14 道题多 14 行）。
- **同来源**还要 `userMessage` 也相同才算同一条。文案不同就是两条事实，**两条都留**。

修复后 `complex-reading.pdf` 的 group-2 渲染 **2 行**（两条事实都在）。

**根因仍在后端**（见 F-R9-11）：`issueId` 必须真的唯一，`resolution` 才能安全地按它继承。
后端给出稳定事实 id 之前，前端只能退到「来源 + 文案」来区分 —— 这是权宜之计，
已在代码注释里写明。

**教训（两次都犯同一个错）**：先按 code 去重（吞根因），再按「根因+目标」去重（吞事实）。
两次都是**用了一个不够细的键，并把它当成权威身份**。正确做法是先问
「这条键凭什么能唯一标识一个事实」，答不上来就不能用它去重。

## F-R10-1（P0，验收判定，已修）：必需步骤的 skipped / not-executable / 未知状态会漏到 `passed`

`computeChainVerdict` 原先只查三样：`missing`（步骤名不在数组里）、
`failed`（`status === "failed"`）、`blocked`（`status === "blocked"`）。
于是状态是 `skipped`、`not-executable`、`pending`，或者干脆**拼错**的必需步骤，
三样都不占，**直接走到最后的 `passed`**。

最坏的一条路径：发布被门禁正确拦下 + 另一步 `skipped` → `otherBlocked.length === 0`
成立 → 判 `passed-negative-case`（退出码 0）。**负例模式因此会变成一条永远绿色的通道。**

**修**：加 `notOutcome`（必需步骤里状态不在 `{passed, failed, blocked}` 的），
必须排在 `blocked` / 负例之前判；`not-executable` 单独判 `not-executable`(5)，
其余判 `incomplete`(2)。并在最后加一条自检：`passed.length === required.length`，
将来规则被改坏也会被拦住，而不是让没覆盖到的状态静默变绿。

**不是白名单枚举**：列出的是「有结论」的三个状态，其余一律按「没有明确结论」处理 ——
免得将来冒出第四个状态又漏过去。

**回归**：+7 条（`scripts/e2e/lib/chain-verdict.test.mjs`，26 → 33）。
真实 recorder 只产出 `passed/blocked/failed`，所以这条是**纯护栏**，不改变现有行为。

## F-R10-2（P1，验收脚本，已修）：过期候选以「按钮消失」为成功证据

`stale-suggestion-protected` 原先在「接受按钮不存在」时**直接 return 成功**：

```js
if (!hasAccept) {
  return { ..., acceptAvailable: false, note: "旧建议已不可接受（过期/已处理），用户修改未被覆盖" };
}
```

既没读值、也没看决策状态、更没重开。按钮消失可能只是面板渲染问题，
而权威稿里用户的值**可能已经被覆盖** —— 那样这个场景会给出**假绿**。

**修**：三条都必须成立，缺一条判失败：
1. **用户内容**：读权威稿比对（接受尝试之后、重开之后各比一次）；
2. **实际 outcome**：后端 `stale` 必须为真（它由 `baseEditVersion` 与当前版本比较得出，
   是**后端事实**，不是按钮可见性），且该决策有确定的 `resolution`；
3. **重开持久化**：① ② 与 `resolution` 重开后一致。

`acceptAvailable` 降级为**线索记录**，不参与判定；返回里加
`protectedEvidence: "user-value-intact + backend-stale-flag + survives-reopen"`
把通过依据写清楚。

## F-R10-3（P0，前端，已修）：撤销以会话内 `Set` 作为完成依据

`RecognitionPanel` 用一个会话内的 `Set<decisionId>` 记「我点过撤销」，并据此显示
「已撤销」。**刷新页面/重开/换会话，这个 Set 就没了** —— 界面又会把「撤销」按钮放回来，
而用户以为已经撤销完了。这正是任务书说的「以会话内 Set 作为完成依据」。

后端**没有**持久化的「已撤销」状态：`RecognitionResolutionV1` 只有
`agreed | auto_fixed | needs_review | unverifiable`，`build_view` 还会把 `auto_fixed`
的项无条件留在 `autoApplied` 里。所以**不能靠状态码判**，也**不能凭空发明一个**。

**修**：撤销的语义本身是**可观测的持久化事实** —— `undo` 补丁带着「改回哪个值」，
只要权威稿里那个答案位已经等于它，撤销就已生效（无论是刚点的、上次会话点的、
还是用户自己手改回去的）。新增纯函数 `isUndoAlreadyApplied(undo, answerKey)`，
面板改成按 `answerKey` 现算，删掉会话 `Set`。重开/刷新后判定不变。

**回归**：+7 条（`recognitionDecisions.test.ts`，25 → 32）。

## F-R10-4（构建，**未查明原因**）：`npx vite build` 瞬时失败一次

实测一次：`npx vite build` 失败（只有一帧栈 `at async Object.defaultBuildApp`），
随后 `npx tauri build` 因为 `dist/` 没更新而在 1.05s 内「成功」返回，
于是 exe 相对源码变旧 —— 被新鲜度护栏正确拦下为 `cannot-run`(3)，**没有产生假绿**。

**未能复现**：同一目录、同一命令随后连续成功（`vite exit=0` / `tauri exit=0`）。
没有残留的 `ielts-author-studio.exe` 进程（已用 `tasklist` 确认），因此不是「应用占着 dist」。

**我自己的工具缺陷（已修）**：当时把构建输出接进了 `| tail -2`，**管道把退出码换成了 `tail` 的 0**，
于是 `&&` 继续执行、真正的原因行被截掉。现在改为：完整日志落盘
（`artifacts/vite-build.log`、`artifacts/tauri-build.log`）+ 显式检查退出码 + `set -o pipefail`。

**诚实边界**：**原因未查明**，我不编一个解释。可依赖的是护栏：
构建失败不会被当成通过，而是 `cannot-run`。

---

# R11（2026-09-16）后端交付已到，但**交付物编译不过** —— 任务 6 仍不可开始

## F-R11-1（P0，后端，**未修**）：后端交付的 `auto_pipeline.rs` 无法编译，全链与全部 E2E 被挡

**背景**：R10 时我判断任务 6「被后端阻塞」。本轮复核发现后端**确实已交付** ——
`src-tauri/src/reconcile/**`（`mod reconcile;` 已注册进 `lib.rs:87`，两个命令薄壳在
`lib.rs:972/986`）、`src-tauri/src/schema/recognition_v1.rs`、两个契约 JSON Schema，
且写入面契约漂移已收敛为 **0 处破坏性不一致**（`node scripts/recognition/contract-drift.mjs`，exit=0）。
也就是说：**R10 记的「等后端交付」这一条已经满足。**

**但交付物不编译。** 两次独立构建，错误完全一致：

| 错误 | 位置 |
|---|---|
| `E0596` cannot borrow `entry` as mutable | `src/auto_pipeline.rs:564:9` |
| `E0716` temporary value dropped while borrowed | `src/auto_pipeline.rs:595:43` |
| `E0716` 同上 | `src/auto_pipeline.rs:607:43` |
| `E0716` 同上 | `src/auto_pipeline.rs:615:43` |

`error: could not compile `ielts-author-studio` (lib) due to 4 previous errors`。

**归属已核实**：`git diff --stat src-tauri/src/auto_pipeline.rs` = `446 insertions(+), 0 deletions(-)`，
全部是**未提交新增**（`extract_docx_plain_text` / `docx_part_plain_text`）。
`auto_pipeline.rs` 按契约 §1.1 属后端独占写入面，我**只读**，因此**未改动**。

**后果**：无法产出可验收二进制 → 完整链、问题列表校验、候选按钮流程全部 `cannot-run`。
这不是「候选为空」那种内容层阻塞，而是**构建层阻塞**：连跑都跑不了。

**为什么这比 R10 的判断更严重**：R10 我写「等后端交付即可跑」。现在交付到了，
却发现交付物本身是坏的 —— 说明「交付」不等于「可验收」。**交付的判定标准必须是
「在稳定点上能编译并跑通链路」，不是「文件出现了」。**

## F-R11-2（P1，协作）：工作树在验证期间被并发修改，导致结果不可归因

用 `find -printf '%T@ %s %p'` 在每次构建前后取指纹对比，**两次构建期间都有后端写入**：

| 构建 | 期间被改的文件 | 大小 |
|---|---|---|
| 第 1 次 | `processing/scheduler.rs` | 41873 → 42620 |
| 第 2 次 | `ielts_grammar/instruction_signature.rs` | 20018 → 20047 |
| 第 2 次 | `ielts_grammar/quality.rs` | 162185 → 164972 |

看到 `quality.rs` / `instruction_signature.rs` 在动，推测正在处理 R9 交接单第 2 条
（`WORD_LIMIT_UNPARSED` 误判）——方向正确，但此刻树不编译。

**方法论教训**：构建/验证必须**同时**记录「构建前后源码指纹」，否则
「这次构建对应哪个版本」是无法回答的。上一轮我用 exe mtime 判新鲜度（被动），
本轮改成**主动指纹对比**，才看见并发写入。没有这个对比，我会把
「某次构建失败」错误地归因给一个已经不再存在的版本。

## F-R11-3（已确认，非缺陷）：写入面契约漂移已收敛

R9/R10 期间交接单（`HANDOFF_2026-09-15_frontend_contract_drift.md`）记「写入面未修」，
其后果是「点『采用修正』是静默空操作」。本轮实测：`recognitionClient.ts` **已按交接单 §3.1 修好**
（对外 `decisions[]`，对内发 `{requestId, batchId, baseEditVersion, accept[], reject[]}`，
并把 `outcomes[]` 归一成 `accepted/rejected/stale/failed`）。

`node scripts/recognition/contract-drift.mjs` → **`契约漂移检查：0 处破坏性不一致，1 处需要留意`**（exit=0），
唯一提示是 `RecognitionDecisionViewV1` 上多出 `jobId`/`generatedAt`（仅提示，非破坏性）。

**结论**：那份交接单的 §1/§3「写入面仍未修」**已过期**，不应再被引用为当前状态。
（这正是「二手结论会过期」的又一例：交接单是别人写的快照，不能替代当场复测。）

---

## R11 续（同日 11:11 之后）：后端修好构建；稳定事实 id 已交付并采用

### F-R11-1 已解除：后端在 11:11 自行修掉那 4 个编译错误

`auto_pipeline.rs` 于 `11:11:22` 被后端再次修改（141877 → 142012）。随后构建：

```text
vite build    → VITE_EXIT=0
tauri build   → TAURI_EXIT=0   error_count=0
构建期间并发写入 → 无（指纹 diff 为空）
```

**交接单 `HANDOFF_2026-09-16_build_broken_blocks_all_e2e.md` 的阻塞已失效**，
不必再按「构建坏了」处理。保留该文件作为历史（它记录的 4 个错误、行号与归属核实仍然成立）。

### F-R11-4（P0，已交付并采用）：后端给出**稳定事实 id**，任务书第 3 条的前置条件已满足

任务书第 3 条原文：「等待后端稳定事实 ID；在此之前不能把 rootCause + targetId 当作可靠唯一身份。」

后端已交付。`src-tauri/src/ielts_grammar/quality.rs` 的 `issue()`：

```rust
// The previous scheme `phase4-{code}-{target_id}` collapsed two genuinely different
// problems reported against the same target into one id (e.g. the two
// `SLOT_HOST_MISSING` variants in `validate_completion_host`...).
//   (1) two different facts on the same target get different ids; and
//   (2) the same fact recomputed on a later save yields an *identical* id
"issueId": format!("phase4-{code}-{target_id}-{slug}")
```

它同时修掉了 F-R9-12/F-R9-13 的**两个相反方向**：撞键（吞事实）与不可复算。
`preflight` 把该 id 放进 `ISSUE_UNRESOLVED` 的 `internal`（`authoring_v2_commands.rs:478`），
所以前端**确实拿得到**。

**我做的采用（`src/features/editor/actionableIssues.ts`）**：

- 新增 `factId?: string` 与 `factIdOf(blocker)` —— **只有 `ISSUE_UNRESOLVED` 的 `internal` 才是 id**；
  `QUALITY_HARD_FAILURE` 的 `internal` 是质量码本身，取它就把质量码当成了 id。
- `sameFact()` 里把 `factId` 当**单向判据**：id 不同 ⇒ 一定是两条事实（哪怕文案逐字相同）；
  id 相同或某一侧缺失 ⇒ **不作结论**，继续比文案。
  **单向是关键**：旧载荷的 id 会撞键，拿它「断言同一」就会重演 F-R9-13。
  单向使用保证**合并永远不比上一轮更多**。
- **顺带修掉一个真实缺陷**：门禁行的 `issueId` 原本拼成 `gate:{根因}:{目标}:{code}`，
  在「同 code + 同目标的两条不同事实」上撞键 —— 而它就是 React 的 `key`
  （`ExamWorkspacePage` 的 `<li key={issue.issueId}>`）。现在用 `factId` 当 key。
- 行上新增 `data-issue-fact-id`，校验脚本据此断言。

**真实二进制验证**（exe `11:16:55`，含本轮前端改动；`runProfile=cdp-diagnostic`）：

| 运行 | 结果 |
| --- | --- |
| `run-issue-list-2026-09-16T10-17-13-157Z`（PDF） | **8/8 断言通过，`verdict=passed exit=0`**；`expected=16, renderedDistinct=16, missing=[], duplicated=[]`；`rendered=32`，逐键 `mismatches=[]` |
| `run-issue-list-2026-09-16T10-17-56-776Z`（complex-reading 负控） | **7/7 通过，`exit=0`**；`expected=4, renderedDistinct=4, missing=[], duplicated=[]`；`rendered=5`（3 → 5），`mismatches=[]` |

即：**稳定事实 id 全部到达界面，且不隐藏、不重复** —— 两个方向同时成立。

### F-R11-5（已解除）：预检现在尊重 `resolution`

`authoring_v2_commands.rs` 的 `unresolved_blockers` 过滤条件新增：

```rust
&& !matches!(issue.pointer("/details/resolution").and_then(Value::as_str),
              Some("resolved") | Some("ignored"))
```

这正是 R10 交接单第 2 条「门禁无视 `resolution`」。**已修**，
「用户看得到错误、永远修不掉」的死路不再由这一条造成。

### F-R11-6（P0，未解除）：**无云 profile 时候选按钮流程结构上不可达**

任务书第 6 条要求「完成真实候选按钮流程」。实测 5 个场景全部 `not-executable`，
报告里的事实是：

```json
{ "batchId": null,
  "chainStates": { "adjudication": {"state":"not_run"}, "cloud": {"state":"not_run"},
                   "local": {"state":"not_run"}, "source": {"state":"not_run"} },
  "actionableCount": 0, "needsReviewCount": 0, "autoFixedCount": 0 }
```

**不是夹具碰巧没有候选，而是 reconcile 根本没跑。** 根因在
`src-tauri/src/processing/scheduler.rs`：

```rust
let launch_cloud = cloud_will_run && resolved_profile.is_some();
...
if !launch_cloud {
    // 无云端（或未解析到 profile）：本地稿即可检查
    ...advance(STAGE_READY_FOR_REVIEW, ...);
    return;          // ← 提前返回：reconcile 永远不会被调用
}
```

`resolved_profile` 来自 `job.active_llm_profile_id`（或 progress 的 `cloudProfileId`）。
E2E harness 每次运行都用**全新的临时数据目录**（`PDF2TEST_AUTOMATION_DATA_DIR`），
因此**一个 profile 都没有** → `launch_cloud=false` → 提前返回 → `batchId=null`。

**两个后果**：

1. 任务书第 6 条的「真实候选按钮流程」在**任何无凭据环境**（含 CI）里都跑不了 ——
   这不是脚本缺陷，是产品流程的前置条件。要跑通，需要真实云 profile，
   或给 harness 一个「指向 localhost 的失败 profile」以便 `launch_cloud=true`
   （云端会失败，但 `Some(Err)` 分支仍会进 reconcile，候选来自本地 vs 原文件核验）。
   **这是一个测试设计决定，我没有单方面去造**。
2. 契约 §3.2 宣称 `source`（原文件核验）是一条独立链、有自己的 `state`/`reasonCode`，
   §4.2 也列了 `ANSWER_SOURCE_*` 一族。但**无云时整段 reconcile 被跳过**，
   于是「只有原文件核验」这条路径**在产品里不可达**。这值得后端确认是有意为之还是缺口。

**这一条不因 F-R11-1 的解除而解除**：构建好了、后端交付到了，
但候选流程仍需要云 profile 才能产生对象。

---

## R12（2026-09-16 14:00–14:40）：构建归因、无云四链与撤销协议的当前事实

本轮任务书六项产品集成。开工先做现状核查，随后被两个**后端在途缺陷**挡住
（F-R12-1 编译、F-R12-2 顺序），如实记录如下。

### F-R12-1 后端在途改动一度不编译（`E0596`×2），阻断一切真实 E2E

`src-tauri/src/reconcile/commands.rs` 在 14:04:20 被写入后，`cargo build` 报：

```
error[E0596]: cannot borrow `items` as mutable, as it is a captured variable in a `Fn` closure
   --> src\reconcile\commands.rs:801:41   （accept 分支，第 799 行的 &|tx, _version| 闭包）
error[E0596]: cannot borrow `items` as mutable, as it is a captured variable in a `Fn` closure
   --> src\reconcile\commands.rs:901:37   （undo 分支，第 899 行的闭包）
```

两处同型：闭包按 `Fn` 捕获 `items`，闭包内却要 `&mut items[i]`。属后端独占区
（`reconcile/**`），前端侧**未做任何修改**，只出交接单
`Plan With Files/Dual_Recognition/HANDOFF_2026-09-16_r12_undo_closure_borrow.md`。

**后端已于 14:08:18 自行修复**（`cargo check` 通过，仅 107 条告警）。
这是 F-R11-1 的同型复发：后端在途改动未编译就落盘，且落盘时间正好在构建进行中。

### F-R12-2 冻结快照早于权威稿播种 —— 无云与有云**两条**路径都产不出批次与候选

这是本轮最重要的发现，也是任务书第 3、5、6 项至今不通过的真实原因。

`processing/scheduler.rs` 的执行顺序：

| 行 | 内容 |
| --- | --- |
| 464 | `freeze_local_candidate_snapshot(...)` ← 需要 `get_canonical_ds` 已存在 |
| 495 | `set_item_status_ready(...)` ← 它内部（第 996 行）才调 `migrate_single_item` **播种**权威稿 |
| 508 | 冻结失败 ⇒ 拒绝裁决（14:04:47 新增的加严逻辑） |
| 529 | `run_local_only_recognition_cycle(...)` ← 因此永远到不了 |

`migrate_single_item`（`library/migration.rs:155`）是按需播种，生产路径上只有三个调用点：
`get_workspace_item_core`（`library/commands.rs:26`）、`get_publish_preflight_core`
（`authoring_v2_commands.rs:83`）、`set_item_status_ready`（`scheduler.rs:996`）——
**没有一个早于第 464 行的冻结**。而产品**不会**在导入后自动打开工作区
（`LibraryItemRow` 的 `onClick` 才 `onOpen`），所以这不是竞态，是**确定性**失败。

真实二进制（`sha256 5961c9cf…`，清单自陈 `inputsDriftedDuringBuild: false`，
即与工作树逐字节对应）上的实测：无云导入 `demanding-reading-passage-3.pdf` 后，
`processing_jobs_v2` 的行是

```
stage=ready_for_review  local_status=succeeded  cloud_status=not_run
reconcile_status=failed actionable_count=0
last_error_code=canonical_not_seeded:import-20260916131334-d7ab7039
```

而 `recognition_batches_v1` / `recognition_decisions_v1` / `recognition_decision_journal_v1`
**各 0 行**；`get_recognition_decision` 返回 `batchId=null` 且四链全 `not_run`。

**结论**：后端已把「无云分支不再提前 return」修好（`run_local_only_recognition_cycle`
确实存在且被调用），但**上游的冻结顺序**让它在第一步就失败。修好顺序后，
第 3 项（无云有批次）、第 5 项（候选按钮）才有前提。交接单：
`Plan With Files/Dual_Recognition/HANDOFF_2026-09-16_r12-2_freeze_before_seed.md`。

**有云路径同样被这一处顺序阻断**：`scheduler.rs:543–546` 在 `freeze_error.is_some()` 时
丢弃云端结果并跳过裁决。所以「接上可用云端就会有候选」目前也不成立——
在顺序修好之前，第 5 项不必再去排查前端按钮。

### F-R12-3 我自己的等待条件恒真（工具缺陷，已修）

`get_recognition_decision` 在**尚未**产生批次时也返回视图，其 `chains` 是四条 `not_run`
（后端 `load_latest_batch` 为 None 的分支）。我原先的等待条件写成
`if (view.batchId || view.chains) break;` —— `chains` 恒存在，条件**恒真**，
于是循环立刻退出，把「还没开始识别」误报成「四条链全 not_run」，
制造了一个**假失败**（第一次 local-chain 运行即如此）。

修法：判据改为「**批次出现**或**本地链进入终态**」。同一处隐患也存在于
`tauri-cdp-recognition-write-path.mjs`（其注释「只要有 batch 或 chains 就算识别已落盘」
本身就是错的），已一并修正。

### F-R12-4 「假 fresh」的根治：从 mtime 启发式改为内容哈希清单

任务书要求「构建必须关联源码、dist 和 exe，避免旧前端被新 exe 包入后仍判 fresh」。
本轮先把 `assertBuildFresh` 从「源码 vs exe」升级为**三段链**
（前端输入 → dist → exe，后端输入 → exe），并补 7 条回归（含用户点名的
「旧 dist + 新 exe」）。该护栏**当场生效**：14:05 那次失败的构建重跑了 `npm run build`
产出新 dist 却没产出 exe，护栏立刻报 `exe 早于 dist` 并拒绝验收。

但 mtime 只能证明顺序、不能证明内容，且会被两个 agent 的并发写入淹没。因此进一步引入
**构建清单**（`scripts/e2e/lib/build-manifest.mjs` + `scripts/e2e/build-app.mjs`）：

- 构建脚本显式分两步（`npm run build` → `tauri build --no-bundle --config <file>`），
  逐步取**内容哈希**，最后写 `artifacts/build-manifests/<exeSha256>.json`；
- 构建期间若前端/后端输入漂移，清单标记 `inputsDriftedDuringBuild` 并以退出码 4 退出
  ——**不可归因的二进制不得用于验收**；
- 护栏改为**内容优先**：清单三段哈希与当前工作树一致 ⇒ fresh（与 mtime 无关）；
  找不到清单才退回 mtime 三段链。前端两段不豁免，后端段可在
  `--tolerate-concurrent-edits` 下豁免并原样列出。

`scripts/e2e/lib/build-freshness.test.mjs` 现有 **14** 条回归，覆盖：
用户点名的「旧 dist + 新 exe」、`exe 早于 dist`、dist 缺失、
「内容一致但 mtime 被推新（mtime 误报、内容放行）」、
「内容变了但 mtime 未变（mtime 看不见、内容抓住）」、
后端段豁免、清单自陈漂移、exe 换内容后不误用旧清单。

本轮首次用该机制产出的可归因二进制：`sha256 5961c9cf…`，
`frontendInputs e2627a34… / dist f515ee85… / backendInputs a05b7a30…`，
`inputsDriftedDuringBuild: false`。

### F-R12-5 隔离 localhost 协议测试服务已就位（任务书第 4 项，待校准样本）

`scripts/e2e/lib/fake-llm-gateway.mjs`：独立进程、只监听 127.0.0.1 的
OpenAI 兼容 `/v1/chat/completions` 替身。它不是 mock——产品通过真实的 `reqwest`
调用访问它，与访问真实网关走同一条代码路径，因此请求体形状、鉴权头、超时/重试、
JSON 解析与校验器都被真实覆盖；它**不接触**产品的 SQLite/文件，候选只能由产品自己算出来。

已确认协议细节（供后续校准样本）：
- 端点 `{baseUrl}/chat/completions`；`http` 仅允许回环/私有/链路本地主机
  （`llm_gateway.rs:169–188`），`localhost` 合法；
- 请求体：`{model, temperature, messages:[{system},{user}], response_format?}`；
- 响应取 `/choices/0/message/content`，再把该字符串按 JSON 解析；
- `generate_pdf_reading_outline` 的输出契约：`{title, groups[], answerKey{}, confidence, warnings[]}`，
  每个 group 需 `kind`（白名单）/`layoutHint`/`notesText`/`range[2]`/`evidence.quotes[]`
  （校验器 `validate_cloud_outline_output`，`llm_gateway.rs:1191`）；
- profile 落盘 `config/llm-profiles.json`，密钥落盘 `config/secrets/<id>.key`
  （需 `EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK=1`）；`launch_cloud` 只要求**解析出 profileId**，
  不要求密钥存在。

**样本尚未校准完成**：样本要在「与本地稿结构对齐」的前提下故意分歧才能产出
`needs_review` 候选，而本地稿在 F-R12-2 修好前根本无法生成。因此本项**未完成**，
下一轮应在顺序修复后按「先取真实本地稿 → 由它派生分歧样本」的方式校准，
而不是凭空写死一份答案。

---

## R13（2026-09-16 晚）撤销接线、可归因重建与受控服务

### F-R13-1 本机安全删除护栏会让 vite 的 `emptyDir` 失败（构建基础设施）

一次完整 vite 构建产出 50+ 个 assets，而本机 `node-safe-delete` 护栏对**单回合批量删除**
有配额（实测阈值 50）。`emptyDir(dist/assets)` 因此抛
`[safe-delete][SAFE_DELETE_BULK_CONFIRM_REQUIRED] {"count":54,"threshold":50,...}`，
构建失败；更糟的是**抛之前它已经删掉一部分**，`dist` 停在半删状态（实测 54 → 剩 6）。

解法不是关护栏（那是保护用户文件的机制，不该为构建让路），而是让这次构建
**根本不需要批量删除**：`build-app.mjs` 在跑前端构建前把旧 `dist` 整体 `rename` 到
`tmp/dist-prev-<stamp>`（重命名是一次目录项操作，不是删除），vite 于是在全新空目录上构建，
`emptyDir` 面对空目录无事可做。副产品是 `dist` 内容 100% 来自本次构建，清单里的 `dist`
哈希更干净（不会混进上一版残留 asset）。`tmp/` 与 `dist/` 均已被 `.gitignore` 覆盖。

### F-R13-2 手工接受的项会从识别视图里**整体消失**，撤销闭环对它不成立（后端独占区）

`Plan With Files/Dual_Recognition/CONTROLLED_LLM_SCENARIO_2026-09-16.md` §4 明确要求：
「点接受 → 题稿 `q14` 写入 `stencilling`，该项离开待办，**撤销入口出现**」，
并强调「撤销对**手工接受**的项也应按同一闭环退出」。

代码读下来这条闭环不成立，两个独立的阻断点：

1. **呈现层把该项丢掉了。** `build_view`（`reconcile/commands.rs:202-215`）只把
   `resolution == AutoFixed && status == Accepted` 的项放进 `autoApplied`；
   其余走 `is_actionable()`（`schema/recognition_v1.rs:481-484`）＝
   `resolution != AutoFixed && status ∈ {Open, Failed}`。
   而**手工接受**一条 `needs_review` 项时，后端只改 `status → Accepted`、
   `auto_applied → false`（`commands.rs:788`），**`resolution` 保持 `needs_review`**。
   于是它既不满足 `autoApplied`（resolution 不是 AutoFixed），也不满足 `is_actionable`
   （status 不是 Open/Failed）→ **两份列表都不进**，前端 `normalizeDecisionView` 的
   `items` 由 `autoApplied + actionable` 拼出，该项直接不存在 → 界面无从渲染撤销入口。
2. **命令层也会拒绝。** 撤销分支的门是
   `if status != Accepted || resolution != AutoFixed → RECOGNITION_NOT_UNDOABLE`
   （`commands.rs:662-671`）。即使前端硬发 `undo[]`，手工接受的项也会被判不可撤销。

补充：接受路径**确实**算出了撤销补丁（`commands.rs:790` 调
`adjudicate::undo_patch_for`），所以数据层面「可回滚的目标值」是有的 ——
缺的是「把该项呈现出来」与「放开命令层的门」这两步。

**归属**：`reconcile/commands.rs`、`schema/recognition_v1.rs` 均在后端独占区，本轮**未改**，
只出交接单 `HANDOFF_2026-09-16_r13_manual_accept_undo_gap.md`。
本条的**实证**由 `scripts/e2e/tauri-cdp-controlled-service.mjs` 的
`undo-manual-accept` 场景给出（真实界面点击 + 真实权威稿 + 后端视图三处对照）。

### F-R13-3 受控模型服务样本已由后端校准（F-R12-5 的阻塞解除）

后端于 14:25 交付 `fixtures/controlled-llm/{reading-outline.json,expected-decisions.json}`
与 `scripts/controlled-llm-service.mjs`，并附前端执行文档。
F-R12-5 记录的「样本未校准」阻塞因此解除：不再需要「先取真实本地稿再派生样本」，
样本本身是后端从实测投影反推并写死、且被 Rust 用例逐字断言的唯一真源。

踩坑（后端已记录，前端同样适用）：样本里 `evidence.quotes[].pageIndex` **必须 ≥ 1**，
网关把 0 判为非法（`cloud_outline_group_quote_invalid`）。

### F-R13-4 `cloudEnabled` 根本不是应用设置字段（旧脚本里的死代码）

`AppSettingsV1`（`src/features/settings/appSettings.ts`）只有
`keepSourceFiles / nasDestination / localConcurrency / cloudConcurrency / developerMode`，
**没有 `cloudEnabled`**。因此 `tauri-cdp-recognition-buttons.mjs` 里那句
`localStorage.setItem(..., { cloudEnabled: false })` 读不回来，是**会骗人的死代码**
（读起来像「本脚本关掉了云端」，其实没关）。已删除并加注说明。

云端是否启用由**机制**决定，不是由这个设置决定：
`useImportFiles.ts:26-28` 自己 `listLlmProfiles()`，只要存在一个
`enabled && profileId !== "profile-local-placeholder"` 的 profile，就
`cloudEnabled = true` 并把它当 `cloudProfileId`。所以：
- 无云运行靠 **全新数据目录**（没有 `config/llm-profiles.json` → 只剩 placeholder）；
- 有云运行靠 **写一个真实 profile**（`tauri-cdp-controlled-service.mjs` 就是这么做的），
  不需要去点任何开关。

### F-R13-5 撤销按钮已改走正式后端命令（任务书第 1 项，代码完成）

`recognitionClient.ts` 早先已支持 `undo[]` / `undone`；本轮完成接线：
- `RecognitionPanel.tsx`：撤销按钮改为 `applyRecognitionDecisions({decisions:[{decisionId, action:"undo"}]})`，
  与接受/拒绝共用同一 `submit` 路径；删除 `onUndoAutoFix` 这条**编辑器补丁**老路
  （它只改权威稿、改不动决策状态，是上一轮的假完成来源）；
- `recognitionDecisions.ts`：新增 `undoState()`（判据优先级：后端 `status==="undone"` →
  权威稿值已等于撤销目标 → 有无可回滚补丁）与 `decisionStatusLabel()`
  （修掉「已撤销」被嵌套三元显示成「处理失败」的缺陷）；`isDecided()` 纳入 `undone`；
- `ExamWorkspacePage.tsx`：随之移除 `onUndoAutoFix` 与仅供它使用的两个 import。

单测：新增 19 条（`undoState` 六条、`isDecided`/`decisionStatusLabel` 六条、
wire 层 `action:"undo"` 与 `undone` 归一七条）。全量 217 passed（原 198）。
注意其中一条**既有**断言必须同步修改：它断言 `args.input` 恰好等于
`{requestId,batchId,baseEditVersion,accept,reject}`，而 wire 现在必须多带 `undo: []`。

### F-R13-6 无云四链验收：任务书第 3 项**未达成**（确定性，非竞态）

用新可归因二进制 `ad934efa…` 跑 `node scripts/e2e/tauri-cdp-local-chain.mjs`：

```
library-page-loads-cloud-off  passed
import-pdf                    passed
chains-pdf                    FAILED  batchId=null，chains 四条全 not_run
import-docx                   passed
chains-docx                   FAILED  batchId=null，chains 四条全 not_run
verdict = failed
```

PDF 与 DOCX **症状完全一致**，与 F-R12-2 的根因吻合（`reconcile_status=failed`、
`last_error_code=canonical_not_seeded:<jobId>`、`recognition_batches_v1` 0 行）。
因此任务书第 3 项的两个要求同时不成立：

1. **「有 batch」**——不成立：批次根本没建。
2. **「本地核验状态准确」**——不成立：`processing_jobs_v2.local_status = succeeded`，
   而 `get_recognition_decision` 报 `local: not_run`。链状态取自批次行，
   无批次时视图只能用四条 `not_run` 兜底，于是**把一次成功的本地识别报成「从未运行」**。
   这一条与第 1 条是**两个不同的问题**，顺序修好后仍需复核（详见 R12-2 交接单第 4 节）。
3. 「云端明确未运行」——这一条**形式上成立但无意义**：云端确实 `not_run`，
   但四条链全 `not_run`，无法据此判断云端是被正确禁用还是整条链没跑。

构建新鲜度护栏这次走的是**清单模式**（`mode: "manifest"`）而非 mtime：
`frontendInputs 6f104c05… / dist 69ca780e… / backendInputs c02a5a59…` 三段内容哈希全部一致，
`ok: true`。这正是 F-R12-4 想要的效果——判定依据是内容，不是时间戳。

### F-R13-7 后端正在改的是**裁决调用预算**，与本轮两个缺陷无关（避免误判）

护栏第二次拒绝验收，理由是**后端输入**变了
（`c02a5a59… → c12ebf0c…`，`src-tauri/src/processing/scheduler.rs`）。
核查后确认后端在途改的是 **A4 裁决调用预算 + 受约束修复**
（`run_recognition_cycle_core_with_adjudicator`、`MAX_ADJUDICATION_MODEL_CALLS`、
`MAX_CONSTRAINED_REPAIRS`、把校验器原话回给模型再问一次），
**没有**动冻结顺序（462 行仍早于 495 行），也**没有**动撤销门与 `build_view`
（已逐行确认 `resolution == AutoFixed && status == Accepted` 与 `is_actionable()` 均未变）。

因此 F-R12-2（无批次）与 F-R13-2（手工接受撤销闭环）的结论**不受该在途改动影响**。
本轮受控服务验收以 `--tolerate-concurrent-edits` 针对 `ad934efa` 执行，
报告中已标明该 exe 是**后端开始编辑之前**的可归因构建，结果描述的是该二进制。

---

# R14（2026-09-17）工作区收敛、前端契约适配与 A3/A4 联调

承接后端两个提交（A3 = `937dda5`，A4 = `d353e9d`）。用户流程不变：
导入 → 看到题稿 → 处理明确问题 → 预览 → 导出。本轮先同步契约，再验证交互。
归属：前端（`src/**`）、E2E（`scripts/**`）、受控服务（`scripts/controlled-llm-service.mjs`）。
**后端独占区本轮只读，未改一行。**

## 一、契约核实（任务书第 1 条）：报告里的 Rust 字段名 ≠ 前端 JSON 字段名

结论来自**读序列化路径**（`schema/recognition_v1.rs`、`reconcile/engine.rs`、`reconcile/source.rs`），
不是读报告正文：

| 项 | 线上事实 |
| --- | --- |
| 四路链 | `chains.{local,cloud,source,adjudication}`，每路 = `StageStatusV1` |
| `StageStatusV1` | `{state, reasonCode?, message?, updatedAt?}`，**camelCase**；`reasonCode` 为 `None` 时**整个键不出现** |
| `StageStateV1` 线上取值 | `queued / running / succeeded / partial / unusable / not_run / failed / canceled`（`snake_case`）。**没有** `not_started`、`unavailable`、`skipped`、`done` |
| `DecisionItemV1.reasonCode` | **必填 `String`**（无 `skip_serializing_if`），永远出现 —— 前端不能当可选 |
| 视图字段 | `schemaVersion / itemId / jobId / batchId / baseEditVersion / editVersion / stale / generatedAt / chains / summary / actionable / autoApplied` |

**必须区分的一件事**：前端内部词表把 `not_run` 归一成 `not_started`、`unusable` 归一成
`unavailable`（`CHAIN_STATE_TO_STATUS`）。那是**前端内部**叫法，不是后端线上值。
写断言与交接单时必须分开，否则就会重复上一轮「按报告字段名猜线上形状」的错误。

**A3 链状态的真实算法**（`reconcile/source.rs:656-676`）：

```
total == 0                    -> NotRun
!has_text && confirmed == 0   -> NotRun
confirmed == total            -> Succeeded
其余                          -> Partial
```

`unusable_reason()` **只**覆盖 `model_status == Unusable`；`BudgetExhausted` 只写
`model_reason_code`，**不改链状态、不改链 `reason_code`**。
因此「A3 新增了 `SOURCE_VERIFY_BUDGET_EXHAUSTED`」对**逐项 code**成立，对**链状态**不成立。

## 二、前端契约适配：空列表不等于核验成功（任务书第 2 条）

删掉旧的一元 `describeCloudStatus(cloudStatus)`（它只要云端跑完就说「云端核验完成」），
换成 `describeVerificationStatus({localStatus, cloudStatus, sourceStatus, adjudicationStatus, pendingCount})`：

- `partial` 存在 → 「部分内容尚未完成校验，请检查标出的题目」；
- 三路里只要有一路 ∈ `{partial, failed, unavailable, not_started, skipped}` → 一律**不能**说
  「没有发现需要处理的问题」，没有待处理项时说「部分内容尚未完成校验」；
- 只有三路**都**完成且 `pendingCount == 0` 才说「云端校验完成，没有发现需要处理的问题」。

**一处刻意的保守选择**：A4 在「本地与云端没有分歧」时本来就该 `not_run`，那是**正常**的；
但当前契约里没有「有无分歧」这一位，所以前端只能保守地说「部分完成」，不能升级成「没有发现问题」。
这条由单测钉住（`recognitionClient.test.ts`，9 条用例，含「空列表不等于核验成功」）。
新 reason code 只用于内部分类，不进任何用户文案。

## 三、问题列表收敛成用户任务（任务书第 3/4/5 条）

新增 `src/features/editor/userTasks.ts`（+17 条单测）。合并规则：
连续缺答并成题号区间；同一题组的内部问题并成一条「这组题没有识别完整」；
同目标同修复动作去重；**每条合并后的任务保留 `covers`（底层根因集合）**，供验收断言「没有隐藏」。
未知码降级成 `processing-failed` 而**不丢弃**；`BLOCKER_LIST_TRUNCATED` 这类元信息不进界面。

第 4 条（泛化问题只在阻塞原因被**完整**表达时才隐藏）落成：

```ts
const explained = new Set<string>();
for (const task of tasks) for (const code of rootCausesOf(issues, task)) explained.add(code);
const unexplained = [...genericIssues, ...structureIssues]
  .filter((issue) => !GENERIC_ONLY.has(issueRootCause(issue)) && !explained.has(issueRootCause(issue)));
```

即「存在任意一个具体问题」**不足以**隐藏其他尚未解释的发布失败。

补齐质量码表（对齐 `src-tauri/src/ielts_grammar/issue_codes.rs`）：`INCOMPLETE_RECOGNITION`
从 22 个扩到 46 个，`MISSING_ANSWER`/`ANSWER_MISMATCH`/`STRUCTURE_INCOMPLETE` 各自补全。
此前漏码会让真实阻断落进 `processing-failed`，是**界面说假话**的来源。

### F-R14-1（前端，已修，P0）：门禁还没回来就说「可以导出」

**最小复现**：打开工作区后立即读 `[data-testid="workspace-issue-list"]` 的 `data-can-export`。
**预期**：门禁未返回时不得声称可导出。**实际**：`rendered cards=0 mergedRows=0 canExport=true`，
而同一时刻 `get_publish_preflight` 报 **34 条阻断**。

根因：面板挂载时 `preflight` 仍是 `undefined` → `issues` 为空 → 任务为空 → 空态直接渲染「可以导出」。
即**用「还没查」冒充「查过了没问题」**。

修复：`canExport` 增加「门禁确实读到了」这一条，并暴露 `data-preflight-state`（`loading|loaded|error`）
供验收等待：

```ts
const canExport = taskSummary.tasks.length === 0 && Boolean(preflight) && !preflightError && editor.pendingCount === 0;
```

E2E 断言（`tauri-cdp-issue-list.mjs`）：`PASS 门禁还有阻断时界面不说「可以导出」
:: {"canExport":"false","rawBlockers":34,"preflightState":"loaded"}`。

### F-R14-2（前端，已修，P0）：内联填空题的「去填写」定位不到输入框

**最小复现**：`demanding-reading-passage-3.pdf` 导入后点「第 11–13 题缺少答案」的「去填写」。
**预期**：滚动到 `q27` 的输入控件。**实际**：定位失败并提示「当前题面上没有对应的元素」。

根因：`answerSlots["q27"].hostNodeId` 指向的是 **stimulus 节点**（`group-1-stimulus-b032`），
而真正渲染输入框的节点是 `taskGroups[0].stimulus[1].children[3]`（`id = "slot-node-q27"`，
`type = answer_slot`）。`hostNodeId` 只走两跳，永远落不到控件上。

修复：新增第三跳 `contentNodeIdsForSlot(draft, slotId)`（只遍历 `taskGroups`，找承载该槽位的内容节点 id），
候选顺序变成 `[targetId, hostNodeId, ...contentNodeIds]`。
E2E 断言：`PASS 至少有一条「去填写」真的定位到了题面上的答案控件
:: {"target":"q27","scrolled":["group-1-stimulus-b032"]}`。

附带修掉一处**假信息**：`locate-miss` 原先把具体题目位置也说成「整份文档级别」，
现按目标是否为 `document` 分岔措辞。

### F-R14-3（验收脚本，已修）：`Page.reload` 会打断 CDP 会话

`tauri-cdp-issue-list.mjs` 原版在导入前 `Page.reload`，理由是「写 `cloudEnabled:false`」。
但 `AppSettingsV1`（`src/features/settings/appSettings.ts`）**根本没有 `cloudEnabled` 这个字段**——
重载只带来风险，不带来任何前置条件。实测重载后 `CDP 连接已关闭`，等待条件超时。
已删除该重载（与 F-R13-4 是同一件事的另一半）。

## 四、A3/A4 受控联调（任务书第 6 条）：**未达成，根因在后端**

先回答任务书要求先核实的那一问：**原受控服务样本不覆盖 A3/A4**。
`fixtures/controlled-llm/reading-outline.json` 只产 outline 候选，A3/A4 请求拿到它会被网关
整份拒绝。本轮已给 `scripts/controlled-llm-service.mjs` 补上按 prompt 标记分派
（`--- SLOTS BEGIN ---` → `verify_source_answers`；`--- DIVERGENCES BEGIN ---` → `adjudicate_divergence`）
与 5 种模式（`normal|decline|partial|fail|garbage`），并用 curl 冒烟确认分派正确。

**但真实联调在更前面就被挡住了：本次导入没有产出任何识别批次。**

`artifacts/e2e-cdp/run-controlled-service-2026-09-17T16-12-34-283Z`：

```
controlled-service-drives-candidates        failed        没有产出批次
expected-sample-reproducible                not-executable 样本目标 q14 在本仓夹具里不存在
accept-manual-candidate                     not-executable 没有「待确认且带可应用补丁」的候选
undo-manual-accept                          not-executable 同上
a3-a4-requests-reach-service                not-executable（分类见下，**该分类是错的**）
verification-status-matches-chains          passed
late-model-result-does-not-overwrite-...    failed        重跑后没有产出批次
a3-partial-not-reported-as-complete         failed        chains.source 实际 not_started
a3-model-failure-not-reported-as-complete   failed        chains.source 实际 not_started
```

四路链全部 `not_started`（线上值 `not_run`），`batchId=null`。
默认导入没批次，脚本按既有绕行「播种 + 重试」重试后**仍然**没批次，
换派生样本重跑一次**仍然**没批次 —— 即 F-R12-2 的绕行在 `937dda5` 上已不再有效。

唯一通过的是 `verification-status-matches-chains`：它用**独立实现**的规则算出「界面该说什么」，
再与真实 DOM 逐字比对，并断言三路原因码没有泄漏进用户文案。这一条证明的是
**前端契约适配本身正确**（含「有链路没跑完就不许说没有发现问题」），不证明 A3/A4 联通。

### F-R14-4（后端，未修，已交接，P0）：A3 的网关调用只留下输入缓存，既没有结果也无法诊断

这是本轮最值得交接的一条，因为它同时解释了「A3 为什么没结果」和「为什么查不出原因」。

**证据（同一作业目录，三处互斥的事实同时成立）**：

1. `cache/llm/verify_source_answers-input-1789661600016.json` **存在**（17:13:20 落盘），
   说明 `run_llm_gateway("verify_source_answers", ...)` 已被进入，且请求是**真实**的：
   `mode=verify_source_answers`、`slots` **14 项**（`q27`…，每项带 `localValue` 与 `questionNumber`）、
   `pdfPath` 指向真实上传件、`profile.baseUrl = http://127.0.0.1:11435/v1`（受控服务）。
2. 受控服务**全程零 POST**（它每收到一个 `/chat/completions` 都会打一行日志；
   本次只收到 1 个 outline 请求）。
3. 作业目录里**没有** `llm-calls.jsonl`，也**没有** `verify_source_answers-output-*.json`。

**为什么第 3 点是硬矛盾**：`llm_gateway.rs:30-79` 的写法是「写输入缓存 → 执行 → **无论成败**
追加一行 `llm-calls.jsonl` → 成功才写输出」。既然输入缓存写了、调用记录却没有，
说明执行分支在**追加记录之前**就没有返回——`run_openai_compatible_source_verification_llm`
要么 panic 掉了线程，要么永久阻塞。**两种机制都未被证实**，故本条只报事实、不报根因。

**影响**：A3 在真实产品路径上**从未产生任何结果**，而且失败得**不可诊断**
（唯一能说明原因的调用记录缺失）。任务书第 6 条里除「状态一致性」外的场景因此全部不可达。

**归属**：`llm_gateway.rs` / `auto_pipeline.rs` / `processing/scheduler.rs` 全在后端独占区，
本轮**未改**。已出交接单 `Plan With Files/Dual_Recognition/HANDOFF_2026-09-17_r14_a3_gateway_no_trace.md`。

### F-R14-5（验收脚本，已修）：把「发起了但没到」误判成「前提不成立」

`a3-a4-requests-reach-service` 原先只看「受控服务收到几个 A3/A4 请求」，收到 0 个就记
`not-executable`，理由写成「本夹具里没有可核验的槽位」。
但本次请求里**明明带了 14 个槽位**，输入缓存也在盘上 —— 那是**假解释**，
等于用「前提不成立」掩盖「东西坏了」。

已改为读作业目录的网关痕迹（输入/输出缓存 + `llm-calls.jsonl`）分三态判定：
应用**没发起** → `not-executable`；应用**发起了但服务零 POST** → `failed`（并附三处证据）；
服务收到 → 断言形状。同时把痕迹写进 `report.service.gatewayTraces` 便于交接。

**修正后重跑一次（`run-controlled-service-2026-09-17T16-52-23-738Z`）**，同一证据再次出现，
分类已正确：

```
[scenario] FAILED a3-a4-requests-reach-service :: A3（原文件核验）在应用侧已经发起（输入缓存 1 份），
  却从未到达受控服务：服务收到的请求={"total":1,"outline":1,"a3":0,"a4":0}；
  作业目录 llm-calls.jsonl 存在=false，网关痕迹={"verify_source_answers":{"input":1,"output":0}}。
  输入缓存有、调用记录与输出都没有、服务端零 POST，说明这次调用既没有结果也无法诊断。

gatewayTraces = { "files": ["verify_source_answers-input-1789663989376.json"],
                  "byCommand": { "verify_source_answers": { "input": 1, "output": 0 } },
                  "callRecordExists": false }
```

即 F-R14-4 是**可复现**的（两次独立运行、两个不同作业 id），不是一次性抖动。

## 五、A3/A4 三个非成功态为什么测不到（任务书第 6 条其余部分）

`mode=partial` / `mode=fail` 两个场景都因**没有批次**而失败：链状态恒为 `not_started`，
断言「`chains.source` 应当是 `partial`」自然不成立。
受控服务侧的分派与模式切换本身已用 `/health` 与 curl 验证可用，
但**没有批次就没有对象**，这部分只能记**未验证**，不得记通过。


## 六、真实导入—编辑—保存—预览—导出（任务书第 7 条）

三轮真实全链路，用真实 Tauri 工作区（CDP 通道，`--diagnostic-args`，报告里 `runProfile=cdp-diagnostic`），
每轮 10 张截图（`01-library` … `10-recognition-panel`）：

| 夹具 | 入口 | 导入 | 编辑/保存 | 重开存活 | 学生预览 | 作答隔离 | 导出 | 报告目录 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `demanding-reading-passage-3.pdf` | `pick-folder` | ✅ | ✅ | ✅ | ✅ | ✅ | **blocked** | `run-chain-2026-09-17T16-40-53-718Z` |
| `demanding-reading-passage-1.docx` | `pick-files` | ✅ | ✅ | ✅ | ✅ | ✅ | **blocked** | `run-chain-2026-09-17T16-42-31-472Z` |
| `complex-reading.pdf`（带答案段） | `pick-folder` | ✅ | ✅ | ✅ | ✅ | ✅ | **blocked** | `run-chain-2026-09-17T16-43-09-693Z` |

三轮都是 11 步 passed、1 步 **blocked**（`publish-via-workspace-button`），`verdict=blocked`、`exit=4`。
**blocked 不是 passed**：判定明确写着「发布未发生，不得计为通过」。

**导出为什么没发生（这是正确行为，不是缺陷）**：门禁给出的具体阻断——

- `passage-3.pdf`：34 条 = `ISSUE_UNRESOLVED`×16 + `ANSWER_MISSING`×14（q27–q40）+ `QUALITY_HARD_FAILURE`×3
  （`WORD_LIMIT_UNPARSED`、`ANSWER_KEY_MISSING_SLOT`、`RUNTIME_COMPILER_FAILED`）+ `QUALITY_NOT_READY`×1；
  编译器探针 `RUNTIME_ANSWER_UNRESOLVED:q27…`：**这份 PDF 没有答案**，题面画得出来但发不出去。
- `passage-1.docx`：34 条，同类（多出 `PROMPT_EMPTY`、`PROMPT_BOUNDARY_AMBIGUOUS`）。
- `complex-reading.pdf`（唯一带 `## Answers` 段的夹具）：只剩 8 条 = `ISSUE_UNRESOLVED`×4 +
  `QUALITY_HARD_FAILURE`×3（`SLOT_HOST_MISSING`、`SIGNIFICANT_REGION_UNASSIGNED`、`RUNTIME_COMPILER_FAILED`）
  + `QUALITY_NOT_READY`×1。**答案已被解析**（没有 `ANSWER_MISSING`），剩下的是结构类阻断。

**结论**：本仓现有的真实 PDF/DOCX 语料**都到不了可导出**，卡在内容/结构缺陷上，不是链路坏了。
所以「导出成功路径 + 学生端可读性」必须另找一条**真实发布**来验证，见下。

### 导出成功路径（真实 UI → 真实 publish_items → 真实 NAS 包）

`tauri-publish-ready.mjs`（selenium）本轮连续两次死在驱动握手
（`SessionNotCreatedError: session not created / chrome not reachable`、
`NoSuchSessionError: session deleted as the browser has closed the connection`），
连工作区都进不去 —— 与 F-R11 系列同型，属本机 WebView2 + tauri-driver 的稳定性问题。

因此新增 `scripts/e2e/tauri-cdp-publish-ready.mjs`（CDP 通道），把这条成功路径搬到与其余验收同一条通道：

```
[publish-ready-cdp] verdict=passed exit=0
steps: library-page-loads:passed | ready-item-visible-in-library:passed |
       workspace-opens-for-ready-item:passed | publish-via-workspace-button:passed
报告：artifacts/e2e-cdp/run-publish-ready-2026-09-17T16-51-08-159Z
```

产物（真实落盘）：`manifest.js`、`releases/<batchId>/early-approaches.js`、
`releases/<batchId>/snapshots/…/{authoring-ir-v2,manifest-v2,reading-source-v2}.json`、
`resources/early-approaches/asset-manifest.json`。
**数据来源如实声明**：用 proven-ready 夹具播种（`fixtures/golden/synthetic/ielts/early-approaches-authoring-v2.json`），
**不是**真实 PDF/DOCX 自动识别的产物；本套件只证明导出链，不证明自动识别能到 ready。

### 学生端可读性：两道，都过

1. **本仓契约脚本（已按真实规则修正，见 F-R14-6）**：
   `node scripts/e2e/nas-student-contract.mjs --package <上面那个 nas-library>` → **22/22 PASS**，
   逐项包含 `script-exists`、`script-sha256`、`script-register-parses`、`script-key-matches`、
   `runtime-sha256`、`asset-manifest-sha256`、每个 asset 的 `file-sha256` / `file-byteLength`。
   报告：`artifacts/e2e-cdp/nas-contract-publish-ready.json`。
2. **学生端仓库自带的真实 loader 验收**（独立佐证）：
   `cd F:/workspace/IELTS-NASfor-WenDao && IELTS_PDF2TEST_REPO=F:/workspace/PDF2Test npm run verify:cross-repo-reading-v2`
   → `{"status":"PASS","authorRepo":"F://workspace//PDF2Test","examId":"early-approaches","slots":["q14","q15"],"sharedOptionCount":5}`

**仍未验证（不得含糊）**：Electron 学生端**真实加载与作答一致性**属 M6 的 NAS 实测（需桌面运行），
本轮**未做**，两份脚本都只到「读取规则 + 完整性绑定」这一层。

### 扫描 PDF：**实测到不了可作答**（不是「没跑」，是跑了不行）

样本：`fixtures/parser/image-only-reading.pdf`（29KB，含 `/Image` 而**无 `/Font`**，确属图像型）。
`run-chain-2026-09-17T17-20-37-292Z`：

```
library-page-loads / import-pdf-via-folder-hook / background-pipeline-reaches-stable-stage
  / workspace-opens / edit-body-and-save / edit-survives-reopen      passed
student-preview-renders   FAILED  预览里没有任何可作答控件（slot=0），这份题稿学生无法作答
student-preview-answering-isolated  FAILED  同上
edit-after-preview-survives-reopen  FAILED  预览返回后无法再次进入原位编辑
publish-via-workspace-button        blocked 5 条：QUESTION_RANGE_UNPARSED、V1_COMPATIBILITY_COMPILER_FAILED、
                                             ISSUE_UNRESOLVED×2、QUALITY_NOT_READY×1
verdict=failed exit=1
```

用户看到的提示是「预览与已保存的内容一致。还有 3 处问题没有处理完，修好之前这道题发不出去。」

**结论**：在没有 OCR 的前提下，扫描 PDF 的题稿里**没有任何可作答槽位**，学生端根本无从作答，
导出被门禁拦下。这不是本轮能解决的（需要 OCR 服务能力），因此**扫描 PDF 列为未验证**：
OCR 通道（`transcribe_pdf_images`）既没有受控响应，也没有真实视觉模型凭据。
**不写「通过」，也不写「没测」** —— 测了，结果是「当前不可用」。

### F-R14-6（验收脚本，已修，P1）：学生端契约与产物判定用的是**旧布局**，会把成功的发布判成失败

这是本轮第二个「工具在说假话」的缺陷，方向与 F-R14-1 相反（这次是假**红**）。

**真实规则**（学生端 `NasJsDirectReadingAssetProvider.ts`）：运行时脚本从
`manifest.entry.script` 解析（`:114` `asString(value.script, \`${assetId}.js\`)`，`:236`
`safeJoinNasRoot(root, entry.script)`），当前 publisher 把它放在
`./releases/<batchId>/<examId>.js`。真实学生端还会做两道完整性绑定：
`checksums.scriptSha256`（整份脚本文本，`:381`）与 `checksums.runtimeSha256`
（`canonicalJson(payload)`，`:393`），V2 条目还强制要求 `resourcesBase` + `assetManifest`。

**旧脚本的实际行为**：`nas-student-contract.mjs` 检查 `path.join(packageDir, \`${examId}.js\`)`；
`chain-verdict.evaluatePublication` 用 `^v2-p.*\.js$` 通配包根。上面那条**成功**的发布里，
包根只有 `manifest.js`（运行时在 `releases/<batchId>/`），于是两者都判「没有题目 JS」。
即：**这套契约根本回答不了「学生端能不能读我们的导出」** —— 它对正确的包也报失败。

**修法**：两个脚本都改为按 `entry.script` 解析并断言文件存在；契约脚本补齐
`scriptSha256` / `runtimeSha256` / `script-key-matches` / V2 `resourcesBase` 必填，
并把包根旧布局降级为 `INFO`（不依赖、也不因它消失而报警）。
单测同步：`chain-verdict.test.mjs` 把那条「预检不一致即失败」改成两条 ——
产物缺失仍失败；`entry.script` 布局不再误判。

### F-R14-7（后端，未修，待判定，P1）：`get_publish_preflight` 与实际可发布性口径不一致

**最小复现**：用 proven-ready 夹具播种 → 打开工作区 → 调 `get_publish_preflight`（发布前）→
点「发布」→ 再调一次（发布后）。

**实测**：

```
preflightBefore = {"passed": false, "editVersion": 1,
                   "blockers": [{"code":"QUALITY_NOT_READY","internal":"quality_state=review_required"}]}
preflightAfter  = {"passed": false, "editVersion": 1, 同上}
发布结果        = "发布完成：early-approaches"，产物齐全，学生端契约 22/22
```

即：**预检说不能发，发布却成功且产物完整**。两侧至少有一侧口径不对，需后端判定：
`get_publish_preflight_core`（`authoring_v2_commands.rs:417-424`）在 `quality.state != "ready"` 时
一律报 `QUALITY_NOT_READY`，而 `export_authoring_v2_core`（`:255-262`）要求
`quality.state == Ready`，否则 `authoring_v2_export_blocked:quality_state=review_required`。
两侧读的却是同一份持久化阴影（作业目录里发布后仍是 `state=review_required`、`hardFailures=0`）。

**用户可见后果**：工作区会显示一条「这道题还有未确认的内容」的阻断任务，而「发布」按钮其实能成功。
本轮**未改**任何后端文件，只把观察记进报告；`evaluatePublication` 已不再拿它当产物缺失
（否则会把「发布成功」写成「没发布」），改由 `describePreflightDisagreement()` 单独上报。

**归属**：`authoring_v2_commands.rs` 在后端独占区。相关文档见
`Plan With Files/Dual_Recognition/HANDOFF_2026-09-17_r14_a3_gateway_no_trace.md` 第 4 节。


## 七、本轮环境备注（不是产品结论，但会污染下次验收）

1. **学生端仓库的 `server/dist` 被我改名挪开过**：`npm run verify:cross-repo-reading-v2` 第一步是
   `npm run build:server`，它内部 `rmSync('server/dist', {recursive:true, force:true})`，
   本机安全删除护栏按「单回合批量删除」配额拦下（`SAFE_DELETE_BULK_CONFIRM_REQUIRED count=54`）——
   与 F-R13-1 同型，只是这次发生在**另一个仓库**里（那边的构建脚本我不能改）。
   更糟的是护栏抛错前已经删掉一部分（54 → 41），`dist` 停在半删状态。
   处置：把 `server/dist` 整体 `mv` 成 `server/dist-prev-r14`（重命名不是删除），
   让测试在全新目录上重建 —— 重建后的 `server/dist` 正常，跨仓验收通过。
   **`server/dist-prev-r14` 是半删的残留，我没有删除它**（护栏存在的意义就是别让我随手删），
   需要清理请自行确认后处理。
2. **selenium/tauri-driver 路径在本机不可用**：`tauri-publish-ready.mjs` 两次分别以
   `SessionNotCreatedError: chrome not reachable` 与 `NoSuchSessionError: session deleted` 失败。
   同一功能在 CDP 通道（`tauri-cdp-publish-ready.mjs`）一次通过。
   下次要跑成功发布路径，请直接走 CDP 版本。
3. **`--diagnostic-args`**：本轮所有真实工作区验收都带该参数，报告里 `runProfile=cdp-diagnostic`。
   默认参数下 `tauri-cdp-issue-list.mjs` 多次无法启动（WebView2 渲染器不稳），
   所以这些结果**不代表默认路径已通过**。


## 八、用户视角总结（任务书要求的四问）

### 1. 用户看到了什么

导入 `demanding-reading-passage-3.pdf` 后，工作区顶部是 `已保存` 和 `问题 3 · 阻断 3`；
问题列表里是**三条可执行的任务**（不是 34 条内部问题行）：

| 任务 | 详情 | 按钮 |
| --- | --- | --- |
| ⚠ 第 27–40 题缺少答案 | — | 去填写（滚到 q27 的输入框） |
| ⚠ 第 27–31 题没有识别完整 | 请对照原文件检查题干和答案。 | 查看原文（开原文件抽屉）、重新识别（真的重新入队） |
| ⚠ 这道题还有内容没有处理完，暂时不能导出。 | 重新识别一次；仍然不行请把原文件重新导入。 | 重新识别 |

34 条门禁阻断 → 3 条任务；没有任何问题码、版本号、批次号、slot、schema 出现在界面上。
底部一句：「这道题存在必须修复的内容缺陷，请按问题列表逐项处理。」

### 2. 每类问题怎么解决

- **缺答案** → 「去填写」把用户送到那道题的输入框（本轮修掉定位不到控件的缺陷，F-R14-2）。
- **识别不完整** → 「查看原文」打开原文件并定位到题组；「重新识别」真的重新入队，
  完成后问题按新结果重算。
- **结构性未完成** → 只能重试；重试无效时如实说明要重新导入原文件。
- **没有任何问题** → 不生成问题卡片，只保留一句完成状态；且**门禁确实读到了**才说「可以导出」
  （F-R14-1：读不到时说「可以导出」是把「还没查」冒充「查过了没问题」）。

### 3. 哪些问题仍阻止导出

本轮**真实 PDF/DOCX 语料全部无法导出**，而且这是**正确行为**：三份夹具都缺答案或缺结构，
门禁逐条给出原因（`ANSWER_MISSING`、`ANSWER_KEY_MISSING_SLOT`、`WORD_LIMIT_UNPARSED`、
`PROMPT_EMPTY`、`SLOT_HOST_MISSING`、`SIGNIFICANT_REGION_UNASSIGNED`、`RUNTIME_COMPILER_FAILED` …）。
扫描 PDF 更前一步：题稿里 0 个可作答槽位。

**后端侧仍有一处阻止 A3/A4 联调**：本次导入完全不产出识别批次（`batchId=null`，四路链全 `not_run`），
且 F-R12-2 的「播种 + 重试」绕行已失效；A3 的网关调用还留下一个「有输入、无记录、无输出、零 POST」
的半截痕迹（F-R14-4，已交接）。

### 4. 三类验证分别到什么程度

| 通道 | 结论 |
| --- | --- |
| **受控模型服务** | 服务侧已按请求分派 outline / A3 / A4 并支持 5 种模式（curl 冒烟通过）。**但端到端只通过 1/9 个场景**（`verification-status-matches-chains`，证明前端契约适配正确）；其余因**没有批次**或 **A3 不落地**而 failed / not-executable。**不得把「受控服务跑通」当作 A3/A4 已联通。** |
| **真实云服务** | **未验证**：本轮没有真实供应商凭据，所有云端请求都指向 `127.0.0.1:11435` 的受控服务。 |
| **学生端** | 本仓契约脚本 **22/22 PASS**（含 `scriptSha256` / `runtimeSha256` 两道完整性绑定）+ 学生端仓库自带真实 loader 验收 **PASS**。**Electron 真实加载与作答一致性未验证**（M6 范围，需桌面运行）。 |


# R15：把「改了但没跑过」的 5 个 E2E 脚本真实跑一遍

## 一、本轮要解决的问题

上一轮（`88f966c`）有 **5 个 E2E 脚本被一起提交，却从未运行过**。按仓库护栏
（「无证据必须标未完成」），它们不能算验证过。本轮逐个真实运行，如实记录结果，
并修掉脚本自身会让结论失真（假绿/假红）的地方。

**归属**：改动全部落在前端 `src/**` 与 `scripts/**`；后端独占区（`src-tauri/src/processing/**`、
`reconcile/**`、`recognition/**`、`llm_gateway.rs` 等）**改动 0 个文件**。

## 二、5 个脚本的真实运行结果

| # | 脚本 | 首次运行 | 处理 | 复跑 |
| --- | --- | --- | --- | --- |
| 1 | `tauri-cdp-smoke.mjs` | **CANNOT-RUN**（无参运行） | 修默认参数（F-R15-1） | **passed 5/5**（无参） |
| 2 | `tauri-cdp-recognition-buttons.mjs` | not-executable ×5（exit 5） | 删残留 `Page.reload`（F-R15-2） | not-executable ×5（**结果不变**） |
| 3 | `tauri-cdp-publish-unblock-probe.mjs` | `attribution=data-fix-insufficient` | 修 `saved` 判定假阴性（F-R15-3） | 同一归因；`notSavedSlots` 由 `["q1","q2"]` → `[]` |
| 4 | `tauri-cdp-recognition-write-path.mjs` | **passed 9/9** | 无需改动 | — |
| 5 | `tauri-direct-canonical.mjs` | **CANNOT-RUN**（selenium 会话握手失败） | 无法在本机验证（F-R15-4） | — |

### 1. `tauri-cdp-smoke.mjs` —— 无参必挂，是脚本自己没照注释做

首次（无参）运行：

```
[smoke] CANNOT-RUN 页面在 90000ms 内未渲染出可见文本（求值持续失败（最后一次：CDP 连接已关闭，无法发送 Runtime.evaluate））
```

**这不是产品问题**。脚本第 33-35 行的注释写着「环境必需的诊断参数……不加这两个开关时会中途
崩溃（`CDP 连接已关闭`）」，可默认值偏偏是空串（`const extraArgs = ... : ""`）。同类脚本
（`recognition-write-path`）是**硬编码**这两个开关并标 `diagnosticRun=true`，所以它一跑就过。

带参验证：`--extra-args "--no-sandbox --disable-gpu"` → **passed 5/5**。
改默认值后无参复跑 → **passed 5/5**，报告里 `diagnosticRun=true`（结论仍标注为诊断参数运行）。

### 2. `tauri-cdp-recognition-buttons.mjs` —— 5 个场景全部「前提不成立」

```
[recog-buttons] verdict=not-executable exit=5
reason=以下场景本次**无法执行**（前提不成立，例如没有真实候选项）
```

报告里的实测数据（不是推测）：

```json
"candidates": {
  "batchId": null,
  "chainStates": {"adjudication":{"state":"not_run"},"cloud":{"state":"not_run"},
                  "local":{"state":"not_run"},"source":{"state":"not_run"}},
  "actionableCount": 0, "needsReviewCount": 0, "autoFixedCount": 0
}
```

`batchId=null`、四路链全 `not_run` → 没有候选项，按钮流程无从执行。**这是如实的环境限制**
（与 R14 的 F-R12-2 同一件事），不是脚本回归，也**不能记作通过**。

顺手删掉了脚本里残留的 `session.cdp.send("Page.reload")`：它唯一的旧理由是「刚写进
localStorage 的设置要重载才生效」，而那行设置上一轮已经删掉（`cloudEnabled` 根本不是
`AppSettingsV1` 的字段），于是这次重载**没有任何作用**。删掉后复跑，结果与删除前**完全一致**。

> **关于「reload 会不会打断 CDP 会话」——本节不主张因果。**
> 上一轮的记录里写着 `tauri-cdp-issue-list.mjs` 是被 `Page.reload` 打断的，本轮实测**不支持**
> 把它当成必然因果：`tauri-cdp-publish-unblock-probe.mjs`、`product-chain`、`local-chain`
> 都还留着 `Page.reload`，却都能完整跑完（probe 本轮复跑三次均正常）。所以删掉它的理由是
> **「它已经没有作用」**，不是「它一定会把会话弄断」。至于它与会话中断到底有没有关系、
> 在什么条件下有关系，**本轮没有做受控实验，不结论**。

### 3. `tauri-cdp-publish-unblock-probe.mjs` —— 归因结论成立，但报告里有一处假阴性

该脚本不产 `passed/failed`，它输出**归因**结论。实测：

```
[probe] baseline quality.state = blocked
[probe] baseline hardFailures = ["SLOT_HOST_MISSING","SIGNIFICANT_REGION_UNASSIGNED","RUNTIME_COMPILER_FAILED"]
[probe] attribution = data-fix-insufficient
[probe] ignoredClearedGate = false   resolutionBlind = true
[probe] cleared = []   persisted = ["QUALITY_NOT_READY","QUALITY_HARD_FAILURE","ISSUE_UNRESOLVED"]
```

即：**用户把界面上能改的槽位都改了、并把所有 blocking 问题标成 ignored，门禁依然拦下，
且一条码都没被清掉**（`resolutionBlind=true`）。归因指向后端门禁的 resolution 语义，属后端职责，
本轮只记录、不改后端。

但报告里 `notSavedSlots: ["q1","q2"]` 是**假阴性**。查 `baseline.answerKey` 可见：

| 槽位 | baseline 已有值 | 探针点的值 | 版本变化 | 旧判定 |
| --- | --- | --- | --- | --- |
| q1 | `text/TRUE` | TRUE | 1→1 | `saved=false` ❌ |
| q2 | `text/FALSE` | FALSE | 1→1 | `saved=false` ❌ |
| q3 | `text/NOT GIVEN` | TRUE | 1→2 | `saved=true` ✅ |
| q4 | `text/maps` | diaries | 2→3 | `saved=true` ✅ |
| q5 | `text/diaries` | diaries | 3→3 | `saved=true` ✅ |

根因有两条，都不是产品缺陷：

1. **判定只认一种答案形状**。同一份题稿里，「文本型答案位」存 `values`
   （`{kind:"text", normalization:"ielts_default", values:[...]}`），「选项型答案位」才存 `labels`。
   q1/q2 渲染成单选框、底层却是文本形状，旧判定只读 `labels` → 一律判成「没保存」，
   读起来像「用户填了没落盘」。
2. **探针的目标值本来就等于现值**。探针取的是独立答案表里的**正确答案**，而题稿里 q1/q2
   本来就是正确答案，于是点击不产生变更、`editVersion` 不动 —— 那是「无需修改」，
   不是「没保存」。

修法：判定同时接受 `labels ?? values`，并在归因里补一句口径说明
（`notSavedSlotsNote`），把「值本来就对」与「没保存」明确分开。
复跑：`notSavedSlots=[]`，逐槽位 `saved` 全为 `true`，而**归因结论不变**（仍
`data-fix-insufficient`）——说明修的是报告准确性，结论本身是稳的。

### 4. `tauri-cdp-recognition-write-path.mjs` —— 9/9 通过

```
[step] PASSED library-page-loads / import-fixture / read-recognition-decision
[step] PASSED write-path-rejects-legacy-shape / -empty-decisions / -conflict
[step] PASSED write-path-shape-accepted / -invalid-identity / undo-channel-writes-canonical
[recog-write] verdict=passed
```

该脚本硬编码诊断参数并标 `report.diagnosticRun = true`，所以默认即可运行。注意它自己的定位：
**这是接线验证（请求形状被真实后端接受、结构校验可达），不是用户流程验收** —— 当前夹具
`get_recognition_decision` 仍返回 0 条可核对项，没有对象可决策。

### 5. `tauri-direct-canonical.mjs` —— 本机跑不到，改动**未验证**

```
[e2e:tauri] starting tauri-driver on :57943
SessionNotCreatedError: session not created
from chrome not reachable
```

这是本机 selenium 通道的老问题（R14 已为 `tauri-publish-ready.mjs` 记录过一次，本次是第三次
独立复现）。**直接后果**：该脚本里那处 `verifyIssueTarget` 的改动（从
`button[data-issue-target-id]` 改为按 `button[data-action-id="fill-answer"]` +
`data-action-target` 找）**根本没有被执行到**，因此**必须标为未验证**。

它依赖的 DOM 契约本身**已由另一条通道证实**：`tauri-cdp-issue-list.mjs` 在 CDP 通道上断言了
「每条任务至少一个真实动作按钮、动作种类合法」「每个『去填写』都有作用」「至少有一条真的定位到
题面元素」——用的是同一套 `data-action-id` / `data-action-target` 属性。

## 三、本轮新发现的缺陷

### F-R15-1（验收工具，已修）：smoke 默认参数与自身注释矛盾 → 无参必 CANNOT-RUN
见 §二.1。修后无参 `passed 5/5`。

### F-R15-2（验收工具，已修）：recognition-buttons 残留会打断 CDP 会话的 `Page.reload`
见 §二.2。删除后复跑结果不变。

### F-R15-3（验收工具，已修）：probe 的 `saved` 判定只认 `labels`，对文本形状答案位假阴性
见 §二.3。修后 `notSavedSlots` 由 `["q1","q2"]` 变为 `[]`，归因不变。

### F-R15-4（环境阻塞，未验证）：`tauri-direct-canonical.mjs` 的改动无法在本机执行
见 §二.5。**不记作通过，也不记作失败**——它是「未验证」。

### F-R15-5（产品缺陷，已修）：草稿未就绪时问题列表渲染出**不可定位**的任务

这是本轮唯一的产品缺陷，而且**间歇**、**用户可见**。

**证据链**（同一份构建、同一份夹具、同一脚本，四次运行）：

| 运行 | 任务 id | 点「去填写」的结果 |
| --- | --- | --- |
| 16:00 | `missing-answer:q27+q28+…+q40` | `scrolled=["group-1-stimulus-b032"]` → **PASS** |
| 17:39 | `missing-answer:unnumbered` | `scrolled=[]` + 「目标 q27 在题面上没有对应的元素」 → **FAIL** |
| 17:41 | `missing-answer:q27+…+q40` | 命中 → PASS |
| 17:42 | 快照时 `unnumbered` | 点击时该按钮**已不存在**（`clickSelector` 超时）→ FAIL |

最后一次运行给出了决定性线索：**快照里那个 `unnumbered` 按钮，在点击时已经不在 DOM 里了**
—— 说明任务列表在运行中被**重算过**（`unnumbered` → `q27+…`）。

**根因**：`preflight`（后端门禁）与草稿是两条**并行**的异步链，门禁完全可能先返回。
此时 `buildUserTasks` 拿不到 `answerSlots`，`slotIdsOfTarget` 一律返回空，
带题号的缺答问题就退化成 `missing-answer:unnumbered`；而题面此刻也还没渲染出那道题，
于是「去填写」定位必然落空。草稿就绪后任务重算、taskId 变化，用户若在窗口期内点击，
得到的就是「找不到」或者按钮被换掉。

**修法（产品侧）**：`ExamWorkspacePage` 在 `editor.loading` 为真时**不渲染任务**，
改显示「正在打开这道题…」，并暴露 `data-tasks-ready` 供验收等待。
（题面画布早就有这道保护——`{editor.loading ? <p>正在打开这道题…</p> : null}`——漏的是问题列表面板。）

**修法（验收侧）**：
- 快照前等 `data-tasks-ready="true"`，不再跟产品竞速；
- 点击前按「动作 + 目标」**重新解析** testid（任务重算后旧 testid 会消失）；
- 点击失败不再终止采集，而是记进报告；
- 新增诊断 `diagnosis`：`slotInDraft` / `slotCount` / `questionNumber` / `hostNodeId` /
  `domMatchCount` —— 下次再出问题，报告能直接回答「草稿里没有这个槽位」还是「草稿有但题面没渲染」。

**修复后验证**：重建 exe（`e38f9095…`，内容哈希清单归因）后**连跑三次，全部 `passed` 13/13**，
taskId 稳定为 `missing-answer:q27+…+q40`，定位命中 `group-1-stimulus-b032`。
诊断字段实测：`{"slotInDraft":true,"slotCount":14,"questionNumber":27,"hostNodeId":"group-1-stimulus-b032","domMatchCount":1}`。

## 四、`workspace-publish` 是否应以 `canExport` 作为 disabled 条件

**结论：不应当。** 现状（`disabled={Boolean(busyAction)}`）是正确设计，本轮不改。

1. `canExport` 的口径是「**能不能说『可以导出』这句话**」：任务列表为空 + 门禁确实读到了
   + 没有待保存修改。它是一个**文案判据**，不是发布动作的前置条件。
2. 发布动作本身由**后端门禁兜底**（点发布会被逐条拦下并给出原因），前端不重复实现门禁规则
   ——这是「产品现实优先」。
3. **若拿它去 disabled 发布按钮，会把可发布的题锁死**：R14 实测 F-R14-7 —— 预检恒
   `passed=false / quality_state=review_required`，而**发布成功、产物齐全**。用预检当门禁开关，
   就是让一个已知不可靠的读数和发布能力绑定。
4. 任务书第 5 条要求的是「『可以导出』**以当前题稿的后端发布检查为准**」——指的是那句状态文案
   的判据（已满足：`data-can-export` + `data-preflight-state` 三重条件），不是按钮可用性。

## 五、环境备注（本轮）

1. **诊断参数在本机是必需的**：`--no-sandbox --disable-gpu`。不加则 WebView2 渲染器中途崩
   （`CDP 连接已关闭`）。所有以此运行得到的结论都标注 `runProfile=cdp-diagnostic`，
   **不代表默认启动路径已通过**。
2. **selenium / tauri-driver 通道在本机不可用**：`SessionNotCreatedError: chrome not reachable`，
   本轮第三次独立复现。
3. **构建归因**：本轮重建两次（`build-app.mjs`），报告里都是 `buildFresh.mode=manifest`
   （三段链内容哈希一致），不是靠 mtime 判断的：
   - `e38f9095b5c7a8118fcabc0f460cd379b6415b39a4f0ea222491b2b3fc65e946`（第一版修复）
   - `6eaefd155be9003c2996830cff1e9204204ce5674d729326a26c9bdb8931c47a`（领域规则下沉后，含 §六 的复跑）
4. `npx tsc --noEmit` 干净；`npx vitest run` **241 passed / 14 files**。

## 六、修复后的复跑确认（新构建 `6eaefd15…`）

产品行为变了（草稿未就绪不再渲染任务），凡与问题列表 / 草稿 / 题面控件交互的脚本都必须复跑，
否则「修好了 A」可能只是把 B 弄坏了而没人知道。

| 脚本 | 复跑结果 |
| --- | --- |
| `tauri-cdp-issue-list` | **passed 13/13 ×5**（修复后连跑 5 次：3 次在 `e38f9095`，2 次在 `6eaefd15`） |
| `tauri-cdp-publish-unblock-probe` | 归因不变（仍 `data-fix-insufficient`）；`notSavedSlots=[]`；UI 控件仍可读（q1–q5） |
| `tauri-cdp-recognition-write-path` | **passed 9/9** |
| `tauri-cdp-publish-ready` | **passed 4/4** —— 发布成功路径没有被草稿就绪保护破坏 |

### 领域规则下沉（让 F-R15-5 可单测，而不只靠 E2E 兜着）

第一版修复把判据放在组件里（`editor.loading`），E2E 能过，但**规则本身没有单测**——
而它恰恰是「宁可晚一点显示，也不要显示点不动的按钮」这类容易被后人改回去的约束。

现在把它下沉进 `buildUserTasks`：返回类型加 `ready`，草稿没读进来时返回
`{ ready: false, tasks: [], headline: "正在打开这道题…" }`，**不退化成「没问题」**
——后者会让界面在题稿打开之前就宣称「可以导出」，正是任务书第 2 条要禁的那类假信息。
`ExamWorkspacePage` 的列表分支、顶部入口（`问题 N · 阻断 M`，此前会显示「问题 0」）
与 `data-tasks-ready` 全部改用这一个判据。

新增 3 条单测（`userTasks.test.ts`）：

1. `ds = undefined` → `ready=false`、无任务、`headline !== "可以导出"`；
2. 同一批问题在草稿就绪后落到题号上（`missing-answer:q11+q12`，动作目标 `q11`），而不是退化成 `unnumbered`；
3. 有草稿且确实无问题 → 这时才可以说「可以导出」。

`npx vitest run` → **244 passed / 14 files**；`npx tsc --noEmit` 干净。

### 同类死代码的收尾（`recognition-write-path`）

`tauri-cdp-recognition-write-path.mjs` 里也留着同样的两行（写 `cloudEnabled` + `Page.reload`）。
既然 issue-list 与 recognition-buttons 都清了，这个也一并清掉，保持一致；
删后复跑 **passed 9/9**，与删除前一致。

仍在其他脚本里的 `Page.reload`（`publish-unblock-probe`、`product-chain`、`local-chain`、
`import-batch-timeline`、`freeze-order-proof`）**本轮不动**：它们要么已验证通过、要么不在本轮范围，
在没有新增证据的情况下改一个已经跑通的脚本，只会引入没有验证过的改动。

## 七、未跟踪文件的审查，与「无批次」根因的新证据

上一轮留下 5 个**未跟踪**文件（既没提交、也没说明）。收尾时逐一审查：

| 文件 | 判断 | 处置 |
| --- | --- | --- |
| `tauri-cdp-freeze-order-proof.mjs` | **证据类**：F-R12-2 根因的因果实验（A 无批次 → B 播种 → C 重试） | 提交（并修正其过度结论，见 F-R15-6） |
| `tauri-cdp-import-batch-timeline.mjs` | **证据类**：观测批次是否出现，并抓应用日志里的冻结失败行 | 提交 |
| `inspect-run-db.mjs` | **通用诊断**：只读宿主 SQLite，看批次/决策/链状态有没有落盘 | 提交 |
| `.patch-issue-list.mjs` | 一次性补丁脚本（自称「用完即删」），补丁**已应用**（issue-list 里有 3 处 `ROOT_CAUSE_ALIASES`） | 移到 `tmp/superseded-scripts/` |
| `lib/fake-llm-gateway.mjs` | **坏文件**：`import "./cloud-outline-samples.mjs"` 而该模块**不存在**；且已被 `scripts/controlled-llm-service.mjs` 取代；全仓无任何引用 | 移到 `tmp/superseded-scripts/` |

两个坏/一次性文件是**移走**而不是删除（可逆），没有提交。

### F-R15-6（验收工具，已修）：因果实验把「实验没跑成」读成了「假设被否证」

`tauri-cdp-freeze-order-proof.mjs` 原版在 C 阶段只要「重试后仍无批次」就直接断言
**「H1 不成立：冻结失败并非唯一原因，另有缺陷阻断 reconcile」**。

实测跑一次后，用 `inspect-run-db.mjs` 打开该 run 的数据库，看到的是：

```json
{"stage":"failed","local_status":"failed","cloud_status":"skipped","reconcile_status":"skipped",
 "last_error_code":"editable_draft_exists; pass allowOverwrite=true before regenerating draft",
 "retry_count":"1"}
```

**C 阶段的重试压根没跑起来** —— 它被产品自己的草稿保护（`editable_draft_exists`）挡下了。
「重试没执行」与「重试执行了但仍无批次」是两件完全不同的事，只看识别视图分不出来，
于是脚本把一个**条件不成立的实验**当成了**对假设的否证**。

修法（三处）：

1. C 阶段失败时先只读宿主 SQLite 取 `last_error_code`；命中 `editable_draft_exists` →
   报 **`inconclusive`**（新增退出码 4），不再写「H1 不成立」。
2. 结论从**两态改为三态**：`established` / `refuted` / `inconclusive`
   ——「没证实」和「被否证」不是一回事。
3. 结论的计算**移到日志提取之后**（原来在 `try` 里算，而应用日志要等 `finally` 关掉应用才拿得到；
   算早了就会得出「日志里没有证据」这种自己造出来的结论）。
   同时新增 `freezeFailures` / `freezeFailureObserved`：单独提取冻结失败行。

复跑确认（`causality=inconclusive`）：

```
[freeze-order] causality=inconclusive
[freeze-order] freezeFailureObserved=true
[freeze-order] lines=["[processing] freeze local candidate snapshot failed for import-…: canonical_not_seeded:import-…"]
[freeze-order] conclusion=本实验**不确定**：C 阶段的重试被产品自身的草稿保护挡下
  （last_error_code=editable_draft_exists; …），条件不成立，H1 既未被证实也未被否证。
  不过 H1 的**机制前提有日志证据**：首次导入时冻结确实因权威稿未播种而失败（…canonical_not_seeded…）。
```

### 关于 F-R12-2（「无云导入无批次」）——本轮把根因推进了一步，交给后端

这一条**本轮不改后端**，只把证据整理清楚：

| 观测 | 证据 | 结论 |
| --- | --- | --- |
| 导入后**没有**批次 | `freeze-order` A 阶段：`batchId=null`、四链全 `not_run`；`timeline` 观测 300 秒始终无批次 | 现象稳定复现 |
| 打开工作区**确实播种**了权威稿 | `freeze-order` B 阶段：`hasCanonicalDs=true` | 播种本身没问题 |
| 冻结**确实失败**，原因是权威稿未播种 | 应用日志：`freeze local candidate snapshot failed … canonical_not_seeded:…` | **H1 的机制前提成立** |
| 「播种之后重试就能恢复」**无法验证** | C 阶段被 `editable_draft_exists` 挡下 | **H1 的后半段仍是未知** |

**所以 H1 目前的状态是「机制前提有日志支持、恢复路径未被验证」，不是「被否证」。**
下一步要推进它，需要让 C 阶段的重试真正执行（允许覆盖已有草稿后重跑）——这属于后端侧能力
（`retry_processing` 与草稿保护的交互），本轮只提交证据与工具，不动后端代码。

`tauri-cdp-recognition-buttons.mjs` 的 5 个场景之所以全部 `not-executable`，
**不是**因为「没有批次」——第八节查清了：批次是产出的，卡住的是原文核验链。见下。

### 顺带清理：12 个命令输出残留

仓库根与 `src-tauri/` 下还散着 12 个 `*_out.txt`（`gitcheck_out.txt`、`status2_out.txt`、
`tmp_build.txt`、`stash_out.txt` …）。抽查确认它们全是 PowerShell 重定向残留
（内容里还留着 `Set-Location "F:\workspace\PDF2Test"; git status …` 这类命令行本身），
没有产品价值，已移到 `tmp/command-output-residue/`（**移动不是删除**，可逆）。
`.workbuddy/memory/**` 按仓库护栏**不动**。

至此 `git status` 除 `.workbuddy/memory/**` 外干净。

## 八、把 recognition-buttons 的「无法执行」查到底（F-R15-7），并修正 F-R12-2 的表述

### 起因：数据库里对不上的两件事

`inspect-run-db` 打开 issue-list 的 run 时发现 `recognition_batches_v1` **有 1 行**、
`local_status=succeeded`、`reconcile_status=succeeded`、`actionable_count=14`
——**批次是产出的**。而 `freeze-order` / `timeline` 的 run 里没有批次。
差别在于：后两者**刻意不打开工作区**（它们要观测「只导入不打开」这个条件）。

### F-R15-7（验收工具，已修）：`not-executable` 的归因两处都不准

**第一处：等得太短。** 旧版导入后 `sleep(2000)` 就读决策视图，而实测批次在 **6.2 秒**才落盘
（修正后报告里 `candidateWait={"waitedMs":6274,"batchId":"rec-…-v1-f13bd65cb5f5","localState":"succeeded"}`）。
改为轮询等批次（120 秒超时，或本地链进终态），并把 `candidateWait` 写进报告。

**第二处：文案把原因说反了。** 旧文案是「本仓当前没有真实候选项（actionable/autoApplied 为空）」，
而实测 `actionableCount=14`。真实原因是：**14 条候选全部没有可采纳/可拒绝的动作**
（`needsReview=0`、`autoFixed=0`、其余 14 条无可操作动作），因为 `source` 链
`not_run`/`EVIDENCE_MISSING`：「原文件没有可核验的文本证据，结论只能标记为无法判断。」
（后端侧对应 `unverifiable_count=14`。）

改成按真实情况分岔：真的没有候选 → 说「没有候选」；候选存在但无可操作项 → 说出条数分布
并附 source 链状态。顶层 `verdictReason` 也去掉了「例如没有真实候选项」这个会把原因带偏的举例。

> **我自己的判断也被实测修正了一次。** 我先怀疑是「读得太早」，等待修复确实必要（2s < 6.2s），
> 但**单独修等待并不能让场景可执行**——5 个场景仍然 `not-executable`，真正的阻塞是原文核验链。

### F-R12-2 的表述修正：「无云导入无批次」不准确

| 观测条件 | 结果 | 证据 |
| --- | --- | --- |
| 导入后**不打开工作区** | 无批次 | `freeze-order` A 阶段、`timeline`（冻结 `canonical_not_seeded`） |
| 导入后**打开工作区**（用户正常流程） | **有批次** | `issue-list` / `recognition-buttons` 的库里 `recognition_batches_v1` 有行、`local_status=succeeded`、`actionable_count=14` |

**所以在用户正常流程下，批次是会产出的。** 无批次只出现在「不打开工作区」的观测条件下
（没人播种 → 冻结必然 `canonical_not_seeded` 失败）。`freeze-order` 的 A 阶段正是这种条件，
它测到的「无批次」是**设计造成的**，不代表用户会遇到。上一节据此写的「无云导入无批次」
过于笼统，这里更正。

**真正卡住 A3/A4 按钮流程的是另一件事**：`source` 链 `not_run` / `EVIDENCE_MISSING`
（原文件没有可核验的文本证据）→ 14 条候选全部无法判断 → 没有可采纳/可拒绝的项。
这与 R14 的 F-R14-4（A3 的网关调用只留下输入缓存，没有调用记录、没有输出、服务端零 POST）
指向**同一条链**，交后端。

## 九、把 A3 的「没有 trace」查到根因：一个可复现的 tokio panic

### 9.1 先撤回一条我自己的推断（竞速假设不成立）

上一节末尾我把「导入后点开得慢 → 识别会输掉竞速」写成了推断，并为此在
`tauri-cdp-recognition-buttons.mjs` 加了 `--open-delay`。本轮**实测否证了它**。

`--open-delay 30000`（比真人慢得多）后的批次与不延迟时**逐项一致**：

| 观测项 | `--open-delay 30000` | 不延迟（对照） |
| --- | --- | --- |
| `batchId` | `rec-import-…-v1-f13bd65cb5f5` | 同 |
| `local_state` | `succeeded` | `succeeded` |
| `actionable_count` | 14 | 14 |
| `retry_count` | 0 | 0 |
| `last_error_code` | `null` | `null` |

延迟 30 秒既没有让识别失败，也没有任何副作用（`candidateWait.waitedMs=3195`，
说明打开工作区时批次**早已产出**）。所以「用户操作速度决定识别成败」是**没有证据支持的
推断**，已撤回。脚本里那段断言该机制的注释也一并改掉——留着就是把假结论写进代码。
参数本身保留为诊断旋钮，但不得再用它论证时序。

> 教训（与 §七 F-R15-6 同源）：我连续两次把「机制上讲得通」当成了「已被证实」。
> 讲得通只值得写进假设，不值得写进结论。

### 9.2 A3 请求为什么到不了服务：应用侧在调用途中 panic

`tauri-cdp-controlled-service.mjs` 的 `a3-a4-requests-reach-service` 在 16:52 那次
（commit `937dda5`，含 A3+A4）报 FAILED，证据是：

```
A3（原文件核验）在应用侧已经发起（输入缓存 1 份），却从未到达受控服务：
  服务收到的请求={"total":1,"outline":1,"a3":0,"a4":0}
  网关痕迹={"verify_source_answers":{"input":1,"output":0}}
  llm-calls.jsonl 存在=false
```

**输入缓存写了，但既没有调用记录、也没有输出、服务端零 POST。** 当时脚本只能说到
「这次调用既没有结果也无法诊断」——那是一个**没有根因的结论**。根因就在应用日志里：

```
thread 'tokio-rt-worker' (13152) panicked at tokio-1.52.3/src/runtime/blocking/shutdown.rs:51:21:
Cannot drop a runtime in a context where blocking is not allowed.
This happens when a runtime is dropped from within an asynchronous context.
```

线程名是 `tokio-rt-worker`——**运行时的 worker 线程（异步上下文）**，不是 blocking 池线程。
即：在异步上下文里 drop 了一个 `Runtime`。

### 9.3 判别实验：这个 panic 不是退出残留

「应用退出时打了条 panic」和「panic 就是调用失败的原因」是两件事，不能混。判别办法是找一个
**退出方式相同、但模型路径没被走到**的运行做对照：

| 运行 | 云端 | 批次 | 同一 panic |
| --- | --- | --- | --- |
| `recognition-buttons` 18:37 | **关闭** | **正常产出**（`local=succeeded`, `actionable=14`） | **无** |
| `controlled-service` 16:12 | 开启 | **未产出**（四链 `not_run`） | **有**（线程 12684） |
| `controlled-service` 16:52 | 开启 | **未产出**（四链 `not_run`） | **有**（线程 13152） |

两次云端开启的运行各自复现（线程号不同 → 不是同一条日志被重复读），而云端关闭、批次正常
产出的那次**完全没有**。两者退出方式相同，所以 panic 与模型调用路径绑定，不是退出残留。

### 9.4 根因链：A3/A4 的模型调用**没有**放进阻塞线程池（而云端预取放了）

代码排除法把根因收敛到了一个点。链条如下（全部为只读核对，后端文件**未改动**）：

```
scheduler.rs:595  run_recognition_cycle(...)            ← 同步 fn，被【直接】调用
  └ scheduler.rs:718 source_verifier 闭包
      └ auto_pipeline.rs:1511 verify_source_answers_through_gateway   （同步 fn）
          └ auto_pipeline.rs:1546 run_llm_gateway                     （同步 fn）
              └ llm_gateway.rs:390 reqwest::blocking::Client::builder()…build()
                  ⇒ 函数返回时 drop client ⇒ drop 其内部 Runtime ⇒ 在 async worker 上 panic
```

三条支撑证据：

1. **调用点是异步上下文，且没有 `spawn_blocking`。** `run_recognition_cycle` 的调用方
   （`scheduler.rs:595`）与 `.await`（同函数 `:584`）在同一函数体内 —— 即它跑在 tokio
   worker 线程上。`processing/scheduler.rs` 里 **`spawn_blocking` 出现次数为 0**。
2. **同一个文件里，云端预取是包了的。** `scheduler.rs:405` 的
   `generate_cloud_reading_outline` 外面套着 `tauri::async_runtime::spawn_blocking`，
   注释还写着「模型调用移入阻塞线程池，不占 async runtime」。
   **这条规则只落到了云端预取，没落到 A3/A4。** 这正好解释了实测的
   `outline:1 / a3:0`：outline 走了阻塞池所以成功，A3 没有所以 panic。
3. **应用代码里没有任何别的 `Runtime` 可以 drop。** 全仓 `src-tauri/src` 检索
   `Runtime::new` / `new_current_thread` 无命中；唯一启用 `blocking` 的 HTTP 客户端是
   `Cargo.toml:32` 的 `reqwest = { version = "0.12", features = […, "blocking", …] }`。
   panic 是「在异步上下文里 drop 运行时」，所以能 drop 的运行时只可能是它。

**为什么后端自己的测试抓不到**：`reconcile/commands.rs:2254` 等用例在
`#[test]` 的**同步线程**里调 `run_llm_gateway`，那里 drop 运行时是合法的，
永远不会 panic。缺陷只在「从 async 上下文直接调」时出现——那正是产品路径。

**建议修法**（与 `scheduler.rs:405` 保持一致，一行模式）：
把 `scheduler.rs:595` 的 `run_recognition_cycle(...)` 放进
`tauri::async_runtime::spawn_blocking` 并 `.await` 其 join 结果；
或退一步，在 `run_llm_gateway` 内部把 `reqwest::blocking` 换成异步 `reqwest`。

**尚可再确认一步**：带 `RUST_BACKTRACE=1` 重跑可拿到 panic 的完整栈。
本报告未做这一步——因为无论栈指向哪里，修法都是上面那一条，且排除法已把候选收敛到唯一。

### F-R15-8（后端，已修，P0，原阻塞任务书第 6 条）：模型调用路径在异步上下文里 drop 运行时

> **处置（2026-09-17 本轮）：已修。** 把整段同步识别周期放进**一次**阻塞边界
> （`run_cycle_in_blocking_boundary` → `spawn_blocking`），使 A3 `verify_source_answers` /
> A4 `adjudicate_divergence` 的 blocking HTTP 客户端的**创建、请求、释放**全部发生在该边界内；
> 周期失败经 `settle_cycle_failure` 落**持久化终态**（join 失败 `RECOGNITION_CYCLE_JOIN_FAILED`、
> 普通失败 `RECONCILE_FAILED`，被取消则判 `cancelled` 而非 `failed`），不再停在 processing、
> 也不伪装成核验成功。回归证据（单测 + 真机受控服务）见 `progress.md` 的
> 「2026-09-17 A3/A4 阻塞边界」小节。**下面这段复现记录保留为修复前的证据快照，不再代表当前状态。**
>
> 顺带纠正本条目里的一个观察：当时记为「`verification-status-matches-chains` **PASSED**」，
> 那次通过是**空转通过**——受控验收脚本把 `chainSnapshot()` 的键（`local`/`cloud`/…）直接展开
> 传给 `expectedStatusText()`，而该函数解构的是 `cloudStatus`/`sourceStatus`/…，四个形参全
> `undefined`，规则在第 3 条就返回常量「题稿已生成，可以开始编辑」。批次产不出来时真实文案
> 恰好也是这句，于是「期望 == 实际」在两边都空的场景里成立。本轮已把这层映射补成
> `expectedStatusForSnapshot()`，断言才真正被数据驱动。

- **最小复现**：`node scripts/e2e/tauri-cdp-controlled-service.mjs --diagnostic-args`。
  该脚本写入一个启用的受控 profile，前端按产品正常判定自动 `cloudEnabled=true`
  （`useImportFiles.ts:26-28`），于是云端/A3 路径被走到。
- **预期**：A3（`verify_source_answers`）请求到达受控服务；批次产出。
- **实际**：A3 输入缓存落盘后调用中断——`llm-calls.jsonl` 不存在、无 `-output-`、服务端
  零 POST；**批次根本不产出**（`batchId=null`，四链 `not_run`），连「播种+重试」绕行也救不回；
  应用日志同时出现 `Cannot drop a runtime in a context where blocking is not allowed`。
- **当前构建复现**（exe `6eaefd15…`，后端输入哈希 `5adc278a…` 与 16:52 那次**完全相同**）：

  | 观测 | 值 |
  | --- | --- |
  | `verdict` / `exitCode` | `failed` / `1` |
  | `controlled-service-drives-candidates` | FAILED（无批次） |
  | `a3-a4-requests-reach-service` | FAILED（`a3:0`，网关 `{input:1, output:0}`） |
  | `verification-status-matches-chains` | **PASSED** |
  | `usedSeedRetryWorkaround` 绕行结果 | `batchId=null cloud=not_run`（绕行无效） |
  | panic | `tokio-rt-worker (27860)` 同一条 |

  即：**换前端不改变结论，缺陷在后端。** `accept-manual-candidate` / `undo-manual-accept`
  因此仍是 `not-executable`（没有可采纳的候选）。
- **阻塞影响**：任务书第 6 条的 A3/A4 全部场景、以及「候选采用/撤销」按钮层**全部不可达**。
  云端**关闭**时批次反而正常（`source=not_run/EVIDENCE_MISSING`），所以是「启用模型 → 整条
  链崩」而不是「模型没接上」。
- **归属**：调用链在 `processing/**`、`llm_gateway.rs`（后端独占区）。本轮**未改动**这些文件。
- **根因位置**：`processing/scheduler.rs:595`（A3/A4 的模型调用未放进阻塞线程池），
  完整链条与建议修法见 §9.4。
- **附带交付**：为了让这条根因以后不用再翻日志，受控服务脚本新增 `report.appPanics`
  （`extractAppPanics`），把 panic 提到报告顶层，并在 A3 失败文案里指向它。
  **验证状态要说清**：函数已对**三份真实日志**验证（16:12、16:52、18:40 各提取到 1 条，
  云端关闭那次 0 条），语法校验通过；但 `report.appPanics` 的**写入路径尚未在完整运行中执行过**
  ——18:40 那次运行是在本次编辑**之前**启动的（Node 启动时载入脚本），所以报告里
  `appPanics` 为 `undefined`。写入只是 `finally` 里的两行赋值，下次运行即生效；
  本轮**不把它算作已验证**。

### F-R15-9（后端，未修，P2）：`retry_processing` 丢弃「到底有没有入队」，前端因此只能无条件宣称已入队

`queue.rs:392` 的 `retry()` 返回 `Ok(updated > 0)`，而 UPDATE 带条件
`stage IN ('failed','ready_for_review','cancelled')`。任务处于 `queued`/`running`/
`local_recognition` 等阶段时**更新 0 行、返回 `Ok(false)`**。但 `scheduler.rs:226` 写的是

```rust
retry(&conn, &job_id_owned)?;   // ← bool 被丢掉
```

于是 `retry_processing` 只要 SQL 不报错就**永远返回 Ok(())**。前端
（`ExamWorkspacePage.tsx:223`、`:384`）据此无条件显示「已重新加入识别队列」／「已加入识别队列」。

- **最小复现**：在识别进行中（`stage=running`）点「重新识别」——`retry_count` 不变、
  无 `event_seq` 推进，界面却宣称已入队。
- **预期**：命令能区分「真的重新入队」与「因为阶段不可重试而什么都没做」。
- **实际**：两者都返回 `Ok(())`，前端无法区分。
- **阻塞影响**：违反任务书第 5 条「每个按钮必须有真实作用」的可验证性。
  注意仓库自己已有这条原则的先例——`libraryTypes.ts:138` 写着
  「恢复上限路径**不得谎称**『已自动排队重试』」。同一原则没有覆盖到工作区的重试按钮。
- **为什么本轮不改前端文案**：前端**拿不到**任何可区分信号（命令无返回值、事件只报
  `stateVersion`），在不改后端的前提下改文案只能二选一：要么弱化通常正确的确认，要么
  猜。两者都不比现状更诚实。**根因在后端契约**，故只记录、交后端，不做投机改动。
- **可选的正确修法**（供后端参考）：`retry_job` 用 `retry()` 的返回值决定是否报错
  （例如 `if !retry(...)? { return Err("processing_retry_not_applicable:<stage>") }`），
  前端已有的 `withBusy` 错误路径就会把真实原因显示出来，无需新增 IPC。

### 本轮脚本改动（E2E，已语法校验）

| 文件 | 改动 | 原因 |
| --- | --- | --- |
| `scripts/e2e/tauri-cdp-recognition-buttons.mjs` | 改掉断言「播种先后决定识别成败」的两处注释；保留 `--open-delay` 为诊断旋钮 | 该机制已被 9.1 的实测否证，留着就是假结论 |
| `scripts/e2e/tauri-cdp-controlled-service.mjs` | 新增 `extractAppPanics` + `report.appPanics`，A3 失败文案指向它 | 把「没有根因的结论」变成可读根因（9.2） |



## 十、编辑工作区渲染异常：根因、修复与验收（用户截图 + `demanding-reading-passage-3.pdf`）

复现对象就是用户给的那张截图：1080×617 视口、`fixtures/parser/demanding-reading-passage-3.pdf`（q27–q40）。
新增取证脚本 `scripts/e2e/tauri-cdp-workspace-layout.mjs`（真实 Tauri exe + WebView2 CDP，9 条几何断言）。

### 10.1 先复现：三处缺陷都在真实窗口里量到了

修复前构建 `6eaefd15…`，运行 `run-workspace-layout-2026-09-17T19-16-23-566Z`：

| 快照 | `.workspace-body` 左边界 | 题稿宽 | 建议面板宽 | 题目栏右边界 |
| --- | --- | --- | --- | --- |
| 建议面板关闭 | 0.0 | 1080.0 | — | 1080.0 |
| **建议面板打开** | **556.0** | **524.0** | **556.0** | **1138.0（越过 1080 视口 58px）** |
| **问题列表打开** | **540.0** | **540.0** | **540.0** | **1122.0** |
| 窄屏 900 | **401.7** | **498.3** | **401.7** | 900.0 |

`consoleErrors: []`、`pageExceptions: []` —— **不是运行时异常，是布局错位**。
建议面板高度 284px（内容只有一行状态 + 四个计数胶囊 + 一句说明），其余全是空白。

### 10.2 根因：单列显式网格 + 两个 `grid-row: 3` 生出隐式第 2 列

```css
.workspace-page { display: grid; grid-template-rows: 56px auto minmax(0,1fr); }  /* 没有 grid-template-columns */
.workspace-header      { grid-row: 1; }
.workspace-sub-header  { grid-row: 2; }
.workspace-body        { grid-row: 3; }                        /* 不带 grid-column */
.workspace-recognition { grid-row: 3; grid-column: 1 / -1; }   /* 只有 1 条显式列时 -1 就是第 1 列 */
```

显式网格只有 1 列。按 CSS Grid 自动放置（§8.5）：先落**行列都确定**的项（建议面板 → 第 3 行第 1 列），
再落**只确定行**的项。`.workspace-body` 要第 3 行，而第 3 行第 1 列已被占 → 它只能被推进
**隐式第 2 列**。两列都是 `auto`，在 `width: 100%` 下平分 → 工作区被劈成 556/524 两半。
第 3 行是 `minmax(0, 1fr)`，网格项默认 `align-self: stretch` → 内容仅 150px 高的建议面板被拉满整列
≈ 视口高 − 90px，于是留下那一大片空白。

题稿被推到右半后，body 内部的双栏网格 `minmax(280px,…) 2px minmax(300px,1fr)` 需要至少 582px，
而它只有 524px → 两栏各自压到下限并**整体右溢 58px**（题目栏右边界 1138）。

同一机制解释了「问题列表打开也劈成两半」（540/540）——`.workspace-issues` 没有任何 `grid-row/column`，
是完全自动放置的项，照样触发。**凡是在 `.workspace-page` 下动态出现的兄弟节点都会踩这个坑**，
所以修法不能只针对建议面板。

### 10.3 题号逐字符竖排的根因：全局 `overflow-wrap: anywhere` + flex 的 `min-width: auto`

截图里「27」被折成两行的「2」「7」。两个成因叠加：

1. `src/styles/reset.css:24-27` 对 `:is(p, td, th, label, button, …)` 施加了 `overflow-wrap: anywhere`，
   而该属性**可继承**。行内填空（summary/sentence/note/form completion）的题号是
   `<label class="v2-answer-slot v2-answer-slot-text">` 里的 `<span class="v2-slot-label">`，
   于是也拿到了「任意字符之间都能断」的许可。
2. flex 子项默认 `min-width: auto`，解析结果是**最小内容宽度**；在允许任意断行时，
   「27」的最小内容宽度就是 1 个字符。题目栏一窄，题号就被压到 1 字符宽。

顺带发现：`.v2-slot-label` 在样式表里**完全没有定义**（`grep` 全仓无匹配），一直是裸 `span`。
这也解释了为什么首版探针只查 `.v2-slot-number` 时一个竖排题号都没量到——行内填空根本不用那个类。

### 10.4 题面重复：**源数据层已经重复**，不是前端重复渲染

先说清楚**不是**什么（两条都是实测，不是推断）：

- `duplicateText.echoedCount = 0`：题目栏按句切开 16 句，**没有一句**逐字出现在原文栏。
- `irDuplication.duplicatedLayers = []`：`taskGroups[].instructions` / `[].stimulus` /
  `responseGroups[].prompt` 里**没有任何一层**的文本出现在 `passage.content` 里。

再说**是什么**。`passage.content` 自身（27 个文本节点、6159 字符）里有一个逐字重复的节点：

```
passage-title-text : "You should spend about 20 minutes on Que"
passage-text-1     : "You should spend about 20 minutes on Que"   ← 同一字符串
```

而且这一段是**跨栏重排**的产物，相邻节点连起来是：

```
"You should spend about 20 minutes on Que" | "You should spend about 20 minutes on Que"
| "on pages 10 and 11." | "10" | "stions 27-40, which are based on Reading Passage 3"
```

`Questions` 被切成 `Que` + `stions`，页码 `10` 插在句子中间，整句被拆成 5 个片段。
真实原文应该是「You should spend about 20 minutes on **Questions 27–40**, which are based on
Reading Passage 3 on pages 10 and 11.」——这句话**只应出现一次**。

对照 `taskGroups[0].instructions`，那里是一句完整独立的
「Questions 27 - 31 Complete the summary using the list of words and phrases, A-H, below. …」，
说明被拆碎的这段是**页眉说明**，抽取器把它同时写进了标题节点和正文首个文本节点。

**结论**：重复产生在**源数据（后端抽取）**层，前端按原样渲染。
因此**没有**在前端做任何「按文本相似度删内容 / 隐藏某个字段」的处理——那会丢掉真实题干。
本项作为后端复现材料记录，见 10.7。

### 10.5 修复内容

`src/styles/workspace.css` —— 页面骨架从 grid 改回**单列 flex**：

```css
.workspace-page { display: flex; flex-direction: column; height: 100dvh; overflow: hidden; }
.workspace-header, .workspace-sub-header { flex: 0 0 auto; }        /* 去掉 grid-row: 1 / 2 */
.workspace-notice, .workspace-save-recovery, .workspace-issues,
.workspace-recognition, .workspace-pane-tabs { flex: 0 0 auto; }    /* 动态带统一：按内容取高，缺席归零 */
.workspace-body { flex: 1 1 auto; min-height: 0; }                  /* 去掉 grid-row: 3 */
.workspace-recognition { /* 去掉 grid-row / grid-column */ }
.workspace-body > .empty, .workspace-body > .workspace-load-error { grid-column: 1 / -1; }
```

选 flex 列而不是「补上 `grid-template-columns` 再给每个动态带分配固定行」的理由：动态带的数量和顺序
都会变（通知 / 保存失败提示 / 预检错误 / 保存恢复条 / 问题列表 / 建议面板 / 窄屏切换栏），
固定行号是脆的；flex 列没有「列」可挤，谁出现都只是纵向多一条。
旁证：`.workspace-selection` 一直写着 `flex: 0 0 auto`——这个页面**原本就是按 flex 列写的**。

`src/styles/legacy.css` —— 题号：

```css
.exam-canvas-v2 .v2-slot-number,
.exam-canvas-v2 .v2-slot-label { flex: 0 0 auto; white-space: nowrap; }
.exam-canvas-v2 .v2-slot-label { font-weight: 700; color: var(--exam-accent); }   /* 此前无样式 */
.exam-canvas-v2 .v2-answer-slot-text > input { flex: 0 1 auto; min-width: 4ch; max-width: 100%; }
```

`white-space: nowrap` 关掉元素内部的断行机会，`flex: 0 0 auto` 让它退出收缩计算（不再被同一行的
答案输入框挤压）。二者缺一不可。

`src/styles/exam-canvas.css` —— 答案控件：`.v2-text-answer` 补 `min-width: 0 / max-width: 100% / box-sizing`。

### 10.6 修复后实测（构建 `1e37e626…`，`run-workspace-layout-2026-09-17T19-21-15-641Z`）

9 条断言全部 PASS，且**每条都在 9 个快照下同时成立**（建议面板开 / 关 / 再关、问题列表开 / 关、
窄屏 900、学生预览、回到编辑、保存后重开）：

| 断言 | 修复前 | 修复后 |
| --- | --- | --- |
| L1 题稿不被推到一侧 | 556.0 / 540.0 / 401.7px | **0.0px（全部快照）** |
| L2 题稿占满工作区 | 差 556 / 540 / 401.7px | **差 0.0px** |
| L3 建议面板横跨工作区 | 556 / 1080 | **1080 / 1080** |
| L4 与题稿上下排列（非左右并排） | 水平重叠 0.0px | **水平重叠 1080.0px** |
| L5 建议面板不占大块空白 | 284.0px | **164.0px** |
| L6 题号完整 | 只量到 9 个（漏查行内题号） | **14 个（含 5 个行内题号），换行 0 个** |
| L7 两栏在视口内 | 题目栏右边界 1138 > 1080 | **原文栏 540 + 题目栏 538 = 1080** |
| L8 双栏独立滚动 | auto / auto | auto / auto |
| L9 编辑保存与重新打开 | 未覆盖 | **写入「qa-layout」→ 返回题库 → 重开 → 读回「qa-layout」** |

窄屏 900 快照下，L7 明确区分「设计如此」与「被挤没了」：可见题目栏 900px，
原文栏 `display: none`（由 `.workspace-pane-tabs` 接管），断言据此判定。

### 10.7 断言设计：为什么存在性断言看不见这个缺陷

旧护栏只查「元素存在」与「页面没有横向滚动条」，而这两条在缺陷状态下**全部为真**：
建议面板确实存在，页面也真的没有横向滚动条（溢出发生在 `.workspace-body` 的 `overflow: hidden` 内部）。
所以新脚本断言的是**几何与网格归属**：`getBoundingClientRect` 的实际边界、子项的
`grid-row/column` computed 值、题号的 `white-space/flex-*` 与「高度是否超过 1.6 倍行高」。

三个把「工具自己出错」挡在外面的设计：

- 题号探针**同时**查 `.v2-slot-number` 与 `.v2-slot-label`（首版只查前者，漏掉全部行内题号）。
- 窄屏快照按 `.workspace-pane-tabs` 是否可见来切换判据，不把「按设计隐藏一栏」当成错位。
- 题面重复的判定**分两层**取证：页面内比「题目栏句子是否逐字出现在原文栏」，
  Node 侧比「IR 各层文本是否出现在 `passage.content`」——只做前者会把源数据问题误判成前端问题。

### 复现材料（交后端）

- 夹具：`fixtures/parser/demanding-reading-passage-3.pdf`（sha256 见报告 `identity.fixtureSha256`）
- 权威稿：运行目录 `appdata/data/authoring_hub.db` → `library_items_v2.canonical_ds_json`
- 待查：`passage.content` 内 `passage-title-text` 与 `passage-text-1` 文本完全相同，
  且该句被跨栏重排为 5 个片段（`Que` / `stions` 被切开、页码 `10` 插入句中）。
  期望：页眉说明只出现一次且完整。
- 可直接复跑：`node scripts/e2e/tauri-cdp-workspace-layout.mjs`（报告 `irDuplication` 字段给出各层字符数与重复清单）

### 本轮脚本改动（E2E）

| 文件 | 改动 | 原因 |
| --- | --- | --- |
| `scripts/e2e/tauri-cdp-workspace-layout.mjs` | **新增**：几何/网格归属断言 L1–L9 + 题面重复分层取证 | 存在性断言对本次缺陷全部为真，看不见错位（10.7） |

## 十一、受控服务验收本轮实跑（构建 `1e37e626…`）：首次执行 `appPanics`，并确认卡点在更早一层

### 11.1 `appPanics` 首次在完整运行中被执行

上一轮交付 `extractAppPanics` + `report.appPanics` 时，我如实标注过一句：
「函数本身已对三份真实日志验证，但**写入路径未在完整运行中执行过**」。本轮补上了。

运行 `run-controlled-service-2026-09-17T19-44-39-577Z`（27m53s，`exit=1`），`report.appPanics` 非空：

```
thread 'tokio-rt-worker' (30528) panicked at
  tokio-1.52.3/src/runtime/blocking/shutdown.rs:51:21:
  Cannot drop a runtime in a context where blocking is not allowed.
  This happens when a runtime is dropped from within an asynchronous context.
```

线程号 30528 与此前三次（12684 / 13152 / 27860）不同，panic 位置逐字相同——
F-R15-8 在**新构建**上稳定复现，不是残留、不是偶发。

### 11.2 受控（有云）路径这次卡在**更早**一层：根本没有产出批次

`chains` 四条链**全部** `not_run`：

```
{"adjudication":{"state":"not_run"},"cloud":{"state":"not_run"},
 "local":{"state":"not_run"},"source":{"state":"not_run"}}
```

脚本自陈「默认导入未产出批次（后端冻结顺序缺陷 F-R12-2）」，它内置的两条绕行——
「播种 + 重试」与「派生受控样本（应答目标 → q27）重跑」——结果都是 `batchId=null cloud=not_run`。

对照**无云**路径（`run-recog-buttons-2026-09-17T18-37-06-084Z`）：批次正常产出
（`batchId=rec-import-20260917183711-…-v1-f13bd65cb5f5`、`local: succeeded`、`actionableCount: 14`），
但 `source` 链是 `not_run / EVIDENCE_MISSING`（「原文件没有可核验的文本证据」），
14 条候选全部不可判定 → 5 个按钮场景**没有按钮可点**。

**结论：按钮层验收被上游堵住，不是脚本的问题。** 两条上游各自独立：

| 路径 | 批次 | 卡点 |
| --- | --- | --- |
| 有云（受控服务） | **不产出**（F-R12-2） | 更早，连候选都没有 |
| 无云 | 产出（14 条） | `source` 链 `EVIDENCE_MISSING` → 候选全不可判定 |

### 11.3 A3「到网关但到不了服务」的现场数据

- 受控服务收到：`{"total":1,"outline":1,"a3":0,"a4":0}`
- 网关痕迹：`{"verify_source_answers":{"input":1,"output":0}}`
- 作业目录 `llm-calls.jsonl`：**不存在**

输入缓存有、调用记录与输出都没有、服务端零 POST —— 与 11.1 的 panic 完全一致：
调用在途中崩掉，既没有结果也无法诊断。这就是为什么 F-R15-8 被定为 P0。

### 11.4 场景 8/9 失败的归因（不要误读成独立缺陷）

`a3-partial-not-reported-as-complete` 与 `a3-model-failure-not-reported-as-complete`
报的是「`chains.source` 应当是 `partial`，实际 `not_started`」。
这是**上游的后果**而不是这两条断言本身的问题：A3 从未真正返回，`chains.source` 自然只能是 `not_started`。
要等 F-R15-8 修好、A3 真的跑出「部分返回 / 调用失败」之后才能判这两条。

唯一通过的是 `verification-status-matches-chains`（界面那句话与后端四路链状态逐字一致）。

### F-R15-10（验收工具，已修，P1）：`controlled-service` 的稳定性参数默认关闭 → 无参运行必崩

`tauri-cdp-controlled-service.mjs` 原文：

```js
const diagnosticArgsRequested = process.argv.includes("--diagnostic-args");
const extraArgs = diagnosticArgsRequested ? "--no-sandbox --disable-gpu" : "";
```

紧挨着的注释写着「某些环境下 WebView2 的渲染进程会崩，关掉 GPU/沙箱能让它稳定起来」，
默认却是**关**的。实测：

- 无参运行：**10 秒**即 `CDP 连接已关闭（renderer 或应用退出）`，
  `verdict=incomplete exit=2 reason=没有任何场景被执行`，一个场景都没跑到
- 加 `--diagnostic-args`：正常跑到场景层，27m53s

这与 F-R15-1（`tauri-cdp-smoke`「无参必挂，是脚本自己没照注释做」）是**同一类缺陷**。

**修法**：默认打开，新增 `--no-diagnostic-args` 显式关掉；旧的 `--diagnostic-args` 仍被接受（向后兼容）。
报告字段同步改准确：`runProfile` 由 `cdp-diagnostic`/`cdp-default` 改为
`cdp-with-stability-args`/`cdp-plain`，新增 `stabilityArgs`，`diagnosticRun` 改为只表示
「显式传了 `--diagnostic-args`」——原来的命名会把默认运行误标成「诊断运行」。

**验证**：修复后无参运行越过启动阶段（存活 10 分钟、受控服务已拉起、`derived-outline.json` 已写出），
而修复前无参运行 10 秒即崩。因果链由「无参崩 / 带参不崩」两次运行直接给出。

### 11.5 同类缺陷的分布（仅记录，未一并修改）

`grep "const extraArgs" scripts/e2e/*.mjs` 显示两类并存：

| 默认 | 脚本 |
| --- | --- |
| **开** | `freeze-order-proof`、`local-chain`、`recognition-write-path`、`tauri-cdp-smoke`（F-R15-1 已修）、`workspace-layout`（本轮新增） |
| **关**（需 `--diagnostic-args`） | `controlled-service`（本轮已修）、`issue-list`、`publish-unblock-probe`、`recognition-buttons` |

未一并改动是为了避免与在途改动冲突（这些文件常有其他 agent 的改动）。
建议单独一轮统一，并顺带把 `product-chain` / `publish-ready` 的 `--extra-args` 写法一起对齐。

### 11.6 本轮被主动中断的一次运行

`run-controlled-service-2026-09-17T20-12-53-140Z` 在确认「无参默认已生效」后主动停止：
同构建、同上游缺陷，结论必然与 11.2 相同，再等约 18 分钟不会改变任何东西。
该目录内**没有** `report.json`，已写入 `INTERRUPTED.md` 说明它不能当证据引用。
可引用的完整运行是 `run-controlled-service-2026-09-17T19-44-39-577Z`。

### 本轮脚本改动（E2E）

| 文件 | 改动 | 原因 |
| --- | --- | --- |
| `scripts/e2e/tauri-cdp-controlled-service.mjs` | 稳定性参数默认打开，新增 `--no-diagnostic-args`；报告字段改准确（`runProfile` / `stabilityArgs` / `diagnosticRun`） | F-R15-10：默认关闭导致无参运行 10 秒即崩、一个场景都跑不到 |

## 十二、第二轮：让用户有足够空间阅读和编辑题稿（构建 `a4a4badd…`）

上一轮把「工作区被劈成两半」修好之后，用户给出的新验收重点是**题稿可用空间**：
「问题列表、识别建议同时展开后，题稿被压缩到约 150px」。本轮先量到它，再改。

### 12.1 修复前：两个面板各自带 vh 上限，同时展开吃掉约 80vh

修复前（上一轮的产物，构建 `1e37e626…`，运行 `run-workspace-layout-2026-09-17T19-25-26-065Z`）：

| 快照 | 题稿（`.workspace-body`）高 | 说明 |
| --- | --- | --- |
| 面板全关 | 527.3px | 正常 |
| 识别建议展开 | 363.3px | 建议面板 164px |
| **两个面板同时展开** | **153.4px** | ← 用户报的「约 150px」，实测 153.4 |
| 窄屏 900 + 两个面板 | 可见栏 137px | 更糟 |

原因很直白：`.workspace-issues { max-height: 34vh }` 与 `.workspace-recognition { max-height: 46vh }`
各自独立封顶，同时展开合计约 80vh。视口 617px 时 56px 顶栏 + 34px 模式栏之外只剩约 527px，
被两个面板拿走约 374px。

**这条缺陷的结构性问题是「分别设上限」本身**：面板数量或内容再多一个，同样的加法又会重演。
所以本轮不是把 34vh 调小，而是把上限收到**一处**。

### 12.2 修复：互斥展开 + 共用一个有总高度上限的区域

两层都要有，缺一不可：

- **互斥**由顶栏两个按钮的 `onClick` 保证（打开一个就关掉另一个）——只设互斥的话，单个面板仍可能独占大半屏；
- **共用上限**由新容器 `.workspace-aside` 保证——只设共用上限的话，两个面板仍会同时展开去抢同一个上限。

```css
.workspace-aside {
  --workspace-aside-max-height: min(32vh, 300px);
  display: flex; flex-direction: column;
  min-height: 0; max-height: var(--workspace-aside-max-height);
  overflow-y: auto; overscroll-behavior: contain;
}
.workspace-aside > .workspace-issues,
.workspace-aside > .workspace-recognition { flex: 0 0 auto; max-height: none; overflow: visible; }
```

上限同时给 px 与 vh 两个界（`min()` 取小者）：低高度窗口（520px）下 32vh 只有 166px，
题稿仍能拿到 263.6px；高窗口下 px 界生效，避免辅助面板在高屏上无意义地长高。
溢出交给容器自己滚动，所以面板不再各自带 `max-height` / `overflow`（否则会出现两层滚动条）。

**为什么不用 grid**：见 §10.2——这个页面骨架已经是单列 flex，动态带的数量与顺序都会变，
grid 的固定行号是脆的。

### 12.3 修复后实测（构建 `a4a4badd…`，运行 `run-workspace-layout-2026-09-17T20-59-28-001Z`）

14 条断言全部 PASS，每条在 **12 个快照**下同时成立（面板全关 / 识别建议开 / 问题列表开 /
反向再验互斥 / 再关 / 低高度 520 开与关 / 窄屏 900 开与关 / 学生预览 / 回到编辑 / 保存后重开）。
`consoleErrors: []`、`pageExceptions: []`。

| 状态 | 修复前 题稿高 | 修复后 题稿高 |
| --- | --- | --- |
| 面板全关 | 527.3px | 527.3px |
| 识别建议展开 | 363.3px | 452.7px（建议面板 74.7px） |
| **问题列表展开** | **153.4px** | **329.8px** |
| 低高度窗口 520 + 问题列表 | 未覆盖 | 263.6px |
| 窄屏 900 + 问题列表 | 137px | 274.5px |

新增/替换的 4 条断言（用户任务书第 1/3 条）：

- **L11 `passage-usable-height`**：题稿可用高度 ≥ `max(200, 视口高×32%)`。用比例而不是固定 px，
  因为缺陷的本质是「辅助带按比例把空间吃光」，固定 px 在高窗口下判不出来。实测最矮 263.6px（下限 200）。
- **L12 `no-overlay-on-passage`**：题稿之前的**每一带**（顶栏、模式栏、通知、保存失败提示、
  辅助容器、窄屏切换栏）都必须落在题稿上方。旧版只比「建议面板与题稿的水平重叠」，漏掉其余动态带。
- **L13 `aside-shared-height-cap`**：三条一起才说明「上限被收到了容器上」——
  容器有有限 `max-height`、容器内**同时只有一个**面板、**面板自身 `max-height: none`**。
  实测 `容器 max-height=197.547px / 面板 max-height=none / 容器内面板数=1`。
  另加一条与设计无关的信封 `min(视口高×40%, 320px)`：旧行为（34vh+46vh≈60%）会突破它，所以能拦住回归。
- **L14 `aside-panels-exclusive`**：互斥要**两个方向**都验到（点问题 → 识别建议消失；点识别建议 → 问题消失）。

### 12.4 L8 的判据被替换：从「读 `overflow-y`」改成「真的滚一下」

旧版 L8 只读两栏的 computed `overflow-y` 是不是 `auto`。**这对缺陷恒为真**——
「声明了 `overflow-y: auto`」与「真的能独立滚动」是两件事，用户明确要求不能据此判通过。

现在在页面里真的设 `scrollTop`，再量另一栏的**矩形**与 **scrollTop**：

```
issues-open: 原文栏可滚 3128px / 题目栏可滚 4549px；
  滚原文栏 scrollTop 0→140（题目栏矩形不变、scrollTop 0）；
  滚题目栏 scrollTop 0→140（原文栏矩形不变、scrollTop 140）
```

判据里刻意加了「两栏都真的能滚」这一条：若某栏内容没有溢出，这条断言就**没被行使**，
如实判 FAIL 并写清原因，免得把「没得滚」当成「滚过了，独立」。探针跑完会把滚动位置复原。

### 12.5 没有建议时收敛成一行（任务书第 2 条）

修复前识别面板 164px，内容是「状态行 + 四颗全零计数胶囊 + 一块虚线空面板」。
四颗全零的胶囊不携带任何信息，却要从题稿身上拿走一块高度。

新增纯函数 `isRecognitionQuiet(view)`（`recognitionDecisions.ts`，8 条单测）——
判据刻意**保守**：只要有任何一条待确认 / 无法验证 / 已自动修正的条目，或任何一次过期提示，
或四个计数里任何一个非零，就照常展开完整面板；拿不到视图（还没读到 / 读失败）也返回 `false`
（那是「不知道」，不是「没什么可说」）。

安静时整块面板收敛成一行：`题稿已生成，可以开始编辑 识别还没有产出可核对的结果，这里暂时没有建议可看。`
——第二句只在**四路链路一次都没跑过**时追加：只留前一句会被读成「查过了，没问题」（沿用 §10 的判据）。
实测面板 164px → **74.7px**，题稿 363.3px → 452.7px。

### 12.6 内部题型名 → 用户文案（任务书第 2 条）

`ExamCanvas` 的题组小标题此前直接渲染 `task.taskType`，用户看到的是 `summary_completion`。
`utils/displayLabels.ts` 新增 `taskTypeLabels: Record<TaskTypeV2, string>`（**不是** `Partial`：
契约以后新增题型时类型检查会立刻报缺项，不会再漏一个没翻译的名字）+ `taskTypeLabel()`。
`authoringV2Patches.ts` 里原本还有一份只覆盖 7 个取值的私有表且**无人调用**，
现在委托给同一份映射，避免两张表分叉。实测题面小标题已是「摘要填空」。
单测断言「全部 18 个题型都有中文名，且没有一个等于自己的枚举名、没有一个含下划线」。

### 12.7 顶部按钮文字挤连与问题卡片文字间距（任务书第 2 条）

- **顶部按钮**：`.workspace-header-actions { gap: 0 }` + 按钮 `padding: 0` 对纯图标按钮没问题
  （56px 方框本身就是间距），但顶栏里有两个**带文字**的按钮，零内边距 + 零间隙让它们连写成
  「问题 3 · 阻断 3识别建议」。改为容器 `gap: 2px` + 按钮 `padding: 0 12px` + `white-space: nowrap`。
  图标按钮有 `min-width: 56px` 兜底（16px 图标 + 2×12px = 40px < 56px），所以仍是 56px 方框，外观不变。
- **问题卡片**：`severity-badge` / `workspace-task-title` / `workspace-task-detail`
  **三个类名在样式表里完全无定义**，于是徽标「⚠」与标题零空隙、`<small>` 说明是行内元素
  跟在标题后面随机换行、与下方按钮之间也没有间距。改为两列网格（徽标 | 标题）+ 说明与按钮各占一整行。
- **顺带**：操作按钮原本被 `width: 100%` 撑成每行一个，三张卡片光按钮就占掉约 315px，
  是辅助面板高度的主要来源——与「给题稿留出空间」直接冲突，改为按内容取宽、横向排布。

### 12.8 右侧摘要重复：具体节点与来源（任务书第 4 条）

用户明确指出上一轮给的「文章标题重复」**不是**这一项，要求重查右侧摘要。重查结论如下。

**上一轮的探针轴不对。** `duplicateText` 比的是「题目栏 ↔ 原文栏」，实测 `echoedCount = 0`
（题目栏 16 句没有一句逐字出现在原文栏）——看起来「没有重复」，而用户看到的重复一直都在。
**重复发生在题目栏内部**，所以本轮新增 `QUESTION_DUPLICATION_FN`，按 `ExamCanvas` 的真实渲染结构
逐块取文本（`instructions` / `stimulus` / `responsePrompt`）再两两比对。

**具体节点**（实测 DOM 文本，题组 `group-1`，小标题「摘要填空」）：

| 文本块 | 来源字段 | 原始字数 | 空位形态 | 答案位 |
| --- | --- | --- | --- | --- |
| `.v2-instruction` | `taskGroup.instructions` | 670 | 源文点线 `............`（1 处 12 点） | 0 |
| `.v2-stimulus` | `taskGroup.stimulus` | 636 | 真答案位（输入框 + 作者态 `＋ ×` 工具） | 5 |
| `.v2-response-prompt` | `responseGroup.prompt` | 无此块 | — | — |

两块的最长公共子串 = **449 字**，占较短块 **70.7%**。重复的那一段就是整段摘要：

```
How is social history different from historical study? Since it became an academic discipline,
historical study has been too concerned with a search for 27 [B] by researchers who want to
develop their careers. Social history, however, is more closely related to the 28 [B] of the public. …
```

`instructions` = 「指令句 + 摘要前半段（空位是源文点线）」，`stimulus` = 「完整摘要（空位是真答案位）」
——**两者互相都不完整包含对方**（`instructions` 多一个指令句前缀，`stimulus` 多 30/31 两题的后半段），
所以「谁包含谁」的判据也会漏报。只有最长公共子串能量到真正重复的那 449 字。

**来源判定：源数据层，不是前端重复渲染。** 对照权威稿（真实 IR）：

```
taskGroup group-1 instructions : 670 字（含整段摘要，空位是点线）
taskGroup group-1 stimulus     : 601 字（同一段摘要，含 slot-node-q27…q31 五个真实 answer_slot）
```

两份内容在 IR 里就已经各自存在；前端只是把 `.v2-instruction` 与 `.v2-stimulus` **各渲染一次**。
另外两组题型（`group-2` 判断题、`group-3` 单选题）最长公共子串只占较短块的 3% / 10%，无重复
——说明这不是渲染逻辑的通病，是这一份抽取结果的问题。

**本轮没有修，也没有按相似度删内容。** 上一轮已定过这条边界：源数据重复就交后端复现材料，
前端按文本相似度删内容 / 隐藏字段会丢掉真实题干。因此它记为**诊断项**而不是硬断言
（做成硬断言会让布局验收因为一个无关缺陷长期变红，反而盖住布局本身的回归信号）。

**复现材料（交后端）**：
- 夹具 `fixtures/parser/demanding-reading-passage-3.pdf`，题组 `group-1`
- 期望：`taskGroup.instructions` 只保留指令句；摘要段落只作为 `stimulus` 存在一次
- 可直接复跑 `node scripts/e2e/tauri-cdp-workspace-layout.mjs`，
  报告字段 `questionPaneLayers`（逐块字数 / 点线数 / 答案位数 / 样本）与
  `questionPaneDuplication`（重复对、重复字数、占比、样本）

### 12.9 本轮改动的文件

| 文件 | 改动 |
| --- | --- |
| `src/features/editor/ExamWorkspacePage.tsx` | 新增 `.workspace-aside` 容器；两个面板按钮改为互斥展开（补 `aria-expanded`） |
| `src/styles/workspace.css` | 辅助面板共用上限；问题卡片两列网格与文字间距；操作按钮按内容取宽；顶部按钮内边距与间隙 |
| `src/features/editor/RecognitionPanel.tsx` | 安静态收敛成一行；`data-quiet` 供样式收紧内边距 |
| `src/features/editor/recognitionDecisions.ts` | 新增 `isRecognitionQuiet`（+8 条单测） |
| `src/utils/displayLabels.ts` | 新增 `taskTypeLabels` / `taskTypeLabel`（+3 条单测） |
| `src/utils/displayLabels.test.ts` | 覆盖 18 个题型、兜底、映射表键集合与枚举一致 |
| `src/services/authoringV2Patches.ts` | 删掉私有题型表，委托给 `displayLabels` |
| `src/exam-canvas/ExamCanvas.tsx` | 题组小标题改用 `taskTypeLabel` |
| `scripts/e2e/tauri-cdp-workspace-layout.mjs` | L8 换成真实滚动；新增 L11–L14；新增题目栏内部重复取证；新增低高度视口档 |

### 12.10 本轮遗留问题

1. **右侧摘要重复**（§12.8）：源数据层，待后端修；前端不按相似度删内容。
2. **文章标题重复**（§10.4）：`passage.content` 内 `passage-title-text` 与 `passage-text-1` 文本相同，
   且该句被跨栏重排成 5 个片段。仍是**另一项**问题，与 §12.8 不是同一件事。
3. **下拉菜单的样式被顶栏规则覆盖**（既有，本轮未动）：`.workspace-menu` 在 `.workspace-header` 内部，
   而 `.workspace-header button:not(.workspace-back-button)` 特异性 (0,2,1) **高于**
   `.workspace-menu button` (0,1,1)。于是 `.workspace-menu button` 里写的 `height: auto` /
   `padding: 9px 10px` / `border-radius: 8px` 全部被覆盖，菜单项实际是 56px 高、零圆角。
   本轮只把按钮内边距从 `0` 改成 `0 12px`，没有扩大范围去修菜单（超出任务书范围，且需要自己的验收）。
   修法：把顶栏规则收窄成 `.workspace-header-actions > button`。
4. **辅助面板被上限截断时靠容器滚动，Windows 覆盖式滚动条不悬停时不可见**，
   面板底部内容看起来像「被切掉」。可考虑加渐隐提示，或在面板底部留一条可见的「还有 N 条」。
5. **题稿可用高度是设计取舍**：`min(32vh, 300px)` 让 617px 视口下最矮一档是 263.6px（低高度档）。
   是否够用取决于用户，这个数值可调，改动点是 `--workspace-aside-max-height` 一处。
6. **A3/A4 按钮验收仍等后端**（F-R12-2 有云不产批次、F-R15-8 A3 调用中途 panic），按用户指示不做。
7. `#35`（批次前后成对的面板状态证据）仍缺。

### 12.11 工具与协作备注（不是产品结论）

- **构建归因在双 agent 仓库里会卡住**：清单路径的 `inputsDriftedDuringBuild` 是**一律 stale**
  （刻意如此，`build-freshness.test.mjs` 有专门用例，即使开 `--tolerate-concurrent-edits` 也不豁免）。
  而 `--tolerate-concurrent-edits` 只在 **mtime 路径**（无清单时）生效。
  结果是：后端 agent 并发写 `src-tauri/src/processing/*.rs` 时，连续三次 `build-app.mjs`
  都因为**只有** `backendInputs` 漂移而不可归因，尽管前端两段完全一致
  （三次的 `frontendInputs=6d48be6471fb`、`dist=7f6b61cff7f1` 一字不差）。
  第四次改为「等 45 秒无后端写入再构建」，得到干净构建 `a4a4badd…`。
  这不是缺陷（两条语义都各自正确），但值得知道：**并发写入会让构建归因变成概率事件**。
- **根目录 `tmp_*.txt` 又出现了一批**，时间戳落在本会话期间，是**并发后端 agent** 的输出
  （`tmp_test_processing.txt` / `tmp_build.txt` / `tmp_ice.txt` …），不是我的残留。
  它们在 `.gitignore` 之外但均为未跟踪文件，本轮按**路径精确提交**自己的文件，未去动它们
  （动了可能打断对方的下一步命令）。§七记过的同类清理针对的是我自己那批。

---

## 十三、A3/A4 阻塞边界的复核（前端侧，2026-09-17 晚）

本节是 §12.10 第 6 条「A3/A4 按钮验收仍等后端」的**续做**。用户原话：
「A3/A4 按钮验收等待后端修复后再继续。」后端已把 F-R15-8 修掉（`8e1df28`，
`run_cycle_in_blocking_boundary` 把整段同步周期放进一次 `spawn_blocking`），
本节回答「修好之后 A3/A4 走到哪一步」。

### 13.1 F-R15-8 确已解除（独立复跑两次，逐项一致）

| 观测 | §11 的失败现场 | 本次 |
| --- | --- | --- |
| `appPanics` | 1 条 `Cannot drop a runtime…` | **`[]`** |
| 网关 A3 痕迹 | `{input:1, output:0}` | **`{input:1, output:1}`** |
| 受控服务收到 | `a3:0` | **`a3:1`** |
| `controlled-service-drives-candidates` | FAILED（无批次） | **passed** |
| `a3-a4-requests-reach-service` | FAILED | **passed** |
| `llm-calls.jsonl` | **不存在** | 存在，A3 `ok:true` |
| `source-verification.json` | 无 | `status: succeeded`，14 条 findings 全 `confirmed` |

后端回归 8/8 通过，且**正反两向都有**：正向用例让 A3 穿过真实异步边界打到本地受控服务；
反向用例刻意用修复前的写法，日志里真的复现了
`Cannot drop a runtime in a context where blocking is not allowed`。
「修好了」不是靠「不再报错」推断的，而是靠**反向用例仍能复现旧 panic** 证明的——
这一点值得保留：只有正向用例时，「没 panic」也可能只是没跑到那条路。

### 13.2 回答「尚未判定的一条」：A3 在该轮次**确实没有被调用**

并发 agent 在 `progress.md` 里留了一条待判：

> 「尚不能判定是『A3 在该轮次根本没被调用（无可核验项 → `model_status=NotRun`）』还是
> 『模型通道失败/部分返回没有被如实反映到链状态』。要分清需要带 `--keep` 重跑并检查该轮次的
> `verify_source_answers-input/output` 与 `llm-calls.jsonl`——本轮运行结束后作业目录已被回收，
> 无法事后判定。」

本次运行的作业目录**还在**（脚本默认保留 run dir）。证据（两次运行逐项一致）：

`llm-calls.jsonl`：

| 命令 | 次数 | 时刻（UTC） |
| --- | --- | --- |
| `verify_source_answers` | **1** | 21:32:43（首次导入） |
| `generate_pdf_reading_outline` | 4 | 21:32:48 / 21:37:52 / 21:42:55 / 21:47:59 |

即**三次重跑都重新调了 outline，A3 一次都没再调**。

**为什么**：批次 ID 是**内容寻址**的——`batch_id = (job_id, source_sha256, base_edit_version)`
（见 `scheduler.rs` 候选落盘的注释）。重跑时这三者都没变，于是 `batch_id` 不变：

```
recognition/current.json  → batchId = rec-import-20260917213203-201de082-v1-f13bd65cb5f5
recognition/ 下只有一个 *.source-verification.json，时间 21:32:43（= 首次）
```

而脚本的 `rerunRecognition()` 要求 `decision.batchId !== previousBatchId` 才算「拿到新批次」，
该条件永远为假 → **每次等满 `timeoutMs = 300000` 才返回**。三次重跑的时刻间隔
（21:37:52 / 21:42:55 / 21:47:59，各差约 5 分钟）正好是这个超时值，与推断吻合。

A3 的输入（待核验槽位集合）没变 → 复用首次结论 → `chains.source` 保持 `succeeded`。
因此场景 7/8 期望的 `source: partial` 在当前夹具上**不可能被驱动出来**。

**判定**：这属于「验收工具无法驱动 A3 重跑」的**能力缺口**，
**不是**「模型通道失败没有被如实反映到链状态」。要把两者分开，需要让输入真的变化
（换夹具、改本地答案、或提供强制重核验的入口），而不是再重跑一次。
**本轮不把它算作已修复，也不算作已确认的产品缺陷。**

### 13.3 按钮层仍不可执行：不是「按钮不显示」，而是「没有可应用补丁」

29 条候选的 `proposedPatch` **全部为空**：

| 分组 | 条数 | `resolution` | `cloudValue` |
| --- | --- | --- | --- |
| `cloud-q14`（`LOCAL_SLOT_MISSING`） | 1 | `needs_review` | `["stencilling"]` |
| `q27`–`q40`（`CLOUD_SLOT_MISSING`） | 14 | `needs_review` | 无 |
| `q27`–`q40`（`ANSWER_CONFLICT_UNVERIFIED`） | 14 | `unverifiable` | 无 |

脚本按「`status === "open"` 且 `proposedPatch` 存在」筛候选，因此
`accept-manual-candidate` / `undo-manual-accept` / `late-model-result-…` 三条全部
`not-executable` / `failed`。

**但界面与脚本的判据不同，这点必须说清**：`canAccept()`
（`src/features/editor/recognitionDecisions.ts:163`）只要求
`resolution !== "unverifiable" && status === "open"`，**不检查补丁**。
实测面板 DOM 里因此渲染了 **15 个「采用修正」按钮**（面板全文含
「… → 建议：stencilling 采用修正 保持现状」，以及每条 `CLOUD_SLOT_MISSING` 的
「→ 建议：（无） 采用修正 保持现状」）。

即：**「按钮可见」与「可执行」是两件事**。当前状态是「按钮可见、但没有一条带可应用补丁」。
其中 `CLOUD_SLOT_MISSING` 那些条目的建议值是「（无）」，却同样给了「采用修正」按钮——
语义可疑，但**这是既有设计，本轮未改**（改动会牵动按钮流程的验收判据，需要自己的验收）。

> **补丁为什么一条都没有——完整因果链见 §13.7。** 简言之：云端对真实槽位无值 →
> 不构成答案分歧 → A4 从未被调用 → 补丁从未产生。不是「产生了又被丢弃」。
> 因此本节的「归属」准确说法是**上游（云端候选）**，不是裁决层。

### 13.4 `verification-status-matches-chains`：从「空转通过」到「真通过」

这一条在修复时暴露出它**长期假通过**的完整链条（结论与并发 agent 一致，此处补前端侧实测）：

- 脚本把 `chainSnapshot()` 的键（`local`/`cloud`/…）直接展开传给 `expectedStatusText()`，
  而该函数解构的是 `cloudStatus`/`sourceStatus`/… → 四个形参全 `undefined` →
  规则在第 3 条 `if (!cloudStatus || …)` 就返回常量「题稿已生成，可以开始编辑」。
- R14 时 cloud 链是 `not_run`，前端真实文案**恰好也是**这句 → 期望 == 实际，长期假通过。
- 本轮四链全 `succeeded`，前端改说「云端发现 29 处建议」，期望值仍停在旧句 → 暴露。
- 另有**场景 6 缺 `openPanel()`**：场景 1 重跑识别后工作区重挂，`recognitionOpen` 回到默认
  关闭，`[data-testid="workspace-recognition-cloud"]` 随之从 DOM 消失 → 读到 `null`。
  （场景 7/8 一直有 `openPanel()` + 等待，只有场景 6 漏了。）

**前端侧实测**（复用该次运行的 appdata 直接启动应用，不重跑整条链路）：

| 时点 | 面板 | 状态行 |
| --- | --- | --- |
| 打开工作区后（未点面板） | 不存在（`aria-expanded=false`） | `null` |
| 点开面板后 | 存在 | **「云端发现 29 处建议」** |
| 再等 4s | 存在 | 同上（稳定） |

`errorPresent: false`、`panicLines: []`。**前端渲染是正确的**：四链全 `succeeded` + 29 条
待处理，就该说「云端发现 29 处建议」。

修好映射与 `openPanel()` 后复跑，场景 6 详情：

```
chains: {local: succeeded, cloud: succeeded, source: succeeded, adjudication: succeeded}
pendingCount: 29
statusLine: "云端发现 29 处建议"     ← 期望 == 实际
```

### 13.5 本轮复跑的场景总表

| 场景 | 结果 | 归属 |
| --- | --- | --- |
| `controlled-service-drives-candidates` | passed | — |
| `a3-a4-requests-reach-service` | passed | — |
| `verification-status-matches-chains` | passed | 修好映射与 `openPanel()` 之后 |
| `expected-sample-reproducible` | not-executable | 样本目标 q14 不在本仓夹具（前提不成立） |
| `accept-manual-candidate` / `undo-manual-accept` | not-executable | 无带补丁候选（§13.3，后端裁决层） |
| `late-model-result-…` | failed | 同上 |
| `a3-partial-…` / `a3-model-failure-…` | failed | 验收工具无法驱动 A3 重跑（§13.2） |

`verdict = failed`，但**失败项的归属已全部落到具体位置**，没有一条是「不知道」。

### 13.6 协作备注：同一个文件被两个 agent 在同一分钟改

我在 21:30 改 `tauri-cdp-controlled-service.mjs` 修字段名 bug 时，另一 agent 在**同一分钟**
修了**同一个 bug**，加了功能完全相同的 `expectedStatusForSnapshot()`。处置：删掉我重复的
`statusTextInput()`，保留对方已接线的那份（3 处调用点都已改）；我独有的
「场景 6 缺 `openPanel()`」修复保留。两个改动最终由 `85b76ec` 一并提交。

**教训**：并发 agent 改同一文件时，`git status` 的 M 标记会「凭空消失」——
不是你的改动被回滚，而是对方提交时把工作区一起带上了。判断某句话是谁写进去的，
要用 `git log -S "<那句话>"`，而不是看 `git status`。

### 13.7 按钮不可执行的完整因果链（把 §13.2 与 §13.3 串成一条）

§13.2（重跑不产新批次）与 §13.3（无补丁）**不是两个独立问题，是同一个根因的两种表现**。

实测数据（本次运行的 `decision.json`，29 条）：

| `field` | `resolution` | 条数 | local | cloud | source | `reasonCode` |
| --- | --- | --- | --- | --- | --- | --- |
| `slot_placement` | `needs_review` | 1（`cloud-q14`） | 无 | **有** | 无 | `SUBSTANTIVE_DIVERGENCE` |
| `answer` | `unverifiable` | 14（`q27`–`q40`） | 有 | **无** | 无 | `EVIDENCE_MISSING` |
| `source_coverage` | `needs_review` | 14（`q27`–`q40`） | 有 | **无** | 无 | `SUBSTANTIVE_DIVERGENCE` |

**云端对 `q27`–`q40` 一个值都没给。** 而 A4 的入选条件是
`needs_adjudication()`（`reconcile/adjudicate.rs:229`）：

```rust
cloud_usable
    && item.field == DecisionFieldV1::Answer
    && matches!(item.resolution, NeedsReview | Unverifiable)
    && item.reason_code != USER_EDITED
    && item.reason_code != DEPENDENCY_BLOCKED
    && has_answer_divergence(item)   // 三路里「确实给出值」的答案必须互不相同
```

- `q27`–`q40` 的 14 条 `answer` 项：只有 `local` 一路有值，
  `has_answer_divergence` 的 `distinct.len() > 1` **为假**（`None` 不算一种取值）
  → 不构成分歧 → 不入 A4。
- 唯一的 `cloud-q14` 有云端值，但它的 `field` 是 `slot_placement`，
  被 `item.field == Answer` 挡在 A4 之外。
- 另 14 条 `source_coverage` 同样不是 `answer` 字段，也不入 A4。

于是 `eligible` 为空 → `apply_adjudication` 在 `adjudicate.rs:401` 直接返回 `Succeeded`
（「没有需要模型裁定的分歧」）→ **A4 一次都不调用**（`a4: 0`，与 `llm-calls.jsonl` 里
没有 `adjudicate_divergence` 一致）→ 没有任何 `proposed_patch`（补丁只在 A4 裁定
「采用某一路答案」之后产生，见 `adjudicate.rs:342`）→ 三条按钮场景全部不可执行。

**为什么云端对真实槽位无值**：受控样本的目标是 `q14`，而本仓夹具是 `q27`–`q40`
（脚本注释已写明这一点）。脚本准备了**派生样本**（目标改成 `q27`）正是为此，
但派生样本的重跑**没有产出新批次**（§13.2：batch_id 内容寻址）——于是
`decision.json` 仍是首次那批，云端照旧只回答 `q14`。

**完整因果链**：

```
派生样本重跑 → retry_processing
  → batch_id = (job_id, source_sha256, base_edit_version) 三者未变 → batch_id 不变
  → 复用首次的冻结快照与首次云端候选
  → 云端对 q27–q40 始终无值
  → has_answer_divergence 为假 → needs_adjudication 为假 → eligible 为空
  → A4 从不调用（a4: 0），adjudication 如实报 Succeeded
  → 没有 proposed_patch
  → accept / undo / late-model 三个场景全部 not-executable
```

**这条链上没有任何一环是「错」的**：batch_id 内容寻址是对的（保证幂等）、
`has_answer_divergence` 只看有值的链是对的（`None` 不该算一种取值）、
`field == Answer` 的限定是对的（槽位放置不是答案分歧）。**堵点在于验收工具
没有能力把「不同的输入」送进去**——它依赖 `retry_processing` 造新批次，
而重试的语义恰恰是「同一输入再算一次」。

## 2026-09-19 本轮收口补记

- 合并提交：`aa443d5`；合并后基线 Rust 850 passed / 0 failed / 11 ignored，Vitest 309 passed / 0 failed；real-PDF harness 因 private corpus 缺失外层报红，Rust 内层明确 skip，非合并回归。
- DOCX 真实 fixture 的题面丢失落点在 `ielts_grammar/mod.rs:1013-1066` 的 V2 response 投影：结构题无条件吞掉已有 prompt_text；改为仅在 prompt_text 为空时抑制 detached qN。product-chain 反例已验证 complex-reading DOCX 两个 response group 均有真实文本、无 pending placeholder。
- blocking 无目标已有后端反例 `cloud_repair/tests.rs:2330`，动作由后端变为 `review_source`，前端给出“打开原文件核对”；本轮补了 blocking/no-target 前端断言。
- 重试通过独立 attempt/batch 身份进入现有 reconcile/cloud-repair 写入边界；真实链路反例覆盖人工 q14 保持、q15 新改进落库、q14 进入剩余任务。
- CAS 的 `Ok(0)` 反例与修复已落在 `library/repository.rs:1153-1190`；force 前端仅保留 strict，并对 force 明确报错。
- E8-26 的发布判定未改变；修复的是 readiness blocker 可观察性，预览报告新增 `publishReadiness`。option_alphabet 样本统计仍为 58 groups / 4 false positives / 0 true mistakes，不改顺序。

**给后端的最小交接**：要验按钮流程，需要一条能**改变云端候选**的路径
（换夹具使其与样本目标一致，或提供强制重核验/换样本的入口），
而不是继续依赖 `retry_processing`。这也解释了 §13.3 里那个「按钮可见却没有补丁」
的表面矛盾——补丁不是被丢掉的，而是**从头到尾没有任何一条候选走到会产出补丁的那一步**。

## 2026-09-19 最终验证补记

- 全量测试没有合并回归：Rust `856/0/11`、Vitest `311/0`；真实 PDF harness 的唯一失败是缺私有 corpus，内层 exact test 未执行而是 skip。
- 当前仓库的 `fixtures/parser/complex-reading.docx` 实际是 5 题、2 个 response group；修后两组 prompt 均有真实文本且无 `[prompt pending review]`。另一个 13 题的 demanding-reading fixture 是 passage-only，缺少题目/答案表，不能作为同一输入问题的反例。
- option alphabet 真实样本统计保持：`58 groups / 4 false positives / 0 true mistakes`，因此没有改循环顺序。
- 单一工作区状态已恢复：`main` 指向 `aa443d5`，发布分支/worktree 已移除；用户原有 `.workbuddy` 改动保留。

## 2026-09-19 提交后 CDP 复核纠正

### 指定 PDF harness

首次直接运行因前端 `dist` stale-build 被前置检查拒绝；使用仓库既有 `npm run build:app` 刷新前端和 debug exe 后，原命令
`node scripts/e2e/tauri-cdp-cloud-repair-chain.mjs` 使用 `fixtures/parser/demanding-reading-passage-3.pdf` 获得：

- `prepass-import-for-scenario-derivation` passed
- `derive-scenario-from-real-draft` passed
- 其余真实导入、云端候选、修复、画布刷新、保存/重开、预览、导出和学生 runtime 场景全部 passed
- 总计 **13/13 passed，exit 0**

因此此前把这条链归因于 `fixtures/golden/private-real/` 缺失是错误的。private-real 缺失只影响另一条
`npm run verify:phase5:real-pdf` 脚本。

### DOCX 结论必须收窄

同一条脚本指定 `--pdf fixtures/parser/complex-reading.docx` 后：

- 真实 Tauri UI 导入 passed；报告记录 `taskGroups=2`、`responseGroups=2`、`placeholderPrompts=0`。
- `derive-scenario-from-real-draft` failed，原因是脚本加载的 golden `demanding-reading-passage-3.annotation.json` 固定 source hash `f13bd65c...`，而 DOCX 实际 hash 为 `717918f5...`；脚本因此没有继续到 UI 云端修复/预览/导出。
- 同次运行落盘的 `authoring-ir-v2.shadow.json` 显示质量状态 `blocked`，硬阻断为 `SLOT_HOST_MISSING` 与 `RUNTIME_COMPILER_FAILED`。后者包含 `RUNTIME_CHOICE_SLOT_ANSWER_NOT_OPTION` 和 `RUNTIME_RESPONSE_ANSWER_KIND_MISMATCH`。

准确结论：本轮已证明 DOCX 题面不再被投影成 placeholder，并且服务层能把同一 DOCX 送到云端修复网关；本轮没有证明用户可以用当前 DOCX 走完产品级“导入→云端修复→预览→导出”，当前 fixture 仍被质量门禁阻断，完整 CDP 链还被 PDF-only golden 绑定阻断。

## F-REC-SURVEY-2026-09-20

- 真实听力 PDF 的 V1 parser 是 pdfium（非 fallback），但关键题目页的词边界在输出中仍缺失或被错误插入；上游 `Questions`/instruction 解析因此全线未命中。
- 听力实际落盘为 0 task groups / 0 slots；7 个 golden 阅读样本的 V2 组/slot 结构与标注一致，但所有答案值均 unresolved，质量门禁仍 blocked。
- 识别层 Rust 没有正则引擎调用；主要是空格敏感的 literal `contains`/`starts_with`/字符扫描规则。阅读语料应按 SHA + 结构标注 + known issue 作为私有回归输入，结构门与答案/OCR门分离。

## F-ANSWER-EVIDENCE-2026-09-20（初步）

- 现有答案视觉代码并非完全缺失：`auto_pipeline.rs` 已有 `main_pdf_vision_extraction`、`vision_answer_candidate_for_job` 和 `vision-answer-candidates.json` 持久化路径；但从命名和调用位置看，当前更像云端/诊断候选链，尚未证明它会把候选写入 authoring `answerKey`。
- `parser.rs` 已有 Python PDF image sidecar、pdfium page-render fallback 与 `PdfImageExtractionV1` 合并逻辑；需要继续确认 Windows/Tauri 默认设置、调用调度和答案页是否包含在 extraction pages/assets 中。
- `parse_answer_source_candidates` 只遍历独立的 `SourceFile.role == "AnswerKey"`，不能直接解释同一份阅读 PDF 的答案页；主 PDF 答案页的 page-role 识别和答案键唯一写入入口仍待定位。
- 七份真实答案页已渲染检查：均为清晰的单页/多页答案表或说明页，版式不是简单逐行纯文本；存在表格、多栏/分组标题、答案与证据解释并列。V2 物理层把答案页判为 `scanned`，每页有 `PDF_IMAGE_ONLY_PAGE_REQUIRES_OCR`、`requiresOcrRegions=1` 和至少一个 image placement/assets。
- 直接调用仓库 Python sidecar 的首次尝试在当前 Python 3.12 环境失败：`missing_pdf_dependency:pypdf:No module named 'pypdf'`。这不是产品断言；需继续核对 Rust fallback 与产品运行时的 Python 解析器选择，不能把 sidecar 能力当成已可用。

## F-ANSWER-EVIDENCE-2026-09-20（实施收口）

- 七份 golden 的答案页均走真实 Tauri 导入、scheduler、pdfium 渲染和视觉网关请求；答案页页号分别为 `5`、`5`、`5`、`6,7`、`7,8`、`5,6`、`5`。
- 在受控视觉服务返回人工从原图核对的答案表后，canonical answer slots 从 `0/95` resolved 变为 `95/95`，7 份逐项人工核对错误为 0；应用报告均标 `source=answer_page_recognition`。
- Western 首次 `8/13` 是真实暴露的罗马数字选项大小写映射缺陷；先有失败单测/产品复跑，再把 option labels 统一成大小写不敏感比较，修复后 `13/13`。
- 空白答案页单测证明旧的答案页机器值会清回 unresolved；无扫描答案页的 `121. P2(仅原文无题)` 负控生成 `answerPageImageCount=0`、不调用视觉抽取、`applied=false` 且无错误。
- 全量 Rust `860 passed / 0 failed / 11 ignored`，Vitest `311 passed / 0 failed`；指定 `tauri-cdp-cloud-repair-chain.mjs` 使用仓库内 demanding-reading fixture，13/13 passed。

## F-ANSWER-GENERALIZATION-2026-09-20（进行中）

- 本轮验证目标是未参与开发调参的外部阅读 PDF；上一轮 7 份 golden、`231. 铅笔的历史(流程图版)` 和 `121. P2(仅原文无题)`必须排除。
- 运行必须逐文件 staging，不能复用 folder hook 的目录批量导入行为；每份样本需要保留 source SHA、答案页物理分类、图像请求/候选/应用产物和约束筛查结果。
- 真实错误率只能由人工查看原卷确认；受控视觉服务返回的答案不能单独作为正确性证据。自动约束只负责缩小人工复核集合，不把“未违反约束”当作正确。

## F-ANSWER-GENERALIZATION-2026-09-20（外部服务阻断）

- 固定样本已冻结：277 份总体、268 份排除后候选、28 份样本，seed `20260920`，manifest 提交 `fab14d3`。
- 产品真实 Tauri profile 已确认 `hasApiKey=true`，凭据来自 OS secret store；同一 endpoint/model 的两次 profile 测试都返回 `HTTP 503 model_service_unavailable`。
- 28 份视觉批跑没有有效完成样本；第一份的单文件 staging 在视觉产物生成前中止，不计入指标。没有把服务不可用误报为“无错误”。
- 因此真实错误率、覆盖率、resolved 率、约束违反率、低置信度触发和人工确认错误均为 N/A；本轮没有调参数、提示词、阈值或产品代码。

## F-ANSWER-FAILURE-CONSTRAINTS-2026-09-20（进行中）

- 轨一现状：答案页路径的失败不会通过 `apply_vision_answer_candidate` 写入答案，scheduler 也会继续收尾本地稿；但 `visionAnswerExtraction` 只有 `attempted/applied/failure`，没有 `succeeded/failed/not_executed` 三态，也没有把“没有答案页”和“有答案页但服务未执行”做成不同原因。
- 轨一现状：`run_cloud_conversion_worker` 将答案视觉错误带回主线程，主线程等待 worker；需要对失败结果显式落盘终态，保证用户看到可重试动作而不是 running 或无答案页提示。
- 轨二现状：选项答案已有基于 response group/option bank 的范围拒绝；文本答案路径直接构造 `kind=text`，尚未检查 slot/group word limit、题号越界/连续性或组内答案分布异常。

## F-ANSWER-FAILURE-CONSTRAINTS-2026-09-20（完成）

- 受控 gateway 的 503/超时/401/403 现在落为 `not_executed`，分别带 `service_unavailable` 或 `credentials_invalid`；HTTP 成功但畸形 JSON 落为 `failed/invalid_response`；没有扫描答案页落为 `not_executed/no_answer_page`，三者在 pipeline report 和前端动作上分开。
- 有答案页但 503 的红测证明本地 authoring draft 会完成落盘、job 不停在 `Working`，answerKey 保持 unresolved；重试红测两次实际调用 `extract_pdf_image_answers`，没有复用失败缓存。
- 语义守卫覆盖 TFNG/Y-N-NG、选项/配对范围、实际选项集合、word/number limit、题号闭包/连续性和同组完全一致分布提示。7 份 golden 的 95 个已知答案全部通过，注入 `Yes please`、`K`、五词填空均被拒绝，命令构造器为这些答案生成 0 条 `setAnswer`。
- 全量验证：Rust `870 passed / 0 failed / 11 ignored`，Vitest `315 passed / 0 failed`；28 份未见样本没有因服务仍未形成有效视觉候选而启动，真实错误率继续记为 N/A（0/0，不是 0%）。

## F-REAL-MODEL-CLOUD-REPAIR-2026-09-20（进行中）

- 本轮问题是网关上的真实模型能否遵守现有五类请求契约并驱动自主修复链，不以识别准确率或 13/13 场景通过为目标。
- 所有结论必须来自真实 HTTP 响应与真实 Tauri/CDP 产品路径；模型列表、协议形状、失败码、调用数、token 与时延将作为基线落入 NOTES。
- 阶段 0：网关只列出 3 个 xAI 名称模型。`grok-4.3/4.5` 能按产品所需 `json_object` 返回标准 JSON，三者也能发原生 tool call；`grok-4.6` 后续有 502。所有视觉最小请求均 503，尚无可用视觉模型。

## F-REAL-MODEL-CLOUD-REPAIR-2026-09-20 阶段 1（五类生产请求实测）

运行方式：`cargo test --lib real_gateway_five_semantic_contract_probe -- --ignored`，模型 `grok-4.5`，
走**产品自己的** prompt 构造器、HTTP 客户端与校验器（`llm_gateway::run_llm_gateway`），未改 prompt、
未改校验器、未改阈值。key 从 OS 凭据库读出，不进入源码、日志或产物。

| 请求 | 结果 | 时延 | 备注 |
| --- | --- | --- | --- |
| `generate_pdf_reading_outline` | **通过** | 32.1 s | 返回 `title/groups/answerKey/confidence/warnings/metadata`；5714 tokens（其中 reasoning 1944） |
| `verify_source_answers`（A3） | **失败** | 1.8 s | `source_verification_findings_missing_or_invalid` |
| `adjudicate_divergence`（A4） | **失败** | 2.4 s | `adjudication_rulings_missing_or_invalid` |
| `generate_authoring_candidate` | **通过** | 83.0 s | 返回 `passage/taskGroups/answerSlots/answerKey/unresolvedRegions/sourceCoverageNotes/warnings` |
| `repair_authoring_step` | **通过** | 48.8 s | 首个工具调用是 `read_draft`——先读再改，没有凭题面直接下笔 |

即 **3/5 通过**。两条失败都不是 HTTP 故障：`llm_gateway.rs:1475-1478` 的顺序是
`openai_post` → `openai_chat_content` → `parse_llm_json_content` → 契约校验，因此这两个 errorClass
只可能在 **HTTP 200 且 JSON 解析成功之后**产生；`llm-calls.jsonl` 也记录 `ok=false` 且 errorClass 为
契约类而非 `llm_http_*`。三态没有坍缩。

### 根因（已由源码逐条核对，不是推测）

**A3/A4 的 prompt 从未声明校验器强制要求的顶层信封键。**

- `source_verification_prompt`（`llm_gateway.rs:1389`）写了 6 条 hard rules，逐条讲 `verdict` 枚举、
  `quote`、`pageIndex`、`observedValue`，但**通篇没有出现 `findings`**。而
  `validate_source_verification_output`（`:1510`）第一件事就是取 `output["findings"]` 数组，取不到整份拒绝。
- `adjudication_prompt`（`:1258`）同样：5 条规则讲 `chosen` 枚举、`value` 逐字一致、`rationale` 非空，
  **没有出现 `rulings`**；`validate_adjudication_output`（`:1625`）取不到 `rulings` 即整份拒绝。
- 对照组是决定性的：三条通过的请求，prompt 里**都**明确写了信封。
  `cloud_outline_prompt`（`:506`）"Return exactly one JSON object with this shape: {…\"groups\":[…]}"；
  `repair_step_prompt`（`:1114`）"exactly one object {\"callId\":…,\"tool\":…,\"arguments\":{…}}"；
  候选识别同理。**通过与失败的分界线正好落在"prompt 有没有声明信封"上。**

### 为什么这个缺陷能活到今天

受控假模型 `scripts/controlled-llm-service.mjs` 是**照着校验器**写的（它的文件头就写明"必须回
`{findings:[…]}`"），所以它永远"记得"这个键。prompt 与校验器的不一致在假模型下**结构上不可观测**。
这正是交接文件 §3「验收自证」那一类，只是换了个位置：不是断言自证，而是**被测对象的两端由同一份
理解写成**，于是两端之间的缝隙没有任何测试能看见。真实模型第一次发请求就踩中。

注意这**不是**"真实模型不够好"。模型无从知道一个从未被告知的键名；把它算作模型能力不足会指向
完全错误的修复方向（换模型、调温度、加重试），而正确修复是把校验器已经要求的信封写进 prompt——
**不放宽任何一条校验规则**。

### 附带发现：契约被拒时不留原始回复

`run_llm_gateway` 只在成功时写 `<command>-output-*.json`；契约校验失败时只留 input 与
`llm-calls.jsonl` 里的 errorClass，**模型到底回了什么没有落盘**。本轮靠对照 prompt 与校验器源码
反推出根因，但产品里用户遇到 `MODEL_INVALID_OUTPUT` 时同样没有证据可看。已作为独立卡片记下。

### 环境事实（本轮反复踩到，记给下一位）

`cargo test --lib` 结束并打印 `test result` 之后，测试二进制
`target/debug/deps/ielts_author_studio_lib-*.exe` **仍留在进程表里**，于是下一次构建在链接阶段以
`LNK1104 无法打开文件` 失败。本轮命中 4 次。绕过办法：每次 `cargo test` 前先
`Get-Process | ? { $_.ProcessName -like '*ielts_author_studio*' } | Stop-Process -Force`。
这不是本轮改动引入的（第一次命中发生在只跑既有 ignored 测试时），但它会让人把链接失败
误读成代码错误——本轮第一次就差点这样归因。根因未查（疑似某个测试留下非 daemon 线程或
被杀进程的 pending-delete 状态），单独记卡。

### 阶段 1 末尾：网关凭据失效，真实模型复验未完成

修完 prompt 信封后重跑探针，五条请求全部 `llm_http_401 INVALID_API_KEY`（约 460 ms/条）。
用最朴素的 `GET /v1/models`（只带 Bearer、无请求体）单独探测：同样 401，485 ms；key 仍在 OS
凭据库、长度 67 未变；而阶段 0 同一个 `GET /v1/models` 是 200 并列出 3 个模型。

结论按三态记：**未能执行（`not_executed` / `credentials_invalid`）**，不是"修复失败"，也不是
"真实模型不合格"。prompt 修复目前只有单元级证据。阶段 2/3 未开始，原因是凭据失效。

## F-REAL-MODEL-CLOUD-REPAIR-2026-09-20（Windhub / grok-4.3 复测）

- 新网关 `https://windhub.cc/v1` 的 `/v1/models` 返回 200，共 12 个模型；`grok-4.3` 的最小 JSON 与原生工具调用均返回 200。`gemini-3.8-flash` 的最小 `image_url` 请求也返回 200，但本轮完整链固定使用 `grok-4.3`，没有跑视觉批量。
- 五类生产请求全部得到合法协议结果：outline 27.293 s、A3 53.133 s、A4 21.869 s、candidate 32.339 s、repair step 16.364 s。A3/A4 结果受当前工作树已有的 prompt 信封改动影响，不能当作干净 HEAD baseline；本阶段没有新增 prompt 改动或校验放宽。
- 真实 Tauri/CDP 链使用仓库 PDF fixture 跑了两遍导入。两次 `generate_authoring_candidate` 分别耗时 134.043 s、144.323 s，均为 `llm_timeout_budget_exhausted`；本地初稿和题目结构仍落盘，但云端候选写为 `not_run/CLOUD_DISABLED`。
- 因候选请求失败，完整链实际为 2 次候选、0 次修复，没有 `read_source`、`sourceAnchors`、`apply_edits`、`record_ruling` 或 `finish` 证据。模型协议层可用，但当前预算下不能驱动真实自主修复链；主要阻断是完整候选请求的时延/预算。
- 失败记录没有保存 usage，所以完整导入 token 数为未知；不能用小型阶段 1 请求的 token 数代替。两次候选等待合计 278.366 s。artifact：`artifacts/e2e-cdp/run-cloud-repair-chain-2026-09-20T13-22-27-910Z`。
- 阶段 2 因候选失败停止等待并清理临时进程/OS secret；没有把“未进入修复回合”误报成模型已经收敛。

## F-CLOUD-TIMEOUT-ATTRIBUTION-2026-09-20

复核 windhub / `grok-4.3` 那次真实链失败的落库状态，得到两条与"API 好不好"无关的结论。

### 1. 这次失败在界面上被显示成"未启用云端"（已修）

同一次运行的库里两行互相矛盾：任务行 `cloud_status=failed` +
`last_error_code=llm_timeout_budget_exhausted:llm_http_timeout:…`、`progress_json.cloudEnabled=true`；
批次行 `cloud_status=not_run`、`cloud_reason_code=CLOUD_DISABLED`、
`stages_json.cloud.message="本次导入未启用云端识别。"`。

前端状态行读批次行，`not_run` 被 `recognitionClient.ts:224` 归一成 `not_started`，最终渲染
**「题稿已生成，可以开始编辑」**——134 秒的真实超时在界面上消失。用户不会去排查一个真实存在的
故障，下一个接手的人也会被这条 `CLOUD_DISABLED` 指向错误方向（正是交接文件 §3 第 2 条那个坑）。

成因是接缝：批次行由本地周期建出，本地周期按设计 `cloud_enabled=false`，它写的 `CLOUD_DISABLED`
当时是诚实的；`scheduler.rs` 指望 advance 用真实修复状态覆盖，但 `write_batch_repair` 只写
`repair_json`，而候选失败时修复循环根本没启动。没有任何代码改写这一格。

修复只补"云端起了却没交出可用结果"这一条路径，原因码复用 `classify_cloud_error`
（超时 → `unusable`/`MODEL_TIMEOUT`），前端既有文案随即正确
（`unusable → unavailable →「云端校验暂时不可用，不影响继续编辑」`）。成功/部分成功路径未改，单独排卡。

### 2. "API 太慢"目前**不成立**，因为我们从未测到它的完成时间

- 客户端预算来自 profile `timeoutMs`：harness 写的是 `120000`
  （`scripts/e2e/tauri-cdp-cloud-repair-chain.mjs:355`）。
- 记录的 134.043 s / 144.323 s 是 `run_llm_gateway` 的**整格时延**，包含 HTTP 之前的本地准备
  （读 PDF + base64 + prompt 构造）。即 ≈14 s / ≈24 s 本地 + 120 s HTTP 预算。
- 错误串 `llm_http_timeout:error sending request` 是 **reqwest 客户端超时**——是我们先挂断的。
  因此已证明的只有"服务端 > 120 s"，**没有**证明它 > 144 s，更没有证明它不会在 180 s 或 240 s 返回。
- 重试不会放大等待：`openai_post_once` 拿到的 timeout 就是剩余预算，第一次尝试即可用尽全部预算，
  之后 `remaining.is_zero()` 直接退出（`llm_gateway.rs:311-338`）。所以 278 s 是两次导入各一次候选，
  不是重试叠加。
- 真实上限是硬编码的：`llm_timeout` 把 `timeoutMs` clamp 到 **1 s–300 s**（`llm_gateway.rs:142-150`）。

### Welfare / grok-4.6 复测

- `/models` 200，明确列出 `grok-4.6`；最小 JSON 和 native tool call 都一次成功，因此基础
  OpenAI-compatible 协议不是当前阻断。
- 产品 `generate_cloud_authoring_candidate_raw` 的真实完整请求在生成前被 HTTP 402 拒绝：
  `available=2.132969`、`required=6.01536 credits`。这不是超时，也不能评价模型输出质量。
- 命令输入缓存 12,414 bytes 不等于 wire payload；实际 HTTP body 还包含 PDF/base64。网关未
  返回 completion，因此 token usage 不可得。
  即便设置页填更大的值也不会超过 300 s。这是一个常量，不是接口设计问题。

结论：判"API 不行"之前必须先有一次**不设 120 s 上限**的测量。在没有这个数之前重写网关接口层，
与当年把 PDF harness 失败归因到缺私有语料是同一类错误。

## F-SOURCE-COVERAGE-REVIEW-2026-09-20（独立复核）

对 `719c901` 的复核结论：**实现与"不能伪报完整"这条硬规则一致**，链路是通的，不是只加了个分数。

逐条核到的事实：

- 阻断链完整:`SOURCE_QUESTION_COVERAGE_MISSING` 是 blocking issue → 进 `hardFailures` →
  `readiness_from_facts` 第一条即 `Blocked`（`quality.rs:73`）→ `authoring_validation.rs:449` 把任何
  非 `Ready` 映射成 `PublishVerdict::Blocked{QUALITY_NOT_READY}` → 导出被拦。不是只压低
  `sourceCoverage` 分数。
- `undetermined` 确实不阻断：它是 warning，而 `readiness_from_facts` 只看 hardFailures、
  document_score、low task score、节点级 `source_coverage` 分和 unresolved blocking 数——
  warning 不进其中任何一项。与"提示但不阻塞"的产品语义一致。
- **两向比对**是这版实现最关键的一点：`missing`（declared − canonical）与
  `extra`（canonical − declared）任一非空都判 `Missing`。因此字形空格合并这类启发式一旦解析
  短了（例如把 27–40 读成 27–31），多出来的 32..40 会落进 `extra` → 仍然阻断。
  **启发式的失败方向指向安全侧，不会变成"系统说没问题但漏了题"。**
- 任何一条声明解析失败即 `Undetermined`，且该分支排在 missing/complete 之前。所以一条乱码声明
  不会被另一条"看起来完整"的声明盖过去。
- `answer_box_context` 守卫避免把随机的 "box 1" 表格单元当成全卷题域。
- listening 由 modality 显式跳过，不会在听力实现前误报阻断。
- 12/13 → 13/13 的根因有记录（NOTES §13）：pdfium 把多位题号抽成 `Questions 2 7 – 3 1`，
  先有真实反例再修，且只在 coverage 的数字声明入口合并相邻数字的字形空格。不是无解释的红转绿。

两处小瑕疵（不影响收口，不单独开卡）：

1. blocking 文案固定为「可能有题目被漏掉」，但同一个码也在 `extra` 非空时触发——那种情况事实
   相反（稿里有、原文没声明）。`details` 里两个集合都有，只是首句措辞会误导。
2. `undetermined` 的那条单测只断言"不在 hardFailures 里"，没有断言 `state` 仍未被阻断。
   若将来有人把它改成 blocking，这条测试**仍会通过**。补一句
   `assert_ne!(report["state"], "blocked")` 就能封住。

## F-WEBVIEW2-CDP-UNAVAILABLE-2026-09-22（**已更正：执行环境问题，非机器**）

> **2026-09-23 复核更正。** 本条原结论「本机 WebView2 DevTools 远程调试端点完全无法建立 /
> 本机当前状态退化」**不成立，已推翻**。质量方在**同一台机器、同一分支（`wave3-listening`）
> 的新构建**上实跑：`scripts/e2e/tauri-cdp-smoke.mjs` **5/5 通过**、
> `scripts/e2e/tauri-cdp-cloud-repair-chain.mjs` **13/13 通过**（阅读无回退）。
> 所以 CDP 通道本身是好的：**不是机器、不是代码回归**。
> 同理，当时的 T5-5 与 T6 **不是被机器阻塞**，只是当时的执行环境起不来端点。

**真实原因（执行进程环境被污染）**，当时启动 App 的进程里带着：

- `PATH` 首项被写坏——`dirname`/`cd` 等基础命令直接 `command not found`，子进程查找同样受影响；
- `HTTP_PROXY` / `HTTPS_PROXY` 指向宿主代理，使**本地探测给出假阳性**（见下「配套的坑」第 1 条）；
- `__COMPAT_LAYER=Installer`。

处置办法：用**干净的进程环境**启动 App（去掉被写坏的 `PATH` 首项、代理变量、`__COMPAT_LAYER`），
并用 `curl --noproxy '*'` 验证端点（带代理的 `curl` 会给出假阳性，不可信）；
环境无法修复时，把 CDP 类验收**写好脚本交质量方实跑**，并在回报里标注
「产品端到端待质量方执行」——**不要写成「机器不可用」**。

### 当时的取证与误判路径（保留，作为环境踩坑记录）

1. **新建脚本**：`scripts/e2e/tauri-cdp-single-file-import.mjs` 报
   `CANNOT-RUN WebView2 DevTools 端点未在 90000ms 内就绪`。
2. **仓库自带脚本**：`scripts/e2e/tauri-cdp-smoke.mjs` 报**同一句话**
   → 排除了「是我新写的脚本有问题」。
3. **直接启动 exe（`env -i` 干净环境）**：
   - `DevToolsActivePort` 文件**从未生成**（`--remote-debugging-port=0` 也不生成）；
   - `netstat` 里没有任何 `639xx` 监听；
   - `curl --noproxy '*' http://127.0.0.1:<port>/json/version` 返回 **exit 7**（连接被拒）。
4. **对照实验（证明机器本身没封远程调试）**：
   `msedge.exe --headless=new --remote-debugging-port=63980` **3 秒内**就绪，
   `/json/version` 返回 `{"Browser":"Edg/153.0.4234.48", …}`。
   → 操作系统/安全策略**没有**封禁 remote debugging。
   ⚠️ 当时由此推断「问题出在 WebView2/App 侧」是**错的**：真正原因是**启动 App 的那个进程环境**
   被污染（见上「真实原因」），而不是 WebView2 或 App 本身。

### 逐一排除的「非原因」

- 宿主代理与 `__COMPAT_LAYER`（只导致 DB 写失败 `library_v2_migrate_begin:attempt to write a
  readonly database`，不影响端口）；
- `tasklist /FI` 计数假象（`MSYS_NO_PATHCONV=1` 后确认计数真实）；
- `Page.reload` 打断 CDP 会话（那是另一个坑，见下）；
- 环境变量白名单/黑名单、`stdio` 配置、窗口可见性、`--no-sandbox --disable-gpu`；
- `wry`/`tauri` 的 `additional_browser_args` 接线（读源码 294–327 行确认：有值就**只**用该值，
  而我们设的值正是它）；
- `WEBVIEW2_USER_DATA_FOLDER`（`webview/EBWebView/` 确实被创建，含
  `Last Version = 153.0.4234.48`）。

### 历史事实（说明不是「一直如此」）

`artifacts/e2e-cdp/run-cloud-repair-chain-2026-09-21T18-51-21-267Z` 的 CDP 链**真的跑通过**：
`verdict=passed`、`exitCode=0`、`scenarioFacts.total=13`、`identity.buildFresh.mode=manifest`、
   commit `67f271b`。同一个 exe 在 2026-09-22 11:49 那次也能起来。
   ⇒ 该次失败是**当时的进程环境**造成的，不是机器状态退化，也不是「一直如此」。

### 配套的坑（本轮踩到，记下来备用）

1. **`curl` 被宿主 HTTP 代理劫持造成假阳性**：`HTTP_PROXY=http://127.0.0.1:49812` 会让 curl
   把对 127.0.0.1 的请求也发给代理，代理返回错误页被当成响应体，
   **`upstream connect failed: … (os error 10061)` 却仍 exit 0**。
   所有本地探测必须 `curl --noproxy '*'`（此后 `exit 7` / `http_code=000` 才可信）。
2. **`tasklist /FI` 在 Git Bash 下被路径转换破坏**：过滤器静默返回空。需 `export MSYS_NO_PATHCONV=1`。
3. **`Page.reload` 会打断 CDP 会话**：重载后 WebView2 的 page target 重建，
   `Runtime.evaluate` 一律报「CDP 连接已关闭」。`tauri-cdp-issue-list.mjs:253` 早有记录。
   新增的 T5-5 脚本曾误抄 `Page.reload`，已删。
4. **`reg.exe` 被宿主安全策略封锁**（Program Blacklist），不能用来查 Edge 策略。
5. **误建的 `%SystemDrive%/` 目录**：某次 `env -i` 实验里 `SystemRoot="C:\\windows"` 被解释成
   字面路径 `%SystemDrive%/ProgramData/...`，在仓库根建出 6 个 Windows 缓存文件。已清理。

### 对任务的影响（已随结论更正）

- **T5-4 / T5-5**：当时 CDP 链未取得，核心断言改到**命令处理器层**并做了两次独立先红
  （`job_commands.rs:608`、`job_commands.rs:651`）——这部分工作有效、保留。
  但「降级为待环境恢复执行」的定性要改：CDP 通道在质量方环境里可用，
  脚本与命令处理器层证据齐备后应**交质量方实跑**，而不是等机器恢复。
- **T6**：核心价值（弹窗、拖拽、asset 协议播放）依赖真实 App，当时未取得。
  它**不是被机器阻塞**——见 2026-09-23 返工任务 R6。
- **质量把关方**：复核 §6「重建 App 跑 `tauri-cdp-cloud-repair-chain.mjs` 必须保持 13/13」
  这一条，在质量方环境里**已经跑通（13/13）**；当时判断「在当前机器上同样会失败」是错的。

### 2026-09-23 返工实测：R4 已通过（含一条必须标清的口径）

修好环境后，`scripts/e2e/tauri-cdp-single-file-import.mjs` **6/6 通过**
（`verdict=passed`，run dir `artifacts/e2e-cdp/run-single-file-import-2026-09-23T18-19-48-637Z`，
exe `5603565241050210…`，commit `816bdd3`）。

**但这条证据有一个必须写明的口径**：该次运行带 `--no-sandbox --disable-gpu`，
即 `runProfile=cdp-diagnostic`、`report.diagnosticRun=true`。原因是**本仓库脚本自己早就写明的约束**：

- `scripts/e2e/tauri-cdp-smoke.mjs:33-40` 注释原文：「本沙箱环境下 WebView2 的 renderer
  **在不加这两个开关时会中途崩溃（`CDP 连接已关闭`）**」——所以它的**默认值**就是这两个开关，
  并明确要求「以此运行得到的结论必须标注『诊断参数运行』」。
- 我用受控探测复现了这一点（`tmp/probe-app-exit.mjs`，直接 spawn exe，不建任何 CDP WebSocket）：
  不加这两个开关时，App 进程**一直活着**（50s 内无退出），页面也已导航到
  `http://tauri.localhost/#/library`；但 WebView2 的 DevTools HTTP 端点会在启动后
  **约 7.4 秒**彻底消失（`fetch /json/list` 从此一直 `fetch failed`），且**永不回来**。
  带 / 不带 `PDF2TEST_AUTOMATION_SOURCE_FILES` 两次对照**逐行同形** —— 排除该变量是诱因。
- 因此 `tauri-cdp-single-file-import.mjs` 的**默认档（`cdp-default`，不带诊断参数）在本机跑不通**。
  这**不是产品缺陷、也不是本轮改动引入**，而是本沙箱环境的既有约束；
  诊断档 6/6 是**可复现的真实 App 产品链**证据，但必须标注为**诊断参数运行**，
  不得写成「默认产品路径通过」。

**结论**：R4 的验收判据（真实 App 里 6/6）在诊断档下达成；默认档的阻塞已定位到
「WebView2 renderer 需要 `--no-sandbox --disable-gpu`」这一条环境约束，与产品无关。
另外，`launchTauriAppCdp` 现在会在 CDP 会话断开时**自动重新附着 page target**，
把「页面 90000ms 内未渲染出可见文本」这种误导性报错收敛成精确原因；它**不**会把
上面这种 renderer 崩溃伪装成成功（此时 `attachToPageTarget` 会因端点消失而如实失败）。
