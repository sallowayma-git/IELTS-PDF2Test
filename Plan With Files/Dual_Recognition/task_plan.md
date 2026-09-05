# Task Plan — IELTS PDF2Test 产品简化 / 双路识别 / WYSIWYG 重构

> 权威计划：[../IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md](../IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md)
> 实施基线 commit：`bb978be`（`fix: harden AI import review and V2 publishing`）→ 冻结基线 `5daa04f`（2026-09-05 审计快照）
> 追踪目录：`Plan With Files/Dual_Recognition/`
> 启动日期：2026-09-05；续建计划（M0-M7）启动日期：2026-09-05

## Goal

把当前 11 页面、双事实源、V1-first 识别的应用收敛为**三个主表面 + 一个权威数据源**：

```
题库/批量导入  ->  最终考试界面式所见即所得工作区  ->  题库内选择并发布 NAS
设置：仅模型连接与少量必要偏好
唯一权威内容：Canonical Exam DS（基于 IeltsAuthoringIRV2）
```

同时达成：本地 `DocumentIRV2` 几何直接识别、云端完整可渲染候选、确定性对齐合并、持久化后台任务队列、零横向溢出。

## 续建里程碑 M0–M7（对齐原任务书任务 ID）

> 续建计划的实施顺序：真实验收基础 → Canonical DS → 持久化队列 → 完整编辑能力 → 本地/云端识别 → 合并 → 批量发布与清理 → 旧链退出。
> 记录格式：原任务 ID — 续建里程碑 — 实现状态 — 验证层级 — 证据 — 剩余缺口。

| 里程碑 | 对应原任务 | 内容 | 状态 |
|---|---|---|---|
| **M0** 真实验收基础 | P0-T01/T02/T03 | 基线固化（commit/schema/语料 + --reason 强制）；真实 Tauri E2E（tauri-driver）；AUTHORING_V2_NOT_AVAILABLE 复现结论；跨仓 NAS 契约入口；追踪口径修正 | **complete** |
| **M1** 唯一权威稿 | P2-T01~T04、P3-T03、P7-T02 | library_items_v2 等新表 + 版本化迁移；Repository 切换；编辑事务（baseVersion/user_edited/有界恢复）；typed preflight；devFallback 出生产 | pending（next） |
| **M2** 后端接管调度 | P6-T01/T02/T04/T05 | import_files + Rust scheduler + lease/取消/启动恢复 + `processing://item-updated`；删前端队列与 2s 轮询 | pending |
| **M3** 完整结构编辑 | P3-T01~T06 剩余 | 9 类 EditorCommandV1 全量；renderer/editor 按题型拆分；SourceDrawer/手工补录/热点调整；NAS renderer parity | pending |
| **M4** 本地识别主链替换 | P4-T01~T06 | DocumentIRV2 直通、Question Layout Graph、题号 token-first、硬闭包、physical table、未分配账本 | pending |
| **M5** 云端完整候选与合并 | P5-T01~T06、P6-T03 | skill bundle、CloudRecognitionCandidateV1、repair/salvage、三方合并与 user_edited 保护 | pending |
| **M6** 批量原子发布与清理 | P7-T03/T05、P7-T02 收尾 | publish_library_items 整批原子提交、发布恢复记录、引用集合清理、NAS Electron 实测 | pending |
| **M7** 旧链退出与统一交付 | P8-T01~T05 | 删退休页面/双写/前端调度/旧预览发布；超大文件迁移；Windows 安装包 + 100 PDF 语料 + 故障矩阵验收 | pending |

## Phase Map（本地执行阶段 ↔ 计划 PR，历史记录）

