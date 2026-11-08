# 学生端真实产物

- v1：学生原版 f685552 的真实 server/src 通过 git archive 后编译，实际服务生成的 SubmissionSnapshotV1。
- v2：学生端 4ccf390 的 Fastify 保存路由和实际签到、锁卷、听力、写作、交卷服务生成的 V2。
- v2-electron：真实 macOS Electron UI 听力→阅读→写作→最终交卷后，复制 .ready 对应的 sqlite/meta 及实际发布包原字节；有 9 条听力答案、14 条事件、2 篇作文。

不是 Rust 手写学生表。meta 中含原始字节 SHA256；导入器独立校验。evidence.json 保存生产路径与来源。无真实学生数据。服务/产品证据分开，不声称作者端 Tauri UI 已通过。

重新生成：学生端 npm run build:server，然后 node developer/tests/tools/backend-data-v2-fixture.cjs <临时目录>。Electron 产物执行 python3 developer/tests/e2e/listening_v1_student_flow.py，报告目录会保留快照与题包供导入复证。
