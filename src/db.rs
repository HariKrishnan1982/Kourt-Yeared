use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
    SqlitePool,
};
use std::{str::FromStr, time::Duration};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS users(
  id INTEGER PRIMARY KEY,
  username TEXT NOT NULL UNIQUE COLLATE NOCASE,
  pw_hash TEXT NOT NULL,
  created_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions(
  token_hash TEXT PRIMARY KEY,
  user_id INTEGER NOT NULL REFERENCES users(id),
  expires_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_user ON sessions(user_id);
CREATE TABLE IF NOT EXISTS cases(
  id TEXT PRIMARY KEY,
  plaintiff_id INTEGER NOT NULL REFERENCES users(id),
  defendant_id INTEGER REFERENCES users(id),
  defendant_label TEXT NOT NULL,
  title TEXT NOT NULL,
  charge TEXT NOT NULL,
  details TEXT NOT NULL,
  tier TEXT NOT NULL,
  demand TEXT NOT NULL,
  phase TEXT NOT NULL,
  deadline_ms INTEGER NOT NULL,
  summons_ms INTEGER NOT NULL,
  args_ms INTEGER NOT NULL,
  vote_ms INTEGER NOT NULL,
  outcome TEXT,
  debtor_id INTEGER,
  creditor_id INTEGER,
  sentence TEXT,
  sentence_paid INTEGER NOT NULL DEFAULT 0,
  guilty INTEGER NOT NULL DEFAULT 0,
  not_guilty INTEGER NOT NULL DEFAULT 0,
  created_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS cases_open ON cases(phase, deadline_ms);
CREATE INDEX IF NOT EXISTS cases_debtor ON cases(debtor_id, sentence_paid);
CREATE INDEX IF NOT EXISTS cases_creditor ON cases(creditor_id, sentence_paid);
CREATE TABLE IF NOT EXISTS participants(
  case_id TEXT NOT NULL REFERENCES cases(id),
  user_id INTEGER NOT NULL REFERENCES users(id),
  role TEXT NOT NULL,
  joined_ms INTEGER NOT NULL,
  PRIMARY KEY(case_id, user_id)
);
CREATE INDEX IF NOT EXISTS participants_user ON participants(user_id);
CREATE TABLE IF NOT EXISTS invites(
  token TEXT PRIMARY KEY,
  case_id TEXT NOT NULL REFERENCES cases(id),
  role TEXT NOT NULL,
  claimed_by INTEGER
);
CREATE INDEX IF NOT EXISTS invites_case ON invites(case_id);
CREATE TABLE IF NOT EXISTS messages(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  case_id TEXT NOT NULL REFERENCES cases(id),
  user_id INTEGER NOT NULL,
  side TEXT NOT NULL,
  kind TEXT NOT NULL,
  body TEXT NOT NULL,
  created_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS messages_case ON messages(case_id, id);
CREATE INDEX IF NOT EXISTS messages_user ON messages(user_id, created_ms);
CREATE TABLE IF NOT EXISTS votes(
  case_id TEXT NOT NULL REFERENCES cases(id),
  user_id INTEGER NOT NULL,
  vote TEXT NOT NULL,
  PRIMARY KEY(case_id, user_id)
);
";

/// Each entry upgrades the database by one version (entry 0 takes v1 to v2).
/// v1 is the original schema above. Never edit an old entry; add a new one.
const MIGRATIONS: &[&str] = &[
    "ALTER TABLE users ADD COLUMN banned INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE users ADD COLUMN ban_reason TEXT NOT NULL DEFAULT '';
     ALTER TABLE users ADD COLUMN admin_note TEXT NOT NULL DEFAULT '';
     ALTER TABLE users ADD COLUMN last_login_ms INTEGER;
     ALTER TABLE messages ADD COLUMN removed INTEGER NOT NULL DEFAULT 0;
     CREATE TABLE IF NOT EXISTS admins(
       id INTEGER PRIMARY KEY,
       username TEXT NOT NULL UNIQUE COLLATE NOCASE,
       pw_hash TEXT NOT NULL,
       created_ms INTEGER NOT NULL
     );
     CREATE TABLE IF NOT EXISTS admin_sessions(
       token_hash TEXT PRIMARY KEY,
       admin_id INTEGER NOT NULL REFERENCES admins(id),
       expires_ms INTEGER NOT NULL
     );
     CREATE TABLE IF NOT EXISTS admin_log(
       id INTEGER PRIMARY KEY AUTOINCREMENT,
       at_ms INTEGER NOT NULL,
       admin TEXT NOT NULL,
       action TEXT NOT NULL,
       target TEXT NOT NULL,
       detail TEXT NOT NULL
     );
     CREATE TABLE IF NOT EXISTS settings(
       key TEXT PRIMARY KEY,
       value TEXT NOT NULL
     );",
    "CREATE INDEX IF NOT EXISTS sessions_user ON sessions(user_id);
     CREATE INDEX IF NOT EXISTS admin_sessions_admin ON admin_sessions(admin_id);
     CREATE INDEX IF NOT EXISTS cases_debtor ON cases(debtor_id, sentence_paid);
     CREATE INDEX IF NOT EXISTS cases_creditor ON cases(creditor_id, sentence_paid);
     CREATE INDEX IF NOT EXISTS invites_case ON invites(case_id);
     CREATE INDEX IF NOT EXISTS messages_user ON messages(user_id, created_ms);",
];

async fn migrate(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let current: i64 = sqlx::query_scalar("PRAGMA user_version").fetch_one(pool).await?;
    // A database made before versioning existed already has the v1 schema.
    let have = current.max(1);
    let mut version = have;
    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let target = i as i64 + 2;
        if have >= target {
            continue;
        }
        let script = format!("BEGIN;\n{sql}\nPRAGMA user_version = {target};\nCOMMIT;");
        if let Err(e) = sqlx::raw_sql(&script).execute(pool).await {
            let _ = sqlx::query("ROLLBACK").execute(pool).await;
            return Err(e);
        }
        version = target;
    }
    if version != current && current < 1 {
        sqlx::raw_sql(&format!("PRAGMA user_version = {version};")).execute(pool).await?;
    }
    Ok(())
}

pub fn resolve_db_url(url: &str) -> String {
    #[cfg(windows)]
    {
        for prefix in ["sqlite:///tmp/", "sqlite:/tmp/", "sqlite://tmp/", "sqlite://./tmp/"] {
            if let Some(rest) = url.strip_prefix(prefix) {
                let temp = std::env::temp_dir();
                let db_path = temp.join(rest);
                return format!("sqlite://{}", db_path.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    url.to_string()
}

/// One connection on purpose: SQLite serialises writers anyway, and a single
/// connection makes every transaction in this app strictly ordered. Never call
/// `&pool` helpers while a transaction is open or the app will wait on itself.
pub async fn connect(url: &str) -> Result<SqlitePool, sqlx::Error> {
    let resolved = resolve_db_url(url);
    let opts = SqliteConnectOptions::from_str(&resolved)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await?;
    sqlx::raw_sql(SCHEMA).execute(&pool).await?;
    migrate(&pool).await?;
    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_db_url() {
        #[cfg(windows)]
        {
            let resolved = resolve_db_url("sqlite:///tmp/e2e.db");
            assert!(resolved.starts_with("sqlite://") && resolved.ends_with("e2e.db"));
        }
        assert_eq!(resolve_db_url("sqlite://kourt.db"), "sqlite://kourt.db");
    }
}

