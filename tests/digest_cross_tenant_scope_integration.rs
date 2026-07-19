//! digest 跨租户 scope 缺陷回归钉(P1_closed_loop / 多租户隔离)。
//! 全部 `#[ignore]`,需 Docker testcontainers。
//! CI:`cargo test --test digest_cross_tenant_scope_integration -- --ignored`。
//!
//! ## 缺陷背景(2026-07-02 Stage4 孤儿#digest.1 确证为真缺陷 → 已修)
//! `GET /api/knowledge/digest/today` 的 `digest_today` handler 按
//! `admin.current_workspace + query.accountId` 查 `knowledge_daily_reports`
//! (digest_inbox.rs:47-58)。**未命中**时兜底调 `generate_today_digest` 同步合成。
//! 旧代码 `generate_today_digest(&state)` **只收 state、不收租户**,函数体
//! **硬编码** `state.config.default_workspace_id / default_account_id`
//! (mod.rs 旧:740-741)。后果:非 default workspace 的 admin 触发按需合成时——
//!   1. 拿到并展示的是 **default 租户**的日报(跨租户读泄漏);
//!   2. 合成结果 upsert 落库时 workspace_id 写成 default,该 admin 下次
//!      `find_one`(按自己 ws 查)仍 miss → 每次刷新都重复烧 LLM,永远看不到
//!      属于自己的报告(跨租户写)。
//!
//! ## 修复
//! `generate_today_digest(state, workspace_id, account_id)` 增租户入参;
//! 两个 route(`digest_today` / `digest_regenerate`)传 `admin.current_workspace`,
//! worker 传 default。
//!
//! ## 本测试如何钉死接线(删修复即变红)
//! 以**非 default** workspace(`ws_tenant_b`)的 admin 直调 `digest_today`,
//! DB 无当日报告 → 触发兜底合成。断言:
//!   1. 返回报告 `workspaceId == "ws_tenant_b"`(修复前 = "default");
//!   2. 落库在 `ws_tenant_b` 下有该报告(修复前 = 0);
//!   3. 落库在 `default` 下**无**该报告(修复前 = 有,即跨租户串写)。
//! 把 `generate_today_digest` 的租户入参改回硬编码 default,三条断言全翻 → 变红。
//!
//! ## 测试形态(state-only 直调 handler)
//! 沿用 `tests/knowledge_auto_verify_enforce_integration.rs` 惯例:`TestApp`
//! state-only 工厂,直调 route handler(axum extractor 手工构造)。
#![cfg(test)]

mod common;

use axum::extract::{Extension, Query, State};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde_json::json;

use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::models::KnowledgeDailyReport;
// `digest_today` handler + query 结构经 routes::ext_knowledge 再导出(handler 本身
// 是 pub(in crate::routes),ext_knowledge 是集成测试绕 axum 直驱的既定通道)。
use wechatagent::routes::ext_knowledge::{digest_today, DigestTodayQuery};

use crate::common::TestApp;

/// 仿 `knowledge_auto_verify_enforce_integration::test_admin`:`current_workspace`
/// 决定 handler 的可见/可写租户范围——本缺陷的命门就在它是否被透传下去。
fn test_admin(workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: "digest_tenant_admin".to_string(),
        username: "digest_tenant_admin".to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

/// 红线:非 default workspace 的 admin 触发按需合成,报告必须落在**它自己的**
/// workspace,绝不串到 default 租户。
///
/// 删除 digest_inbox.rs 里向 `generate_today_digest` 透传 `admin.current_workspace`
/// 的实参(改回硬编码 default),本测试三条断言全翻 → 变红。
#[tokio::test]
#[ignore]
async fn digest_today_synth_uses_admin_workspace_not_default() {
    let app = TestApp::start().await;
    let default_ws = app.state.config.default_workspace_id.clone();
    let tenant_ws = "ws_tenant_b";
    // 前置校验:测试租户与 default 必须不同,否则本用例失去区分力。
    assert_ne!(
        tenant_ws, default_ws,
        "测试租户不能等于 default_workspace_id,否则测不出跨租户串写"
    );

    // compose_cards 恒调一次 LLM(mod.rs:554);空 DB 无 blocked 运行 → 0 次
    // summarize_logs。多押 1 条冗余 mock 无害(TestLlmGenerator 留在队列不消费),
    // 让合成走 status=ok 的 happy path。返 `{"cards": []}` → 空卡片报告即可。
    for _ in 0..2 {
        app.llm.push_response(json!({ "cards": [] }));
    }

    // 以 tenant_ws admin 直调 handler:DB 无当日报告 → 触发兜底 generate_today_digest。
    // report_date/account_id 均缺省(用今天 + default_account_id),与合成入口对齐。
    let resp = digest_today(
        State(app.state.clone()),
        Extension(test_admin(tenant_ws)),
        Query(DigestTodayQuery {
            account_id: None,
            report_date: None,
        }),
    )
    .await
    .expect("digest_today handler 应成功(兜底合成)");

    // ── 断言 1:返回报告归属 tenant_ws(handler 出参层) ──
    let body = resp.0;
    assert_eq!(
        body["workspaceId"].as_str(),
        Some(tenant_ws),
        "返回报告必须归属 admin 的 workspace={tenant_ws}(修复前硬编码 default={default_ws}),实际 body={body:?}"
    );

    // ── 断言 2 & 3:落库复查(DB 层) ──
    let all_reports = app
        .state
        .db
        .knowledge_daily_reports()
        .find(doc! {}, None)
        .await
        .expect("查日报应成功")
        .try_collect::<Vec<KnowledgeDailyReport>>()
        .await
        .expect("collect 日报应成功");

    let in_tenant = all_reports
        .iter()
        .filter(|r| r.workspace_id == tenant_ws)
        .count();
    let in_default = all_reports
        .iter()
        .filter(|r| r.workspace_id == default_ws)
        .count();

    assert_eq!(
        in_tenant,
        1,
        "合成的日报必须落在 tenant_ws={tenant_ws} 下(修复前 = 0,因写成了 default),实际落库={:?}",
        all_reports
            .iter()
            .map(|r| r.workspace_id.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        in_default,
        0,
        "绝不得跨租户串写到 default={default_ws}(修复前 = 1),实际落库={:?}",
        all_reports
            .iter()
            .map(|r| r.workspace_id.clone())
            .collect::<Vec<_>>()
    );
}
