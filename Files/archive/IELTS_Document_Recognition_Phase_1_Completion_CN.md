# IELTS Document Recognition Phase 1 完成记录

本记录对应总计划 `Files/IELTS_Document_Recognition_Overhaul_Plan_CN.md` 的 16.2
“Schema、artifact store 和兼容骨架”。Phase 0 golden corpus、V1 baseline 和
默认关闭的 feature flags 保持不变。

## 已完成

- V2 contract bundle：`DocumentIRV2`、`ContentDocV2`、`IeltsAuthoringIRV2`、
  `QualityReportV2`。
- Rust/TypeScript 类型、JSON Schema、`schemaVersion` 精确分派和 bundle hash
  manifest，bundle 版本为 `2026.08.0`。
- canonical JSON：递归稳定对象键序、数组保持语义顺序、SHA-256 可复现。
- `JobArtifactLayoutV1`：sources、extraction、authoring revisions/patches、
  assets、preview、export-history、legacy 目录。
- canonical V2 artifact 的同目录临时文件写入、flush/sync、替换；revision 的
  base revision 冲突检测、immutable revision record 和缺失指针恢复。
- V1 → V2 只读 migration preview：保留原始 V1 artifact、标记 `needs_review`；
  V2 → V1 在无 lossless compiler 前明确阻止。
- 旧 job 可在没有 Phase 1 目录和元数据时读取；旧 `job.json`、V1 document/
  authoring artifact 和旧导出路径不被 V2 store 改写。
- PR-02 的 PDF facts shadow 仍为开发环境显式 opt-in；所有 V2 flags 默认关闭，
  `pdfPerQuestionLlmRepair` 继续强制关闭。

## Phase 1 收尾验收

2026-08-10 复审后的 contract/corpus 收敛：

- Phase 0 的 authoritative private corpus 改为
  `manifest.requiredPrivateCorpus` 精确选择 8 个 case；`private-real` 目录允许同时
  保存固定随机种子的扩展样本，目录文件总数不是契约。
- 5 个 Phase 3 有意重生成的 DOCX fixture 已同步 source hash、size、metadata 和
  V1 baseline；每份 metadata 的 `sourceRevision` 保存旧 hash、生成器和修订原因。
- 8 个计划 PDF 的 golden metadata 已补齐 response group、cardinality、assignment、
  option bank scope/binding/reuse、per-slot scoring、source evidence 和 runtime
  expectation；Phase 0 verifier 校验 slot 唯一归属和全部引用闭包。
- NAS contract mirror 位于 `E:/NAS/developer/contracts/authoring`。默认
  `verify:phase1:schema` 必须执行 peer hash 校验；单仓 CI 只能显式使用
  `verify:phase1:schema:local`，不再静默返回 `not_checked`。
- Windows CI path gate 已覆盖 `contracts/**` 和 `fixtures/**`，并运行 tracked corpus
  contract 与 local schema contract。私有 PDF 仍不进入仓库，完整 strict corpus gate
  在有授权本地语料的环境运行。

收尾时运行：

    cargo build --manifest-path src-tauri/Cargo.toml --bin ielts-author-studio
    npm run verify:phase0:strict
    npm run verify:phase0:ci
    npm run check
    npm run build
    npm run verify:phase1:schema
    npm --prefix E:/NAS run verify:authoring-schema-contract
    cargo test --manifest-path src-tauri/Cargo.toml schema
    cargo test --manifest-path src-tauri/Cargo.toml artifact_store
    cargo test --manifest-path src-tauri/Cargo.toml schema::migration_v1::tests
    cargo test --manifest-path src-tauri/Cargo.toml pdf_facts_shadow

其中 Phase 0 strict 必须使用当前 Rust/Cargo 输入构建的 debug CLI；缺失或陈旧 CLI
会直接失败，不允许复用旧二进制取得 V1 baseline 假绿。

Phase 0/1 默认 flags 也由 gate 直接解析 `src/config/featureFlags.ts` 的 TypeScript AST：
必需 flag 缺失、默认 `true`、非字面量、重复定义或无法解析的 spread 都会失败；不再
只相信 manifest 中现有键。

Phase 0 Rust 环境中的既有失败项仍按 Phase 0 记录处理，不归因于本阶段改动。

## Phase 2 启动边界

下一阶段从总计划 PR-03 开始：line builder、region/column candidate、reading
order shadow 和 V1/V2 compare report。PR-03 不进入 authoring、不改变 V1 输出，
不启用生产 feature flag；先用 Phase 0 corpus 验证行完整性、双栏/三栏顺序和
source anchor 覆盖，再进入 PR-04 的 asset/table/selective OCR shadow。
