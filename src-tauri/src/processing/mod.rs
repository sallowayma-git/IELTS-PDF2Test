//! M2（原 P6-T01/T02/T04/T05）：持久化处理队列与调度。
//!
//! - [`queue`]：processing_jobs_v2 上的 durable 操作（认领/lease/阶段推进/恢复/取消/重试）。
//! - [`scheduler`]：调度循环 + 本地/云端并发 + `processing://item-updated` 事件。
//! - [`commands`]：`import_files`（后端接管批量导入）与取消/重试命令核心。
//!
//! 权威状态在 SQLite（计划 §5.2）；事件只携带可比较的状态版本，前端以 DB 为准刷新。

pub(crate) mod answer_page;
pub(crate) mod commands;
pub(crate) mod queue;
pub(crate) mod scheduler;
