//! 鉴权红线集成测试:authenticate 不泄漏存在性 / session 三级回退 / 过期 / 登出幂等 /
//! switch_workspace ACL。全部 `#[ignore]`,需 Docker testcontainers。
//! CI `integration` job 用 `cargo test --test auth_middleware_integration -- --ignored` 跑。
//!
//! ## 形态:直调 auth::session 纯 async fn(取 &Database)+ 直调 switch_workspace handler。
//! session 函数本就 pub,无需提可见性;switch_workspace 本 Task 提为 pub。
#![cfg(test)]

mod common;

use axum::extract::{Extension, State};
use axum_extra::extract::cookie::CookieJar;

use wechatagent::auth::session::{
    authenticate, bootstrap_admin_if_needed, create_session, delete_session, lookup_session,
    AuthError,
};
use wechatagent::auth::{AdminSession, AdminUser, AuthenticatedAdmin};
use wechatagent::routes::auth::switch_workspace;

use crate::common::TestApp;

fn test_admin(user_id: &str, workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: user_id.to_string(),
        username: "auth_test".to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

/// 红线:用户不存在 与 密码错 必须返回同一 InvalidCredentials,不泄漏哪个错(账户枚举防护)。
#[tokio::test]
#[ignore]
async fn authenticate_does_not_leak_user_existence() {
    let app = TestApp::start().await;
    // seed 一个 admin(workspaces=[default])
    bootstrap_admin_if_needed(
        &app.state.db,
        Some("alice"),
        Some("correct-horse"),
        Some(&app.state.config.default_workspace_id),
    )
    .await
    .expect("bootstrap admin 失败");

    // 不存在的用户
    let no_user = authenticate(&app.state.db, "ghost", "whatever").await;
    // 存在但密码错
    let wrong_pw = authenticate(&app.state.db, "alice", "wrong-password").await;

    assert!(
        matches!(no_user, Err(AuthError::InvalidCredentials)),
        "不存在用户必须 InvalidCredentials,实际 {no_user:?}"
    );
    assert!(
        matches!(wrong_pw, Err(AuthError::InvalidCredentials)),
        "密码错必须 InvalidCredentials(与不存在同错,不泄漏存在性),实际 {wrong_pw:?}"
    );
    // 正确凭据成功
    assert!(
        authenticate(&app.state.db, "alice", "correct-horse")
            .await
            .is_ok(),
        "正确凭据应成功"
    );
    app.cleanup().await;
}

/// 设计意图:create_session 的 current_workspace 三级回退
/// authorized default_workspace → workspaces.first() → fallback。
#[tokio::test]
#[ignore]
async fn create_session_workspace_fallback_chain() {
    let app = TestApp::start().await;
    let db = &app.state.db;

    // ① default_workspace 在 ACL 内 → 用它
    let u1 = AdminUser {
        user_id: "u1".into(),
        username: "u1".into(),
        password_hash: "x".into(),
        created_at: chrono::Utc::now(),
        last_login_at: None,
        workspaces: vec!["ws_default".into(), "ws_list_first".into()],
        default_workspace: Some("ws_default".into()),
    };
    let s1 = create_session(db, &u1, 24, "ws_fallback")
        .await
        .expect("s1");
    assert_eq!(s1.current_workspace.as_deref(), Some("ws_default"));

    // default_workspace 已从 ACL 移除 → 不得继续授予，改用首个 ACL workspace。
    let stale_default = AdminUser {
        workspaces: vec!["ws_list_first".into()],
        ..u1.clone()
    };
    let stale = create_session(db, &stale_default, 24, "ws_fallback")
        .await
        .expect("stale default session");
    assert_eq!(stale.current_workspace.as_deref(), Some("ws_list_first"));

    // ② 无 default、有 workspaces → 用 workspaces.first()
    let u2 = AdminUser {
        default_workspace: None,
        workspaces: vec!["ws_list_first".into()],
        ..u1.clone()
    };
    let s2 = create_session(db, &u2, 24, "ws_fallback")
        .await
        .expect("s2");
    assert_eq!(s2.current_workspace.as_deref(), Some("ws_list_first"));

    // ③ 空 ACL 是完整撤权，不再回落默认 workspace。
    let u3 = AdminUser {
        default_workspace: None,
        workspaces: vec![],
        ..u1.clone()
    };
    let s3 = create_session(db, &u3, 24, "ws_fallback").await;
    app.cleanup().await;
    assert!(matches!(s3, Err(AuthError::NoAuthorizedWorkspace)));
}

/// 红线：真实过期 session lookup 必须返回 SessionExpired；活跃 session 登出保持幂等。
/// 完整 Cookie middleware 的 401 由 `sr176_real_route_isolation` 走真实 Router 覆盖。
#[tokio::test]
#[ignore]
async fn expired_session_rejected_and_logout_idempotent() {
    let app = TestApp::start().await;
    let user = AdminUser {
        user_id: "u_exp".into(),
        username: "u_exp".into(),
        password_hash: "x".into(),
        created_at: chrono::Utc::now(),
        last_login_at: None,
        workspaces: vec!["ws".into()],
        default_workspace: Some("ws".into()),
    };
    let expired = AdminSession {
        session_id: "auth-test-expired-session".into(),
        admin_user_id: user.user_id.clone(),
        username: user.username.clone(),
        created_at: chrono::Utc::now() - chrono::Duration::hours(2),
        expires_at: chrono::Utc::now() - chrono::Duration::hours(1),
        current_workspace: Some("ws".into()),
    };
    app.state
        .db
        .raw()
        .collection::<AdminSession>("admin_sessions")
        .insert_one(&expired, None)
        .await
        .expect("insert expired session");
    let expired_result = lookup_session(&app.state.db, &expired.session_id).await;
    let expired_rejected = matches!(expired_result, Err(AuthError::SessionExpired));

    let s = create_session(&app.state.db, &user, 1, "ws")
        .await
        .expect("create active session");
    let active_lookup_ok = lookup_session(&app.state.db, &s.session_id).await.is_ok();

    // 登出幂等:删两次都不报错
    delete_session(&app.state.db, &s.session_id)
        .await
        .expect("logout 1");
    delete_session(&app.state.db, &s.session_id)
        .await
        .expect("logout 2(幂等)");

    // 删除后 lookup → SessionNotFound
    let after = lookup_session(&app.state.db, &s.session_id).await;
    let logout_removed = matches!(after, Err(AuthError::SessionNotFound));

    // 外部 Mongo 模式下始终先清理随机库，再执行断言；失败证据也不遗留测试数据。
    app.cleanup().await;

    assert!(
        expired_rejected,
        "expires_at<=now 必须返回 SessionExpired，实际 {expired_result:?}"
    );
    assert!(active_lookup_ok, "新 session 应可查到");
    assert!(
        logout_removed,
        "删除后 lookup 必须 SessionNotFound,实际 {after:?}"
    );
}

/// 红线:switch_workspace 切到 user.workspaces 之外的 workspace 必须被拒(workspace_not_in_user_acl)。
#[tokio::test]
#[ignore]
async fn switch_workspace_rejects_outside_acl() {
    let app = TestApp::start().await;
    // seed user(workspaces=[default_workspace_id])。用唯一的 default ws,使 "ws_b" 必在 ACL 外。
    let own_ws = app.state.config.default_workspace_id.clone();
    bootstrap_admin_if_needed(
        &app.state.db,
        Some("acl_user"),
        Some("pw-123456"),
        Some(&own_ws),
    )
    .await
    .expect("bootstrap admin 失败");
    // 用公开路径 authenticate 拿回真实 user_id(switch_workspace 按 user_id 查 ACL)。
    let user = authenticate(&app.state.db, "acl_user", "pw-123456")
        .await
        .expect("authenticate 失败");

    // 切到 ACL 外的 ws_b → BadRequest("workspace_not_in_user_acl")(拒绝在拿 cookie 之前,空 jar 可)
    let result = switch_workspace(
        State(app.state.clone()),
        Extension(test_admin(&user.user_id, &own_ws)),
        CookieJar::new(),
        axum::Json(
            serde_json::from_value(serde_json::json!({ "workspaceId": "ws_b_not_in_acl" }))
                .expect("构造请求体失败"),
        ),
    )
    .await;

    assert!(
        result.is_err(),
        "切到 ACL 外 workspace 必须被拒(workspace_not_in_user_acl)"
    );

    // 反向:切到自己 ACL 内的 own_ws 不应因 ACL 被拒(可能因缺 cookie 返 Unauthorized,但不是 ACL 拒绝)
    let in_acl = switch_workspace(
        State(app.state.clone()),
        Extension(test_admin(&user.user_id, &own_ws)),
        CookieJar::new(),
        axum::Json(
            serde_json::from_value(serde_json::json!({ "workspaceId": own_ws.clone() }))
                .expect("构造请求体失败"),
        ),
    )
    .await;
    // own_ws 在 ACL 内 → 不会是 "workspace_not_in_user_acl";此处只断言错误信息不是 ACL 拒绝。
    if let Err(e) = &in_acl {
        assert!(
            !format!("{e:?}").contains("workspace_not_in_user_acl"),
            "ACL 内的 workspace 不应触发 ACL 拒绝,实际 {e:?}"
        );
    }
    app.cleanup().await;
}
