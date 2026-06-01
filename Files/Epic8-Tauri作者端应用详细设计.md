# Epic 8：Tauri 作者端应用详细设计

来源：[[Epic8-作者端Web导题与组卷器工程设计]]

目标项目：`/Users/maziheng/Downloads/0.3.1 working`

最后更新：2026-05-29

## 产品形态

最终作者端建议做成 **Tauri + Rust 后端 + Web UI** 的本地桌面应用：

- Windows 分发：`.exe` / `.msi`
- macOS 分发：`.dmg`
- Web UI 负责复杂人工操作：上传、解析预览、题组切分、修题、LLM diff、统一阅读页预览、Pack 发布。
- Rust 后端负责本地权限边界：文件读写、任务状态机、密钥/设置、sidecar 调度、日志、导出。
- Node sidecar 复用现有项目的 JS 生成器、DOM 校验器、统一阅读页预览/E2E。
- Python sidecar 负责 PDF/Word/OCR/Docling 等文档转换。
- LLM Gateway 统一接 OpenAI compatible API、本地模型或机构代理。

设计原则：作者端是内部生产工具，不随学生端交付；它生成学生端需要的题库 JS/manifest/Pack，但本身可以携带导题、LLM、校验和调试能力。

## 桌面应用结构树

```text
ielts-author-studio/
├─ src-tauri/
│  ├─ Cargo.toml
│  ├─ tauri.conf.json
│  ├─ capabilities/
│  │  └─ default.json
│  ├─ binaries/
│  │  ├─ node-validator-{target}
│  │  ├─ python-parser-{target}
│  │  └─ preview-server-{target}
│  └─ src/
│     ├─ main.rs
│     ├─ app_state.rs
│     ├─ commands/
│     │  ├─ jobs.rs
│     │  ├─ files.rs
│     │  ├─ parser.rs
│     │  ├─ llm.rs
│     │  ├─ authoring.rs
│     │  ├─ validate.rs
│     │  ├─ preview.rs
│     │  ├─ pack.rs
│     │  └─ settings.rs
│     ├─ services/
│     │  ├─ job_service.rs
│     │  ├─ file_service.rs
│     │  ├─ parser_service.rs
│     │  ├─ llm_service.rs
│     │  ├─ authoring_service.rs
│     │  ├─ template_service.rs
│     │  ├─ export_service.rs
│     │  ├─ validation_service.rs
│     │  ├─ preview_service.rs
│     │  ├─ pack_service.rs
│     │  └─ settings_service.rs
│     ├─ models/
│     │  ├─ job.rs
│     │  ├─ document_ir.rs
│     │  ├─ authoring_ir.rs
│     │  ├─ reading_source.rs
│     │  ├─ validation.rs
│     │  ├─ llm.rs
│     │  ├─ pack.rs
│     │  └─ settings.rs
│     └─ storage/
│        ├─ db.rs
│        ├─ migrations/
│        └─ paths.rs
├─ src/
│  ├─ app/
│  │  ├─ router.tsx
│  │  ├─ layout/
│  │  └─ stores/
│  ├─ pages/
│  │  ├─ Dashboard.tsx
│  │  ├─ ImportWizard.tsx
│  │  ├─ DocumentReview.tsx
│  │  ├─ SplitAndAnswers.tsx
│  │  ├─ GroupEditor.tsx
│  │  ├─ LlmReview.tsx
│  │  ├─ UnifiedPreview.tsx
│  │  ├─ PackBuilder.tsx
│  │  ├─ JobAudit.tsx
│  │  └─ Settings.tsx
│  ├─ components/
│  │  ├─ document/
│  │  ├─ authoring/
│  │  ├─ validation/
│  │  ├─ llm/
│  │  └─ preview/
│  ├─ api/
│  │  └─ tauriCommands.ts
│  └─ types/
│     ├─ job.ts
│     ├─ document-ir.ts
│     ├─ authoring-ir.ts
│     ├─ validation.ts
│     └─ settings.ts
└─ sidecars/
   ├─ node-validator/
   ├─ python-parser/
   └─ preview-server/
```

## 本地数据目录

应用只应读写自己的 app data 目录和用户显式选择的导出目录。不要默认扫描用户磁盘。

```text
AppData / Application Support / IELTS Author Studio/
├─ config/
│  ├─ settings.json
│  └─ llm-profiles.json
├─ secrets/
│  └─ keyring-or-stronghold-managed
├─ jobs/
│  └─ import-20260529-001/
│     ├─ job.json
│     ├─ uploads/
│     │  ├─ source.pdf
│     │  └─ answers.docx
│     ├─ document-ir.json
│     ├─ split-candidates.json
│     ├─ authoring-ir.json
│     ├─ revisions.jsonl
│     ├─ llm-calls.jsonl
│     ├─ validation-report.json
│     ├─ preview/
│     │  ├─ manifest.js
│     │  └─ p1-medium-001.js
│     └─ exports/
│        ├─ p1-medium-001.json
│        ├─ p1-medium-001.js
│        └─ manifest.js
├─ packs/
│  └─ pack-20260529-basic/
├─ logs/
│  └─ app-2026-05-29.log
└─ cache/
   ├─ parser/
   ├─ thumbnails/
   └─ preview-server/
```

## Web UI 信息架构

```text
主窗口
├─ 左侧导航
│  ├─ 工作台
│  ├─ 导题任务
│  ├─ 题库草稿
│  ├─ Pack 组卷
│  ├─ 校验报告
│  ├─ LLM 调用记录
│  └─ 设置
└─ 主内容区
   ├─ 顶部 job 状态条
   ├─ 当前步骤面包屑
   ├─ 编辑/预览主体
   └─ 底部操作栏
```

### 页面树

```text
/dashboard
/jobs
/jobs/new
/jobs/:jobId
/jobs/:jobId/document
/jobs/:jobId/split
/jobs/:jobId/groups
/jobs/:jobId/llm-review
/jobs/:jobId/preview
/jobs/:jobId/validate
/jobs/:jobId/export
/packs
/packs/new
/packs/:packId
/settings
/settings/llm
/settings/parsers
/settings/storage
/settings/about
```

## 核心用户流程

### 流程 1：首次配置 LLM

1. 用户打开应用。
2. 进入 `设置 -> 大模型`。
3. 新建 LLM Profile。
4. 填写 Provider、Base URL、API Key、Model、温度、超时、是否强制 JSON。
5. 点击“测试连接”。
6. 后端发起一个轻量 JSON 测试请求。
7. 成功后保存 profile，API Key 进入系统 keychain/stronghold，不进入普通 JSON。

### 流程 2：创建导题任务

1. 用户点击“新建导题任务”。
2. 选择 PDF/Word。
3. 可选上传答案文档或答案页。
4. 设置题目标题、Passage 分类、难度、标签。
5. 点击“开始解析”。
6. Rust 创建 job 目录，复制文件，计算 hash，启动 parser sidecar。
7. 解析完成后进入文档预览页。

### 流程 3：解析预览与粗切

1. 页面左侧展示原文页面缩略图或渲染图。
2. 右侧展示 Document IR 块列表。
3. 用户确认 Passage 区、题组区、答案区。
4. 系统运行规则粗切，生成 split candidates。
5. 用户合并/拆分题组，修正题号范围和答案。

