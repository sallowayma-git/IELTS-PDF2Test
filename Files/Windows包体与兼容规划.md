# Windows 包体与兼容规划

## 执行追踪

- 追踪任务书：[Windows包体与兼容任务追踪.md](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/Files/Windows包体与兼容任务追踪.md)
- 当前状态：`complete`
- 当前批次：W1-W10 实现与本机验证已完成；真实 NSIS/MSI 产物、签名和 WebView2 offline installer 需在 Windows runner 上验证。
- 默认边界保持不变：Windows 初版不打包 Node、Python、Tesseract、本地 OCR 或离线 WebView2 Runtime；offline WebView2 只作为独立发行配置。

## 目标

为 `IELTS Author Studio` 规划 Windows 版 EXE/NSIS/MSI 分发路径，提前明确包体依赖、扫描 PDF 处理策略、平台兼容改造点和后续开发任务。

本轮边界：

- 不强制把本地 OCR / OCL / Tesseract / Python runtime 打包进应用。
- 优先保持小包体，扫描 PDF 继续走“页面图片提取/渲染 + 云端 vision LLM + 人工确认”的产品路径。
- Windows 版应允许普通 TXT/MD、文字层 PDF、DOCX、LLM 辅助、导出和 Pack 构建不依赖 Node/Python/OCR。

## 当前包体事实

当前 macOS 产物：

- `.app` 约 `18 MB`
- `.dmg` 约 `6.6 MB`
- `sidecars` 约 `136 KB`

当前配置：

- [src-tauri/tauri.conf.json](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/src-tauri/tauri.conf.json:26) 中 `bundle.externalBin` 为空。
- [src-tauri/tauri.conf.json](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/src-tauri/tauri.conf.json:30) 只把 `../sidecars` 作为资源打包。
- [scripts/package-audit.mjs](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/scripts/package-audit.mjs:49) 明确阻止生产包包含 Node、Python、Tesseract、PDFium、OCR engine 等 runtime。

结论：当前包体小，是因为没有打包浏览器、Node、Python、OCR 或 PDF 渲染运行时。Windows 版只要继续维持这个边界，包体不会天然膨胀到很大。

## Windows 分发依赖评估

### Tauri / WebView2

Windows Tauri 桌面应用依赖 Microsoft Edge WebView2 Runtime。Windows 分发有几种策略：

1. `downloadBootstrapper`
   - 安装包体积最小。
   - 用户机器缺 WebView2 时，安装器联网下载 bootstrapper/Runtime。
   - 离线、代理、校园网环境可能失败。

2. `embedBootstrapper`
   - 安装包约增加 `1.8 MB` 量级。
   - bootstrapper 随安装器走，但 WebView2 Runtime 本体仍联网获取。
   - 对体积影响小，用户体验比纯下载 bootstrapper 更稳定。

3. `offlineInstaller`
   - 内置 Evergreen Standalone Installer。
   - 包体约增加 `127 MB` 量级。
   - 适合学校机房、企业内网、U 盘离线分发。

4. `fixedRuntime`
   - WebView2 Runtime 固定版本随应用分发。
   - Tauri/Microsoft 文档显示可能增加 `180 MB+`，固定版本 binaries 甚至可能超过 `250 MB`。
   - 安全更新责任转移到我们自己，不建议默认使用。

5. `skip`
   - 不安装也不检查 WebView2。
   - 仅适合客户 IT 明确保证 WebView2 已预装的受管环境。

建议：第一版 Windows 默认使用 `embedBootstrapper` 或维持 Tauri 默认的 `downloadBootstrapper`。如果要面向学校/企业弱网环境，单独做 `offlineInstaller` 发行通道，不要污染默认小包体。

### 代码签名 / SmartScreen

Windows 不会有 macOS Gatekeeper，但会有 SmartScreen/未知发布者提示。未签名 EXE 可以运行，但用户体验会有安全警告。

建议：

- 内测阶段可以先不签名，但说明文档要写清楚。
- 对外分发前准备 Authenticode 代码签名证书。
- CI/release 脚本预留签名步骤，不要把签名和业务构建揉死。
- 新证书或新应用早期仍可能出现 SmartScreen 提示，EV 证书或 Azure Trusted Signing 更利于积累信誉。

## OCR / 扫描 PDF 策略

当前扫描件路径不是本地 OCR，而是：

1. Rust 优先解析 TXT/MD、文字层 PDF、DOCX。
2. 扫描 PDF 或低置信 PDF 尝试提取/渲染页面图片。
3. 页面图片交给 OpenAI-compatible vision LLM 转写。
4. 结果进入 SourceReview/AuthoringReview，人工确认后发布。

