//! m039 精确租户身份回填集成测试。
//!
//! 启动链路已执行迁移；测试随后写入模拟 legacy 行并直接重跑 m039，验证：
//! 1. revision 从父 chunk 精确得到 workspace；
//! 2. behavior signal 仅在 `(workspace, wxid)` 唯一归属一个 account 时回填；
//! 3. 二次运行幂等；同 wxid 多账号歧义时 fail-closed 且不写猜测值。

mod common;

use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, Document},
    options::IndexOptions,
    IndexModel,
};

#[tokio::test]
#[ignore = "需要 MongoDB / testcontainers"]
async fn m039_backfills_exact_identity_and_rejects_ambiguous_signal() {
    let app = common::TestApp::start().await;
    let db = &app.state.db;

    // Recreate the pre-m039 index topology. The migration must establish all
    // scoped replacements before dropping these legacy keys, and a second run
    // must remain idempotent.
    let revisions = db.raw().collection::<Document>("chunk_revisions");
    for name in [
        "chunk_revisions_ws_chunk_rev_idx",
        "chunk_revisions_ws_created_at_idx",
    ] {
        revisions
            .drop_index(name, None)
            .await
            .expect("drop final revision index");
    }
    revisions
        .create_index(
            IndexModel::builder()
                .keys(doc! { "chunk_id": 1, "revision_id": -1 })
                .options(
                    IndexOptions::builder()
                        .name("chunk_revisions_chunk_rev_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await
        .expect("create legacy revision index");
    revisions
        .create_index(
            IndexModel::builder()
                .keys(doc! { "created_at": -1 })
                .options(
                    IndexOptions::builder()
                        .name("chunk_revisions_created_at_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await
        .expect("create legacy revision time index");

    let signals = db.raw().collection::<Document>("behavior_signals");
    signals
        .drop_index("uniq_behavior_signals_ws_account_dedupe_key", None)
        .await
        .expect("drop final signal unique index");
    signals
        .drop_index(
            "workspace_id_1_account_id_1_contact_wxid_1_observed_at_-1",
            None,
        )
        .await
        .expect("drop final signal timeline index");
    signals
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "dedupe_key": 1 })
                .options(
                    IndexOptions::builder()
                        .name("uniq_behavior_signals_workspace_dedupe_key".to_string())
                        .unique(true)
                        .partial_filter_expression(doc! {
                            "dedupe_key": { "$type": "string" }
                        })
                        .build(),
                )
                .build(),
            None,
        )
        .await
        .expect("create legacy signal unique index");
    signals
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "contact_wxid": 1, "observed_at": -1 })
                .build(),
            None,
        )
        .await
        .expect("create legacy signal timeline index");

    let chunk_id = ObjectId::new();
    db.raw()
        .collection::<Document>("operation_knowledge_chunks")
        .insert_one(
            doc! {
                "_id": chunk_id,
                "workspace_id": "ws-a",
            },
            None,
        )
        .await
        .expect("insert parent chunk");
    let revision_id = ObjectId::new();
    db.raw()
        .collection::<Document>("chunk_revisions")
        .insert_one(
            doc! {
                "_id": revision_id,
                "chunk_id": chunk_id.to_hex(),
                "revision_id": "legacy-rev",
                "created_at": mongodb::bson::DateTime::now(),
            },
            None,
        )
        .await
        .expect("insert legacy revision");

    db.raw()
        .collection::<Document>("contacts")
        .insert_one(
            doc! {
                "workspace_id": "ws-a",
                "account_id": "account-a",
                "wxid": "wxid-unique",
            },
            None,
        )
        .await
        .expect("insert unique contact");
    let signal_id = ObjectId::new();
    db.raw()
        .collection::<Document>("behavior_signals")
        .insert_one(
            doc! {
                "_id": signal_id,
                "workspace_id": "ws-a",
                "contact_wxid": "wxid-unique",
                "dedupe_key": "reply_length:wxid-unique:m1",
            },
            None,
        )
        .await
        .expect("insert legacy signal");

    wechatagent::db::migrations::m039_scope_revision_and_behavior_identity::run_step(db)
        .await
        .expect("first m039 run");
    // 可重启：显式身份已存在时二次运行不得依赖默认 scope 或重复改写。
    wechatagent::db::migrations::m039_scope_revision_and_behavior_identity::run_step(db)
        .await
        .expect("second m039 run");

    let mut revision_indexes = revisions
        .list_indexes(None)
        .await
        .expect("list revision indexes");
    let mut revision_names = Vec::new();
    let mut revision_keys = Vec::new();
    while let Some(index) = revision_indexes
        .try_next()
        .await
        .expect("read revision index")
    {
        revision_keys.push(index.keys);
        if let Some(name) = index.options.and_then(|options| options.name) {
            revision_names.push(name);
        }
    }
    assert!(revision_names.contains(&"chunk_revisions_ws_chunk_rev_idx".to_string()));
    assert!(revision_names.contains(&"chunk_revisions_ws_created_at_idx".to_string()));
    assert!(!revision_names.contains(&"chunk_revisions_chunk_rev_idx".to_string()));
    assert!(!revision_names.contains(&"chunk_revisions_created_at_idx".to_string()));
    assert!(!revision_keys.contains(&doc! { "chunk_id": 1, "revision_id": -1 }));
    assert!(!revision_keys.contains(&doc! { "created_at": -1 }));

    let mut signal_indexes = signals
        .list_indexes(None)
        .await
        .expect("list signal indexes");
    let mut signal_names = Vec::new();
    let mut signal_keys = Vec::new();
    while let Some(index) = signal_indexes.try_next().await.expect("read signal index") {
        signal_keys.push(index.keys);
        if let Some(name) = index.options.and_then(|options| options.name) {
            signal_names.push(name);
        }
    }
    assert!(signal_names.contains(&"uniq_behavior_signals_ws_account_dedupe_key".to_string()));
    assert!(signal_keys.contains(&doc! { "workspace_id": 1, "account_id": 1, "dedupe_key": 1 }));
    assert!(signal_keys.contains(&doc! {
        "workspace_id": 1,
        "account_id": 1,
        "contact_wxid": 1,
        "observed_at": -1,
    }));
    assert!(!signal_keys.contains(&doc! { "workspace_id": 1, "dedupe_key": 1 }));
    assert!(
        !signal_keys.contains(&doc! { "workspace_id": 1, "contact_wxid": 1, "observed_at": -1 })
    );

    let revision = db
        .raw()
        .collection::<Document>("chunk_revisions")
        .find_one(doc! { "_id": revision_id }, None)
        .await
        .expect("read revision")
        .expect("revision exists");
    assert_eq!(revision.get_str("workspace_id").unwrap(), "ws-a");
    let signal = db
        .raw()
        .collection::<Document>("behavior_signals")
        .find_one(doc! { "_id": signal_id }, None)
        .await
        .expect("read signal")
        .expect("signal exists");
    assert_eq!(signal.get_str("account_id").unwrap(), "account-a");

    // Explicit account identity is historical. A current Contact may have
    // been removed by an earlier cleanup migration, so reruns must preserve
    // the materialized account rather than requiring a live parent row.
    let historical_id = ObjectId::new();
    signals
        .insert_one(
            doc! {
                "_id": historical_id,
                "workspace_id": "ws-a",
                "account_id": "account-b",
                "contact_wxid": "wxid-unique",
                "dedupe_key": "reply_length:wxid-unique:historical",
            },
            None,
        )
        .await
        .expect("insert explicit historical signal");
    wechatagent::db::migrations::m039_scope_revision_and_behavior_identity::run_step(db)
        .await
        .expect("explicit historical identity remains valid without current contact");
    let historical = signals
        .find_one(doc! { "_id": historical_id }, None)
        .await
        .expect("read historical signal")
        .expect("historical signal exists");
    assert_eq!(historical.get_str("account_id").unwrap(), "account-b");

    // A materialized workspace does not make an orphan revision valid. The
    // parent Chunk remains the authoritative tenant owner on every rerun.
    let orphan_revision_id = ObjectId::new();
    revisions
        .insert_one(
            doc! {
                "_id": orphan_revision_id,
                "workspace_id": "ws-a",
                "chunk_id": ObjectId::new().to_hex(),
                "revision_id": "orphan-rev",
                "created_at": mongodb::bson::DateTime::now(),
            },
            None,
        )
        .await
        .expect("insert explicit orphan revision");
    let orphan_error =
        wechatagent::db::migrations::m039_scope_revision_and_behavior_identity::run_step(db)
            .await
            .expect_err("explicit orphan revision must fail closed");
    assert!(orphan_error.to_string().contains("without parent chunk"));
    revisions
        .delete_one(doc! { "_id": orphan_revision_id }, None)
        .await
        .expect("remove orphan revision fixture");

    for account_id in ["account-a", "account-b"] {
        db.raw()
            .collection::<Document>("contacts")
            .insert_one(
                doc! {
                    "workspace_id": "ws-a",
                    "account_id": account_id,
                    "wxid": "wxid-ambiguous",
                },
                None,
            )
            .await
            .expect("insert ambiguous contact");
    }
    // This row is valid and would be rewritten if m039 still mutated while
    // scanning. The later ambiguous signal must abort validation before this
    // or any other planned backfill is applied.
    let uncommitted_revision_id = ObjectId::new();
    revisions
        .insert_one(
            doc! {
                "_id": uncommitted_revision_id,
                "chunk_id": chunk_id.to_hex(),
                "revision_id": "must-remain-unmodified",
                "created_at": mongodb::bson::DateTime::now(),
            },
            None,
        )
        .await
        .expect("insert valid revision before invalid signal");
    let ambiguous_id = ObjectId::new();
    db.raw()
        .collection::<Document>("behavior_signals")
        .insert_one(
            doc! {
                "_id": ambiguous_id,
                "workspace_id": "ws-a",
                "contact_wxid": "wxid-ambiguous",
                "dedupe_key": "reply_length:wxid-ambiguous:m1",
            },
            None,
        )
        .await
        .expect("insert ambiguous legacy signal");

    let error =
        wechatagent::db::migrations::m039_scope_revision_and_behavior_identity::run_step(db)
            .await
            .expect_err("ambiguous account ownership must fail closed");
    assert!(error.to_string().contains("2 matching accounts"));
    let ambiguous = db
        .raw()
        .collection::<Document>("behavior_signals")
        .find_one(doc! { "_id": ambiguous_id }, None)
        .await
        .expect("read ambiguous signal")
        .expect("ambiguous signal exists");
    assert!(!ambiguous.contains_key("account_id"));
    let uncommitted_revision = revisions
        .find_one(doc! { "_id": uncommitted_revision_id }, None)
        .await
        .expect("read uncommitted revision")
        .expect("uncommitted revision exists");
    assert!(
        !uncommitted_revision.contains_key("workspace_id"),
        "cross-collection validation failure must happen before the first backfill write"
    );

    app.cleanup().await;
}

