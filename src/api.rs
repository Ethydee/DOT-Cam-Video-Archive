use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde_json::json;
use sqlx::{Row, SqlitePool};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

use crate::auth::{AdminUser, AuthUser};
use crate::dvr;
use crate::models::*;
use crate::recorder::RecorderState;
use crate::rtmp::IngestState;

// ── Shared application state ────────────────────────────────────────────────
#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub recorder: Arc<Mutex<RecorderState>>,
    pub ingest: Arc<Mutex<IngestState>>,
}

pub fn api_router(state: AppState) -> Router {
    Router::new()
        // Public
        .route("/api/cameras", get(list_cameras))
        .route("/api/cameras/:id", get(get_camera))
        .route("/api/cameras/:id/live.m3u8", get(live_playlist))
        .route("/api/cameras/:id/full.m3u8", get(full_playlist))
        .route("/api/cameras/:id/dvr.m3u8", get(dvr_playlist))
        .route("/api/cameras/:id/dates", get(archive_dates))
        .route("/api/cameras/:id/range", get(segment_range))
        .route("/api/segments/:cam_id/:filename", get(serve_segment))
        .route("/api/auth/login", post(login))
        .route("/api/auth/me", get(get_me))
        // Admin: cameras
        .route("/api/admin/cameras", post(create_camera))
        .route("/api/admin/cameras/:id", put(update_camera))
        .route("/api/admin/cameras/:id", delete(delete_camera))
        // Admin: users
        .route("/api/admin/users", get(list_users))
        .route("/api/admin/users", post(create_user))
        .route("/api/admin/users/:id", put(update_user))
        .route("/api/admin/users/:id", delete(delete_user))
        // Admin: settings
        .route("/api/admin/settings", get(get_settings))
        .route("/api/admin/settings", put(update_settings))
        .with_state(state)
}

// ═══════════════════════════════════════════════════════════════════════════
// AUTH
// ═══════════════════════════════════════════════════════════════════════════
async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> impl IntoResponse {
    let row = sqlx::query("SELECT password_hash, role FROM users WHERE username = ?")
        .bind(&body.username)
        .fetch_optional(&state.db)
        .await;

    match row {
        Ok(Some(row)) => {
            let hash: String = row.get("password_hash");
            let role: String = row.get("role");

            match bcrypt::verify(&body.password, &hash) {
                Ok(true) => {
                    match crate::auth::create_token(&body.username, &role) {
                        Ok(token) => (
                            StatusCode::OK,
                            Json(json!({
                                "token": token,
                                "username": body.username,
                                "role": role
                            })),
                        ).into_response(),
                        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Token generation failed"}))).into_response(),
                    }
                }
                _ => (StatusCode::UNAUTHORIZED, Json(json!({"error": "Invalid credentials"}))).into_response(),
            }
        }
        _ => (StatusCode::UNAUTHORIZED, Json(json!({"error": "Invalid credentials"}))).into_response(),
    }
}

async fn get_me(auth: AuthUser) -> impl IntoResponse {
    Json(json!({
        "username": auth.0.sub,
        "role": auth.0.role
    }))
}

