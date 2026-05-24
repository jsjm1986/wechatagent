use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use mongodb::bson::DateTime;
use tokio::net::TcpListener;
use tower_http::{
    cors::{Any, CorsLayer},
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use wechatagent::{
    config::AppConfig,
    db::{self, Database},
    llm::{LlmClient, LlmGenerator},
    mcp::McpClient,
    prompts,
    routes::{api_router, AppState},
    tasks, webhooks, APP_STARTED_AT,
};
use wechatagent::agent::run_outbox_dispatcher;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "wechatagent=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = AppConfig::from_env()?;
    // 在连接 DB 之前记录进程启动时间，供 HP-1 worker stale 回收逻辑使用。
    let _ = APP_STARTED_AT.set(DateTime::now());
    let db = Database::connect(&config.mongodb_uri, &config.mongodb_database).await?;
    db::migrations::run(&db).await?;
    db.ensure_indexes().await?;
    let llm: Arc<dyn LlmGenerator> = Arc::new(LlmClient::new(
        config.openai_base_url.clone(),
        config.openai_api_key.clone(),
        config.openai_model.clone(),
        config.llm_timeout_seconds,
        config.llm_max_retries,
        config.llm_retry_base_ms,
    )?);
    let state = AppState {
        db,
        mcp: McpClient::new(config.mcp_base_url.clone(), config.mcp_api_key.clone())?,
        llm,
        config: config.clone(),
        prompt_pack_version: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        chat_progress_bus: std::sync::Arc::new(
            wechatagent::knowledge_task::ChatProgressBus::new(),
        ),
    };
    prompts::ensure_prompt_pack_v2(
        &state.db,
        &state.config.default_workspace_id,
        &state.config.default_account_id,
    )
    .await?;
    // M4 W2 Task 3.2：种入演化器 Critic prompt（不可自我演化的固定 prompt）。
    prompts::ensure_evolution_prompt_pack_v1(&state.db, &state.config.default_workspace_id).await?;
    // M4 W4 Task 5.3：seed 完成后 fetch_add 一次 prompt_pack_version，让启动后第一个
    // run 的 LRU cache key 与种入后的 prompt 内容对齐。
    state
        .prompt_pack_version
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    // S-18 / Task 18：种入示例评测场景，缺失时用 fallback 满足 spec 要求。
    let _ = ensure_example_evaluation_scenario(&state.db, &state.config.default_workspace_id).await;

    let worker_state = state.clone();
    tokio::spawn(async move {
        tasks::run_task_worker(worker_state).await;
    });

    let outbox_state = state.clone();
    tokio::spawn(async move {
        if let Err(err) = run_outbox_dispatcher(outbox_state).await {
            tracing::error!(?err, "outbox dispatcher exited");
        }
    });

    if state.config.strategic_planner_enabled {
        let planner_state = state.clone();
        tokio::spawn(async move {
            wechatagent::planner::run_strategic_planner(planner_state).await;
        });
    }

    // agent-self-evolution M4 W1：演化器 worker。
    // 关停态默认（`EVOLUTION_ENABLED=false`）；run_evolutionary_worker 内部
    // 会立即 return，不消耗任何资源。打开后周期跑 cohort 选择 + 候选生成
    // + shadow eval（W2/W3 落地后）。
    {
        let evolution_state = state.clone();
        tokio::spawn(async move {
            wechatagent::evolution::run_evolutionary_worker(evolution_state).await;
        });
    }

    // knowledge-digest-workstation Phase 1：日报合成 worker。
    // 关停态默认（`KNOWLEDGE_DIGEST_ENABLED=false`）；worker_loop 内部立即
    // return。打开后每天 `KNOWLEDGE_DIGEST_RUN_HOUR` 整点扫 4 数据源 + 合成
    // 卡片（Phase 2 落地）。
    {
        let digest_state = state.clone();
        tokio::spawn(async move {
            wechatagent::knowledge_digest::worker_loop(digest_state).await;
        });
    }

    // knowledge-digest-workstation Phase 4：chat 长任务 worker。
    // 默认间隔 30s（`KNOWLEDGE_TASK_WORKER_INTERVAL_SECONDS=0` 关停）。
    // 取 pending knowledge_chat_tasks 按 sessionId 串行执行 plannedSteps，
    // 进度回写 knowledge_chat_turns 并经 ChatProgressBus 推 SSE。
    {
        let task_state = state.clone();
        tokio::spawn(async move {
            wechatagent::knowledge_task::worker_loop(task_state).await;
        });
    }

    let static_files = ServeDir::new("frontend/dist")
        .not_found_service(ServeFile::new("frontend/dist/index.html"));
    let app = Router::new()
        .nest("/api", api_router(state.clone()))
        .route(
            "/webhooks/wechat",
            axum::routing::post(webhooks::wechat_webhook),
        )
        .with_state(state)
        .fallback_service(static_files)
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        );

    let addr: SocketAddr = format!("{}:{}", config.app_host, config.app_port).parse()?;
    let listener = TcpListener::bind(addr).await?;
    tracing::info!("wechatagent listening on http://{}", addr);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

/// S-18 / Task 18：启动时确保至少存在一个示例评测场景，便于运营人员上手。
async fn ensure_example_evaluation_scenario(
    db: &wechatagent::db::Database,
    workspace_id: &str,
) -> anyhow::Result<()> {
    use mongodb::bson::{doc, DateTime};
    let exists = db
        .evaluation_scenarios()
        .find_one(
            doc! { "workspace_id": workspace_id, "scenario_id": "example_high_intent_user" },
            None,
        )
        .await?;
    if exists.is_some() {
        return Ok(());
    }
    let now = DateTime::now();
    let scenario = wechatagent::models::EvaluationScenario {
        id: None,
        workspace_id: workspace_id.to_string(),
        scenario_id: "example_high_intent_user".to_string(),
        title: "高意向用户主动询问产品能力".to_string(),
        description: "用户主动表达需求并询问能否落地，期望模型给出有信任、有具体性、不施压的回应。"
            .to_string(),
        account_id: None,
        contact_seed: doc! {
            "operationState": "need_discovery",
            "intentLevel": "高意向"
        },
        inbound_messages: vec![
            "我们销售经常跟丢客户，AI 能不能帮忙跟进？".to_string(),
            "如果客户三天没回，你们会一直追吗？".to_string(),
        ],
        ground_truth: doc! {
            "trust": 7,
            "conversionReadiness": 6,
            "emotionalValue": 7,
            "nextBestActionScore": 7
        },
        tags: vec!["example".to_string(), "high_intent".to_string()],
        status: "active".to_string(),
        created_at: now,
        updated_at: now,
    };
    db.evaluation_scenarios().insert_one(scenario, None).await?;
    Ok(())
}
