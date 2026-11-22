//! The clerk's office: a separate admin login and tools for running the site.
//! Admin sessions use their own cookie and tables, so a normal account can never
//! reach them, and by default they only answer requests from this computer.

use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, FromRequestParts, Path, Query, State},
    http::{header, request::Parts, HeaderMap, StatusCode},
    response::{AppendHeaders, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use rand::RngCore;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::SqlitePool;

use crate::{
    api::{line, text},
    auth::{hash_password, hash_token, valid_username, verify_password},
    engine,
    error::{ApiError, ApiResult},
    model::*,
    state::AppState,
};

const ADMIN_COOKIE: &str = "kourt_admin";
const ADMIN_SESSION_MS: i64 = 12 * 60 * 60 * 1000;
const PER_PAGE: i64 = 25;
const REMOVED: &str = "[removed by the court]";

/* ---------- settings (also read by the public side) ---------- */

pub async fn get_setting(db: &SqlitePool, key: &str, default: &str) -> String {
    sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| default.to_string())
}

async fn set_setting(db: &SqlitePool, key: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO settings(key, value) VALUES(?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(key)
        .bind(value)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn signups_open(db: &SqlitePool) -> bool {
    get_setting(db, "signups_open", "1").await == "1"
}

pub async fn max_open_cases(db: &SqlitePool) -> i64 {
    get_setting(db, "max_open_cases", "10").await.parse().unwrap_or(10)
}

/// Public: what every visitor may know about the site's state.
pub async fn site(State(st): State<AppState>) -> Json<Value> {
    Json(json!({
        "announcement": get_setting(&st.db, "announcement", "").await,
        "signups_open": signups_open(&st.db).await,
    }))
}

/* ---------- the local-only gate and the admin session ---------- */

fn forwarded(h: &HeaderMap) -> bool {
    ["x-forwarded-for", "x-forwarded-host", "x-forwarded-proto", "x-real-ip", "forwarded", "via", "cf-connecting-ip"]
        .iter()
        .any(|n| h.contains_key(*n))
}

/// Local means a loopback connection with no proxy headers. A reverse proxy on the
/// same machine would otherwise make every visitor look local.
pub fn is_local(addr: &SocketAddr, h: &HeaderMap) -> bool {
    addr.ip().to_canonical().is_loopback() && !forwarded(h)
}

fn nf() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "Not found.")
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    for v in headers.get_all(header::COOKIE) {
        if let Ok(s) = v.to_str() {
            for part in s.split(';') {
                if let Some(t) = part.trim().strip_prefix(prefix.as_str()) {
                    if !t.is_empty() {
                        return Some(t.to_string());
                    }
                }
            }
        }
    }
    None
}

pub struct AdminUser {
    pub id: i64,
    pub username: String,
}

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, ApiError> {
        if !st.admin_remote {
            let local = parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|c| is_local(&c.0, &parts.headers))
                .unwrap_or(false);
            if !local {
                return Err(nf());
            }
        }
        let tok = cookie_value(&parts.headers, ADMIN_COOKIE).ok_or_else(ApiError::unauthorized)?;
        let row: Option<(i64, String)> = sqlx::query_as(
            "SELECT a.id, a.username FROM admin_sessions s JOIN admins a ON a.id = s.admin_id \
             WHERE s.token_hash = ? AND s.expires_ms > ?",
        )
        .bind(hash_token(&tok))
        .bind(now_ms())
        .fetch_optional(&st.db)
        .await?;
        row.map(|(id, username)| AdminUser { id, username }).ok_or_else(ApiError::unauthorized)
    }
}

fn admin_cookie(st: &AppState, value: &str, max_age_s: i64) -> String {
    let secure = if st.secure_cookies { "; Secure" } else { "" };
    format!("{ADMIN_COOKIE}={value}; HttpOnly; SameSite=Strict; Path=/; Max-Age={max_age_s}{secure}")
}

async fn audit(db: &SqlitePool, admin: &str, action: &str, target: &str, detail: &str) {
    let r = sqlx::query("INSERT INTO admin_log(at_ms, admin, action, target, detail) VALUES(?,?,?,?,?)")
        .bind(now_ms())
        .bind(admin)
        .bind(action)
        .bind(target)
        .bind(detail)
        .execute(db)
        .await;
    if let Err(e) = r {
        eprintln!("could not write the admin log: {e}");
    }
}

