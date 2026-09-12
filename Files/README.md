# Files/ 文档索引

> 最后整理：2026-09-12
> 目的：只保留**当前权威文档**与**另一主题参考**，旧世代（文档识别 Phase 0–7 / Overhaul Plan）文档已全部清理。

---

## 1. 当前权威计划与追踪（以这些为准）

| 文档 | 位置 | 说明 |
|---|---|---|
| **产品简化 / 双路识别 / WYSIWYG 计划** | `Plan With Files/IELTS_PDF2Test_Product_Simplification_Dual_Recognition_WYSIWYG_Plan_CN.md` | 2026-09-04，V1.0。**当前唯一权威计划** |
| 里程碑追踪 | `Plan With Files/Dual_Recognition/task_plan.md` | M0–M7 状态与口径修正 |
| 代码级发现 | `Plan With Files/Dual_Recognition/findings.md` | |
| 会话日志 | `Plan With Files/Dual_Recognition/progress.md` | |
| 门禁状态 | `Plan With Files/Dual_Recognition/gate-status.md` | |
| **本轮逐章节对抗审计** | `Plan With Files/Dual_Recognition/audit-2026-09-12/` | 见该目录 `README.md`（复审入口） |
| 上一轮审计与修复 | `Plan With Files/Dual_Recognition/audit-2026-09-07/`、`repair-2026-09-07/` | 历史轮次 |

---

## 2. 另一主题参考（**非过时**）

| 文档 | 说明 |
|---|---|
| `Epic8-Tauri作者端应用详细设计.md` | Epic 8 作者端设计，与当前计划不冲突 |
| `Epic8-作者端Web导题与组卷器工程设计.md` | 同上 |
| `Windows包体与兼容规划.md` | Windows 打包与兼容规划 |
| `Windows包体与兼容任务追踪.md` | 同上（追踪） |

> 这 4 份属另一主题，未被当前计划取代。如确认同样过时，可再归档或删除。

---

## 3. 相关但位于仓库根目录的旧追踪文档

| 文档 | 状态 |
|---|---|
| `task_plan.md` / `findings.md` / `progress.md`（仓库根） | 多世代累积的旧追踪文档，含已作废的 "Current Active Goal"。当前追踪以 `Plan With Files/Dual_Recognition/` 为准 |

---

## 4. 已清理文档（2026-09-12 删除，可追溯）

以下文档属旧世代（文档识别 Phase 0–7 与 Overhaul Plan），已被当前计划取代。它们此前分别被 6 个校验脚本、1 个 golden 注册脚本与 8 个 golden metadata 以**纯溯源方式**引用（无行为依赖）。

2026-09-12 已先解耦全部引用，再删除文档本体：

- 校验脚本 `requiredFiles` 中移除文档条目：`verify-phase2-shadow` / `verify-phase3-docx` / `verify-phase3-docx-package` / `verify-phase4-grammar` / `verify-phase6-runtime` / `verify-phase7-listening-contract`
- `scripts/register-phase0-plan-corpus.mjs` 的 `review.method` 由 `source-text-and-overhaul-plan-evidence` 改为 `source-text-evidence`，并移除 2 条 Overhaul Plan 证据行
- 8 个 golden metadata（`chili-peppers` / `conformity` / `fishbourne-roman-palace` / `listening-to-the-ocean` / `organisational-design` / `petri-dish` / `sleep-study` / `western-celebrity`）同步做相同调整

**恢复方式**：这些文件在删除前的最后一个提交中完整存在，可用 `git show <sha>:<path>` 取回。删除前的 HEAD 为 `e33d20a`。

| 文档 | 删除前最后提交 |
|---|---|
| `IELTS_Document_Recognition_Overhaul_Plan_CN.md` | `87b7747` |
| `IELTS_Document_Recognition_Phase_2_Completion_CN.md` | `ac0a68c` |
| `IELTS_Document_Recognition_Phase_3_C001_Completion_CN.md` | `ac0a68c` |
| `IELTS_Document_Recognition_Phase_3_Completion_CN.md` | `ac0a68c` |
| `IELTS_Document_Recognition_Phase_4_Completion_CN.md` | `15cf526` |
| `IELTS_Document_Recognition_Phase_5_Progress_CN.md` | `36cd3f1` |
| `IELTS_Document_Recognition_Phase_6_Progress_CN.md` | `8806272` |
| `IELTS_Document_Recognition_Phase_7_Progress_CN.md` | `8806272` |
| `archive/IELTS_Document_Recognition_Phase_0_4_Audit_CN.md` | `e33d20a` |
| `archive/IELTS_Document_Recognition_Phase_0_Plan_CN.md` | `e33d20a` |
| `archive/IELTS_Document_Recognition_Phase_1_Completion_CN.md` | `e33d20a` |
| `archive/IELTS_Document_Recognition_Phase_4_8_PDF_Acceptance_CN.md` | `e33d20a` |
| `archive/README.md` | `e33d20a` |

> 注意：`Files/archive/` 目录（含其 `README.md`）已一并删除，目录不再存在。

---

## 5. 复审建议

复审者应**只以第 1 节**的文档作为实施依据。第 2 节属另一主题，与当前计划不冲突但也不构成依据。若在历史审计材料中看到对第 4 节所列文档的引用，请注意那些引用已随文档清理而失效，结论应以当前计划与 `audit-2026-09-12/` 为准。
