# A9 — 第 13/14 章对抗审计（发布流程简化 / 设置页简化）

- 审计日期：2026-09-12
- 审计对象：`Plan With Files/IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md` 第 13 章（2234–2338）与第 14 章（2341–2392）
- 代码基线：`HEAD = 47a3806` + 未提交工作区改动（`src-tauri/src/nas_package_v2.rs`、`src/api/publishClient.ts`、`src-tauri/src/product_chain.rs` 等均为 modified）
- 方法：只读。Grep/Read/Glob + 只读 `git status`。未运行 `cargo build/test`、`npm run build`，未对任何 NAS 目录发起写操作。
- 对抗立场：默认每条论断为错/过时/未完成，以当前代码证伪。

## 章节范围

| 章节 | 论断摘要 | 关键代码锚点 |
|---|---|---|
| §13.1 | 题库多选「发布已选择」+ 工作区「发布」主按钮；删除独立 `/export` 主导航 | `src/app/router.ts`、`src/features/library/LibraryBatchBar.tsx`、`src/features/editor/ExamWorkspacePage.tsx`、`src/components/AppShell.tsx` |
| §13.2 | `PublishLibraryItemsInput { item_ids, destination_id, overwrite_policy }`；NAS 目录设置一次记住 | `src-tauri/src/nas_package_v2.rs`、`src/features/settings/appSettings.ts` |
| §13.3 | 保留 9 项业务闭包；移除/降级 5 项历史门禁 | `src-tauri/src/authoring_v2_commands.rs` |
| §13.4 | typed `PublishCheckResultV1` / `PublishBlocker`；前端不再解析长字符串 | `authoring_v2_commands.rs`、`src/api/publishClient.ts` |
| §13.5 | 原子发布保留但隐藏；staging→…→manifest last→atomic rename→failure cleanup | `src-tauri/src/nas_package_v2.rs` |
| §13.6 | 批量 publisher：一次 stage 全部→更新 manifest→probe→commit；默认不提交整批；「仅发布通过项」 | `nas_package_v2.rs`、`src-tauri/src/product_chain.rs` |
| §14.1 | 默认视图 6 字段 + 文件 + 高级展开 | `src/features/settings/SettingsPage.tsx`、`src/pages/Settings.tsx` |
| §14.2 | 高级 6 项；不再显示 forceJson/profile enabled/temperature/不受支持 provider | `SettingsPage.tsx`、`llm_commands.rs`、`llm_gateway.rs` |
| §14.3 | 单 active profile；多 Profile 仅开发者模式 | `SettingsPage.tsx`、`src/pages/Settings.tsx`、`src/app/legacyRoutes.tsx` |
| §14.4 | 不 700ms 自动保存；保存并测试/离开保存；测试成功设 active；API Key 安全存储 | `SettingsPage.tsx`、`appSettings.ts`、`llm_commands.rs` |

## 断言核对表

判定：`HOLDS` / `PARTIAL` / `FALSE` / `UNVERIFIABLE`。证据层级：`product`（产品链路）/`command`（命令/后端）/`static`（静态代码）/`doc-only`。

