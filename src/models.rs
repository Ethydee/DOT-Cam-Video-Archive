use serde::{Deserialize, Serialize};

// ── JWT Claims ──────────────────────────────────────────────────────────────
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UserClaims {
    pub sub: String,       // username
    pub role: String,      // "admin" or "user"
    pub exp: usize,
}

// ── Camera ──────────────────────────────────────────────────────────────────
#[derive(Debug, Serialize, Deserialize, Clone)]
#[allow(dead_code)]
pub struct Camera {
    pub id: i64,
    pub name: String,
    pub location: Option<String>,
    pub source_type: String,      // "hls" or "stream_key"
    pub stream_url: Option<String>,
    pub stream_key: Option<String>,
    pub rewind_hours: i64,        // max rewind in hours
    pub recording: bool,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateCamera {
    pub name: String,
    pub location: Option<String>,
    pub source_type: String,
    pub stream_url: Option<String>,
    pub rewind_hours: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCamera {
    pub name: Option<String>,
    pub location: Option<String>,
    pub source_type: Option<String>,
    pub stream_url: Option<String>,
    pub rewind_hours: Option<i64>,
    pub recording: Option<bool>,
}

// ── User ────────────────────────────────────────────────────────────────────
#[derive(Debug, Serialize, Deserialize, Clone)]
#[allow(dead_code)]
pub struct User {
    pub id: i64,
    pub username: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub role: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateUser {
    pub username: String,
    pub password: String,
    pub role: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUser {
    pub password: Option<String>,
    pub role: Option<String>,
}

// ── Settings ────────────────────────────────────────────────────────────────
#[derive(Debug, Serialize, Deserialize, Clone)]
#[allow(dead_code)]
pub struct SystemSettings {
    pub default_rewind_hours: i64,
    pub rtmp_port: i64,
}
