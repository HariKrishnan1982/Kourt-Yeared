use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::broadcast;

use crate::model::now_ms;

/// One broadcast channel per live case room. A message means "something changed,
/// re-read your snapshot" so every viewer gets only what their role may see.
#[derive(Clone, Default)]
pub struct Hub(Arc<Mutex<HashMap<String, broadcast::Sender<()>>>>);

impl Hub {
    pub fn subscribe(&self, case_id: &str) -> broadcast::Receiver<()> {
        let mut m = self.0.lock().unwrap();
        m.retain(|_, s| s.receiver_count() > 0);
        m.entry(case_id.to_string())
            .or_insert_with(|| broadcast::channel(16).0)
            .subscribe()
    }
    pub fn notify(&self, case_id: &str) {
        let mut m = self.0.lock().unwrap();
        if let Some(s) = m.get(case_id) {
            if s.receiver_count() > 0 {
                let _ = s.send(());
            } else {
                m.remove(case_id);
            }
        }
    }
}

#[derive(Clone, Default)]
pub struct Limiter(Arc<Mutex<HashMap<String, (u32, i64, i64)>>>);

impl Limiter {
    /// Records an attempt; returns false once `max` attempts land inside `window_ms`.
    pub fn hit(&self, key: &str, max: u32, window_ms: i64) -> bool {
        let now = now_ms();
        let mut m = self.0.lock().unwrap();
        if m.len() > 10_000 {
            m.retain(|_, (_, start, win)| now - *start < *win);
        }
        let e = m.entry(key.to_string()).or_insert((0, now, window_ms));
        if now - e.1 >= e.2 {
            *e = (0, now, window_ms);
        }
        e.0 += 1;
        e.0 <= max
    }

    /// True when `key` already has `max` recorded failures inside the window.
    pub fn blocked(&self, key: &str, max: u32, window_ms: i64) -> bool {
        let now = now_ms();
        match self.0.lock().unwrap().get(key) {
            Some((n, start, win)) if now - *start < window_ms.min(*win) => *n >= max,
            _ => false,
        }
    }

    pub fn record(&self, key: &str, window_ms: i64) {
        let now = now_ms();
        let mut m = self.0.lock().unwrap();
        let e = m.entry(key.to_string()).or_insert((0, now, window_ms));
        if now - e.1 >= e.2 {
            *e = (0, now, window_ms);
        }
        e.0 += 1;
    }

    pub fn clear(&self, key: &str) {
        self.0.lock().unwrap().remove(key);
    }
}

#[derive(Clone)]
pub struct AppState {
    pub db: sqlx::SqlitePool,
    pub hub: Hub,
    pub limiter: Limiter,
    pub secure_cookies: bool,
    pub admin_remote: bool,
    pub started_ms: i64,
}
