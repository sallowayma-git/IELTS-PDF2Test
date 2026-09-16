//! 识别闭环：本地链 / 云端链 / 原文件核验三路结果统一对齐与裁决。
//!
//! 设计约束（对应任务书第二～四步）：
//! - **统一语义**：所有内容先归一到 `IeltsAuthoringIRV2` 语义视图再比较，JS 只是发布派生产物。
//! - **一份建议**：同一 `(target, field)` 只能有 `DecisionItemV1`，不是「每路一份权威稿」。
//! - **禁止猜**：证据不足时输出 `Unverifiable`，不得强行选版本。
//! - **自动应用靠规则**：不依赖模型置信度，只依赖确定性条件（未被用户修改 + 原文断言 + 低风险字段 + 合并后结构合法）。
//! - **有界**：模型调用有上限，不做无限递归校验。
//!
//! 模块划分：
//! - [`candidate`]：权威稿 / 本地识别 / 云端输出 → 统一候选结构（含容错 salvage）。
//! - [`source`]：原文件核验，产出「问题 + 修正建议」，不产出第二份权威稿。
//! - [`rules`]：确定性规则比较，逐字段产出候选裁决项。
//! - [`adjudicate`]：合并去重、依赖分组、自动应用资格、结构校验。
//! - [`store`]：数据库读取权威 + job 目录证据留痕 + 幂等日志。
//! - [`commands`]：命令层（完整识别周期、安全自动应用、人工决策）。
//! - [`engine`]：把上述环节连成端到端流水线（供调度与命令层调用）。

pub(crate) mod adjudicate;
pub(crate) mod candidate;
pub(crate) mod commands;
pub(crate) mod engine;
pub(crate) mod rules;
pub(crate) mod source;
pub(crate) mod store;
