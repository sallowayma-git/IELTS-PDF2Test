# IELTS PDF2Test 产品简化、双路识别与所见即所得重构实施计划

> 审计对象：`sallowayma-git/IELTS-PDF2Test`  
> 固定审计基线：`bb978be07d0d1391e2c73852b1489efc88e563b7`（`fix: harden AI import review and V2 publishing`）  
> 计划版本：V1.0  
> 编制日期：2026-09-04  
> 审计方式：GitHub 远端源码静态审计、模块依赖梳理、业务链路逆向、四轮主线程对抗复核  
> 说明：本文不是对历史 Phase 0-7 完成记录的复述，而是以当前用户需求为准，对现有实现重新裁剪产品边界和开发主线。

---

## 0. 文档目的、结论与使用方式

### 0.1 这份计划解决什么

本计划处理当前应用最核心的五个问题：

1. 前端信息架构和页面数量过多，普通用户需要理解任务、源文档确认、预览、题组、LLM 复核、结构化编辑器、导出等多个技术步骤。
2. 现有 CSS 和多栏布局在桌面最小窗口、Windows 125%/150% 缩放、长文件名和长错误文本下存在横向溢出、文字裁切和卡片变形风险。
3. 本地识别虽然已经建立 `DocumentIRV2` 物理层，但最终题目生成仍大量经过 V1 block/split/authoring 启发式链路，几何信息没有成为题干、选项和共享选项库识别的主依据。
4. 云端目前主要产出 `CloudReadingOutlineV1`，它是“只读对照提纲”，不是完整可渲染 DS；队列又由 `UnifiedPreview.tsx` 使用 `localStorage + lease + window worker` 管理，不能成为可靠的桌面后台任务系统。
5. 编辑、预览、题库详情和发布仍是多个界面与多份数据投影。用户真正需要的是：导入后直接看到最终考试界面，在同一个界面修改，最终从题库选择并发布。

### 0.2 一句话目标架构

将产品收敛为三个用户界面和一个权威数据源：

```text
题库 / 批量导入
       |
       v
最终考试界面式所见即所得编辑器
       |
       v
题库内选择并发布 NAS

设置：只保留模型连接与少量必要偏好

唯一权威内容：Canonical Exam DS（基于 IeltsAuthoringIRV2）
运行时 JSON / JS / NAS 文件：发布时编译生成，不作为题库权威稿
```

### 0.3 最终产品只保留三个主表面

| 主表面 | 主要职责 | 不再单独存在的页面 |
|---|---|---|
| 题库 | 导入 PDF/DOCX、批量任务进度、搜索、打开、选择发布 | Dashboard、JobList、独立 ImportWizard、独立 ExportPage |
| 题目工作区 | 最终 IELTS 风格渲染、原文与题目编辑、差异提示、必要人工确认 | DocumentReview、UnifiedPreview、单独 Phase 5 Editor、LLM Review 页 |
| 设置 | 默认模型连接、测试连接、源文件保留策略 | 大量普通用户不需要的诊断卡、强制 JSON 开关、多个不受支持 Provider 选项 |

### 0.4 本计划的工程原则

- **需求优先于历史 Phase 边界。** 当前仓库已经同时存在 V1、V2、Phase 5、Phase 6、Phase 7 和历史兼容代码；不能继续按“再新增一个页面或一条并行路径”解决问题。
- **本地和云端都是候选生成器，Canonical DS 才是权威稿。** 两路结果都不能直接覆盖用户已经编辑的内容。
- **自动识别先尽可能做好，人工只处理具体缺口。** 不把“置信度 0.76”本身当作用户任务，只展示“第 13 题题干缺失”“B 选项可能断行”等可执行问题。
- **编辑 Canonical DS，不直接编辑生成 JS。** 用户改一个字符时只改对应文本节点；预览立即读取同一 DS；发布时再编译 JS/NAS。
- **简化用户界面，不删除最低限度的数据正确性。** 原子写入、数据库事务、路径约束和必要资源完整性仍在后台保留，但不作为普通用户的页面和复选框。
- **不依赖应用正常退出才清理。** 退出时清理只是补充，还必须在任务成功、取消、启动扫描和 TTL 回收时清理。

---

## 1. 当前远端状态审计

### 1.1 审计基线

本计划固定在以下提交上，避免后续远端变化导致计划与代码错位：

```text
commit: bb978be07d0d1391e2c73852b1489efc88e563b7
message: fix: harden AI import review and V2 publishing
```

当前配置已经默认打开 Reading V2、Authoring V2、Runtime V2、NAS Package V2、Quality Gate V2 和 Phase 5 Editor；Listening 仍默认关闭，逐题 PDF LLM Repair 强制关闭。因此当前问题不是“功能开关尚未启用”，而是新旧链路同时存在、产品表面没有完成收敛。

### 1.2 前端模块清单与当前职责

| 文件/目录 | 当前职责 | 当前判断 | 目标处理 |
|---|---|---|---|
| `src/app/App.tsx` | 11 个页面的路由分发、全局 job 刷新 | 页面职责过多，路由和产品流程耦合 | 重写为 Library / Workspace / Settings 三路由 |
| `src/app/router.ts` | hash 路由；包含 document/split/groups/llm-review/preview/authoring-v2/export 等 | 暴露内部流水线阶段 | 删除内部阶段路由，任务阶段变为题库行状态 |
| `src/components/AppShell.tsx` | 侧栏、转化工具分组、步骤条、当前 job 技术状态 | 导航过深，重复表达流程 | 改为极简三入口；工作区隐藏侧栏或使用窄工具条 |
| `src/components/ExamCanvasV2.tsx` | Reading V2 学生/作者双模式渲染、文本/表格/选项/图形操作 | 当前最值得保留的前端核心 | 升级为唯一 WYSIWYG 渲染与编辑引擎 |
| `src/editor/authoringTiptap.tsx` | 另一套 ContentDoc ↔ Tiptap 映射与编辑器 | 与 ExamCanvas 重复；媒体/流程图为占位表达 | 从主链路移除；只保留迁移期或删除 |
| `src/pages/Dashboard.tsx` | 工作台汇总 | 与题库首页重复 | 删除，题库成为默认首页 |
| `src/pages/JobList.tsx` | 导入任务列表 | 与题库任务态重复 | 删除，任务直接作为题库行 |
| `src/pages/ImportWizard.tsx` | 文件选择、元数据、parse mode、答案文件、云端开关、批量顺序处理 | 普通用户选择过多；导入后跳到第一题 | 改为题库内 ImportDrawer；只选择文件，其他使用默认值 |
| `src/pages/DocumentReview.tsx` | 源 block 检查、低置信度和人工转录 | 技术中间页 | 改为工作区中的“查看原文件”抽屉 |
| `src/pages/UnifiedPreview.tsx` | V1 编辑、预览、云端队列、视觉答案、LLM 建议、确认 | 过度集中且仍不是最终 WYSIWYG | 拆解；云端队列移后端，内容编辑并入 ExamCanvas |
| `src/pages/StructuredAuthoringEditorV2.tsx` | V2 outline、三栏编辑、issue rail、inspector、预览、导出 | 功能完整但产品复杂 | 替换为 `ExamWorkspacePage`，默认就是最终渲染 |
| `src/pages/LibraryPage.tsx` | 题库搜索、过滤、状态、回收站 | 应成为产品中心，但当前字段和操作偏多 | 全面重写为任务+题库统一列表 |
| `src/pages/LibraryExamDetail.tsx` | V1 payload/HTML 详情、元数据、跳导出 | 不是真正可编辑 V2 工作区 | 删除，由 `ExamWorkspacePage` 取代 |
| `src/pages/ExportPage.tsx` | 单题/批量/目录/NAS 发布和复杂错误引导 | 独立页面不符合目标流程 | 发布并入题库批量操作和工作区主按钮 |
| `src/pages/Settings.tsx` | 多 Profile、高级 Provider、URL、模型、Key、超时、温度、JSON、启用、预检、诊断 | 普通用户选项过多；Provider 能力不一致 | 默认简化，只显示一个主模型配置；高级项折叠 |
| `src/api/tauriCommands.ts` | 所有 V1/V2/题库/LLM/导出命令的平铺封装 | 过大且和 dev fallback 强耦合 | 按业务拆成 5 个 client |
| `src/services/devFallbackBackend.ts` | 在浏览器 localStorage 中复制大量后端行为 | 容易与真实 Tauri 分叉 | 从生产路径移除，仅测试构建使用最小 fake adapter |
| `src/services/authoringV2Patches.ts` | 文本、节点、选项、表格、热点等 patch | 可复用的良好基础 | 收敛为稳定的 EditorCommand 协议 |
| `src/services/runtimeViewModelV2.ts` | 从同一 V2 DS 构建 runtime view/interaction model | 符合“同源渲染”方向 | 保留并移入 exam-canvas 领域层 |
| `src/styles.css` | 整个应用所有布局和组件样式，约 64 KB | 固定栏宽、全局耦合、溢出难定位 | 拆分为 tokens/layout/library/workspace/canvas/settings |

### 1.3 后端模块清单与当前职责

| 模块 | 当前职责与问题 | 目标方向 |
|---|---|---|
| `lib.rs` | 同时包含大量类型、Tauri command 薄壳、模块注册和巨量测试 | 仅保留 AppState、命令注册、公共类型导出；测试移模块 |
| `parser.rs` | V1 DocumentIR 构建、文本/Markdown/DOCX/PDF 入口、HTML 与启发式 role | 降级为格式入口和 V1 migration adapter；新识别直接进入 DocumentIRV2 |
| `pdf_facts_shadow.rs` | PDF glyph、vector、image、geometry facts | 保留并更名为正式 `pdf_facts`，取消 shadow 语义 |
| `pdf_ingest/*` | line、region、reading order、table、OCR merge、坐标 | 保留，是本地 V2 几何识别基础 |
| `pdf_geometry.rs` | PDFium 文字与 block 几何提取 | 与正式 V2 facts 合并，减少双重 PDF 解析 |
| `docx_facts_shadow.rs` / `docx_ingest/*` | OOXML facts、关系、Drawing、表格和 DOCX V2 | 保留，统一进入 DocumentIRV2 |
| `authoring_pipeline.rs` | 超大 V1 split/authoring 启发式链，包含大量字符串规则 | 冻结为 V1 兼容；新任务不再以它为主识别器 |
| `ielts_grammar/*` | Phase 4 语法层，从 V1 candidate 和 physical lines 构建 V2 | 重构为直接消费 DocumentIRV2 的本地识别器 |
| `auto_pipeline.rs` | 本地解析、视觉、云端 outline、答案候选、报告、清理的大型编排 | 拆成 durable processing queue + local/cloud/reconcile workers |
| `llm_gateway.rs` | OpenAI-compatible/Ollama 请求、JSON 提取、重试、视觉/outline | 保留低层 HTTP；移除业务 prompt，增加完整候选和修复协议 |
| `llm_suggestions.rs` | V1 group repair input、云端 outline input、候选落盘 | 拆成 versioned skill bundle、candidate validator、reconcile proposal |
| `llm_commands.rs` | Profile 管理、题组建议、应用建议 | Profile 保留；旧 V1 group suggestion 只兼容迁移，随后删除 |
| `artifact_store.rs` | V2 job 目录、不可变 revision、patch 文件、hash、锁 | 当前过重且产生大量文件；改为题库 current DS + 有界恢复快照 |
| `job_store.rs` | `job.json` 为事实源，同时 best-effort 双写题库 DB | 取消双事实源；ProcessingJob 只在数据库中保存 |
| `db.rs` | legacy `exams` + `library_items` + revisions + ingest_jobs 等双模型 | 迁移为单一 LibraryRepository；删除 legacy exams 双写 |
| `library_commands.rs` | Job/Writing 与 DB 双写；元数据再回写 JSON | 改为单向 repository transaction，不再 DB↔JSON 双写 |
| `cleanup.rs` | 删除部分中间文件，但仍保留 job、authoring、project、source review、uploads、exports | 以 canonical DS 和被引用资产为保留集合，其余按 TTL 清理 |
| `authoring_v2_commands.rs` | Session、patch、revision、publish readiness、资源预览 | 拆为 editor service、canonical repository、publish preflight |
| `authoring_review.rs` / `source_review.rs` | 技术 review 状态和来源审核 | 转为后台 `ActionableIssue`，只在工作区呈现具体问题 |
| `reading_source_v2.rs` | 从 IeltsAuthoringIRV2 编译 Runtime V2 并检查 slot closure | 保留，成为唯一 Reading runtime compiler |
| `reading_runtime_v2.rs` | attempt/scoring/runtime 逻辑 | 保留，不进入作者端 UI |
| `nas_package_v2.rs` | V2 NAS staging/manifest/commit | 保留最低正确性，简化调用和用户文案 |
| `runtime_validation.rs` / `authoring_validation.rs` | V1/V2 校验 | 合并成 typed preflight，减少字符串错误和重复门禁 |
| `preview_commands.rs` | 生成另一个预览产物 | WYSIWYG 后大部分删除；预览直接读 canonical DS |
| `export_*` | V1 JS、pack、writing、NAS 多套发布 | Reading 主线只保留 V2 compile + NAS publish；V1 一版迁移后移除 |
| `environment.rs` / `diagnostics.rs` | 环境预检和诊断设置 | 保留后台；普通设置页只显示“可用/不可用”和修复建议 |

### 1.4 当前代码的正向基础

本轮不是推倒重写。以下代码应明确复用：

- `DocumentIRV2` 的 glyph/span/line/region/vector/table/asset/reading-order 模型。
- `pdf_ingest` 中的坐标统一、行构建、区域构建、表格检测和 OCR merge。
- `IeltsAuthoringIRV2` 的 task、response group、option bank、answer slot、ContentDoc、asset 等语义模型。
- `ExamCanvasV2` 的 student/author 双模式和表格、图形、热点、答案槽渲染框架。
- `authoringV2Patches.ts` 的细粒度 patch 概念。
- `runtimeViewModelV2.ts` 和 `reading_source_v2.rs` 的“同一作者稿生成预览和运行时”方向。
- `llm_gateway.rs` 已有的请求超时、部分重试、Retry-After、平衡 JSON 提取和 OpenAI-compatible 路由。
- `nas_package_v2.rs` 已有的 staging/commit 机制；只需隐藏技术细节，不应破坏原子发布。

### 1.5 当前 P0/P1/P2 缺口矩阵

| ID | 优先级 | 缺口 | 用户影响 | 根因 |
|---|---:|---|---|---|
| UX-001 | P0 | 入口与流程页面过多 | 用户不知道该点哪里、哪一步才是最终结果 | 历史 Phase 页面直接成为产品导航 |
| UX-002 | P0 | 固定三栏/多栏 CSS 导致溢出和裁切 | 1100px 窗口、Windows 缩放、长文本下不可用 | 大量固定 240/360/380/420px 列宽，主 surface 还设置 overflow hidden |
| UX-003 | P0 | 编辑、预览、题库详情不是同一界面 | 改完仍不确定学生端效果 | ExamCanvas、Tiptap、UnifiedPreview、V1 HTML 多套 renderer |
| REC-001 | P0 | 新本地识别仍绕回 V1 candidate | 简单题型也可能缺题干/选项 | DocumentIRV2 几何没有直接驱动 QuestionBlock |
| REC-002 | P0 | 题号/题干/选项仍以行序和字符串为主 | 题号独立一行、折行、多栏时丢失 | anchor/prompt/option run 是一维 line-first |
| REC-003 | P0 | 复杂表格/流程图语义结构无法完整闭包 | 题面信息丢失 | semantic line 没有直接消费 physical table/visual object |
| CLD-001 | P0 | 云端只返回 outline，不返回完整可渲染 DS | 无法作为真正第二识别候选 | CloudReadingOutlineV1 contract 太窄 |
| CLD-002 | P0 | 云端队列在 React 页面 localStorage 中 | 退出/多窗口/崩溃后任务状态不可靠 | 后台编排没有归属到 Rust/DB |
| CLD-003 | P0 | Prompt/Skill 硬编码在 Rust | 难版本化、测试、回滚和同步给模型 | 没有独立 skill bundle |
| CLD-004 | P0 | 云端畸形 JSON 不能形成分组级 salvage | 用户可能只看到失败，丢掉部分正确结果 | 只有 JSON 提取，没有完整 schema repair pipeline |
| LIB-001 | P0 | job.json、authoring JSON、legacy exams、library_items 多事实源 | 状态/内容可短时分叉，bug 难定位 | best-effort 双写与回写 |
| LIB-002 | P0 | 清理后仍保留大量 job 项目文件 | 题库和 AppData 越用越复杂 | Artifact layout 以研发审计为中心 |
| BAT-001 | P0 | 批量导入本地循环串行，随后跳进第一题 | 用户看不到全部任务整体状态 | ImportWizard 是表单页，不是任务中心 |
| PUB-001 | P1 | 发布独立页面且门禁错误是长字符串 | 用户操作多、错误难理解 | 校验/发布模块与 UI 直接耦合 |
| SET-001 | P1 | 设置展示过多高级字段和不一致 Provider | 普通用户配置困难，部分配置必然运行失败 | Profile schema 比 gateway 协议更宽 |
| TEST-001 | P1 | UI E2E 主要运行 Vite + dev fallback | 不能证明真实 Tauri/Rust/SQLite/文件链路 | 测试替身复制后端过多 |
| CODE-001 | P1 | 多个 100KB-500KB 单文件 | 修改容易引发跨功能回归 | 长期追加式开发，没有按领域拆分 |
| OBS-001 | P2 | 用户主界面暴露过多置信度、hash、source review 和内部状态 | 噪声大，真实错误反而不突出 | 研发诊断直接进入产品表面 |

---

## 2. 用户需求转化为不可妥协的验收标准

### 2.1 导入与任务管理

