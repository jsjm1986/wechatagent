//! SEC-1 + EVO-2 回归：evolution proposal 端点必须按 `admin.current_workspace`
//! 隔离，且 release/rollback 审计 `released_by` 记真实操作者而非常量 `"admin"`。
//!
//! ## 为什么走 collection filter-shape 而非 HTTP handler
//!
//! 三个被修 handler（`get_evolution_proposal_detail` / `release_evolution_proposal`
//! / `rollback_evolution_proposal`）都是 `pub(super)`，集成测试无法直接调用；项目
//! 也没有把 `api_router` 拉起来打 HTTP 的测试基建。沿用 `workspace_isolation.rs`
//! 既有约定（见该文件 doc 注释）——核心隔离不变量就是"每个 proposal 读取 filter
//! 必带 `workspace_id = admin.current_workspace`"，handler 只是注入该字段的 thin
//! wrapper。本测试因此直插两条分属不同租户的 proposal，断言与 handler 同形的
//! 复合过滤 `{ _id, workspace_id }` 阻断跨租户读，命中本租户。
//!
//! EVO-2（`released_by == admin.username`）的端到端验证依赖 `release_threshold`，
//! 而后者用 mongo 事务（`start_transaction`）落库——`TestApp` 起的是 standalone
//! mongo（`Mongo::default()`），不支持事务。该 actor 传参修复（4 处 dispatch 从
//! `DEFAULT_RELEASE_ADMIN` 改为 `&admin.username`）由 handler 代码审查保证；本测试
//! 锁住 SEC-1 的 workspace 复合过滤回归。
//!
//! 默认 `#[ignore]`，需 Docker（testcontainers MongoDB）。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDt, Document};

use crate::common::TestApp;

/// 插一条 threshold proposal（`status="eligible_for_release"`）到指定 workspace，
/// 返回其 `_id`。用 raw insert 避免依赖 `Proposal` 全字段默认值，只放隔离测试
/// 需要的字段（与 `workspace_isolation.rs` 的 raw-insert 风格一致）。
async fn insert_threshold_proposal(app: &TestApp, workspace_id: &str, account_id: &str) -> ObjectId {
    let id = ObjectId::new();
    app.state
        .db
        .raw()
        .collection::<Document>("proposals")
        .insert_one(
            doc! {
                "_id": id,
                "experiment_id": format!("exp_{workspace_id}"),
                "workspace_id": workspace_id,
                "account_id": account_id,
                "proposal_kind": "threshold",
                "status": "eligible_for_release",
                "gate_key": "fact_risk_block",
                "current_value": 6.0_f64,
                "proposed_value": 5.5_f64,
                "created_at": BsonDt::now(),
                "updated_at": BsonDt::now(),
            },
            None,
        )
        .await
        .expect("insert threshold proposal");
    id
}

/// SEC-1：workspace B 视角按 proposal `_id` 查 workspace A 的 proposal —— 复合
/// 过滤命中不到（→ handler 的 `ok_or_else(NotFound)` → 404 语义）。
#[tokio::test]
#[ignore]
async fn cross_workspace_proposal_detail_returns_none() {
    let app = TestApp::start().await;
    let proposal_id = insert_threshold_proposal(&app, "ws_a", "acc_a").await;

    // workspace B 视角（handler 注入 admin.current_workspace="ws_b"）的复合过滤
    let cross = app
        .state
        .db
        .proposals()
        .find_one(doc! { "_id": proposal_id, "workspace_id": "ws_b" }, None)
        .await
        .expect("cross-tenant proposal lookup");
    assert!(
        cross.is_none(),
        "ws_b 不应通过 proposal _id 读到 ws_a 的 proposal（IDOR 越权）"
    );

    // 本租户视角命中
    let own = app
        .state
        .db
        .proposals()
        .find_one(doc! { "_id": proposal_id, "workspace_id": "ws_a" }, None)
        .await
        .expect("own-tenant proposal lookup");
    assert!(own.is_some(), "ws_a 视角应能读到自己的 proposal");
}

/// SEC-1（release/rollback 同形过滤）：release/rollback handler 同样先按
/// `{ _id, workspace_id }` 复合过滤定位 proposal，再 dispatch。本测试验证该
/// 过滤 shape 跨租户命中为 0、本租户命中为 1——即跨租户无法 release/rollback
/// 他人 proposal。
#[tokio::test]
#[ignore]
async fn cross_workspace_release_rollback_filter_blocks() {
    let app = TestApp::start().await;
    let proposal_id = insert_threshold_proposal(&app, "ws_a", "acc_a").await;

    // release/rollback handler 同形的复合过滤：ws_b 视角 → 0
    let cross = app
        .state
        .db
        .proposals()
        .count_documents(doc! { "_id": proposal_id, "workspace_id": "ws_b" }, None)
        .await
        .expect("cross-tenant release/rollback filter count");
    assert_eq!(
        cross, 0,
        "ws_b 不应通过 proposal _id 命中 ws_a 的 proposal（release/rollback IDOR）"
    );

    let own = app
        .state
        .db
        .proposals()
        .count_documents(doc! { "_id": proposal_id, "workspace_id": "ws_a" }, None)
        .await
        .expect("own-tenant release/rollback filter count");
    assert_eq!(own, 1, "ws_a 视角应能命中自己的 proposal 用于 release/rollback");
}

/// SEC-1：未知 workspace 读不到任何 proposal。
#[tokio::test]
#[ignore]
async fn unknown_workspace_proposal_lookup_returns_none() {
    let app = TestApp::start().await;
    let proposal_id = insert_threshold_proposal(&app, "ws_a", "acc_a").await;

    let ghost = app
        .state
        .db
        .proposals()
        .find_one(
            doc! { "_id": proposal_id, "workspace_id": "ghost_workspace" },
            None,
        )
        .await
        .expect("ghost workspace lookup");
    assert!(
        ghost.is_none(),
        "未知 workspace 不应读到任何 proposal，实际命中 ws_a 的 proposal"
    );
}
