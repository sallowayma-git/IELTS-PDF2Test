# Task Plan — 逐章节对抗审计（audit-2026-09-12）

> 权威计划：`../IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md`
> 汇总报告：`./00-CONSOLIDATED-REPORT.md`；发现登记：`./findings.md`
> 审计基线（审计期间）：`47a3806` + 43 项未提交改动（审计过程中工作树被并发修改，见 findings §0/R6）
> **已固化基线**：`6affc571f43b175ffdb5a13d1823ba6f2d4962a3`（2026-09-12 冻结，`npm run check` / `cargo check --locked` 均通过）

## 本轮目标

对计划文档 28 章 + 附录做逐章节对抗审计，产出可执行、带 `file:line` 证据、分层标注证据强度的发现清单。

## 完成情况

| 项 | 状态 |
|---|---|
| 13 个只读子代理按章节派发 | complete |
| 13 份章节子报告（`A1-*.md` ~ `A13-*.md`） | complete |
| 汇总报告（`00-CONSOLIDATED-REPORT.md`） | complete |
| 发现登记（`findings.md`） | complete |
| 主线程独立复核 7 项关键结论 | complete（纠正 1 条、裁定 1 处冲突） |
| 构建校验：`npm run check` / `cargo check --locked` | complete（均通过） |

**统计**：437 条断言、157 条发现（P0 14 / P1 46 / P2–P3 97）；Phase 0 complete / 6 partial / 3 not started；DoD 30 项满足 0。

## 下一步行动（按优先级，尚未执行）

| # | 行动 | 依赖 | 风险 |
|---|---|---|---|
| 1 | ~~**固化基线**~~ **已完成**：提交为 `6affc57`，并重录 `fixtures/product-baseline.json`（`--reason` 门已过，strict 校验 `no drift`） | — | — |
| 2 | **修正计划文档事实层**：§1.2 前端清单、§1.5 缺口矩阵、§10.1 前提、§16/§17 路径、附录 A 失效路径、§26 结论措辞 | 无 | 低（文档） |
| 3 | **补产品级验收门**：修 `title-persists` / `publish` 两个失败步骤；补 `tauri-workspace-edit.mjs`、`tauri-publish.mjs`；接入 CI | 需真实 Tauri 运行环境 | 中 |
| 4 | **修数据正确性缺陷**：A4-F01 导入失败孤儿、A4-F02 取消被吞、A3-F04 空操作开关、A7-F04 原始错误码直送 UI | 无 | 中（产品代码） |
| 5 | **先修 `user_edited` 护栏再启动 M5**（A7-F02） | 无 | 中 |
| 6 | **M4 接入主链**：`QuestionLayoutGraphV1` 目前仅附加 artifact；需实现 §6.5–§6.8 | 依赖 #1 | 高 |
| 7 | **M5 前置**：skill bundle + `CloudRecognitionCandidateV1` + repair/salvage | 依赖 #5 | 高 |

## 明确不做（本轮）

- 不修改任何产品代码与配置（只读审计）。
- 不修改权威计划文档正文（发现以 errata 形式记录在 `findings.md`，待用户确认后再回写）。
- 不运行 `cargo test` / `npm run build` / e2e / 真实 LLM / 真实 NAS 写入。
- 不提交、不 stash、不清理工作树（存在并行写入者）。

## 风险登记

| 风险 | 说明 |
|---|---|
| 基线漂移 | 审计期间 `src-tauri/src/recognition/local/` 新增（mtime 11:44–11:47），结论具时序性 |
| 并行写入者 | 存在另一会话/代理正在实现 M4，任何产品代码改动前需先协调 |
| 无产品级证据 | 唯一真实 Tauri E2E 为 failed，DoD 无一项达 `product` 级 |