| # | 章节 | 断言 | 判定 | 证据（绝对路径:行号） | 层级 |
|---|---|---|---|---|---|
| 1 | §13.1 | 题库多选发布入口存在 | HOLDS（文案不符） | `src/features/library/LibraryBatchBar.tsx:21`（按钮文案为「发布到 NAS」，非「发布已选择」）；`src/features/library/LibraryPage.tsx:161-167` | product |
| 2 | §13.1 | 工作区「发布」主按钮 | HOLDS | `src/features/editor/ExamWorkspacePage.tsx:142-144`、`:86-102` | product |
| 3 | §13.1 | 删除独立 `/export` 主导航 | PARTIAL | 主导航仅「题库/设置」`src/components/AppShell.tsx:10-11`；但 `/export` 仍在 `LEGACY_PAGES` 与重定向中 `src/app/router.ts:18,58`；旧页 `src/pages/WritingStudio.tsx:142`、`src/pages/UnifiedPreview.tsx:920` 仍 `go("/export")` | static |
| 4 | §13.2 | 输入结构 `{item_ids,destination_id,overwrite_policy}` | FALSE | 实际 `PublishItemsInput { item_ids, destination, fault }` `src-tauri/src/nas_package_v2.rs:253-259`；无 `destination_id`/`overwrite_policy` | command |
| 5 | §13.2 | NAS 目录设置一次后记住 | PARTIAL | 功能上成立：`LibraryPage.tsx:74-81`、`ExamWorkspacePage.tsx:89-98`、`SettingsPage.tsx:180-185`；但仅落 `localStorage` `src/features/settings/appSettings.ts:40-73`，非后端设置 | product |
| 6 | §13.3 | 保留「Canonical DS 可反序列化」等 9 项 | PARTIAL | `check_publish_preflight` 仅查 schema/quality/hardFailures/unresolved `authoring_v2_commands.rs:352-456`；题干非空、option 完整、answer key 由 `validate_authoring`+quality 折入；引用资源/编译/目录可写/原子完成在 export/publish 阶段（`:662`、`nas_package_v2.rs:491-517`） | command |
| 7 | §13.3 | 移除「所有题组手工逐个 Confirmed」 | HOLDS（主线） | 新 typed 路径无 humanVerified 要求；旧函数仍要求 `audit.humanVerified==true` `authoring_v2_commands.rs:268-274`（仅 legacy/死路径） | command |
| 8 | §13.3 | 移除「历史 fallback 永久禁止发布」 | HOLDS（主线） | 旧扫描仍在 `authoring_v2_commands.rs:300-330`（`collect_publish_gate_markers`）；新 typed 路径不扫描 | command |
| 9 | §13.3 | 移除「保留完整 SourceReviewV1」 | HOLDS（主线） | 旧要求 `authoring_v2_commands.rs:276-298`；新路径无 | command |
| 10 | §13.3 | 不向用户显示 schema/hash/CAS/manifest 细节 | PARTIAL | `publishClient.ts:18-35` 映射常见错误，但未匹配时 `return raw`（`:34`），仍可能回显 `nas_package_v2_*` 机器码/路径 | static |
| 11 | §13.3 | 发布成功后可回收 DS 引用之外的资源 | FALSE | 全仓无 releases/资源 GC；`nas_package_v2.rs:1195-1198` 仅清 staging+backup；task_plan M6 自认「引用集合清理未做」 | static |
| 12 | §13.4 | 存在 typed `PublishCheckResultV1`/`PublishBlocker`/`PublishFixAction` | FALSE | 仅有 JSON 字符串 `"schemaVersion":"PublishCheckResultV1"` `authoring_v2_commands.rs:448-455`；无 Rust struct、无 `PublishBlocker`、无 `PublishFixAction` 枚举（全仓 grep 无命中） | command |
| 13 | §13.4 | 前端不再解析长字符串 | FALSE | 仍解析 `publish_check_failed:` JSON `src/api/publishClient.ts:20-29`；仍判断 `authoring_v2_export_blocked` `:30` | static |
| 14 | §13.4 | 提供 typed 结果给前端 | FALSE | 后端把结果包成错误字符串 `publish_check_failed:{json}` `authoring_v2_commands.rs:653-656`；无独立 preflight 命令暴露（`lib.rs` 仅 `publish_items`） | command |
| 15 | §13.5 | staging→asset copy→compile→manifest last→atomic commit→cleanup | PARTIAL | 单条路径成立（`stage_and_commit` `nas_package_v2.rs:582-692`，manifest 最后 `atomic_replace_file` `:953`，回滚 `:838-967`）；批量路径见 #18/#19 | command |
| 16 | §13.5 | 用户只看到「正在发布 3/12」 | PARTIAL | UI 文案存在 `LibraryPage.tsx:92,95`；但 `publishItems` 只在开始/结束各回调一次 `publishClient.ts:42,45`，命令阻塞无中间进度 | product |
| 17 | §13.6 | 批量 `publish_batch` 一次 stage 全部→manifest→probe→commit | PARTIAL | `publish_items_core` `nas_package_v2.rs:261-352`：全部 stage 到私有 `.batch-staging-{id}`（`:287-323`），组装 manifest（`:325-330`），CAS（`:331`），rename 到 `releases/{id}`（`:333`），原子替换 manifest（`:336`）；**无 batch 级 probe** | command |
| 18 | §13.6 | 任一题失败默认不提交整批 | HOLDS | 任一 `?` 失败即 `Err`，清理 staging/release `nas_package_v2.rs:339-341`；测试 `product_chain.rs:1093-1108` | command |
| 19 | §13.6 | 「整批冻结快照 + 单次原子清单替换 + 全量回滚」覆盖「部分题可见」窗口 | PARTIAL | 冻结快照成立（`:266-272`）；清单原子替换使 manifest 可见性原子（release 文件先落地但无引用）；但**无 journal → 崩溃不恢复**、**releases 永不回收**、测试仅注入 `after_item_1` 未覆盖 rename 后窗口 | command |
| 20 | §13.6 | 返回每题具体问题 | FALSE | 后端整体 `Err`，错误串不含 `item_id`；前端把全部项标同一文案 `publishClient.ts:47-51` | command |
| 21 | §13.6 | 「仅发布通过项」二次动作 | FALSE | 无实现；UI 仅显示成功/失败数 `LibraryPage.tsx:97-101` | static |
| 22 | §13.6 | `probe_stage` 存在 | FALSE | 无 `probe_stage` 符号；probe 内联于 `stage_package_files` `nas_package_v2.rs:545-551`（逐题） | command |
| 23 | §13（F12） | `canonical_ds` 不读 shadow 绑定 | HOLDS（但对产品已非主路径） | `nas_package_v2.rs:154-189` 仅非 canonical 才比 shadow；batch 路径根本不调用 `validate_v2_export_binding` | command |
| 24 | §13（F12） | 发布期间编辑使绑定失效的竞态 | HOLDS（已消除） | batch 用冻结 DS（`export_authoring_snapshot` `db_direct=true` `authoring_v2_commands.rs:611-618`），状态更新按版本 CAS `nas_package_v2.rs:345` | command |
| 25 | §14.1 | 默认 6 字段 | PARTIAL | 云端识别/服务地址/模型/API Key/保存并测试/文件保留/高级展开均在 `SettingsPage.tsx:209-309`；按钮为「保存并测试」而非「测试连接」；额外多出「发布/NAS 目录」组 `:288-299` | product |
| 26 | §14.2 | 高级 6 项 | PARTIAL | 超时/本地并发/云端并发/过程文件/环境诊断有（`:327-404`）；「Ollama 模式」无独立开关，改为「协议」下拉（`:313-325`）；额外有「开发者模式」 | product |
| 27 | §14.2 | forceJson 永远开启 | PARTIAL | 新 UI 恒传 true `SettingsPage.tsx:141`；但后端 `llm_force_json` 仍尊重 `false` `llm_gateway.rs:131-133` | command |
| 28 | §14.2 | profile enabled 由全局开关表达 | HOLDS | `SettingsPage.tsx:214-222`→`saveLlmProfile enabled`；导入读 `profile.enabled` `useImportFiles.ts:25-28` | product |
| 29 | §14.2 | temperature 固定 0/极低 | PARTIAL | 新 UI 恒传 0 `SettingsPage.tsx:139`；后端 `llm_temperature` 原样读取 `llm_gateway.rs:114-119`，无固定 | command |
| 30 | §14.2 | 不受支持 provider 删除 | PARTIAL | UI 只列 2 种 `SettingsPage.tsx:29-32`；后端写入仍接受 `AnthropicCompatible`/`Custom` `llm_commands.rs:78-91`，而 gateway 拒绝它们 `llm_gateway.rs:146-151` | command |
| 31 | §14.3 | 单 active profile | PARTIAL | 新页只维护一个表单 `SettingsPage.tsx:73-76`；旧多 Profile 页 `src/pages/Settings.tsx:249-269` 仍在仓库（孤儿，经 `legacyRoutes.tsx` 引用，不进 bundle） | static |
| 32 | §14.4 | 不 700ms 自动保存 | HOLDS（新页） | 新页仅本地编辑、显式保存 `SettingsPage.tsx:118-122,124-154`；旧页仍 700ms 自动保存 `src/pages/Settings.tsx:145-180`（孤儿） | static |
| 33 | §14.4 | 离开页面时保存 | FALSE | `SettingsPage.tsx` 无 unmount/离开 flush；仅「保存并测试」触发持久化 | static |
| 34 | §14.4 | 测试成功后设为 active | FALSE | 先 `saveLlmProfile({enabled: cloudEnabled})` 再 `testLlmProfile` `SettingsPage.tsx:130-146`；测试失败仍已保存（可能 enabled） | product |
| 35 | §14.4 | API Key 安全存储且不展示 backend 文案 | HOLDS | `save_profile_secret` 走 OS/文件兜底 `llm_commands.rs:96-112`；新页无 backend 文案（旧页 `secretBackendLabel` `src/pages/Settings.tsx:55-60` 为孤儿） | command |
| 36 | §14.4/§14.2 | 设置真正生效到 Rust 调度器 | FALSE | 无 `get_app_settings`/`save_app_settings` 命令（全仓 grep 无命中）；`localConcurrency/cloudConcurrency/keepSourceFiles` 仅 `localStorage` `appSettings.ts:15-32,59-73`；调度器用 CPU 推导默认值 `src-tauri/src/processing/scheduler.rs:42-51`（注释自认「M2 从设置读」） | command |