- 用户在题库首页点击“导入”，一次选择 1-N 份 PDF/DOCX。
- 文件选中后立即建立 N 条题库行，不需要填写 category、frequency、tags、parse mode 等必填表单。
- 标题默认来自文件名，可在工作区直接修改。
- 每一行显示一个简短阶段：`排队中 / 本地识别 / 云端识别 / 合并结果 / 待检查 / 可发布 / 失败`。
- 本地识别和云端识别在单个题目维度并行；批量任务由后端限流，不冻结 UI。
- 用户随时可以打开处理中的题目；已有本地结果时先显示本地草稿，云端结果回来后在当前界面增量提示差异。
- 用户返回题库后，任务继续运行；应用重启后可以恢复未完成任务或明确标记可重试。

### 2.2 本地识别

- 新任务必须直接以 `DocumentIRV2` 为输入，不能先压平为 V1 block 再作为主要识别依据。
- 单选、多选、TFNG、YNNG、Matching 的 Ready 条件包含完整题干和完整选项，而不仅是题型和题号。
- 任一简单题 `prompt == empty` 必须产生 blocker，不允许显示“可发布”。
- 题号独立一行、题干折行、选项标签与正文分行、双栏、跨页时仍能通过几何邻接恢复。
- 表格、流程图、diagram 无法完全结构化时必须保留源视觉区域和答案槽覆盖层，不允许丢题面。

### 2.3 云端识别

- 云端接收 PDF 或分页图像、版本化 skill/prompt、IELTS 题型定义和完整输出 JSON Schema。
- 云端必须返回完整 `CloudRecognitionCandidateV1`，至少包含 passage、task groups、题干、选项、option bank、answer slots、source evidence；不能只返回 range/kind outline。
- 原始输出经过本地 JSON 提取、别名归一化、JSON Schema 校验、语义校验；失败时最多触发一次受约束修复请求。
- 修复后仍失败时，按 task group 分块保留可验证部分；前端只显示“云端有 2 个题组未采用”，不显示原始 JSON parse error。
- 第二次“校对”调用输出 `ReconciliationProposalV1`，只指出局部差异和建议，不直接写权威稿。

### 2.4 所见即所得编辑

- 导入完成或本地草稿可用后，打开的就是最终 IELTS 风格界面。
- Reading 左侧是 passage，右侧是 questions；用户不需要切换“编辑/预览”。
- 点击文字即可原位编辑；点击选项可修改文字、增加、删除和排序；表格单元格可直接编辑；答案槽可定位。
- 编辑后当前页面立即更新，因为页面直接读取同一 canonical DS。
- 保存动作修改 canonical DS，不修改生成 JS；发布时编译 JS/NAS。
- 普通编辑只显示保存状态：`正在保存 / 已保存 / 保存失败`，不显示 revision、hash、schema 名称。

### 2.5 题库与发布

- 题库只呈现每个题目的一个当前版本 DS。
- 历史内部 patch、临时 OCR、LLM 原始输入输出、预览 HTML 不作为题库条目展示。
- 题库支持多选后“发布到 NAS”；工作区也有一个“发布”主按钮。
- 发布检查只向用户展示可执行问题，如“第 8 题没有答案”“第 17 题缺少 B 选项”。
- 成功发布后自动清理未被 DS 引用的过程文件。

### 2.6 设置

- 默认只展示：模型服务地址、模型名、API Key、测试连接、是否启用云端识别。
- Provider 只展示当前真正支持的协议；高级超时等收进“高级设置”。
- `forceJson` 在产品逻辑中固定开启，不给普通用户复选框。
- 环境预检只在有错误时出现一条摘要；完整诊断放到开发者模式。

---
## 3. 目标信息架构与用户流程

### 3.1 路由收敛

目标路由只保留：

```ts
export type RouteName =
  | "library"
  | "workspace"
  | "settings";

/library
/items/:itemId
/settings
```

导入不是一个独立页面，而是题库页上的 drawer/modal：

```text
/library
  ├── [导入 PDF/DOCX]
  ├── 搜索
  ├── 状态筛选（全部/处理中/待检查/可发布/失败）
  ├── 题库行
  └── [发布已选择]
```

工作区：

```text
/items/:itemId
  ├── 返回题库
  ├── 标题（原位编辑）
  ├── 保存状态
  ├── [查看原文件]
  ├── [问题 3]
  ├── [发布]
  └── ExamCanvas
       ├── Passage pane
       └── Questions pane
```

### 3.2 单文件流程

```text
选择 PDF
  -> create ProcessingItem + LibraryItem shell
  -> copy source to transient workspace
  -> start local worker and cloud worker concurrently
  -> local candidate available
       -> create initial Canonical DS
       -> workspace becomes openable
  -> cloud candidate available
       -> validate / repair / salvage
  -> reconciliation
       -> safe additions applied automatically
       -> conflicts become localized issues
  -> user edits final rendered canvas
  -> publish from workspace or library
  -> compact current DS
  -> clean transient artifacts
```

### 3.3 批量流程

```text
选择 50 份 PDF
  -> 一次事务建立 50 条 library item + processing job
  -> 题库立即显示 50 行
  -> 后端调度：
       local concurrency = min(cpu_count - 1, 3)
       cloud concurrency = configured 1-3
       PDF rendering concurrency = 1-2
  -> 每一行独立更新 stage/progress/action count
  -> 打开任意行不影响其他任务
  -> 关闭应用后未完成 job 保持 queued/running-interrupted
  -> 下次启动将 running-interrupted 改为 queued 或 action_required
```

### 3.4 普通用户永远不需要看到的词

以下词汇只保留在日志、开发者模式或代码中：

```text
DocumentIRV2
IeltsAuthoringIRV2
SourceAnchor
ResponseGroup
AssetManifest
SHA-256
CAS
Revision conflict
LLM JSON parse failed
CloudReadingOutlineV1
QualityReportV2
```

普通用户看到：

```text
正在识别
云端识别完成
第 11 题题干需要确认
第 18 题选项不完整
已保存
可以发布
```

---

## 4. 唯一权威数据：Canonical Exam DS

### 4.1 决策

短期不再发明第三套 schema。将现有 `IeltsAuthoringIRV2` 作为 Canonical DS 的实现基础，并在业务代码中使用别名：

```rust
pub type CanonicalExamDsV1 = IeltsAuthoringIRV2;
```

```ts
export type CanonicalExamDsV1 = IeltsAuthoringIRV2;
```

这样可以直接复用：

- passage ContentDoc
- taskGroups
- responseGroups
- optionBank
- answerSlots
- answerKey
- assets
- sourceAnchors
- `reading_source_v2.rs` runtime compiler
- `ExamCanvasV2`

不建议立刻把磁盘 schemaVersion 改名为 `CanonicalExamDSV1`，否则会引发作者端、NAS、JSON Schema 和 fixture 的无价值迁移。产品和代码层先统一“Canonical DS”概念；等 V1 链路删除后再评估 schema 重命名。

### 4.2 Canonical DS 与派生产物关系

```text
Canonical DS                   权威，可编辑，题库保存
  |
  +-> RuntimeViewModelV2       内存投影，预览使用
  |
  +-> ReadingExamSourceV2      发布编译产物
  |
  +-> NAS JS / manifest/assets 发布打包产物
```

禁止以下反向写入：

```text
NAS JS ----------------X----> Canonical DS
Preview HTML ----------X----> Canonical DS
Cloud raw JSON --------X----> Canonical DS
```

### 4.3 当前稿的最小数据库表示

```sql
CREATE TABLE library_items_v2 (
    id                   TEXT PRIMARY KEY,
    modality             TEXT NOT NULL CHECK (modality IN ('reading','listening','writing')),
    title                TEXT NOT NULL,
    status               TEXT NOT NULL,
    current_edit_version INTEGER NOT NULL DEFAULT 1,
    canonical_ds_json    TEXT,
    source_asset_id      TEXT,
    created_at           TEXT NOT NULL,
    updated_at           TEXT NOT NULL,
    deleted_at           TEXT
);

CREATE TABLE processing_jobs_v2 (
    id                   TEXT PRIMARY KEY,
    library_item_id      TEXT NOT NULL REFERENCES library_items_v2(id),
    source_asset_id      TEXT NOT NULL,
    stage                TEXT NOT NULL,
    local_status         TEXT NOT NULL,
    cloud_status         TEXT NOT NULL,
    reconcile_status     TEXT NOT NULL,
    progress_json        TEXT NOT NULL DEFAULT '{}',
    actionable_count     INTEGER NOT NULL DEFAULT 0,
    last_error_code      TEXT,
    retry_count          INTEGER NOT NULL DEFAULT 0,
    lease_owner          TEXT,
    lease_expires_at     TEXT,
    created_at           TEXT NOT NULL,
    updated_at           TEXT NOT NULL
);
```

### 4.4 有界恢复，而不是每次按键永久建 revision 文件

当前 `artifact_store.rs` 会为每次保存建立不可变 artifact、meta 和 patch 文件。长期编辑会生成大量文件。目标策略：

- 数据库保存 `canonical_ds_json + current_edit_version`。
- 每次文本输入先更新前端内存，停止输入 400-600ms 后保存。
- 每 30 秒或每 20 次成功保存更新一个隐藏的 `last_good_snapshot`。
- 只保留当前 DS、上一个可恢复快照和最近 100 条小型 edit journal；成功关闭工作区后可压缩 journal。
- 发布历史只保存发布记录和输出位置，不复制整套 authoring workspace。

```sql
CREATE TABLE library_item_recovery_v1 (
    library_item_id TEXT PRIMARY KEY,
    edit_version    INTEGER NOT NULL,
    snapshot_json   TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE TABLE editor_journal_v1 (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    library_item_id TEXT NOT NULL,
    base_version    INTEGER NOT NULL,
    command_json    TEXT NOT NULL,
    created_at      TEXT NOT NULL
);
```

这不违背“题库只保留一个 DS”：恢复快照是隐藏的崩溃恢复基础设施，不是第二个用户可见题目版本，也不参与组卷选择。

### 4.5 资源与源文件策略

区分三类文件：

| 类型 | 示例 | 生命周期 |
|---|---|---|
| Canonical assets | passage 图片、diagram crop、必须随题发布的资源 | 与 DS 同生命周期，不能清理 |
| Source evidence | 原 PDF/DOCX | 默认保留到首次确认；之后按设置保留或删除 |
| Transient artifacts | page render、raw LLM response、DocumentIR、compare report、临时 preview | 成功合并后 TTL 清理 |

默认产品策略建议：

```text
“保留原文件以便后续核对” = 开启
```

原因是如果立即只保留 DS，一旦用户后来发现题干错漏，将失去本地来源对照。用户可以在设置中关闭；关闭后在第一次人工确认或成功发布后删除 source evidence。

---

## 5. 后端目标架构：持久化任务调度而不是页面内编排

### 5.1 新模块布局

```text
src-tauri/src/
  processing/
    mod.rs
    queue.rs
    scheduler.rs
    worker.rs
    state.rs
    events.rs
    recovery.rs

  recognition/
    mod.rs
    candidate.rs
    local/
      mod.rs
      document.rs
      question_blocks.rs
      question_numbers.rs
      stems.rs
      options.rs
      matching.rs
      completion.rs
      visual_stimulus.rs
      reliability.rs
    cloud/
      mod.rs
      skill_bundle.rs
      request.rs
      response.rs
      validate.rs
      repair.rs
      salvage.rs
    reconcile/
      mod.rs
      alignment.rs
      field_compare.rs
      merge.rs
      report.rs

  library/
    mod.rs
    repository.rs
    migration.rs
    assets.rs
    cleanup.rs

  editor/
    mod.rs
    commands.rs
    apply.rs
    validation.rs

  publish/
    mod.rs
    compile.rs
    preflight.rs
    nas.rs
```

### 5.2 任务状态模型

```rust
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StageStatus {
    NotStarted,
    Queued,
    Running,
    Succeeded,
    ActionRequired,
    Failed,
    Cancelled,
    Interrupted,
}

#[derive(Serialize, Deserialize)]
struct ProcessingItemV1 {
    job_id: String,
    library_item_id: String,
    stage: ProcessingStage,
    local: StageResult,
    cloud: StageResult,
    reconcile: StageResult,
    progress: ProcessingProgress,
    actionable_count: u32,
    updated_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
enum ProcessingStage {
    Queued,
    PreparingSource,
    LocalRecognition,
    CloudRecognition,
    Reconciling,
    ReadyForReview,
    ReadyToPublish,
    Failed,
}
```

### 5.3 Tauri 命令收敛

```rust
#[tauri::command]
async fn import_files(input: ImportFilesInput) -> CommandResult<Vec<LibraryItemSummaryV2>>;

#[tauri::command]
async fn list_library_items(filter: LibraryFilterV2) -> CommandResult<Vec<LibraryItemSummaryV2>>;

#[tauri::command]
async fn get_workspace_item(item_id: String) -> CommandResult<WorkspaceItemV1>;

#[tauri::command]
async fn apply_editor_commands(input: ApplyEditorCommandsInput) -> CommandResult<ApplyEditorCommandsResult>;

#[tauri::command]
async fn retry_processing(item_id: String, stage: Option<ProcessingStage>) -> CommandResult<()>;

#[tauri::command]
async fn publish_library_items(input: PublishLibraryItemsInput) -> CommandResult<PublishBatchResult>;

#[tauri::command]
async fn save_app_settings(input: AppSettingsV2) -> CommandResult<AppSettingsV2>;
```

现有几十个命令在一个兼容周期内仍可注册，但新 UI 不再直接调用 V1 `split`、`document review`、`generate preview assets`、`apply LLM suggestion` 等命令。

### 5.4 后端事件

```rust
app.emit("processing://item-updated", ProcessingItemEvent {
    library_item_id,
    stage,
    local_status,
    cloud_status,
    reconcile_status,
    progress_percent,
    actionable_count,
    display_message,
})?;
```

前端只订阅这一种事件，不在页面里自己维护云端队列、lease 和全局 window promise。

### 5.5 并发调度伪代码

```rust
async fn run_processing_job(job_id: JobId, ctx: AppContext) {
    let permit = ctx.job_semaphore.acquire().await?;
    let job = ctx.repo.claim_job(job_id, ctx.worker_id, lease(10_minutes))?;

    ctx.repo.update_stage(job_id, PreparingSource)?;
    let prepared = prepare_source(&job).await?;

    // 两条主识别链并发。云端失败不能取消本地链。
    let local_future = async {
        let _permit = ctx.local_semaphore.acquire().await?;
        // PDF/DOCX 解析和几何聚类属于 CPU/阻塞文件 I/O，不能占用 Tauri async runtime。
        tokio::task::spawn_blocking({
            let prepared = prepared.clone();
            move || run_local_recognition_blocking(&prepared)
        }).await?
    };

    let cloud_future = async {
        if !ctx.settings.cloud_enabled {
            return Ok(CloudOutcome::Disabled);
        }
        let _permit = ctx.cloud_semaphore.acquire().await?;
        run_cloud_recognition(&prepared, &ctx.skill_bundle).await
    };

    let (local_result, cloud_result) = tokio::join!(local_future, cloud_future);

    ctx.repo.persist_candidate(job_id, CandidateKind::Local, local_result)?;
    ctx.repo.persist_candidate(job_id, CandidateKind::Cloud, cloud_result)?;

    let canonical = match (&local_result, &cloud_result) {
        (Ok(local), Ok(cloud)) => reconcile(local, cloud)?,
        (Ok(local), Err(_)) => canonical_from_local(local),
        (Err(_), Ok(cloud)) => canonical_from_cloud_with_review(cloud),
        (Err(local_err), Err(cloud_err)) => {
            return ctx.repo.fail_job(job_id, aggregate(local_err, cloud_err));
        }
    };

    ctx.repo.save_canonical_ds(job.library_item_id, canonical.ds)?;
    ctx.repo.save_actionable_issues(job.library_item_id, canonical.issues)?;
    ctx.repo.complete_job(job_id, canonical.readiness)?;
    ctx.cleanup.schedule(job_id, CleanupReason::CandidateMerged)?;
    drop(permit);
}
```

### 5.6 为什么不能继续让 `UnifiedPreview.tsx` 管云端队列

当前做法依赖：

- localStorage queue
- localStorage lease
- `window.__IELTS_CLOUD_REVIEW_WORKER__`
- 页面加载后调用 scheduler
- CustomEvent 更新当前页面

它无法可靠处理：

- 应用在云端请求过程中退出；
- 用户一直停留在题库页，没有加载 UnifiedPreview 模块；
- 多窗口竞争；
- WebView localStorage 不可用；
- 批量 100 份文件；
- 网络断开后延迟重试；
- 后端 job 已更新但前端 lease 仍未释放。

因此迁移到 Rust/SQLite 是本计划的 P0，不是“架构美化”。

---

## 6. 本地 V2 几何识别重构

### 6.1 当前断点

现有 `DocumentIRV2` 已经有足够丰富的物理事实，但 `ielts_grammar` 仍然从 V1 `questionGroupCandidates` 开始，并把 physical layer 主要转为 `SemanticLine`。结果是：

- 题号、题干、选项先被压成一维行序；
- table/region/vector 不能直接成为题面结构；
- 题号独立行或选项正文折行时，字符串规则容易失败；
- V1 candidate 的错误会限制 V2 grammar 的搜索范围。

### 6.2 新的中间层：Question Layout Graph

```rust
struct QuestionLayoutGraphV1 {
    document_id: String,
    pages: Vec<PageLayoutGraph>,
    instruction_zones: Vec<InstructionZoneCandidate>,
    question_blocks: Vec<QuestionBlockCandidateV1>,
    option_banks: Vec<OptionBankCandidateV1>,
    visual_stimuli: Vec<VisualStimulusCandidateV1>,
    unassigned_evidence: Vec<UnassignedEvidence>,
}

struct QuestionBlockCandidateV1 {
    candidate_id: String,
    question_number: u32,
    number_anchor: SourceAnchorV2,
    stem_node_ids: Vec<String>,
    stem_text: String,
    option_run: Option<OptionRunCandidateV2>,
    shared_option_bank_ref: Option<String>,
    visual_object_refs: Vec<String>,
    source_coverage: f64,
    boundary_confidence: f64,
    ambiguities: Vec<RecognitionIssueCode>,
}
```

### 6.3 数据处理顺序

```text
DocumentIRV2
  -> page role segmentation
  -> instruction zone detection
  -> question number token detection
  -> geometric question-block expansion
  -> local option-run detection
  -> shared option-bank detection
  -> task semantic classification
  -> ContentDoc/TaskGroup compilation
  -> hard completeness validation
```

