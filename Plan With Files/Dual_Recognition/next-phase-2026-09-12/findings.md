# Findings

## 已知事实

- 当前 HEAD：`3dfd6c0 fix(workspace): restore back button to library`。
- 前置修复提交：`6967c53`；UI 改版提交：`a47131c`。
- 工作树在本次检查时为空。
- 用户报告 blocker 返回按钮已恢复，且 `npm run check`、`npm run build` 通过。
- 用户报告剩余 F3/F8 为按钮样式分裂，当前定性为非阻断架构边界。
- 审计目录包含总报告、13 份章节报告及独立的 task_plan/findings/progress/README。

## 初步证据判断

- `check` 和 `build` 能证明类型/静态检查与打包成功，尚不能证明 Tauri 导入—编辑—预览—导出—学生端链路。
- 下一阶段需要避免再次做泛化 UI 审计；更有价值的是把已修复内容变成产品路径回归门，并补齐未覆盖的关键链路。

## 审计与后续修复记录对齐

- 原审计在冻结基线 `6affc57` 上给出 157 项发现（P0 14 / P1 46 / P2–P3 97），但其后的修复和功能提交已显著改变 HEAD，不能直接把原优先级清单照搬为新计划。
- 审计当时 DoD 30 项没有任何一项达到 `product` 级；唯一真实 Tauri E2E 证据是旧构建导致的失败记录。
- 后续已新增 `e2e:tauri:workspace-edit`、`e2e:tauri:publish` 和前端 Vitest 基础设施；需要核实这些门在当前 HEAD 是否实际执行、是否进入 CI、产物是否新鲜。
- 旧失败中的 `title-persists` 后被诊断为陈旧 exe，而非持久化逻辑失败；新鲜度守卫已加入，但尚需当前 HEAD 的真实 Tauri 成功报告来闭环。
- 原审计建议中的数据正确性、`user_edited` 护栏、M4 主链接入曾因并发 agent 避让而未在 UI 修复轮处理；近期提交 `807221f` 显示 M4 P4-T03~T06 已推进，需按当前代码重新盘点。
- 根 `task_plan.md` 混有多个历史 active goal，不能作为唯一的下一步任务源；专项计划应明确新主目标与旧目标的去留。

## 可用验证入口

- 静态/单测：`npm run check`、`npm run build`、`npm test`、`cargo test`。
- 产品路径：`npm run e2e:tauri`、`npm run e2e:tauri:workspace-edit`、`npm run e2e:tauri:publish`。
- 学生端/发布契约：`npm run e2e:nas-contract`（必要时先 `dump:nas-fixture`）。
- 语料与识别：`verify:phase4-eight-pdf-acceptance` 在审计 README 中被提及，但当前 `package.json` 未列出同名 script，提示计划/工具可能继续漂移。

## 当前 HEAD 差异化复盘

### 仍开放或仅部分完成

- 数据正确性仍是最高优先级：A4-F01 导入 queue 失败孤儿、A4-F02 运行中取消/迟到结果、A4-F03 恢复状态文案误报、A4-F05 reclaim 覆盖 local 状态、A3-F04 `keepSourceFiles` 空操作均仍开放。
- M5 写入前置护栏仍缺：A7-F02 `user_edited` 仍只有写入标记、没有 reconcile 读取保护；cloud candidate / proposal-only / reconciliation engine 尚未落地。
- A7-F03 issue 仍为多来源：后端表缺 CRUD/消费，前端仍从 draft 派生；A7-F04 只完成前端集中人话错误层，后端仍返回字符串机器码。
- M4 已有实质进展：`QuestionLayoutGraphV1`、题型分类、硬闭包、quality blocker 已接入；但 V1 authoring 仍是权威输入，graph 尚未直接产 canonical DocumentIRV2，因此只能判“部分完成”。
- F3/F8 按钮分裂仍是非阻断债，但返回按钮存在新增的具体回归风险：`.workspace-back-button` 的 40×40/8px 声明可能被后置、特异性更高的 `.workspace-header button` 覆盖，需要浏览器 computed style 或 Tauri 实测裁定。

### 产品证据缺口

- 当前 HEAD 没有新鲜、完整成功的真实 Tauri 产品链报告；最新持久化真实报告仍是 2026-09-05 的失败记录。
- 8-PDF 报告为失败，原因是私有 PDF 缺失；100-PDF live 没有报告；NAS 只有旧 fixture，没有 contract 报告，更没有真实学生 Electron loader 验证。
- CI 当前主要覆盖 TS/build、browser fallback、Rust test/check、Windows 打包审计，未接真实 Tauri E2E、NAS/student、8-PDF 或 100-PDF。

## 规划方向

- Now：先等当前修复 agent 落地并冻结新 HEAD；重建新鲜 Tauri executable，跑完整产品验收；并行修数据安全护栏。
- Next：把 M4 从“附加 evidence + blocker”推进为量化、可默认启用的 direct canonical 主链；随后再允许 M5 candidate/reconcile 写入。
- Later：NAS/student 真实加载、100-PDF/50-file release gate、V1 退出与巨型模块拆分。
- F3/F8 应单独成小型治理任务，不能再触发一轮泛 UI 重构；首先修确定性的 selector cascade，并用 computed style/a11y 回归闭环。

## 2026-09-13 G2 flag-on 产品验证发现

- 现有共享 Tauri harness 只支持目录 PDF 自动选择；DOCX 真产品 E2E 需要在“选择文件”命令路径增加仅由环境变量启用的自动化钩子。该钩子可复用真实 ImportDrawer、create job、后台 pipeline、SQLite 和 Workspace，不绕过产品导入链。
- direct canonical 成功可由 `jobs/<itemId>/authoring-ir-v2.shadow.json` 的 `audit.notes` 精确证明：包含 `Direct canonical from QuestionLayoutGraphV1 (G2-T04); no V1 authoring input.`。只有 QLG 文件存在并不足以证明未回退。
- Workspace issue 点击的 selector 支持 `data-editor-id`，但 task group 根节点此前只有 `data-group-id`，导致匹配题共享选项缺失这类 task-level issue 点击后无法滚动定位；已在 ExamCanvas 根 article 增加同值 `data-editor-id`。
- 当前前端 actionable issues 只从 canonical DS 派生 prompt/options/answer 等少量问题，尚未把顶层 `recognitionBlockers` 或后端 `quality.issues` 统一呈现；flag-on E2E 只能证明现有问题列表可定位，不能证明识别 blocker 已完整产品化。
