# Repair Decisions

- Prefer fixing existing repository/scheduler/publisher code over adding parallel abstractions.
- Saved canonical DS must be updated once per transaction. A derived cache must never determine whether a successful save is acknowledged.
- An editor request ID identifies one batch of commands across retries and windows.
- Publish must await successful saves and operate on backend-loaded frozen snapshots.
- Existing user edits and unrelated worktree changes remain intact.
- 2026-09-11: 数据库是唯一权威稿。DB 编辑不再回写 shadow 派生文件，导出/发布改为按 `editVersion` 从 canonical DS 解析；`product_chain` 的旧断言（shadow 被同步）随之改写为新契约，而不是保留旧行为、退回派生回写。
- 2026-09-11: 测试替身退出生产 bundle 必须依赖**编译期**常量（`import.meta.env.DEV`）。仅靠运行时开关（URL/localStorage）时 Rollup 无法摇树，动态 import 仍会产出独立 chunk。
- 2026-09-11: 文案分层只在两处生效——工作区加载失败与编辑失败。结构操作自身抛出的中文提示原样透传；机器码统一折成人话，原始码只在开发者模式下出现。
- 2026-09-11: M4/M5 未纳入本轮。二者是计划自身的里程碑范围（识别主链替换与云端合并），不是本工作树引入的缺陷；在未实现新主链前改不动，硬改只会制造不可验证的半成品。
