# 现状证据

- 作者端 main=a7fa8ca，有 7 个用户未提交文件，未携带入 worktree。
- 学生端 main=848c76a，工作区干净。
- 两仓库 AGENTS.md 已读；学生端的重复确认规则已有用户完整实施授权，直接执行。
- 作者端无已有 .zread/wiki/current，使用实际命令/服务代码核实，不生成额外 wiki。
- 当前快照契约 SubmissionSnapshotV1 / dbSchemaVersion=1，整库快照 + ready 提交点。
- 当前试卷契约 V1.1：自包含 entry，发布指针唯一提交点，批次不可变、锁卷判分。

- 听力原实现确实只返回分数；已事务落表，原始行幂等冻结。另发现音频读取漏传 resourcesBase，已修复批次资源解析。
- 作者端听力包最低运行时为 1.0.0，因此统一 EXAM_RUNTIME_VERSION=1.0.0，保留阅读 alias，兼容旧阅读0.2.0和听力1.0.0，安装包版本不参与。
- 学生现行 reading_passages.passage_index 是 1-based，旧快照保留原序号，契约已纠正。
- Reading 保存路由逐字段投影会丢 events，已显式接入并用真实 Fastify handler 生成回归快照。
- 已修复阅读保存把 questionTimelineLite 固定为空、切篇重置已有逐题时间的问题：保存/恢复原时间线。
- 真实 macOS Electron 主链通过；原 Windows 托管模式断言改为平台能力断言，Windows 检查不被标成已验证。
- 原“更新 exportedAt 能覆盖已交卷事实”的旧测试与用户不可变原则冲突：先更新 V2 契约，再改为冲突拒绝及异常记录；不恢复旧行为。
- 运行时未提供解析字段时不制造解析，题目上下文返回 explanation=null / explanationAvailable=false。