关键代码：

- [src-tauri/src/parser.rs](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/src-tauri/src/parser.rs:430) `parse_pdf_with_rust_text_extractor` 使用 Rust `pdf_extract::extract_text_by_pages`，只适合文字层 PDF。
- [src-tauri/src/parser.rs](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/src-tauri/src/parser.rs:1233) `parse_with_python_sidecar` 当前调用 `python3`。
- [src-tauri/src/parser.rs](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/src-tauri/src/parser.rs:1260) `extract_pdf_images_with_python_sidecar` 当前调用 `python3` + sidecar。
- [src-tauri/src/parser.rs](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/src-tauri/src/parser.rs:1365) `render_pdf_pages_with_adapter` 当前直接落到 macOS `sips`。
- [src-tauri/src/parser.rs](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/src-tauri/src/parser.rs:1375) `render_pdf_pages_with_macos_sips` 是 macOS-only。
- [src-tauri/src/auto_pipeline.rs](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/src-tauri/src/auto_pipeline.rs:190) `main_pdf_vision_extraction` 依赖上述提取/渲染结果。
- [sidecars/python-parser/parser.py](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/sidecars/python-parser/parser.py:300) `render_pdf_pages_for_vision` 已有 `PyMuPDF -> Poppler pdftoppm -> macOS sips` 的尝试顺序，但 [sidecars/python-parser/parser.py](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/sidecars/python-parser/parser.py:329) `extract_pdf_images` 先要求 `pypdf`，没有 Python/pypdf 时整条 sidecar 路径会被挡住。

Windows 第一版策略：

- 不打包 Tesseract、语言包、Python、OCL。
- 清晰文字层 PDF/DOCX/TXT/MD 完整支持。
- 扫描 PDF：
  - 若已能提取嵌入图片，则继续走 vision LLM。
  - 若需要页面渲染，而 Windows renderer 尚未实现，则进入人工转写/提示“扫描 PDF 需要后续 Windows PDF renderer 支持”。
  - 若目标 provider 支持直接 PDF file input，可新增 direct PDF cloud vision 路线，不经过本地图像渲染。

后续增强建议：

1. 优先补 PDFium renderer
   - 目标是“把 PDF 页面渲染成图片”，不是本地 OCR。
   - 继续复用云端 vision LLM。
   - 包体可控，能力闭环更干净。

2. direct PDF cloud vision
   - 本地包体最小。
   - 复用现有 PDF data URL 能力，但需要确认 provider 支持 PDF 文件输入。
   - 适合作为 Windows 无 renderer 时的轻量增强。
   - 需要 UI 明示隐私、费用和模型兼容风险。

3. Poppler / PyMuPDF
   - Poppler 成熟但 Windows DLL/许可证/体积和工具链更重。
   - PyMuPDF 接入快，但生产若不打包 Python runtime，则交付不稳定。
   - 更适合作为开发诊断或用户自带环境的可选路径。

4. 不优先补 Tesseract
   - Tesseract + 语言包会显著增大包体。
   - 它只是 OCR，不是 PDF 页面渲染；仍需要 PDFium/Poppler/PyMuPDF 先把页面转图。
   - IELTS 版面恢复和题型结构仍需要大量后处理。
   - 离线 OCR 是第二阶段产品决策，不应混入第一版 Windows 包体。

## 需要改造的平台兼容点

### 1. 平台化 PDF renderer adapter

当前 `render_pdf_pages_with_adapter` 是一个假抽象，实际固定调用 macOS `sips`。

建议改造：

- `render_pdf_pages_with_adapter(...)`
  - macOS: 调用 `render_pdf_pages_with_macos_sips`
  - Windows: 调用 `render_pdf_pages_with_windows_pdfium`，未实现时返回结构化错误
  - 其他平台: 返回 unsupported renderer

预留配置：

- `EPIC8_PDF_RENDERER=auto|none|sips|pdfium|poppler|pymupdf`
- `EPIC8_ENABLE_CLOUD_PDF_VISION=0|1`
- `EPIC8_ENABLE_LOCAL_OCR=0|1`，默认 `0`

同时把 `PdfImageExtractionV1` 能力字段稳定下来：

- `rendererProvider`
- `rendererVersion`
- `pageCount`
- `renderedPageCount`
- `dpi`
- `ocrPerformed=false`
- `failureReason`
- `requiresManualReview`

### 2. Python 命令探测

当前写死 `python3`。Windows 常见命令是 `py` 或 `python`。

建议：

- 增加 `resolve_python_command()`：
  - 环境变量 `EPIC8_PYTHON`
  - macOS/Linux: `python3`, `python`
  - Windows: `py -3`, `python`
