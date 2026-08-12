# IELTS 文档识别重构 Phase 5 进度

## Phase 5 自设目标（已完成）

依据总任务书第 16.6、20.4、24.10 与 24.11 节，完成从真实 PDF 导入结果到结构化编辑、学生同源预览、版本化保存和 V2 bundle 导出的可验收闭环。普通用户只接触上传、处理、编辑预览和导出；复杂 IR 诊断通过 issue rail/source overlay 按需出现。V1 文件继续可读，PDF 逐题 LLM repair 始终关闭。

## 已完成交付

- `AuthoringEditorSessionV1` 从 V2 shadow 或 immutable revision 打开；V2 patch 按 `baseRevision` 做乐观并发检查，验证后以 append-only journal/revision 原子保存。
- 完整 patch 语义：文本和内容树替换、节点插入/删除/移动、节点属性、题型/题号表达式、response group/cardinality、答案、source binding、issue resolution、资源裁剪和 diagram/figure hotspot；删除包含答案槽的节点会 fail closed。
- 节点移动和删除的撤销/重做保留嵌套父节点上下文；键盘和工具栏支持 undo/redo；650ms autosave、localStorage crash recovery、revision conflict 提示与重载均已接通。
- Tiptap 3 schema 已接入真实编辑界面：paragraph/heading/list/text/answer slot、table row/header/cell、image、figure、flowchart、diagram 和 source/provenance metadata 均有 canonical IR ↔ Tiptap 映射；表格增删行列、合并/拆分和富文本 marks 可编辑。
- 结构化编辑器支持 passage、instruction、共享 prompt、公共 option bank、答案槽、table、asset alt/crop、hotspot 坐标拖拽/录入/删除、issue rail/source overlay；issue 点击按 `targetId` 定位具体编辑节点，源锚点有 page/bbox 显示；学生 parity preview 通过 `RuntimeViewModelV2` 和 `ReadingInteractionModelV2` 读取同一份 V2 authoring/runtime contract。
- 增加 V2 导出按钮和 history receipt：先生成 `authoring-ir-v2.json`、`reading-source-v2.json`、`manifest-v2.json` staging bundle，再原子提交；lock/journal 覆盖 staging、commit、失败清理与 history 失败回滚；导出前阻断未解析答案、未解决 blocker、编译失败和 revision 冲突。V1 NAS/export 路径不被覆盖。
- 真实 auto pipeline 在 `EPIC8_AUTHORING_V2_SHADOW=1` 受控开关下写入 V2 shadow；关闭时保留 V1 默认路径，导入后的 V2 路由探测失败自动回退 legacy preview。
- `/jobs/:jobId/authoring-v2` 与 `#/phase5` fixture 已接入；`authoringEditorV2` 默认关闭，只接受 `VITE_IELTS_AUTHORING_EDITOR_V2=1` 或明确的 localStorage opt-in；`pdfPerQuestionLlmRepair` 强制为 `false`。

## 端到端验收

已通过的关键验收：

- `npm run check`
- `npm run build`
- `npm run verify:phase5:editor`
- `npm run verify:phase5:real-pdf`：真实 `chili-peppers` PDF 通过导入 → V2 shadow → 答案编辑 → 文本编辑 → immutable revision 2 → `ReadingExamSourceV2` bundle export；报告包含 answer patch count、revision、manifest 和 schema 版本。
- `npm run verify:phase1:schema:local`
- `cargo check --manifest-path src-tauri/Cargo.toml`
- `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`
- `npm run verify:phase6:runtime`
- `cargo test --manifest-path src-tauri/Cargo.toml --lib authoring_v2_commands -- --nocapture`
- `git diff --check`

生产边界保持明确：Phase 6 才负责 NAS V2 loader、学生端 Vue node renderer、slot-based attempt/scoring 与正式双读 rollout；本阶段导出的是可验证的 V2 shadow bundle，并保留现有 V1 学生端和发布链路。Phase 5 审计复核以当前工作区代码和最后一轮真实 PDF 报告为准。
