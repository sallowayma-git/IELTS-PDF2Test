# 最终验证与边界（2026-11-10）

作者端 [PR #6](https://github.com/sallowayma-git/IELTS-PDF2Test/pull/6) 已合入 `9b4463f94d5e5e6aa2b2ac05f8e55452b8dddab1`，包含原界面PR#5；学生端 [PR #5](https://github.com/sallowayma-git/IELTS-NASfor-WenDao/pull/5) 已合入 `4b95765dfb886626293726fe77d3b1ac18636670`。只发布一种Data V2，保留V1升级。

## 实际产品路径

- macOS arm64真实Electron：登记、阅读作答/高亮/笔记、跨Part恢复、写作、最终交卷、下一位、快照导出，通过。
- 独立听力真实Electron：作者格式ListeningExamSourceV1、音频、9/9判分、阅读、写作、最终ready V2快照，通过。
- Windows真实Tauri/WebView2：操作人设置保存并刷新回读、PDF导入、本地识别、工作区修改、重开后内容保留，通过。
- 两份真实Electron合成快照保存于fixtures/analytics/student-electron-{v2,listening-v2}，记录学生提交与源文件hash。它们由实际renderer/server/exporter生成，没有事后修改快照数据库。

## 服务与命令层（UI之下）

- macOS全量Rust：1600通过、19配置忽略；最终analytics定向25/25通过。
- Windows全量Rust：1601通过、19配置忽略。原高压并发保存和新增6秒锁等待回归均通过，内部并发线程数量保持不变。
- 作者消费两份真实Electron快照：原始阅读JSON、两处高亮、笔记、写作正文、听力9条答案和事件保留；重复导入不增长记录。篡改原始事实整笔回滚，教师复核/排除/审计不被重导覆盖。
- V1迁移回滚、归档恢复、CAS改判/撤销、写作review/publish、重判令牌复核、统计与导出回归通过。
- Linux后端产品链从真实导入、识别、编辑重开至NAS包发布，通过；这是命令/文件/数据库层证据，不冒充整条原生UI链。
- 学生22组exam文件、55项renderer测试通过；静态18/20，两项Windows PowerShell检查在本机缺pwsh，未记为通过。

## 前端与CI

- 作者671项前端测试、TypeScript检查、生产构建、Rust格式及审查后的产品面基线通过。
- 浏览器fallback：84项横向溢出矩阵、13步导入/工作区冒烟通过。这一级与实际Tauri证据分别报告。
- [Linux与surface门禁](https://github.com/sallowayma-git/IELTS-PDF2Test/actions/runs/42917815859) 全绿。
- [Windows门禁](https://github.com/sallowayma-git/IELTS-PDF2Test/actions/runs/42917815816) 全绿：全量Rust、真实Tauri、NSIS安装包构建、包内容审计、签名状态审计。
- 签名审计记录为NotSigned，当前门禁未要求证书有效签名；没有把它描述成已签名发行包。
- 高权限WebView2忽略环境覆盖的问题由隔离debug构建的显式API配置修正；正式发行构建不包含该调试逻辑。DOCX辅助桩的Windows资源争用通过限制独立重型用例并行度解决，未跳过用例或放宽PDF断言。

## 尚未覆盖

Windows学生kiosk系统策略的实机部署验收，以及作者与学生同时运行、经实际SMB/NAS设备完成在途重新发布的整条原生UI链，本轮没有声称通过。当前证据覆盖上述真实应用路径和明确列出的服务/文件/数据库路径。

## 清理

本地产物已清理：Rust target、node_modules、前端dist、server/dist、测试报告/截图与Python缓存、临时对比目录及测试生成的相对ProgramData目录。正式源码、回归夹具和Git历史保留。
