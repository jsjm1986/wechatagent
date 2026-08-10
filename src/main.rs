use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use mongodb::bson::DateTime;
use tokio::net::TcpListener;
use tower_http::{
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use wechatagent::agent::run_outbox_dispatcher;
use wechatagent::{
    config::AppConfig,
    db::{self, Database},
    llm::{LlmClient, LlmFormat, LlmProvider, LlmProviderMeta, LlmRegistry},
    mcp::McpClient,
    prompts,
    routes::{api_router, AppState},
    tasks, webhooks, APP_STARTED_AT,
};

fn main() -> anyhow::Result<()> {
    // Windows 上 main 线程默认栈较小，启动期深调用（migrations / prompt seed /
    // taxonomy cache 预热等）会触发 STATUS_STACK_OVERFLOW。把实际启动逻辑挪到一个
    // 配置了大栈（32MB）的专用线程上跑，绕开 main 线程的栈限制；该线程内自建
    // 多线程 tokio runtime 承载全部 async 工作。
    let child = std::thread::Builder::new()
        .name("wechatagent-main".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(32 * 1024 * 1024)
                .build()?;
            runtime.block_on(async_main())
        })?;
    child
        .join()
        .map_err(|_| anyhow::anyhow!("wechatagent-main thread panicked"))?
}