题型分类不得先于题面边界恢复。先恢复“这道题包含哪些物理节点”，再判断这是 single choice、matching 还是 completion。

### 6.4 页角色与区域角色

每个 region 增加推断角色，但不修改原始 facts：

```rust
enum SemanticRegionRole {
    Passage,
    QuestionInstruction,
    QuestionPrompt,
    OptionRun,
    SharedOptionBank,
    CompletionStimulus,
    AnswerKey,
    HeaderFooter,
    Unknown,
}
```

打分特征：

```text
instruction lexical signature
Questions N-M range
font size / bold / spacing
region x/y location
question number density
option label sequence
paragraph labels A-G
repeated header/footer position
page number position
nearby table or figure object
```

### 6.5 题号识别：从 line-first 改为 token/geometry-first

当前 anchor 主要从整行开头解析数字。新流程：

```rust
fn detect_question_number_tokens(
    page: &DocumentPageV2,
    expected_range: Option<RangeInclusive<u32>>,
) -> Vec<QuestionNumberToken> {
    page.spans
        .iter()
        .filter_map(parse_small_numeric_token)
        .filter(|t| expected_range.map_or(true, |r| r.contains(&t.value)))
        .filter(|t| !inside_header_footer(t))
        .filter(|t| !looks_like_year_or_measurement(t))
        .map(|t| score_number_token(t, page))
        .filter(|t| t.score >= 0.55)
        .collect()
}
```

`score_number_token` 至少包含：

```text
+0.30 在 instruction 声明范围内
+0.20 与前后题号构成单调序列
+0.15 位于问题区域左缘/空位附近
+0.10 与右侧或下一行正文相邻
+0.10 字号与其他题号一致
-0.30 位于页脚/页码区域
-0.25 左右是单位、年份或百分号
-0.20 位于 passage 连续正文内部
```

### 6.6 题干扩展算法

```rust
fn assemble_question_stem(
    number: &QuestionNumberToken,
    next_number: Option<&QuestionNumberToken>,
    graph: &PageLayoutGraph,
) -> StemCandidate {
    let search_region = geometric_interval(number, next_number, graph);

    let same_row = graph.text_nodes_right_of(number)
        .filter(within_baseline_tolerance)
        .filter(not_option_label);

    let wrapped = graph.text_lines_below(same_row.or(number))
        .take_while(|line| {
            !is_next_question_number(line)
            && !starts_valid_option_run(line)
            && same_column_or_hanging_indent(line)
            && vertical_gap_below_threshold(line)
        });

    let nodes = reading_order_sort(chain(same_row, wrapped));
    let text = join_preserving_punctuation(nodes);

    StemCandidate {
        text,
        node_ids,
        coverage: assigned_visible_text_ratio(search_region),
        boundary_confidence: boundary_score(...),
    }
}
```

必须支持这些形态：

```text
5 Which ...                 同行

5
Which ...                   题号独立一行

5 Which ...
  continues ...             折行

5        Which ...          大缩进

5 Which ...                 题干跨页
(page break)
continues ...
A ...
```

### 6.7 选项识别

选项不再只要求一行形如 `A text`。先找 label token，再收集其右侧和后续悬挂缩进行：

```rust
struct OptionLabelToken {
    label: String,
    bbox: Rect,
    node_id: String,
}

fn assemble_option(
    label: &OptionLabelToken,
    next_label: Option<&OptionLabelToken>,
    block: &LayoutBlock,
) -> OptionCandidate {
    let inline = text_right_of(label, baseline_tolerance = 0.6);
    let continuations = lines_below(inline.or(label))
        .take_while(|line| {
            !is_next_option_label(line)
            && !is_next_question(line)
            && hanging_indent_matches(line, inline)
        });
    ...
}
```

一个合法 option run 的证据：

- label 按 A/B/C/D 或 i/ii/iii 连续；
- label x 坐标形成稳定列；
- 每个 label 有非空正文；
- 整组位于同一个 question block 或 option-bank region；
- 不把 `Paragraph A`、文章首字母 A、Section B 当作答案选项。

### 6.8 简单题型硬闭包

```rust
fn validate_basic_task(task: &TaskCandidate) -> Vec<ActionableIssue> {
    match task.kind {
        SingleChoice | MultipleChoice => {
            require_nonempty_prompt(task);
            require_expected_option_labels(task);
            require_nonempty_option_text(task);
            require_unique_option_labels(task);
            require_question_source_coverage(task, 0.92);
        }
        TrueFalseNotGiven | YesNoNotGiven => {
            require_every_statement_nonempty(task);
            require_exact_fixed_response_set(task);
        }
        MatchingHeadings | MatchingInformation | MatchingFeatures | Classification => {
            require_every_item_prompt(task);
            require_shared_option_bank(task);
            require_nonempty_bank_options(task);
        }
        _ => {}
    }
}
```

下列问题直接阻止 Ready：

```text
QUESTION_PROMPT_MISSING
QUESTION_PROMPT_BOUNDARY_AMBIGUOUS
OPTION_LABEL_MISSING
OPTION_TEXT_MISSING
OPTION_RUN_INCOMPLETE
SHARED_OPTION_BANK_MISSING
SIGNIFICANT_SOURCE_TEXT_UNASSIGNED
```

### 6.9 Matching Headings 与 passage A-G

Matching Headings 必须显式分离：

```text
Passage labels: Paragraph A-G       -> passage.paragraphMap
List of Headings i-x                -> task.optionBank
Questions 14-20 / Paragraph A-G     -> response groups / slots
```

分类顺序：

```rust
if has_list_of_headings_title
   && has_roman_option_run
   && has_paragraph_targets {
    task_type = MatchingHeadings;
}
```

在 passage 构建时，任何被 `option_bank_candidate` 消费的 region 都不得进入 passage ContentDoc。

### 6.10 表格、流程图和图片

#### 表格

优先使用 `DocumentIRV2.pages[].tables[]`：

```rust
fn compile_table_stimulus(table: &PhysicalTableV2, doc: &DocumentIRV2) -> ContentNodeV2 {
    ContentNodeV2::Table {
        rows: table.rows.map(|r| TableRow {
            cells: physical_cells_in_row(r).map(|cell| TableCell {
                row_span: cell.row_span,
                col_span: cell.col_span,
                children: content_from_region_ids(cell.content_region_ids),
                ...
            })
        })
    }
}
```

不要再按题号创建“一题一行一列”的假表格。

#### 流程图/diagram

短期优先 Hybrid：

```json
{
  "type": "diagram",
  "id": "task-27-31-visual",
  "assetId": "source-crop-...",
  "display": { "widthPercent": 100, "align": "center" },
  "hotspots": [
    {
      "hotspotId": "q27-place",
      "slotId": "q27",
      "normalizedRect": [0.41, 0.28, 0.22, 0.07]
    }
  ]
}
```

后台可以继续尝试 graph reconstruction，但失败不能导致题面缺失。

### 6.11 未分配证据账本

识别完成后，对问题区域中的每个可见 line/region/table/visual object 标记：

```text
assigned_to_instruction
assigned_to_prompt:q5
assigned_to_option:q5:A
assigned_to_option_bank:task2
assigned_to_stimulus:task3
ignored_header_footer
unassigned
```

若题组范围内存在大面积 `unassigned`，特别是题号之间的正文，生成 blocker。这样能检测“题型对了但整句题干丢了”。

---
## 7. 云端完整识别、Prompt/Skill 与 JSON 修复链

### 7.1 当前实现与目标差距

当前云端整卷 contract 主要返回：

```text
title
groups[].range
groups[].kind
groups[].layoutHint
groups[].questionIds
groups[].notesText
answerKey
confidence
warnings
```

并明确限定为“comparison only”。这无法承担用户要求的第二条完整 PDF 转题链，因为没有 passage ContentDoc、逐题 prompt、options、option bank、answer slot、visual stimulus 和可渲染结构。

### 7.2 版本化 Skill Bundle

新增仓库目录：

```text
recognition/skills/ielts-reading-v1/
  manifest.json
  system.md
  conversion-rules.md
  task-taxonomy.json
  output.schema.json
  renderer-contract.md
  error-policy.md
  examples/
    single-choice.json
    multi-choice-shared.json
    tfng.json
    matching-headings.json
    matching-features.json
    note-completion.json
    table-completion.json
    visual-hybrid.json
```

`manifest.json`：

```json
{
  "skillId": "ielts-reading-conversion",
  "version": "1.0.0",
  "outputSchema": "CloudRecognitionCandidateV1",
  "schemaSha256": "...",
  "minimumClientVersion": "0.2.0",
  "supportedModalities": ["reading"],
  "supportedInput": ["application/pdf", "image/png", "image/jpeg"]
}
```

这里的 hash 只用于程序内部确定“请求使用哪个 schema 版本”，不在普通 UI 展示。

### 7.3 CloudRecognitionCandidateV1

```rust
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CloudRecognitionCandidateV1 {
    schema_version: String,
    skill_version: String,
    source_document: CloudSourceDocument,
    passage: Option<CloudPassageCandidate>,
    task_groups: Vec<CloudTaskGroupCandidate>,
    answer_slots: BTreeMap<String, CloudAnswerSlotCandidate>,
    answer_key_candidates: BTreeMap<String, CloudAnswerCandidate>,
    assets: Vec<CloudAssetReference>,
    unresolved_regions: Vec<CloudUnresolvedRegion>,
    warnings: Vec<CloudWarning>,
}

struct CloudTaskGroupCandidate {
    candidate_id: String,
    display_range: QuestionNumberExpressionV2,
    task_type: TaskTypeV2,
    instructions: Vec<ContentNodeV2>,
    stimulus: Option<Vec<ContentNodeV2>>,
    option_bank: Option<OptionBankV2>,
    response_groups: Vec<ResponseGroupV2>,
    evidence: Vec<CloudEvidence>,
    confidence: f64,
}

struct CloudEvidence {
    page_index: u32,
    quote: String,
    bbox: Option<[f64; 4]>,
    evidence_kind: String,
}
```

关键约束：

- 每个 task group 必须有 question range、task type、instructions 和 source evidence。
- 每个 choice/matching response 必须有完整 option 或可解析 option bank。
- 每个 slot 必须归属一个 response group。
- 不确定时使用 `unresolvedRegions`，不得发明文本。
- 不要求模型生成 JS 或 HTML。
- 云端 `ContentNodeV2` 只允许安全节点子集：paragraph、heading、text、list、table、answer_slot、option_bank、figure/diagram reference；不允许任意 HTML。

### 7.4 请求体

```rust
struct CloudRecognitionRequestV1 {
    request_id: String,
    skill_bundle: SkillBundleDescriptor,
    source: SourceUploadDescriptor,
    page_manifest: Vec<PageDescriptor>,
    local_hints: Option<LocalRecognitionHints>,
    output_contract: JsonSchemaDescriptor,
}
```

建议默认给模型 PDF；若服务端不接受 PDF，则本地一次性渲染全部页并上传图像。不要重复为“答案识别”和“整卷识别”分别渲染同一 PDF。

`local_hints` 只能包含低风险事实：

```text
page count
page dimensions
native extracted text snippets
candidate question ranges
known visual asset ids
```

不能把本地错误题干当作事实强迫模型复制。

### 7.5 System Prompt 核心约束

`system.md` 应明确：

1. 任务是视觉和文本结构转录，不是答题或改写文章。
2. 输出必须匹配给定 JSON Schema。
3. 题干和选项必须逐字保留，不得摘要、润色或补写。
4. 每个字段必须附可核对 page/quote/bbox 证据。
5. 复杂视觉题若不能可靠结构化，返回 visual region + slot placement 建议，不得只返回空位附近文字。
6. Questions X and Y 表示多个答案位；共享题干只出现一次。
7. List of Headings/List of People/List of Features 是 option bank，不进入 passage。
8. passage 和 question 的物理先后顺序不决定左右栏归属。
9. 不确定内容使用 unresolved，不得推测。
10. 不生成 JavaScript、HTML、Markdown 代码围栏或解释性文本。

### 7.6 响应处理状态机

```rust
enum CloudResponseState {
    Received,
    JsonExtracted,
    Normalized,
    SchemaValid,
    SemanticallyValid,
    RepairRequested,
    Salvaged,
    Rejected,
}
```

处理伪代码：

```rust
async fn process_cloud_response(raw: &str, ctx: &CloudContext)
    -> CloudCandidateOutcome
{
    let extracted = match extract_single_json_value(raw) {
        Ok(v) => v,
        Err(err) => return request_repair_or_fail(raw, vec![err], ctx).await,
    };

    let normalized = normalize_safe_aliases(extracted);

    let schema_errors = validate_json_schema(&normalized, ctx.output_schema);
    let semantic_errors = validate_cloud_candidate_semantics(&normalized);

    if schema_errors.is_empty() && semantic_errors.is_empty() {
        return CloudCandidateOutcome::Valid(deserialize(normalized)?);
    }

    let repaired = request_constrained_repair(
        raw,
        &normalized,
        schema_errors + semantic_errors,
        ctx,
    ).await;

    match repaired {
        Ok(value) if fully_valid(&value, ctx) => {
            CloudCandidateOutcome::Repaired(deserialize(value)?)
        }
        Ok(value) => {
            let salvage = salvage_valid_task_groups(value, ctx);
            if salvage.valid_groups.is_empty() {
                CloudCandidateOutcome::Rejected {
                    user_code: "CLOUD_RESULT_UNUSABLE",
                    internal_errors: salvage.errors,
                }
            } else {
                CloudCandidateOutcome::Partial(salvage)
            }
        }
        Err(error) => CloudCandidateOutcome::Rejected {
            user_code: "CLOUD_RESULT_UNUSABLE",
            internal_errors: vec![error],
        },
    }
}
```

### 7.7 安全归一化允许范围

本地可自动修正：

```text
"single-choice" -> "single_choice"
"TFNG" -> "true_false_not_given"
question number string "12" -> integer 12
null optional array -> []
trim label " A " -> "A"
source page from 1-based -> 0-based（仅当 request 明确声明）
```

本地不得自动补：

```text
缺失题干 -> 空字符串
缺失选项 -> 自动生成 A/B/C/D
缺失 answer slot -> 根据 range 默默补齐
缺失 evidence -> 伪造 source anchor
未知 task type -> short_answer
```

这些做法会把协议错误伪装成“合法但错误”的题目。

### 7.8 一次受约束修复请求

修复请求只包含：

```json
{
  "originalOutput": "...",
  "parsedCandidate": {},
  "validationErrors": [
    {"path":"$.taskGroups[2].responseGroups[0].options","code":"REQUIRED"}
  ],
  "outputSchema": {},
  "rules": [
    "Do not change source-transcribed text unless needed to restore omitted content from the supplied pages.",
    "Return JSON only.",
    "Do not invent missing evidence."
  ]
}
```

最多一次，避免模型在多轮修复中逐渐改写原文。

### 7.9 分组级 Salvage

```rust
fn salvage_valid_task_groups(value: Value, ctx: &CloudContext) -> SalvageResult {
    let mut valid_groups = vec![];
    let mut rejected_groups = vec![];

    for raw_group in extract_group_array(&value) {
        match validate_one_group(raw_group, ctx) {
            Ok(group) => valid_groups.push(group),
            Err(errors) => rejected_groups.push(GroupRejection { range_hint, errors }),
        }
    }

    SalvageResult { valid_groups, rejected_groups }
}
```

前端文案：

```text
云端识别已补充 8 个题组；2 个题组未采用，请以本地结果为准。
```

不显示：

```text
serde_json error at line 1 column 3492
additionalProperties false
```

### 7.10 第二条云端校对链

第一条调用负责独立识别；第二条调用只负责对比：

```rust
struct ReconciliationReviewRequestV1 {
    source_pages: Vec<PageDescriptor>,
    local_candidate: RecognitionCandidateV1,
    cloud_candidate: CloudRecognitionCandidateV1,
    deterministic_diff: DeterministicDiffV1,
}

struct ReconciliationProposalV1 {
    proposals: Vec<FieldProposal>,
    unresolved: Vec<ReviewConflict>,
}

struct FieldProposal {
    target_id: String,
    field: String,
    operation: ProposedOperation,
    before: Value,
    after: Value,
    evidence: Vec<CloudEvidence>,
    confidence: f64,
}
```

校对模型不得重新生成整卷，只能针对 deterministic diff 中的字段提出建议。

### 7.11 Cloud Evidence Resolver 与 ID 规范化

模型给出的 page/quote/bbox 不能直接成为 Canonical `sourceAnchors`。必须本地解析：

```rust
fn resolve_cloud_evidence(
    evidence: &CloudEvidence,
    document: &DocumentIRV2,
) -> Result<Vec<SourceAnchorV2>, EvidenceResolutionError> {
    let page = document.pages.get(evidence.page_index as usize)?;
    let by_bbox = evidence.bbox
        .map(|bbox| page.nodes_overlapping(normalize_bbox(bbox)))
        .unwrap_or_default();
    let by_quote = page.find_quote_normalized(&evidence.quote);
    unique_consistent_match(by_bbox, by_quote)
}
```

- quote 与 bbox 必须指向同一或相邻源节点；
- 无法唯一解析的 evidence 只可作为 proposal，不能自动合并；
- 模型生成的 `candidateId/taskId/nodeId/slotId` 全部视为临时 ID；进入 canonical 前由本地 `IdAllocator` 重新分配，避免重复 ID、路径注入或覆盖现有节点；
- cloud raw output 只存 transient workspace，candidate 验证完成并生成摘要后按 TTL 删除。

### 7.12 大文件与上下文预算

IELTS 单篇通常页数有限，但产品不能假设所有上传都很短。Cloud request 在本地预检：

```text
<= 15 pages and below provider byte limit -> 整卷单请求
> 15 pages or above limit                -> 按 instruction/question region 分片
```

分片只允许按页和已检测题组边界拆分；最后由本地合并 candidate。禁止在 token 截断后把缺失页当作完整结果。

### 7.13 校对调用触发条件

第二次校对不是每份 PDF 无条件调用。仅在以下条件之一满足时启动：

