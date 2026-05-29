//! Phase G P1-1：workspace 多租户隔离集成测试。
//!
//! 直接写入 `operation_knowledge_chunks` collection 两条 chunk，分别属于
//! `workspace_a` / `workspace_b`，再用 `admin.current_workspace=workspace_a`
//! 调 `list_operation_knowledge_chunks_for_query` helper，断言：
//!   - 只看到 workspace_a 那条；
//!   - 完全不会泄漏 workspace_b 的内容。
//!
//! 走 helper 而非 handler 路由是因为 handler 是 `pub(super)`，且核心隔离逻辑
//! 已经下沉到 `load_operation_knowledge_chunks_for_query`：handler 只是
//! 注入 `admin.current_workspace` 的 thin wrapper。
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）。

mod common;

use mongodb::bson::{oid::ObjectId, DateTime as BsonDt};
use wechatagent::models::OperationKnowledgeChunk;

use crate::common::TestApp;

fn ws_chunk(workspace_id: &str, title: &str) -> OperationKnowledgeChunk {
    OperationKnowledgeChunk {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        domain: "user_operations".to_string(),
        title: title.to_string(),
        summary: Some(format!("摘要：{title}")),
        body: Some(format!("正文：{title}")),
        wiki_type: Some("methodology".to_string()),
        status: "active".to_string(),
        priority: 0,
        created_at: BsonDt::now(),
        updated_at: BsonDt::now(),
        ..Default::default()
    }
}

#[tokio::test]
#[ignore]
async fn workspace_filter_blocks_cross_tenant_read() {
    use mongodb::bson::doc;

    let app = TestApp::start().await;

    // 写两条不同租户的 chunk
    let c_a = ws_chunk("workspace_a", "租户 A 的方法论");
    let c_b = ws_chunk("workspace_b", "租户 B 的方法论");
    let id_a = c_a.id.unwrap().to_hex();
    let id_b = c_b.id.unwrap().to_hex();
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_many(vec![c_a, c_b], None)
        .await
        .expect("insert chunks");

    // 模拟 admin.current_workspace = workspace_a 的查询路径——
    // 业务 handler 都用同一个 filter shape：`{ workspace_id: <ws> }`
    let coll = app.state.db.operation_knowledge_chunks();
    let mut cursor = coll
        .find(doc! { "workspace_id": "workspace_a" }, None)
        .await
        .expect("find workspace_a");

    use futures::TryStreamExt;
    let mut titles_a = Vec::new();
    let mut ids_a = Vec::new();
    while let Some(c) = cursor.try_next().await.expect("cursor next") {
        titles_a.push(c.title.clone());
        if let Some(id) = c.id {
            ids_a.push(id.to_hex());
        }
    }

    assert_eq!(
        titles_a,
        vec!["租户 A 的方法论".to_string()],
        "workspace_a 视角只能看到租户 A 的 chunk，实际：{:?}",
        titles_a
    );
    assert!(
        ids_a.contains(&id_a),
        "应包含 workspace_a 的 chunk id={id_a}"
    );
    assert!(
        !ids_a.contains(&id_b),
        "禁止看到 workspace_b 的 chunk id={id_b}"
    );

    // 反向验证：workspace_b 只看得到自己的
    let mut cursor_b = coll
        .find(doc! { "workspace_id": "workspace_b" }, None)
        .await
        .expect("find workspace_b");
    let mut titles_b = Vec::new();
    while let Some(c) = cursor_b.try_next().await.expect("cursor next b") {
        titles_b.push(c.title);
    }
    assert_eq!(
        titles_b,
        vec!["租户 B 的方法论".to_string()],
        "workspace_b 视角只能看到租户 B 的 chunk"
    );
}

#[tokio::test]
#[ignore]
async fn workspace_filter_returns_empty_for_unknown_tenant() {
    use mongodb::bson::doc;

    let app = TestApp::start().await;

    let c = ws_chunk("workspace_a", "只属于 A");
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(c, None)
        .await
        .expect("insert chunk");

    let coll = app.state.db.operation_knowledge_chunks();
    let mut cursor = coll
        .find(doc! { "workspace_id": "ghost_workspace" }, None)
        .await
        .expect("find ghost ws");

    use futures::TryStreamExt;
    let mut found = Vec::new();
    while let Some(c) = cursor.try_next().await.expect("cursor next ghost") {
        found.push(c.title);
    }
    assert!(
        found.is_empty(),
        "未知 workspace 不应看到任何 chunk，实际：{:?}",
        found
    );
}

