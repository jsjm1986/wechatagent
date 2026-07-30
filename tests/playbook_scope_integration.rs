#![cfg(test)]

mod common;

use axum::extract::{Extension, Path, State};
use axum::Json;
use mongodb::bson::{doc, oid::ObjectId, DateTime};

use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::error::AppError;
use wechatagent::models::{OperationPlaybook, WechatAccount};
use wechatagent::routes::playbooks::{
    set_default_operation_playbook, update_operation_playbook, OperationPlaybookRequest,
    PlaybookMutationIdentity,
};

use crate::common::TestApp;

fn admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "playbook_admin".into(),
        username: "playbook_admin".into(),
        current_workspace: workspace_id.into(),
    }
}

fn account(workspace_id: &str, account_id: &str) -> WechatAccount {
    let now = DateTime::now();
    WechatAccount {
        id: None,
        workspace_id: workspace_id.into(),
        account_id: account_id.into(),
        alias: account_id.into(),
        display_name: account_id.into(),
        app_id: None,
        wxid: None,
        nick_name: None,
        avatar_url: None,
        mcp_base_url: None,
        mcp_api_key: None,
        webhook_secret: None,
        online: true,
        status: None,
        last_sync_at: now,
        capacity: 0,
        persona_tag: None,
        off_hours: vec![],
        created_at: now,
        updated_at: now,
    }
}

fn playbook(
    workspace_id: &str,
    account_id: &str,
    name: &str,
    version: i32,
    is_default: bool,
) -> OperationPlaybook {
    let now = DateTime::now();
    OperationPlaybook {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.into(),
        account_id: account_id.into(),
        name: name.into(),
        description: None,
        method_prompt: "original method".into(),
        profile_method: None,
        tag_method: None,
        stage_method: None,
        intent_method: None,
        follow_up_method: None,
        reply_style: None,
        forbidden_rules: None,
        success_criteria: None,
        created_by: "manual".into(),
        release_status: "published".into(),
        is_default,
        version,
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
#[ignore]
async fn wrong_account_and_stale_version_are_conflicts_with_zero_writes() {
    let app = TestApp::start().await;
    let workspace = app.state.config.default_workspace_id.clone();
    for id in ["account-a", "account-b"] {
        app.state
            .db
            .accounts()
            .insert_one(account(&workspace, id), None)
            .await
            .unwrap();
    }
    let target = playbook(&workspace, "account-a", "target", 2, false);
    let current_default = playbook(&workspace, "account-a", "default", 7, true);
    let target_id = target.id.unwrap();
    app.state
        .db
        .operation_playbooks()
        .insert_one(&target, None)
        .await
        .unwrap();
    app.state
        .db
        .operation_playbooks()
        .insert_one(&current_default, None)
        .await
        .unwrap();
    let raw = app
        .state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("operation_playbooks");
    let before: Vec<_> = {
        let mut rows = raw.find(doc! {}, None).await.unwrap();
        let mut out = Vec::new();
        while rows.advance().await.unwrap() {
            out.push(rows.deserialize_current().unwrap());
        }
        out.sort_by_key(|row| row.get_object_id("_id").unwrap().to_hex());
        out
    };

    let wrong_account = update_operation_playbook(
        State(app.state.clone()),
        Extension(admin(&workspace)),
        Path(target_id.to_hex()),
        Json(OperationPlaybookRequest {
            account_id: Some("account-b".into()),
            expected_version: Some(2),
            name: "must not persist".into(),
            description: None,
            method_prompt: "must not persist".into(),
            profile_method: None,
            tag_method: None,
            stage_method: None,
            intent_method: None,
            follow_up_method: None,
            reply_style: None,
            forbidden_rules: None,
            success_criteria: None,
            is_default: Some(true),
        }),
    )
    .await;
    assert!(matches!(wrong_account, Err(AppError::Conflict(_))));

    let stale_default = set_default_operation_playbook(
        State(app.state.clone()),
        Extension(admin(&workspace)),
        Path(target_id.to_hex()),
        Json(PlaybookMutationIdentity {
            account_id: "account-a".into(),
            expected_version: 1,
        }),
    )
    .await;
    assert!(matches!(stale_default, Err(AppError::Conflict(_))));

    let mut rows = raw.find(doc! {}, None).await.unwrap();
    let mut after = Vec::new();
    while rows.advance().await.unwrap() {
        after.push(rows.deserialize_current().unwrap());
    }
    after.sort_by_key(|row| row.get_object_id("_id").unwrap().to_hex());
    app.cleanup().await;
    assert_eq!(
        after, before,
        "rejected Playbook mutations must be byte-for-byte zero-write"
    );
}

#[tokio::test]
#[ignore]
async fn setting_draft_default_publishes_target_and_demotes_old_default_atomically() {
    let app = TestApp::start_repl_set().await;
    let workspace = app.state.config.default_workspace_id.clone();
    let account_id = "playbook-release-account";
    app.state
        .db
        .accounts()
        .insert_one(account(&workspace, account_id), None)
        .await
        .unwrap();

    let current = playbook(&workspace, account_id, "current", 4, true);
    let current_id = current.id.unwrap();
    let mut draft = playbook(&workspace, account_id, "candidate", 5, false);
    draft.release_status = "draft".into();
    draft.created_by = "agent_optimized".into();
    let draft_id = draft.id.unwrap();
    app.state
        .db
        .operation_playbooks()
        .insert_one(&current, None)
        .await
        .unwrap();
    app.state
        .db
        .operation_playbooks()
        .insert_one(&draft, None)
        .await
        .unwrap();

    let response = set_default_operation_playbook(
        State(app.state.clone()),
        Extension(admin(&workspace)),
        Path(draft_id.to_hex()),
        Json(PlaybookMutationIdentity {
            account_id: account_id.into(),
            expected_version: 5,
        }),
    )
    .await
    .expect("publish draft and switch default");
    assert_eq!(response.0["version"], 5);

    let current_after = app
        .state
        .db
        .operation_playbooks()
        .find_one(doc! { "_id": current_id }, None)
        .await
        .unwrap()
        .unwrap();
    let target_after = app
        .state
        .db
        .operation_playbooks()
        .find_one(doc! { "_id": draft_id }, None)
        .await
        .unwrap()
        .unwrap();
    let default_count = app
        .state
        .db
        .operation_playbooks()
        .count_documents(
            doc! {
                "workspace_id": &workspace,
                "account_id": account_id,
                "is_default": true,
            },
            None,
        )
        .await
        .unwrap();

    app.cleanup().await;
    assert!(!current_after.is_default);
    assert_eq!(current_after.release_status, "published");
    assert!(target_after.is_default);
    assert_eq!(target_after.release_status, "published");
    assert_eq!(default_count, 1);
}
