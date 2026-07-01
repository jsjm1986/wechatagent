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
    let coll = app.state.db.raw().collection::<Document>("operation_domain_configs");
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
    let coll = app.state.db.raw().collection::<Document>("operation_state_policies");
    // 先 seed v1 底座:TestApp::start 后 operation_state_policies 为空——m013(migrations::run,
    // 早于 ensure_prompt_pack_v2)遍历 operation_domain_configs 生成 policy,但 domain_configs
    // 要到 ensure_prompt_pack_v2 才 seed,故 m013 跑时读到 0 行 domain_configs → seed 出 0 行
    // state_policies。缺了这条 v1 行,下面单插 1 行 v2 时旧 3-key unique 建在单行上不会 E11000,
    // 测试即便在旧 bug 存在时也会 pass(空转)。补上同 (ws, domain, state_key) 的 v1 行后,
    // 与 v2 行共享 (default, user_operations, new_contact) → 旧 3-key unique 重建时撞重复键。
    coll.insert_one(
        doc! {
            "workspace_id": "default",
            "domain": "user_operations",
            "state_key": "new_contact",
            "version": 1_i32,
            "current_version": true,
        },
        None,
    )
    .await
    .expect("seed v1 state policy 底座");
    coll.insert_one(
        doc! {
            "workspace_id": "default",
            "domain": "user_operations",
            "state_key": "new_contact",
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
    let coll = app.state.db.raw().collection::<Document>("operation_domain_configs");
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