| 阶段 | 内容 | 计划 PR | 状态 |
|---|---|---|---|
| P0 | 基线与护栏：追踪文档、构建基线、溢出矩阵 | PR-01 | complete (S0.6 deferred) |
| P1 | 三表面外壳：三路由、极简 AppShell、CSS 拆分与溢出硬规则 | PR-02 | complete |
| P2 | 题库为产品中心：统一任务+题库列表、ImportDrawer、批量发布 | PR-03 | complete |
| P3 | 工作区与 ExamCanvas：ExamWorkspacePage、原位编辑、题型 renderer | PR-05/06/07 | complete |
| P4 | 后端 Canonical DS 与 Workspace API：library_items_v2、apply_editor_commands | PR-04/05 | next |
| P5 | Durable Processing Queue：SQLite job、lease、事件、启动恢复 | PR-08 | pending |
| P6 | 本地 DocumentIRV2 直接识别：Question Layout Graph、题号/题干/选项几何恢复 | PR-09/10 | pending |
| P7 | 云端完整识别：versioned skill bundle、CloudRecognitionCandidateV1、repair/salvage | PR-11/12 | pending |
| P8 | Reconciliation：对齐、字段级合并、ActionableIssue、原位差异 | PR-13 | pending |
| P9 | 发布/设置/清理收敛：typed preflight、batch publish、artifact cleanup | PR-14/15 | pending |
| P10 | 旧链删除与真实回归 | PR-16 | pending |

## Phase 0 — 基线与护栏（PR-01）  [complete，S0.6 延后]

- [complete] S0.1 建立 `Dual_Recognition/{task_plan,findings,progress}.md` 与 baseline 记录
- [complete] S0.2 记录并验证当前构建基线：`npm run check`、`vite build`、`cargo check`
- [complete] S0.3 新增 `scripts/ui/layout-matrix.mjs`：视口/缩放矩阵断言 `scrollWidth <= clientWidth + 1`
- [complete] S0.4 新增 `fixtures/ui/long-content.json`：长文件名/长错误/多选项/宽表压力内容
- [complete] S0.5 新增 `scripts/verify-product-baseline.mjs`：route/命令/表/flag 快照与漂移检测
- [complete] S0.6 真实 Tauri E2E —— **M0 已落地**（2026-09-05 更新：旧结论「本机未验证 tauri-driver 可用」作废，
  `tauri-driver.exe` 实际存在于 cargo bin，学生端仓库实际在 `F:/workspace/IELTS-NASfor-WenDao`）。
  `npm run e2e:tauri`（scripts/e2e/tauri-import-edit-publish.mjs）：tauri-driver + selenium-webdriver 驱动真实
  Tauri 进程 + WebView2 + 真实 SQLite + 真实文件系统，覆盖 导入 -> 流水线 -> 打开工作区 -> 改一个字符 -> 已保存 ->
  重开持久化 -> 发布（发布被质量门阻止时如实记 blocked，不计通过）。隔离方案：`PDF2TEST_AUTOMATION_DATA_DIR`
  数据根钩子 + `WEBVIEW2_USER_DATA_FOLDER` + `PDF2TEST_AUTOMATION_PDF_DIR/EXPORT_DIR` 免对话框钩子。
  与 product_chain.rs（Rust 命令级）和 library-workspace-smoke.mjs（浏览器级）分层报告，不互相替代。

**出口**：不改变产品行为，但能用自动化证明产品链与溢出状态。
**实测结果**：溢出矩阵 72 项检查 **0 溢出** —— 计划 UX-002 的溢出前提在受支持窗口范围内不成立，
死 CSS 才是真实问题（`.settings-grid/.editor-grid/.llm-grid` 无人使用）。详见 findings.md F9。
因此 P1 的 CSS 工作重定义为「删除死代码 + 建立回归门」，不是「修复线上缺陷」。

## Phase 1 — 三表面外壳（PR-02）  [complete]

- [complete] S1.1 `src/app/router.ts` 收敛为 `library` / `workspace` / `settings`，旧链接重定向 + 记录日志
- [complete] S1.2 `src/app/App.tsx` 只分发三页；删除全局 `listJobs()`/`activeJob`/`refreshToken` 全页重载
- [complete] S1.3 `src/components/AppShell.tsx` 极简两入口；删除转化工具组、stepper、job-strip
- [complete] S1.4 CSS 拆分为 `src/styles/{tokens,reset,app-shell,library,workspace,exam-canvas,settings,overlays,utilities}.css`
- [complete] S1.5 全局溢出硬规则 + 删除 `.review-grid/.editor-grid/.llm-grid/.settings-grid` 固定三栏 + 主 surface 去 `overflow:hidden`

