use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde_json::json;

use crate::models::UserClaims;

// In production, load from env. For now, a static secret.
const JWT_SECRET: &[u8] = b"traffic-dvr-secret-change-me-in-production";

/// Create a signed JWT for the given user.
pub fn create_token(username: &str, role: &str) -> Result<String, jsonwebtoken::errors::Error> {
    let expiration = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::hours(72))
        .expect("valid timestamp")
        .timestamp() as usize;

    let claims = UserClaims {
        sub: username.to_string(),
        role: role.to_string(),
        exp: expiration,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(JWT_SECRET),
    )
}

/// Validate a JWT and return the claims.
pub fn validate_token(token: &str) -> Result<UserClaims, jsonwebtoken::errors::Error> {
    let data = decode::<UserClaims>(
        token,
        &DecodingKey::from_secret(JWT_SECRET),
        &Validation::default(),
    )?;
    Ok(data.claims)
}

// ── Axum extractor: AuthUser ────────────────────────────────────────────────
/// Extracts and validates JWT from `Authorization: Bearer <token>` header.
pub struct AuthUser(pub UserClaims);

#[axum::async_trait]
impl<S: Send + Sync> FromRequestParts<S> for AuthUser {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let header_val = auth_header.ok_or_else(|| {
            (StatusCode::UNAUTHORIZED, Json(json!({"error": "Missing authorization header"}))).into_response()
        })?;

        let token = header_val.strip_prefix("Bearer ").ok_or_else(|| {
            (StatusCode::UNAUTHORIZED, Json(json!({"error": "Invalid authorization format"}))).into_response()
        })?;

        let claims = validate_token(token).map_err(|_| {
            (StatusCode::UNAUTHORIZED, Json(json!({"error": "Invalid or expired token"}))).into_response()
        })?;

        Ok(AuthUser(claims))
    }
}

// ── Axum extractor: AdminUser ───────────────────────────────────────────────
/// Like AuthUser but additionally requires role == "admin".
pub struct AdminUser(pub UserClaims);

#[axum::async_trait]
impl<S: Send + Sync> FromRequestParts<S> for AdminUser {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let header_val = auth_header.ok_or_else(|| {
            (StatusCode::UNAUTHORIZED, Json(json!({"error": "Missing authorization header"}))).into_response()
        })?;

        let token = header_val.strip_prefix("Bearer ").ok_or_else(|| {
            (StatusCode::UNAUTHORIZED, Json(json!({"error": "Invalid authorization format"}))).into_response()
        })?;

        let claims = validate_token(token).map_err(|_| {
            (StatusCode::UNAUTHORIZED, Json(json!({"error": "Invalid or expired token"}))).into_response()
        })?;

        if claims.role != "admin" {
            return Err(
                (StatusCode::FORBIDDEN, Json(json!({"error": "Admin access required"}))).into_response()
            );
        }

        Ok(AdminUser(claims))
    }
}
