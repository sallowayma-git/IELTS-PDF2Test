# IELTS 文档识别重构 Phase 5 进度

## 本轮自设目标

交付结构化编辑器的第一条可运行纵向切片：用户不需要理解 block、split 或 IR，就能编辑 V2 内容节点、题组、共享答案位、选项库和答案；点击问题或质量 issue 可以定位到源锚点；编辑稿与学生预览保持同源；保存具备防抖、版本冲突保护和本地恢复。

## 已完成

- 新增 `AuthoringEditorSessionV1`，从 `authoring-ir-v2.shadow.json` 或 immutable revision 读取 V2 编辑会话。
- 新增 `replaceText`、`setNodeAttrs`、`setTaskType`、`setQuestionExpression`、`setResponseGroup`、`setAnswer`、`bindSource` patch 操作；后端按 `baseRevision` 做乐观并发检查，验证通过后原子追加 revision。
- 新增结构化编辑器页面：passage 节点、题组类型、instruction、共享 response group、option bank、answer slots、issue rail、source overlay 和学生 parity preview。
- 新增 650ms autosave、本地 recovery、冲突提示/重载；开发模式提供 `phase5-editor-fixture`，现有 job 通过 `/jobs/:jobId/authoring-v2` 进入。
- 保留 V1 authoring 文件和既有 V1 preview 路径；`pdfPerQuestionLlmRepair` 仍为 `false`。

## 验收结果

已通过：

- `npm run verify:phase5:editor`
- `npm run build`
- `npm run verify:phase1:schema:local`
- `cargo check --manifest-path src-tauri/Cargo.toml`
- `cargo test --manifest-path src-tauri/Cargo.toml authoring_v2_commands -- --nocapture`（3 passed）
- `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`
- `git diff --check`

## 未纳入本轮

完整节点增删移动、table/asset/hotspot 的专用编辑器、undo/redo、原生 Tiptap schema 适配，以及真实 PDF 从导入到导出的端到端验收，作为下一阶段继续开发。生产 feature flag 默认关闭；开发环境可直接访问 `#/phase5` 验证纵向切片。