## 原子性与设置生效链路核查

### 1. 批量发布原子性：可见性原子成立，但恢复/回收不成立

`publish_items_core` 的真实顺序（`nas_package_v2.rs:261-352`）：

1. 一个 DB 事务内对每个 item `get_canonical_ds` 取 `(ds, version)`，提交事务（`:266-272`）——**冻结快照成立**，不依赖 mutable shadow。
2. 全程持有 `library_root/.publish-control/export.lock` 排他锁（`:279-281`，锁路径与单条路径共用 `make_paths` `:1004`），并先 `recover_incomplete_transactions`（`:282`）。
3. 逐题 `export_authoring_snapshot`（db_direct）→ `stage_package_files`（资产拷贝 + **逐题 probe** `:545-551`）到私有 `.batch-staging-{batch_id}`；条目路径改写为 `./releases/{batch_id}/...`（`:314-317`）。
4. 组装 manifest → 写入 `staging/manifest.js` → CAS 校验 → `fs::rename(staging, release)` → `atomic_replace_file(release/manifest.js, reading_root/manifest.js)`（`:325-336`）。
5. 失败时 `remove_dir_all(staging)` + `remove_dir_all(release)`（`:339-341`）。

结论：
- **「部分题在 NAS 可见」的清单窗口被消除**：学生在 manifest 指向 releases 之前看不到任何新条目；release 文件虽先落地但无引用，无害。
- **但失败清理仅覆盖进程内错误**。批量路径**不写 journal**，而 `recover_incomplete_transactions` 只识别 `*.journal.json`（`:1384-1497`）。若在 `rename` 之后、`atomic_replace_file` 之前进程崩溃/断电，`releases/{batch_id}` 与 `.batch-staging-*` 将永久残留，无任何恢复或回收路径。
- **`releases/*` 永不回收**：每次成功批量都新建一个 release 目录（`:288,333`），且该目录内**同时**含 `snapshots/{examId}-r0-{uuid}/`（`export_authoring_snapshot` 产出的 authoring+source+资产副本）与 `resources/{examId}/`（`stage_package_files` 再拷一份资产）——同一批资产被复制两次。重复发布同一批题目会线性累积，50 题含图场景磁盘成本被明显低估。
- **批量提交未复用单条的备份/journal/CAS 恢复机制**：单条 `commit_package` 有旧包备份、journal 状态机、`recover_post_commit_metadata_failure`（`:708-981`、`:699-706`）；批量是另起一套更简化的 rename+manifest 替换。§13.5「复用 nas_package_v2 的 staging 和最终提交」只复用了 staging 与 probe，未复用「最终提交」的持久化保证。

