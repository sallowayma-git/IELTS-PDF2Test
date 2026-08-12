# IELTS 文档识别重构 Phase 7 目标（Listening V1）

## Phase 7 自设目标

依据总任务书第 8、12、16.8、20.7 和 24.12 节，在 Phase 6 的版本化 source、slot-based attempt、资产完整性、NAS staging/probe 与双读机制之上，交付一个不依赖 HTML 反推、可离线运行且 fail closed 的 Listening V1 vertical slice，并逐步扩展至官方题型范围。

1. 固化 `ListeningAuthoringSourceV1`、`ListeningExamSourceV1`、audio manifest、cue、playback policy 和 attempt contract；完整卷与 partial practice 必须有显式 scope。
2. 首个 vertical slice 只覆盖一个 Part 的 form completion，但必须贯通 authoring → package → student provider/controller → slot-based attempt/submit，并携带题面、音频、cue 和资源 provenance。
3. 建立本地 audio probe：decode、codec、duration、hash、preload、近静音/严重削波和 cue 边界失败均产生稳定 issue code；阻断项在开始考试前 fail closed。
4. practice/mock 播放策略成为 source contract，明确 seek、replay、pause、refresh 和 crash recovery 行为；恢复流程不得绕过 mock 限制。
5. 后续按 fixture 驱动扩展 note/table、MCQ、matching、map/diagram hotspot 和 shared short-answer；hotspot 必须提供等价键盘列表。
6. 完整卷校验四 Part、每 Part 10 题和共 40 个 scoring slot；partial practice 只校验其声明范围，不误报完整卷不变量。
7. 复用 Phase 6 的 NAS lock/CAS/journal/manifest-last/asset closure/student probe，不另建不兼容发布协议；Listening feature flag 默认关闭，V1 Reading 路径保持不变。

## Phase 7 启动门

- Phase 6 `ReadingExamSourceV2` contract、slot attempt API、asset manifest 和 NAS V2 transaction/probe 已形成可重复执行的绿色基线。
- 明确学生端支持的单一 playback codec 与最低 runtime version。
- 提供至少一个可授权进入仓库的短音频 fixture，以及官方样例的结构化标注（不提交受限原始内容）。
- 明确 mock/practice 对 seek、replay、pause、refresh 的产品策略。

## 里程碑与验收

- [completed] L1 Schema-only：Listening source/audio/cue/policy/attempt JSON Schema、Rust/TS 镜像和跨仓 hash 校验。
- [completed] L2 Audio probe：有效音频通过；decode/hash/codec/near-silent/clipping/cue 越界的负向 fixture 使用稳定 issue code。
- [completed] L3 One-Part vertical slice：form completion 已通过结构化 NAS package/provider 加载真实 WAV，按 slotId 作答、恢复、提交与评分；缺音频、hash/size 不一致或 probe 失败均不能开始。
- [completed] L4 Playback policy：practice/mock 的控制、刷新与崩溃恢复测试通过，状态机不能通过重载绕过限制。
- [completed] L5 Official families：note/table/MCQ/matching/map/diagram/shared short-answer fixture 通过；map/diagram hotspot 同时暴露可键盘选择的 option list。
- [completed] L6 Scope/package：完整卷 4×10/40 slot 与 partial practice 分流通过；题面、音频、cue、asset 一起进入与 Phase 6 相同的 manifest-last package 协议，并通过 student provider、real-path、checksum、corruption fail-closed probe。

## 当前决策

- Phase 7 不从 transcript 或 passage 猜答案，也不启用 PDF 逐题 LLM repair。
- 音频是计时与作答契约的一部分，不只是普通附件；controller 状态必须可序列化并与 attempt revision 绑定。
- 第一条开发切片坚持 one Part + form completion，先证明 contract/provider/controller/attempt 闭环，再增加题型。
- Phase 6 基线若出现 source revision、asset closure、transaction 或 student probe 回归，Phase 7 只继续 schema/audio probe 等解耦工作，不进入 NAS/student 集成。

