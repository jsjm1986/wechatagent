mod agent;
mod config;
mod db;
mod error;
mod llm;
mod mcp;
mod models;
mod routes;
mod tasks;
mod webhooks;

use std::net::SocketAddr;

use axum::Router;
use config::AppConfig;
use db::Database;
use llm::LlmClient;
use mcp::McpClient;
use routes::{api_router, AppState};
use tokio::net::TcpListener;
use tower_http::{
    cors::{Any, CorsLayer},
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

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
    let db = Database::connect(&config.mongodb_uri, &config.mongodb_database).await?;
    let state = AppState {
        db,
        mcp: McpClient::new(config.mcp_base_url.clone(), config.mcp_api_key.clone())?,
        llm: LlmClient::new(
            config.openai_base_url.clone(),
            config.openai_api_key.clone(),
            config.openai_model.clone(),
        )?,
        config: config.clone(),
    };

    let worker_state = state.clone();
    tokio::spawn(async move {
        tasks::run_task_worker(worker_state).await;
    });

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
