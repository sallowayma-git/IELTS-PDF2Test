# 学生端阅读体验四项调整：返工说明（2026-10-16）

你上一轮的改动已经过审查。四项需求基本达成，命令层测试已复现通过（PDF2Test：check 和 551 项单元测试；NAS：31 项；Rust：4 项 fontScale 测试）。下面 6 个小问题请在**现有工作区**上修，不要提交 git，不要动识别管线、golden baseline 和发布门禁。Tauri 原生应用的手动验收由用户自己做，你不用再尝试。

## 需要修改

1. **文档注释挂错了测试**：`src-tauri/src/product_chain.rs` 第 1323 行附近。新测试 `product_chain_font_scale_persists_through_canonical_save_undo_redo_and_preview` 插在了"M1（原 P2-T03/T04）验收"那段 `///` 注释和它原本描述的 `library_v2_workspace_api_persists_edits_without_new_revisions` 之间。请把新测试移到注释前面，让注释重新紧贴原测试。

2. **测试包生成函数被全局改动**：`dump_v2_visual_package_for_part`（同一文件，约第 984 行）现在固定在文章开头插入一个 `fontScale: 1.3` 的标题，所有用它生成的 NAS 测试包都会变。请改成按需开启，只有 `dump_published_v2_visual_package_for_student_e2e` 这条路径插入；其他调用方生成的包内容要和改动前一致。改完后重跑 `npm run e2e:nas-contract` 和 `npm run e2e:student-real-provider`，确认仍是 77/77 和 19/19。

3. **高亮工具条文字两端不一致**：作者端预览（`src/exam-canvas/StudentReadingTools.tsx`）是 "Highlight / 取消高亮"，NAS（`SelectionHighlightToolbar.vue`）是 "Highlight / Remove highlight"。学生端界面以英文为准，两端统一成 "Highlight / Remove highlight"，同步更新依赖这些文字的测试和 e2e 脚本。

4. **NAS 测试配置超出范围**：`apps/student-exam/vitest.config.ts` 把 `root` 改到了仓库根，还强制给 `vue` 指定了别名。请优先恢复原样，把 `developer/tests/student-exam/highlightDom.test.ts` 改为放在 `apps/student-exam/src/` 下与 `highlightDom.ts` 同目录。如果确实必须改配置，在交付说明里写清楚原因和影响。之后 NAS `npm test` 仍须全过。

5. **NAS 阅读页高亮自动保存要写进交付说明**：`ReadingExamPage.vue` 现在每次高亮变化都会调用 `flushDraftToServer` 写一次服务端草稿。行为本身保留，但请在交付说明里写明：何时触发，`hydratingPart` 期间不触发，与导航和提交保存串行，失败时显示"高亮保存失败，请重试。"。如果能用简单的防抖（例如 500ms）合并连续高亮，可以加，但要说明。

6. **缩进**：`apps/student-exam/src/modules/reading-engine/renderers/ReadingExamV2Renderer.vue` 第 11 行 `<div v-for="node in task.instructions" ...>` 多缩进了一层，恢复成和上下文一致的缩进。

## 交付要求

- 列出两个仓库本轮改动的文件。
- 附上重跑结果：PDF2Test 的 `npm run check`、`npm test`、`cargo test --manifest-path src-tauri/Cargo.toml font_scale`、`node scripts/e2e/student-reading-experience.mjs`、上面第 2 条的两个 e2e；NAS 的 `npm test`、`reading_v2_student_flow.py`、`listening_v1_student_flow.py`。
- 照旧分清三类证据：端到端产品验证、命令层验证、只有 schema 的证据。