### 流程 4：LLM 辅助结构化

1. 用户在题组编辑器选择“LLM 识别题型”。
2. 前端调用 `llm_classify_group(jobId, groupId, profileId)`。
3. Rust 读取候选文本，拼 prompt，调用模型。
4. 模型返回 JSON。
5. Rust 校验 JSON schema。
6. 前端展示 diff：当前 IR vs LLM 建议。
7. 用户点击“应用建议”或“拒绝”。
8. 所有模型输入输出写入 `llm-calls.jsonl`。

### 流程 5：模板渲染与统一阅读页预览

1. 用户点击“生成预览”。
2. Rust 将 Authoring IR 转成 ReadingExamSourceV1。
3. Rust 调 Node validator 检查 schema 和 DOM 协议。
4. Rust 输出临时 `manifest.js` 和单题 JS 到 job preview 目录。
5. 前端 iframe 打开统一阅读页预览。
6. 用户可手动答题，也可点击“自动填正确答案测试”。
7. E2E 校验通过后进入导出。

### 流程 6：导出 JS / 组 Pack

1. 用户点击“导出题库 JS”。
2. 系统输出：
   - `p1-medium-001.json`
   - `p1-medium-001.js`
   - `manifest.js`
   - `validation-report.json`
3. 用户进入 Pack 组卷。
4. 选择多个已通过题目。
5. 填 Pack 名称、版本、机构、有效期、说明。
6. 点击“生成 Pack”。
7. 系统生成 Pack 目录或 zip。

## 页面详细设计

### 1. 工作台 Dashboard

结构：

```text
Dashboard
├─ 顶部统计
│  ├─ 草稿数
│  ├─ 待人工确认
│  ├─ 校验失败
│  └─ 可发布
├─ 最近任务列表
├─ 最近 Pack
└─ 快捷操作
   ├─ 新建导题任务
   ├─ 打开 Pack 组卷
   └─ LLM 设置
```

主要动作：

- 点击任务进入 `/jobs/:jobId`。
- 点击“新建导题任务”进入上传向导。

### 2. 新建导题任务 ImportWizard

结构：

```text
ImportWizard
├─ Step 1 上传文件
│  ├─ 主文件 PDF/DOCX
│  ├─ 答案文件 可选
│  └─ 解析模式 文本 / OCR / 自动
├─ Step 2 基础信息
│  ├─ 标题
│  ├─ Passage 分类 P1/P2/P3
│  ├─ 难度 low/medium/high
│  └─ 标签
├─ Step 3 LLM 配置
│  ├─ 是否启用 LLM 辅助
│  ├─ LLM Profile
│  └─ 自动识别范围
└─ 底部操作
   ├─ 取消
   └─ 创建并解析
```

表单字段：

| 字段 | 类型 | 说明 |
|---|---|---|
| `sourceFilePath` | file | 用户选择的 PDF/DOCX |
| `answerFilePath` | file? | 可选答案文件 |
| `parseMode` | enum | `auto/text/ocr` |
| `title` | string | 题目标题，可从文档推断 |
| `category` | enum | `P1/P2/P3` |
| `frequency` | enum | `low/medium/high` |
| `tags` | string[] | 机构自定义 |
| `llmEnabled` | boolean | 是否在解析后调用 LLM |
| `llmProfileId` | string? | 使用哪个大模型配置 |

### 3. 文档解析预览 DocumentReview

结构：

```text
DocumentReview
├─ 左侧 DocumentCanvas
│  ├─ 页面缩略图
│  ├─ bbox 高亮
│  └─ 低置信度标记
├─ 右侧 BlockInspector
│  ├─ block 列表
│  ├─ 文本内容
│  ├─ 类型 paragraph/table/image
│  └─ confidence
└─ 底部操作
   ├─ 重新解析
   ├─ 保存修正
   └─ 进入粗切
```

交互：

- 点击 block，高亮原文位置。
- 修改 block 类型。
- 合并 block。
- 标记 block 为 Passage / Question / Answer / Ignore。

### 4. 粗切与答案对齐 SplitAndAnswers

结构：

```text
SplitAndAnswers
├─ Passage 区
│  ├─ 标题候选
│  └─ 正文 block 范围
├─ 题组区
│  ├─ group 列表
│  ├─ 题号范围
│  ├─ 说明文字
│  └─ block 范围
├─ 答案区
│  ├─ answerKey 表格
│  ├─ 自动抽取候选
│  └─ 缺失/冲突提示
└─ 底部操作
   ├─ 运行规则粗切
   ├─ 应用 LLM 初始识别
   └─ 进入题组编辑
```

答案表字段：

| 字段 | 说明 |
|---|---|
| `displayNumber` | 原文题号，如 `27` |
| `internalId` | 运行时题号，如 `q1` |
| `answer` | 标准答案 |
| `answerType` | `text/option/list` |
| `source` | `manual/rule/llm/imported` |
| `confidence` | 抽取置信度 |
| `verified` | 是否人工确认 |

### 5. 题组结构化编辑器 GroupEditor

结构：

```text
GroupEditor
├─ 左栏 GroupNavigator
│  ├─ group-1 Questions 1-8
│  ├─ group-2 Questions 9-13
│  └─ 校验状态
├─ 中栏 GroupForm
│  ├─ kind
│  ├─ instruction
│  ├─ questions
│  ├─ options
│  ├─ answers
│  └─ layout template
├─ 右栏 LiveBodyHtmlPreview
│  ├─ 题组 HTML 预览
│  ├─ DOM 协议检查
│  └─ 采集结果预览
└─ 底部操作
   ├─ LLM 建议
   ├─ 渲染模板
   ├─ 校验当前题组
   └─ 下一组
```

题目编辑字段：

| 字段 | 说明 |
|---|---|
| `id` | 内部题号 `q1` |
| `displayNumber` | 原题号 |
| `prompt` | 题干 |
| `interaction.type` | `radio/checkbox/text/textarea/select/dragdrop/table/diagram` |
| `interaction.options` | 选项数组 |
| `answer` | 标准答案 |
| `sourceBlockIds` | 来源 block |
| `confidence` | 机器识别置信度 |
| `verified` | 人工确认 |

### 6. LLM 建议审阅 LlmReview

结构：

```text
LlmReview
├─ 左侧 CurrentIR
├─ 中间 DiffViewer
├─ 右侧 LlmSuggestion
├─ 顶部 PromptInfo
│  ├─ profile
│  ├─ model
│  ├─ prompt version
│  └─ token/cost
└─ 底部操作
   ├─ 应用全部
   ├─ 逐项应用
   ├─ 拒绝
   └─ 重新调用
```

规则：

- 低置信度建议不能自动应用。
- 修改答案必须额外确认。
- 所有建议以 JSON patch 形式保存。

### 7. 统一阅读页预览 UnifiedPreview

结构：

```text
UnifiedPreview
├─ 顶部预览状态
│  ├─ schema 校验
│  ├─ DOM 校验
│  ├─ E2E 校验
│  └─ 运行时错误
├─ iframe 预览区
│  └─ reading-practice-unified.html?examId=temp
├─ 右侧调试面板
│  ├─ collectedAnswers
│  ├─ answerKey
│  ├─ scoreInfo
│  └─ console errors
└─ 底部操作
   ├─ 重新生成预览
   ├─ 自动填正确答案
   ├─ 跑完整校验
   └─ 导出
```