**出口**：普通导航只显示题库与设置；1100×760 无横向溢出；旧功能仅通过兼容 URL 可达。
**实测**：`routeNames` 15 -> 4（library/workspace/settings + 不可导航的 legacy），
`AppShell.tsx` -73.7%、`App.tsx` -61.1%、`styles.css` -99.1%（只剩 10 行分层导入）。
溢出矩阵 84 项检查 0 溢出。旧链接 `#/dashboard`、`#/jobs`、`#/jobs/new`、`#/export`、`#/library/:id`、
`#/jobs/:id/*` 全部重定向；旧页面保留在 `#/legacy/...`（P10 删除）。

## Phase 2 — 题库为产品中心（PR-03）  [complete]

- [complete] S2.1 `src/features/library/*`：LibraryPage/Header/List/Row/BatchBar/store/types
- [complete] S2.2 任务行与题库行统一：一个列表同时表达 processing 阶段与 library 状态
- [complete] S2.3 `src/features/import/*`：ImportDrawer 只选文件，其余默认值
- [complete] S2.4 批量多选 -> 发布已选择；行菜单重试/删除
- [complete] S2.5 状态筛选（全部/处理中/待检查/可发布/失败）+ 搜索 + 回收站折叠为筛选项

**出口**：导入在题库内完成，批量选中后立即出现行，不等第一份 PDF 解析完成。
**实测**（`npm run e2e:library-workspace`）：导入抽屉选文件 -> 开始导入 -> 抽屉立即关闭 ->
题库出现条目 -> 后台识别完成后阶段变为「待检查」。两阶段导入 + 固定并发（本地 2 / 云端 2）已落地。

## Phase 3 — 工作区与 ExamCanvas（PR-05/06/07）  [complete]

- [complete] S3.1 `src/features/editor/ExamWorkspacePage.tsx`：header + Canvas + SourceDrawer + 问题侧栏
  （**口径修正 2026-09-05**：审计指出「IssuePopover」实为中央问题列表 + scrollIntoView 定位，非 §8.6 的
  题组右上角标记 + 小型 popover；原位云端 diff 属 M3。原记录言过其实）
- [complete] S3.2 `src/exam-canvas/*`：改名迁移完成 + MatchingMatrix/InlineTextEditor 两个子模块
  （**口径修正 2026-09-05**：计划 §16.7 列出的 5 renderer + 4 editor + ContentNodeRenderer 共 10 个文件
  只落地 2 个；choice/completion/table 仍内联于 ExamCanvas.tsx——拆分剩余项归 M3）
- [complete] S3.3 删除裸 `contentEditable` + `document.execCommand`，改为原位 AutoSizeTextarea（IME 安全）
- [complete] S3.4 `EditorCommandV1` 协议（含 `set_text` + `expectedText` 乐观并发）
- [complete] S3.5 `MatchingMatrix` 等题型 renderer；author/student 同 DOM 结构
- [complete] S3.6 workspace 两栏布局 + 窄窗 tab 切换，source 以 overlay drawer 打开

**出口**：打开题目即最终布局，无 edit/preview 开关；改一个字符刷新后仍在。
**实测**：13 步冒烟全过，含「没有编辑/预览切换」「进入原位编辑」「删一个字符后保存」「刷新后修改仍在」。
`contentEditable` 与 `document.execCommand` 在 ExamCanvas 中已完全移除。

## Current Decisions

- **不发明第三套 schema**：`CanonicalExamDsV1 = IeltsAuthoringIRV2` 别名，复用现有 runtime compiler 与 ExamCanvas。
- **渐进收敛而非推倒重写**：Phase 1 保留旧页面为兼容 URL（不在导航），待新表面稳定后在 P10 删除。
- **P1-P3 前端优先**：先让客户获得可用的三表面产品，再在 P4-P8 提升识别质量；避免同时改前后端造成不可验证。
- **改一个字符 = 改 canonical text node**，不在每次输入重写磁盘 JS；发布时编译。
- **CSS 不靠提高 minWidth 掩盖**：`tauri.conf.json` 的 `minWidth: 1100` 保持不变，布局必须自适应。

## Errors Encountered