async fn async_main() -> anyhow::Result<()> {
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
    // P0 鉴权：admin_users 集合空且 env 提供 BOOTSTRAP_ADMIN_USERNAME +
    // BOOTSTRAP_ADMIN_PASSWORD 时创建第一个 admin。env 留着也幂等。
    match wechatagent::auth::session::bootstrap_admin_if_needed(
        &db,
        config.bootstrap_admin_username.as_deref(),
        config.bootstrap_admin_password.as_deref(),
        Some(&config.default_workspace_id),
    )
    .await
    {
        Ok(true) => tracing::info!(
            "bootstrap admin created from env (username={:?})",
            config.bootstrap_admin_username
        ),
        Ok(false) => {}
        Err(e) => tracing::warn!("bootstrap admin failed: {}", e),
    }
    // S1.2 (Phase 0)：active operation_domain_configs 必须配非空 state_machine。
    // 与 check_state_transition 的 fail-closed 路径配对——启动期先拒绝错误配置，
    // runtime defense-in-depth 兜底。
    run_active_domain_state_machine_sanity_check(&db).await?;
    // Phase A / A3 + SR-008：启动期预热 system_taxonomies cache；版本流存在但
    // 没有唯一 current 时 fail closed，不能以空字典继续服务。
    wechatagent::agent::init_global_taxonomy_cache(&db).await?;
    // universal-domain-adaptation 1G-c：同款预热 active DomainProfile 进程级 cache。
    // 查询失败或多 active 是配置完整性错误，启动必须 fail closed。
    wechatagent::agent::init_global_domain_profile_cache(&db).await?;
    // LLM 配置：DB 优先，缺则用 .env 当种子。
    // 启动时若 `llm_provider_configs` 没有 active 记录，写一条来自 .env 的
    // openai 形态默认记录；之后每次启动都按当前 active 记录构造 LlmClient。
    let active_provider = wechatagent::llm::ensure_default_llm_provider(&db, &config).await?;
    let active_providers = load_active_llm_providers(&db).await?;
    let (llm_client, active_meta) = build_runtime_llm(&active_provider, &config)?;
    let registry = Arc::new(LlmRegistry::new(
        config.default_workspace_id.clone(),
        llm_client,
        active_meta,
    ));
    for provider in active_providers {
        if provider.workspace_id == config.default_workspace_id {
            continue;
        }
        let (client, meta) = build_runtime_llm(&provider, &config)?;
        registry.swap(&provider.workspace_id, client, meta).await;
    }
    let llm: Arc<dyn LlmProvider> = registry.clone();
    // Phase E / E2：reviewer 双脑并行——`REVIEWER_DUAL_ENABLED=true` 且第二
    // provider 4 件套 (BASE_URL/API_KEY/MODEL/FORMAT) 齐备时，构建独立 LlmClient
    // 注入 AppState.second_reviewer_llm；review_decision 看到 Some 即并行调用。
    // 缺件视为配置错误：拒绝启动，避免静默退化为单 reviewer。
    let second_reviewer_llm: Option<Arc<dyn LlmProvider>> = if config.reviewer_dual_enabled {
        let base_url = config
            .reviewer_second_provider_base_url
            .as_ref()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "REVIEWER_DUAL_ENABLED=true 但 REVIEWER_SECOND_PROVIDER_BASE_URL 未配置"
                )
            })?;
        let api_key = config
            .reviewer_second_provider_api_key
            .as_ref()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "REVIEWER_DUAL_ENABLED=true 但 REVIEWER_SECOND_PROVIDER_API_KEY 未配置"
                )
            })?;
        let model = config
            .reviewer_second_provider_model
            .as_ref()
            .ok_or_else(|| {
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
        let arc: Arc<dyn LlmProvider> = Arc::new(client);
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
    // P1-7：JWT keys。`JWT_ENABLED=true` 时强制要求 PEM 双密钥；未配置直接 panic
    // 拒起，避免"以为开了实际没开"。`false`（默认）时为 None，仅 cookie 路径。
    let jwt_keys_arc: Option<Arc<wechatagent::auth::jwt::JwtKeys>> = if config.jwt_enabled {
        let keys = wechatagent::auth::jwt::JwtKeys::from_config(&config)?;
        tracing::info!(
            ttl_minutes = keys.ttl_minutes,
            "jwt enabled — Authorization: Bearer route is open"
        );
        Some(Arc::new(keys))
    } else {
        None
    };
    let state = AppState {
        db,
        mcp: McpClient::new(config.mcp_base_url.clone(), config.mcp_api_key.clone())?,
        llm,
        llm_registry: Some(registry.clone()),
        llm_concurrency: Arc::new(wechatagent::llm_concurrency::LlmConcurrencyGovernor::new(
            config.llm_max_concurrency,
            config.llm_foreground_reserved,
        )),
        config: config.clone(),
        prompt_pack_version: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        chat_progress_bus: std::sync::Arc::new(wechatagent::knowledge_task::ChatProgressBus::new()),
        second_reviewer_llm,
        chunk_locks: std::sync::Arc::new(dashmap::DashMap::new()),
        chunk_event_bus: tokio::sync::broadcast::channel(
            wechatagent::routes::chunk_locks::CHUNK_EVENT_CHANNEL_CAPACITY,
        )
        .0,
        jwt_keys: jwt_keys_arc,
        auth_rate_limiter: Arc::new(wechatagent::auth::rate_limit::AuthRateLimiter::new(
            config.auth_rate_limit_window_seconds,
            config.auth_rate_limit_client_capacity,
            config.auth_rate_limit_target_capacity,
            config.auth_rate_limit_global_capacity,
        )),
        completeness_cache: std::sync::Arc::new(dashmap::DashMap::new()),
    };
    // 启动期 LRU 缓存尚未建立，无需 bump；显式忽略返回的"是否写入"bool，保留 ? 传播错误。
    let _ = prompts::ensure_prompt_pack_v2(
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

    // P1-7：所有长寿 worker 经 supervisor 包裹，panic 后退避重启 + 写
    // agent_events.kind=background_worker_panic。
    use wechatagent::supervisor::spawn_supervised;

    spawn_supervised(state.clone(), "task_worker", |s| async move {
        tasks::run_task_worker(s).await;
    });

    spawn_supervised(state.clone(), "inbound_reply_worker", |s| async move {
        tasks::run_inbound_reply_worker(s).await;
    });

    // 异步知识导入 worker。常开（异步导入的必需件，非可选部署行为，故不 gate）；
    // inert 时只是空轮询。认领 import_jobs pending → 跑分块抽取 → 回写进度/终态。
    spawn_supervised(state.clone(), "import_worker", |s| async move {
        wechatagent::import_worker::run_import_worker(s).await;
    });

    // Management side effects are never replayed after an uncertain crash. This sweeper
    // converges expired execution leases to execution_unknown.
    spawn_supervised(
        state.clone(),
        "management_command_sweeper",
        |s| async move {
            wechatagent::management_worker::management_command_sweeper_loop(s).await;
        },
    );

    spawn_supervised(state.clone(), "outbox_dispatcher", |s| async move {
        if let Err(err) = run_outbox_dispatcher(s).await {
            tracing::error!(?err, "outbox dispatcher exited");
        }
    });

    // Profile/memory projections are durable but intentionally off the customer-delivery path.
    spawn_supervised(state.clone(), "post_decision_worker", |s| async move {
        wechatagent::agent::run_post_decision_worker(s).await;
    });

    // HC-006 / SR-017: reconcile local content-addressed media immediately at
    // startup and hourly thereafter. This repairs DB-commit-before-rename
    // crashes, removes orphans, and fail-closes rows whose object is missing.
    spawn_supervised(state.clone(), "media_storage_reconciler", |s| async move {
        wechatagent::media_storage::reconciler_loop(
            s.db.clone(),
            std::path::PathBuf::from(&s.config.media_storage_dir),
        )
        .await;
    });

    if state.config.strategic_planner_enabled {
        spawn_supervised(state.clone(), "strategic_planner", |s| async move {
            wechatagent::planner::run_strategic_planner(s).await;
        });
    }

    // Phase D / D3：冷联系人重激活 worker。默认关停（COLD_CONTACT_WORKER_ENABLED=false）；
    // worker 内部检查 flag 后立即 return。打开后周期挑 last_outbound_at 旧的 managed
    // contact，写 follow_up 任务，下游仍走 gateway / outbox。
    spawn_supervised(state.clone(), "cold_contact_worker", |s| async move {
        wechatagent::cold_contact_worker::run_cold_contact_worker(s).await;
    });

    // 自学习采集管道（第一阶段）/ S6：沉默删失探测 worker。
    // 默认关停（`SILENCE_SIGNAL_WORKER_ENABLED=false`）；run_silence_signal_worker
    // 内部会立即 return。打开后周期把"最后一条 outbound 至今无回"的 contact
    // 落成 censored=true 的删失信号——只采集，不发任何消息。
    spawn_supervised(state.clone(), "silence_signal_worker", |s| async move {
        wechatagent::silence_signal_worker::run_silence_signal_worker(s).await;
    });

    // agent-self-evolution M4 W1：演化器 worker。
    // 默认关闭（`EVOLUTION_ENABLED=false`）；只有 env=true 后，UI 总开关（Mongo
    // runtime flag）才有权进一步放行。打开后周期跑 cohort 选择 + 候选生成 + shadow eval。
    spawn_supervised(state.clone(), "evolutionary_worker", |s| async move {
        wechatagent::evolution::run_evolutionary_worker(s).await;
    });

    // knowledge-digest-workstation Phase 1：日报合成 worker。
    // 关停态默认（`KNOWLEDGE_DIGEST_ENABLED=false`）；worker_loop 内部立即
    // return。打开后每天 `KNOWLEDGE_DIGEST_RUN_HOUR` 整点扫 4 数据源 + 合成
    // 卡片（Phase 2 落地）。
    spawn_supervised(state.clone(), "knowledge_digest_worker", |s| async move {
        wechatagent::knowledge_digest::worker_loop(s).await;
    });

    // knowledge-digest-workstation Phase 4：chat 长任务 worker。
    // 默认间隔 30s（`KNOWLEDGE_TASK_WORKER_INTERVAL_SECONDS=0` 关停）。
    // 取 pending knowledge_chat_tasks 按 sessionId 串行执行 plannedSteps，
    // 进度回写 knowledge_chat_turns 并经 ChatProgressBus 推 SSE。
    spawn_supervised(state.clone(), "knowledge_task_worker", |s| async move {
        wechatagent::knowledge_task::worker_loop(s).await;
    });

    // knowledge-wiki Phase E：catalog rebuild worker。
    // 默认 3s 一轮（`CATALOG_REBUILD_WORKER_INTERVAL_SECONDS=0` 关停）。消费
    // `catalog_rebuild_jobs` 队列，把每条 job 对应 document 的所有 active chunk
    // 渲染为 markdown 落到 `documents.catalog_summary_persisted` + 自增
    // `catalog_version`，把 catalog 拉取从 O(N 字段) 降到 O(1)。
    {
        let interval = state.config.catalog_rebuild_worker_interval_seconds;
        spawn_supervised(state.clone(), "catalog_rebuild_worker", move |s| {
            let interval = interval;
            async move {
                wechatagent::knowledge_wiki::catalog_rebuild::catalog_rebuild_worker_loop(
                    s.db.clone(),
                    interval,
                )
                .await;
            }
        });
    }

    // knowledge-wiki Phase F：feedback worker。
    // 默认 600s 一轮（`KNOWLEDGE_FEEDBACK_INTERVAL_SECONDS=0` 关停）。逐 workspace
    // 跑 30d usage_stats 滑窗回写 + dynamic_confidence 计算 + structural lint +
    // stage 1 sweep。stage 2（LLM）暂留接口,本轮不进入热路径。
    {
        let interval = state.config.knowledge_feedback_interval_seconds;
        spawn_supervised(state.clone(), "knowledge_feedback_worker", move |s| {
            let interval = interval;
            async move {
                wechatagent::knowledge_wiki::feedback_worker::feedback_worker_loop(s, interval)
                    .await;
            }
        });
    }

    // Phase G P1-6：auto-ingest worker。默认关停（`INGEST_WORKER_ENABLED=false`）。
    // 每轮跨 workspace 扫 status="active" 的 IngestSource → 条件 GET → feed-rs/scraper
    // 解析 → ingest_chunked_text 落 chunks（draft + needs_review，红线"AI 永不自动 verify"）。
    if state.config.ingest_worker_enabled {
        let interval = state.config.ingest_worker_interval_seconds;
        spawn_supervised(state.clone(), "ingest_worker", move |s| {
            let interval = interval;
            async move {
                wechatagent::knowledge_wiki::ingest_worker::ingest_worker_loop(s, interval).await;
            }
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
        )
        // gzip 压缩所有响应（对带 Accept-Encoding: gzip 的客户端生效）。roster 端点
        // 4832 好友 ~1.5MB JSON 高度可压（头像 URL / 字段名重复），实测压到 ~1/6；
        // 静态前端 JS/CSS 也一并受益。挂最外层：响应流出时最后一步压缩。
        .layer(CompressionLayer::new());

    let addr: SocketAddr = format!("{}:{}", config.app_host, config.app_port).parse()?;
    let listener = TcpListener::bind(addr).await?;
    tracing::info!("wechatagent listening on http://{}", addr);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
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

fn build_runtime_llm(
    provider: &wechatagent::models::LlmProviderConfig,
    config: &AppConfig,
) -> anyhow::Result<(LlmClient, LlmProviderMeta)> {
    let format = LlmFormat::parse(&provider.format)?;
    let client = LlmClient::with_format(
        provider.base_url.clone(),
        provider.api_key.clone(),
        provider.model.clone(),
        format,
        provider
            .timeout_seconds
            .unwrap_or(config.llm_timeout_seconds),
        provider.max_retries.unwrap_or(config.llm_max_retries),
        provider.retry_base_ms.unwrap_or(config.llm_retry_base_ms),
    )?;
    let meta = LlmProviderMeta {
        provider_id: provider.provider_id.clone(),
        format,
        model: provider.model.clone(),
        base_url: provider.base_url.clone(),
        revision_ms: provider.updated_at.timestamp_millis(),
        runtime_fingerprint: wechatagent::llm::llm_provider_runtime_fingerprint(provider)?,
    };
    Ok((client, meta))
}

async fn load_active_llm_providers(
    db: &wechatagent::db::Database,
) -> anyhow::Result<Vec<wechatagent::models::LlmProviderConfig>> {
    use futures::TryStreamExt;
    use mongodb::bson::doc;
    use std::collections::HashSet;

    let mut cursor = db
        .llm_provider_configs()
        .find(doc! { "isActive": true }, None)
        .await?;
    let mut seen_workspaces = HashSet::new();
    let mut providers = Vec::new();
    while let Some(provider) = cursor.try_next().await? {
        if !seen_workspaces.insert(provider.workspace_id.clone()) {
            anyhow::bail!(
                "multiple active LLM providers found for workspace {}",
                provider.workspace_id
            );
        }
        providers.push(provider);
    }
    Ok(providers)
}

/// S1.2 (Phase 0)：扫描所有 `status="active"` 的 `operation_domain_configs`，
/// 拒绝 `state_machine.states` 缺失或为空的记录。
///
/// 目的：与 [`wechatagent::agent::check_state_transition`] 的 fail-closed 路径配对。
/// runtime 已经在 `states.is_empty() && cfg.is_some()` 分支里返回拦截 reason；
/// 但更早的兜底是启动期就拒绝这种配置，避免 100% 的决策被 guards 拦下。
///
/// 不影响 simulation / 老路径：`check_state_transition(None, ...)` 仍然 fail-open。
async fn run_active_domain_state_machine_sanity_check(
    db: &wechatagent::db::Database,
) -> anyhow::Result<()> {
    use futures::TryStreamExt;
    use mongodb::bson::doc;

    let mut cursor = db
        .operation_domain_configs()
        .find(doc! { "status": "active" }, None)
        .await?;
    let mut offenders: Vec<String> = Vec::new();
    while let Some(cfg) = cursor.try_next().await? {
        let states = cfg
            .state_machine
            .get_array("states")
            .ok()
            .map(|arr| arr.len())
            .unwrap_or(0);
        if states == 0 {
            offenders.push(format!(
                "workspace={} domain={} version={}",
                cfg.workspace_id, cfg.domain, cfg.version
            ));
        }
    }
    if !offenders.is_empty() {
        anyhow::bail!(
            "active operation_domain_configs 缺少非空 state_machine.states：{}",
            offenders.join("; ")
        );
    }
    Ok(())
}