### 8. PackBuilder

结构：

```text
PackBuilder
├─ 左侧可发布题库
│  ├─ 搜索
│  ├─ P1/P2/P3 筛选
│  ├─ 标签筛选
│  └─ 校验状态筛选
├─ 中间已选题
│  ├─ 顺序
│  ├─ Pack manifest
│  └─ 版本信息
├─ 右侧发布设置
│  ├─ packId
│  ├─ version
│  ├─ institution
│  ├─ validFrom/validTo
│  └─ description
└─ 底部操作
   ├─ 运行发布前检查
   └─ 生成 Pack
```

### 9. 设置 Settings

```text
Settings
├─ 大模型
│  ├─ Profiles
│  ├─ Base URL
│  ├─ API Key
│  ├─ Model
│  ├─ JSON Mode
│  ├─ Timeout
│  └─ Test Connection
├─ 解析器
│  ├─ PDF parser
│  ├─ OCR mode
│  ├─ Python sidecar status
│  └─ Temp cache
├─ 存储
│  ├─ App data path
│  ├─ Export default path
│  └─ Cache cleanup
├─ 预览
│  ├─ 统一阅读页路径
│  ├─ Node validator status
│  └─ Preview server port
└─ 关于
   ├─ 版本
   ├─ sidecar versions
   └─ 日志导出
```

## Rust 后端模型定义

### Job

```rust
#[derive(Serialize, Deserialize, Clone)]
pub struct ImportJob {
    pub job_id: String,
    pub title: String,
    pub status: JobStatus,
    pub category: Option<PassageCategory>,
    pub frequency: Option<Frequency>,
    pub tags: Vec<String>,
    pub source_files: Vec<SourceFile>,
    pub active_llm_profile_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub current_step: WorkflowStep,
    pub issue_counts: IssueCounts,
}

pub enum JobStatus {
    Draft,
    Uploaded,
    Parsed,
    SplitReady,
    AuthoringReady,
    NeedsHumanReview,
    ValidationFailed,
    PreviewReady,
    ExportReady,
    Published,
}
```

字段说明：

| 字段 | 功能 |
|---|---|
| `job_id` | 导题任务唯一 ID，也是 job 目录名 |
| `title` | 题目标题，默认从文档识别，可人工修改 |
| `status` | 任务状态机 |
| `category` | P1/P2/P3 |
| `frequency` | low/medium/high |
| `source_files` | 上传文件列表 |
| `active_llm_profile_id` | 当前使用的大模型配置 |
| `current_step` | UI 当前流程步骤 |
| `issue_counts` | 校验错误、警告、待确认数量 |

### SourceFile

```rust
#[derive(Serialize, Deserialize, Clone)]
pub struct SourceFile {
    pub file_id: String,
    pub original_name: String,
    pub stored_name: String,
    pub file_type: SourceFileType,
    pub sha256: String,
    pub size_bytes: u64,
    pub role: SourceFileRole,
    pub imported_at: DateTime<Utc>,
}

pub enum SourceFileRole {
    MainQuestion,
    AnswerKey,
    Explanation,
    Asset,
}
```

字段说明：

| 字段 | 功能 |
|---|---|
| `file_id` | 文件唯一 ID |
| `original_name` | 用户原文件名 |
| `stored_name` | app data 内保存文件名 |
| `file_type` | pdf/docx/image/txt |
| `sha256` | 去重和审计 |
| `role` | 主试题、答案、解析或素材 |

### Document IR

```rust
#[derive(Serialize, Deserialize, Clone)]
pub struct DocumentIr {
    pub schema_version: String,
    pub job_id: String,
    pub pages: Vec<DocumentPage>,
    pub assets: Vec<DocumentAsset>,
    pub parser: ParserInfo,
}

pub struct DocumentPage {
    pub page_index: u32,
    pub width: f32,
    pub height: f32,
    pub blocks: Vec<DocumentBlock>,
}

pub struct DocumentBlock {
    pub block_id: String,
    pub block_type: BlockType,
    pub text: Option<String>,
    pub html: Option<String>,
    pub table: Option<TableIr>,
    pub bbox: Option<[f32; 4]>,
    pub confidence: f32,
    pub role_hint: Option<BlockRole>,
}
```

字段说明：

| 字段 | 功能 |
|---|---|
| `block_type` | paragraph/table/image/list/header/footer |
| `text` | 纯文本抽取结果 |
| `html` | 保留基础格式的 HTML |
| `table` | 表格结构 |
| `bbox` | PDF 页面坐标，用于预览定位 |
| `confidence` | OCR/解析置信度 |
| `role_hint` | passage/question/answer/ignore |

### Authoring IR

```rust
#[derive(Serialize, Deserialize, Clone)]
pub struct ReadingAuthoringIr {
    pub schema_version: String,
    pub job_id: String,
    pub exam: ExamMetaDraft,
    pub passage: PassageDraft,
    pub groups: Vec<QuestionGroupDraft>,
    pub answer_key: BTreeMap<String, AnswerValue>,
    pub question_order: Vec<String>,
    pub question_display_map: BTreeMap<String, String>,
    pub audit: AuthoringAudit,
}

pub struct QuestionGroupDraft {
    pub group_id: String,
    pub kind: GroupKind,
    pub question_range: Option<(u32, u32)>,
    pub instruction: Vec<String>,
    pub questions: Vec<QuestionDraft>,
    pub layout: LayoutSpec,
    pub allow_option_reuse: Option<bool>,
    pub source_block_ids: Vec<String>,
    pub confidence: f32,
    pub verified: bool,
}

pub struct QuestionDraft {
    pub id: String,
    pub display_number: String,
    pub prompt: String,
    pub interaction: InteractionSpec,
    pub answer: Option<AnswerValue>,
    pub source_block_ids: Vec<String>,
    pub confidence: f32,
    pub verified: bool,
}
```

字段说明：

| 字段 | 功能 |
|---|---|
| `groups[].kind` | 映射到 `ReadingExamSourceV1.questionGroups[].kind` |
| `question_range` | 原文题号范围 |
| `interaction` | Web 编辑器和模板渲染器使用 |
| `answer` | 标准答案，发布前必须存在 |
| `layout` | 表格、summary、flow-chart 等模板参数 |
| `allow_option_reuse` | matching/classification 必须明确 |
| `verified` | 是否人工确认 |

### LLM Profile

```rust
#[derive(Serialize, Deserialize, Clone)]
pub struct LlmProfile {
    pub profile_id: String,
    pub name: String,
    pub provider: LlmProvider,
    pub base_url: String,
    pub model: String,
    pub api_key_secret_ref: String,
    pub temperature: f32,
    pub timeout_ms: u64,
    pub force_json: bool,
    pub enabled: bool,
}

pub enum LlmProvider {
    OpenAiCompatible,
    AnthropicCompatible,
    Ollama,
    Custom,
}
```

字段说明：