/// Creates the admin account on first start (printing a one-time password), or
/// resets it from KOURT_ADMIN_PASSWORD if that is set.
pub async fn bootstrap(db: &SqlitePool) {
    let user = std::env::var("KOURT_ADMIN_USER").unwrap_or_else(|_| "admin".into());
    let user = user.trim().to_string();
    if !(3..=32).contains(&user.chars().count()) || !user.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)) {
        eprintln!("KOURT_ADMIN_USER must be 3 to 32 characters: letters, numbers, dots, dashes, underscores.");
        std::process::exit(1);
    }
    match std::env::var("KOURT_ADMIN_PASSWORD").ok().filter(|p| !p.is_empty()) {
        Some(pw) => {
            if pw.chars().count() < 12 {
                eprintln!("KOURT_ADMIN_PASSWORD must be at least 12 characters.");
                std::process::exit(1);
            }
            let hash = hash_password(pw).await.ok().expect("could not hash the admin password");
            sqlx::query(
                "INSERT INTO admins(username, pw_hash, created_ms) VALUES(?,?,?) \
                 ON CONFLICT(username) DO UPDATE SET pw_hash = excluded.pw_hash",
            )
            .bind(&user)
            .bind(hash)
            .bind(now_ms())
            .execute(db)
            .await
            .expect("could not save the admin account");
            sqlx::query("DELETE FROM admin_sessions WHERE admin_id = (SELECT id FROM admins WHERE username = ?)")
                .bind(&user)
                .execute(db)
                .await
                .ok();
            println!("Admin '{user}' is ready (password taken from KOURT_ADMIN_PASSWORD).");
        }
        None => {
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM admins").fetch_one(db).await.unwrap_or(1);
            if count > 0 {
                return;
            }
            const A: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZabcdefghjkmnpqrstuvwxyz23456789";
            let mut b = [0u8; 20];
            rand::rngs::OsRng.fill_bytes(&mut b);
            let pw: String = b.iter().map(|x| A[*x as usize % A.len()] as char).collect();
            let hash = hash_password(pw.clone()).await.ok().expect("could not hash the admin password");
            sqlx::query("INSERT INTO admins(username, pw_hash, created_ms) VALUES(?,?,?)")
                .bind(&user)
                .bind(hash)
                .bind(now_ms())
                .execute(db)
                .await
                .expect("could not create the admin account");
            println!("\n  =====================================================");
            println!("   ADMIN ACCOUNT CREATED  (shown once, save it now)");
            println!("   Username: {user}");
            println!("   Password: {pw}");
            println!("   Log in at /admin on this computer.");
            println!("  =====================================================\n");
        }
    }
}

/* ---------- pages and login ---------- */