- local/cloud task alignment 存在结构冲突；
- 基础题 prompt/option 不一致；
- 任一候选有 significant unassigned source evidence；
- answer key 候选冲突；
- local 或 cloud candidate 为 partial。

两路完全一致时直接结束，减少成本和延迟。

### 7.14 Provider 设置与协议一致性

当前 UI 可选择 `AnthropicCompatible` 和 `Custom`，但 gateway 路由只接受 `OpenAiCompatible` 和 `Ollama`。本计划第一阶段直接从普通 UI 删除不受支持项：

```text
OpenAI-compatible
Ollama（本地）
```

若以后增加 Anthropic，必须新增独立 wire adapter 和测试后再展示，不能只增加一个下拉选项。

---

## 8. 本地与云端的确定性对齐、合并和人工确认

### 8.1 两路都不是最终稿

```text
LocalRecognitionCandidateV1
CloudRecognitionCandidateV1
            |
            v
ReconciliationEngine
            |
            v
Canonical DS + Actionable Issues
```

默认优先级不是简单的“本地永远覆盖云端”或“云端置信度更高就覆盖本地”，而是字段级策略。

### 8.2 对齐键

题组对齐顺序：

1. question number set 完全相等；
2. range 高重叠且 instruction signature 一致；
3. source page/bbox 高重叠；
4. 题干 normalized similarity；
5. 无法唯一对齐则产生 conflict，不强行合并。

```rust
fn align_task_groups(local: &[TaskCandidate], cloud: &[TaskCandidate]) -> Vec<TaskAlignment> {
    bipartite_max_weight_matching(
        local,
        cloud,
        |l, c| {
            0.45 * question_number_overlap(l, c)
          + 0.20 * instruction_similarity(l, c)
          + 0.15 * page_bbox_overlap(l, c)
          + 0.15 * prompt_similarity(l, c)
          + 0.05 * task_type_match(l, c)
        },
        unique_threshold = 0.72,
        ambiguity_margin = 0.08,
    )
}
```

### 8.3 自动合并矩阵

| 字段情况 | 自动处理 | 是否提示用户 |
|---|---|---|
| 本地与云端完全相同 | 接受本地，记录 agreement | 否 |
| 本地题干为空，云端题干非空且有 source evidence | 补入云端题干，标记 `cloud_fill` | 仅在问题摘要显示“自动补全 1 处” |
| 本地题干有内容，云端多一个明显续行，证据 bbox 紧邻 | 合并续行，前提是 local source ledger 显示未分配文本 | 可撤销提示 |
| 两边题干不同且都非空 | 不自动覆盖 | 是，原位差异 |
| 本地缺 B 选项，云端有 B 且证据有效 | 补 B | 是，轻提示 |
| 两边 option label 数量不同且无法对应 | 不自动合并结构 | 是，blocker |
| task type 不同但 instruction signature 明确 | 采用 deterministic instruction result | 是，显示局部问题 |
| answer 冲突 | 不自动合并 | 是，必须确认 |
| 云端只有 outline 无正文 | 不进入 canonical | 否，内部记录 |
| cloud partial group invalid | 丢弃该 group | 题库行显示“有 1 项待检查” |

### 8.4 自动合并的最低条件

```rust
fn can_auto_fill(field: &CandidateField) -> bool {
    field.target_is_empty()
        && field.incoming_is_nonempty()
        && field.incoming_schema_valid()
        && field.evidence_resolves_to_source()
        && field.evidence_quote_matches()
        && field.confidence >= 0.88
        && !field.is_answer_key()
        && !field.changes_question_numbering()
}
```

### 8.5 ActionableIssue

```rust
struct ActionableIssueV1 {
    issue_id: String,
    library_item_id: String,
    target_id: String,
    severity: IssueSeverity,
    code: IssueCode,
    title: String,
    user_message: String,
    suggested_action: SuggestedAction,
    source_anchor: Option<SourceAnchorV2>,
    local_value: Option<Value>,
    cloud_value: Option<Value>,
    status: IssueStatus,
}
```

用户界面只展示：

```text
第 12 题题干可能缺少下一行       [查看]
第 18 题 B 选项未识别             [补充]
Questions 27-28 被识别为两个题组   [合并]
```

### 8.6 云端差异原位显示

在 `ExamCanvas` Author Mode 中：

- 云端建议新增：绿色浅底 + “采用/忽略”。
- 云端建议删除：原文字红色删除线 + “采用删除/保留原文”。
- 结构冲突：题组右上角一个问题标记，点击打开小型对比 popover。
- 不创建独立 LLM Review 页面。

### 8.7 用户编辑后的保护

用户编辑过的节点：

```text
provenanceStatus = user_edited
```

后续迟到的 cloud result 不允许自动修改。只能生成 proposal：

```rust
if canonical_node.provenance_status == UserEdited {
    return MergeDecision::ProposalOnly;
}
```

---

## 9. 所见即所得 ExamCanvas 重构

### 9.1 核心决定

只保留一套正式 renderer：

```tsx
<ExamCanvas
  ds={canonicalDs}
  mode="author" | "student"
/>
```

Author Mode 与 Student Mode 使用相同的：

- Passage 布局；
- task instructions；
- single/multiple choice；
- matching matrix；
- option bank；
- completion；
- table；
- figure/diagram/hotspot；
- answer slot；
- 响应式 CSS。

Author Mode 只额外增加轻量编辑行为和问题标记，不改变结构排版。

### 9.2 不再保留两套编辑器

当前有：

```text
ExamCanvasV2 contentEditable
AuthoringTiptapEditor
UnifiedPreview V1 form editor
LibraryExamDetail V1 read-only HTML
```

目标：

- `ExamCanvasV2.tsx` 重命名为 `src/exam-canvas/ExamCanvas.tsx`。
- `authoringTiptap.tsx` 从主流程删除。
- `UnifiedPreview` 的题目输入框全部删除。
- `LibraryExamDetail` 删除。
- V1 HTML 只在迁移适配器中临时转换为 ContentDoc，不直接渲染。

### 9.3 文本编辑不要继续使用裸 contentEditable

当前 `ExamCanvasV2` 使用 `contentEditable`、blur 提交，并在 paste 时调用已不推荐的 `document.execCommand`。这会带来：

- 中文输入法 composition 与 rerender 冲突；
- 光标跳动；
- 粘贴富文本污染；
- React reconciliation 与 DOM 状态分叉；
- browser undo 与应用 undo 不一致；
- blur 前崩溃时内容丢失。

第一版采用更简单、可靠的原位输入模式：

```tsx
function EditableTextNode({ node, editing }) {
  if (!editing) return <span onDoubleClick={start}>{node.text}</span>;
  return (
    <AutoSizeTextarea
      value={draft}
      aria-label="编辑题目文字"
      onCompositionStart={() => composing = true}
      onCompositionEnd={handleCompositionEnd}
      onChange={setDraft}
      onBlur={commit}
      onKeyDown={handleShortcut}
    />
  );
}
```

textarea 样式去边框、继承字体和行高，在视觉上仍是原位编辑。用户只是在点击后看到淡色 focus ring。

### 9.4 文本命令协议

新增比范围 patch 更直接的命令：

```ts
export type EditorCommandV1 =
  | {
      op: "set_text";
      nodeId: string;
      expectedText: string;
      text: string;
    }
  | { op: "set_option_text"; optionId: string; text: string }
  | { op: "add_option"; responseGroupId: string; afterOptionId?: string }
  | { op: "delete_option"; responseGroupId: string; optionId: string }
  | { op: "move_option"; responseGroupId: string; optionId: string; direction: "up" | "down" }
  | { op: "set_answer"; slotId: string; value: AnswerValueV2 }
  | { op: "set_title"; title: string }
  | { op: "set_table_cell_text"; cellId: string; text: string }
  | { op: "set_slot_placement"; nodeId: string; slotId: string; rect: NormalizedRect };
```

`expectedText` 用于简单乐观并发；它比字符下标更适合中文、emoji 和组合字符。

### 9.5 编辑保存链

```text
用户输入
 -> 前端内存 DS 立即更新
 -> 450ms debounce
 -> apply_editor_commands(itemId, baseVersion, commands)
 -> Rust 在事务中加载当前 DS
 -> 验证 baseVersion
 -> 应用命令
 -> 运行轻量增量校验
 -> 更新 canonical_ds_json 和 editVersion
 -> 返回已保存版本与具体问题变化
 -> 前端显示“已保存”
```

伪代码：

```rust
fn apply_editor_commands(input: ApplyEditorCommandsInput) -> Result<ApplyResult> {
    db.transaction(|tx| {
        let row = repo.get_for_update(tx, input.item_id)?;
        ensure!(row.edit_version == input.base_version, EDIT_VERSION_CONFLICT);

        let mut ds = deserialize(row.canonical_ds_json)?;
        for command in input.commands {
            apply_command(&mut ds, command)?;
        }

        let issues = validate_changed_targets(&ds, input.commands.targets());
        let next_version = row.edit_version + 1;
        repo.update_ds(tx, input.item_id, next_version, &ds)?;
        recovery.append_commands(tx, input.item_id, input.commands)?;

        Ok(ApplyResult { next_version, issues, ds_hash_internal })
    })
}
```

### 9.6 不在每次输入时重写 JS

用户所说“后面的 JS 直接删一个字符”应实现为语义等价，而不是磁盘上真的持续重写 JS：

```text
用户删一个字符
 -> canonical text node 删一个字符
 -> ExamCanvas 立即呈现
 -> 发布时 compiler 生成包含该字符变化的 JS
```

若每次输入都重写 JS，会产生：

- 生成文件可能处于半写状态；
- JS 与 DS 双事实源；
- undo 无法可靠恢复；
- schema 和运行时校验成本高；
- 多题批量编辑会频繁 I/O。

### 9.7 Reading 布局

```css
.exam-workspace-canvas {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(360px, 0.86fr);
  min-width: 0;
  height: calc(100dvh - var(--workspace-header-height));
}

.exam-passage,
.exam-questions {
  min-width: 0;
  overflow: auto;
}
```

在窄窗口：

- 默认仍保留两栏，最小可用宽度 1024；
- 小于 980 时切换顶部 tab `原文 / 题目`，不是把侧栏、导航、三栏 inspector 全部堆成一列；
- source PDF 以 overlay drawer 打开，不永久占第三栏。

### 9.8 Matching Matrix Renderer

针对用户描述的“左侧 1-6 个题干，上方 A/B/C 列，下面有 A/B/C 内容”：

```tsx
<MatchingMatrix
  prompts={items}
  options={optionBank.options}
  selected={answers}
  mode={mode}
/>
```

呈现：

```text
                         A    B    C
21  PEST                ○    ○    ○
22  Drill Down          ○    ○    ○
23  PMI                 ○    ○    ○

A  saves time
B  is visualized
C  is difficult to apply
```

每行一个 slot；若 assignment 为 per_slot，使用 radio semantics。只有明确多选才使用 checkbox。

### 9.9 复杂视觉编辑

Hybrid 图形在 Author Mode 下支持：

- 拖动裁剪边界；
- 拖动/调整 slot placement；
- 点击 slot 编辑 display label；
- 替换视觉资源；
- 切换“按原图显示 / 使用结构化重建”。

不要求老师手工重画流程图。

### 9.10 工作区 Header

只保留：

```text
[返回题库]   标题（可编辑）   本地已完成 · 云端识别中   已保存   [查看原文件] [发布]
```

高级信息通过右上角 `...`：

```text
重新运行本地识别
重新运行云端识别
查看技术日志（开发者模式）
删除题目
```

---
## 10. 前端视觉和溢出专项整改

### 10.1 当前溢出的直接技术原因

当前 `src/styles.css` 同时存在：

```css
.review-grid   { grid-template-columns: 360px minmax(0,1fr) 380px; }
.editor-grid   { grid-template-columns: 240px minmax(0,1fr) 420px; }
.llm-grid      { grid-template-columns: minmax(0,1fr) minmax(0,1fr) 320px; }
.settings-grid { grid-template-columns: minmax(460px,1.35fr) minmax(320px,.9fr) 340px; }
.metric-row    { grid-template-columns: repeat(6,1fr); }
.surface       { overflow: hidden; }
```

这些列宽在 1100px Tauri 最小窗口中无法同时容纳 workspace padding、sidebar、gap、border 和 Windows 缩放后的逻辑像素。`overflow:hidden` 又将溢出内容裁掉，而不是让布局自适应。

### 10.2 样式目录重构

```text
src/styles/
  tokens.css
  reset.css
  app-shell.css
  library.css
  workspace.css
  exam-canvas.css
  settings.css
  overlays.css
  utilities.css
```

删除单一 `src/styles.css`，过渡期只保留 imports：

```css
@import "./styles/tokens.css";
@import "./styles/reset.css";
@import "./styles/app-shell.css";
...
```

### 10.3 全局硬规则

```css
*, *::before, *::after { box-sizing: border-box; }

html, body, #root {
  width: 100%;
  min-width: 0;
  min-height: 100%;
}

:is(.app-main, .panel, .grid-child, .flex-child, .table-wrap) {
  min-width: 0;
}

:is(p, td, th, label, button, .file-name, .error-message) {
  overflow-wrap: anywhere;
  word-break: normal;
}

img, svg, video, canvas {
  max-width: 100%;
}

button, input, select, textarea {
  max-width: 100%;
}
```

主 surface 不再使用 `overflow:hidden`：

```css
.app-surface {
  overflow: clip; /* 只用于装饰层 */
}

.app-surface-content {
  min-width: 0;
  overflow: visible;
}
```

### 10.4 页面宽度和容器查询

题库和设置采用内容最大宽度：

```css
.library-page,
.settings-page {
  width: min(100%, 1440px);
  margin-inline: auto;
}
```

题目工作区使用全宽，不套大型圆角卡片：

```css
.workspace-page {
  width: 100%;
  height: 100dvh;
  background: var(--surface);
}
```

组件级响应式优先使用 container query：

```css
.library-shell { container-type: inline-size; }

@container (max-width: 760px) {
  .library-row {
    grid-template-columns: minmax(0,1fr) auto;
  }
  .library-row-meta {
    grid-column: 1 / -1;
  }
}
```

### 10.5 视觉基调

作者端不需要与 IELTS 官方品牌完全复制，但考试题面区域需要尽可能接近正式机考的信息密度和结构：

- 应用外壳：中性浅灰、少量蓝绿色强调、低阴影、8-12px 圆角；
- 题面：白色、黑灰正文、清晰分隔、少装饰；
- “与 IELTS 官方界面一致”按**信息架构、题型交互、左右栏、字号密度、题号导航和作答反馈的功能/视觉同构**验收；不复制官方商标、受版权保护的图形资产或误导性品牌标识；
- 不使用当前大面积渐变、超大圆形装饰和过多胶囊按钮；
- 正文优先系统无衬线字体；题面字号 15-17px，行高 1.55-1.7；
- 主操作按钮最多一个强调色；次操作改为文字按钮或菜单；
- 不在一个卡片里再嵌三层卡片。

### 10.6 Library 行布局

```text
[选择框] 文件标题                     [阶段状态] [需要检查 2]
          Reading · 13题 · 刚刚更新     进度条/失败说明
```

处理中：

```text
Listening to the Ocean
本地识别完成 · 云端识别中              62%
```

失败：

```text
Paper 18
本地 PDF 无法读取                      [重试]
```

普通行不要显示 hash、source path、schema、revision、错误总数技术码。

### 10.7 设置页布局

不再使用三栏固定 grid。目标：

```text
模型识别
  [启用云端识别开关]
  服务地址
  模型
  API Key
  [测试连接]

文件
  [保留原始 PDF 以便核对]

[高级设置 ▸]
```

在 720-1600px 都是一列主表单，最大宽度 720px。

### 10.8 溢出验收矩阵

必须自动截图并断言 `scrollWidth <= clientWidth + 1`：

| 视口/缩放 | 页面 |
|---|---|
| 1100×760 @ 100% | Library、Workspace、Settings、Import drawer |
| 1440×960 @ 100% | 全部 |
| 1920×1080 @ 125% Windows | 全部 |
| 1920×1080 @ 150% Windows | Library、Workspace |
| 2560×1440 @ 100% | 全部 |

内容压力用例：

- 180 字符文件名；
- 500 字符错误说明；
- 20 个 tags（迁移数据）；
- 12 个 A-L 选项；
- 10 列表格；
- 超长 URL；
- 中英文混排；
- 200% 浏览器文本缩放。

---

## 11. 题库、数据库和过程文件重构

### 11.1 当前双事实源必须终止

当前流程大致是：

```text
jobs/<id>/job.json                 一份状态
jobs/<id>/authoring-ir.json        V1 内容
jobs/<id>/authoring/revisions/*    V2 内容
exams.payload_json                 legacy DB 内容
library_items + revisions          新 DB 内容
```

`save_job()` 先写文件，再 best-effort 双写 DB；元数据又先写 DB，再 best-effort 回写 JSON。任何一步失败都可能形成短时或永久分叉。

目标：

```text
SQLite LibraryRepository = 状态和 Canonical DS 的唯一权威
Content-addressed asset store = 二进制资源权威
Transient workspace = 可丢弃过程数据
```

### 11.2 数据库迁移步骤

#### M1：新增 V2 表，不删除旧表

- 建立 `library_items_v2`、`processing_jobs_v2`、`actionable_issues_v1`、`library_item_recovery_v1`。
- 旧 `exams/library_items/jobs` 继续可读。

#### M2：迁移现有数据

```rust
for legacy in load_all_active_legacy_items() {
    let candidate = prefer_in_order(
        current_v2_revision(legacy.id),
        authoring_v2_shadow(legacy.id),
        convert_v1_authoring(legacy.authoring_ir),
    );

    if let Some(ds) = candidate {
        insert_library_item_v2(ds, source_reference, migrated_status);
    } else {
        insert_shell_item(status = "migration_required");
    }
}
```

迁移必须幂等，以 `migration_version` 标记，不覆盖用户后来编辑的 V2 行。

#### M3：新 UI 只读写新表

- 所有编辑命令只调用 `LibraryRepositoryV2`。
- `job_store` 和 legacy `library_commands` 不再由新 UI 调用。