| 字段 | 功能 |
|---|---|
| `base_url` | API 地址，如 `https://api.openai.com/v1` 或机构代理 |
| `model` | 模型名 |
| `api_key_secret_ref` | 指向系统 keychain/stronghold，不保存明文 |
| `temperature` | 默认建议 0 或 0.1 |
| `force_json` | 强制 JSON 输出 |
| `timeout_ms` | 单次请求超时 |

### ValidationReport

```rust
#[derive(Serialize, Deserialize, Clone)]
pub struct ValidationReport {
    pub job_id: String,
    pub passed: bool,
    pub layers: Vec<ValidationLayerReport>,
    pub issues: Vec<ValidationIssue>,
    pub generated_at: DateTime<Utc>,
}

pub struct ValidationIssue {
    pub issue_id: String,
    pub severity: Severity,
    pub layer: ValidationLayer,
    pub path: String,
    pub message: String,
    pub fix_hint: Option<String>,
}
```

## Tauri Command API

前端只通过 Tauri command 调 Rust，不直接读写任意路径。

### Jobs

```rust
#[tauri::command]
async fn create_import_job(input: CreateJobInput, state: State<'_, AppState>) -> Result<ImportJob>;

#[tauri::command]
async fn list_jobs(filter: JobFilter, state: State<'_, AppState>) -> Result<Vec<ImportJob>>;

#[tauri::command]
async fn get_job(job_id: String, state: State<'_, AppState>) -> Result<JobDetail>;

#[tauri::command]
async fn update_job_meta(job_id: String, patch: JobMetaPatch, state: State<'_, AppState>) -> Result<ImportJob>;

#[tauri::command]
async fn delete_job(job_id: String, state: State<'_, AppState>) -> Result<()>;
```

### Files

```rust
#[tauri::command]
async fn import_source_file(job_id: String, file_path: String, role: SourceFileRole, state: State<'_, AppState>) -> Result<SourceFile>;

#[tauri::command]
async fn reveal_job_folder(job_id: String, state: State<'_, AppState>) -> Result<()>;

#[tauri::command]
async fn choose_export_dir() -> Result<Option<String>>;
```

### Parser

```rust
#[tauri::command]
async fn parse_document(job_id: String, options: ParseOptions, state: State<'_, AppState>, app: AppHandle) -> Result<DocumentIr>;

#[tauri::command]
async fn rerun_ocr(job_id: String, page_indices: Vec<u32>, state: State<'_, AppState>, app: AppHandle) -> Result<DocumentIr>;
```

### Split / Authoring

```rust
#[tauri::command]
async fn run_rule_split(job_id: String, state: State<'_, AppState>) -> Result<SplitCandidates>;

#[tauri::command]
async fn save_split_adjustments(job_id: String, patch: SplitPatch, state: State<'_, AppState>) -> Result<SplitCandidates>;

#[tauri::command]
async fn build_authoring_ir(job_id: String, state: State<'_, AppState>) -> Result<ReadingAuthoringIr>;

#[tauri::command]
async fn update_authoring_ir(job_id: String, patch: AuthoringPatch, state: State<'_, AppState>) -> Result<ReadingAuthoringIr>;

#[tauri::command]
async fn render_group_html(job_id: String, group_id: String, state: State<'_, AppState>) -> Result<GroupRenderResult>;
```

### LLM

```rust
#[tauri::command]
async fn list_llm_profiles(state: State<'_, AppState>) -> Result<Vec<LlmProfilePublic>>;

#[tauri::command]
async fn save_llm_profile(input: SaveLlmProfileInput, state: State<'_, AppState>) -> Result<LlmProfilePublic>;

#[tauri::command]
async fn test_llm_profile(profile_id: String, state: State<'_, AppState>) -> Result<LlmTestResult>;

#[tauri::command]
async fn llm_classify_group(job_id: String, group_id: String, profile_id: String, state: State<'_, AppState>) -> Result<LlmSuggestion>;

#[tauri::command]
async fn llm_extract_group(job_id: String, group_id: String, profile_id: String, state: State<'_, AppState>) -> Result<LlmSuggestion>;

#[tauri::command]
async fn apply_llm_suggestion(job_id: String, suggestion_id: String, selected_paths: Vec<String>, state: State<'_, AppState>) -> Result<ReadingAuthoringIr>;
```

### Validation / Preview / Export

```rust
#[tauri::command]
async fn validate_authoring_ir(job_id: String, state: State<'_, AppState>) -> Result<ValidationReport>;

#[tauri::command]
async fn generate_preview_assets(job_id: String, state: State<'_, AppState>) -> Result<PreviewAssets>;

#[tauri::command]
async fn run_preview_e2e(job_id: String, state: State<'_, AppState>, app: AppHandle) -> Result<ValidationReport>;

#[tauri::command]
async fn export_reading_assets(job_id: String, export_dir: String, state: State<'_, AppState>) -> Result<ExportResult>;

#[tauri::command]
async fn build_pack(input: BuildPackInput, state: State<'_, AppState>) -> Result<PackBuildResult>;
```

## Rust 服务伪代码

### 创建任务

```rust
async fn create_import_job(input: CreateJobInput, state: &AppState) -> Result<ImportJob> {
    let job_id = generate_job_id();
    let job_dir = state.paths.job_dir(&job_id);

    fs::create_dir_all(job_dir.join("uploads"))?;
    fs::create_dir_all(job_dir.join("preview"))?;
    fs::create_dir_all(job_dir.join("exports"))?;

    let job = ImportJob {
        job_id,
        title: input.title.unwrap_or("Untitled Reading".into()),
        status: JobStatus::Draft,
        category: input.category,
        frequency: input.frequency,
        tags: input.tags,
        source_files: vec![],
        active_llm_profile_id: input.llm_profile_id,
        created_at: now(),
        updated_at: now(),
        current_step: WorkflowStep::Upload,
        issue_counts: IssueCounts::default(),
    };

    state.store.save_job(&job).await?;
    state.audit.append(&job.job_id, "job.created", &job).await?;
    Ok(job)
}
```

### 导入文件

```rust
async fn import_source_file(job_id: String, file_path: PathBuf, role: SourceFileRole, state: &AppState) -> Result<SourceFile> {
    let job_dir = state.paths.job_dir(&job_id);
    ensure_inside_app_data(&job_dir, &state.paths)?;

    let original_name = file_path.file_name_as_string()?;
    let file_type = detect_file_type(&file_path)?;
    validate_allowed_source_type(file_type)?;

    let sha256 = hash_file(&file_path).await?;
    let stored_name = format!("{}-{}", short_hash(&sha256), sanitize_filename(&original_name));
    let dest = job_dir.join("uploads").join(&stored_name);

    fs::copy(&file_path, &dest).await?;

    let source = SourceFile {
        file_id: generate_id("file"),
        original_name,
        stored_name,
        file_type,
        sha256,
        size_bytes: fs::metadata(&dest).await?.len(),
        role,
        imported_at: now(),
    };

    state.store.add_source_file(&job_id, source.clone()).await?;
    state.audit.append(&job_id, "file.imported", &source).await?;
    Ok(source)
}
```

### 调用 Python parser sidecar

