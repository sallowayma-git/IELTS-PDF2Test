# Task Plan: Epic 8 Tauri Authoring App

## Goal
实现 `Epic8-Tauri作者端应用详细设计.md` 描述的本地 Tauri 作者端应用全部开发任务，并持续维护工程追踪记录表、发现记录和进度日志。

## Current Phase
Phase 4: Authoring UI and Runtime Integration

## Engineering Tracking Table
| ID | Task | Source | Status | Dependencies | Acceptance |
|----|------|--------|--------|--------------|------------|
| E8-00 | 初始化 Plan With Files 工作区与工程追踪记录 | User request | complete | 无 | 根目录存在 `Plan With Files/task_plan.md`、`findings.md`、`progress.md` |
| E8-01 | 细读 Tauri 设计文档并用旧工程文档抽取输出契约 | Tauri + output-contract docs | complete | E8-00 | 任务表覆盖页面、Rust command、数据模型、解析、LLM、预览、导出、设置 |
| E8-02 | 选择并搭建 Tauri 本地应用与内嵌界面工程骨架 | Tauri doc: 开发顺序 | complete | E8-01 | 可启动桌面开发环境与内嵌 UI，基础页面路由存在 |
| E8-03 | 实现本地数据目录、配置与 Job 存储 | Tauri doc: 本地数据目录、Rust 模型 | complete | E8-02 | Rust `cargo check` 与 Tauri release build 已验证 app data/job 存储命令可编译 |
| E8-04 | 实现文件导入与 sidecar/parser 接入骨架 | Tauri doc: Files/Parser commands | complete | E8-03 | TXT/MD/PDF/DOCX Python parser sidecar 已接入 Rust 调度；PDF/DOCX fixture smoke 已验证 |
| E8-05 | 实现规则粗切、答案对齐与 Authoring IR 编辑 | Tauri + output-contract pipeline | in_progress | E8-04 | 可从 Document IR 生成题组草稿并在 UI 编辑 |
| E8-06 | 实现 LLM profile、密钥存储、调用与建议审阅 | Tauri doc: LLM 设置与安全、LLM Review | in_progress | E8-05 | 本地 LLM gateway sidecar、profile 密钥文件引用、结构化建议与审阅 UI 已接入并通过 Rust 编译；系统 keychain/Stronghold 待完成 |
| E8-07 | 实现模板渲染、统一阅读页预览与校验 | Tauri + output-contract docs | in_progress | E8-05 | JS/manifest 生成、增强 DOM validator、RuntimePreview contract simulator 已接入；外部真实统一页源码 E2E 仍待接入 |
| E8-08 | 实现 PackBuilder、JS 导出和组 Pack 发布 | Tauri doc: Pack 发布 | in_progress | E8-07 | 单题 JS/manifest、Pack 目录/标准 `.zip` 已实现；导出/Pack 已强制 AuthoringIR + ReadingExamSourceV1 + DOM + RuntimePreview 四层门禁 |
| E8-09 | 完成 Dashboard、ImportWizard、DocumentReview、SplitAndAnswers、GroupEditor、LlmReview、UnifiedPreview、PackBuilder、Settings 全页面 | Tauri doc: 页面详细设计 | in_progress | E8-02..E8-08 | 页面流程闭环且状态可持久化 |
| E8-10 | 验收测试、错误修复与文档同步 | Tauri doc: 最小 MVP 范围、关键验收用例 | pending | E8-03..E8-09 | 关键验收用例通过，追踪表状态准确 |

## Phases

### Phase 1: Requirements & Discovery
- [x] 创建 Plan With Files 文件夹与三份文档
- [x] 阅读并摘要两份设计文档
- [x] 拆分本地端全部开发任务
- [x] 识别当前仓库状态、工具链和约束
- **Status:** complete

### Phase 2: Architecture & Scaffold
- [x] 决定 Tauri/Rust/内嵌界面技术栈与目录结构
- [x] 初始化 package、src、src-tauri 等工程文件
- [x] 建立共享类型、API 调用封装与基础路由
- **Status:** complete