#### M4：一个稳定版本后删除双写

- 删除 `save_job -> upsert_reading_job`。
- 删除 `library meta -> write_back_meta_to_source`。
- 删除 legacy `exams` 查询。

### 11.3 Library Item 状态

```rust
enum LibraryItemStatusV2 {
    Processing,
    ActionRequired,
    Ready,
    Publishing,
    Published,
    Failed,
    Archived,
}
```

内部阶段与用户状态映射：

```text
任一 worker running              -> Processing
Canonical DS 有 blocker          -> ActionRequired
Canonical DS 完整                -> Ready
正在发布                          -> Publishing
发布成功                          -> Published
无可用 candidate                 -> Failed
```

### 11.4 过程文件目录

```text
<AppData>/
  authoring_hub.db
  assets/
    sha256/<prefix>/<hash>
  source-evidence/
    <asset-id>.<ext>          # 可选保留
  tmp/
    jobs/<job-id>/
      input/
      render/
      local/
      cloud/
      reconcile/
```

不再为每个题目保留 `preview/legacy/export-history/patches/revisions` 的深层目录树。

### 11.5 清理机制

```rust
fn cleanup_on_startup(now: DateTime<Utc>) {
    delete_tmp_jobs_where(
        completed_older_than = 24_hours,
        failed_older_than = 7_days,
        orphaned_older_than = 24_hours,
    );
    remove_unreferenced_assets_after_grace_period(7_days);
}

fn cleanup_after_canonical_commit(job_id: JobId) {
    keep_only([
        source_if_retention_enabled,
        assets_referenced_by_canonical_ds,
        minimal_processing_summary,
    ]);
}

fn cleanup_on_close_best_effort() {
    flush_editor_buffers();
    mark_running_jobs_interrupted();
    delete_disposable_render_cache();
}
```

应用正常关闭不是唯一触发点，因为进程可能崩溃或被强制结束。

### 11.6 资产去重

保留内部 SHA-256 作为内容地址和重复文件识别，不在 UI 展示：

```rust
let hash = sha256(bytes);
let stored_path = assets_root.join(&hash[0..2]).join(hash);
if !stored_path.exists() {
    atomic_write(stored_path, bytes)?;
}
```

这是存储去重和避免同名覆盖，不属于需要用户理解的“安全功能”。

---

## 12. 批量导入和实时状态

### 12.1 Import Drawer

用户只操作：

```text
拖入文件 / 选择文件
[可选：答案文件匹配方式]
[开始导入]
```

默认值：

```text
title = filename stem
modality = auto detect / Reading default
parseMode = auto
cloud = app setting
category = unknown（识别后填）
frequency = unset
```

这些字段不再要求导入前选择。

### 12.2 批量创建必须先落库

```rust
fn import_files(files: Vec<PickedFile>) -> Result<Vec<LibraryItemSummaryV2>> {
    // 文件复制和 hash 计算不得放在长时间 SQLite 事务中。
    // 先写 batch staging，任何单文件失败都形成明确结果。
    let staged = stage_import_batch(files)?;

    let created = db.transaction(|tx| {
        staged.successes.iter().map(|source| {
            let asset = register_staged_source(tx, source)?;
            let item = create_library_shell(tx, default_title(source), asset.id)?;
            let job = create_processing_job(tx, item.id, asset.id)?;
            Ok((item, job.id))
        }).collect::<Result<Vec<_>>>()
    })?;

    promote_staged_files_after_commit(&staged.successes)?;
    for (_, job_id) in &created {
        queue.enqueue(*job_id)?;
    }
    cleanup_batch_staging(staged.batch_id);
    Ok(created.into_iter().map(|(item, _)| item.summary()).collect())
}
```

若 `promote_staged_files_after_commit` 失败，repository 将对应 item 标记为 `failed/source_commit_failed`，而不是留下“Processing 但永远找不到源文件”的悬挂任务。批量中单个文件失败不应阻止其他文件建行。

创建成功后前端立即关闭 drawer，列表出现所有条目；不要等第一份 PDF 解析完成。

### 12.3 进度不是虚假百分比

进度使用阶段权重：

```text
prepare_source          5%
render_or_extract      20%
local_layout           20%
local_semantics        15%
cloud_upload            5%
cloud_inference        20%
validate_repair         5%
reconcile              10%
```

当云端不可用但本地成功时，状态可以进入 `待检查`，不永远卡在 80%。

### 12.4 任务取消与重试

- 取消未开始 job：立即 cancelled 并清理 temp。
- 取消本地正在运行：检查取消 token，在页/阶段边界退出。
- HTTP 云端调用：取消 request 或忽略迟到结果；迟到结果不能覆盖用户稿。
- 重试只重跑失败阶段，已有 valid local candidate 不重复解析。

### 12.5 应用重启恢复

```rust
fn recover_jobs_on_startup(repo: &Repo) {
    for job in repo.jobs_with_status(Running) {
        repo.mark_interrupted(job.id);
        if job.retry_count < MAX_AUTO_RECOVERY {
            repo.enqueue(job.id);
        } else {
            repo.require_action(job.id, "处理被中断，请点击重试");
        }
    }
}
```

### 12.6 前端订阅

```ts
useEffect(() => listen<ProcessingItemEvent>(
  "processing://item-updated",
  ({ payload }) => libraryStore.patch(payload.libraryItemId, payload)
), []);
```

不要轮询每个条目，也不要让每个详情页拥有自己的 worker。

---

## 13. 发布流程简化

### 13.1 用户入口

- 题库多选 -> `发布已选择`。
- 题目工作区 -> `发布`。
- 删除独立 `/export` 主导航。

### 13.2 发布输入

```rust
struct PublishLibraryItemsInput {
    item_ids: Vec<String>,
    destination_id: Option<String>,
    overwrite_policy: OverwritePolicy,
}
```

NAS 目录在设置中选择一次，后续记住；用户不需要每次选目录和发布模式。

### 13.3 发布前检查只保留业务闭包

必须保留：

```text
Canonical DS JSON 可反序列化
题组与 slot 归属完整
必需题干非空
选择题 option 完整
answer key 完整（若产品要求带答案）
引用资源存在
ReadingExamSourceV2 compiler 通过
目标目录可写
原子发布可完成
```

从普通发布主线移除或降级：

```text
要求所有题组手工逐个 Confirmed
因历史日志中出现过 fallback 就永久禁止发布
要求保留完整 SourceReviewV1 文件
向用户显示 schema/hash/CAS/manifest 技术细节
扫描整个 authoring/pipeline JSON 寻找任意字符串 partial/fallback
```

发布门禁应基于当前 Canonical DS 和当前 ActionableIssue，而不是历史过程痕迹。

### 13.4 Typed PublishCheckResult

```rust
struct PublishCheckResultV1 {
    passed: bool,
    blockers: Vec<PublishBlocker>,
    warnings: Vec<PublishWarning>,
}

struct PublishBlocker {
    code: String,
    target_id: Option<String>,
    user_message: String,
    action: PublishFixAction,
}
```

前端不再解析 `authoring_v2_export_blocked:...` 长字符串。

### 13.5 原子发布保留但隐藏

继续复用 `nas_package_v2` 的 staging 和最终提交。用户只看到：

```text
正在发布 3/12
发布完成：12 题
```

内部仍做：

```text
staging
asset copy
runtime compile
manifest last
atomic rename/commit
failure cleanup
```

### 13.6 多题发布

当前 V2 UI 路径对单 item 支持更成熟。新增批量 publisher：

```rust
async fn publish_batch(items: Vec<CanonicalExamDsV1>, destination: &Path) {
    let stage = create_batch_stage(destination)?;
    for item in items {
        compile_and_stage(item, &stage)?;
    }
    update_library_manifest_in_stage(&stage)?;
    probe_stage(&stage)?;
    commit_stage(stage, destination)?;
}
```

任一题失败时默认不提交整个批次，并返回每题具体问题；提供“仅发布通过项”作为二次明确动作，而不是默认静默跳过。

---

## 14. 设置页简化

### 14.1 默认视图字段

```text
云端识别                       [开关]
服务地址                       [input]
模型                           [input/select]
API Key                        [password]
                               [测试连接]

文件
保留原始 PDF 以便后续核对      [开关]

高级设置                       [展开]
```

### 14.2 高级设置

仅展开后显示：

```text
请求超时
云端并发数
本地并发数
Ollama 模式
过程文件保留（开发者模式）
环境诊断
```

不再显示：

```text
forceJson checkbox       -> 永远开启
profile enabled checkbox -> 由全局“云端识别”开关表达
temperature              -> 固定 0 或极低，除非开发者模式
不受支持 provider        -> 删除
```

### 14.3 Profile 数量

第一版普通用户只维护一个 active profile。需要多 Profile 的内部团队可以在开发者模式打开高级 Profile Manager，但不占普通页面。

### 14.4 保存策略

不要在 Base URL 每输入一个字符时 700ms 自动保存并立即使配置生效。改为：

- 表单本地编辑；
- 点击“保存并测试”或离开页面时保存；
- 测试成功后设为 active；
- API Key 继续使用现有安全存储，但不展示 storage backend 技术文案。

---

## 15. 错误处理和用户文案

### 15.1 错误分层

```rust
enum UserErrorCategory {
    SourceUnreadable,
    LocalRecognitionIncomplete,
    CloudUnavailable,
    CloudResultInvalid,
    ReconciliationConflict,
    SaveFailed,
    PublishBlocked,
    DestinationUnavailable,
}
```

### 15.2 用户错误与内部错误分离

```rust
struct AppErrorV2 {
    code: String,
    category: UserErrorCategory,
    user_message: String,
    retryable: bool,
    target_id: Option<String>,
    internal_detail_id: String,
}
```

用户看到：

```text
云端返回内容不完整，已保留本地识别结果。可以继续编辑或重试云端识别。
```

日志中才记录：

```text
JSON_SCHEMA_REQUIRED $.taskGroups[3].responseGroups[0].options
request_id=...
```

### 15.3 降级规则

| 场景 | 结果 |
|---|---|
| 本地成功、云端失败 | 立即提供本地稿；题库显示“云端未完成”但可编辑 |
| 本地失败、云端成功 | 提供云端稿，所有低证据字段标记待检查 |
| 两者部分成功 | 合并有效题组，缺失题组保留 source visual/manual shell |
| 两者失败 | 题库保留失败行，可重试；不创建伪造题目 |
| 云端 JSON 无法修复 | 内部记录 rejected；前端不展示 raw JSON |
| 保存冲突 | 自动拉取最新 DS；若只改不同 node，重放本地命令；否则原位提示 |
| 发布被阻止 | 点击问题直接定位到 Canvas 对应节点 |

---
## 16. 前端逐文件改造清单

### 16.1 `src/app/App.tsx`

**现状**：直接分发 Dashboard、JobList、ImportWizard、DocumentReview、UnifiedPreview、ExportPage、WritingStudio、LibraryPage、LibraryExamDetail、StructuredAuthoringEditorV2。

**改造**：

```tsx
export function App() {
  const route = useRoute();
  return (
    <AppShell route={route}>
      {route.name === "library" && <LibraryPage />}
      {route.name === "workspace" && <ExamWorkspacePage itemId={route.itemId} />}
      {route.name === "settings" && <SettingsPage />}
    </AppShell>
  );
}
```

- 删除全局 `listJobs()` 与 `activeJob` 读取；Library store 统一提供摘要。
- 删除 `refreshToken` 全页面重载模式，改用 query cache/store 的局部更新。
- Writing 若仍需保留，作为 Library 的 modality filter 或后续插件，不继续占主导航。

**验收**：App 入口不再 import 被退休页面；默认 hash 为 `/library`。

### 16.2 `src/app/router.ts`

**改造**：

```ts
export type RouteState =
  | { name: "library" }
  | { name: "workspace"; itemId: string }
  | { name: "settings" };
```

提供旧链接重定向：

```text
/jobs/:id/*      -> /items/:mapped-item-id
/library/:id     -> /items/:id
/export          -> /library?publish=1
/dashboard       -> /library
```

兼容重定向保留一个版本，并记录 telemetry/log，不在导航展示。

### 16.3 `src/components/AppShell.tsx`

**改造**：

```text
Logo
题库
设置
```

- 删除“转化工具”展开组。
- 删除 job stepper。
- 删除 category/frequency/error count 技术条。
- Workspace 路由下侧栏自动折叠为 56px 或完全隐藏。
- 顶部栏职责交给各页面，AppShell 不读取 active job。

### 16.4 `src/pages/LibraryPage.tsx`

**全面重写**：

```text
LibraryPage
  ├── LibraryHeader
  │    ├── Search
  │    ├── StatusTabs
  │    └── ImportButton
  ├── ImportDrawer
  ├── LibraryBatchBar
  └── LibraryItemList
       └── LibraryItemRow
```

新增目录：

```text
src/features/library/
  LibraryPage.tsx
  LibraryHeader.tsx
  LibraryItemList.tsx
  LibraryItemRow.tsx
  LibraryBatchBar.tsx
  libraryStore.ts
  libraryTypes.ts
```

- 每行同时表达 processing 与 library 状态。
- 打开行统一进入 `/items/:id`。
- 发布、删除、重试在行菜单/批量栏。
- 回收站放在一个小型筛选，不使用大型 tab + 多指标仪表板。

### 16.5 `src/pages/ImportWizard.tsx`

**退休页面，保留逻辑迁移**：

迁移到：

```text
src/features/import/ImportDrawer.tsx
src/features/import/useImportFiles.ts
src/features/import/FileDropzone.tsx
```

删除普通用户字段：category、frequency、tags、parseMode、答案文件的复杂角色配置、页面内云端 queue。

第一版可保留“附加答案文件”折叠入口，但默认只选择主文件。

调用：

```ts
const items = await processingClient.importFiles({ files, cloudEnabled: settings.cloudEnabled });
libraryStore.prepend(items);
closeDrawer();
```

### 16.6 `src/pages/ExamWorkspacePage.tsx`（新建）

取代 `LibraryExamDetail`、`UnifiedPreview`、`StructuredAuthoringEditorV2` 的主职责：

```tsx
function ExamWorkspacePage({ itemId }: { itemId: string }) {
  const workspace = useWorkspaceItem(itemId);
  return (
    <WorkspaceShell>
      <WorkspaceHeader ... />
      <ExamCanvas
        ds={workspace.draft}
        mode="author"
        issues={workspace.issues}
        onCommand={workspace.applyCommand}
      />
      <SourceDrawer ... />
      <IssuePopover ... />
    </WorkspaceShell>
  );
}
```

职责边界：

- 页面负责加载、保存状态、发布和 drawer。
- ExamCanvas 负责题面呈现和原位编辑。
- SourceDrawer 只读源 PDF。
- Issues 以 targetId 定位，不做独立问题清单页。

### 16.7 `src/components/ExamCanvasV2.tsx`

迁移为：

```text
src/exam-canvas/ExamCanvas.tsx
src/exam-canvas/ContentNodeRenderer.tsx
src/exam-canvas/renderers/ChoiceTask.tsx
src/exam-canvas/renderers/MatchingMatrix.tsx
src/exam-canvas/renderers/CompletionTask.tsx
src/exam-canvas/renderers/TableTask.tsx
src/exam-canvas/renderers/VisualTask.tsx
src/exam-canvas/editors/InlineTextEditor.tsx
src/exam-canvas/editors/OptionEditor.tsx
src/exam-canvas/editors/TableCellEditor.tsx
src/exam-canvas/editors/SlotPlacementEditor.tsx
```

具体修改：

- 删除裸 `contentEditable` 和 `document.execCommand`。
- `onTextChange` 改成 `onCommand(EditorCommandV1)`。
- `VisualAssetNode` 保留资源预览，但资源 URL 获取通过 workspace payload/cache，不由每个节点独立 Tauri 请求。
- 添加 `MatchingMatrix`。
- 作者工具默认隐藏，仅 hover/focus 当前节点出现。
- `student` 和 `author` 共用 DOM 结构；作者模式只增加 editor overlay。
- 给所有题型写明确 renderer，不让未知题型落入无声 generic fallback。

### 16.8 `src/editor/authoringTiptap.tsx`

**处理策略**：

- 第一阶段停止在生产路由使用。
- 将其 ContentDoc round-trip 测试保留一版，确保历史 V2 revision 仍能读。
- ExamCanvas 的文本编辑稳定后删除文件及 Tiptap 依赖；若未来需要富文本，再以 ExamCanvas NodeView 形式重新引入，而不是第二套编辑器。

### 16.9 `src/pages/UnifiedPreview.tsx`

拆除内容：

| 当前能力 | 新归属 |
|---|---|
| cloud localStorage queue/lease | Rust `processing` |
| V1 question form editing | 删除，新 Canvas 编辑 |
| source review | `SourceDrawer` + actionable issue |
| vision answer candidate | Canvas 内 answer issue |
| LLM group suggestion | Canvas 内 localized proposal |
| preview iframe/HTML | 删除，Canvas 本身就是预览 |
| validate/generate preview | 发布前 typed preflight |

迁移完成后删除文件。

### 16.10 `src/pages/StructuredAuthoringEditorV2.tsx`

- 将保存/undo/redo/恢复的可用逻辑提取到 `src/features/editor/useCanonicalEditor.ts`。
- 将 source overlay 提取为 `SourceDrawer`。
- 将 issue 定位逻辑提取为 `issueTargeting.ts`。
- 删除 outline pane、独立 edit/preview switch、复杂 revision 展示、页面内 export 结果技术卡。
- 完成迁移后删除原页面。

### 16.11 `src/pages/DocumentReview.tsx`

- 删除正常路由。
- 可复用 PDF/page overlay 组件到 `SourceDrawer`。
- 手工转录能力改为一个 blocker 的修复动作：`打开源文件并补录题面`，直接在 Canvas 建节点，不生成独立 manual transcription document。

### 16.12 `src/pages/ExportPage.tsx`

- 将目标目录选择和一次性保存偏好移到 Settings / 首次发布 modal。
- 将 `publishNasPackageV2` 调用移到 `publishClient.ts`。
- 将错误字符串解析删除，改用 `PublishCheckResultV1`。
- 迁移后删除页面和 `/export` 路由。