- 仍然把 Python 标记为 optional，不作为生产 blocker。

相关开发诊断脚本也要处理 Windows 命令差异：

- `npm` 在 Windows 常见为 `npm.cmd`。
- Node 脚本中 spawn npm/Node/Python 时要有平台分支或统一 command resolver。

### 3. 环境预检改造

当前 [src-tauri/src/environment.rs](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/src-tauri/src/environment.rs:232) 固定检查 `renderer:macos-sips`。

建议输出平台化检查：

- `renderer:pdf-page-renderer`
- `renderer:macos-sips` 仅 macOS 显示
- `renderer:windows-pdfium` Windows 显示
- `ocr:local` 默认 disabled，说明未打包本地 OCR
- `vision:cloud` 检查 LLM profile/API key 是否可用
- LLM profile 增加 capability 字段：
  - `supportsVisionImages`
  - `supportsPdfFileInput`
  - `maxPdfBytes`
  - `maxVisionImages`

### 4. Release scripts 拆分

当前 [package.json](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/package.json:17) 的 `verify:release` 固定执行 `repack:macos-dmg`，Windows 会失败。

建议：

- `verify:release:macos = npm run tauri build -- --bundles app,dmg && npm run repack:macos-dmg && npm run audit:package:macos`
- `verify:release:windows = npm run tauri build -- --bundles nsis && npm run audit:package:windows`
- `verify:release:windows:offline = npm run tauri build -- --bundles nsis --config src-tauri/tauri.windows.offline.conf.json && npm run audit:package:windows`
- `audit:package` 拆为平台化入口。

另外需要把 JS 脚本里的 `new URL(...).pathname` 改成 `fileURLToPath(import.meta.url)`，否则 Windows 下会出现 `/C:/...` 形式的错误路径。涉及：

- [scripts/package-audit.mjs](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/scripts/package-audit.mjs:4)
- [scripts/repack-macos-dmg.mjs](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/scripts/repack-macos-dmg.mjs:5)
- [sidecars/ui-flow-e2e/ui-flow-e2e.mjs](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/sidecars/ui-flow-e2e/ui-flow-e2e.mjs:9)

### 5. Windows package audit

当前 package audit 只检查 macOS `.app/.dmg`。

Windows 需要新增：

- 确认 `src-tauri/target/release/bundle/nsis/*setup.exe` 或 `bundle/msi/*.msi` 存在。
- 确认未打包 Node/Python/Tesseract/OCR/PDFium，除非明确开启 feature。
- 确认 `sidecars` 仍只是小体积诊断资源。
- 输出包体大小和 WebView2 分发模式。
- 输出 artifact SHA-256、git commit、Tauri/Cargo/npm lock 版本。
- 可选签名审计：
  - PowerShell `Get-AuthenticodeSignature`
  - `signtool verify /pa /tw`
- 可选依赖审计：
  - `dumpbin /dependents`
  - Dependencies CLI

### 6. macOS-only 脚本隔离

以下脚本只属于 macOS 分发：

- [scripts/macos-open-unsigned.command](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/scripts/macos-open-unsigned.command:1)
- [scripts/macos-install-instructions.txt](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/scripts/macos-install-instructions.txt:1)
- [scripts/repack-macos-dmg.mjs](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/scripts/repack-macos-dmg.mjs:1)

Windows 不应引用这些脚本；后续如需 Windows 未签名说明，应单独创建 `windows-install-instructions.txt`。

### 7. 开发/测试脚本 Windows 兼容

以下不影响生产主路径，但会影响 Windows CI 和回归测试：

- [sidecars/ui-flow-e2e/ui-flow-e2e.mjs](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/sidecars/ui-flow-e2e/ui-flow-e2e.mjs:121) `findChrome` 目前缺 Windows Chrome/Edge 默认路径。
- [sidecars/ui-flow-e2e/ui-flow-e2e.mjs](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/sidecars/ui-flow-e2e/ui-flow-e2e.mjs:91) 直接 spawn `npm`，Windows 需要 `npm.cmd` 或 shell 包装。
- [sidecars/preview-e2e/preview-e2e.mjs](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/sidecars/preview-e2e/preview-e2e.mjs:551) Python resolver 偏 macOS/Linux，需要支持 `py -3`。
- [scripts/pdf-regression-sample.mjs](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/scripts/pdf-regression-sample.mjs:7) 默认数据路径绑定本机 macOS 目录；Windows 需要显式传参或改成 fixture 默认。
- [scripts/pdf-regression-sample.mjs](/Users/maziheng/Downloads/Desktop/copy/PDF2Test/scripts/pdf-regression-sample.mjs:44) CLI 二进制路径需要 Windows `.exe` 分支。

