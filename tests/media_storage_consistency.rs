//! HC-006 / SR-017: local media object and Mongo metadata consistency.

#![cfg(test)]

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use mongodb::bson::{doc, oid::ObjectId, DateTime};
use reqwest::StatusCode;
use tokio::net::TcpListener;
use wechatagent::auth::session::create_session;
use wechatagent::auth::{AdminUser, SESSION_COOKIE_NAME};
use wechatagent::media_storage;
use wechatagent::models::ContentAsset;
use wechatagent::routes::api_router;

fn media_asset(id: ObjectId, workspace_id: &str, file_path: String, sha: String) -> ContentAsset {
    let now = DateTime::now();
    ContentAsset {
        id: Some(id),
        workspace_id: workspace_id.to_string(),
        account_id: None,
        kind: "media".to_string(),
        title: format!("asset-{id}"),
        body: None,
        tags: vec![],
        url: None,
        media_id: Some("cached-media-id".to_string()),
        usage_scene: None,
        media_type: Some("file".to_string()),
        file_path: Some(file_path),
        file_name: Some("asset.pdf".to_string()),
        file_size: Some(7),
        mime_type: Some("application/pdf".to_string()),
        file_sha256: Some(sha),
        sendable: Some(true),
        send_trigger_hint: None,
        target_stages: None,
        expression_pref: None,
        requires_principal_approval: Some(false),
        review_status: Some("approved".to_string()),
        review_note: None,
        min_inject_tier: None,
        created_at: now,
        updated_at: now,
    }
}

fn unique_root(prefix: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("{prefix}_{}", ObjectId::new().to_hex()))
}

fn upload_multipart(boundary: &str, file_bytes: &[u8]) -> Vec<u8> {
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nforce-db-failure\r\n\
         --{boundary}\r\nContent-Disposition: form-data; name=\"mediaType\"\r\n\r\nfile\r\n\
         --{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"failure.pdf\"\r\n\
         Content-Type: application/pdf\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(file_bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

#[tokio::test]
#[ignore = "requires mongo"]
async fn router_upload_db_failure_removes_pending_and_final_object() {
    let app = common::TestApp::start().await;
    let root = unique_root("media_upload_failure");
    let user = AdminUser {
        user_id: "media-upload-admin".to_string(),
        username: "media-upload-admin".to_string(),
        password_hash: "unused".to_string(),
        created_at: chrono::Utc::now(),
        last_login_at: None,
        workspaces: vec!["default".to_string()],
        default_workspace: Some("default".to_string()),
    };
    app.state
        .db
        .raw()
        .collection::<AdminUser>("admin_users")
        .insert_one(&user, None)
        .await
        .expect("insert media upload admin");
    let session = create_session(&app.state.db, &user, 1, "default")
        .await
        .expect("create media upload session");

    app.state
        .db
        .raw()
        .run_command(
            doc! {
                "collMod": "content_assets",
                "validator": { "title": { "$ne": "force-db-failure" } },
                "validationLevel": "strict",
                "validationAction": "error",
            },
            None,
        )
        .await
        .expect("install content asset failure validator");

    let mut state = app.state.clone();
    state.config.media_storage_dir = root.to_string_lossy().to_string();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind media upload API");
    let address = listener.local_addr().expect("media upload API address");
    let router = Router::new()
        .nest("/api", api_router(state.clone()))
        .with_state(state);
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve media upload API");
    });

    let file_bytes = b"%PDF-1.7 synthetic upload failure";
    let sha = media_storage::sha256_hex(file_bytes);
    let rel =
        media_storage::safe_relative_path("default", &sha, "pdf").expect("expected upload path");
    let boundary = "wa-media-upload-boundary";
    let response = reqwest::Client::new()
        .post(format!("http://{address}/api/content-assets/upload"))
        .header(
            reqwest::header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={}", session.session_id),
        )
        .header(
            reqwest::header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(upload_multipart(boundary, file_bytes))
        .send()
        .await
        .expect("send multipart upload");
    let status = response.status();
    let stored_rows = app
        .state
        .db
        .content_assets()
        .count_documents(doc! { "title": "force-db-failure" }, None)
        .await
        .expect("count rejected upload rows");
    let final_exists = root.join(&rel).exists();
    let pending_exists = root
        .join(media_storage::pending_relative_path(&rel).expect("pending upload path"))
        .exists();

    server.abort();
    let _ = server.await;
    let _ = tokio::fs::remove_dir_all(&root).await;
    app.cleanup().await;

    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(stored_rows, 0);
    assert!(
        !final_exists,
        "failed DB insert must not publish final media"
    );
    assert!(
        !pending_exists,
        "failed DB insert must discard pending media"
    );
}