## 2026-08-12 L1 交付

- 将 `IeltsAuthoringIRV2.listening` 从开放的 `sections: unknown[]` 占位符替换为严格的 scope、media、Part、cue、playback policy 和 transcript 结构。
- 新增 `ListeningExamSourceV1` 与 `ListeningAttemptV1` JSON Schema、Rust/TypeScript 镜像及 one-Part form completion fixture。
- 语义门区分 `complete_exam` 与 `partial_practice`；完整卷才强制 4 Part/40 scoring slots，partial fixture 不产生完整卷误报。
- cue 必须位于音频时长内、单调不重叠、置信度至少 0.9 且已确认；source/attempt 与 audio asset、examId、revision 和 playback policy 绑定。
- contract bundle 已同步至 `E:\NAS\developer\contracts\authoring` 并通过 byte-for-byte hash 校验。
- 验证命令：`npm run verify:phase7:listening-contract`、`npm run verify:phase1:schema`、NAS `npm run verify:authoring-schema-contract`。

## 2026-08-12 L2 交付

- 新增纯 Rust `Symphonia 0.6` 本地音频 probe，编译启用 WAV/PCM、MP3 与 AAC-in-ISO-MP4 解码能力，不依赖 ffmpeg 或云端服务。
- probe 流式计算 SHA-256，并完整解码生成 codec、duration、channels、sample rate、RMS、peak 和 clipped-sample ratio；结果由严格的 `ListeningAudioProbeResultV1` JSON Schema、Rust 和 TypeScript 三端镜像约束。
- 真实生成的 1 秒 PCM16 WAV fixture 验证 decode 与 duration；静音、全幅削波、损坏音频、hash mismatch 和不支持扩展名均 fail closed，issue code 去重且稳定。
- MP3/AAC 解码器注册与 MP3/M4A/WAV MIME/container policy 有单测覆盖；当前仓库内可授权的实际二进制回归样本为运行时生成的 WAV，未把受限 IELTS 音频提交进仓库。
- `passed` probe 必须拥有完整媒体事实、非空 64 位 hash、非零字节数且 `issueCodes=[]`；矛盾结果会被 schema 负向测试拒绝。
- `npm run verify:phase7:listening-contract` 通过 7 个 Rust 测试及 TypeScript 检查；contract bundle 已再次同步至 `E:\NAS\developer\contracts\authoring`，两仓 hash gate 均为绿色。

## 2026-08-12 Phase 6 依赖复审（已被 2026-08-13 基线恢复取代）

- NAS 端 Reading V2 生产 loader/student vertical slice 为绿色。
- 本段保留当时的阻塞快照；它不代表当前门禁状态。

## 2026-08-13 Phase 6 gate restored

- [completed] 作者端 `reading_runtime_v2`（4 tests）与 `nas_package_v2`（8 tests）已注册并实跑通过；`verify:phase6:runtime` 会明确拒绝 0-test 假绿。
- [completed] NAS Reading V2 loader/student/package 基线仍通过，Phase 7 现可进入 NAS package/provider/student 集成，不再受 Phase 6 基线阻塞。
- [completed] L3 以 one-Part form fixture 为输入，补齐 Listening package/provider/slot attempt/submit 的可重复垂直切片；`npm run verify:phase7:listening-package` 实测 provider 与音频资源完整性。

## 2026-08-13 Phase 7 收尾验证

- [completed] 新增 `src/services/listeningRuntimeV1.ts`：source semantic gate、interaction model、attempt revision/media binding、slotId answer mutation、submit、IELTS text/option scoring 和 asset descriptor resolver 均为 framework-neutral API。
- [completed] NAS 新增 `server/src/lib/library/listening/listening-v1-loader.ts` 与 `/api/exam/listening/:examId` 及音频资源端点；V1 Reading loader 不变，Listening 仅由 `modality: listening` manifest entry 进入。
- [completed] `phase7-listening-families-v1.json` 与 verifier 生成并校验 note/table/multiple-choice/matching/plan-map/diagram/shared short-answer 结构；hotspot 缺失 option list 会 fail closed。
- [completed] verifier 生成四 Part/40-slot complete fixture，确认 `complete_exam` 才强制 4×10，partial practice 不误报；cue、任务归属、题号和 scoring slot closure 均有语义门。
- [completed] `npm run verify:phase7:listening-contract`、`npm run verify:phase7:listening-package`、`npm run verify:phase6:runtime`、NAS `npm run verify:phase6:reading-v2` 与两仓 schema hash gate 全部通过。