### Phase 3: Core Local Backend
- [x] 实现 Rust 数据模型、存储、命令 API
- [x] 实现导入、解析骨架、粗切、校验、导出服务
- [x] 将规则粗切与 Authoring IR 生成改为优先从 `DocumentIRV1` 动态推导
- [x] 将 TXT/MD Python parser sidecar 与 Node validator sidecar 接入 Rust command 优先路径
- [x] 建立 Rust 工具链后的命令级验证
- [x] 建立桌面文件选择器与导出目录选择 UI
- [x] 添加 txt/md parser sidecar 与 Node validator sidecar 入口
- [x] 添加 PDF/DOCX deterministic parser adapter：PDF 走 `pypdf`，DOCX 走 Python stdlib OOXML 解析
- **Status:** complete

### Phase 4: Authoring UI
- [x] 实现 9 个页面的主要交互骨架
- [x] 接通页面到 Tauri command/dev fallback 的主流程
- [x] 添加本地 RuntimePreview contract simulator，执行生成的 manifest/wrapper 并校验自动填正确答案 100%
- [ ] 完成外部真实统一阅读页 E2E、系统 keychain/Stronghold、手工修订审计等闭环细节
- **Status:** in_progress

### Phase 5: Verification & Completion
- [ ] 跑通 MVP 导题链路
- [ ] 修复问题并更新追踪表
- [ ] 完成最终交付说明
- **Status:** pending

## Key Questions
1. 当前机器是否具备 Rust、Node、Tauri 构建工具链？
2. 设计文档要求的 Python parser sidecar 是否已有代码，还是本轮需要实现最小替代解析器？
3. 最终导出 JS/manifest 需要兼容哪个现有运行时项目？当前工作区是否包含该项目代码？
4. LLM 调用应先实现真实 provider，还是先实现本地占位/可配置 gateway 以保证离线 MVP？

## Decisions Made
| Decision | Rationale |
|----------|-----------|
| 将计划文件放在根目录 `Plan With Files/` 而非根目录平铺 | 用户明确要求建立 `Plan With Files` 文件夹放置三个文档 |
| 先以 MVP 闭环为开发顺序，再扩展完整页面与 LLM | Tauri 设计文档明确给出最小 MVP 范围和开发顺序 |
| 旧 Web 设计只作为输出契约参考，不作为作者端产品形态 | 用户明确更正作者端没有独立 Web；React/TS 仅是 Tauri 内嵌界面 |
| 保留 `ReadingExamSourceV1.sourceRefs.primaryProvider = "author_web"` | 旧输出契约明确该字段，可能被学生端运行时兼容逻辑依赖；用 audit notes 标识本地 Tauri 来源 |

## Errors Encountered
| Error | Attempt | Resolution |
|-------|---------|------------|
| `/goal` create failed because thread already has active goal | 1 | 使用现有活动目标继续推进 |
| Rust dynamic helper 大块替换 patch 上下文不匹配 | 1 | 改为新增动态 helper 并切换 command 入口，保留静态样例兜底 |
| Browser smoke 暴露题干重复 | 1 | `blockText`/`dynamic_block_text` 改为优先使用 `text`，仅在缺失时 fallback 到 stripped HTML |
| `rustc` / `cargo` 不在 PATH | 多次 | 2026-05-30 已用官方 `rustup` 安装 Rust stable 1.96.0，并配置 `~/.zshrc`/`~/.zprofile` |
| `brew install rustup-init` 在 macOS 13 上转为源码编译 CMake，耗时过长 | 1 | 终止 Homebrew 安装树并清理缓存，改用官方 rustup 安装 |
| 首次 `cargo check` 发现 Rust 函数重名、类型转换、图标缺失等编译错误 | 1 | 修复 `render_group_html` 重名、`Value::String` 转换、`write_json` 泛型约束，添加 `src-tauri/icons/icon.png` |
| `cargo clippy --all-targets -D warnings` 发现 10 个惯用法 lint | 1 | 按 clippy 建议修复后通过 |
| Homebrew Python `pip install --user` 触发 PEP 668 externally managed environment | 1 | 不污染全局 Python；PDF 使用已存在 `pypdf`，DOCX 使用 Python 标准库 OOXML 解析 |
| `cargo tauri --version` 初次不可用 | 1 | 通过 `cargo install tauri-cli --version 2.11.2 --locked` 安装全局 `cargo-tauri`，现 `cargo tauri` 与 npm 版 `tauri` 均可用 |

## Notes
- 每完成一个阶段或发现关键事实，需要更新本文件。
- 所有错误需记录，避免重复失败。
