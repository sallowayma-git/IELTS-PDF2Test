# Files/ 文档索引

> 最后整理：2026-09-12
> 目的：区分**当前权威文档**、**历史（已被取代）**与**另一主题参考**，避免复审者被旧世代文档误导。

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

## 2. 历史文档（已被当前计划取代）

> 依据：当前计划 §0.1 明确"不是对历史 Phase 0-7 完成记录的复述"，并重新裁剪了产品边界（V2 由 shadow 转为权威稿）。
> **本目录下的历史文档一律不再作为实施依据。**

### 2.1 留在 `Files/` 根目录（**不可移动：被工具链钉住**）

这些文档仍被脚本或 golden 语料溯源引用，**移动或删除会打断校验链**，因此保留原位。

| 文档 | 被谁引用 | 引用方式 |
|---|---|---|
| `IELTS_Document_Recognition_Overhaul_Plan_CN.md` | `scripts/register-phase0-plan-corpus.mjs`、`fixtures/golden/metadata/*.json`（8 个文件 × 2 处） | 溯源引用 `…Plan_CN.md#2.1` / `#23.2` |
| `IELTS_Document_Recognition_Phase_2_Completion_CN.md` | `scripts/verify-phase2-shadow.mjs` | `readFileSync` 硬依赖，缺失即 `exit(1)` |
| `IELTS_Document_Recognition_Phase_3_C001_Completion_CN.md` | `scripts/verify-phase3-docx-package.mjs`、`scripts/verify-phase3-docx.mjs` | 同上 |
| `IELTS_Document_Recognition_Phase_3_Completion_CN.md` | `scripts/verify-phase3-docx.mjs` | 同上 |
| `IELTS_Document_Recognition_Phase_4_Completion_CN.md` | `scripts/verify-phase4-grammar.mjs` | 同上 |
| `IELTS_Document_Recognition_Phase_5_Progress_CN.md` | 根 `task_plan.md`（散文引用） | 无硬依赖，但属同一世代，一并留档 |
| `IELTS_Document_Recognition_Phase_6_Progress_CN.md` | `scripts/verify-phase6-runtime.mjs` | `readFileSync` 硬依赖 |
| `IELTS_Document_Recognition_Phase_7_Progress_CN.md` | `scripts/verify-phase7-listening-contract.mjs` | 同上 |

> 如需彻底移出 `Files/`，必须同时改 7 个校验脚本 + 8 个 golden metadata 的引用路径。该动作会触碰 golden 语料，按仓库 `AGENTS.md` 的基线漂移规则应单独评审，不在本次整理范围内。

### 2.2 已归档到 `Files/archive/`

零外部工具链引用，可安全移动：

| 文档 | 归档理由 |
|---|---|
| `IELTS_Document_Recognition_Phase_0_4_Audit_CN.md` | 2026-08-10/12 历史审计（基线 `06f2ddf`，当时判定"V1 仍 authoritative"），结论已被当前计划取代；无外部引用 |
| `IELTS_Document_Recognition_Phase_0_Plan_CN.md` | 旧世代阶段计划 |
| `IELTS_Document_Recognition_Phase_1_Completion_CN.md` | 旧世代完成记录 |
| `IELTS_Document_Recognition_Phase_4_8_PDF_Acceptance_CN.md` | 旧世代验收门记录（其结论自称"仍只证明 shadow acceptance，不等于 V2 已进入生产"） |

---

## 3. 另一主题参考（**非过时**，未纳入本次整理）

| 文档 | 说明 |
|---|---|
| `Epic8-Tauri作者端应用详细设计.md` | Epic 8 作者端设计，与当前计划不冲突 |
| `Epic8-作者端Web导题与组卷器工程设计.md` | 同上 |
| `Windows包体与兼容规划.md` | Windows 打包与兼容规划 |
| `Windows包体与兼容任务追踪.md` | 同上（追踪） |

> 这 4 份属另一主题，未被当前计划取代。如确认同样过时，可再归档。

---

## 4. 相关但位于仓库根目录的旧追踪文档

| 文档 | 状态 |
|---|---|
| `task_plan.md` / `findings.md` / `progress.md`（仓库根） | 多世代累积的旧追踪文档（各约 180–195 KB），含已作废的 "Current Active Goal"。当前追踪以 `Plan With Files/Dual_Recognition/` 为准 |