### 16.13 `src/pages/Settings.tsx`

- 重写为 `src/features/settings/SettingsPage.tsx`。
- 单列、最大宽度 720px。
- 默认单 Profile。
- Provider 只展示真实 adapter。
- `forceJson` 固定，不显示。
- 温度固定 0；timeout/并发放高级。
- preflight 只有出现错误时显示摘要。
- 过程文件保留开关仅开发者模式。

### 16.14 `src/api/tauriCommands.ts`

拆分：

```text
src/api/libraryClient.ts
src/api/processingClient.ts
src/api/workspaceClient.ts
src/api/publishClient.ts
src/api/settingsClient.ts
src/api/transport.ts
```

`transport.ts` 只负责：

```ts
invokeTyped<TInput, TOutput>(command, input)
listenTyped<TEvent>(eventName, handler)
normalizeAppError(error)
```

不再在每个 API 函数中判断 `isTauriRuntime()` 后调用巨型 dev backend。

### 16.15 `src/services/devFallbackBackend.ts`

- 从生产 bundle 移除。
- 新建 `src/test-support/fakeBackend.ts`，只实现测试当前用到的 6-8 个接口。
- 测试通过依赖注入 `BackendTransport`，而不是生产代码运行时自动回退 localStorage。
- 浏览器预览开发使用 fixture server，不复制 Rust 识别/发布逻辑。

### 16.16 `src/services/authoringV2Patches.ts`

- 保留纯函数节点定位和结构操作。
- 对外协议改名为 `EditorCommandV1`。
- 新增 `set_text`，避免 Unicode 字符下标错误。
- 删除与已退休页面耦合的 label/表单 helper。
- Rust 和 TypeScript 共享 JSON Schema/fixtures。

### 16.17 `src/services/runtimeViewModelV2.ts`

- 移入 `src/exam-canvas/model/runtimeProjection.ts`。
- 保持纯函数，不做网络请求或 UI 状态。
- 添加断言：每个 renderable slot 必须能在 ContentDoc 或 response group 中找到 host。
- Preview 和发布 compiler 使用同一组 fixture 验证。

### 16.18 `src/styles.css`

- 按第 10 章拆分。
- 删除未使用的 `.review-grid/.editor-grid/.llm-grid/.split-grid/.phase5-*`。
- 逐页面使用 CSS module 或 BEM 前缀，避免历史样式互相覆盖。
- 加入 `npm run test:layout`，扫描横向溢出。

---

## 17. 后端逐文件改造清单

### 17.1 `src-tauri/src/lib.rs`

**目标**：从巨型命令/测试容器变成薄入口。

```rust
mod processing;
mod recognition;
mod library;
mod editor;
mod publish;

pub fn run() {
    tauri::Builder::default()
      .manage(AppState::new(...))
      .invoke_handler(tauri::generate_handler![
          import_files,
          list_library_items,
          get_workspace_item,
          apply_editor_commands,
          retry_processing,
          publish_library_items,
          get_app_settings,
          save_app_settings,
      ])
      .setup(start_services)
      .run(...);
}
```

- 旧 commands 放到 `legacy_commands.rs`，只为迁移兼容注册。
- `#[cfg(test)] mod tests` 中数千行测试按模块移动。

### 17.2 `parser.rs`

- 保留文件类型探测和旧 V1 import adapter。
- 新 PDF/DOCX 主线调用 `build_document_ir_v2()`。
- 删除新链路对 `collapse_whitespace`、fake bbox 和 `markdownish_to_html` 的依赖。
- `DocumentIRV1` 只用于打开旧题和回归对照。

目标接口：

```rust
pub async fn ingest_source_to_document_v2(
    source: &StoredSourceAsset,
    options: IngestOptions,
) -> Result<DocumentIRV2>;
```

### 17.3 `pdf_facts_shadow.rs`

- 重命名 `pdf_facts.rs`。
- 将 shadow artifact 写入逻辑与 facts collector 分离。
- 只做 PDF 物理事实，不推断 IELTS。
- 复用现有 glyph、path、image、page geometry。
- 统一与 `pdf_geometry.rs` 的 PDFium/文本提取入口，避免同一 PDF 多次扫描。

### 17.4 `pdf_ingest/*`

逐文件：

| 文件 | 保留/修改 |
|---|---|
| `coordinates.rs` | 保留；补充 crop/page transform 单测 |
| `line_builder.rs` | 保留；输出明确 `lineId/spanId/glyphId`，禁止提前压平重要 gap |
| `region_builder.rs` | 增加 semantic-neutral region adjacency graph |
| `reading_order.rs` | 输出 primary + alternatives；低置信度时不给下游伪确定顺序 |
| `table_detector.rs` | 保留；将 tables 真正交给 local recognizer；改善无边框表格多候选 |
| `ocr_router.rs` | 保留 selective OCR 决策；不承担题型理解 |
| `ocr_merge.rs` | 保留 native/OCR 冲突；把冲突作为 field evidence |
| `compare_report.rs` | 迁入开发者诊断，不作为产品必需 artifact |
| `mod.rs` | 只编排 physical enrichment，拆除 shadow 命名 |

### 17.5 `docx_facts_shadow.rs` 与 `docx_ingest/*`

- 与 PDF 输出同一 `DocumentIRV2`。
- Word 原生 table 优先产生 physical table。
- DrawingML/SmartArt/浮动文本框难以结构化时生成 visual fallback asset + contained text regions。
- 删除 downstream 对“Word 一定按 XML 顺序就是阅读顺序”的假设。

### 17.6 `authoring_pipeline.rs`

处理原则：**冻结、隔离、逐步删除**。

- 移入 `legacy/v1_authoring_pipeline.rs`。
- 新 processing worker 不再调用它生成权威稿。
- 只用于：旧任务打开、V1 fixture regression、迁移工具。
- 不在此文件继续增加题型规则。

### 17.7 `ielts_grammar/mod.rs`

重构为新 `recognition/local` 模块，不保留一个 100KB 聚合文件。

迁移映射：

```text
anchors.rs               -> question_numbers.rs + question_blocks.rs
prompt_assembler.rs      -> stems.rs
option_run.rs            -> options.rs
option_bank.rs           -> matching.rs
completion.rs            -> completion.rs
reading.rs               -> document.rs
quality.rs               -> reliability.rs + current_ds_validation.rs
mod.rs                    -> local/mod.rs（薄 orchestrator）
```

新入口：

```rust
pub fn recognize_local(document: &DocumentIRV2) -> LocalRecognitionOutcome;
```

不再接受 V1 `split` 或 `v1_authoring` 作为必要输入。

### 17.8 `auto_pipeline.rs`

- 逐步删除现有超大同步编排。
- 第一阶段加 adapter，让旧 command 调新 `processing::enqueue_import`。
- 将 PDF render、vision answer、cloud outline、authoring write、cleanup 分拆到 worker。
- 不再由一个 blocking Tauri command 等待整条链完成。
- 本地 candidate 生成后立即持久化，不等待 cloud join 才返回。

### 17.9 `llm_gateway.rs`

保留：

- HTTP transport；
- timeout；
- Retry-After；
- JSON balanced extraction；
- request ID/log correlation；
- OpenAI-compatible/Ollama adapter。

移出：

- IELTS prompt 字符串；
- `CloudReadingOutlineV1` 业务组装；
- 业务字段默认值；
- 对 candidate 的容错补齐。

新增：

```rust
trait LlmTransport {
    async fn complete_json(&self, request: LlmJsonRequest) -> Result<LlmRawResponse>;
}
```

### 17.10 `llm_suggestions.rs`

拆分：

```text
recognition/cloud/skill_bundle.rs
recognition/cloud/request.rs
recognition/cloud/response.rs
recognition/reconcile/report.rs
legacy/v1_llm_suggestions.rs
```

- 当前 `CloudReadingOutlineV1` 保留为诊断/迁移兼容，不作为新链路输入。
- deterministic fallback 不再保存为“LLM suggestion”；应明确标记 `source=local_fallback`。

### 17.11 `llm_commands.rs`

- Profile CRUD 迁到 `settings/model_profiles.rs`。
- 旧 `llm_extract_group/apply_llm_suggestion` 命令从新 UI 移除。
- 新增 `retry_cloud_recognition(item_id)`。
- Provider 表单验证与 gateway adapter 枚举一致。

### 17.12 `db.rs`

- 新增 V2 单事实源表和 migration。
- SQL 拆到 `library/schema.rs` 或 migration 文件。
- 数据访问函数移到 repository，不继续在一个 50KB 文件中追加。
- 删除 production query 对 legacy `exams`/new `library_items` 的 COALESCE 混合。

### 17.13 `library_commands.rs`

- 新增 thin Tauri commands 调 `LibraryRepository`。
- 删除 `save file -> best effort DB` 和 `DB -> best effort source file` 双向同步。
- 旧数据迁移阶段只读旧 file/job，不再继续写回。

### 17.14 `job_store.rs`

- 替换为 `processing/repository.rs`。
- `job.json` 不再是权威状态。
- source import 创建 DB job；临时目录可随时重建/清理。
- `list_saved_jobs` 被 `list_library_items` 取代。

### 17.15 `artifact_store.rs`

短期：

- 保留读取现有 revision 的迁移能力。
- 新编辑不再创建每次保存的深层 revision 文件。
- asset blob 逻辑迁到全局 content-addressed asset store。
- atomic write helper 下沉到 `util/atomic.rs`。

长期删除：

```text
JobArtifactPaths.revisions_dir
patches_dir
preview_runtime_dir
legacy_dir
inspect_job_artifacts 普通产品入口
```

### 17.16 `authoring_v2_commands.rs`

拆分：

```text
editor/commands.rs
editor/apply.rs
editor/session.rs
publish/preflight.rs
library/assets.rs
```

当前 `validate_authoring_v2_publish_readiness` 递归扫描历史 JSON 中 fallback/partial 字符串，容易让已经修好的当前稿仍被历史痕迹阻止。目标只检查：

```text
current canonical DS
current actionable blockers
current compiler result
current referenced assets
```

### 17.17 `cleanup.rs`

- 重写为 `library/cleanup.rs`。
- 当前 kept 列表不再固定保留 job/project/source-review/uploads/exports。
- 清理依据变为“是否被 library item/current DS/source retention policy 引用”。
- 启动、成功、取消、退出四个触发点。
- 清理失败记录日志，不中断用户关闭。

### 17.18 `source_review.rs` / `authoring_review.rs`

- 保留来源页/bbox 定位算法。
- 输出 `ActionableIssue`，不要求用户进入单独页面做统一“source resolved”。
- 当所有当前 blocker 被修复，item 自动 Ready，不需要额外“我已人工确认全部”的总开关。

### 17.19 `authoring_validation.rs` / `runtime_validation.rs` / `validator.rs`

合并职责：

```text
CanonicalDsValidator
RecognitionCandidateValidator
PublishPreflight
```

错误返回 typed object；不再由 UI 解析字符串前缀。

### 17.20 `reading_source_v2.rs`

- 保留唯一 runtime compiler。
- 增加业务完整性检查：prompt、option text、visual fallback host。
- 移除“Phase 4 runtime slice”措辞和只支持部分 scope 的历史限制。
- 提供 `compile_with_report()` 返回 runtime + typed issues。

### 17.21 `nas_package_v2.rs`

- 保留原子 staging/commit/path safety。
- 增加 batch compile。
- 接口只接受已通过 preflight 的 canonical DS 列表。
- 用户结果只返回 item 数、成功/失败和目标，不返回 hash/内部路径清单。

### 17.22 `preview_commands.rs`

WYSIWYG 完成后：

- 删除 HTML preview 生成主链。
- 仅保留 source PDF page/crop preview 和 asset preview。
- `generatePreviewAssets` 不再是打开工作区的前置条件。

### 17.23 `environment.rs` / `diagnostics.rs`

- 环境检测在应用启动后台运行一次。
- 只有 blocking 项显示用户摘要。
- 完整 dependency/version 报告仅开发者模式。
- 不让可选 Python/Poppler 缺失阻止 born-digital PDF 主链。

### 17.24 `export_nas_library.rs` / `export_artifacts.rs` / `export_pack.rs`

- 新 Reading 产品只调用 V2 publisher。
- V1 export 保留一个 migration release，并在日志统计实际调用量。
- 无调用后删除 V1 export 和 Pack 页面逻辑。

### 17.25 `schema/*` 与 `contracts/*`

新增：

```text
processing-item-v1.schema.json
recognition-candidate-v1.schema.json
cloud-recognition-candidate-v1.schema.json
reconciliation-proposal-v1.schema.json
actionable-issue-v1.schema.json
editor-command-v1.schema.json
publish-check-result-v1.schema.json
```

继续保留：

```text
document-ir-v2
content-doc-v2
ielts-authoring-ir-v2
reading-exam-source-v2
```

schema manifest 和跨仓 NAS contract 继续自动校验，但不进入普通 UI。

---

## 18. 分阶段实施计划

> 估算以 2-3 名熟悉 Rust/Tauri/React 的工程师为基准。若只有 1 名工程师，按依赖顺序执行，不并行修改同一模块。

### Phase 0：冻结基线、建立真实产品验收门（3-5 个工程日）

#### P0-T01 固定审计和迁移基线

**文件**：

```text
Files/Product_Simplification_Baseline.md
scripts/verify-product-baseline.mjs
```

**工作**：

- 固定 commit SHA、当前 schema hash、8 份 Reading PDF fixture 和新增批量 fixture。
- 记录当前 route、命令、数据库表和 feature flag。
- 不再以 `task_plan.md` 中多个 “Current Active Goal” 作为唯一状态源。

**验收**：任何后续 PR 都能输出 baseline/migration compatibility 报告。

#### P0-T02 建立真实 Tauri smoke

**文件**：

```text
scripts/e2e/tauri-library-import.mjs
scripts/e2e/tauri-workspace-edit.mjs
scripts/e2e/tauri-publish.mjs
```

**工作**：

- 测试真实 Tauri command、SQLite 和文件系统；不依赖 devFallback localStorage。
- fixture 导入后读取 DB/Canonical DS 验证。

**验收**：真实 Tauri 环境中完成：导入 -> 打开 -> 改一个字符 -> 保存 -> 发布。

#### P0-T03 UI 截图与溢出基线

**文件**：

```text
scripts/ui/layout-matrix.mjs
fixtures/ui/long-content.json
```

**验收**：当前缺陷可复现并形成截图；后续 PR 做视觉回归。

**Phase 0 出口**：没有改变产品行为，但已经能用自动测试证明产品链和溢出问题。

---

### Phase 1：外壳和题库首页简化（5-8 个工程日）

#### P1-T01 三路由重构

**修改**：`App.tsx`、`router.ts`、`AppShell.tsx`。

**工作**：

- `/library` 默认入口。
- `/items/:id` 工作区。
- `/settings`。
- 旧 route 重定向。

#### P1-T02 题库统一列表

**修改/新增**：`LibraryPage.tsx`、`features/library/*`。

**工作**：

- 合并 Dashboard/JobList/Library。
- processing rows 与 ready items 同表。
- 状态、搜索、多选、导入入口。

#### P1-T03 Import Drawer

**新增**：`features/import/*`。

**工作**：

- 仅文件选择和开始导入。
- 暂时调用旧 import backend adapter，但 UI 不再展示 parse/category 等。

#### P1-T04 CSS 紧急整改

**修改**：拆分 `styles.css`。

**工作**：

- 删除固定三栏主布局。
- 修复所有 min-width/overflow-wrap。
- 题库和设置一列。

**Phase 1 验收**：

- 普通导航只显示题库、设置。
- 批量选择后题库立即出现行。
- 1100×760 无水平溢出。
- 旧功能暂可通过兼容 route 访问，但不在主导航。

---

### Phase 2：单一 Canonical DS 与题库 Repository（8-12 个工程日）

#### P2-T01 新表和 repository

**新增**：`library/repository.rs`、`library/schema.rs`、migration。

**工作**：创建 `library_items_v2/processing_jobs_v2/actionable_issues/recovery`。

#### P2-T02 迁移现有 V1/V2 题目

**新增**：`library/migration.rs`。

**工作**：优先当前 V2 revision；否则转换 V1 authoring；不可转换项标记 migration_required。

#### P2-T03 Workspace API

**新增**：

```text
get_workspace_item
apply_editor_commands
```

返回：

```rust
struct WorkspaceItemV1 {
    summary: LibraryItemSummaryV2,
    ds: CanonicalExamDsV1,
    edit_version: u64,
    issues: Vec<ActionableIssueV1>,
    source_preview: Option<SourcePreviewDescriptor>,
    processing: Option<ProcessingItemV1>,
}
```

#### P2-T04 停止新 UI 双写

新 UI 不再调用 `job_store.save_job` 或 legacy library CRUD。

**Phase 2 验收**：

- 新建题只有一个 canonical DS 权威字段。
- 修改标题/题干后 DB 与重新打开结果一致。
- 旧题迁移幂等。
- 模拟写入失败不会产生 DB/JSON 不一致。

---

### Phase 3：唯一 WYSIWYG 工作区（10-15 个工程日）

#### P3-T01 ExamWorkspacePage

**新增**：`features/editor/ExamWorkspacePage.tsx`。

#### P3-T02 ExamCanvas 拆分和统一

**修改**：`ExamCanvasV2.tsx`；新增 renderer/editor 子组件。

#### P3-T03 稳定文本编辑

- 移除 contentEditable/execCommand。
- 实现 AutoSizeTextarea、IME、粘贴纯文本、撤销。
- `set_text` 命令。

#### P3-T04 题型 renderer

至少覆盖：

```text
single choice
multiple choice
TFNG/YNNG
matching list
matching matrix
sentence/note/summary completion
table
visual/hybrid
```

#### P3-T05 SourceDrawer 和 localized issues

- 原文件只读抽屉。
- 问题点击定位。
- cloud proposal 原位 diff。

#### P3-T06 退休重复页面

从主 bundle 移除 `UnifiedPreview`、`DocumentReview`、`StructuredAuthoringEditorV2`、`LibraryExamDetail`。

**Phase 3 验收**：