pub async fn page(State(st): State<AppState>, ConnectInfo(addr): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Response {
    if !st.admin_remote && !is_local(&addr, &headers) {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    match crate::Assets::get("admin.html") {
        Some(f) => (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8".to_string()),
                (header::CACHE_CONTROL, "no-store".to_string()),
            ],
            f.data.into_owned(),
        )
            .into_response(),
        None => (StatusCode::INTERNAL_SERVER_ERROR, "admin page missing").into_response(),
    }
}

#[derive(Deserialize)]
pub struct AdminCreds {
    username: String,
    password: String,
}

pub async fn login(
    State(st): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(c): Json<AdminCreds>,
) -> ApiResult<impl IntoResponse> {
    if !st.admin_remote && !is_local(&addr, &headers) {
        return Err(nf());
    }
    const WINDOW: i64 = 15 * MIN;
    let ip_key = format!("adminip:{}", addr.ip());
    let user_key = format!("adminuser:{}", c.username.trim().to_lowercase());
    if st.limiter.blocked(&ip_key, 8, WINDOW) || st.limiter.blocked(&user_key, 5, WINDOW) {
        return Err(ApiError::new(StatusCode::TOO_MANY_REQUESTS, "Too many attempts. Wait 15 minutes and try again."));
    }
    let row: Option<(i64, String, String)> = sqlx::query_as("SELECT id, username, pw_hash FROM admins WHERE username = ?")
        .bind(c.username.trim())
        .fetch_optional(&st.db)
        .await?;
    let (id, name, stored) = match row {
        Some((i, n, h)) => (Some(i), n, Some(h)),
        None => (None, c.username.trim().to_string(), None),
    };
    let ok = verify_password(c.password, stored).await;
    let Some(id) = id.filter(|_| ok) else {
        st.limiter.record(&ip_key, WINDOW);
        st.limiter.record(&user_key, WINDOW);
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Wrong admin username or password."));
    };
    st.limiter.clear(&ip_key);
    st.limiter.clear(&user_key);
    let token = random_token(32);
    sqlx::query("INSERT INTO admin_sessions(token_hash, admin_id, expires_ms) VALUES(?,?,?)")
        .bind(hash_token(&token))
        .bind(id)
        .bind(now_ms() + ADMIN_SESSION_MS)
        .execute(&st.db)
        .await?;
    sqlx::query("DELETE FROM admin_sessions WHERE expires_ms < ?").bind(now_ms()).execute(&st.db).await?;
    audit(&st.db, &name, "login", "admin", "").await;
    Ok((
        AppendHeaders([(header::SET_COOKIE, admin_cookie(&st, &token, ADMIN_SESSION_MS / 1000))]),
        Json(json!({ "username": name })),
    ))
}

pub async fn logout(State(st): State<AppState>, headers: HeaderMap) -> ApiResult<impl IntoResponse> {
    if let Some(t) = cookie_value(&headers, ADMIN_COOKIE) {
        sqlx::query("DELETE FROM admin_sessions WHERE token_hash = ?").bind(hash_token(&t)).execute(&st.db).await?;
    }
    Ok((AppendHeaders([(header::SET_COOKIE, admin_cookie(&st, "", 0))]), Json(json!({ "ok": true }))))
}

pub async fn me(a: AdminUser) -> Json<Value> {
    Json(json!({ "username": a.username }))
}

#[derive(Deserialize)]
pub struct PwChange {
    current: String,
    new: String,
}

pub async fn change_password(
    State(st): State<AppState>,
    a: AdminUser,
    headers: HeaderMap,
    Json(b): Json<PwChange>,
) -> ApiResult<Json<Value>> {
    let stored: String = sqlx::query_scalar("SELECT pw_hash FROM admins WHERE id = ?").bind(a.id).fetch_one(&st.db).await?;
    if !verify_password(b.current, Some(stored)).await {
        return Err(ApiError::bad("Your current password is wrong."));
    }
    if !(12..=128).contains(&b.new.chars().count()) {
        return Err(ApiError::bad("Use at least 12 characters for the admin password."));
    }
    let hash = hash_password(b.new).await?;
    sqlx::query("UPDATE admins SET pw_hash = ? WHERE id = ?").bind(hash).bind(a.id).execute(&st.db).await?;
    let keep = cookie_value(&headers, ADMIN_COOKIE).map(|t| hash_token(&t)).unwrap_or_default();
    sqlx::query("DELETE FROM admin_sessions WHERE admin_id = ? AND token_hash != ?").bind(a.id).bind(keep).execute(&st.db).await?;
    audit(&st.db, &a.username, "change_admin_password", "admin", "other admin sessions ended").await;
    Ok(Json(json!({ "ok": true })))
}

/* ---------- overview ---------- */

async fn n0(db: &SqlitePool, sql: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(sql).fetch_one(db).await
}

async fn n1(db: &SqlitePool, sql: &str, a: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(sql).bind(a).fetch_one(db).await
}

pub async fn stats(State(st): State<AppState>, _a: AdminUser) -> ApiResult<Json<Value>> {
    let db = &st.db;
    let now = now_ms();
    let day = 24 * 60 * MIN;
    let phases: Vec<(String, i64)> = sqlx::query_as("SELECT phase, COUNT(*) FROM cases GROUP BY phase").fetch_all(db).await?;
    let outcomes: Vec<(String, i64)> =
        sqlx::query_as("SELECT outcome, COUNT(*) FROM cases WHERE outcome IS NOT NULL GROUP BY outcome").fetch_all(db).await?;
    let recent_users: Vec<(i64, String, i64)> =
        sqlx::query_as("SELECT id, username, created_ms FROM users ORDER BY id DESC LIMIT 6").fetch_all(db).await?;
    let recent_cases: Vec<(String, String, String, Option<String>, i64, String)> = sqlx::query_as(
        "SELECT c.id, c.title, c.phase, c.outcome, c.created_ms, u.username FROM cases c \
         JOIN users u ON u.id = c.plaintiff_id ORDER BY c.created_ms DESC LIMIT 6",
    )
    .fetch_all(db)
    .await?;
    Ok(Json(json!({
        "users": n0(db, "SELECT COUNT(*) FROM users WHERE pw_hash != '!'").await?,
        "banned": n0(db, "SELECT COUNT(*) FROM users WHERE banned = 1 AND pw_hash != '!'").await?,
        "new_24h": n1(db, "SELECT COUNT(*) FROM users WHERE created_ms > ?", now - day).await?,
        "new_7d": n1(db, "SELECT COUNT(*) FROM users WHERE created_ms > ?", now - 7 * day).await?,
        "cases": n0(db, "SELECT COUNT(*) FROM cases").await?,
        "open_cases": n0(db, "SELECT COUNT(*) FROM cases WHERE phase != 'closed'").await?,
        "unpaid": n0(db, "SELECT COUNT(*) FROM cases WHERE sentence IS NOT NULL AND sentence_paid = 0").await?,
        "messages": n0(db, "SELECT COUNT(*) FROM messages").await?,
        "sessions": n1(db, "SELECT COUNT(*) FROM sessions WHERE expires_ms > ?", now).await?,
        "db_bytes": n0(db, "SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()").await?,
        "uptime_s": (now - st.started_ms) / 1000,
        "signups_open": signups_open(db).await,
        "phases": phases.into_iter().collect::<std::collections::BTreeMap<_, _>>(),
        "outcomes": outcomes.into_iter().collect::<std::collections::BTreeMap<_, _>>(),
        "recent_users": recent_users.into_iter().map(|(id, u, at)| json!({"id": id, "username": u, "created_ms": at})).collect::<Vec<_>>(),
        "recent_cases": recent_cases.into_iter().map(|(id, t, p, o, at, pl)| json!({"id": id, "title": t, "phase": p, "outcome": o, "created_ms": at, "plaintiff": pl})).collect::<Vec<_>>(),
    })))
}

/* ---------- users ---------- */

#[derive(Deserialize)]
pub struct ListQ {
    q: Option<String>,
    phase: Option<String>,
    page: Option<i64>,
}

fn like(q: &str) -> String {
    format!("%{}%", q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"))
}

pub async fn users_list(State(st): State<AppState>, _a: AdminUser, Query(q): Query<ListQ>) -> ApiResult<Json<Value>> {
    let term = q.q.unwrap_or_default().trim().to_string();
    let page = q.page.unwrap_or(1).max(1);
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM users u WHERE (?1 = '' OR u.username LIKE ?2 ESCAPE '\\' OR CAST(u.id AS TEXT) = ?1)",
    )
    .bind(&term)
    .bind(like(&term))
    .fetch_one(&st.db)
    .await?;
    let rows: Vec<(i64, String, i64, Option<i64>, i64, String, i64, i64)> = sqlx::query_as(
        "SELECT u.id, u.username, u.created_ms, u.last_login_ms, u.banned, u.ban_reason, \
         (SELECT COUNT(*) FROM participants p WHERE p.user_id = u.id), \
         (SELECT COUNT(*) FROM cases c WHERE c.debtor_id = u.id AND c.sentence IS NOT NULL AND c.sentence_paid = 0) \
         FROM users u WHERE (?1 = '' OR u.username LIKE ?2 ESCAPE '\\' OR CAST(u.id AS TEXT) = ?1) \
         ORDER BY u.id DESC LIMIT ?3 OFFSET ?4",
    )
    .bind(&term)
    .bind(like(&term))
    .bind(PER_PAGE)
    .bind((page - 1) * PER_PAGE)
    .fetch_all(&st.db)
    .await?;
    let users: Vec<Value> = rows
        .into_iter()
        .map(|(id, name, created, last, banned, reason, cases, owes)| {
            json!({ "id": id, "username": name, "created_ms": created, "last_login_ms": last,
                    "banned": banned == 1, "ban_reason": reason, "cases": cases, "owes": owes })
        })
        .collect();
    Ok(Json(json!({ "users": users, "total": total, "page": page, "per_page": PER_PAGE })))
}

async fn user_name(db: &SqlitePool, id: i64) -> ApiResult<String> {
    sqlx::query_scalar::<_, String>("SELECT username FROM users WHERE id = ?")
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "No such user."))
}