### 2. F12 现状：产品路径已不读 shadow，但「修复点」在死路径

- 单条 `validate_v2_export_binding` 的 shadow 比较已被 `authoringSource=="canonical_ds"` 跳过（`nas_package_v2.rs:154-189`），且导出侧 `revision=0`/`authoringSource=canonical_ds`（`authoring_v2_commands.rs:614-618,718-720`）。
- 但当前产品 UI **只调用 `publish_items`**（`publishClient.ts:44`）；`exportAuthoringV2`/`exportNasPackageV2` 仅被孤儿页 `ExportPage.tsx`/`StructuredAuthoringEditorV2.tsx` 引用（经孤儿 `legacyRoutes.tsx`，不进 bundle）。批量路径从不调用 `validate_v2_export_binding`。故 F12「发布绑定 shadow」在产品上确已不成立，但**并非由文档所述代码位置修复**，而是批量路径天然不读 shadow。
- 「发布期间编辑使绑定失效」竞态：批量用冻结 DS 且状态更新带版本 CAS（`nas_package_v2.rs:345`），故已消除。

### 3. 设置生效链路：只有模型连接真的进后端，其余偏好全部悬空

- **真生效**：`云端识别` 开关 → `saveLlmProfile.enabled` → 导入时 `useImportFiles.ts:25-28` 读 `profile.enabled` 决定 `cloudEnabled`；API Key → `save_profile_secret`（OS 安全存储/文件兜底）。
- **不生效**：`localConcurrency`/`cloudConcurrency`/`keepSourceFiles`/`developerMode`/`nasDestination` 仅写 `localStorage`（`appSettings.ts:40-73`），**无任何后端读写命令**（全仓无 `get_app_settings`/`save_app_settings`）。Rust 调度器并发度来自 `available_parallelism()` 推导（`scheduler.rs:42-51`），并在源码注释里自认「M2 从设置读」尚未实现。即 §14.2「云端/本地并发数」「过程文件保留」与 §14.4 所暗示的「设置生效」在调度器侧**当前无效**。

