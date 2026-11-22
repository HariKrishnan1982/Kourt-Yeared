mod admin;
mod api;
mod auth;
mod db;
mod engine;
mod error;
mod model;
mod state;
mod ws;

use std::{net::SocketAddr, time::Duration};

use axum::{
    extract::Request,
    http::{header, HeaderName, HeaderValue, Method, StatusCode, Uri},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    extract::DefaultBodyLimit,
    Router,
};
use rust_embed::RustEmbed;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::{
    model::now_ms,
    state::{AppState, Hub, Limiter},
};

#[derive(RustEmbed)]
#[folder = "web/"]
pub struct Assets;

/// Every state-changing API call must carry a custom header. Browsers won't add
/// one on a cross-site request without a CORS preflight, which we never allow.
async fn require_marker(req: Request, next: Next) -> Response {
    let changes_state = !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if changes_state && !req.headers().contains_key("x-kourt") {
        return (StatusCode::BAD_REQUEST, "missing x-kourt header").into_response();
    }
    next.run(req).await
}

async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    if !path.is_empty() && !path.contains("..") {
        if let Some(f) = Assets::get(path) {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            return (
                [
                    (header::CONTENT_TYPE, mime.as_ref().to_string()),
                    (header::CACHE_CONTROL, "public, max-age=300".to_string()),
                ],
                f.data.into_owned(),
            )
                .into_response();
        }
        if path.contains('.') {
            return (StatusCode::NOT_FOUND, "not found").into_response();
        }
    }
    // Everything else is a page route handled by the front end.
    match Assets::get("index.html") {
        Some(f) => (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8".to_string()),
                (header::CACHE_CONTROL, "no-cache".to_string()),
            ],
            f.data.into_owned(),
        )
            .into_response(),
        None => (StatusCode::INTERNAL_SERVER_ERROR, "front end missing").into_response(),
    }
}

pub fn app(state: AppState) -> Router {
    let api_routes = Router::new()
        .route("/signup", post(auth::signup))
        .route("/login", post(auth::login))
        .route("/logout", post(auth::logout))
        .route("/me", get(auth::me))
        .route("/cases", post(api::create_case).get(api::list_cases))
        .route("/cases/{id}", get(api::get_case))
        .route("/cases/{id}/messages", post(api::post_message))
        .route("/cases/{id}/vote", post(api::vote))
        .route("/cases/{id}/paid", post(api::mark_paid))
        .route("/cases/{id}/withdraw", post(api::withdraw))
        .route("/cases/{id}/ws", get(ws::ws_handler))
        .route("/join/{token}", get(api::peek_join).post(api::join))
        .route("/site", get(admin::site))
        .nest("/admin", admin::routes())
        .layer(middleware::from_fn(require_marker));

    let csp = "default-src 'self'; script-src 'self'; style-src 'self' https://fonts.googleapis.com; \
               font-src https://fonts.gstatic.com; img-src 'self' data:; connect-src 'self'; \
               frame-ancestors 'none'; base-uri 'none'; form-action 'self'";
    Router::new()
        .nest("/api", api_routes)
        .route("/admin", get(admin::page))
        .route("/admin/{*rest}", get(admin::page))
        .fallback(static_handler)
        .layer(DefaultBodyLimit::max(16 * 1024))
        .layer(SetResponseHeaderLayer::if_not_present(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(csp)))
        .layer(SetResponseHeaderLayer::if_not_present(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(SetResponseHeaderLayer::if_not_present(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer")))
        .layer(SetResponseHeaderLayer::if_not_present(HeaderName::from_static("x-frame-options"), HeaderValue::from_static("DENY")))
        .with_state(state)
}

/// Once a second, move along any case whose clock has run out. State lives in the
/// database, so a restart picks up exactly where the court left off.
fn spawn_clock(st: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tick.tick().await;
            let now = now_ms();
            let due: Vec<String> = match sqlx::query_scalar("SELECT id FROM cases WHERE phase != 'closed' AND deadline_ms <= ?")
                .bind(now)
                .fetch_all(&st.db)
                .await
            {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("clock query failed: {e}");
                    continue;
                }
            };
            for id in due {
                match engine::advance(&st.db, &id, now).await {
                    Ok(true) => st.hub.notify(&id),
                    Ok(false) => {}
                    Err(e) => eprintln!("advance failed for {id}: {e}"),
                }
            }
        }
    });
}

#[tokio::main]
async fn main() {
    let addr: SocketAddr = std::env::var("KOURT_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:3000".into())
        .parse()
        .expect("KOURT_ADDR must look like 127.0.0.1:3000");
    let db_url = std::env::var("KOURT_DB").unwrap_or_else(|_| "sqlite://kourt.db".into());
    let pool = db::connect(&db_url).await.expect("could not open the database");
    let state = AppState {
        db: pool,
        hub: Hub::default(),
        limiter: Limiter::default(),
        secure_cookies: std::env::var("KOURT_SECURE_COOKIES").map(|v| v == "1").unwrap_or(false),
        admin_remote: std::env::var("KOURT_ADMIN_REMOTE").map(|v| v == "1").unwrap_or(false),
        started_ms: now_ms(),
    };
    admin::bootstrap(&state.db).await;
    spawn_clock(state.clone());
    let listener = tokio::net::TcpListener::bind(addr).await.expect("could not bind the address");
    println!("Kourt Yeared is in session at http://{addr}");
    axum::serve(listener, app(state).into_make_service_with_connect_info::<SocketAddr>())
        .await
        .expect("server error");
}