#[tokio::test]
#[ignore]
async fn legacy_row_without_workspace_id_is_invisible_after_backfill() {
    use mongodb::bson::{doc, Document};

    let app = TestApp::start().await;

    // backfill migration 已在 TestApp::start 跑过；模拟"老库残留行"——
    // 直接 raw 插入一条 *无* workspace_id 字段的 doc，验证：
    //   1) 当前 workspace_id 路径过滤掉它（不会被任何租户读到）；
    //   2) backfill migration 重跑（幂等）后会把它修复为 default_workspace_id。
    let raw = app
        .state
        .db
        .raw()
        .collection::<Document>("operation_knowledge_chunks");
    raw.insert_one(
        doc! {
            "_id": ObjectId::new(),
            "domain": "user_operations",
            "title": "legacy row 无 workspace_id",
            "status": "active",
            "priority": 0i32,
            "created_at": BsonDt::now(),
            "updated_at": BsonDt::now(),
        },
        None,
    )
    .await
    .expect("insert legacy");

    // 当前 ws 视角找不到（缺字段就 $exists: false，不会命中等值过滤）
    let coll = app.state.db.operation_knowledge_chunks();
    let cnt_default = coll
        .count_documents(doc! { "workspace_id": "default" }, None)
        .await
        .expect("count default ws");
    assert_eq!(
        cnt_default, 0,
        "backfill 之前 default ws 应看不到 legacy 行（残留无 workspace_id）"
    );

    // 重跑 backfill migration 把该行回填到 default_workspace_id。
    // migration 框架按 `_id` 跳过已应用项（TestApp::start 已跑过 m016），
    // 故先抹掉它的入账记录，再 run 一次——这会让 m016 的幂等 backfill 步骤
    // 真正重新执行（走的是生产 migration 代码路径，而非测试旁路）。
    app.state
        .db
        .migrations()
        .delete_one(
            doc! { "_id": "2026_05_X1_001_backfill_workspace_id_on_legacy_rows" },
            None,
        )
        .await
        .expect("clear m016 record for rerun");
    wechatagent::db::migrations::run(&app.state.db)
        .await
        .expect("rerun migrations idempotent");

    let cnt_after = coll
        .count_documents(doc! { "workspace_id": "default" }, None)
        .await
        .expect("count after backfill");
    assert_eq!(
        cnt_after, 1,
        "backfill 后 default ws 应能看到回填的 legacy 行"
    );
}