### 8. 安全写入和导出路径

自定义 Rust 命令会接收前端传回的导出目录路径并写文件。功能上可行，但自定义 Rust 写入不会自动受 Tauri FS scope 限制。

建议补：

- 验证导出目录存在且可写。
- 拒绝危险系统目录。
- 记录 dialog 返回过的路径，后续导出只接受该路径或用户重新选择。

## 初版 Windows 开发任务建议

### Phase 1: 构建与审计骨架

- 新增 `verify:release:windows`。
- 新增 `audit:package:windows`。
- 新增 `src-tauri/tauri.windows.offline.conf.json`，专供 `offlineInstaller`。
- 明确默认 WebView2 模式为 `embedBootstrapper` 或保留 Tauri 默认 `downloadBootstrapper`。
- 保留现有 macOS release 流程，不把 Windows 逻辑塞进 DMG repack。
- 输出 Windows 包体大小、WebView2 模式、runtime dependency summary。

### Phase 2: 运行时预检平台化

- 改造 `environment_preflight_report`。
- Windows 上不要显示 `macOS sips renderer unavailable` 这种误导信息。
- 明确显示“本地 OCR 未启用/未打包，扫描 PDF 使用云端 vision 或人工转写”。

### Phase 3: 扫描 PDF 降级路径

- 让 Windows renderer 未实现时返回可理解的产品状态。
- SourceReview 页面提示用户：
  - 文字层文件可继续自动生成。
  - 扫描 PDF 当前 Windows 版需要人工转写或后续 renderer。
- 不因缺少 Python/sips 造成硬失败。
- 新增 direct PDF cloud vision 的 feature flag 和 profile capability 检查。
- 将 cloud outline/direct PDF 调用从 `main_pdf_vision_extraction` 成功前提中解耦。

### Phase 4: PDFium renderer 预留

- 设计 `PdfPageRenderer` trait / adapter 接口。
- Windows 首选 PDFium 渲染页面图片。
- renderer 输出继续沿用 `PdfImageExtractionV1`，避免影响后续 vision LLM 和审计链路。

### Phase 5: Windows 签名与分发

- 内测：允许未签名，但写清 SmartScreen 处理说明。
- 对外：接入 Authenticode 签名。
- CI 中预留 `WINDOWS_SIGNING_CERT` / `WINDOWS_SIGNING_PASSWORD` / timestamp server 配置。
- 新增 `audit-windows-signatures.ps1` 和 release manifest。

### Phase 6: Windows CI smoke

- `windows-latest` 跑 `npm ci`、`npm run check`、`cargo test --manifest-path src-tauri/Cargo.toml`。
- 构建 NSIS/MSI。
- 验证 `keyring` Windows Credential Manager 保存/读取/删除 API key。
- 验证导入文字层 PDF、DOCX、导出目录选择、Pack 构建。
- 扫描 PDF 路径验证为“需要人工/云端 vision 未启用”，不能崩溃。

## 包体预测

不打包 OCR/Python/WebView2 离线包时：

- 预计 Windows setup 包体会接近当前 `.app` 压缩后的量级，可能是十几 MB 左右。
- 具体大小要以 Windows runner 实际 `nsis` / `msi` 产物为准。

使用 `embedBootstrapper` 时：

- 约增加 `1.8 MB` 量级。
- 仍需要联网安装 WebView2 Runtime。

使用 `offlineInstaller` 时：

- 约增加 `127 MB` 量级。
- 只建议给离线企业环境做单独发行渠道。

使用 `fixedRuntime` 时：

- 约增加 `180 MB+`，甚至可能到 `250 MB+`。
- 需要我们自己承担 runtime 更新和安全补丁节奏，不建议默认。

打包 PDFium renderer 时：

- 包体会增加，但通常远小于“Python + OCR + 语言包”的路线。
- 这是扫描 PDF 支持的优先增强方向。

打包 Tesseract/OCR 时：

- 包体和复杂度显著上升。
- 需要额外处理语言包、版面恢复、准确率评测、授权和安全审计。
- 不建议进入 Windows 初版。

## 推荐下一步

第一批开发不要碰本地 OCR。建议按这个顺序推进：

1. 先拆 release/audit 脚本，保证 macOS 和 Windows 两条构建线互不干扰。
2. 再做环境预检和扫描 PDF 降级提示，确保 Windows 版即使没有 renderer 也不硬失败。
3. 接着补 direct PDF cloud vision feature flag，给“不打 OCR 的扫描 PDF”一条轻量增强路。
4. 最后设计 PDFium renderer adapter，作为 Windows 扫描 PDF 完整体验的第二阶段。