## 发现清单

### A9-F01 [P1] 设置项未接入后端，并发/文件保留开关为装饰性控件
- 结论：`localConcurrency`、`cloudConcurrency`、`keepSourceFiles`（以及 `developerMode`）只落 `localStorage`，无对应 Rust 命令；调度器固定用 CPU 推导值。
- 证据：`src/features/settings/appSettings.ts:15-32,40-73`；`src/features/settings/SettingsPage.tsx:340-362`；`src-tauri/src/processing/scheduler.rs:42-51`（`cloud_enabled_default` 也固定 false）；全仓无 `get_app_settings`/`save_app_settings`。
- 影响：用户改并发/保留策略对识别调度零效果，与 §14.2/§14.4 承诺不符；真实 Tauri 验收必然暴露。
- 建议：落 `get_app_settings`/`save_app_settings` 命令 + 启动时注入 `ProcessingSettings`；`keepSourceFiles` 接清理策略。

### A9-F02 [P1] 批量发布无崩溃恢复，releases 目录永不回收且资产重复拷贝
- 结论：批量路径不写 journal；`recover_incomplete_transactions` 仅处理 `*.journal.json`；成功后 `releases/{batch_id}` 无 GC；release 内资产被复制两份（snapshots + resources）。
- 证据：`src-tauri/src/nas_package_v2.rs:261-352`（无 `write_journal`）、`:1384-1497`（仅 journal 恢复）、`:1195-1198`（仅清 staging/backup）、`:287-288,333`（release 生成）、`src-tauri/src/authoring_v2_commands.rs:713`（快照内拷资产）。
- 影响：崩溃后 NAS 残留不可回收；重复发布线性占用磁盘；与 §13.3「回收引用外资源」、M6「引用集合清理」缺口一致。
- 建议：批量写 `NasBatchJournalV1` 并在启动/下次发布时回收；提交成功后清理旧 release 与孤儿 staging；快照目录不落资产或发布后删除。

### A9-F03 [P1] 批量失败不返回每题问题，也无「仅发布通过项」二次动作
- 结论：任一题失败即整体 `Err`，错误串不含 item 身份；前端把全部选中项标为同一条失败文案；无「仅发布通过项」入口。
- 证据：`src-tauri/src/nas_package_v2.rs:289-338`；`src/api/publishClient.ts:47-51`；`src/features/library/LibraryPage.tsx:94-101`。
- 影响：50 题批次中 1 题阻塞时用户无法定位问题题，也无法只发布其余通过项，直接违背 §13.6。
- 建议：失败时返回 `{passed:[],failed:[{itemId,blockers}]}` typed 结构；UI 提供「仅发布通过项」二次确认。

### A9-F04 [P1] §13.4 typed PublishCheckResult 未交付，前端仍解析字符串
- 结论：无 Rust `PublishCheckResultV1`/`PublishBlocker`/`PublishFixAction`；后端把结果塞进 `publish_check_failed:{json}` 错误串；前端仍解析该前缀与 `authoring_v2_export_blocked`；无独立 preflight 命令。
- 证据：`src-tauri/src/authoring_v2_commands.rs:448-455,653-656`；`src/api/publishClient.ts:20-30`；`src-tauri/src/lib.rs:1534`（仅注册 `publish_items`）。
- 影响：§13.4 的「不再解析长字符串」「typed blocker」均未达成，且当前字符串面比计划写作时更宽。
- 建议：定义 typed struct + `code→PublishFixAction` 映射与中文 `user_message` 表；新增 `check_publish` 命令返回结构化结果。

