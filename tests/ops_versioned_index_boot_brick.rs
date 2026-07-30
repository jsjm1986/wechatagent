//! H8 回归:多版本数据下 ensure_indexes 不得因残留旧 unique 索引 E11000 boot-brick。
//!
//! 背景:Phase E5-T1 前,operation_domain_configs / operation_state_policies 各有一个
//! 旧 unique(2-key / 3-key)索引由 ensure_all 用 .await? 创建;E5-T1 迁 4-tuple 多版本
//! 索引后这两处 create 是残留(建完即被 ensure_ops_versioned_indexes drop)。多版本数据下
//! 旧 unique create 撞 E11000 → ensure_indexes 返 Err → main.rs:59 ? → 启动崩溃。
//! 删除残留后,唯一性由 4-tuple unique(含 version)独家保证。
//!
//! 全部 #[ignore],需 Docker(testcontainers MongoDB)。
//! CI:`cargo test --test ops_versioned_index_boot_brick -- --ignored`。
#![cfg(test)]

mod common;

use mongodb::bson::{doc, Document};

use crate::common::TestApp;

/// 红线:operation_domain_configs 存在同 (workspace_id, domain) 多 version 行时,
/// 重跑 ensure_indexes 必须成功(旧 bug 下 2-key unique 会 E11000)。
#[tokio::test]
#[ignore]
async fn ensure_indexes_survives_multi_version_domain_configs() {
    let app = TestApp::start().await;
    // TestApp::start 已跑首次 ensure_indexes(空库单 version 底座)。
    let coll = app
        .state
        .db
        .raw()
        .collection::<Document>("operation_domain_configs");
    // 手工插同 (ws, domain) 的第 2 行 version=2,模拟 admin publish 攒下的多版本行。
    coll.insert_one(
        doc! {
            "workspace_id": "default",
            "domain": "user_operations",
            "version": 2_i32,
            "current_version": false,
        },
        None,
    )
    .await
    .expect("seed v2 domain config");

    // 模拟二次启动:重跑 ensure_indexes。旧 bug 下这里会 E11000 Err。
    let result = app.state.db.ensure_indexes().await;
    assert!(
        result.is_ok(),
        "多版本 operation_domain_configs 下 ensure_indexes 必须成功,不得 boot-brick,实际 {result:?}"
    );
}

/// 红线:operation_state_policies 存在同 (workspace_id, domain, state_key) 多 version 行时,
/// 重跑 ensure_indexes 必须成功(旧 bug 下 3-key unique 会 E11000)。
#[tokio::test]
#[ignore]
async fn ensure_indexes_survives_multi_version_state_policies() {
    let app = TestApp::start().await;
    let coll = app
        .state
        .db
        .raw()
        .collection::<Document>("operation_state_policies");
    // 使用测试专属 scope，避免依赖 TestApp 启动后默认 policy 是否已经物化。
    // 两行只差 version，仍能精确复现旧 3-key unique 的 boot-brick 条件。
    let workspace = "ops-versioned-index-test";
    let domain = "test_domain";
    let state_key = "test_state";
    coll.insert_one(
        doc! {
            "workspace_id": workspace,
            "domain": domain,
            "state_key": state_key,
            "version": 1_i32,
            "current_version": true,
        },
        None,
    )
    .await
    .expect("seed v1 state policy 底座");
    coll.insert_one(
        doc! {
            "workspace_id": workspace,
            "domain": domain,
            "state_key": state_key,
            "version": 2_i32,
            "current_version": false,
        },
        None,
    )
    .await
    .expect("seed v2 state policy");

    let result = app.state.db.ensure_indexes().await;
    assert!(
        result.is_ok(),
        "多版本 operation_state_policies 下 ensure_indexes 必须成功,不得 boot-brick,实际 {result:?}"
    );
}

/// 正向:4-tuple unique 仍挡"重复 version"——同 (ws, domain, version) 两行不合法。
/// 证明删旧 2-key unique 后唯一性没被削弱,只是维度对了(含 version)。
#[tokio::test]
#[ignore]
async fn four_tuple_unique_still_blocks_duplicate_version() {
    let app = TestApp::start().await;
    let coll = app
        .state
        .db
        .raw()
        .collection::<Document>("operation_domain_configs");
    // 先确保 4-tuple unique 已建(TestApp::start 已跑 ensure_indexes;此处再跑一次幂等)。
    app.state.db.ensure_indexes().await.expect("ensure indexes");
    // 插第一行 (ws=dup_ws, domain=dup_domain, version=1)。
    coll.insert_one(
        doc! { "workspace_id": "dup_ws", "domain": "dup_domain", "version": 1_i32, "current_version": true },
        None,
    )
    .await
    .expect("insert first version row");
    // 插完全相同 (ws, domain, version) 的第二行 → 4-tuple unique 必须拒绝。
    let dup = coll
        .insert_one(
            doc! { "workspace_id": "dup_ws", "domain": "dup_domain", "version": 1_i32, "current_version": false },
            None,
        )
        .await;
    assert!(
        dup.is_err(),
        "同 (workspace_id, domain, version) 重复行必须被 4-tuple unique 拒绝(唯一性未降级)"
    );
}