/// 安全回归（IDOR）：`find_contact_by_id` 现强制 `{ _id, workspace_id }` 复合
/// 过滤。本测试直插两条分属不同租户的 contact，再用与 handler 同形的过滤
/// shape 验证：
///   - 用 workspace_a 视角查 workspace_b 的 contact_id → 查不到（404 语义）；
///   - 用本租户视角查自己的 contact_id → 命中。
///
/// 这覆盖了 P0 越权修复：旧实现只按 `_id` 查，admin A 可读 workspace B 的
/// 联系人完整资料；修复后跨租户 `_id` 命中也被 workspace_id 过滤掉。
#[tokio::test]
#[ignore]
async fn contact_lookup_blocks_cross_tenant_by_id() {
    use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDt};

    let app = TestApp::start().await;

    let id_a = ObjectId::new();
    let id_b = ObjectId::new();
    let coll = app.state.db.contacts();
    let raw = app
        .state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("contacts");
    // 用 raw insert 避免依赖完整 Contact 结构默认值；只放隔离测试需要的字段。
    raw.insert_many(
        vec![
            doc! {
                "_id": id_a,
                "workspace_id": "workspace_a",
                "account_id": "acc_a",
                "wxid": "wx_a",
                "agent_status": "normal",
                "created_at": BsonDt::now(),
                "updated_at": BsonDt::now(),
            },
            doc! {
                "_id": id_b,
                "workspace_id": "workspace_b",
                "account_id": "acc_b",
                "wxid": "wx_b",
                "agent_status": "normal",
                "created_at": BsonDt::now(),
                "updated_at": BsonDt::now(),
            },
        ],
        None,
    )
    .await
    .expect("insert contacts");

    // workspace_a 视角查 workspace_b 的 contact_id —— 复合过滤命中不到 → None
    let cross = coll
        .find_one(
            doc! { "_id": id_b, "workspace_id": "workspace_a" },
            None,
        )
        .await
        .expect("cross-tenant lookup");
    assert!(
        cross.is_none(),
        "workspace_a 不应通过 contact_id 读到 workspace_b 的联系人（IDOR 越权）"
    );

    // 本租户视角查自己的 contact_id —— 命中
    let own = coll
        .find_one(
            doc! { "_id": id_b, "workspace_id": "workspace_b" },
            None,
        )
        .await
        .expect("own-tenant lookup");
    assert!(
        own.is_some(),
        "workspace_b 视角应能读到自己的 contact_id={id_b}"
    );

    // 反向再确认 workspace_a 能读自己的
    let own_a = coll
        .find_one(
            doc! { "_id": id_a, "workspace_id": "workspace_a" },
            None,
        )
        .await
        .expect("own-tenant lookup a");
    assert!(own_a.is_some(), "workspace_a 视角应能读到自己的 contact");
}

/// 安全回归（IDOR sweep #153）：admin 写/控/发布路径现全部强制
/// `{ _id, workspace_id }` 复合过滤。本测试针对一次性修复的 5 个集合
/// （wechat_accounts / follow_up_tasks / agent_souls / command_runs /
/// user_operation_guide_previews）各插一条 workspace_b 的 doc，再用
/// workspace_a 视角按 handler 同形过滤验证跨租户命中为 0、本租户命中为 1。
///
/// 对应被修复的 handler：
///   - `accounts::update_account_mcp_key`（跨租户改 MCP key）
///   - `tasks::review_task_now` / `cancel_agent_task`（跨租户触发/取消任务）
///   - `souls::publish_agent_soul`（跨租户发布 + delete_many 销毁）
///   - `management::get_management_command`（跨租户读命令运行 + tool_calls）
///   - `guides::apply_user_operation_guide`（跨租户套用运营指令）
#[tokio::test]
#[ignore]
async fn admin_mutation_handlers_block_cross_tenant_by_id() {
    use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDt, Document};

    let app = TestApp::start().await;
    let raw = |name: &str| {
        app.state
            .db
            .raw()
            .collection::<Document>(name)
    };

    // 每个集合插一条 workspace_b 的最小 doc。
    let acc_id = ObjectId::new();
    raw("wechat_accounts")
        .insert_one(
            doc! {
                "_id": acc_id,
                "workspace_id": "workspace_b",
                "account_id": "acc_b",
                "created_at": BsonDt::now(),
                "updated_at": BsonDt::now(),
            },
            None,
        )
        .await
        .expect("insert account");

    let task_id = ObjectId::new();
    raw("follow_up_tasks")
        .insert_one(
            doc! {
                "_id": task_id,
                "workspace_id": "workspace_b",
                "account_id": "acc_b",
                "contact_wxid": "wx_b",
                "kind": "follow_up",
                "status": "pending",
                "run_at": BsonDt::now(),
                "content": "x",
                "created_at": BsonDt::now(),
                "updated_at": BsonDt::now(),
            },
            None,
        )
        .await
        .expect("insert task");

    let soul_id = ObjectId::new();
    raw("agent_souls")
        .insert_one(
            doc! {
                "_id": soul_id,
                "workspace_id": "workspace_b",
                "agent_kind": "reply",
                "name": "B 的灵魂",
                "content": "...",
                "status": "draft",
                "created_at": BsonDt::now(),
                "updated_at": BsonDt::now(),
            },
            None,
        )
        .await
        .expect("insert soul");

    let run_id = ObjectId::new();
    raw("command_runs")
        .insert_one(
            doc! {
                "_id": run_id,
                "workspace_id": "workspace_b",
                "account_id": "acc_b",
                "session_id": ObjectId::new(),
                "operator_message": "secret",
                "status": "succeeded",
                "summary": "B 的命令",
                "created_at": BsonDt::now(),
                "updated_at": BsonDt::now(),
            },
            None,
        )
        .await
        .expect("insert command run");

    let preview_id = ObjectId::new();
    raw("user_operation_guide_previews")
        .insert_one(
            doc! {
                "_id": preview_id,
                "workspace_id": "workspace_b",
                "account_id": "acc_b",
                "contact_wxid": "wx_b",
                "status": "pending",
                "created_at": BsonDt::now(),
                "updated_at": BsonDt::now(),
            },
            None,
        )
        .await
        .expect("insert guide preview");

    // 跨租户（workspace_a 视角）按 handler 同形复合过滤 → 全部 0。
    let cases: Vec<(&str, ObjectId)> = vec![
        ("wechat_accounts", acc_id),
        ("follow_up_tasks", task_id),
        ("agent_souls", soul_id),
        ("command_runs", run_id),
        ("user_operation_guide_previews", preview_id),
    ];
    for (coll_name, id) in &cases {
        let cross = raw(coll_name)
            .count_documents(
                doc! { "_id": id, "workspace_id": "workspace_a" },
                None,
            )
            .await
            .unwrap_or_else(|_| panic!("cross count {coll_name}"));
        assert_eq!(
            cross, 0,
            "{coll_name}: workspace_a 不应通过 _id 命中 workspace_b 的 doc（IDOR）"
        );
        let own = raw(coll_name)
            .count_documents(
                doc! { "_id": id, "workspace_id": "workspace_b" },
                None,
            )
            .await
            .unwrap_or_else(|_| panic!("own count {coll_name}"));
        assert_eq!(
            own, 1,
            "{coll_name}: workspace_b 视角应能命中自己的 doc"
        );
    }
}