- 打开题目即最终布局，无 edit/preview 开关。
- 改一个字符，刷新后仍在。
- 作者模式与 student mode DOM snapshot 一致，只有 editor overlay 差异。
- 中文输入法、快速输入、撤销/重做通过。

---

### Phase 4：本地 DocumentIRV2 直接识别（12-20 个工程日）

#### P4-T01 正式化 Physical Ingest

- `pdf_facts_shadow` 去 shadow。
- 统一 PDF 物理提取入口。
- DocumentIRV2 producer schema gate。

#### P4-T02 Question Layout Graph

新增 `recognition/local/question_blocks.rs` 和 graph types。

#### P4-T03 题号、题干和选项几何恢复

- token-based numbers；
- same-row/right/below stem assembly；
- wrapped option assembly；
- page header/footer exclusion。

#### P4-T04 基础题型硬闭包

覆盖：

```text
single choice
multiple choice
TFNG
YNNG
matching headings
matching information/features/classification
```

#### P4-T05 Completion 和 Visual

- table 使用 physical tables。
- flowchart/diagram 低置信度时 source crop + slot overlay。

#### P4-T06 Unassigned Evidence Ledger

显著未分配文字阻止 Ready。

**Phase 4 验收指标**：

- Golden corpus 中 Ready 的 simple choice `empty prompt = 0`。
- 单选预期 option label recall >= 99.5%。
- statement completeness >= 99%。
- Matching item + bank exact structure >= 98%。
- complex visual 100% 至少有 semantic 或 source-faithful fallback。

---

### Phase 5：云端完整识别 Skill 与容错 JSON 链（10-15 个工程日）

#### P5-T01 Versioned Skill Bundle

建立 `recognition/skills/ielts-reading-v1`。

#### P5-T02 完整 Cloud Candidate contract

Rust/TS/JSON Schema 三端同步。

#### P5-T03 Cloud request adapter

- PDF direct；
- 不支持时一次性分页图 fallback；
- 请求、skill、source version 绑定。

#### P5-T04 JSON extract/normalize/validate

- 单一 JSON；
- schema；
- semantic closure。

#### P5-T05 constrained repair 和 salvage

- 最多一次 repair；
- 分组 salvage；
- typed outcome。

#### P5-T06 第二次校对 proposal

只对 deterministic diff 请求校对。

**Phase 5 验收**：

- malformed JSON、代码围栏、前后解释、字段别名、缺字段均有 fixture。
- 原始 parse error 不进入普通 UI。
- 云端 valid group 可单独保存，invalid group 不污染 canonical。
- cloud output 不生成 HTML/JS。

---

### Phase 6：持久化双路并发和 Reconciliation（8-12 个工程日）

#### P6-T01 Processing Queue

- SQLite job；
- lease；
- cancellation；
- startup recovery；
- Tauri events。

#### P6-T02 Local/Cloud 并发

- 单题两路并发；
- 独立 semaphore；
- 一路失败不取消另一路。

#### P6-T03 Reconciliation Engine

- task alignment；
- field diff；
- safe auto-fill；
- conflict issue。

#### P6-T04 Workspace live update

- 本地稿先打开；
- cloud 迟到后 issues/proposals 增量进入；
- user_edited 保护。

#### P6-T05 删除前端 cloud queue

移除 `UnifiedPreview` localStorage queue/lease/window worker。

**Phase 6 验收**：

- 应用在 cloud running 时退出，重启后状态可恢复。
- 50 文件批量时 UI 不冻结。
- 用户编辑节点后，迟到 cloud 不覆盖。
- local/cloud 双失败时条目可重试且无伪 DS。

---

### Phase 7：发布、设置和清理收敛（7-10 个工程日）

#### P7-T01 发布并入 Library/Workspace

删除独立 ExportPage 主流程。

#### P7-T02 Typed publish preflight

当前 DS/issue/compiler/asset 检查，不扫描历史 fallback 字符串。

#### P7-T03 Batch NAS publisher

多题 staging 和全批提交。

#### P7-T04 设置简化

单 active model、真实 Provider、文件保留、高级折叠。

#### P7-T05 Artifact cleanup

- source policy；
- transient TTL；
- orphan asset GC；
- startup/success/cancel/close。

**Phase 7 验收**：

- 用户从题库选 20 题一键发布。
- 失败定位到具体题/节点。
- 普通 UI 不显示 hash/schema/manifest/CAS。
- 完成题目只保留 current DS、引用资产和配置允许的 source。

---

### Phase 8：旧链删除、真实回归和发布（8-15 个工程日）

#### P8-T01 删除退休前端

```text
Dashboard
JobList
ImportWizard page
DocumentReview page
UnifiedPreview
StructuredAuthoringEditorV2
LibraryExamDetail
ExportPage
Phase5 fixture route
```

#### P8-T02 删除生产 dev fallback

只保留 test adapter。

#### P8-T03 删除 V1 新写入和双写

旧 V1 只读 migration adapter 保留一个发行周期。

#### P8-T04 拆分超大 Rust 文件

完成 `auto_pipeline/authoring_pipeline/quality/lib.rs` 迁移和 dead code 删除。

#### P8-T05 真实发布验收

- Windows 安装包；
- 100 PDF corpus；
- 50 文件 batch；
- cloud endpoint live smoke；
- NAS student load；
- restart/crash/low disk/network interruption。

**Phase 8 出口**：新用户和新题不经过 V1 pipeline；旧题可迁移；主产品只有三个表面。

---
## 19. 测试和验收体系

### 19.1 测试金字塔

```text
Pure unit
  geometry / token / stem / option / schema / merge / editor command

Contract
  Rust <-> TypeScript <-> JSON Schema <-> NAS

Fixture integration
  PDF/DOCX -> DocumentIRV2 -> candidates -> canonical DS -> runtime

Real Tauri E2E
  import -> processing -> WYSIWYG edit -> publish

Visual regression
  author canvas / student canvas / overflow matrix

Fault injection
  exit / network / disk / malformed cloud / conflicting edits
```

### 19.2 本地识别 Golden Corpus

每份 fixture 不能只标“题型”，至少标：

```json
{
  "taskGroups": [
    {
      "range": [5, 7],
      "taskType": "single_choice",
      "questions": [
        {
          "number": 5,
          "prompt": "Which extra service ...?",
          "options": {
            "A": "changing the bed linen",
            "B": "washing the windows",
            "C": "cleaning the fridge"
          }
        }
      ]
    }
  ],
  "unassignedAllowed": ["page_number", "watermark"]
}
```

指标：

- Question Number Recall。
- Prompt Exact/Normalized Match。
- Prompt Non-empty Rate。
- Option Label Recall。
- Option Text Exact/Normalized Match。
- Shared Option Bank Exact Match。
- Slot Count Exact Match。
- Source Coverage。
- Significant Unassigned Text Rate。
- Visual Fidelity Closure。

### 19.3 Cloud Contract 测试

必须包含：

1. 纯 JSON 正常结果。
2. Markdown JSON 代码围栏。
3. JSON 前后说明文字。
4. 两个 JSON 对象（必须拒绝歧义）。
5. 非法枚举。
6. 缺 taskGroups。
7. 缺一个 option 正文。
8. range 与 slot 数不一致。
9. bbox 超出 0-1。
10. evidence quote 不在 source page。
11. repair 成功。
12. repair 仍失败但 3/4 groups 可 salvage。
13. 全部不可用。
14. HTTP 429 + Retry-After。
15. timeout。
16. provider 不支持 PDF 后 image fallback。
17. cloud 返回 prompt 改写而非转录，必须被 evidence/quote check 拒绝。

### 19.4 Reconciliation 测试

| Case | Expected |
|---|---|
| local/cloud same | no issue |
| local empty prompt + valid cloud | auto-fill |
| local nonempty + cloud different | proposal only |
| cloud adds missing wrapped line with unassigned source evidence | auto-append |
| user edited + late cloud | never overwrite |
| answer conflict | blocker/proposal |
| range overlap ambiguous | no auto alignment |
| option bank duplicated into each question | normalize to shared bank |
| Questions 14 and 15 shared prompt | one response group, two slots |

### 19.5 Editor Command 测试

- 中文 IME 输入。
- emoji/组合字符。
- 只删一个字符。
- 全句替换。
- 快速连续输入和 debounce。
- blur/route change 前 flush。
- stale base version。
- 不同 node 并发命令可重放。
- 同一 node 冲突提示。
- option add/delete/move。
- table cell edit。
- answer slot host 不得丢失。
- undo/redo 后保存。
- 浏览器刷新恢复。

### 19.6 Author/Student Parity

同一 fixture：

```text
render author mode
render student mode
strip editor overlays/data attributes
compare semantic DOM and screenshot geometry
```

必须一致：

- task 顺序；
- prompt 文本；
- option 文本；
- table row/col/span；
- slot 位置；
- visual asset crop；
- Matching matrix 行列。

### 19.7 真实 Tauri 测试替代 dev fallback 假绿

至少三条真实 E2E：

```text
E2E-1 born-digital PDF, cloud off
E2E-2 born-digital PDF, cloud on, local+cloud disagreement
E2E-3 batch 20 PDF, restart during processing
```

测试必须确认：

- Rust command 被调用；
- SQLite 行存在；
- canonical DS 存在；
- 临时文件清理；
- NAS package 可被目标 loader 读取。

`devFallbackBackend.ts` 的测试不能作为以上 E2E 通过证据。

### 19.8 故障注入

```text
PDF page render 失败
某一页 text layer 乱码
cloud timeout
cloud malformed JSON
cloud repair timeout
本地 worker panic
应用强制退出
磁盘剩余空间不足
SQLite busy/locked
source 文件导入中被删除
发布目标断开
发布 staging 后进程退出
```

每个场景必须有：

- 数据是否可恢复；
- 用户看到什么；
- 是否可重试；
- 是否会产生错误 Ready/Published。

### 19.9 性能目标

在推荐硬件上：

- 10 页 born-digital PDF 本地首稿 P50 < 3 秒，P95 < 8 秒。
- 本地首稿完成后工作区立即可开，不等待云端。
- 50 份批量导入创建题库行 < 2 秒。
- 单字符编辑视觉反馈 < 50ms。
- 保存确认 P95 < 500ms。
- 1000 题库条目搜索 < 150ms。
- 工作区滚动保持 50-60fps；大型表格不造成整页 rerender。

---

## 20. 删除、保留与迁移清单

### 20.1 直接从普通产品面删除

```text
Dashboard 入口
导题任务入口
独立新建导题页面
源文档确认步骤页
拆分步骤页
LLM review 步骤页
独立 Preview 页
独立结构化编辑器入口
独立 NAS 导出页
步骤条
每个题组的常驻置信度数字
普通用户可见 hash/schema/manifest/revision
```

### 20.2 迁移完成后删除的代码

```text
src/pages/Dashboard.tsx
src/pages/JobList.tsx
src/pages/ImportWizard.tsx
src/pages/DocumentReview.tsx
src/pages/UnifiedPreview.tsx
src/pages/StructuredAuthoringEditorV2.tsx
src/pages/LibraryExamDetail.tsx
src/pages/ExportPage.tsx
src/editor/authoringTiptap.tsx（若无其他调用）
src/services/devFallbackBackend.ts（生产路径）
legacy V1 page routes
browser localStorage cloud queue
```

### 20.3 必须保留但隐藏的工程机制

```text
SQLite transaction
atomic file write
safe relative path
asset reference closure
minimal content hash for asset dedupe
NAS staging and final commit
schema validation
runtime compiler validation
bounded crash recovery
```

删除这些会造成数据损坏或发布半成品；正确做法是让它们退出普通 UI，而不是从实现中彻底删除。

### 20.4 一版兼容期后删除

```text
DocumentIRV1 新写入
V1 authoring pipeline 新任务入口
legacy exams 双写
job.json 权威状态
V1 preview HTML 主路径
V1 Reading JS exporter（确认 NAS 全量支持 V2 后）
```

---

## 21. 开发 PR 顺序和依赖

### PR-01 产品基线与真实 Tauri E2E

- 只增测试和基线，不改行为。
- 阻断后续出现“dev fallback 通过但产品坏掉”。

### PR-02 三路由 + 极简 AppShell + overflow hotfix

- 可独立上线。
- 旧页面保留兼容 URL。

### PR-03 LibraryPage + ImportDrawer 统一入口

- 暂接旧 backend adapter。
- 用户体验先收敛。

### PR-04 LibraryRepositoryV2 + ProcessingJob schema

- 新旧并行读。
- migration test。

### PR-05 Workspace API + ExamWorkspacePage 骨架

- 先只读 Canonical DS。

### PR-06 ExamCanvas 稳定文本编辑与 EditorCommand

- 替换 contentEditable。
- 字符/句子编辑闭环。

### PR-07 Choice/Matching/Completion/Table/Visual renderers

- 同一 Canvas author/student parity。

### PR-08 Durable Processing Queue

- 迁移 ImportWizard browser orchestration。

### PR-09 DocumentIRV2 Direct Local Recognizer 基础题型

- single/multi/TFNG/YNNG/matching。

### PR-10 Physical Table + Hybrid Visual

- 表格和流程图不丢题面。

### PR-11 Versioned Cloud Skill + Full Candidate

- 不替换本地权威。

### PR-12 Cloud JSON Repair/Salvage

- malformed result 不打断用户。

### PR-13 Reconciliation + Localized Proposal

- cloud 差异进入 Canvas。

### PR-14 Batch Publish + Typed Preflight

- 删除独立 ExportPage 主路径。

### PR-15 Cleanup/Settings/Legacy Dual-write Removal

- 完成数据面收敛。

### PR-16 删除退休页面和 dead code

- 只有在调用量和迁移 gate 证明安全后执行。

依赖关系：

```text
PR-01
  -> PR-02 -> PR-03
  -> PR-04 -> PR-05 -> PR-06 -> PR-07
  -> PR-08 -> PR-09 -> PR-10
  -> PR-11 -> PR-12 -> PR-13
  -> PR-14 -> PR-15 -> PR-16
```

PR-08 和 PR-09 可在 PR-06/07 期间由不同工程师并行；PR-13 必须等 canonical workspace 和 cloud candidate 都稳定。

---

## 22. 团队任务分工建议

### 工程师 A：产品前端与 ExamCanvas

```text
PR-02, PR-03, PR-05, PR-06, PR-07
```

负责：路由、题库、工作区、编辑器、CSS、视觉回归。

### 工程师 B：本地识别与数据层

```text
PR-04, PR-08, PR-09, PR-10, PR-15
```

负责：repository、queue、DocumentIRV2 direct recognizer、cleanup。

### 工程师 C：云端与发布

```text
PR-11, PR-12, PR-13, PR-14
```

负责：skill、candidate contract、repair、reconciliation、NAS publish。

### 共同责任

- PR-01 真实 E2E。
- Rust/TS/schema contract。
- 每个 PR 的 migration/rollback。
- 不在同一时间多人修改 `lib.rs`/`styles.css`/`App.tsx`；先拆文件再并行。

---

## 23. 监控指标与发布门

### 23.1 产品指标

```text
Import-to-first-draft time
Prompt missing rate
Option missing rate
Cloud usable candidate rate
Auto-merge rate
Manual edits per paper
Median time to Ready
Publish success rate
Batch completion rate
Crash/restart recovery rate
```

### 23.2 质量目标

| 指标 | Alpha | Beta | 正式发布 |
|---|---:|---:|---:|
| Ready simple task empty prompt | 0 | 0 | 0 |
| Single choice option label recall | 98% | 99% | 99.5%+ |
| Matching shared bank exact | 95% | 97% | 98%+ |
| Visual task has semantic/fallback closure | 100% | 100% | 100% |
| Author/student semantic parity | 99% | 100% | 100% |
| 20-file batch restart recovery | 90% | 99% | 100% |
| Horizontal overflow matrix | 0 blocker | 0 | 0 |

### 23.3 灰度

- 内部：10-20 份已知 PDF。
- 友好客户：100 份真实 PDF，保留旧版只读回退。
- 10% 新导入使用 direct V2；其余旧链 shadow 对照。
- 50%。
- 100% 新导入 direct V2。
- 最后关闭 V1 新写入。

灰度期间只允许回退读取旧题，不允许同一新题同时由 V1/V2 双写修改。

---

## 24. Definition of Done

项目只有同时满足以下条件，才能认定本轮重构完成：

### 产品面

- [ ] 普通导航只有题库和设置；打开题目进入工作区。
- [ ] 导入在题库完成，不存在独立 wizard 主流程。
- [ ] 编辑界面即最终学生题面，不需要预览切换。
- [ ] 批量导入每题有独立实时状态。
- [ ] 发布在题库/工作区完成。

### 数据面

- [ ] 新题只有一个 Canonical DS 权威稿。
- [ ] job.json/legacy exams 不再参与新题写入。
- [ ] 运行时和 JS 只由 Canonical DS 编译。
- [ ] 临时 artifact 能在成功/取消/启动/退出后清理。

### 本地识别

- [ ] 新题 direct DocumentIRV2，不经 V1 authoring 主链。
- [ ] 基础题型 Ready 时题干和选项完整。
- [ ] significant unassigned evidence 阻止错误 Ready。
- [ ] 表格/流程图/diagram 至少有一种完整呈现。

### 云端

- [ ] 使用 versioned skill bundle。
- [ ] 返回完整 CloudRecognitionCandidateV1。
- [ ] malformed JSON 经过 validate/repair/salvage。
- [ ] 第二次校对只产生 proposal。
- [ ] cloud 失败不阻止打开本地稿。

### 编辑器

- [ ] 中文 IME、粘贴、撤销、刷新恢复通过。
- [ ] 用户改一个字符只更新对应 DS 节点。
- [ ] 迟到 cloud 不覆盖 user_edited 节点。
- [ ] author/student parity 100%。

### UI

- [ ] 1100×760、Windows 125%/150% 无横向溢出。
- [ ] 长文本、长文件名、宽表格有可用策略。
- [ ] 普通 UI 不展示技术 hash/schema/manifest。

### 测试

- [ ] Real Tauri import/edit/publish E2E 通过。
- [ ] 100-PDF corpus 报告通过正式门槛。
- [ ] 50 文件 batch + restart 通过。
- [ ] NAS student loader 读取发布包通过。
- [ ] 旧题迁移幂等、可回滚。