### A9-F05 [P2] provider 写入校验与 gateway 路由枚举不一致
- 结论：`save_llm_profile` 接受 `AnthropicCompatible`/`Custom`，但 `llm_gateway` 仅路由 `OpenAiCompatible`/`Ollama`；新 UI 只列 2 种。
- 证据：`src-tauri/src/llm_commands.rs:78-91`；`src-tauri/src/llm_gateway.rs:146-151`；`src/features/settings/SettingsPage.tsx:29-32`。
- 影响：保存成功但请求必然失败，用户得到「已保存」却无法识别；§14.2「不受支持 provider 删除」只删了 UI。
- 建议：后端白名单收敛为 2 种，或补协议适配器与测试。

### A9-F06 [P2] forceJson / temperature 未在后端固定
- 结论：`llm_force_json` 尊重 `false`，`llm_temperature` 原样读取；固定只发生在新 UI 传参层。
- 证据：`src-tauri/src/llm_gateway.rs:114-119,131-133`；`src/features/settings/SettingsPage.tsx:139-141`。
- 影响：经 API/旧数据导入的 profile 可关闭 JSON 或设高温；§14.2「永远开启/固定 0」未在契约层成立。
- 建议：后端固定或在保存时强制归一并拒绝非法值。

### A9-F07 [P2] §14.4 保存策略缺「离开页面保存」且「测试成功才 active」未实现
- 结论：无 unmount/离开 flush；`saveAndTest` 先保存（含 `enabled`）再测试，测试失败仍已保存并可能启用。
- 证据：`src/features/settings/SettingsPage.tsx:124-154`（无离开钩子）；`appSettings.ts` 无页面级 flush。
- 影响：用户编辑后直接离开会丢失；连接不可用时仍可能被置为 active，导入走云端失败路径。
- 建议：unmount/路由离开时 flush；测试通过后再置 `enabled=true`。

### A9-F08 [P2] §13.5 进度显示为假
- 结论：`onProgress` 仅开始(0)与结束(total)两次，命令为单次阻塞调用。
- 证据：`src/api/publishClient.ts:42,45`；`src/features/library/LibraryPage.tsx:92-96`。
- 影响：「正在发布 3/12」实际不会出现，仅 0→N 跳变；多题长任务无反馈。
- 建议：后端按题发进度事件（`publish://progress`）或分片命令。

### A9-F09 [P2] §13.2 输入结构与实现不符
- 结论：无 `destination_id`/`overwrite_policy`；`destination` 为绝对路径字符串，NAS 目录存 `localStorage`。
- 证据：`src-tauri/src/nas_package_v2.rs:253-259`；`src/features/settings/appSettings.ts:19`。
- 影响：计划中的「目标抽象 + 覆盖策略」未落地，覆盖语义隐式为「整批替换 manifest 同名条目」。
- 建议：明确目标抽象与覆盖策略，或修正计划口径。

### A9-F10 [P3] §13.1 文案与路由清理不彻底
- 结论：按钮为「发布到 NAS」；`/export` 仍在 legacy 列表与重定向中，旧页仍有 `go("/export")`。
- 证据：`LibraryBatchBar.tsx:21`；`router.ts:18,58`；`WritingStudio.tsx:142`、`UnifiedPreview.tsx:920`、`LibraryExamDetail.tsx:110`。
- 影响：导航已收敛，但兼容面与文案与计划不完全一致。
- 建议：随 P10 删除旧页时一并收敛。

### A9-F11 [P2] 批量提交未复用单条原子提交机制
- 结论：批量是独立简化提交，缺备份/journal/CAS 恢复；§13.5「复用 staging 和最终提交」名不符实。
- 证据：`nas_package_v2.rs:325-341`（批量）对比 `:582-692,708-981`（单条）。
- 影响：两套提交语义分叉，故障保证不一致，长期维护与验收口径混乱。
- 建议：抽象统一 commit 原语，批量复用其备份/恢复。

### A9-F12 [P3] 无 batch 级 probe，`probe_stage` 不存在
- 结论：仅逐题 probe（`stage_package_files` 内联），未对组合后的 manifest/release 路径做整体探针。
- 证据：`nas_package_v2.rs:545-551`；全仓无 `probe_stage` 符号。
- 影响：批量 manifest 路径拼接（`./releases/{batch_id}/...`）无整体校验；回归测试 `product_chain.rs:1111-1135` 仅断字符串包含。
- 建议：提交前对 release 根做一次 loader 探针。

