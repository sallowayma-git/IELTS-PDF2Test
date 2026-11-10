# 数据 V2 收敛决定

用户确认两套 V2 均未上线，因此最终只发布一种 V2。保留已上线 V1 的升级和快照读取，不构建双 V2 方言、历史 V2 转换器或第二条业务路径。

## 固定输入

| | 作者端 | 学生端 |
|---|---|---|
| A：codex/backend-data-v2 | 94f85b3 | 2be1275 |
| B：codex/backend-data-v2-macos-20261109 | 699c689 | 595631d |
| main 基线 | 8cc30fa | f685552 |

## 取舍矩阵

| 范围 | 差异与决定 |
|---|---|
| 主实现 | 采用 A。按归档、导入、改判、写作、重判、统计划分模块；避免两套命令/表并存。 |
| 作者本地库 | 题库 library 库保管 papers/paper_items/item_content_snapshots；analytics 库保管导入事实与教师层。保留 A 的职责分界。 |
| 迁移 | 作者 analytics user_version=2、library=11；学生 db_schema_version=2。只从已有 V1 迁移，事务失败回滚，未来版本拒绝降级。 |
| 快照 | 只选 A 的 SubmissionSnapshotV2 形态；作者接受 (V1,1)/(V2,2)，严格校验 V2 必需表和字段，不读 NAS 活库。 |
| 事件 | 选 A 的服务端 event_seq + client_event_id 幂等 + clock_id。B 的 event_id 方言不保留。跨重启不得拼接 monotonic 时间。 |
| 听力 | 选 A 的已锁卷提交、幂等事务冻结、Part/题型归属与统计。现有协议逐 slot 一分，保留 weight/is_correct，不增加重复 max_score/score。 |
| 真实流程补漏 | 阅读提交构造器未把 highlights 放入 raw_submission，导致 UI 可恢复高亮而交卷快照丢失；显式保留。听力 response-group cardinality 应按组统计，修正 A 对 per_slot 逐格套用整组数量导致合法提交被拒；采用 B 的整组校验。 |
| 采集 | 移植 B 的实际行为修正：离散改答即时记录，文本编辑按语义合并；恢复后保留已有 timeline，不伪造重新作答。 |
| 运行时 | 两种题型继续引用同一个版本常量，将遗漏的值修正为契约已规定的 1.0.0。Electron 透传可选 sessionId/studentUuid。 |
| 归档 | 采用 A 的实际 wrapper 内容哈希与本地冻结、指针提交后登记、未证实历史标记。缺锁定内容时不借当前稿补齐。 |
| 重导 | 采用 B 的不可变事实原则，在 A 的同一导入事务内校验。exportedAt 更新只允许更新来源/补齐缺失证据；已交卷答案、判分、正文和已有事件改变即整体回滚。保存原始 reading JSON。 |
| 并发导入 | 每次导入独占 UUID 临时目录，避免一个窗口清理另一个窗口正在校验的文件；数据库写入沿用 SQLite 事务和 busy_timeout。 |
| 改判/重判 | 保留 A 的原始事实 + 有效 override，CAS 与审计同事务；exam/slot 范围必须含 runtimeSha256，预览令牌在应用时复核。 |
| 写作发布 | 保留 A 的 draft→reviewed→published，新发布后旧版 superseded；每作文最多一版 published，历史和审计保留。 |
| 身份/权限 | 保留任务书的预留表/字段；不加入 B 的身份合并工作流，不引入账号/名册/权限或额外 UI。 |
| 前端 | 本轮不实现判卷页面。仅学生采集与恢复需改 renderer；作者端只补现有设置页的操作人输入，避免成绩导出被新增必填项阻断；不增加判卷页面。既有 UI PR #5 独立，合并兼容单独验证。 |

## 核心不变量

NAS 每终端单库、DELETE 日志、进程单连接；学生本机不保存作答。作者只读带 ready 的快照副本。锁卷后不随重新发布切换题包。原始事实、教师复核、发布成绩分层保存；无 IELTS Band，统计带 N，导出带模拟成绩声明。

测试分别记录为：产品真实 Electron、服务/命令层（真实文件/数据库）、单测/静态。Windows Tauri/WebView2 与 kiosk 未在 macOS 上伪称通过。
