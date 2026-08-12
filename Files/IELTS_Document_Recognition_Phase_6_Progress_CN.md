# IELTS 文档识别重构 Phase 6 进度

## Phase 6 自设目标

依据总任务书第 10、11、16.7、20.5、20.6、24.8 和 24.9 节，在不改写既有 V1 题库与发布链路的前提下，完成 authoring studio → NAS student 的 Reading V2 runtime vertical slice：

1. 固化 `ReadingExamSourceV2` contract，并让 Phase 5 产出的 V2 source 能被严格校验。
2. 建立完全以 `slotId` 为状态键的 interaction model、attempt 校验、恢复边界和评分 API；共享多选必须按 response group 的 cardinality、重复选择策略和 scoring policy 工作。
3. 建立 `ExamAssetManifestV2` 的路径安全、hash/size/MIME 校验和 runtime asset closure 检查，作为 NAS staging/student probe 的共同底层。
4. 增加 V1/V2 双读路由的框架无关契约：V1 保持 legacy payload，不经过 V2 结构推断；V2 只读取 task、response group 和 answer slot。
5. 完成 NAS V2 package staging、manifest-last 两阶段提交、锁/journal/report、资源 API、V2 学生端节点渲染和生产答题/评分接线。
6. 为上述边界提供跨仓真实 fixture、负向 probe、Rust/TypeScript/Vue 单元与集成验证脚本。

## 依赖与边界

- Phase 5 当前工作区已经提供 immutable V2 revision、`reading-source-v2.json` shadow bundle 和学生同源预览输入，本阶段以它们作为只读输入。
- NAS student repository `E:\NAS` 已作为同机 peer workspace 接入；其既有 Phase 5 合同修改保持不变，本阶段只做增量接线。
- V1 `manifest.js + <examId>.js`、V1 artifact、V1 sanitizer/render path 和现有导出命令不被改写。V2 通过独立 manifest entry、资源 API 和结构化 renderer 上线。
- `runtimeSourceV2`、`nasPackageV2` 默认保持关闭；所有新路径必须显式启用。

## 验收标准

- Phase 5 synthetic authoring fixture 可编译为 `ReadingExamSourceV2` 并通过 JSON Schema 与语义校验。
- attempt 只以 `slotId` 保存答案；未知 slot、错误 interaction kind、越界 cardinality、重复选项、缺失提交答案均 fail closed。
- shared unordered-set 多选支持全对、部分得分、重复/额外选项拒绝，并且 scoring 不从 DOM/name 推断。
- asset manifest 拒绝绝对路径、`..`、URL/UNC/Windows drive escape、符号链接逃逸、hash/size/MIME 不一致；缺失或多余 closure 均阻止 probe。
- NAS 发布具备 exclusive lock、manifest CAS、staging、manifest-last、fault injection rollback、journal/report；缺失资源、hash、manifest、runtime version incompatibility 均阻止加载/提交。
- 负向 probe 不改变 V1 fixture；Phase 5 editor、TypeScript/Vue 检查、Rust 基线和 NAS 现有端到端基线继续通过。

## 当前交付

- [completed] ReadingExamSourceV2 schema 与 contract manifest。
- [completed] framework-neutral slot interaction/attempt/scoring API。
- [completed] ExamAssetManifestV2 path/hash/closure validator 与 Rust tests。
- [completed] NAS V2 package staging、manifest-last commit、锁 owner/heartbeat、提交前二次 CAS、持久 backup/journal recovery/report 与 rollback fault probe。
- [completed] NAS server V1/V2 dual-read、三项 V2 checksum 必填 fail-closed、runtime/asset integrity/version checks、受控资源 API。
- [completed] student Vue V2 structured renderer（无 `v-html`）、slotId attempt wiring、shared response UI 与 V2 submit/scoring。
- [completed] cross-repo fixture/negative probe/schema mirror；验证命令：`npm run verify:phase6:reading-v2`、`npm run verify:authoring-schema-contract`、`npm run verify:cross-repo-reading-v2`。

## 2026-08-12 基线审计补充

- [completed] 修复空文本数组被当作已作答的问题；`{ kind: "text", values: [] }` 在提交阶段现在以 `RUNTIME_ANSWER_REQUIRED` fail closed。
- [completed] 修复跨 exam/revision 的恢复 attempt 可在提交前被修改的问题；set/clear 均在写入前校验 schema、`examId` 和 source revision。
- [completed] `verify:phase6:runtime` 现直接执行 TypeScript runtime API 的负向回归，不再只检查源码 token；新增 attempt resume boundary 与 empty-text submit 用例。

## 2026-08-12 并行变化后复审（已被 2026-08-13 基线修复取代）

- NAS `npm run verify:phase6:reading-v2` 当前通过，生产 server loader、学生端构建、slot scoring、资源 realpath 边界及负向 fixture 为绿色。
- 本段记录并行改动后的原始风险快照；作者端模块当时尚未注册，不能作为当前状态使用。

## 2026-08-13 Phase 6 authoring baseline restored

- [completed] 在不回滚并行 agent 改动的前提下，将 `reading_runtime_v2` 与 `nas_package_v2` 注册到 `src-tauri/src/lib.rs` 的 crate test graph。
- [completed] 新增 `npm run verify:phase6:runtime`，先执行 TypeScript 检查，再强制运行作者端两个模块的 Rust 测试；验证器报告不再接受 0-test 假绿。
- [completed] 实测 `reading_runtime_v2` 4 tests passed，`nas_package_v2` 8 tests passed，覆盖 asset probe、manifest CAS、独占锁、journal、backup/recovery、manifest-last 和 fault rollback。
- [completed] Phase 6 依赖门恢复为可依赖状态；NAS peer 的 `verify:phase6:reading-v2` 与 cross-repo/schema gates 继续保持绿色。

## 2026-08-13 Phase 6 生产发布闭环

- [completed] `publish_nas_package_v2` 已作为真实 Tauri command 注册到 `generate_handler!`，authoring API 暴露 `exportNasPackageV2(input)`。
- [completed] `ExportPage` 与 `StructuredAuthoringEditorV2` 在 `VITE_IELTS_NAS_PACKAGE_V2=1` 时走 V2 两阶段发布，保留 V1 默认路径，并展示 probe、asset count、manifest/report 结果。
- [completed] NAS `ExamReadingService` 对学生 HTTP payload 脱敏：服务端内部 facade 仍保留 answer key 用于计分，但响应中的 `answerKey` 与 `runtimeSourceV2.answerKey` 均被移除。
- [completed] `verify:phase6:runtime` 增加 Tauri/API/UI/HTTP 脱敏静态接线检查，避免仅 Rust 单测通过却遗漏生产入口。
- [completed] `export_authoring_v2` 在提交 authoring receipt 前按 descriptor 安全物化 assets 到 staging，校验真实文件的 size/SHA-256，并将资产摘要写入 manifest；非空图表/图片题不再因 `assetRoot` 缺文件而在 NAS 发布阶段失败。