```rust
async fn parse_document(job_id: String, options: ParseOptions, state: &AppState, app: &AppHandle) -> Result<DocumentIr> {
    let job = state.store.get_job(&job_id).await?;
    let main_file = job.main_source_file()?;
    let input_path = state.paths.job_upload_path(&job_id, &main_file.stored_name);
    let output_path = state.paths.job_dir(&job_id).join("document-ir.json");

    state.store.set_status(&job_id, JobStatus::Uploaded).await?;
    app.emit("job-progress", ProgressEvent::started(&job_id, "parse_document"))?;

    let args = vec![
        "parse".into(),
        "--input".into(), input_path.to_string_lossy().to_string(),
        "--output".into(), output_path.to_string_lossy().to_string(),
        "--mode".into(), options.mode.to_string(),
    ];

    let result = state.sidecars.python_parser.run(args).await?;
    if !result.success {
        return Err(AppError::SidecarFailed(result.stderr));
    }

    let ir: DocumentIr = read_json(&output_path).await?;
    validate_document_ir(&ir)?;

    state.store.save_document_ir(&job_id, &ir).await?;
    state.store.set_status(&job_id, JobStatus::Parsed).await?;
    app.emit("job-progress", ProgressEvent::finished(&job_id, "parse_document"))?;
    Ok(ir)
}
```

### 规则粗切

```rust
async fn run_rule_split(job_id: String, state: &AppState) -> Result<SplitCandidates> {
    let doc = state.store.get_document_ir(&job_id).await?;

    let passage = splitter::detect_passage(&doc);
    let groups = splitter::detect_question_groups(&doc);
    let answers = splitter::detect_answer_key(&doc);

    let candidates = SplitCandidates {
        job_id: job_id.clone(),
        passage_candidates: passage,
        question_group_candidates: groups,
        answer_key_candidates: answers,
        issues: splitter::collect_issues(&doc),
    };

    state.store.save_split_candidates(&job_id, &candidates).await?;
    state.audit.append(&job_id, "split.generated", &candidates.summary()).await?;
    Ok(candidates)
}
```

### LLM 调用

```rust
async fn llm_classify_group(job_id: String, group_id: String, profile_id: String, state: &AppState) -> Result<LlmSuggestion> {
    let profile = state.settings.get_llm_profile(&profile_id).await?;
    let api_key = state.secrets.get(&profile.api_key_secret_ref).await?;
    let ir = state.store.get_authoring_ir(&job_id).await?;
    let group = ir.find_group(&group_id)?;

    let prompt = state.prompts.render("classify_group_v1", json!({
        "allowedKinds": GroupKind::allowed_values(),
        "group": group.to_llm_context()
    }))?;

    let raw = state.llm_client.complete_json(&profile, &api_key, prompt).await?;
    let parsed: ClassifyGroupOutput = parse_and_validate_json(raw.body)?;

    let suggestion = LlmSuggestion::from_classification(job_id.clone(), group_id, parsed, raw.usage);
    state.store.save_llm_suggestion(&job_id, &suggestion).await?;
    state.audit.append(&job_id, "llm.classify_group", &suggestion.redacted()).await?;
    Ok(suggestion)
}
```

### 生成 ReadingExamSourceV1

```rust
async fn export_reading_source(job_id: String, state: &AppState) -> Result<ReadingExamSourceV1> {
    let authoring = state.store.get_authoring_ir(&job_id).await?;
    validate_authoring_ir_or_fail(&authoring)?;

    let passage_html = template_service::render_passage(&authoring.passage)?;
    let question_groups = authoring.groups.iter()
        .map(|group| template_service::render_question_group(group))
        .collect::<Result<Vec<_>>>()?;

    let source = ReadingExamSourceV1 {
        schema_version: "ReadingExamSourceV1".into(),
        exam_id: authoring.exam.exam_id.clone(),
        meta: authoring.exam.to_runtime_meta(),
        passage: Passage {
            blocks: vec![HtmlBlock {
                block_id: "passage-main".into(),
                kind: "html".into(),
                html: passage_html,
            }]
        },
        question_groups,
        answer_key: authoring.answer_key.clone(),
        source_refs: SourceRefs::from_job(&job_id),
        audit: RuntimeAudit::author_verified(),
        question_order: authoring.question_order.clone(),
        question_display_map: authoring.question_display_map.clone(),
    };

    validation_service::validate_reading_source(&source)?;
    Ok(source)
}
```

### 生成预览资产

```rust
async fn generate_preview_assets(job_id: String, state: &AppState) -> Result<PreviewAssets> {
    let source = export_reading_source(job_id.clone(), state).await?;
    let preview_dir = state.paths.job_dir(&job_id).join("preview");

    let js = export_service::build_wrapper(&source);
    let manifest = export_service::build_manifest(vec![&source]);

    write_text(preview_dir.join(format!("{}.js", source.exam_id)), js).await?;
    write_text(preview_dir.join("manifest.js"), manifest).await?;

    let report = validation_service::validate_preview_assets(&preview_dir, &source.exam_id).await?;
    if !report.passed {
        return Err(AppError::ValidationFailed(report));
    }

    Ok(PreviewAssets {
        exam_id: source.exam_id,
        manifest_path: preview_dir.join("manifest.js"),
        script_path: preview_dir.join(format!("{}.js", source.exam_id)),
        preview_url: state.preview_server.url_for(&job_id, &source.exam_id).await?,
    })
}
```

### Pack 发布

```rust
async fn build_pack(input: BuildPackInput, state: &AppState) -> Result<PackBuildResult> {
    let pack_id = input.pack_id;
    let pack_dir = state.paths.pack_dir(&pack_id);
    fs::create_dir_all(pack_dir.join("reading-exams"))?;

    let mut sources = vec![];
    for job_id in input.job_ids {
        let source = export_reading_source(job_id.clone(), state).await?;
        let report = validation_service::run_full_validation(&source).await?;
        if !report.passed {
            return Err(AppError::ValidationFailed(report));
        }
        sources.push(source);
    }

    for source in &sources {
        write_text(
            pack_dir.join("reading-exams").join(format!("{}.js", source.exam_id)),
            export_service::build_wrapper(source)
        ).await?;
    }

    write_text(
        pack_dir.join("reading-exams").join("manifest.js"),
        export_service::build_manifest(sources.iter().collect())
    ).await?;

    write_json(pack_dir.join("pack.json"), &PackManifest::from_input(input, &sources)).await?;
    zip_dir(&pack_dir, state.paths.pack_zip_path(&pack_id)).await?;

    Ok(PackBuildResult { pack_id, output_path: state.paths.pack_zip_path(&pack_id) })
}
```

## 前端调用封装

```ts
import { invoke } from "@tauri-apps/api/core";

export async function createImportJob(input: CreateJobInput): Promise<ImportJob> {
  return invoke("create_import_job", { input });
}

export async function parseDocument(jobId: string, options: ParseOptions): Promise<DocumentIr> {
  return invoke("parse_document", { jobId, options });
}

export async function llmClassifyGroup(jobId: string, groupId: string, profileId: string): Promise<LlmSuggestion> {
  return invoke("llm_classify_group", { jobId, groupId, profileId });
}

export async function generatePreviewAssets(jobId: string): Promise<PreviewAssets> {
  return invoke("generate_preview_assets", { jobId });
}
```

长任务进度事件：

```ts
import { listen } from "@tauri-apps/api/event";

listen<ProgressEvent>("job-progress", (event) => {
  jobStore.updateProgress(event.payload.jobId, event.payload);
});
```

