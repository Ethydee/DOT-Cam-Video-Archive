mod api;
mod auth;
mod db;
mod dvr;
mod models;
mod recorder;
mod rtmp;

use std::{collections::HashMap, collections::HashSet, sync::Arc};
use tokio::sync::Mutex;
use tower_http::{
    cors::{Any, CorsLayer},
    services::ServeDir,
};
use tracing::info;

use crate::api::AppState;

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "traffic_dvr=info,tower_http=info".into()),
        )
        .init();

    info!("Initializing Traffic DVR...");

    // Initialize database
    let pool = db::init_db().await.expect("Failed to initialize database");

    // Shared state
    let recorder_state: Arc<Mutex<HashSet<i64>>> = Arc::new(Mutex::new(HashSet::new()));
    let ingest_state: Arc<Mutex<HashMap<String, tokio::process::Child>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Start background recorders
    recorder::spawn_recorder_loop(pool.clone(), Arc::clone(&recorder_state));
    recorder::spawn_cleanup_loop(pool.clone());

    // Start RTMP listeners for existing stream_key cameras
    {
        let rows = sqlx::query_as::<_, (Option<String>,)>(
            "SELECT stream_key FROM cameras WHERE source_type = 'stream_key' AND stream_key IS NOT NULL AND recording = 1"
        )
        .fetch_all(&pool)
        .await
        .unwrap_or_default();

        let port: u16 = sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = 'rtmp_port'")
            .fetch_optional(&pool)
            .await
            .unwrap_or(None)
            .and_then(|v| v.parse().ok())
            .unwrap_or(1935);

        for (key,) in rows {
            if let Some(k) = key {
                rtmp::spawn_rtmp_listener(k, port, Arc::clone(&ingest_state));
            }
        }
    }

    let app_state = AppState {
        db: pool,
        recorder: recorder_state,
        ingest: ingest_state,
    };

    // Build router
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = api::api_router(app_state)
        .fallback_service(ServeDir::new("public"))
        .layer(cors);

    let addr = "0.0.0.0:5000";
    info!("Server starting on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind");

    axum::serve(listener, app).await.expect("Server error");
}
