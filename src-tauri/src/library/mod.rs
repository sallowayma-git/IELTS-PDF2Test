//! M1（原计划 Phase 2 / PR-04/05）：唯一权威稿数据层。
//!
//! - [`schema`]：V2 表 + `PRAGMA user_version` 版本化迁移（计划 §4.3/§4.4）。
//! - [`repository`]：Canonical DS 仓库与事务编辑（计划 §9.5）。
//! - [`migration`]：旧题迁移，幂等、不覆盖用户编辑（计划 §11.2）。
//!
//! 权威方向（计划 §4.2）：canonical_ds_json → 派生缓存/发布编译；
//! 任何文件树、cloud raw、preview 不得反向写回。

pub(crate) mod commands;
pub(crate) mod migration;
pub(crate) mod repository;
pub(crate) mod schema;