## 2026-08-13 Listening opt-in student integration

- [completed] NAS 增加稳定的 `ListeningLibraryProviderFactory`、`NasJsDirectListeningAssetProvider`、generated-loader 和 `ExamListeningService` 入口；Listening 不回退到旧 Reading HTML loader。
- [completed] 学生端新增 `/listening` opt-in route、slotId answer composable、音频预加载/播放策略控制、键盘选项列表、完整提交窄口和成绩回显；默认 `listeningV1` feature flag 仍为关闭。
- [completed] Listening API 只返回脱敏 runtime payload，不向学生暴露 `answerKey`；提交时服务端重新绑定 source revision、media hash、probe/playback 状态并在服务端计分。
- [completed] Phase7 contract verifier 现在同时检查 provider/service/page 静态接线、NAS server 编译和 student-exam Vite production build。

## 2026-08-12 L4 交付

- `ListeningPlaybackPolicyV2` 现在显式声明 pause、seek、replay/max plays、refresh 与 crash recovery；不再从 UI 控件或 DOM 反推策略。
- mock policy 固定为单次播放、禁止 pause/seek/replay，refresh/crash 必须 `resume_from_snapshot`；矛盾 policy 同时被 JSON Schema 与 Rust semantic gate 拒绝。
- 新增 framework-neutral `listeningPlaybackControllerV1.ts`，其 snapshot 可直接进入 `ListeningAttemptV1`；practice 的暂停、seek、恢复和 replay 均通过确定性 event transition 驱动。
- refresh/crash 保留 `playsStarted` 和 `positionMs`；允许 restart 的 practice 使用 `restart_pending`，重新播放时才消耗下一次 play，达到上限则稳定失败为 `AUDIO_RECOVERY_BLOCKED`。
- 伪造 `playsStarted=0` 的 active snapshot、blocked audio probe、mock pause/seek/replay、越界或倒退 progress 均 fail closed。
- `npm run verify:phase7:listening-contract` 现通过 8 个 Rust 测试、TypeScript 和状态机负向路径；PDF2TEST/NAS contract hash gate 再次同步并通过。

## 2026-08-13 Listening 生产恢复闭环

- [completed] Listening API 新增受 feature flag 保护的 progress 读写端点；服务端将 attempt、source revision、媒体 hash、部分答案和 controller playback snapshot 写入终端 SQLite，启动恢复可辨识 `listening` phase。
- [completed] `StartupCheck` 恢复分支、`useExamSession` 和 Listening page 支持恢复 snapshot/answers；刷新或崩溃不会把 mock 播放次数重置，也不会绕过 probe/policy 校验。
- [completed] 学生页由 `ListeningPlaybackControllerV1` 驱动 HTMLAudioElement 的所有 play/pause/seek/progress/ended 变化，提交不再硬编码播放状态；服务端要求 attempt 归属、source/media 绑定和严格状态/基数/选项校验。
- [completed] Listening submit 成功后锁定 Reading suite，并通过状态机进入 Reading；Reading/Writing 默认路径和 Listening feature flags 继续保持兼容、默认关闭。
- [completed] NAS provider 对 `source.assets` 与 staged `asset-manifest` 做精确全量闭包及 descriptor 一致性校验，避免 orphan/缺失/错 hash 资源进入学生端。
- [completed] Listening payload 现在携带已校验的 asset manifest；学生端通过受限 Listening asset API 加载 diagram/map 视觉资源，同时保留 keyboard option fallback，未暴露 answer key。