## LLM 设置与安全

### 设置界面字段

| 字段 | 默认 | 说明 |
|---|---|---|
| `name` | OpenAI Compatible | 配置名称 |
| `provider` | OpenAI Compatible | 模型供应商类型 |
| `baseUrl` | `https://api.openai.com/v1` | 可改成机构代理或本地服务 |
| `apiKey` | 空 | 只写入系统密钥存储 |
| `model` | 手动填 | 如 `gpt-4.1`、`claude`、`qwen`、本地模型名 |
| `temperature` | `0` | 导题任务要稳定 |
| `timeoutMs` | `60000` | 单次请求超时 |
| `forceJson` | true | 只接受 JSON |
| `maxRetries` | 2 | JSON 解析失败重试 |

### 密钥存储

- API Key 不写进 `settings.json`。
- `settings.json` 只保存 `api_key_secret_ref`。
- macOS 用 Keychain，Windows 用 Credential Manager，或使用 Tauri Stronghold 类方案。
- 导出诊断包时必须脱敏 baseUrl query、headers、API Key。

### LLM Gateway 输出校验

```rust
async fn complete_json(profile, api_key, prompt) -> Result<JsonValue> {
    let response = http.post(profile.chat_url())
        .bearer_auth(api_key)
        .json(build_openai_compatible_payload(profile, prompt))
        .timeout(profile.timeout_ms)
        .send()
        .await?;

    let text = extract_message_text(response).await?;
    let json = parse_json_only(text)?;
    validate_against_task_schema(&json)?;
    Ok(json)
}
```

## 文件读写权限

### 允许

- App data 目录。
- 用户通过文件选择器明确选择的输入文件。
- 用户通过保存对话框明确选择的导出目录。

### 禁止

- 任意扫描用户磁盘。
- 直接把 API Key 写入普通日志。
- 让前端 Web 直接拼任意路径读文件。
- 让 LLM 接收未必要的整份文档和本地路径。

## 状态机

```mermaid
stateDiagram-v2
  [*] --> Draft
  Draft --> Uploaded: import_source_file
  Uploaded --> Parsed: parse_document
  Parsed --> SplitReady: run_rule_split
  SplitReady --> AuthoringReady: build_authoring_ir
  AuthoringReady --> NeedsHumanReview: llm suggestion / low confidence
  NeedsHumanReview --> AuthoringReady: human verifies
  AuthoringReady --> ValidationFailed: validate failed
  ValidationFailed --> AuthoringReady: fix issues
  AuthoringReady --> PreviewReady: generate_preview_assets
  PreviewReady --> ExportReady: run_preview_e2e passed
  ExportReady --> Published: build_pack/export
```

## 最小 MVP 范围

第一版只做这些：

1. Tauri 壳和本地数据目录。
2. LLM Profile 设置和测试连接。
3. 上传 PDF/DOCX 并创建 job。
4. 调 Python sidecar 输出 Document IR。
5. 手动粗切 Passage/题组/答案。
6. 支持 TFNG、YNNG、单选、文本填空、表格填空。
7. LLM 辅助题型分类和题组抽取。
8. Authoring IR 编辑器。
9. 模板渲染成当前统一阅读页兼容 `bodyHtml`。
10. 生成单题 JS 和 manifest。
11. 统一阅读页 iframe 预览。
12. 正确答案自动填入得 100% 的 E2E 校验。

第二版再做：

- 拖拽 matching/classification。
- summary word bank。
- diagram/flow-chart completion。
- Pack 组卷发布。
- 批量导入。
- 授权元数据和加密 Pack 对接。

## 与 Epic 8 工程任务映射

| Tauri 任务 | 对应 Epic 8 任务 |
|---|---|
| 壳与 app data | E8-ENG-03 |
| PDF/DOCX parser sidecar | E8-ENG-04 |
| 粗切服务 | E8-ENG-05 |
| LLM Gateway 和设置 | E8-ENG-06、E8-ENG-07 |
| Authoring IR 编辑器 | E8-ENG-01、E8-ENG-09 |
| 模板渲染器 | E8-ENG-08 |
| DOM 协议校验器 | E8-ENG-11 |
| 统一阅读页预览 | E8-ENG-12 |
| 自动填答案 E2E | E8-ENG-13 |
| Pack 发布 | E8-ENG-14 |
| 审计记录 | E8-ENG-15 |

## 开发顺序

1. 先做 Tauri shell、设置页、LLM Profile 测试。
2. 再做 job 创建、文件导入、app data 目录。
3. 接 parser sidecar，跑通 PDF/DOCX 到 Document IR。
4. 做手动粗切和 answerKey 编辑。
5. 定义并落地 Authoring IR。
6. 做模板渲染器和单题 JS 导出。
7. 做统一阅读页预览和 E2E。
8. 接入 LLM 分类/抽取/diff。
9. 做复杂题型编辑器。
10. 做 Pack 组卷发布。


---

## 最新需求覆盖记录：2026-05-31 18:34 CST

> 本节为当前最新产品决策。执行 Agent 应优先遵循本节；若与上文早期设计中的 SQLite、长期保留原始上传文件、本地 OCR fallback、后端立即模块拆分等描述冲突，以本节为准。

### 产品形态边界

- 作者端是本地 Tauri 桌面应用；前端只是内嵌在 Tauri 中的操作界面，不再规划独立 Web 作者端。
- 旧 Web 导题文档继续作为 `ReadingExamSourceV1`、DOM 协议、JS/manifest/Pack 输出契约参考，不作为独立 Web 产品实现目标。
- 当前 MVP 的核心价值是“PDF/DOCX -> 可编辑结构化题目稿 -> 校验 -> JS/Pack 导出”，不是长期题库管理系统。

### MVP 存储策略：暂不引入 SQL

MVP 不建立 SQLite/SQL 数据库。当前阶段使用 app data 下的文件式工作区即可，理由如下：

- 用户当前主要使用场景是单次或少量批次转换，不需要一开始就做题库级查询、筛选和全文搜索。
- SQL 不应成为保存大量中间过程文件的容器；否则会把调试缓存、原始副本、LLM raw log 等临时数据长期固化，增加包袱。
- 未来如果需要管理几百到几千套题，再引入 SQLite 作为索引层，而不是过程数据仓库。

未来 SQLite 只允许索引这些轻量字段：

- 题目/套题 metadata、标签、分类、难度、状态。
- 可搜索文本摘要或全文索引字段。
- 可编辑稿路径、最近导出路径、导出历史摘要。
- source 摘要信息：原文件名、类型、hash、导入时间、审核摘要。

未来 SQLite 不应保存这些内容：

- 原始 PDF/DOCX 文件副本。
- parser cache、preview assets、临时图片、渲染页图。
- LLM 原始请求/响应全文、调试日志全文。
- `document-ir.json`、`split-candidates.json`、pipeline 大型过程 JSON 的长期版本。

### 任务生命周期

当前导题任务采用面向产品的生命周期，而不是把所有内部步骤都暴露给普通用户：

