# 验证记录（2026-11-10）

## 真实产品路径

- macOS arm64 Electron：登记→阅读作答/高亮/笔记/跨Part恢复→写作→最终交卷→下一位→快照导出，通过。
- 独立听力真实界面：作者格式 ListeningExamSourceV1、音频、9/9判分→阅读→写作→最终ready V2快照，通过。只有一套V2字段。
- 两份真实Electron合成快照保存于 fixtures/analytics/student-electron-{v2,listening-v2}，各自记录学生提交与源文件hash；没有伪造快照字段或修改数据库。
- 作者端真实Tauri/WebView2跨仓库脚本：macOS无法运行Windows exe，记录CANNOT RUN。不能以Rust或浏览器fallback冒充原生链路。

## 服务和命令层

- 全量Rust：1596通过、19配置忽略；其后新增两项真实Electron快照消费回归。最终analytics定向25/25通过，包括双窗口导入、重复导入、原始事实篡改拒绝、教师CAS/审计/撤销、写作发布、重判令牌、V1/V2快照。
- 原始阅读JSON及2处高亮、笔记、正文、听力9条答案和事件可由作者导入；重复导入不增长记录。
- 学生22组exam测试通过；renderer 55项通过。静态18/20，失败项为两项Windows PowerShell测试（本机无pwsh）。

## 前端及门禁

- 作者663项前端测试、TypeScript检查、生产构建通过；84项横向溢出矩阵通过。
- 新操作人字段在现有设置页读取/保存后端review_settings；失败不显示保存成功。未增加完整判卷页面。
- 浏览器fallback题库/工作区冒烟在旧导入脚本的“import drawer closes”等待处失败。该导入页面/脚本未被V2修改；不记为通过。
- 原有远端PR#5已失败：surface基线停留于9月、Rust已有格式差异、Windows全量并发编辑测试出现5次database is locked、Windows真实Tauri冒烟失败。未自动重写golden或放宽断言，也未绕过门禁强行合入。
- 与独立UI分支codex/library-navigation的merge-tree无冲突。

## 范围

不构建双V2转换器，不保留B的替代表/事件方言，不新增身份合并、账号或权限系统。B有用改动已择取，其余属于明确舍弃的替代实现，不能把B分支描述成已全量合并。原分支暂保留，避免在合入完成前丢失追溯。
