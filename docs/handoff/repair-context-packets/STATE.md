# STATE — 校核链路上下文管理

> 唯一进度真源。每个 agent 开工先读、收工前更新并提交。只追加「日志」，阶段勾选按事实改。
> 状态取值：`todo` / `doing` / `done` / `blocked(原因)`。`done` 必须附提交 hash 与证据等级。

## 阶段

| 阶段 | 内容 | 状态 | 提交 / 证据 |
|---|---|---|---|
| P0 | 基线：环境探测、全量测试数字、核对 TASK.md §3 行号 | todo | |
| P1 | `packets.rs` 切分 + 范围 + 包内容（TASK §4.1，测试 1-4） | todo | |
| P2 | `grab.rs` 抓取工具 + `report_insufficient_context` + 升级阶梯（§4.2，测试 6-7） | todo | |
| P3 | 编排改造 + prompt + 请求体去整份 PDF（§4.3/4.4，测试 5、8、9） | todo | |
| P4 | 可观测性 + 受控模型剧本 + 真实 HTTP 集成测试 + 两模式对比（§4.5/§6/§7） | todo | |
| P5 | 独立审计 #1（子代理，只读，对抗式） | todo | |
| P6 | 修复审计 #1 发现 | todo | |
| P7 | 最终验收 + 独立审计 #2 + 修复 | todo | |
| P8 | 收口报告 `REPORT.md` | todo | |

## 基线数字（P0 填）

- 平台：
- Rust `cargo test --lib`：passed / failed / ignored =
- Vitest：
- tsc：
- 已知基线失败（非本任务引入）：

## 审计发现（P5/P7 填，逐条带状态）

## 越界需求（需要改非授权文件时写在这里，不要直接改）

## 日志（只追加：时间、agent 做了什么、提交、下一步）