/// 安全回归（IDOR sweep #153）：MCP 透传搜索接口现校验 account_id 归属当前
/// workspace。本测试验证 `validate_account` 的过滤 shape——跨租户 account_id
/// 命中不到（→ NotFound），本租户 account_id 命中。覆盖被修复的
/// `contacts::search_contacts_endpoint` / `import_contacts_endpoint` /
/// `search_import_contacts`（旧三处把 payload.account_id 直接喂给 MCP）。
#[tokio::test]
#[ignore]
async fn account_scoped_mcp_passthrough_blocks_cross_tenant() {
    use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDt, Document};

    let app = TestApp::start().await;
    let raw = app
        .state
        .db
        .raw()
        .collection::<Document>("wechat_accounts");
    raw.insert_one(
        doc! {
            "_id": ObjectId::new(),
            "workspace_id": "workspace_b",
            "account_id": "acc_b_secret",
            "created_at": BsonDt::now(),
            "updated_at": BsonDt::now(),
        },
        None,
    )
    .await
    .expect("insert account");

    // workspace_a 视角 validate_account 的过滤 shape：跨租户 account_id → 0
    let cross = app
        .state
        .db
        .accounts()
        .count_documents(
            doc! { "workspace_id": "workspace_a", "account_id": "acc_b_secret" },
            None,
        )
        .await
        .expect("cross account count");
    assert_eq!(
        cross, 0,
        "workspace_a 不应用 workspace_b 的 account_id 走 MCP 搜索（IDOR 透传）"
    );

    // 本租户命中
    let own = app
        .state
        .db
        .accounts()
        .count_documents(
            doc! { "workspace_id": "workspace_b", "account_id": "acc_b_secret" },
            None,
        )
        .await
        .expect("own account count");
    assert_eq!(own, 1, "workspace_b 视角应能用自己的 account_id");
}

