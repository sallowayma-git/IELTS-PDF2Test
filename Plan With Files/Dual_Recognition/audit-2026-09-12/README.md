# 逐章节对抗审计 · 复审入口（2026-09-12）

> 本目录是对 `IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md`
> 的**逐章节对抗审计**交付物。本文件供复审 agent 作为唯一入口。

---

## 1. 一句话结论

计划文档的**产品方向自洽**，但**实现层大面积未落地**：437 条断言中 157 条不成立或部分成立；
DoD 30 项 **0 项满足**；计划第 26 章宣称的"四轮对抗审计通过"**不可信**——应为"四轮文档自洽复核通过"。

---

## 2. 审计对象与基线

| 项 | 值 |
|---|---|
| 被审计划 | `Plan With Files/IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md`（28 章 + 附录 A/B/C，4308 行） |
| 审计基线 SHA | **`6affc571f43b175ffdb5a13d1823ba6f2d4962a3`** |
| 计划自称基线 | `bb978be`（**已过时**，见发现 A1-F01） |
| 基线记录提交 | `9cc195f`（重录 `fixtures/product-baseline.json`）、`37824c7`（修 `--strict` gate 缺陷） |
| 审计日期 | 2026-09-12 |
| 审计方式 | 13 个只读子代理按章节分组并行核对 + 主线程 7 项独立复核 |

---

## 3. 建议阅读顺序

| 顺序 | 文件 | 内容 | 篇幅 |
|---|---|---|---|
| 1 | `00-CONSOLIDATED-REPORT.md` | **汇总报告**：基线告警、总体统计、P0/P1 汇总、各章一句话结论、与历史审计对比、局限 | ~230 行 |
| 2 | `findings.md` | **发现登记**：主线程 7 项复核裁定（含纠正 1 处子代理误判、裁定 1 处冲突）+ 157 条发现索引 | ~180 行 |
| 3 | `A1-*.md` … `A13-*.md` | **章节子报告**：逐条断言核对表 + `file:line` 证据 + 发现清单 | 各 180–400 行 |
| 4 | `progress.md` | 审计过程与会话日志 | ~60 行 |
| 5 | `task_plan.md` | 后续行动清单（4 条轨道） | ~70 行 |

子报告 ↔ 章节对照：

| 子报告 | 覆盖章节 | 子报告 | 覆盖章节 |
|---|---|---|---|
| `A1-ch0-1-current-state.md` | §0–1 | `A8-ch9-10-wysiwyg-css.md` | §9–10 |
| `A2-ch2-3-acceptance-ia.md` | §2–3 | `A9-ch13-14-publish-settings.md` | §13–14 |
| `A3-ch4-11-canonical-ds-library.md` | §4 + §11 | `A10-ch16-17-file-checklist.md` | §16–17 |
| `A4-ch5-12-processing-import.md` | §5 + §12 | `A11-ch18-19-24-phases-tests-dod.md` | §18/19/24 |
| `A5-ch6-local-recognition.md` | §6 | `A12-ch20-23-25-appendix.md` | §20–23、§25 + 附录 |
| `A6-ch7-cloud-recognition.md` | §7 | `A13-ch26-28-audit-records.md` | §26–28 |
| `A7-ch8-15-reconcile-errors.md` | §8 + §15 | | |

---

## 4. 复审时必须知道的 4 个前提

1. **基线曾在审计期间漂移。** 审计窗口内有并行写入者新增 `src-tauri/src/recognition/local/`（mtime 11:44–11:47），
   导致先运行的子代理（A4/A5）报告"该模块不存在"，后运行的（A11/A12）报告"已存在"。
   主线程已裁定：**目录与 `QuestionLayoutGraphV1` 类型已存在，但 `recognize_local` 及 §6.5–§6.8 全部具名函数仍为零命中，
   新图仅作为"附加 artifact"写出、未接入识别主链**——实质结论（M4 主链未交付）仍成立。详见 `findings.md` §0/R6。
2. **子代理结论已被主线程复核，其中 1 处被纠正。** `A10-F02` 声称退休页面"由 `legacyRoutes.tsx` 统一 import"，
   经核 `legacyRoutes.tsx` 无任何 import 者（真孤儿），除 `#/legacy/writing` 外所有 legacy 路径一律重定向。
   详见 `findings.md` §0/R2。其余 6 项复核结论均成立。
3. **证据分层严格。** 本审计明确区分 `product`（真实 Tauri 端到端）/ `command` / `static` / `cli` / `browser-dev-fallback` / `doc-only`。
   **`browser-dev-fallback` 与 `cli`/`schema` 证据不得当作产品验收。** 全库唯一 `product` 级证据是**一条失败记录**（A11-F01）。
4. **未执行项。** 未运行 `cargo test`、`npm run build`、任何 e2e、真实 LLM 调用、真实 NAS 写入；
   100-PDF 语料、50 文件 batch、故障注入矩阵、性能目标均未验证。

---

## 5. 复审可自行执行的校验命令

```bash
npm run check                                   # TypeScript 类型检查（基线时通过）
cargo check --manifest-path src-tauri/Cargo.toml --locked   # Rust 编译（基线时通过，88 warnings / 0 errors）
npm run verify:product-baseline:strict           # 产品面漂移门（基线时 no drift）
```

以下命令**需要真实 Tauri / 语料 / NAS，本环境未执行**，复审如有环境可补：

```bash
npm run e2e:tauri                                # 真实 Tauri 导入→编辑→发布
npm run verify:phase4-eight-pdf-acceptance       # 8 份真实 PDF 验收
```

---

## 6. 最需要复审者复核的 5 条结论

若只能抽查，优先复核这 5 条（均已给 `file:line`）：

| ID | 结论 | 证据 |
|---|---|---|
| A2-F02 / A4-F08 | §5.5 承诺的 local/cloud 并发是纸面设计，实为顺序执行 | `src-tauri/src/processing/scheduler.rs:255-361` |
| A7-F02 | `user_edited` 保护只有写入没有读取，M5 落地后必然覆盖用户编辑 | `src-tauri/src/library/repository.rs:205-223,306` |
| A4-F01 | 导入队列失败留下不可见孤儿文件（DB 无行、磁盘有 job.json+uploads） | `src-tauri/src/processing/commands.rs:112-127`、`library/repository.rs:168-176` |
| A8-F01 | §10.1"当前溢出原因"前提虚假：3 条选择器不存在、3 条只服务不可达页面 | `src/styles/legacy.css:8,54,61`、`src/app/router.ts:47,114` |
| A11-F01 | 唯一真实 Tauri E2E 报告 `verdict=failed`，且早于当前 HEAD | `artifacts/e2e-tauri/run-2026-09-05T11-34-16-269Z/report.json` |

---

## 7. 本目录文件的生成方式

13 个只读子代理各写一份章节报告，主线程负责：汇总统计、跨报告冲突裁定、关键结论独立复核、
与上一轮 `audit-2026-09-07` 的关系判定、基线固化。**审计过程未修改任何产品代码**；
唯一的代码改动是 `scripts/verify-product-baseline.mjs` 的 gate 缺陷修复（提交 `37824c7`，理由见该提交信息）。