pub async fn user_detail(State(st): State<AppState>, _a: AdminUser, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let db = &st.db;
    let u: Option<(i64, String, i64, Option<i64>, i64, String, String)> = sqlx::query_as(
        "SELECT id, username, created_ms, last_login_ms, banned, ban_reason, admin_note FROM users WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    let (id, username, created, last, banned, reason, note) = u.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "No such user."))?;
    let cases: Vec<(String, String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT c.id, c.title, c.phase, c.outcome, p.role FROM participants p JOIN cases c ON c.id = p.case_id \
         WHERE p.user_id = ? ORDER BY c.created_ms DESC LIMIT 100",
    )
    .bind(id)
    .fetch_all(db)
    .await?;
    let msgs: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT case_id, body, created_ms, removed FROM messages WHERE user_id = ? ORDER BY id DESC LIMIT 10",
    )
    .bind(id)
    .fetch_all(db)
    .await?;
    Ok(Json(json!({
        "id": id, "username": username, "created_ms": created, "last_login_ms": last,
        "banned": banned == 1, "ban_reason": reason, "admin_note": note,
        "deleted": reason == "Account deleted by an admin",
        "sessions": sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sessions WHERE user_id = ? AND expires_ms > ?").bind(id).bind(now_ms()).fetch_one(db).await?,
        "messages": n1(db, "SELECT COUNT(*) FROM messages WHERE user_id = ?", id).await?,
        "owes": n1(db, "SELECT COUNT(*) FROM cases WHERE debtor_id = ? AND sentence IS NOT NULL AND sentence_paid = 0", id).await?,
        "owed": n1(db, "SELECT COUNT(*) FROM cases WHERE creditor_id = ? AND sentence IS NOT NULL AND sentence_paid = 0", id).await?,
        "cases": cases.into_iter().map(|(cid, t, p, o, r)| json!({"id": cid, "title": t, "phase": p, "outcome": o, "role": r})).collect::<Vec<_>>(),
        "recent_messages": msgs.into_iter().map(|(c, b, at, rm)| json!({"case_id": c, "body": b, "at": at, "removed": rm == 1})).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
pub struct Reason {
    #[serde(default)]
    reason: String,
}

pub async fn ban_user(State(st): State<AppState>, a: AdminUser, Path(id): Path<i64>, Json(b): Json<Reason>) -> ApiResult<Json<Value>> {
    let name = user_name(&st.db, id).await?;
    let reason = line(&b.reason, 0, 200, "The reason")?;
    sqlx::query("UPDATE users SET banned = 1, ban_reason = ? WHERE id = ?").bind(&reason).bind(id).execute(&st.db).await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = ?").bind(id).execute(&st.db).await?;
    audit(&st.db, &a.username, "ban_user", &format!("user:{id} {name}"), &reason).await;
    Ok(Json(json!({ "ok": true })))
}

pub async fn unban_user(State(st): State<AppState>, a: AdminUser, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let name = user_name(&st.db, id).await?;
    let r = sqlx::query("UPDATE users SET banned = 0, ban_reason = '' WHERE id = ? AND pw_hash != '!'").bind(id).execute(&st.db).await?;
    if r.rows_affected() != 1 {
        return Err(ApiError::conflict("Deleted accounts can't be restored."));
    }
    audit(&st.db, &a.username, "unban_user", &format!("user:{id} {name}"), "").await;
    Ok(Json(json!({ "ok": true })))
}

pub async fn user_logout(State(st): State<AppState>, a: AdminUser, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let name = user_name(&st.db, id).await?;
    let r = sqlx::query("DELETE FROM sessions WHERE user_id = ?").bind(id).execute(&st.db).await?;
    audit(&st.db, &a.username, "logout_user", &format!("user:{id} {name}"), &format!("{} sessions ended", r.rows_affected())).await;
    Ok(Json(json!({ "ok": true, "ended": r.rows_affected() })))
}

pub async fn user_reset_password(State(st): State<AppState>, a: AdminUser, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let name = user_name(&st.db, id).await?;
    let deleted: i64 = n1(&st.db, "SELECT COUNT(*) FROM users WHERE id = ? AND pw_hash = '!'", id).await?;
    if deleted == 1 {
        return Err(ApiError::conflict("This account was deleted."));
    }
    let temp = random_token(12);
    let hash = hash_password(temp.clone()).await?;
    sqlx::query("UPDATE users SET pw_hash = ? WHERE id = ?").bind(hash).bind(id).execute(&st.db).await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = ?").bind(id).execute(&st.db).await?;
    audit(&st.db, &a.username, "reset_password", &format!("user:{id} {name}"), "temporary password issued").await;
    Ok(Json(json!({ "password": temp })))
}

#[derive(Deserialize)]
pub struct Rename {
    username: String,
}

pub async fn user_rename(State(st): State<AppState>, a: AdminUser, Path(id): Path<i64>, Json(b): Json<Rename>) -> ApiResult<Json<Value>> {
    let old = user_name(&st.db, id).await?;
    let new = b.username.trim().to_string();
    if !valid_username(&new) {
        return Err(ApiError::bad("Usernames are 3 to 20 characters: letters, numbers, dots and underscores."));
    }
    match sqlx::query("UPDATE users SET username = ? WHERE id = ? AND pw_hash != '!'").bind(&new).bind(id).execute(&st.db).await {
        Ok(r) if r.rows_affected() == 1 => {}
        Ok(_) => return Err(ApiError::conflict("Deleted accounts can't be renamed.")),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => return Err(ApiError::conflict("That username is taken.")),
        Err(e) => return Err(e.into()),
    }
    audit(&st.db, &a.username, "rename_user", &format!("user:{id}"), &format!("{old} -> {new}")).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct Note {
    note: String,
}

pub async fn user_note(State(st): State<AppState>, a: AdminUser, Path(id): Path<i64>, Json(b): Json<Note>) -> ApiResult<Json<Value>> {
    let name = user_name(&st.db, id).await?;
    let note = text(&b.note, 0, 500, "The note")?;
    sqlx::query("UPDATE users SET admin_note = ? WHERE id = ?").bind(&note).bind(id).execute(&st.db).await?;
    audit(&st.db, &a.username, "edit_note", &format!("user:{id} {name}"), "").await;
    Ok(Json(json!({ "ok": true })))
}

/// Cases keep their history, so "delete" blanks the identity instead of erasing rows.
pub async fn user_delete(State(st): State<AppState>, a: AdminUser, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let name = user_name(&st.db, id).await?;
    let mut tx = st.db.begin().await?;
    sqlx::query(
        "UPDATE users SET username = ?, pw_hash = '!', banned = 1, ban_reason = 'Account deleted by an admin' WHERE id = ?",
    )
    .bind(format!("deleted-{id}"))
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = ?").bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    audit(&st.db, &a.username, "delete_user", &format!("user:{id}"), &format!("was {name}")).await;
    Ok(Json(json!({ "ok": true })))
}

/* ---------- cases ---------- */

pub async fn cases_list(State(st): State<AppState>, _a: AdminUser, Query(q): Query<ListQ>) -> ApiResult<Json<Value>> {
    let term = q.q.unwrap_or_default().trim().to_string();
    let phase = q.phase.unwrap_or_default();
    let page = q.page.unwrap_or(1).max(1);
    let filter = "(?1 = '' OR c.phase = ?1) AND (?2 = '' OR c.title LIKE ?3 ESCAPE '\\' OR c.id = ?2 \
                  OR u.username LIKE ?3 ESCAPE '\\' OR c.charge LIKE ?3 ESCAPE '\\')";
    let total: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM cases c JOIN users u ON u.id = c.plaintiff_id WHERE {filter}"))
        .bind(&phase)
        .bind(&term)
        .bind(like(&term))
        .fetch_one(&st.db)
        .await?;
    let rows: Vec<(String, String, String, Option<String>, i64, String, String, i64)> = sqlx::query_as(&format!(
        "SELECT c.id, c.title, c.phase, c.outcome, c.created_ms, u.username, c.defendant_label, \
         (SELECT COUNT(*) FROM participants p WHERE p.case_id = c.id) \
         FROM cases c JOIN users u ON u.id = c.plaintiff_id WHERE {filter} \
         ORDER BY c.created_ms DESC LIMIT ?4 OFFSET ?5"
    ))
    .bind(&phase)
    .bind(&term)
    .bind(like(&term))
    .bind(PER_PAGE)
    .bind((page - 1) * PER_PAGE)
    .fetch_all(&st.db)
    .await?;
    let cases: Vec<Value> = rows
        .into_iter()
        .map(|(id, title, phase, outcome, created, pl, def, people)| {
            json!({ "id": id, "title": title, "phase": phase, "outcome": outcome, "created_ms": created,
                    "plaintiff": pl, "defendant_label": def, "people": people })
        })
        .collect();
    Ok(Json(json!({ "cases": cases, "total": total, "page": page, "per_page": PER_PAGE })))
}

async fn case_row(db: &SqlitePool, id: &str) -> ApiResult<CaseRow> {
    load_case(db, id).await?.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "No such case."))
}

pub async fn case_detail(State(st): State<AppState>, _a: AdminUser, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let db = &st.db;
    let c = case_row(db, &id).await?;
    let plaintiff = user_name(db, c.plaintiff_id).await?;
    let defendant = match c.defendant_id {
        Some(d) => Some(user_name(db, d).await?),
        None => None,
    };
    let people: Vec<(i64, String, String, i64)> = sqlx::query_as(
        "SELECT u.id, u.username, p.role, p.joined_ms FROM participants p JOIN users u ON u.id = p.user_id \
         WHERE p.case_id = ? ORDER BY p.joined_ms",
    )
    .bind(&id)
    .fetch_all(db)
    .await?;
    let msgs: Vec<(i64, String, String, String, String, i64, i64)> = sqlx::query_as(
        "SELECT m.id, u.username, m.side, m.kind, m.body, m.created_ms, m.removed FROM messages m \
         JOIN users u ON u.id = m.user_id WHERE m.case_id = ? ORDER BY m.id",
    )
    .bind(&id)
    .fetch_all(db)
    .await?;
    let votes: Vec<(String, String)> = sqlx::query_as(
        "SELECT u.username, v.vote FROM votes v JOIN users u ON u.id = v.user_id WHERE v.case_id = ? ORDER BY u.username",
    )
    .bind(&id)
    .fetch_all(db)
    .await?;
    let invites: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT i.role, u.username FROM invites i LEFT JOIN users u ON u.id = i.claimed_by WHERE i.case_id = ? ORDER BY i.rowid",
    )
    .bind(&id)
    .fetch_all(db)
    .await?;
    Ok(Json(json!({
        "id": c.id, "title": c.title, "charge": c.charge, "details": c.details, "tier": c.tier, "demand": c.demand,
        "phase": c.phase, "deadline_ms": c.deadline_ms, "created_ms": c.created_ms, "outcome": c.outcome,
        "plaintiff": plaintiff, "defendant": defendant, "defendant_label": c.defendant_label,
        "guilty": c.guilty, "not_guilty": c.not_guilty,
        "sentence": c.sentence, "sentence_paid": c.sentence_paid == 1,
        "people": people.into_iter().map(|(uid, n, r, at)| json!({"id": uid, "username": n, "role": r, "joined_ms": at})).collect::<Vec<_>>(),
        "messages": msgs.into_iter().map(|(mid, u, side, kind, body, at, rm)| json!({"id": mid, "user": u, "side": side, "kind": kind, "body": body, "at": at, "removed": rm == 1})).collect::<Vec<_>>(),
        "votes": votes.into_iter().map(|(u, v)| json!({"user": u, "vote": v})).collect::<Vec<_>>(),
        "invites": invites.into_iter().map(|(r, u)| json!({"role": r, "claimed_by": u})).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
pub struct CloseBody {
    outcome: String,
}

pub async fn case_close(State(st): State<AppState>, a: AdminUser, Path(id): Path<String>, Json(b): Json<CloseBody>) -> ApiResult<Json<Value>> {
    let c = case_row(&st.db, &id).await?;
    let phase = Phase::parse(&c.phase).unwrap_or(Phase::Closed);
    if phase == Phase::Closed {
        return Err(ApiError::conflict("This case is already closed."));
    }
    let outcome = match b.outcome.as_str() {
        "guilty" => Outcome::Guilty,
        "not_guilty" => Outcome::NotGuilty,
        "default_judgment" => Outcome::DefaultJudgment,
        "dismissed" => Outcome::Dismissed,
        "mistrial" => Outcome::Mistrial,
        "abandoned" => Outcome::Abandoned,
        "withdrawn" => Outcome::Withdrawn,
        _ => return Err(ApiError::bad("Pick a valid outcome.")),
    };
    let g: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM votes WHERE case_id = ? AND vote = 'guilty'").bind(&id).fetch_one(&st.db).await?;
    let ng: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM votes WHERE case_id = ? AND vote = 'not_guilty'").bind(&id).fetch_one(&st.db).await?;
    if !engine::close_case(&st.db, &c, phase, outcome, g, ng, now_ms()).await? {
        return Err(ApiError::conflict("The case moved on before it could be closed. Reload and retry."));
    }
    audit(&st.db, &a.username, "close_case", &format!("case:{id}"), outcome.as_str()).await;
    st.hub.notify(&id);
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct Extend {
    minutes: i64,
}

pub async fn case_extend(State(st): State<AppState>, a: AdminUser, Path(id): Path<String>, Json(b): Json<Extend>) -> ApiResult<Json<Value>> {
    if !(1..=10080).contains(&b.minutes) {
        return Err(ApiError::bad("Extend by 1 minute to 7 days."));
    }
    let r = sqlx::query("UPDATE cases SET deadline_ms = max(deadline_ms, ?) + ? WHERE id = ? AND phase != 'closed'")
        .bind(now_ms())
        .bind(b.minutes * MIN)
        .bind(&id)
        .execute(&st.db)
        .await?;
    if r.rows_affected() != 1 {
        return Err(ApiError::conflict("Only open cases can be extended."));
    }
    audit(&st.db, &a.username, "extend_case", &format!("case:{id}"), &format!("+{} min", b.minutes)).await;
    st.hub.notify(&id);
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct PaidBody {
    paid: bool,
}

pub async fn case_paid(State(st): State<AppState>, a: AdminUser, Path(id): Path<String>, Json(b): Json<PaidBody>) -> ApiResult<Json<Value>> {
    let r = sqlx::query("UPDATE cases SET sentence_paid = ? WHERE id = ? AND phase = 'closed' AND sentence IS NOT NULL")
        .bind(b.paid as i64)
        .bind(&id)
        .execute(&st.db)
        .await?;
    if r.rows_affected() != 1 {
        return Err(ApiError::conflict("This case has no sentence to mark."));
    }
    audit(&st.db, &a.username, "mark_paid", &format!("case:{id}"), if b.paid { "paid" } else { "unpaid" }).await;
    st.hub.notify(&id);
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct EditCase {
    title: String,
    charge: String,
    demand: String,
    #[serde(default)]
    details: String,
}

pub async fn case_edit(State(st): State<AppState>, a: AdminUser, Path(id): Path<String>, Json(b): Json<EditCase>) -> ApiResult<Json<Value>> {
    let title = line(&b.title, 3, 80, "The title")?;
    let charge = line(&b.charge, 3, 80, "The charge")?;
    let demand = line(&b.demand, 1, 60, "The demand")?;
    let details = text(&b.details, 0, 600, "The details")?;
    let r = sqlx::query("UPDATE cases SET title = ?, charge = ?, demand = ?, details = ? WHERE id = ?")
        .bind(title)
        .bind(charge)
        .bind(demand)
        .bind(details)
        .bind(&id)
        .execute(&st.db)
        .await?;
    if r.rows_affected() != 1 {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "No such case."));
    }
    audit(&st.db, &a.username, "edit_case", &format!("case:{id}"), "").await;
    st.hub.notify(&id);
    Ok(Json(json!({ "ok": true })))
}

pub async fn message_remove(State(st): State<AppState>, a: AdminUser, Path((id, mid)): Path<(String, i64)>) -> ApiResult<Json<Value>> {
    let r = sqlx::query("UPDATE messages SET body = ?, removed = 1 WHERE id = ? AND case_id = ?")
        .bind(REMOVED)
        .bind(mid)
        .bind(&id)
        .execute(&st.db)
        .await?;
    if r.rows_affected() != 1 {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "No such message."));
    }
    audit(&st.db, &a.username, "remove_message", &format!("case:{id}"), &format!("message {mid}")).await;
    st.hub.notify(&id);
    Ok(Json(json!({ "ok": true })))
}

pub async fn case_delete(State(st): State<AppState>, a: AdminUser, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let c = case_row(&st.db, &id).await?;
    let mut tx = st.db.begin().await?;
    for sql in [
        "DELETE FROM votes WHERE case_id = ?",
        "DELETE FROM messages WHERE case_id = ?",
        "DELETE FROM invites WHERE case_id = ?",
        "DELETE FROM participants WHERE case_id = ?",
        "DELETE FROM cases WHERE id = ?",
    ] {
        sqlx::query(sql).bind(&id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    audit(&st.db, &a.username, "delete_case", &format!("case:{id}"), &c.title).await;
    st.hub.notify(&id);
    Ok(Json(json!({ "ok": true })))
}

/* ---------- site settings and the audit log ---------- */

pub async fn settings_get(State(st): State<AppState>, _a: AdminUser) -> ApiResult<Json<Value>> {
    Ok(Json(json!({
        "signups_open": signups_open(&st.db).await,
        "announcement": get_setting(&st.db, "announcement", "").await,
        "max_open_cases": max_open_cases(&st.db).await,
    })))
}

#[derive(Deserialize)]
pub struct SettingsBody {
    signups_open: bool,
    announcement: String,
    max_open_cases: i64,
}

pub async fn settings_set(State(st): State<AppState>, a: AdminUser, Json(b): Json<SettingsBody>) -> ApiResult<Json<Value>> {
    let announcement = line(&b.announcement, 0, 200, "The announcement")?;
    if !(1..=100).contains(&b.max_open_cases) {
        return Err(ApiError::bad("Open cases per person must be between 1 and 100."));
    }
    set_setting(&st.db, "signups_open", if b.signups_open { "1" } else { "0" }).await?;
    set_setting(&st.db, "announcement", &announcement).await?;
    set_setting(&st.db, "max_open_cases", &b.max_open_cases.to_string()).await?;
    audit(
        &st.db,
        &a.username,
        "change_settings",
        "site",
        &format!("signups_open={} max_open_cases={} announcement={}", b.signups_open, b.max_open_cases, if announcement.is_empty() { "off" } else { "on" }),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}

pub async fn log_list(State(st): State<AppState>, _a: AdminUser) -> ApiResult<Json<Value>> {
    let rows: Vec<(i64, i64, String, String, String, String)> = sqlx::query_as(
        "SELECT id, at_ms, admin, action, target, detail FROM admin_log ORDER BY id DESC LIMIT 200",
    )
    .fetch_all(&st.db)
    .await?;
    Ok(Json(json!({ "log": rows.into_iter().map(|(id, at, ad, ac, t, d)| json!({"id": id, "at": at, "admin": ad, "action": ac, "target": t, "detail": d})).collect::<Vec<_>>() })))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/me", get(me))
        .route("/password", post(change_password))
        .route("/stats", get(stats))
        .route("/users", get(users_list))
        .route("/users/{id}", get(user_detail))
        .route("/users/{id}/ban", post(ban_user))
        .route("/users/{id}/unban", post(unban_user))
        .route("/users/{id}/logout", post(user_logout))
        .route("/users/{id}/password", post(user_reset_password))
        .route("/users/{id}/rename", post(user_rename))
        .route("/users/{id}/note", post(user_note))
        .route("/users/{id}/delete", post(user_delete))
        .route("/cases", get(cases_list))
        .route("/cases/{id}", get(case_detail))
        .route("/cases/{id}/close", post(case_close))
        .route("/cases/{id}/extend", post(case_extend))
        .route("/cases/{id}/paid", post(case_paid))
        .route("/cases/{id}/edit", post(case_edit))
        .route("/cases/{id}/delete", post(case_delete))
        .route("/cases/{id}/messages/{mid}/remove", post(message_remove))
        .route("/settings", get(settings_get).post(settings_set))
        .route("/log", get(log_list))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn loopback_without_proxy_headers_is_local() {
        let h = HeaderMap::new();
        assert!(is_local(&addr("127.0.0.1:5000"), &h));
        assert!(is_local(&addr("[::1]:5000"), &h));
        assert!(is_local(&addr("[::ffff:127.0.0.1]:5000"), &h));
    }

    #[test]
    fn other_machines_are_not_local() {
        let h = HeaderMap::new();
        assert!(!is_local(&addr("192.168.1.20:5000"), &h));
        assert!(!is_local(&addr("203.0.113.9:5000"), &h));
        assert!(!is_local(&addr("[2001:db8::1]:5000"), &h));
    }

    #[test]
    fn a_proxy_on_this_machine_does_not_count_as_local() {
        for name in ["x-forwarded-for", "x-real-ip", "forwarded", "cf-connecting-ip", "via"] {
            let mut h = HeaderMap::new();
            h.insert(name, HeaderValue::from_static("203.0.113.9"));
            assert!(!is_local(&addr("127.0.0.1:5000"), &h), "{name} should disable local access");
        }
    }

    #[test]
    fn search_text_is_escaped_for_like() {
        assert_eq!(like("50%_off\\"), "%50\\%\\_off\\\\%");
        assert_eq!(like(""), "%%");
    }

    #[test]
    fn reserved_and_invalid_usernames() {
        assert!(!valid_username("deleted-5"));
        assert!(!valid_username("Deleted-5"));
        assert!(!valid_username("ab"));
        assert!(!valid_username("has space"));
        assert!(valid_username("kavya_2.0"));
    }
}