### A9-F13 [P3] 旧设置页/旧路由未删除（孤儿）
- 结论：`src/pages/Settings.tsx`（多 Profile/temperature/forceJson/enabled/Anthropic/Custom）与 `src/app/legacyRoutes.tsx` 仍在仓库，仅因不被 `App.tsx` 引用而不进 bundle。
- 证据：`src/app/App.tsx:1-35`（未导入 legacyRoutes）；`src/app/legacyRoutes.tsx:12,79`；`src/pages/Settings.tsx:249-319`。
- 影响：§14.2/§14.3 的「删除」在源码层未发生，tsc 仍编译，存在回归误引用风险；与 task_plan「P10 统一删除」一致但需在 DoD 中如实标注。
- 建议：P10 成组删除并在基线快照中记录。

## 与历史审计的关系

| 历史项 | 本次核对结论 |
|---|---|
| F10 批量发布非原子 | **已修复**：`publish_items_core` 整批冻结 + 单次原子 manifest 替换，可见性原子成立（HOLDS）。但残留 A9-F02（崩溃恢复/GC）、A9-F03（无每题问题/仅发布通过项）。 |
| F12 canonical 绑定 shadow | **产品路径已不成立**：批量路径不读 shadow，冻结 DS + 版本 CAS 消除竞态（HOLDS）。但文档所述修复点 `validate_v2_export_binding` 已不在产品主路径，属死路径修复（A9-F11 相关）。 |
| repair progress 声称「整批冻结快照 + 单次原子清单替换 + 中途失败全量回滚」 | PARTIAL：前两项成立；「全量回滚」仅覆盖进程内错误，不覆盖崩溃；测试仅注入 `after_item_1`（`product_chain.rs:1095`），未覆盖 `before_manifest` 后窗口与崩溃场景。 |
| task_plan M6「引用集合清理与 NAS 实测未做」 | 与 A9-F02/A9-F11 一致，缺口仍在。 |
| 第 24 章 DoD「发布在题库/工作区完成」「普通导航只有题库和设置」 | 成立。 |
| 第 24 章 DoD「普通 UI 不展示技术 hash/schema/manifest」 | PARTIAL：`publishClient.ts:34` 未匹配错误仍回显原始机器码。 |
| 第 24 章 DoD「Real Tauri import/edit/publish E2E 通过」 | 未执行（本环境约束），不可作为当前验收依据。 |

内部矛盾：
1. §13.6「任一题失败默认不提交整批」与 §12.2「批量中单个文件失败不应阻止其他文件建行」分属**发布**与**导入**两个阶段，语义并不冲突，但计划未说明二者边界与「仅发布通过项」如何与原子默认并存；实现层导入容错、发布原子，而「仅发布通过项」缺失（A9-F03）。
2. §13.2「NAS 目录在设置中选择一次，后续记住」暗示真实应用设置，但实现只落 `localStorage` 且无后端命令，与 §14.4 所依赖的「设置生效」链路同源缺失（A9-F01）。
3. §14.2「forceJson 永远开启 / temperature 固定」在 UI 层成立、后端契约层不成立（A9-F06），文档以 UI 行为代替了后端保证。

## 证据层级与局限

- `product`：LibraryPage/ExamWorkspacePage/SettingsPage 的发布与设置交互、导入云开关链路（静态阅读调用链，未运行）。
- `command`：`publish_items_core`/`export_authoring_snapshot`/`check_publish_preflight`/`llm_commands`/`llm_gateway`/`scheduler` 的 Rust 实现。
- `static`：router/AppShell/孤儿页引用关系、`publishClient` 字符串解析、`localStorage` 读写点。
- `doc-only`：计划章节本身。

局限：
- 未运行 `cargo build/test`、`npm run build`、真实 Tauri E2E；未对真实 NAS 做任何写入。所有原子性/恢复结论基于静态代码与现有单元测试 `product_chain.rs:1046-1147` 的阅读，未做运行时故障注入。
- 未逐字节核对 `validate_authoring` 是否等价覆盖「题组/slot 归属、题干非空、option 完整、answer key 完整」四项，仅判定其折入 schema/quality 检查（#6 为 PARTIAL）。
- 「引用集合清理」结论基于全仓 grep 未发现 NAS 侧 GC 实现；不排除存在未在 `src-tauri/src` 命名空间内、或依赖外部学生端仓库的清理逻辑。