// ═══════════════════════════════════════════════════════════════════════════
// CAMERAS (Public read)
// ═══════════════════════════════════════════════════════════════════════════
async fn list_cameras(State(state): State<AppState>) -> impl IntoResponse {
    let rows = sqlx::query(
        "SELECT id, name, location, source_type, stream_url, stream_key, rewind_hours, recording, created_at FROM cameras ORDER BY name"
    )
    .fetch_all(&state.db)
    .await;

    match rows {
        Ok(rows) => {
            let cameras: Vec<serde_json::Value> = rows
                .iter()
                .map(|r| {
                    json!({
                        "id": r.get::<i64, _>("id"),
                        "name": r.get::<String, _>("name"),
                        "location": r.get::<Option<String>, _>("location"),
                        "source_type": r.get::<String, _>("source_type"),
                        "stream_url": r.get::<Option<String>, _>("stream_url"),
                        "stream_key": r.get::<Option<String>, _>("stream_key"),
                        "rewind_hours": r.get::<i64, _>("rewind_hours"),
                        "recording": r.get::<bool, _>("recording"),
                        "created_at": r.get::<String, _>("created_at"),
                    })
                })
                .collect();
            Json(json!(cameras)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        ).into_response(),
    }
}

async fn get_camera(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let row = sqlx::query(
        "SELECT id, name, location, source_type, stream_url, stream_key, rewind_hours, recording, created_at FROM cameras WHERE id = ?"
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await;

    match row {
        Ok(Some(r)) => Json(json!({
            "id": r.get::<i64, _>("id"),
            "name": r.get::<String, _>("name"),
            "location": r.get::<Option<String>, _>("location"),
            "source_type": r.get::<String, _>("source_type"),
            "stream_url": r.get::<Option<String>, _>("stream_url"),
            "stream_key": r.get::<Option<String>, _>("stream_key"),
            "rewind_hours": r.get::<i64, _>("rewind_hours"),
            "recording": r.get::<bool, _>("recording"),
            "created_at": r.get::<String, _>("created_at"),
        })).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({"error": "Camera not found"}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// DVR / Streaming (Public)
// ═══════════════════════════════════════════════════════════════════════════
async fn live_playlist(
    Path(id): Path<i64>,
) -> impl IntoResponse {
    // Serve ffmpeg's own live.m3u8 — it has correct sequencing, timing,
    // and segment references. We just need to rewrite the segment paths
    // so they route through our /api/segments/ endpoint.
    let playlist_path = format!("DATA/segments/{}/live.m3u8", id);

    match tokio::fs::read_to_string(&playlist_path).await {
        Ok(content) => {
            // ffmpeg writes segment filenames as relative paths (e.g. "seg_20260428_170000.ts")
            // Rewrite them to absolute API paths
            let rewritten = content
                .lines()
                .map(|line| {
                    if line.ends_with(".ts") && !line.starts_with('#') {
                        // Extract just the filename (handles both relative and absolute)
                        let filename = line.rsplit('/').next().unwrap_or(line);
                        format!("/api/segments/{}/{}", id, filename)
                    } else {
                        line.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");

            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/vnd.apple.mpegurl"),
                 (header::CACHE_CONTROL, "no-cache")],
                rewritten,
            ).into_response()
        }
        Err(_) => {
            // Fallback to generated playlist if ffmpeg hasn't started yet
            match dvr::generate_playlist(id, None, Some(300), true) {
                Ok(playlist) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/vnd.apple.mpegurl"),
                     (header::CACHE_CONTROL, "no-cache")],
                    playlist,
                ).into_response(),
                Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
            }
        }
    }
}

#[derive(serde::Deserialize)]
pub struct DvrQuery {
    pub from: Option<i64>,
    pub duration: Option<i64>,
}

async fn dvr_playlist(
    Path(id): Path<i64>,
    Query(q): Query<DvrQuery>,
) -> impl IntoResponse {
    match dvr::generate_playlist(id, q.from, q.duration, false) {
        Ok(playlist) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/vnd.apple.mpegurl"),
             (header::CACHE_CONTROL, "no-cache")],
            playlist,
        ).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
    }
}

async fn full_playlist(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    // Get camera's rewind limit
    let max_hours: i64 = sqlx::query_scalar("SELECT rewind_hours FROM cameras WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None)
        .unwrap_or(24);

    match dvr::generate_full_playlist(id, max_hours) {
        Ok(playlist) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/vnd.apple.mpegurl"),
             (header::CACHE_CONTROL, "no-cache")],
            playlist,
        ).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
    }
}

async fn archive_dates(Path(id): Path<i64>) -> impl IntoResponse {
    let dates = dvr::list_archive_dates(id);
    Json(json!(dates))
}

async fn segment_range(Path(id): Path<i64>) -> impl IntoResponse {
    match dvr::get_segment_range(id) {
        Some((oldest, newest, duration)) => Json(json!({
            "oldest_ts": oldest,
            "newest_ts": newest,
            "duration_seconds": duration,
        })).into_response(),
        None => Json(json!({
            "oldest_ts": null,
            "newest_ts": null,
            "duration_seconds": 0,
        })).into_response(),
    }
}

async fn serve_segment(
    Path((cam_id, filename)): Path<(i64, String)>,
) -> impl IntoResponse {
    let path = PathBuf::from(format!("DATA/segments/{}/{}", cam_id, filename));

    if !path.exists() {
        return (StatusCode::NOT_FOUND, "Segment not found").into_response();
    }

    let content_type = if filename.ends_with(".jpg") {
        "image/jpeg"
    } else {
        "video/mp2t"
    };

    match tokio::fs::read(&path).await {
        Ok(data) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, content_type),
             (header::CACHE_CONTROL, "public, max-age=10")],
            Body::from(data),
        ).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read segment").into_response(),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ADMIN: Camera CRUD