---
## 25. 范围边界

### 25.1 本轮必须完成

- Reading PDF/DOCX 导入。
- 本地 DocumentIRV2 直接识别。
- 云端完整识别和校对。
- Reading Canonical DS。
- 题库/批量任务。
- Reading WYSIWYG 工作区。
- Reading V2 NAS 发布。
- 前端简化和溢出治理。

### 25.2 本轮不扩展但必须保持兼容

- Writing 数据和已有发布能力。
- Listening 已有 contract/runtime 代码。
- 旧 V1 题库只读迁移。

这些能力可以暂时从普通主导航隐藏，但不能删除用户已有数据。后续要把 Listening 接入相同 Library/ExamCanvas/processing 架构时，应另立增量任务，不再创建新的平行工作台。

### 25.3 明确不做

- 不让模型直接生成或执行 JavaScript。
- 不让云端结果无审核覆盖用户编辑。
- 不在本轮实现多人实时协同编辑。
- 不追求把所有 PDF 流程图都完美反编译成结构化图；优先 source-faithful hybrid。
- 不把全部研发诊断移入普通用户界面。
- 不通过提高最小窗口宽度来掩盖 CSS 溢出。

---

## 26. 四轮对抗审计记录

> 以下不是形式化附录。每一轮都以“假设该计划会失败”为前提，重新对照用户需求和当前代码，记录发现的矛盾并将修订写回正文。

### 26.1 第一轮：产品简化对抗审计

#### 攻击问题 A：计划是否嘴上说三页面，实际上仍保留独立导入、源审核、预览、编辑和发布？

**发现**：初稿中虽然提出三主表面，但如果只是隐藏导航、继续让 `/jobs/:id/document -> preview -> export` 作为内部主流程，用户仍会被 route 跳转和步骤状态影响；`AppShell` 的 stepper 与 active job 条也会继续制造复杂度。

**修订**：

- 第 3 章把目标路由硬收敛为 `/library`、`/items/:id`、`/settings`。
- 第 16 章明确 `App.tsx/router/AppShell` 的删除和重写，而不是仅换文案。
- Import 变为 Library drawer。
- Source review 变为 Workspace drawer。
- Publish 变为 Library/Workspace action。
- 第 20 章列出退休页面和代码，不允许无限兼容。

#### 攻击问题 B：题库只留一个 DS，为什么计划仍保留 source 和 recovery？

**发现**：若严格在本地识别完成后立即删除原 PDF，用户后来发现题干漏行时无法对照；若完全不留恢复快照，应用崩溃可能丢失未完成编辑。机械执行“只留一个文件”会降低可靠性。

**修订**：

- 第 4.4 将恢复数据定义为“隐藏、有界、非题库版本”，只保留 current DS + last-good snapshot + 有界 journal。
- 第 4.5 将 source evidence 设为用户可配置；默认保留到首次确认，随后按策略清理。
- 第 11.5 明确清理不依赖正常退出。

#### 攻击问题 C：是否仍把置信度和复杂门禁推给用户？

**发现**：初稿部分位置仍使用 `quality/confidence` 作为内部判定，但没有明确主 UI 只显示动作。

**修订**：

- 第 3.4 列出不得暴露的技术词。
- 第 8.5 定义 `ActionableIssue`。
- 第 13.3 将发布门缩为当前业务闭包。
- 第 14 章将环境和诊断移入高级/开发者模式。

**第一轮结论**：通过。三主表面已经落实到 route、组件、命令和退休文件清单，而不是视觉改名。

---

### 26.2 第二轮：数据丢失与实现可行性对抗审计

#### 攻击问题 A：批量导入是否在 SQLite 事务中复制大 PDF，导致长锁和 UI 卡死？

**发现**：初稿 `import_files()` 伪代码把 `stage_source_file()` 放在数据库事务内。50 份大 PDF 会延长 WAL 写锁，其他保存和题库查询可能被阻塞。

**修订**：

- 第 12.2 已改为两阶段：先写 batch staging，再用短事务建立 asset/item/job，事务提交后 promote 文件并 enqueue。
- 单文件 promote 失败会标记具体 item failed，不留下永远 Processing 的悬挂行。

#### 攻击问题 B：CPU 密集 PDF 解析放进 async worker 是否阻塞 Tauri runtime？

**发现**：仅使用 `tokio::join!` 不等于安全并发；PDF 解析、图像渲染和几何聚类包含同步 CPU/文件 I/O。

**修订**：

- 第 5.5 本地 worker 已明确用 `tokio::task::spawn_blocking`。
- 本地、云端、PDF renderer 使用独立 semaphore。
- Tauri command 只 enqueue 并快速返回。

#### 攻击问题 C：用户要求改一个字符就对应最终 JS，计划若不直接改 JS 是否偏离需求？

**发现**：直接改 JS 可以满足表面直觉，却会造成 DS/JS 双事实源和半写文件。

**修订**：

- 第 9.6 明确语义等价实现：字符直接改 canonical text node；Canvas 即时更新；发布 compiler 生成相同字符变化的 JS。
- Runtime projection 直接读取 DS，用户不需要等待发布才看到结果。

#### 攻击问题 D：彻底删除 revision 是否会破坏现有 V2 迁移和冲突恢复？

**发现**：当前 `artifact_store.rs` 的不可变 revision 虽然文件多，但承载迁移和恢复。一次性删除有数据风险。

**修订**：

- 第 17.15 采用分阶段策略：先保留旧 revision 读取，停止新编辑写深层 revision；稳定后再删除目录结构。
- 第 18 Phase 2 明确迁移幂等和回滚。

**第二轮结论**：通过。计划从“概念简化”修正为短事务、阻塞任务隔离、有界恢复和渐进迁移。

---

### 26.3 第三轮：云端链路与不可信输出对抗审计

#### 攻击问题 A：Cloud evidence 中的 bbox/ID 是否会被直接信任，导致错误 source anchor 或 ID 冲突？

**发现**：模型只能给 page/quote/bbox 建议，它不知道本地真实 glyph/line IDs。若直接序列化进 Canonical DS，source overlay 和自动合并可能指向错误区域；模型 ID 也可能重复。

**修订**：

- 新增第 7.11 `Cloud Evidence Resolver`。
- quote 与 bbox 必须在本地 DocumentIRV2 中解析到一致节点。
- 所有 cloud IDs 进入 canonical 前由本地 `IdAllocator` 重分配。
- 无法唯一定位的字段只能成为 proposal。

#### 攻击问题 B：每份 PDF 无条件调用第二次校对是否增加成本和等待？

**发现**：若本地和云端完全一致，再调用 proofreader 没有价值；批量 100 份会翻倍费用。

**修订**：

- 第 7.13 明确只有存在结构/文本/答案/unassigned/partial 冲突时才调用第二次校对。
- proofreader 永远在两路候选完成和 deterministic diff 后运行，不与第一条识别盲目并发。

#### 攻击问题 C：大 PDF 超过模型上下文时是否被静默截断？

**发现**：仅规定“发送 PDF”不足以处理 provider byte/token 上限。

**修订**：

- 第 7.12 定义整卷/分片阈值和按题组边界拆分。
- 截断不能被视作完整结果；每一页必须进入 page manifest/coverage。

#### 攻击问题 D：当前 `Mutex<FnMut>` 会让云端内部操作串行，计划是否真正解决并发？

**发现**：现有 worker 虽然与本地链并行，但 vision answer 和 outline 在同一个 mutable gateway lock 下顺序执行。新设计若继续复用这一抽象，仍会产生隐式串行。

**修订要求**：

- `llm_gateway.rs` 改为线程安全 `LlmTransport: Send + Sync`，HTTP client 可 clone；不再由 scheduler 持有全局 `Mutex<FnMut>`。
- 第一条 full recognition 是一个明确任务；第二条 proofreader 等 deterministic diff 后再调度。
- 并发限制由 cloud semaphore 控制，而不是 gateway mutex。

#### 攻击问题 E：模型返回部分正确结果时，是不是仍以一个 parse error 全部丢弃？

**发现**：只有全对象 schema 校验会把 39 个正确问题和 1 个坏问题一起拒绝。

**修订**：

- 第 7.9 增加 task-group salvage。
- 第 7.6 定义一次 repair，之后按 group 逐个验证。
- UI 只显示未采用 group 数，不显示 serde/schema 原始错误。

**第三轮结论**：通过。Cloud 已从“outline + 一次 JSON 解析”变成 versioned full candidate、证据重绑定、一次修复、分组 salvage 和条件式 proofreader。

---

### 26.4 第四轮：WYSIWYG、跨仓学生端与 UI 回归对抗审计

#### 攻击问题 A：PDF2Test 是 React，NAS 学生端可能是另一套 renderer；仅在本仓写 `mode=student` 是否真的等于最终学生效果？

**发现**：同一 DS 不自动保证 React 作者端和 NAS 实际学生端像素一致。宣称“同一个组件”会忽略跨仓和跨框架事实。

**修订**：

- WYSIWYG 的发布定义改为：同一 Canonical DS、同一 renderer contract、同一题型布局 token，并以实际 NAS renderer 做自动截图 parity。
- `ExamCanvas` Author Mode 不应自己发明不同题型布局。
- 在 PR-07/PR-14 增加 cross-repo fixture：PDF2Test author screenshot 与 NAS student screenshot 比较。
- 若后续差异维护成本仍高，再抽取 framework-neutral Web Component；本轮不先引入该复杂度。

#### 攻击问题 B：把 contentEditable 换成 textarea 会不会不再“所见即所得”？

**发现**：完全受控的 textarea 若样式不同会产生跳动；但裸 contentEditable 对中文输入和 React 更危险。

**修订**：

- 第 9.3 使用原位 auto-size textarea，继承同字体/字号/行高/宽度，编辑时只增加 focus ring。
- 学生 DOM 不包含 textarea；author screenshot parity 在去除 editor overlay 后比较。
- 长段落编辑可按 paragraph node 展开，不建立独立表单页。

#### 攻击问题 C：仅在 CSS 里增加 `overflow-wrap` 是否足够？

**发现**：当前根因是固定列宽、主 surface 裁切和 Tauri 1100px 最小宽度与 1180px breakpoint 冲突；单独 word wrap 无法解决。

**修订**：

- 第 10 章要求删除三栏常驻布局、Source 改 overlay、Workspace 两栏、Settings 单列。
- 不提高 minWidth 掩盖问题。
- 增加 Windows 125%/150%、长内容和 `scrollWidth` 自动断言。

#### 攻击问题 D：现有 UI E2E 通过 devFallback，是否可能继续假绿？

**发现**：当前 sidecar 会直接修改 localStorage store，不能证明 Rust、SQLite、文件清理和 NAS publisher。

**修订**：

- Phase 0 把真实 Tauri smoke 设为所有重构的启动门。
- 第 19.7 明确 dev fallback 不可作为生产链证据。
- `devFallbackBackend.ts` 从生产 bundle 删除。

#### 攻击问题 E：用户要求“和官方界面一模一样”，是否需要复制品牌资产？

**发现**：把“一模一样”理解为复制官方商标或受保护资产既没有必要，也不是题型正确性的关键。

**修订**：

- 第 10.5 将验收定义为信息架构、左右栏、题型交互、字号密度、题号导航、作答反馈的功能/视觉同构。
- 使用客户批准的参考截图做视觉门，不复制品牌标识。

**第四轮结论**：通过。WYSIWYG 不再只依赖“同一数据”的口头承诺，而是加入实际 NAS renderer parity、IME 编辑策略和真实 Tauri/UI 截图门。

---

## 27. 用户需求到任务的追踪矩阵

| 用户需求 | 设计章节 | 实施任务 |
|---|---|---|
| PDF 放入后本地 V2 识别 | 5、6 | P4-T01~T06 |
| 云端并发识别完整 PDF | 5、7 | P5-T01~T05、P6-T01~T02 |
| 同步 prompt/skill/题型/schema | 7.2~7.5 | P5-T01~T03 |
| 云端 JSON 无法解析时内部处理 | 7.6~7.9 | P5-T04~T05 |
| 第二条云端校对链 | 7.10、8 | P5-T06、P6-T03 |
| 本地 JSON/云端 JSON 比对 | 8 | P6-T03 |
| 编辑不应是独立技术页面 | 3、9 | P3-T01~T06 |
| 最终渲染即编辑界面 | 9 | P3-T02~T05 |
| 改一个字符同步最终输出 | 9.4~9.6 | P3-T03、P2-T03 |
| 只保留题库/编辑/设置 | 3、16 | P1-T01~T04、P8-T01 |
| 题库只保留最终 DS | 4、11 | P2-T01~T04、P7-T05 |
| 关闭后清理过程文件 | 11.5 | P7-T05 |
| 批量 PDF 任务列表和转圈状态 | 3.3、12 | P1-T02~T03、P6-T01 |
| 返回题库后任务继续 | 5、12.5 | P6-T01、P6-T04 |
| 前端简洁美观、解决溢出 | 10、16 | P1-T04、P3、P8-T01 |
| 不暴露大量 hash/安全诊断 | 3.4、13、14 | P7-T02~T04 |
| 深入代码并指出改哪些模块 | 1、16、17 | 全部 PR |
| 至少三轮对抗审计 | 26 | 本文四轮 |

---

## 28. 最终执行建议

最先启动的不是 Cloud Skill，也不是继续扩充 Phase 编号，而是以下四个 PR：

1. `PR-01`：真实 Tauri E2E + UI 溢出基线。
2. `PR-02`：三路由和 AppShell 简化。
3. `PR-03`：题库统一列表与 ImportDrawer。
4. `PR-04/05/06`：单一 Canonical DS、Workspace API、稳定 ExamCanvas 文本编辑。

这四步完成后，客户即使仍使用现有识别器，也会先获得明显更简单、可编辑、可追踪的产品；随后 `PR-09~13` 再把本地和云端识别质量真正提升。

不要继续在以下文件中追加临时业务功能：

```text
UnifiedPreview.tsx
auto_pipeline.rs
authoring_pipeline.rs
src/styles.css
lib.rs
devFallbackBackend.ts
```

这些文件应进入“只修阻断 bug、不加新功能”的冻结状态，所有新能力按本计划的新领域模块落地。否则每次增加一个识别或 UI 功能，都会继续扩大当前最主要的复杂度来源。

---

## 附录 A：审计证据索引（按当前提交）

| 结论 | 主要源码 |
|---|---|
| 路由和页面过多 | `src/app/App.tsx`、`src/app/router.ts`、`src/components/AppShell.tsx` |
| 固定列宽和 overflow 风险 | `src/styles.css`、`src-tauri/tauri.conf.json` |
| 导入 localOnly + browser cloud queue | `src/pages/ImportWizard.tsx`、`src/pages/UnifiedPreview.tsx` |
| 作者端多套编辑/渲染 | `ExamCanvasV2.tsx`、`authoringTiptap.tsx`、`UnifiedPreview.tsx`、`LibraryExamDetail.tsx` |
| current ExamCanvas 可复用 | `src/components/ExamCanvasV2.tsx` |
| V2 runtime 同源投影 | `src/services/runtimeViewModelV2.ts`、`src-tauri/src/reading_source_v2.rs` |
| V1-first 识别残留 | `parser.rs`、`authoring_pipeline.rs`、`ielts_grammar/mod.rs` |
| 题号/题干/选项 line-first | `ielts_grammar/anchors.rs`、`prompt_assembler.rs`、`option_run.rs` |
| Physical V2 基础可复用 | `pdf_facts_shadow.rs`、`pdf_ingest/*`、`schema/document_ir_v2.rs` |
| Cloud 只是 outline | `llm_gateway.rs` 中 `CloudReadingOutlineV1` |
| Cloud worker/队列复杂 | `auto_pipeline.rs`、`UnifiedPreview.tsx` |
| Provider UI/adapter 不一致 | `Settings.tsx`、`llm_commands.rs`、`llm_gateway.rs` |
| 多事实源和双写 | `job_store.rs`、`db.rs`、`library_commands.rs`、`artifact_store.rs` |
| 过程文件保留过多 | `cleanup.rs`、`artifact_store.rs` |
| 发布门禁过度依赖历史痕迹 | `authoring_v2_commands.rs` |
| UI E2E 使用 dev fallback | `sidecars/ui-flow-e2e/ui-flow-e2e.mjs`、`devFallbackBackend.ts` |

## 附录 B：交付物清单

完成本计划后，仓库应至少新增或形成：

```text
recognition/skills/ielts-reading-v1/*
contracts/processing-item-v1.schema.json
contracts/recognition-candidate-v1.schema.json
contracts/cloud-recognition-candidate-v1.schema.json
contracts/reconciliation-proposal-v1.schema.json
contracts/actionable-issue-v1.schema.json
contracts/editor-command-v1.schema.json
contracts/publish-check-result-v1.schema.json

src/features/library/*
src/features/import/*
src/features/editor/*
src/features/settings/*
src/exam-canvas/*
src/api/{transport,libraryClient,processingClient,workspaceClient,publishClient,settingsClient}.ts
src/styles/*

src-tauri/src/processing/*
src-tauri/src/recognition/local/*
src-tauri/src/recognition/cloud/*
src-tauri/src/recognition/reconcile/*
src-tauri/src/library/*
src-tauri/src/editor/*
src-tauri/src/publish/*

scripts/e2e/tauri-library-import.mjs
scripts/e2e/tauri-workspace-edit.mjs
scripts/e2e/tauri-publish.mjs
scripts/ui/layout-matrix.mjs
```

## 附录 C：最终决策摘要

```text
产品入口：Library
编辑方式：最终考试界面内直接编辑
权威数据：IeltsAuthoringIRV2-based Canonical DS
本地识别：DocumentIRV2 direct geometry-first
云端识别：versioned full candidate
云端校对：diff-triggered proposal only
队列：Rust + SQLite durable scheduler
预览：Canonical DS direct render
发布：Library/Workspace action -> ReadingExamSourceV2 -> NAS
过程文件：transient + TTL cleanup
用户可见安全/诊断：最小化
底层原子性/路径/资源闭包：保留但隐藏
```