| Error | Attempt | Resolution |
|-------|---------|------------|
| 溢出探针把 `.hero-panel::after` 装饰伪元素误报为内容被裁 | 1 | 改为遍历真实后代矩形与容器内容框比较，排除 absolute/fixed |
| 探针自带 `<input type=radio>` 继承 `input{width:100%}` + UA `margin-left:5px`，稳定误报 5px | 2 | 选项行改为纯文本；同时暴露真实缺陷 F13 |
| 加了 `.table-wrap{overflow-x:auto}` 后宽表格被误报 `past_viewport` | 3 | 只有元素与视口之间无横向滚动/裁切容器时才算缺陷 |
| heredoc 把 `\s` 折成 `\s`，模板字面量里变成 `s`，`split(/s+/)` 切碎类名 | 1 | 改用 `el.classList`，不再在模板里写正则转义 |
| 浏览器 E2E 导入失败：`没有拿到 TXT 真实解析结果` | 1 | 根因是 `takeDevPickedPath` 丢弃 textContent（F14），已修复透传 |
| 浏览器 E2E 保存超时：`element.blur()` 在 headless 无窗口焦点时不派发 focusout | 2 | 测试改用 Enter 提交；同时修掉组件的闭包陈旧风险（F15） |
| 真实导入的 job 打不开工作区（`AUTHORING_V2_NOT_AVAILABLE`） | 1 | 浏览器级：devFallback 不为真实导入造稿（F16），维持环境限制结论。**M0 实测更新**：真实 Tauri 链路上，流水线完整跑完后 V2 shadow 正常写出、工作区正常打开、编辑与重开持久化全通过（`npm run e2e:tauri` 全绿）；仅在流水线**进行中**打开工作区会命中该错误，且降级 UI 把原始错误码+路径暴露给用户——记为真实 UX 缺陷（findings F-M0-3），排入 M2（workspace live update）/M3（错误文案分层）修复 |

## 已知差距与 P4 交接

本轮完成的是**产品表面收敛**，识别质量与数据面收敛尚未开始。交接给 P4 的明确差距：

1. **真实 Tauri E2E 缺失（最高优先）。** 浏览器冒烟不能证明 Rust/SQLite/文件清理/NAS 发布。
   建议 P4 第一步就落 `src-tauri/tests/product_chain.rs`：临时 AppData + 真实 SQLite，
   驱动 `create_import_job -> import_source_file -> run_auto_pipeline -> get_authoring_v2
   -> apply_authoring_v2_patches -> publish_nas_package_v2`，断言 DB 行、canonical DS、临时文件清理。
2. **后台任务仍在页面进程内。** `useImportFiles` 阶段 B 用固定并发池；应用退出会中断。
   P5 把它换成 Rust queue + `processing://item-updated` 事件后，`libraryStore` 的 2 秒轮询也一并删除。
3. **题库状态仍靠前端映射。** `deriveStage()` 把 `JobStatus + WorkflowStep + LibraryStatus`
   折成用户阶段。P4 建 `library_items_v2` / `processing_jobs_v2` 后，阶段应由后端直接给出，
   `libraryTypes.ts` 的映射表随之删除。
4. **`ActionableIssue` 是前端派生的。** `actionableIssues.ts` 按计划 §6.8 做硬闭包检查，
   检查项与将来后端一致，P8 迁到后端时可以两边对比结果。
5. **批量发布不是原子的。** 逐条发布，任一条失败不回滚已提交条目（`nas_package_v2` 只支持单条 staging）。
6. ~~**设置页只做了布局归位**~~（**口径修正 2026-09-05**：实测确认设置页已完整实现 §14——单 Profile、
   只列 OpenAI-compatible/Ollama、`forceJson`/温度固定隐藏、高级折叠、显式「保存并测试」、
   开发者模式诊断；后端 Provider 写入校验收敛仍在 P9）
7. **`src/pages/LibraryPage.tsx` 已成孤儿**，仍参与 `tsc` 但不进 bundle，P10 删除。
8. **`.review-grid` 是唯一活跃三栏**，只在兼容页 `DocumentReview` 用；P10 删除该页时一并清掉。

## Tracking Files

- [findings.md](findings.md) — 代码级审计发现与验证结论
- [progress.md](progress.md) — 会话日志与验证结果
- [baseline.md](baseline.md) — 固定基线快照（route/命令/表/flag/构建）