// ═══════════════════════════════════════════════════════════════════════════
async fn create_camera(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(body): Json<CreateCamera>,
) -> impl IntoResponse {
    // Generate stream key if source_type is "stream_key"
    let stream_key = if body.source_type == "stream_key" {
        Some(uuid::Uuid::new_v4().to_string().replace('-', ""))
    } else {
        None
    };

    let rewind = body.rewind_hours.unwrap_or(24);

    let result = sqlx::query(
        "INSERT INTO cameras (name, location, source_type, stream_url, stream_key, rewind_hours) VALUES (?, ?, ?, ?, ?, ?)"
    )
    .bind(&body.name)
    .bind(&body.location)
    .bind(&body.source_type)
    .bind(&body.stream_url)
    .bind(&stream_key)
    .bind(rewind)
    .execute(&state.db)
    .await;

    match result {
        Ok(r) => {
            let cam_id = r.last_insert_rowid();

            // If stream_key type, start RTMP listener
            if let Some(key) = &stream_key {
                let port = get_rtmp_port(&state.db).await;
                crate::rtmp::spawn_rtmp_listener(
                    key.clone(),
                    port,
                    Arc::clone(&state.ingest),
                );
            }

            (StatusCode::CREATED, Json(json!({
                "id": cam_id,
                "name": body.name,
                "source_type": body.source_type,
                "stream_key": stream_key,
                "rewind_hours": rewind,
            }))).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn update_camera(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateCamera>,
) -> impl IntoResponse {
    // Build dynamic update
    let mut sets: Vec<String> = Vec::new();

    if let Some(name) = &body.name {
        let escaped = name.replace('\'', "''");
        sets.push(format!("name = '{}'", escaped));
    }
    if let Some(loc) = &body.location {
        let escaped = loc.replace('\'', "''");
        sets.push(format!("location = '{}'", escaped));
    }
    if let Some(st) = &body.source_type {
        let escaped = st.replace('\'', "''");
        sets.push(format!("source_type = '{}'", escaped));

        // If changing to stream_key, generate one
        if st == "stream_key" {
            let key = uuid::Uuid::new_v4().to_string().replace('-', "");
            sets.push(format!("stream_key = '{}'", key));

            let port = get_rtmp_port(&state.db).await;
            crate::rtmp::spawn_rtmp_listener(key, port, Arc::clone(&state.ingest));
        }
    }
    if let Some(url) = &body.stream_url {
        let escaped = url.replace('\'', "''");
        sets.push(format!("stream_url = '{}'", escaped));
    }
    if let Some(rh) = body.rewind_hours {
        sets.push(format!("rewind_hours = {}", rh));
    }
    if let Some(rec) = body.recording {
        sets.push(format!("recording = {}", if rec { 1 } else { 0 }));
    }

    if sets.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": "No fields to update"}))).into_response();
    }

    let sql = format!("UPDATE cameras SET {} WHERE id = {}", sets.join(", "), id);
    match sqlx::query(&sql).execute(&state.db).await {
        Ok(_) => (StatusCode::OK, Json(json!({"status": "updated"}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn delete_camera(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    // Stop RTMP if applicable
    let row = sqlx::query("SELECT stream_key FROM cameras WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await;

    if let Ok(Some(row)) = row {
        if let Some(key) = row.get::<Option<String>, _>("stream_key") {
            crate::rtmp::stop_rtmp_listener(&key, &state.ingest).await;
        }
    }

    // Remove from recorder state
    state.recorder.lock().await.remove(&id);

    // Delete segments
    let seg_dir = format!("DATA/segments/{}", id);
    std::fs::remove_dir_all(&seg_dir).ok();

    match sqlx::query("DELETE FROM cameras WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
    {
        Ok(_) => (StatusCode::OK, Json(json!({"status": "deleted"}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ADMIN: User CRUD
// ═══════════════════════════════════════════════════════════════════════════
async fn list_users(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let rows = sqlx::query("SELECT id, username, role, created_at FROM users ORDER BY username")
        .fetch_all(&state.db)
        .await;

    match rows {
        Ok(rows) => {
            let users: Vec<serde_json::Value> = rows
                .iter()
                .map(|r| json!({
                    "id": r.get::<i64, _>("id"),
                    "username": r.get::<String, _>("username"),
                    "role": r.get::<String, _>("role"),
                    "created_at": r.get::<String, _>("created_at"),
                }))
                .collect();
            Json(json!(users)).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn create_user(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(body): Json<CreateUser>,
) -> impl IntoResponse {
    let role = body.role.unwrap_or_else(|| "user".to_string());
    let hash = match bcrypt::hash(&body.password, 10) {
        Ok(h) => h,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Hash failed"}))).into_response(),
    };

    match sqlx::query("INSERT INTO users (username, password_hash, role) VALUES (?, ?, ?)")
        .bind(&body.username)
        .bind(&hash)
        .bind(&role)
        .execute(&state.db)
        .await
    {
        Ok(r) => (StatusCode::CREATED, Json(json!({
            "id": r.last_insert_rowid(),
            "username": body.username,
            "role": role,
        }))).into_response(),
        Err(e) => (StatusCode::CONFLICT, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn update_user(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateUser>,
) -> impl IntoResponse {
    let mut sets: Vec<String> = Vec::new();

    if let Some(password) = &body.password {
        if let Ok(hash) = bcrypt::hash(password, 10) {
            let escaped = hash.replace('\'', "''");
            sets.push(format!("password_hash = '{}'", escaped));
        }
    }
    if let Some(role) = &body.role {
        let escaped = role.replace('\'', "''");
        sets.push(format!("role = '{}'", escaped));
    }

    if sets.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": "No fields to update"}))).into_response();
    }

    let sql = format!("UPDATE users SET {} WHERE id = {}", sets.join(", "), id);
    match sqlx::query(&sql).execute(&state.db).await {
        Ok(_) => (StatusCode::OK, Json(json!({"status": "updated"}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn delete_user(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    // Prevent deleting the last admin
    let admin_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role = 'admin'")
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);

    let user_role = sqlx::query("SELECT role FROM users WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await;

    if let Ok(Some(row)) = user_role {
        let role: String = row.get("role");
        if role == "admin" && admin_count <= 1 {
            return (StatusCode::BAD_REQUEST, Json(json!({"error": "Cannot delete the last admin"}))).into_response();
        }
    }

    match sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
    {
        Ok(_) => (StatusCode::OK, Json(json!({"status": "deleted"}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ADMIN: Settings
// ═══════════════════════════════════════════════════════════════════════════
async fn get_settings(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let rows = sqlx::query("SELECT key, value FROM settings")
        .fetch_all(&state.db)
        .await;

    match rows {
        Ok(rows) => {
            let mut settings = HashMap::new();
            for r in &rows {
                let k: String = r.get("key");
                let v: String = r.get("value");
                settings.insert(k, v);
            }
            Json(json!(settings)).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response(),
    }
}

async fn update_settings(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(body): Json<HashMap<String, String>>,
) -> impl IntoResponse {
    for (key, value) in &body {
        let _ = sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(value)
            .execute(&state.db)
            .await;
    }
    (StatusCode::OK, Json(json!({"status": "updated"}))).into_response()
}

// ── Helpers ─────────────────────────────────────────────────────────────────
async fn get_rtmp_port(db: &SqlitePool) -> u16 {
    let val: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = 'rtmp_port'")
        .fetch_optional(db)
        .await
        .unwrap_or(None);

    val.and_then(|v| v.parse().ok()).unwrap_or(1935)
}
