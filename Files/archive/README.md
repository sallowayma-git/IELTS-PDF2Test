# archive/ — 已归档的历史文档

归档日期：2026-09-12

## 为什么在这里

这些文档属于**旧世代（Phase 0–7 文档识别重构）**。当前权威计划是
`Plan With Files/IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md`（2026-09-04），
它明确声明"不是对历史 Phase 0-7 完成记录的复述"，并重新裁剪了产品边界
（最关键的改判：`DocumentIRV2`/`AuthoringIRV2` 由 shadow 转为权威稿，V1 退出新任务主链）。

因此本目录内容**不再作为实施依据**，仅作历史留档。

## 归档判定

| 文档 | 原始路径 | 归档理由 |
|---|---|---|
| `IELTS_Document_Recognition_Phase_0_4_Audit_CN.md` | `Files/` | 2026-08-10 审计 + 2026-08-12 复核，基线 `06f2ddf`；当时结论为"Phase 0-4 不能判定完成、不应进入 Phase 5""V1 继续 authoritative"。该结论与当前计划的产品边界相反。经扫描确认**无任何脚本/fixture 引用** |
| `IELTS_Document_Recognition_Phase_0_Plan_CN.md` | `Files/` | 旧世代阶段计划 |
| `IELTS_Document_Recognition_Phase_1_Completion_CN.md` | `Files/` | 旧世代完成记录 |
| `IELTS_Document_Recognition_Phase_4_8_PDF_Acceptance_CN.md` | `Files/` | 旧世代验收门记录 |

## 注意事项

1. **文件内的相对链接可能已失效。** 例如 Phase 0-4 Audit 引用 `Files/IELTS_Document_Recognition_Overhaul_Plan_CN.md`，
   该文档**未被移动**（它被 `scripts/register-phase0-plan-corpus.mjs` 与 8 个 golden 语料溯源记录引用），
   所以真实路径现在是 `../IELTS_Document_Recognition_Overhaul_Plan_CN.md`。
2. **这些文件仍在 git 历史中完整保留**，`git log --follow` 可追溯移动前后的完整历史。
3. 为什么其余 8 份旧世代文档没被归档：它们被 phase 校验脚本 `readFileSync` 硬依赖（缺失即 `exit(1)`），
   或被 golden 语料用作溯源引用。详见 `../README.md` 第 2.1 节。
