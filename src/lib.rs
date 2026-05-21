//! wechatagent 库入口。
//!
//! 该库 crate 同时支撑 `src/main.rs` 二进制程序与 `tests/` 下的集成测试：
//! 二进制只持有最小启动逻辑,所有业务模块都通过 `pub mod` 在此暴露,便于
//! 集成测试通过 `wechatagent::xxx` 直接复用。

use mongodb::bson::DateTime;
use tokio::sync::OnceCell;

pub mod agent;
pub mod config;
pub mod db;
pub mod error;
pub mod evolution;
pub mod llm;
pub mod mcp;
pub mod models;
pub mod planner;
pub mod prompts;
pub mod routes;
pub mod tasks;
pub mod webhooks;

/// 进程启动时间。在 `main` 调用 `Database::connect` 之前由
/// `APP_STARTED_AT.set(DateTime::now())` 填充；测试也会 best-effort 填充
/// （重复 `set` 时直接忽略错误）。
///
/// HP-1（Worker stale running 回收）依赖该时间点：当任务 `claimed_at` 缺失但
/// `updated_at < APP_STARTED_AT` 时，说明该任务是当前进程启动前留下的，
/// 可以安全回收；反之则可能正在被本进程的另一个 tick 处理，应跳过一次。
pub static APP_STARTED_AT: OnceCell<DateTime> = OnceCell::const_new();
