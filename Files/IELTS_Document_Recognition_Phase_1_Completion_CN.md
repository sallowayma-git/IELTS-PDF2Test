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

收尾时运行：

    npm run verify:phase0:strict
    npm run check
    npm run build
    npm run verify:phase1:schema
    cargo test --manifest-path src-tauri/Cargo.toml schema
    cargo test --manifest-path src-tauri/Cargo.toml artifact_store
    cargo test --manifest-path src-tauri/Cargo.toml schema::migration_v1::tests
    cargo test --manifest-path src-tauri/Cargo.toml pdf_facts_shadow

Phase 0 Rust 环境中的既有失败项仍按 Phase 0 记录处理，不归因于本阶段改动。

## Phase 2 启动边界

下一阶段从总计划 PR-03 开始：line builder、region/column candidate、reading
order shadow 和 V1/V2 compare report。PR-03 不进入 authoring、不改变 V1 输出，
不启用生产 feature flag；先用 Phase 0 corpus 验证行完整性、双栏/三栏顺序和
source anchor 覆盖，再进入 PR-04 的 asset/table/selective OCR shadow。
