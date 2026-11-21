use std::time::Duration;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::{header, HeaderMap},
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast::error::RecvError;

use crate::{
    api::snapshot,
    auth::AuthUser,
    error::{ApiError, ApiResult},
    model::role_of,
    state::AppState,
};

/// Browsers always send Origin on WebSocket handshakes. Refuse other sites, so a
/// page on another domain can't open a socket using a visitor's cookie.
fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    let origin_host = origin.split("://").nth(1).unwrap_or("");
    !host.is_empty() && origin_host == host
}

pub async fn ws_handler(
    State(st): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> ApiResult<Response> {
    if !same_origin(&headers) {
        return Err(ApiError::forbidden("Cross-site connections aren't allowed."));
    }
    if role_of(&st.db, &id, user.id).await?.is_none() {
        return Err(ApiError::not_found());
    }
    Ok(ws.on_upgrade(move |socket| run(st, user.id, id, socket)))
}

async fn send_snapshot(st: &AppState, id: &str, uid: i64, tx: &mut futures_util::stream::SplitSink<WebSocket, Message>) -> bool {
    match snapshot(&st.db, id, uid).await {
        Ok(Some(v)) => tx.send(Message::Text(v.to_string().into())).await.is_ok(),
        Ok(None) => false,
        Err(e) => {
            eprintln!("snapshot error: {e}");
            true
        }
    }
}

async fn run(st: AppState, uid: i64, id: String, socket: WebSocket) {
    let (mut tx, mut rx) = socket.split();
    let mut sub = st.hub.subscribe(&id);
    if !send_snapshot(&st, &id, uid, &mut tx).await {
        return;
    }
    let mut ping = tokio::time::interval(Duration::from_secs(25));
    ping.tick().await;
    loop {
        tokio::select! {
            r = sub.recv() => match r {
                Ok(()) | Err(RecvError::Lagged(_)) => {
                    if !send_snapshot(&st, &id, uid, &mut tx).await { break; }
                }
                Err(RecvError::Closed) => break,
            },
            _ = ping.tick() => {
                if tx.send(Message::Ping(Vec::new().into())).await.is_err() { break; }
            }
            m = rx.next() => match m {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                _ => {}
            },
        }
    }
}