```mermaid
stateDiagram-v2
  [*] --> Working
  Working --> NeedsReview: parser warning / low confidence / vision transcription / LLM blocked / validation issue
  NeedsReview --> Working: user edits or confirms
  Working --> DraftSaved: editable authoring draft saved
  DraftSaved --> ExportReady: validation + DOM + runtime gate passed
  ExportReady --> Exported: JS or Pack generated
  Exported --> Cleaned: auto cleanup transient artifacts
  Cleaned --> DraftSaved: user reopens editable draft
```

| 状态 | 含义 | 用户视角 |
|---|---|---|
| `Working` | 导入、解析、粗切、LLM 识别、结构化稿生成中的工作态 | 系统自动推进，必要时显示进度和风险 |
| `NeedsReview` | 存在低置信、图片 PDF 转录、LLM 建议被阻断、答案/题型/DOM 校验问题 | 用户必须审核或修正 |
| `DraftSaved` | 已形成可再次编辑的结构化题目稿 | 用户可退出、回来继续改 |
| `ExportReady` | 可编辑稿通过发布门禁 | 用户可导出 JS 或组 Pack |
| `Exported` | 已成功生成 JS/manifest 或 Pack | 系统记录导出摘要 |
| `Cleaned` | 导出后已自动清理过程态文件，只保留长期有价值内容 | UI 提示“中间文件已自动清理，已保留可编辑题目稿” |

### 自动清理策略

导出 JS 或 Pack 成功后，默认自动执行清理。普通用户不需要手动点击“清理工作区”。UI 只需要给出轻量提示：

> 中间文件已自动清理，已保留可编辑题目稿。

长期保留：

- `authoring-project.json`，作为最小可编辑项目稿，包含或引用 `ReadingAuthoringIRV1`。
- 最小 metadata：标题、分类、标签、难度、题号范围、更新时间、状态。
- source summary：原文件名、类型、sha256、大小、导入时间；不保留原始文件副本。
- review summary：哪些低置信/视觉转录/LLM 建议已由人工确认。
- export summary：导出时间、导出目录、examId、PackId、校验摘要。

导出成功后默认删除：

- `uploads/` 中的原始 PDF/DOCX/答案文件副本。
- `cache/`、`preview/`、临时渲染图、临时图片。
- `document-ir.json`、`split-candidates.json`、大型 `pipeline-report.json`。
- `llm-suggestions/`、`llm-calls.jsonl`、LLM 原始请求响应缓存。
- `vision-transcription-output.json`、`vision-transcription.txt`、`manual-transcription.txt`。
- 可由可编辑稿重新生成的 validation/runtime 中间报告；长期只保留摘要。

开发者调试选项：

- 设置页增加 `开发者 / 诊断` 入口。
- `保留完整过程文件` 默认关闭。
- 开启后允许保留 uploads/cache/LLM raw log/pipeline report，便于排查解析与模型问题。
- 该入口不作为普通业务流程的一部分，不在主工作台要求用户频繁操作。

### PDF 解析与 OCR/视觉模型策略

当前产品不默认打包重量级本地 OCR。图片型 PDF、扫描 PDF 的自动化路径由视觉大模型承担，人工审核仍是发布前硬门禁。

调研依据：