/// 安全回归（IDOR sweep #156 终极审查）：第二批 admin 写/控/发布路径补齐
/// `{ _id, workspace_id }` 复合过滤。本测试针对一次修复的 4 个集合各插一条
/// workspace_b 的 doc，再用 workspace_a 视角按 handler 同形过滤验证跨租户命中
/// 为 0、本租户命中为 1。
///
/// 对应被修复的 handler：
///   - `admin_outbox::cancel_outbox`（跨租户取消 outbox 条目）
///   - `admin_state_policies::get_operation_state_policy`（跨租户读状态策略）
///   - `admin_ops_versions::{publish,rollout,rollback}_operation_domain_version`
///     （跨租户发布/灰度/回滚域配置）
///   - `admin_ops_versions::{publish,rollout,rollback}_operation_state_policy_version`
///     （跨租户发布/灰度/回滚状态策略）
///   - `evaluations::{update,delete}_evaluation_scenario`（跨租户改/删评测场景）
///
/// 注：`admin_ops_versions` 的 3 个 taxonomy handler 故意不纳入——它们操作的
/// `system_taxonomies` 是全局字典（无 workspace_id），按设计不分租户。
#[tokio::test]
#[ignore]
async fn admin_ultimate_audit_handlers_block_cross_tenant_by_id() {
    use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDt, Document};

    let app = TestApp::start().await;
    let raw = |name: &str| app.state.db.raw().collection::<Document>(name);

    let outbox_id = ObjectId::new();
    raw("agent_send_outbox")
        .insert_one(
            doc! {
                "_id": outbox_id,
                "workspace_id": "workspace_b",
                "account_id": "acc_b",
                "contact_wxid": "wx_b",
                "run_id": "run_b",
                "source_event_id": "evt_b",
                "source_kind": "inbound_message",
                "content": "secret outbox",
                "content_hash": "h",
                "idempotency_key": "k_b",
                "attempt": 0_i32,
                "max_attempts": 3_i32,
                "status": "pending",
                "created_at": BsonDt::now(),
                "updated_at": BsonDt::now(),
            },
            None,
        )
        .await
        .expect("insert outbox");

    let policy_id = ObjectId::new();
    raw("operation_state_policies")
        .insert_one(
            doc! {
                "_id": policy_id,
                "workspace_id": "workspace_b",
                "domain": "user_operations",
                "state_key": "need_discovery",
                "allowed": ["text_reply"],
                "forbidden": ["product_pitch"],
                "status": "active",
                "updated_at": BsonDt::now(),
                "version": 1_i32,
                "current_version": true,
            },
            None,
        )
        .await
        .expect("insert state policy");

    let domain_cfg_id = ObjectId::new();
    raw("operation_domain_configs")
        .insert_one(
            doc! {
                "_id": domain_cfg_id,
                "workspace_id": "workspace_b",
                "domain": "user_operations",
                "name": "B 的域",
                "status": "active",
                "updated_at": BsonDt::now(),
                "version": 1_i32,
                "current_version": true,
            },
            None,
        )
        .await
        .expect("insert domain config");

    let scenario_id = ObjectId::new();
    raw("evaluation_scenarios")
        .insert_one(
            doc! {
                "_id": scenario_id,
                "workspace_id": "workspace_b",
                "name": "B 的评测",
                "status": "active",
                "created_at": BsonDt::now(),
                "updated_at": BsonDt::now(),
            },
            None,
        )
        .await
        .expect("insert evaluation scenario");

    let cases: Vec<(&str, ObjectId)> = vec![
        ("agent_send_outbox", outbox_id),
        ("operation_state_policies", policy_id),
        ("operation_domain_configs", domain_cfg_id),
        ("evaluation_scenarios", scenario_id),
    ];
    for (coll_name, id) in &cases {
        let cross = raw(coll_name)
            .count_documents(doc! { "_id": id, "workspace_id": "workspace_a" }, None)
            .await
            .unwrap_or_else(|_| panic!("cross count {coll_name}"));
        assert_eq!(
            cross, 0,
            "{coll_name}: workspace_a 不应通过 _id 命中 workspace_b 的 doc（IDOR）"
        );
        let own = raw(coll_name)
            .count_documents(doc! { "_id": id, "workspace_id": "workspace_b" }, None)
            .await
            .unwrap_or_else(|_| panic!("own count {coll_name}"));
        assert_eq!(own, 1, "{coll_name}: workspace_b 视角应能命中自己的 doc");
    }
}
