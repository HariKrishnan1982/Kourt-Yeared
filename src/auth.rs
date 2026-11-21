use std::sync::OnceLock;

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    extract::{FromRequestParts, State},
    http::{header, request::Parts, HeaderMap, StatusCode},
    response::{AppendHeaders, IntoResponse},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{
    error::{ApiError, ApiResult},
    model::{now_ms, random_token},
    state::AppState,
};

const COOKIE: &str = "kourt_session";
const SESSION_MS: i64 = 30 * 24 * 60 * 60 * 1000;

pub struct AuthUser {
    pub id: i64,
    pub username: String,
}

fn hash_token(t: &str) -> String {
    Sha256::digest(t.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn session_token(headers: &HeaderMap) -> Option<String> {
    for v in headers.get_all(header::COOKIE) {
        if let Ok(s) = v.to_str() {
            for part in s.split(';') {
                if let Some(t) = part.trim().strip_prefix("kourt_session=") {
                    if !t.is_empty() {
                        return Some(t.to_string());
                    }
                }
            }
        }
    }
    None
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, ApiError> {
        let tok = session_token(&parts.headers).ok_or_else(ApiError::unauthorized)?;
        let row: Option<(i64, String)> = sqlx::query_as(
            "SELECT u.id, u.username FROM sessions s JOIN users u ON u.id = s.user_id \
             WHERE s.token_hash = ? AND s.expires_ms > ?",
        )
        .bind(hash_token(&tok))
        .bind(now_ms())
        .fetch_optional(&st.db)
        .await?;
        row.map(|(id, username)| AuthUser { id, username })
            .ok_or_else(ApiError::unauthorized)
    }
}

#[derive(Deserialize)]
pub struct Creds {
    username: String,
    password: String,
}

fn valid_username(u: &str) -> bool {
    (3..=20).contains(&u.chars().count())
        && u.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

async fn hash_password(pw: String) -> ApiResult<String> {
    tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default().hash_password(pw.as_bytes(), &salt).map(|h| h.to_string())
    })
    .await
    .ok()
    .and_then(|r| r.ok())
    .ok_or_else(|| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Couldn't secure your password. Try again."))
}

async fn verify_password(pw: String, stored: Option<String>) -> bool {
    static DUMMY: OnceLock<String> = OnceLock::new();
    tokio::task::spawn_blocking(move || {
        // Always do one hash verification, so unknown usernames cost the same time.
        let dummy = DUMMY.get_or_init(|| {
            let salt = SaltString::generate(&mut OsRng);
            Argon2::default().hash_password(b"dummy-password", &salt).unwrap().to_string()
        });
        let known = stored.is_some();
        let hash = stored.unwrap_or_else(|| dummy.clone());
        let ok = PasswordHash::new(&hash)
            .map(|p| Argon2::default().verify_password(pw.as_bytes(), &p).is_ok())
            .unwrap_or(false);
        ok && known
    })
    .await
    .unwrap_or(false)
}

fn cookie(st: &AppState, value: &str, max_age_s: i64) -> String {
    let secure = if st.secure_cookies { "; Secure" } else { "" };
    format!("{COOKIE}={value}; HttpOnly; SameSite=Lax; Path=/; Max-Age={max_age_s}{secure}")
}

async fn start_session(st: &AppState, user_id: i64) -> ApiResult<String> {
    let token = random_token(32);
    sqlx::query("INSERT INTO sessions(token_hash, user_id, expires_ms) VALUES(?, ?, ?)")
        .bind(hash_token(&token))
        .bind(user_id)
        .bind(now_ms() + SESSION_MS)
        .execute(&st.db)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE expires_ms < ?")
        .bind(now_ms())
        .execute(&st.db)
        .await?;
    Ok(cookie(st, &token, SESSION_MS / 1000))
}

pub async fn signup(State(st): State<AppState>, Json(c): Json<Creds>) -> ApiResult<impl IntoResponse> {
    let username = c.username.trim().to_string();
    if !valid_username(&username) {
        return Err(ApiError::bad("Usernames are 3 to 20 characters: letters, numbers, dots and underscores."));
    }
    if !(8..=128).contains(&c.password.chars().count()) {
        return Err(ApiError::bad("Passwords need at least 8 characters."));
    }
    if !st.limiter.hit("signup", 60, 60 * 60 * 1000) {
        return Err(ApiError::new(StatusCode::TOO_MANY_REQUESTS, "Too many new accounts right now. Try again later."));
    }
    let hash = hash_password(c.password).await?;
    let res = sqlx::query("INSERT INTO users(username, pw_hash, created_ms) VALUES(?, ?, ?)")
        .bind(&username)
        .bind(hash)
        .bind(now_ms())
        .execute(&st.db)
        .await;
    let id = match res {
        Ok(r) => r.last_insert_rowid(),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(ApiError::conflict("That username is taken. Pick another."))
        }
        Err(e) => return Err(e.into()),
    };
    let set = start_session(&st, id).await?;
    Ok((AppendHeaders([(header::SET_COOKIE, set)]), Json(json!({ "id": id, "username": username }))))
}

pub async fn login(State(st): State<AppState>, Json(c): Json<Creds>) -> ApiResult<impl IntoResponse> {
    let username = c.username.trim().to_string();
    if !st.limiter.hit(&format!("login:{}", username.to_lowercase()), 12, 10 * 60 * 1000) {
        return Err(ApiError::new(StatusCode::TOO_MANY_REQUESTS, "Too many attempts. Wait a few minutes and try again."));
    }
    let row: Option<(i64, String, String)> =
        sqlx::query_as("SELECT id, username, pw_hash FROM users WHERE username = ?")
            .bind(&username)
            .fetch_optional(&st.db)
            .await?;
    let (id, name, stored) = match row {
        Some((id, n, h)) => (Some(id), n, Some(h)),
        None => (None, username.clone(), None),
    };
    if !verify_password(c.password, stored).await {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Wrong username or password."));
    }
    let id = id.ok_or_else(ApiError::unauthorized)?;
    let set = start_session(&st, id).await?;
    Ok((AppendHeaders([(header::SET_COOKIE, set)]), Json(json!({ "id": id, "username": name }))))
}

pub async fn logout(State(st): State<AppState>, headers: HeaderMap) -> ApiResult<impl IntoResponse> {
    if let Some(t) = session_token(&headers) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
            .bind(hash_token(&t))
            .execute(&st.db)
            .await?;
    }
    Ok((AppendHeaders([(header::SET_COOKIE, cookie(&st, "", 0))]), Json(json!({ "ok": true }))))
}

pub async fn me(State(st): State<AppState>, user: AuthUser) -> ApiResult<Json<Value>> {
    let owes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM cases WHERE debtor_id = ? AND sentence IS NOT NULL AND sentence_paid = 0",
    )
    .bind(user.id)
    .fetch_one(&st.db)
    .await?;
    let owed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM cases WHERE creditor_id = ? AND sentence IS NOT NULL AND sentence_paid = 0",
    )
    .bind(user.id)
    .fetch_one(&st.db)
    .await?;
    Ok(Json(json!({ "id": user.id, "username": user.username, "owes": owes, "owed": owed })))
}