- Tauri v2 支持通过 `externalBin` 打包外部二进制 sidecar，也支持把额外文件作为 resources 打包，用于避免用户自行安装 Node/Python 等依赖。参考：[Tauri sidecar](https://v2.tauri.app/develop/sidecar/) 与 [Tauri resources](https://v2.tauri.app/develop/resources/)。
- `pypdf` 可抽取 PDF 文本层，但官方明确说明它不是 OCR，无法从图片中抽取文字，且 PDF 本身缺少语义层，表格/段落/页眉页脚只能启发式判断。参考：[pypdf text extraction](https://pypdf.readthedocs.io/en/3.17.0/user/extract-text.html)。
- `pdfium-render` 可通过 Rust 绑定 PDFium，支持页面渲染、文本和图片抽取；它适合作为未来 rendered-page adapter，把扫描页渲染成图片后交给视觉 LLM。参考：[pdfium-render docs](https://docs.rs/pdfium-render/latest/pdfium_render/)。
- `pdf-extract` 是纯 Rust PDF 文本抽取库，可作为摆脱宿主 Python/pypdf 的候选，但只解决文本层抽取，不解决图片 OCR。参考：[pdf-extract docs](https://docs.rs/crate/pdf-extract/latest)。
- MuPDF/ PyMuPDF 能力强，但 MuPDF 官方采用 AGPL 或商业许可；若闭源分发，需要先处理许可问题，不作为 MVP 默认依赖。参考：[MuPDF license](https://mupdf.readthedocs.io/en/1.26.9/license.html)。

推荐实施路线：

1. MVP 继续支持当前 Python `pypdf` parser，但把它视为开发期/过渡实现；Settings preflight 必须提示宿主依赖状态。
2. 生产打包优先评估纯 Rust 文本 PDF 抽取适配器，例如 `pdf-extract`、`pdf_text_extract` 或同类轻量 crate，用于清晰文本 PDF，减少对宿主 Python 的依赖。
3. DOCX 继续使用当前 stdlib OOXML 解析或改为 Rust zip/xml 解析，目标同样是减少宿主 Python。
4. 图片 PDF 不引入 Tesseract/Docling/MinerU 等重量级本地 OCR 作为默认包内依赖。
5. 图片 PDF 优先走视觉 LLM：检测无文本或低置信 PDF -> 抽取嵌入图片或渲染页面图 -> 发送视觉模型转录 -> 生成 Document IR -> 强制 `NeedsReview` 人工确认。
6. 对 `pypdf` 无法暴露嵌入图片的扫描页，后续可增加可选 `pdfium-render` adapter：只负责把页面渲染成图片，不做本地 OCR，再交给视觉 LLM。
7. 视觉模型失败、无网络、无可用 LLM profile 时，保留人工转录兜底。

### 包体与依赖原则

- 当前本机已构建 macOS 产物约为 `.app` 11 MB、`.dmg` 3.5 MB；这是因为 Node/Python/pypdf 仍依赖宿主环境，并未完整打入安装包。
- 如果把完整 Node、Python、OCR 引擎和模型都打包进应用，包体和维护复杂度会显著上升，不符合当前 MVP 范围。
- 生产化优先级应是：轻量 Rust 文本解析 > 视觉 LLM 处理图片 PDF > 可选 PDFium 渲染 adapter > 明确 preflight/安装指引。
- 不应在 MVP 默认打包重量级 OCR 模型或完整文档智能框架。

### Rust 后端单文件策略

`src-tauri/src/lib.rs` 当前较大，但在 MVP 阶段暂不作为阻塞项。原因：

- 当前更重要的是先稳定业务闭环、生命周期、自动清理、导出门禁和依赖策略。
- 后端拆分应在核心产品状态达到生产级后进行，避免在需求仍快速变化时过早抽象。
- 后续若继续新增 rendered-page adapter、题库管理、SQLite 索引、复杂题型编辑，再拆分为 storage/parser/pipeline/llm/validator/exporter/pack/settings 等模块。


### 2026-05-31 CST 最新需求覆盖说明：Rust 主链路与诊断依赖边界

本节覆盖“不要把 Node、Python、OCR 引擎一起打进包体”的最新工程决策。若与上文早期 sidecar / OCR / real-runtime hard gate 描述冲突，以本节为准。

- 生产主链路尽量 Rust 化：TXT/MD、文本层 PDF、DOCX、LLM HTTP 调用、ReadingExamSourceV1/DOM 静态合同校验、导出与 Pack 均应由 Rust 主程序承担。
- Node 不进入生产硬依赖：旧 LLM gateway、node-validator、preview E2E 仅保留为开发/CI/诊断资源。普通用户机器不应因为没有 Node 而无法导入、LLM 识别、校验、导出或组 Pack。
- Python 不进入生产硬依赖：Python parser 只作为 legacy fallback 或嵌入图片抽取辅助路径。清晰文本 PDF/DOCX/TXT/MD 不依赖 Python。
- 本地 OCR 引擎不进入默认包体：图片型 PDF 或扫描 PDF 通过页面图/嵌入图 -> 视觉 LLM 转录 -> `DocumentIRV1` -> `SourceReview` 人工确认。视觉转录不等同于人工确认。
- macOS MVP 可使用系统 `sips` 渲染页面图作为视觉 LLM 输入。未来 Windows/Linux 需要时再评估 PDFium page-render adapter；该 adapter 只负责渲染页面，不做本地 OCR。
- 生产发布门禁为 Rust 静态合同 gate + SourceReview/AuthoringReview。真实 unified runtime E2E 是显式诊断/CI 命令，不作为普通导出/Pack 的硬依赖。
- 如果未来必须在本地生产环境跑真实 runtime E2E，应优先使用 Tauri WebView/内嵌 JS 可控方案，而不是要求用户安装 Node。

### 2026-06-01 10:37:59 CST 最新需求覆盖说明：复杂 PDF/DOCX 切分与题型分类增强

本节覆盖复杂版式 PDF/DOCX 的下一阶段优先需求。当前核心自动化闭环已经形成，下一最高优先级不是继续增加人工核验负担，而是提升复杂文档的自动切分、阅读顺序恢复和题型分类准确度。

#### 产品原则

- `SourceReview` 的“一键确认”是正确产品方向。它表示作者对源文档解析风险已做整体确认，不应强制用户逐条勾选 parser warning、低置信 block 或视觉转录项。
- 高置信 LLM 建议进入草稿后，不再要求逐题强制核验。人工操作原则是：作者在 LLM Review 中点击 Apply 或在草稿中做必要修订，然后进行整体确认。
- 高置信自动应用仍然只是进入可编辑草稿，不等于绕过发布门禁。导出/Pack 仍必须通过 Rust 静态合同 gate、SourceReview、AuthoringReview 和整体人工确认。
- 低置信、题型不确定、选项复用规则不确定、题干缺失或切分顺序不确定的部分进入人工修订，但人工介入应聚焦于修正草稿，不做机械 checklist。

#### 复杂版式输入范围

复杂 PDF/DOCX 增强需要预先覆盖以下情况：

- 双栏 passage、双栏题目区、左右栏混排。
- 横向页面、旋转页面、页面方向不一致。
- passage、题组、选项或答案跨页延续。
- PDF 文本抽取顺序与视觉阅读顺序不一致。
- 题干、选项、表格和答案分散在多个 block 中。
- 表格题、匹配题、heading matching、classification、summary/table/sentence completion 等题型中的结构信息不连续。
- Word/DOCX 中通过表格、分栏、缩进、编号列表实现的视觉结构。

#### 切分算法方向

下一阶段不应只依赖 `Questions x-y` 正则作为题组边界。推荐实现一个 layout-aware split pipeline：

1. **Block normalization**
   - 保留 page index、bbox、block type、role hint、confidence、font/line 信息（可得时）。
   - 对旋转页和横向页做坐标标准化，记录 orientation。
   - 对 DOCX 的段落、表格、列表项转成统一 block 序列。

2. **Reading order reconstruction**
   - 基于 bbox 聚类检测列布局，先按页分组，再按列内从上到下、列间从左到右恢复阅读顺序。
   - 双栏页面不能简单按 PDF extractor 输出顺序处理。
   - 对跨页 continuation 识别页尾/页首相邻题组或 passage。

3. **Semantic section graph**
   - 将 block 组织为 passage nodes、question-heading nodes、question-prompt nodes、option nodes、table nodes、answer nodes。
   - 边的依据包括空间邻近、题号连续、heading range、选项编号/字母、表格位置、跨页延续。
   - 规则切分输出应携带置信度和不确定原因，供 AuthoringReview 聚焦展示。

4. **Fallback behavior**
   - 当阅读顺序或结构无法可靠恢复时，生成低置信草稿而不是失败或伪造内容。
   - 若只识别到总题组范围（例如 `Questions 14-26`），继续保留为 umbrella metadata，并生成 manual import scaffold。
   - 复杂 PDF 的失败兜底仍是可编辑草稿 + 人工修订，而不是强制逐项审核。

#### 题型分类增强

题型分类需要从“粗略 kind”升级为“题型 + 交互 + 选项复用规则 + 证据”的组合判断。

- 先使用确定性规则分类，规则不足时再调用 LLM classifier。
- LLM classifier 只输出结构化 JSON，不直接生成最终 JS。
- 分类输出至少应包含：
  - `kind`
  - `interaction.type`
  - `questionRange`
  - `optionSet`
  - `allowOptionReuse`
  - `maxSelections` / `minSelections`（多选题需要）
  - `confidence`
  - `evidence.sourceBlockIds`
  - `warnings`

需要重点区分：

- 单选：`Choose the correct letter, A, B, C or D`，通常 `radio`，不可多选。
- 多选：`Choose TWO/THREE letters`，通常 `checkbox`，需要 `minSelections=maxSelections=N`。
- 匹配题：根据题干判断是否可重复使用选项。
- Heading matching：通常不可重复，除非题干明确允许。
- Classification：经常可能复用选项，尤其题干出现 `You may use any letter more than once`。
- Summary/table/sentence completion：需要识别字数限制、空格数量、表格行列关系。
- True/False/Not Given 与 Yes/No/Not Given：需要区分事实判断和作者观点判断。

选项复用规则必须被显式建模：

- 题干出现 `You may use any letter more than once`、`may be used more than once` 时，设置 `allowOptionReuse=true`。
- 题干出现 `each option may be used once only`、`use each letter once only` 时，设置 `allowOptionReuse=false`。
- 未明确时，根据题型默认值推断，但 confidence 应降低并在 warnings 中说明。

#### 工程约束

- 复杂 PDF/DOCX 增强是下一工程任务最高优先级。
- 生产主链路继续 Rust-first：解析、切分、校验、导出和 Pack 不依赖 Node/Python 作为硬前置。
- 不引入重量级本地 OCR。图片型 PDF 继续走视觉 LLM + SourceReview。
- 不把用户测试 PDF 纳入 git；测试样本可保留本地或使用合成 fixture。
- 对真实复杂样本应建立 fixture 分级：文本层 PDF、双栏 PDF、旋转 PDF、DOCX 表格/分栏、扫描/图片 PDF。
- 每个 fixture 的验收不是“完美识别所有内容”，而是：
  - 能恢复合理 reading order；
  - 能产出可编辑草稿；
  - 能正确标记低置信/不确定项；
  - 能避免生成错误的高置信可发布结果；
  - 导出前必须经过现有发布门禁。
