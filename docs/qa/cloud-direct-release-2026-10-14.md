# cloud-direct 主分支正式构建（2026-10-14）

`feat/cloud-direct` 的21个提交已 squash merge 到 `main` 并推送，产品源码提交为 `dade8fcbcc755c5b19b28c7f4ebc36782259d5ea`。源代码与已完成在线18/18、离线6/6验收的功能分支一致；额外补齐 Windows online/offline 配置中被数组覆盖的 `resources/cloud-direct` 打包项。

## 安装包

- 文件：[IELTS Author Studio 0.1.0 Windows x64 NSIS 安装包](<../../customer-delivery/IELTS Author Studio_0.1.0_x64-setup.exe>)。
- 大小：10,019,390 bytes（约10 MB）。
- SHA256：`cde47e586134e8a5e71293b98d705f5fb5e72b8c9014cb6f48f16d9bd6f605c9`。
- 构建命令：`npm run tauri -- build --bundles nsis --config src-tauri/tauri.windows.conf.json`。这是 release 优化构建，使用内嵌前端。
- Authenticode 状态：**NotSigned**，当前没有配置签名证书。缺少 WebView2 时采用默认联网 bootstrapper。
- 校验值、说明及精简证据同时保存在 `customer-delivery/`。

## 验证

1. Windows package audit 通过。
2. 生成的 `installer.nsi` 明确包含 PDFium DLL、system prompt 和三份产品范例，共5个必需资源；发布暂存资源与源文件 SHA256 一致。
3. 真实 release exe 使用默认沙箱启动，未加入 no-sandbox，6/6冒烟检查通过：内嵌产品UI、真实IPC及主分支构建标识（revision=`dade8fcbcc75`、dirty=false）、真实 Tea PDF 本地导入到 ready_for_review、H段映射、零云调用、学生预览画布。
4. release exe SHA256：`8dad3267d704dcbd40fb2ea4a04a767443f8ae219fde90f4291182f73d0b0349`。

**证据层级**：release 冒烟为真实产品路径；安装包资源清单/哈希和 package audit 为打包证据。没有执行安装器或覆盖用户已安装应用，也没有在本轮增加独立学生端作答/提交/计分验证。此前完整在线/离线默认沙箱验收仍以 [cloud-direct QA报告](cloud-direct-2026-10-14.md) 为准。

精简记录：`artifacts/qa/cloud-direct-release-manifest.json`、`artifacts/release-smoke/report.json`、`artifacts/qa/cloud-direct-release-build.log`、`artifacts/qa/cloud-direct-release-package-audit.log`、`artifacts/qa/cloud-direct-release-signatures.json`。

## 清理

- 用 Cargo 原生清理移除 debug 构建文件（Cargo报告20.1GiB）；交付包复制并验证后，用仓库清理脚本移除剩余 target 构建目录（脚本报告3.65GB）及dist。
- 删除旧安装包、约900MB临时PDF/验证/前端备份、根目录旧日志和经查询为空的测试数据库。
- 移除旧验证目录中的复制PDF、WebView缓存、作业数据库等大体积过程产物；保留63份精简QA报告、契约报告、构建清单及必要日志，约2.64MB，原始相对报告路径保持可读，并在交付目录备份。
- 移除历史原型 harness 与模型原始结果；保留验证摘要、设计、源码、fixtures、开发依赖和PDFium。
- 用户未跟踪的审查文档以及并行新增的设计文档保留。

本地不再保留debug/release裸exe或Rust构建缓存。正式安装包保存在 `customer-delivery/`，不依赖被清理的 target 目录。
