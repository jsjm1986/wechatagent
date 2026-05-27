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
    llm::{LlmClient, LlmFormat, LlmGenerator, LlmProviderMeta, LlmRegistry},
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
    // Phase A / A3：启动期预热 system_taxonomies 进程级 cache。失败被静默
    // （`init_global_taxonomy_cache` 内部 log warning），下一次 check_value 触发懒加载。
    wechatagent::agent::init_global_taxonomy_cache(&db).await;
    // LLM 配置：DB 优先，缺则用 .env 当种子。
    // 启动时若 `llm_provider_configs` 没有 active 记录，写一条来自 .env 的
    // openai 形态默认记录；之后每次启动都按当前 active 记录构造 LlmClient。
    let active_provider =
        ensure_default_llm_provider(&db, &config).await?;
    let active_format = LlmFormat::parse(&active_provider.format)?;
    let llm_client = LlmClient::with_format(
        active_provider.base_url.clone(),
        active_provider.api_key.clone(),
        active_provider.model.clone(),
        active_format,
        active_provider.timeout_seconds.unwrap_or(config.llm_timeout_seconds),
        active_provider.max_retries.unwrap_or(config.llm_max_retries),
        active_provider.retry_base_ms.unwrap_or(config.llm_retry_base_ms),
    )?;
    let registry = Arc::new(LlmRegistry::new(
        llm_client,
        LlmProviderMeta {
            provider_id: active_provider.provider_id.clone(),
            format: active_format,
            model: active_provider.model.clone(),
            base_url: active_provider.base_url.clone(),
        },
    ));
    let llm: Arc<dyn LlmGenerator> = registry.clone();
    // Phase E / E2：reviewer 双脑并行——`REVIEWER_DUAL_ENABLED=true` 且第二
    // provider 4 件套 (BASE_URL/API_KEY/MODEL/FORMAT) 齐备时，构建独立 LlmClient
    // 注入 AppState.second_reviewer_llm；review_decision 看到 Some 即并行调用。
    // 缺件视为配置错误：拒绝启动，避免静默退化为单 reviewer。
    let second_reviewer_llm: Option<Arc<dyn LlmGenerator>> = if config.reviewer_dual_enabled {
        let base_url = config.reviewer_second_provider_base_url.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "REVIEWER_DUAL_ENABLED=true 但 REVIEWER_SECOND_PROVIDER_BASE_URL 未配置"
            )
        })?;
        let api_key = config.reviewer_second_provider_api_key.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "REVIEWER_DUAL_ENABLED=true 但 REVIEWER_SECOND_PROVIDER_API_KEY 未配置"
            )
        })?;
        let model = config.reviewer_second_provider_model.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "REVIEWER_DUAL_ENABLED=true 但 REVIEWER_SECOND_PROVIDER_MODEL 未配置"
            )
        })?;
        let format = LlmFormat::parse(&config.reviewer_second_provider_format)?;
        let client = LlmClient::with_format(
            base_url.clone(),
            api_key.clone(),
            model.clone(),
            format,
            config.llm_timeout_seconds,
            config.llm_max_retries,
            config.llm_retry_base_ms,
        )?;
        let arc: Arc<dyn LlmGenerator> = Arc::new(client);
        tracing::info!(
            base_url = %base_url,
            model = %model,
            format = format.as_str(),
            "reviewer dual mode enabled — second provider attached"
        );
        Some(arc)
    } else {
        None
    };
    let state = AppState {
        db,
        mcp: McpClient::new(config.mcp_base_url.clone(), config.mcp_api_key.clone())?,
        llm,
        llm_registry: Some(registry.clone()),
        config: config.clone(),
        prompt_pack_version: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        chat_progress_bus: std::sync::Arc::new(
            wechatagent::knowledge_task::ChatProgressBus::new(),
        ),
        second_reviewer_llm,
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

    // Phase D / D3：冷联系人重激活 worker。默认关停（COLD_CONTACT_WORKER_ENABLED=false）；
    // worker 内部检查 flag 后立即 return。打开后周期挑 last_outbound_at 旧的 managed
    // contact，写 follow_up 任务，下游仍走 gateway / outbox。
    {
        let cold_state = state.clone();
        tokio::spawn(async move {
            wechatagent::cold_contact_worker::run_cold_contact_worker(cold_state).await;
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

    // knowledge-wiki Phase E：catalog rebuild worker。
    // 默认 3s 一轮（`CATALOG_REBUILD_WORKER_INTERVAL_SECONDS=0` 关停）。消费
    // `catalog_rebuild_jobs` 队列，把每条 job 对应 document 的所有 active chunk
    // 渲染为 markdown 落到 `documents.catalog_summary_persisted` + 自增
    // `catalog_version`，把 catalog 拉取从 O(N 字段) 降到 O(1)。
    {
        let db = state.db.clone();
        let interval = state.config.catalog_rebuild_worker_interval_seconds;
        tokio::spawn(async move {
            wechatagent::knowledge_wiki::catalog_rebuild::catalog_rebuild_worker_loop(
                db, interval,
            )
            .await;
        });
    }

    // knowledge-wiki Phase F：feedback worker。
    // 默认 600s 一轮（`KNOWLEDGE_FEEDBACK_INTERVAL_SECONDS=0` 关停）。逐 workspace
    // 跑 30d usage_stats 滑窗回写 + dynamic_confidence 计算 + structural lint +
    // stage 1 sweep。stage 2（LLM）暂留接口，本轮不进入热路径。
    {
        let feedback_state = state.clone();
        let interval = state.config.knowledge_feedback_interval_seconds;
        tokio::spawn(async move {
            wechatagent::knowledge_wiki::feedback_worker::feedback_worker_loop(
                feedback_state,
                interval,
            )
            .await;
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

/// 启动时确保 `llm_provider_configs` 至少有一条 active 记录。
///
/// 行为：
/// - 若已有 `is_active=true` 的记录，原样返回。
/// - 否则：若任意一条记录存在，把第一条置为 active 返回；
/// - 否则用 `.env` 的 `OPENAI_*` 写一条 openai 形态默认记录并标 active。
async fn ensure_default_llm_provider(
    db: &wechatagent::db::Database,
    config: &AppConfig,
) -> anyhow::Result<wechatagent::models::LlmProviderConfig> {
    use mongodb::bson::{doc, DateTime};
    if let Some(existing) = db
        .llm_provider_configs()
        .find_one(
            doc! { "workspaceId": &config.default_workspace_id, "isActive": true },
            None,
        )
        .await?
    {
        return Ok(existing);
    }
    if let Some(any) = db
        .llm_provider_configs()
        .find_one(
            doc! { "workspaceId": &config.default_workspace_id },
            None,
        )
        .await?
    {
        db.llm_provider_configs()
            .update_one(
                doc! { "workspaceId": &config.default_workspace_id, "providerId": &any.provider_id },
                doc! { "$set": { "isActive": true, "updatedAt": DateTime::now() } },
                None,
            )
            .await?;
        let mut activated = any;
        activated.is_active = true;
        return Ok(activated);
    }
    let now = DateTime::now();
    let seed = wechatagent::models::LlmProviderConfig {
        id: None,
        workspace_id: config.default_workspace_id.clone(),
        provider_id: "default".to_string(),
        name: "默认 LLM".to_string(),
        format: "openai".to_string(),
        base_url: config.openai_base_url.clone(),
        api_key: config.openai_api_key.clone(),
        model: config.openai_model.clone(),
        is_active: true,
        timeout_seconds: Some(config.llm_timeout_seconds),
        max_retries: Some(config.llm_max_retries),
        retry_base_ms: Some(config.llm_retry_base_ms),
        created_at: now,
        updated_at: now,
    };
    db.llm_provider_configs().insert_one(&seed, None).await?;
    Ok(seed)
}
