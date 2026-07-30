//! SR-181 operator-memory lifecycle redline.
//!
//! The test drives the production record/load/revoke helpers. It does not copy
//! their Mongo filters, so removing `revoked_at` from the live loader makes the
//! add -> revoke -> no-injection assertion fail.

mod common;

use mongodb::bson::oid::ObjectId;
use wechatagent::agent::{
    load_operator_memory_read_only, record_operator_memory, revoke_operator_memory,
};

#[tokio::test]
#[ignore = "requires Docker or TEST_MONGODB_URI"]
async fn operator_memory_add_revoke_scope_and_readd_lifecycle() {
    let app = common::TestApp::start().await;
    let db = &app.state.db;
    let ws = app.state.config.default_workspace_id.as_str();
    let account = app.state.config.default_account_id.as_str();
    let operator = "operator-a";

    let added = record_operator_memory(db, ws, account, operator, "preference", "回复保持简洁")
        .await
        .expect("record active memory");
    let original_id = added.id.expect("recorded memory id");
    let active = load_operator_memory_read_only(db, ws, account, operator, 5)
        .await
        .expect("load active memories");
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, Some(original_id));

    for (wrong_account, wrong_operator) in
        [("other-account", operator), (account, "other-operator")]
    {
        let error = revoke_operator_memory(
            db,
            ws,
            wrong_account,
            wrong_operator,
            original_id,
            "admin-a",
            "wrong scope probe",
        )
        .await
        .expect_err("cross-scope revoke must be indistinguishable from not found");
        assert!(matches!(error, wechatagent::error::AppError::NotFound(_)));
    }
    let still_active = load_operator_memory_read_only(db, ws, account, operator, 5)
        .await
        .expect("wrong-scope attempts must be zero-write");
    assert_eq!(still_active.len(), 1);

    let first = revoke_operator_memory(
        db,
        ws,
        account,
        operator,
        original_id,
        "admin-a",
        "preference was incorrect",
    )
    .await
    .expect("first revoke");
    assert!(!first.already_revoked);
    assert_eq!(first.memory.revoked_by.as_deref(), Some("admin-a"));
    assert_eq!(
        first.memory.revocation_reason.as_deref(),
        Some("preference was incorrect")
    );
    assert!(
        load_operator_memory_read_only(db, ws, account, operator, 5)
            .await
            .expect("load after revoke")
            .is_empty(),
        "revoked memory must no longer be injected"
    );

    let repeated = revoke_operator_memory(
        db,
        ws,
        account,
        operator,
        original_id,
        "admin-b",
        "attempt to overwrite audit",
    )
    .await
    .expect("repeat revoke is idempotent");
    assert!(repeated.already_revoked);
    assert_eq!(repeated.memory.revoked_by.as_deref(), Some("admin-a"));
    assert_eq!(
        repeated.memory.revocation_reason.as_deref(),
        Some("preference was incorrect")
    );

    let readded = record_operator_memory(db, ws, account, operator, "preference", "回复保持简洁")
        .await
        .expect("explicit re-add creates a new active row");
    assert_ne!(readded.id, Some(original_id));
    let active_again = load_operator_memory_read_only(db, ws, account, operator, 5)
        .await
        .expect("load re-added memory");
    assert_eq!(active_again.len(), 1);
    assert_eq!(active_again[0].id, readded.id);

    let audit_rows = db
        .knowledge_operator_memory()
        .count_documents(
            mongodb::bson::doc! {
                "workspace_id": ws,
                "account_id": account,
                "operator_id": operator,
                "content": "回复保持简洁",
            },
            None,
        )
        .await
        .expect("count audit rows");
    assert_eq!(
        audit_rows, 2,
        "revoked audit row and new active row both remain"
    );

    // Ensure an unrelated id still has the same non-disclosing NotFound behavior.
    let error = revoke_operator_memory(
        db,
        ws,
        account,
        operator,
        ObjectId::new(),
        "admin-a",
        "unknown id",
    )
    .await
    .expect_err("unknown id must be not found");
    assert!(matches!(error, wechatagent::error::AppError::NotFound(_)));

    app.cleanup().await;
}