#[tokio::test]
#[ignore = "需要 MongoDB / testcontainers"]
async fn startup_reuses_equivalent_scoped_revision_indexes_with_historical_names() {
    let app = common::TestApp::start().await;
    let db = &app.state.db;
    let revisions = db.raw().collection::<Document>("chunk_revisions");

    for name in [
        "chunk_revisions_ws_chunk_rev_idx",
        "chunk_revisions_ws_created_at_idx",
    ] {
        revisions
            .drop_index(name, None)
            .await
            .expect("drop canonical revision index");
    }

    // Production can retain immutable revision history after its parent
    // knowledge chunks have been retired. m039 is already recorded as applied,
    // so startup index reconciliation must not rerun the historical backfill.
    revisions
        .insert_one(
            doc! {
                "_id": ObjectId::new(),
                "workspace_id": "default",
                "chunk_id": ObjectId::new().to_hex(),
                "revision_id": "historical-orphan",
                "created_at": mongodb::bson::DateTime::now(),
            },
            None,
        )
        .await
        .expect("insert historical orphan revision");
    revisions
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "chunk_id": 1, "revision_id": -1 })
                .options(
                    IndexOptions::builder()
                        .name("chunk_revisions_chunk_rev_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await
        .expect("create historical-name scoped identity index");
    revisions
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "created_at": -1 })
                .options(
                    IndexOptions::builder()
                        .name("chunk_revisions_created_at_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await
        .expect("create historical-name scoped timeline index");

    db.ensure_indexes()
        .await
        .expect("equivalent historical-name indexes satisfy startup");

    let mut indexes = revisions.list_indexes(None).await.expect("list indexes");
    let mut names = Vec::new();
    while let Some(index) = indexes.try_next().await.expect("read index") {
        if let Some(name) = index.options.and_then(|options| options.name) {
            names.push(name);
        }
    }
    assert!(names.contains(&"chunk_revisions_chunk_rev_idx".to_string()));
    assert!(names.contains(&"chunk_revisions_created_at_idx".to_string()));
    assert!(!names.contains(&"chunk_revisions_ws_chunk_rev_idx".to_string()));
    assert!(!names.contains(&"chunk_revisions_ws_created_at_idx".to_string()));

    app.cleanup().await;
}

#[tokio::test]
#[ignore = "需要 MongoDB / testcontainers"]
async fn startup_rejects_same_revision_keys_with_incompatible_options() {
    let app = common::TestApp::start().await;
    let db = &app.state.db;
    let revisions = db.raw().collection::<Document>("chunk_revisions");

    revisions
        .drop_index("chunk_revisions_ws_chunk_rev_idx", None)
        .await
        .expect("drop canonical identity index");
    revisions
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "chunk_id": 1, "revision_id": -1 })
                .options(
                    IndexOptions::builder()
                        .name("chunk_revisions_chunk_rev_idx".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await
        .expect("create incompatible same-key index");

    let error = db
        .ensure_indexes()
        .await
        .expect_err("same keys with incompatible options must fail closed");
    let message = error.to_string();
    assert!(message.contains("incompatible options"), "{message}");
    assert!(
        message.contains("chunk_revisions_chunk_rev_idx"),
        "{message}"
    );

    app.cleanup().await;
}
