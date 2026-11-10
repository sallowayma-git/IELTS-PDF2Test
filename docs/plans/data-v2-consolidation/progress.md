# 进度

2026-11-10：用户授权比较、收敛并合入。阅读原任务书与作者 AGENTS，使用 planning-with-files；建立从最新 origin/main 起的独立作者 worktree。正在固定学生端对应输入并逐模块审查。

按用户补充取消双V2兼容。两端已以A建立隔离整合分支（原分支不变）。作者新增单一导入事务中的原始事实校验，保留原始阅读JSON；发现旧测试允许新exportedAt覆盖已交卷判分，已按任务书改为冲突拒绝。学生端合并B的同步离散改答采集与恢复timeline修正，仍沿用A的clockId和clientEventId。未引入第二套事件字段或听力冗余分值列。依赖安装/基础编译通过，正在针对回归验证；真实Electron二进制已安装arm64。

真实 Electron 主流程首轮通过，作者消费其快照后发现 highlights 为 NULL，追到学生 ExamReadingService 构造 rawSubmission 时未保留；已修复，不削弱断言。独立听力真实 UI 暴露 per_slot 的 cardinality 错用：三格文本组 exact=3 被逐格要求三答案。采用 B 的组级计数，9/9 听力→阅读→写作→最终 ready V2 快照实际通过。E2E 发布 fixture 改用真实 current-paper 锁卷契约，未绕过锁卷要求。
作者全 Rust 1596 通过、19 忽略；前端663通过，生产构建通过。学生静态18/20通过，两项因本机无pwsh无法执行Windows脚本；真实macOS Electron不声称Windows kiosk。原有作者PR#5的远端失败已确认：旧surface基线、旧Rust格式、Windows并发编辑5次SQLite锁，以及Windows原生冒烟失败；不是V2业务验证的替代品。

用户明确要求本轮修复门禁后合入。按gh-fix-ci流程核对GitHub日志，采用已授权方案，不重复要许可。合并已确认的UI PR#5至收敛分支（无冲突），一次验证与合入。现有Rust格式单独机械提交；审查当前数据/导入/题库面后更新旧surface基线，SQL注释不再误计为IF表。浏览器冒烟按独立导入页的任务创建/队列断言替换“抽屉关闭”，13步实际通过。Windows连接在任何PRAGMA之前设置30秒busy_timeout；首次schema检查与登记同锁，避免并发重复迁移。Windows原生驱动显式传递官方webviewOptions.userDataFolder和additionalBrowserArguments，保留失败产物，等待远端实际验证。

远端surface-gates已全绿。新增真实锁等待回归（锁持续6秒，超过旧5秒阈值），同原并发压力共3项通过，不降低并发量、不重放事务。依据微软2026-09-11安全文档，高权限WebView2忽略WEBVIEW2环境覆盖；当前CI日志只有默认browser flags，与其一致。仅cfg(windows,debug_assertions)且有隔离测试目录时经Tauri配置API设置调试参数/profile，正式发行构建不包含此逻辑；Windows workflow打印权限级别，待真实远端确认。原生冒烟增加操作人设置保存/刷新回读，并按现有题库表/导入队列入口重开编辑器。