#[tokio::test]
#[ignore = "requires mongo"]
async fn failed_db_write_settlement_cleans_or_publishes_by_live_reference() {
    let app = common::TestApp::start().await;
    let root = unique_root("media_settle");

    let orphan_bytes = b"orphan pending";
    let orphan_sha = media_storage::sha256_hex(orphan_bytes);
    let orphan_rel =
        media_storage::safe_relative_path("default", &orphan_sha, "pdf").expect("orphan path");
    {
        let _guards = media_storage::lock_paths(&root, [orphan_rel.clone()])
            .await
            .expect("lock orphan path");
        assert!(media_storage::stage_bytes(&root, &orphan_rel, orphan_bytes)
            .await
            .expect("stage orphan"));
        media_storage::settle_staged_after_db_failure(&app.state.db, &root, &orphan_rel)
            .await
            .expect("settle orphan");
    }
    assert!(!root.join(&orphan_rel).exists());
    assert!(!root
        .join(media_storage::pending_relative_path(&orphan_rel).expect("pending path"))
        .exists());

    let referenced_bytes = b"referenced pending";
    let referenced_sha = media_storage::sha256_hex(referenced_bytes);
    let referenced_rel = media_storage::safe_relative_path("default", &referenced_sha, "pdf")
        .expect("referenced path");
    {
        let _guards = media_storage::lock_paths(&root, [referenced_rel.clone()])
            .await
            .expect("lock referenced path");
        assert!(
            media_storage::stage_bytes(&root, &referenced_rel, referenced_bytes)
                .await
                .expect("stage referenced")
        );
        app.state
            .db
            .content_assets()
            .insert_one(
                media_asset(
                    ObjectId::new(),
                    "default",
                    referenced_rel.clone(),
                    referenced_sha,
                ),
                None,
            )
            .await
            .expect("insert surviving reference");
        media_storage::settle_staged_after_db_failure(&app.state.db, &root, &referenced_rel)
            .await
            .expect("settle referenced");
    }
    assert_eq!(
        media_storage::read_bytes(&root, &referenced_rel)
            .await
            .expect("read published object"),
        referenced_bytes
    );

    let _ = tokio::fs::remove_dir_all(&root).await;
    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires mongo"]
async fn reconciler_recovers_pending_removes_orphans_and_disables_missing_rows() {
    let app = common::TestApp::start().await;
    let root = unique_root("media_reconcile");

    let recover_bytes = b"recover me";
    let recover_sha = media_storage::sha256_hex(recover_bytes);
    let recover_rel =
        media_storage::safe_relative_path("default", &recover_sha, "pdf").expect("recover path");
    {
        let _guards = media_storage::lock_paths(&root, [recover_rel.clone()])
            .await
            .expect("lock recover path");
        media_storage::stage_bytes(&root, &recover_rel, recover_bytes)
            .await
            .expect("stage recover object");
    }
    app.state
        .db
        .content_assets()
        .insert_one(
            media_asset(ObjectId::new(), "default", recover_rel.clone(), recover_sha),
            None,
        )
        .await
        .expect("insert recover reference");

    let orphan_bytes = b"orphan final";
    let orphan_sha = media_storage::sha256_hex(orphan_bytes);
    let orphan_rel =
        media_storage::safe_relative_path("default", &orphan_sha, "pdf").expect("orphan path");
    media_storage::store_bytes(&root, &orphan_rel, orphan_bytes)
        .await
        .expect("store orphan final");

    let pending_orphan_bytes = b"orphan staged";
    let pending_orphan_sha = media_storage::sha256_hex(pending_orphan_bytes);
    let pending_orphan_rel =
        media_storage::safe_relative_path("default", &pending_orphan_sha, "pdf")
            .expect("pending orphan path");
    {
        let _guards = media_storage::lock_paths(&root, [pending_orphan_rel.clone()])
            .await
            .expect("lock pending orphan");
        media_storage::stage_bytes(&root, &pending_orphan_rel, pending_orphan_bytes)
            .await
            .expect("stage pending orphan");
    }

    let missing_sha = media_storage::sha256_hex(b"missing object");
    let missing_rel =
        media_storage::safe_relative_path("default", &missing_sha, "pdf").expect("missing path");
    let missing_id = ObjectId::new();
    app.state
        .db
        .content_assets()
        .insert_one(
            media_asset(missing_id, "default", missing_rel.clone(), missing_sha),
            None,
        )
        .await
        .expect("insert missing reference");

    let corrupt_expected = b"complete object";
    let corrupt_sha = media_storage::sha256_hex(corrupt_expected);
    let corrupt_rel =
        media_storage::safe_relative_path("default", &corrupt_sha, "pdf").expect("corrupt path");
    let corrupt_id = ObjectId::new();
    {
        let _guards = media_storage::lock_paths(&root, [corrupt_rel.clone()])
            .await
            .expect("lock corrupt path");
        media_storage::stage_bytes(&root, &corrupt_rel, b"truncated")
            .await
            .expect("stage corrupt object");
    }
    app.state
        .db
        .content_assets()
        .insert_one(
            media_asset(corrupt_id, "default", corrupt_rel.clone(), corrupt_sha),
            None,
        )
        .await
        .expect("insert corrupt reference");

    let report = media_storage::reconcile_once(&app.state.db, &root)
        .await
        .expect("reconcile media storage");
    assert_eq!(report.recovered_pending, 1);
    assert_eq!(report.removed_pending, 2);
    assert_eq!(report.removed_orphans, 1);
    assert_eq!(report.disabled_missing_assets, 2);
    assert_eq!(
        media_storage::read_bytes(&root, &recover_rel)
            .await
            .expect("recovered final"),
        recover_bytes
    );
    assert!(!root.join(orphan_rel).exists());
    assert!(!root
        .join(media_storage::pending_relative_path(&pending_orphan_rel).expect("pending path"))
        .exists());

    let missing = app
        .state
        .db
        .content_assets()
        .find_one(doc! { "_id": missing_id }, None)
        .await
        .expect("query missing row")
        .expect("missing row remains for operator repair");
    assert_eq!(missing.sendable, Some(false));
    assert_eq!(missing.review_status.as_deref(), Some("draft"));
    assert_eq!(
        missing.review_note.as_deref(),
        Some("storage_object_missing")
    );
    assert!(missing.media_id.is_none());

    let corrupt = app
        .state
        .db
        .content_assets()
        .find_one(doc! { "_id": corrupt_id }, None)
        .await
        .expect("query corrupt row")
        .expect("corrupt row remains for operator repair");
    assert_eq!(corrupt.sendable, Some(false));
    assert_eq!(corrupt.review_status.as_deref(), Some("draft"));
    assert_eq!(
        corrupt.review_note.as_deref(),
        Some("storage_object_missing")
    );
    assert!(!root.join(&corrupt_rel).exists());
    assert!(!root
        .join(media_storage::pending_relative_path(&corrupt_rel).expect("corrupt pending path"))
        .exists());

    let corrupt_final_expected = b"expected final content";
    let corrupt_final_sha = media_storage::sha256_hex(corrupt_final_expected);
    let corrupt_final_rel = media_storage::safe_relative_path("default", &corrupt_final_sha, "pdf")
        .expect("corrupt final path");
    media_storage::store_bytes(&root, &corrupt_final_rel, b"wrong final content")
        .await
        .expect("store corrupt final");
    let corrupt_final_id = ObjectId::new();
    app.state
        .db
        .content_assets()
        .insert_one(
            media_asset(
                corrupt_final_id,
                "default",
                corrupt_final_rel.clone(),
                corrupt_final_sha,
            ),
            None,
        )
        .await
        .expect("insert corrupt final reference");

    let invalid_path_id = ObjectId::new();
    app.state
        .db
        .content_assets()
        .insert_one(
            media_asset(
                invalid_path_id,
                "default",
                "../outside.pdf".to_string(),
                media_storage::sha256_hex(b"outside"),
            ),
            None,
        )
        .await
        .expect("insert invalid path reference");

    let second_report = media_storage::reconcile_once(&app.state.db, &root)
        .await
        .expect("reconcile corrupt final and invalid path");
    assert_eq!(second_report.removed_corrupt, 1);
    assert_eq!(second_report.disabled_missing_assets, 1);
    assert_eq!(second_report.disabled_invalid_assets, 1);
    assert!(!root.join(&corrupt_final_rel).exists());

    let corrupt_final = app
        .state
        .db
        .content_assets()
        .find_one(doc! { "_id": corrupt_final_id }, None)
        .await
        .expect("query corrupt final row")
        .expect("corrupt final row remains for operator repair");
    assert_eq!(corrupt_final.sendable, Some(false));
    assert_eq!(
        corrupt_final.review_note.as_deref(),
        Some("storage_object_missing")
    );

    let invalid_path = app
        .state
        .db
        .content_assets()
        .find_one(doc! { "_id": invalid_path_id }, None)
        .await
        .expect("query invalid path row")
        .expect("invalid path row remains for operator repair");
    assert_eq!(invalid_path.sendable, Some(false));
    assert_eq!(
        invalid_path.review_note.as_deref(),
        Some("storage_path_invalid")
    );

    let third_report = media_storage::reconcile_once(&app.state.db, &root)
        .await
        .expect("idempotent reconcile");
    assert_eq!(third_report, media_storage::ReconcileReport::default());

    let _ = tokio::fs::remove_dir_all(&root).await;
    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires mongo"]
async fn path_lock_closes_zero_reference_then_new_reference_delete_race() {
    let app = common::TestApp::start().await;
    let root = unique_root("media_barrier");
    let bytes = b"shared content";
    let sha = media_storage::sha256_hex(bytes);
    let rel = media_storage::safe_relative_path("default", &sha, "pdf").expect("shared path");
    media_storage::store_bytes(&root, &rel, bytes)
        .await
        .expect("store initial final");
    let old_id = ObjectId::new();
    app.state
        .db
        .content_assets()
        .insert_one(
            media_asset(old_id, "default", rel.clone(), sha.clone()),
            None,
        )
        .await
        .expect("insert old reference");

    let db_for_delete = app.state.db.clone();
    let root_for_delete = root.clone();
    let rel_for_delete = rel.clone();
    let (zero_tx, zero_rx) = tokio::sync::oneshot::channel();
    let (continue_tx, continue_rx) = tokio::sync::oneshot::channel();
    let delete_task = tokio::spawn(async move {
        let _guards = media_storage::lock_paths(&root_for_delete, [rel_for_delete.clone()])
            .await
            .expect("delete lock");
        db_for_delete
            .content_assets()
            .delete_one(doc! { "_id": old_id }, None)
            .await
            .expect("delete old reference");
        let refs = db_for_delete
            .content_assets()
            .count_documents(doc! { "file_path": &rel_for_delete }, None)
            .await
            .expect("count zero references");
        assert_eq!(refs, 0);
        zero_tx.send(()).expect("signal zero-reference window");
        continue_rx.await.expect("continue delete");
        media_storage::delete_bytes(&root_for_delete, &rel_for_delete)
            .await
            .expect("delete unreferenced final");
    });

    zero_rx.await.expect("wait for zero-reference window");
    let acquired = Arc::new(AtomicBool::new(false));
    let acquired_by_create = acquired.clone();
    let db_for_create = app.state.db.clone();
    let root_for_create = root.clone();
    let rel_for_create = rel.clone();
    let sha_for_create = sha.clone();
    let create_task = tokio::spawn(async move {
        let _guards = media_storage::lock_paths(&root_for_create, [rel_for_create.clone()])
            .await
            .expect("create lock");
        acquired_by_create.store(true, Ordering::SeqCst);
        let staged = media_storage::stage_bytes(&root_for_create, &rel_for_create, bytes)
            .await
            .expect("stage replacement object");
        db_for_create
            .content_assets()
            .insert_one(
                media_asset(
                    ObjectId::new(),
                    "default",
                    rel_for_create.clone(),
                    sha_for_create,
                ),
                None,
            )
            .await
            .expect("insert new reference");
        if staged {
            media_storage::publish_staged(&root_for_create, &rel_for_create)
                .await
                .expect("publish replacement object");
        }
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !acquired.load(Ordering::SeqCst),
        "new reference must wait while delete owns the content path"
    );
    continue_tx.send(()).expect("release delete barrier");
    delete_task.await.expect("delete task");
    create_task.await.expect("create task");

    assert_eq!(
        app.state
            .db
            .content_assets()
            .count_documents(doc! { "file_path": &rel }, None)
            .await
            .expect("count final references"),
        1
    );
    assert_eq!(
        media_storage::read_bytes(&root, &rel)
            .await
            .expect("final object remains readable"),
        bytes
    );

    let _ = tokio::fs::remove_dir_all(&root).await;
    app.cleanup().await;
}
