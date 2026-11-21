use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::SqliteConnection;

use crate::{
    auth::AuthUser,
    engine,
    error::{ApiError, ApiResult},
    model::*,
    state::AppState,
};

fn clean(s: &str, allow_newlines: bool) -> String {
    s.chars()
        .filter(|c| !c.is_control() || (allow_newlines && *c == '\n'))
        .collect::<String>()
        .trim()
        .to_string()
}

fn checked(s: String, min: usize, max: usize, field: &str) -> ApiResult<String> {
    let n = s.chars().count();
    if n > max {
        return Err(ApiError::bad(format!("{field} can be at most {max} characters.")));
    }
    if n < min {
        return Err(ApiError::bad(format!("{field} needs at least {min} character{}.", if min == 1 { "" } else { "s" })));
    }
    Ok(s)
}

fn line(s: &str, min: usize, max: usize, field: &str) -> ApiResult<String> {
    checked(clean(s, false), min, max, field)
}

fn text(s: &str, min: usize, max: usize, field: &str) -> ApiResult<String> {
    checked(clean(s, true), min, max, field)
}

#[derive(Deserialize)]
pub struct NewCase {
    title: String,
    charge: String,
    defendant_name: String,
    #[serde(default)]
    details: String,
    tier: String,
    #[serde(default)]
    demand: String,
    preset: Option<String>,
    minutes: Option<i64>,
}

pub async fn create_case(State(st): State<AppState>, user: AuthUser, Json(b): Json<NewCase>) -> ApiResult<Json<Value>> {
    let title = line(&b.title, 3, 80, "The title")?;
    let charge = line(&b.charge, 3, 80, "The charge")?;
    let label = line(&b.defendant_name, 1, 24, "The defendant's name")?;
    let details = text(&b.details, 0, 600, "The details")?;
    if !matches!(b.tier.as_str(), "snack" | "meal" | "gift") {
        return Err(ApiError::bad("Pick snack, meal or gift for the stakes."));
    }
    let demand = line(&b.demand, 0, 60, "The demand")?;
    let demand = if demand.is_empty() { default_demand(&b.tier).to_string() } else { demand };
    let minutes = match b.preset.as_deref() {
        Some("custom") => b.minutes.filter(|m| (6..=20160).contains(m)),
        Some(p) => minutes_for_preset(p),
        None => None,
    }
    .ok_or_else(|| ApiError::bad("Pick a trial length between 6 minutes and 14 days."))?;

    let open: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cases WHERE plaintiff_id = ? AND phase != 'closed'")
        .bind(user.id)
        .fetch_one(&st.db)
        .await?;
    if open >= 10 {
        return Err(ApiError::new(StatusCode::TOO_MANY_REQUESTS, "You have 10 open cases. Let some finish first."));
    }

    let d = split_minutes(minutes);
    let id = new_case_id();
    let now = now_ms();
    let mut tx = st.db.begin().await?;
    sqlx::query(
        "INSERT INTO cases(id, plaintiff_id, defendant_label, title, charge, details, tier, demand, phase, \
         deadline_ms, summons_ms, args_ms, vote_ms, created_ms) VALUES(?,?,?,?,?,?,?,?,'summons',?,?,?,?,?)",
    )
    .bind(&id)
    .bind(user.id)
    .bind(&label)
    .bind(&title)
    .bind(&charge)
    .bind(&details)
    .bind(&b.tier)
    .bind(&demand)
    .bind(now + d.summons_ms)
    .bind(d.summons_ms)
    .bind(d.args_ms)
    .bind(d.vote_ms)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO participants(case_id, user_id, role, joined_ms) VALUES(?,?,'plaintiff',?)")
        .bind(&id)
        .bind(user.id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    for role in ["defendant", "plaintiff_lawyer", "defendant_lawyer", "jury"] {
        sqlx::query("INSERT INTO invites(token, case_id, role) VALUES(?,?,?)")
            .bind(random_token(16))
            .bind(&id)
            .bind(role)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({ "id": id })))
}

pub async fn list_cases(State(st): State<AppState>, user: AuthUser) -> ApiResult<Json<Value>> {
    let rows: Vec<(String, String, String, Option<String>, String, String)> = sqlx::query_as(
        "SELECT c.id, c.title, c.phase, c.outcome, p.role, c.defendant_label FROM participants p \
         JOIN cases c ON c.id = p.case_id WHERE p.user_id = ? ORDER BY c.created_ms DESC LIMIT 100",
    )
    .bind(user.id)
    .fetch_all(&st.db)
    .await?;
    let cases: Vec<Value> = rows
        .into_iter()
        .map(|(id, title, phase, outcome, role, defendant)| {
            json!({ "id": id, "title": title, "phase": phase, "outcome": outcome, "role": role, "defendant": defendant })
        })
        .collect();
    Ok(Json(json!({ "cases": cases })))
}

/// What an invite link shows before the visitor logs in. The link itself is the secret.
pub async fn peek_join(State(st): State<AppState>, Path(token): Path<String>) -> ApiResult<Json<Value>> {
    let row: Option<(String, String, String, String, String, Option<i64>)> = sqlx::query_as(
        "SELECT c.title, c.phase, c.defendant_label, u.username, i.role, i.claimed_by FROM invites i \
         JOIN cases c ON c.id = i.case_id JOIN users u ON u.id = c.plaintiff_id WHERE i.token = ?",
    )
    .bind(&token)
    .fetch_optional(&st.db)
    .await?;
    let (title, phase, defendant, plaintiff, role, claimed) =
        row.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "This invite link isn't valid."))?;
    Ok(Json(json!({
        "title": title, "phase": phase, "defendant": defendant, "plaintiff": plaintiff,
        "role": role, "taken": claimed.is_some(), "closed": phase == "closed",
    })))
}

async fn claim(conn: &mut SqliteConnection, token: &str, uid: i64) -> ApiResult<()> {
    let r = sqlx::query("UPDATE invites SET claimed_by = ? WHERE token = ? AND claimed_by IS NULL")
        .bind(uid)
        .bind(token)
        .execute(&mut *conn)
        .await?;
    if r.rows_affected() != 1 {
        return Err(ApiError::conflict("Someone already used this link. Ask for a fresh one."));
    }
    Ok(())
}

async fn seat(conn: &mut SqliteConnection, case_id: &str, uid: i64, role: Role, now: i64) -> ApiResult<()> {
    sqlx::query("INSERT INTO participants(case_id, user_id, role, joined_ms) VALUES(?,?,?,?)")
        .bind(case_id)
        .bind(uid)
        .bind(role.as_str())
        .bind(now)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

pub async fn join(State(st): State<AppState>, user: AuthUser, Path(token): Path<String>) -> ApiResult<Json<Value>> {
    let now = now_ms();
    let mut tx = st.db.begin().await?;
    let inv: Option<(String, String)> = sqlx::query_as("SELECT case_id, role FROM invites WHERE token = ?")
        .bind(&token)
        .fetch_optional(&mut *tx)
        .await?;
    let (case_id, role_s) = inv.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "This invite link isn't valid."))?;
    let c: CaseRow = sqlx::query_as("SELECT * FROM cases WHERE id = ?")
        .bind(&case_id)
        .fetch_one(&mut *tx)
        .await?;
    let already: Option<i64> = sqlx::query_scalar("SELECT 1 FROM participants WHERE case_id = ? AND user_id = ?")
        .bind(&case_id)
        .bind(user.id)
        .fetch_optional(&mut *tx)
        .await?;
    if already.is_some() {
        tx.commit().await?;
        return Ok(Json(json!({ "id": case_id })));
    }
    let phase = Phase::parse(&c.phase).unwrap_or(Phase::Closed);
    if phase == Phase::Closed {
        return Err(ApiError::conflict("This case is already closed."));
    }
    let role = Role::parse(&role_s).ok_or_else(|| ApiError::bad("This invite link isn't valid."))?;
    match role {
        Role::Defendant => {
            if phase != Phase::Summons {
                return Err(ApiError::conflict("The summons window has closed."));
            }
            claim(&mut tx, &token, user.id).await?;
            seat(&mut tx, &case_id, user.id, role, now).await?;
            let r = sqlx::query(
                "UPDATE cases SET defendant_id = ?, phase = 'arguments', deadline_ms = ? \
                 WHERE id = ? AND phase = 'summons' AND defendant_id IS NULL",
            )
            .bind(user.id)
            .bind(now + c.args_ms)
            .bind(&case_id)
            .execute(&mut *tx)
            .await?;
            if r.rows_affected() != 1 {
                return Err(ApiError::conflict("The summons was already answered."));
            }
        }
        Role::PlaintiffLawyer | Role::DefendantLawyer => {
            if !matches!(phase, Phase::Summons | Phase::Arguments) {
                return Err(ApiError::conflict("Lawyers have to be appointed before voting starts."));
            }
            claim(&mut tx, &token, user.id).await?;
            seat(&mut tx, &case_id, user.id, role, now).await?;
        }
        Role::Jury => seat(&mut tx, &case_id, user.id, role, now).await?,
        Role::Plaintiff => return Err(ApiError::bad("This invite link isn't valid.")),
    }
    tx.commit().await?;
    st.hub.notify(&case_id);
    Ok(Json(json!({ "id": case_id })))
}

#[derive(Deserialize)]
pub struct NewMessage {
    kind: String,
    #[serde(default)]
    body: String,
}

pub async fn post_message(
    State(st): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(b): Json<NewMessage>,
) -> ApiResult<Json<Value>> {
    let now = now_ms();
    let case = load_case(&st.db, &id).await?.ok_or_else(ApiError::not_found)?;
    let role = role_of(&st.db, &id, user.id).await?.ok_or_else(ApiError::not_found)?;
    let side = role
        .side()
        .ok_or_else(|| ApiError::forbidden("Jurors listen. Only the two sides and their lawyers argue."))?;
    if case.phase != "arguments" || now >= case.deadline_ms {
        return Err(ApiError::conflict("The floor is closed. Arguments aren't open right now."));
    }
    let body = match b.kind.as_str() {
        "statement" => text(&b.body, 1, 500, "A statement")?,
        "objection" => {
            let used: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM messages WHERE case_id = ? AND side = ? AND kind = 'objection'",
            )
            .bind(&id)
            .bind(side.as_str())
            .fetch_one(&st.db)
            .await?;
            if used >= 3 {
                return Err(ApiError::conflict("Your side has used all 3 objections."));
            }
            line(&b.body, 0, 140, "An objection")?
        }
        _ => return Err(ApiError::bad("Send a statement or an objection.")),
    };
    let recent: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE user_id = ? AND created_ms > ?")
        .bind(user.id)
        .bind(now - 10_000)
        .fetch_one(&st.db)
        .await?;
    if recent >= 6 {
        return Err(ApiError::new(StatusCode::TOO_MANY_REQUESTS, "Slow down. The court reporter can't keep up."));
    }
    sqlx::query("INSERT INTO messages(case_id, user_id, side, kind, body, created_ms) VALUES(?,?,?,?,?,?)")
        .bind(&id)
        .bind(user.id)
        .bind(side.as_str())
        .bind(&b.kind)
        .bind(&body)
        .bind(now)
        .execute(&st.db)
        .await?;
    st.hub.notify(&id);
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct VoteBody {
    vote: String,
}

pub async fn vote(
    State(st): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(b): Json<VoteBody>,
) -> ApiResult<Json<Value>> {
    if !matches!(b.vote.as_str(), "guilty" | "not_guilty") {
        return Err(ApiError::bad("Vote guilty or not guilty."));
    }
    let now = now_ms();
    let case = load_case(&st.db, &id).await?.ok_or_else(ApiError::not_found)?;
    let role = role_of(&st.db, &id, user.id).await?.ok_or_else(ApiError::not_found)?;
    if role != Role::Jury {
        return Err(ApiError::forbidden("Only jurors vote."));
    }
    if case.phase != "voting" || now >= case.deadline_ms {
        return Err(ApiError::conflict("Voting isn't open right now."));
    }
    sqlx::query(
        "INSERT INTO votes(case_id, user_id, vote) VALUES(?,?,?) \
         ON CONFLICT(case_id, user_id) DO UPDATE SET vote = excluded.vote",
    )
    .bind(&id)
    .bind(user.id)
    .bind(&b.vote)
    .execute(&st.db)
    .await?;
    engine::advance(&st.db, &id, now).await?;
    st.hub.notify(&id);
    Ok(Json(json!({ "ok": true })))
}

pub async fn mark_paid(State(st): State<AppState>, user: AuthUser, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let r = sqlx::query(
        "UPDATE cases SET sentence_paid = 1 WHERE id = ? AND creditor_id = ? AND phase = 'closed' AND sentence IS NOT NULL",
    )
    .bind(&id)
    .bind(user.id)
    .execute(&st.db)
    .await?;
    if r.rows_affected() != 1 {
        return Err(ApiError::forbidden("Only the person owed can mark a sentence as paid."));
    }
    st.hub.notify(&id);
    Ok(Json(json!({ "ok": true })))
}

pub async fn withdraw(State(st): State<AppState>, user: AuthUser, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let case = load_case(&st.db, &id).await?.ok_or_else(ApiError::not_found)?;
    if case.plaintiff_id != user.id {
        return Err(ApiError::forbidden("Only the person who filed can withdraw the case."));
    }
    if case.phase != "summons" {
        return Err(ApiError::conflict("You can only withdraw before the defendant answers."));
    }
    let done = engine::close_case(&st.db, &case, Phase::Summons, Outcome::Withdrawn, 0, 0, now_ms()).await?;
    if !done {
        return Err(ApiError::conflict("The case moved on before we could withdraw it."));
    }
    st.hub.notify(&id);
    Ok(Json(json!({ "ok": true })))
}

pub async fn get_case(State(st): State<AppState>, user: AuthUser, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    snapshot(&st.db, &id, user.id).await?.map(Json).ok_or_else(ApiError::not_found)
}

async fn username(db: &sqlx::SqlitePool, id: Option<i64>) -> Result<Option<String>, sqlx::Error> {
    match id {
        Some(i) => sqlx::query_scalar("SELECT username FROM users WHERE id = ?").bind(i).fetch_optional(db).await,
        None => Ok(None),
    }
}

/// Everything one viewer is allowed to see. Invite links go only to the people
/// who can hand them out; votes stay hidden until the verdict.
pub async fn snapshot(db: &sqlx::SqlitePool, id: &str, viewer: i64) -> Result<Option<Value>, sqlx::Error> {
    let Some(c) = load_case(db, id).await? else { return Ok(None) };
    let Some(role) = role_of(db, id, viewer).await? else { return Ok(None) };

    let plaintiff = username(db, Some(c.plaintiff_id)).await?.unwrap_or_default();
    let defendant = username(db, c.defendant_id).await?;
    let people: Vec<(String, String)> = sqlx::query_as(
        "SELECT u.username, p.role FROM participants p JOIN users u ON u.id = p.user_id \
         WHERE p.case_id = ? ORDER BY p.joined_ms",
    )
    .bind(id)
    .fetch_all(db)
    .await?;
    let lawyer = |r: &str| people.iter().find(|(_, role)| role == r).map(|(n, _)| n.clone());
    let jury: Vec<&String> = people.iter().filter(|(_, r)| r == "jury").map(|(n, _)| n).collect();
    let voted: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM votes WHERE case_id = ?").bind(id).fetch_one(db).await?;
    let my_vote: Option<String> = sqlx::query_scalar("SELECT vote FROM votes WHERE case_id = ? AND user_id = ?")
        .bind(id)
        .bind(viewer)
        .fetch_optional(db)
        .await?;

    let msgs: Vec<(i64, String, String, String, String, String, i64)> = sqlx::query_as(
        "SELECT m.id, m.side, m.kind, u.username, p.role, m.body, m.created_ms FROM messages m \
         JOIN users u ON u.id = m.user_id JOIN participants p ON p.case_id = m.case_id AND p.user_id = m.user_id \
         WHERE m.case_id = ? ORDER BY m.id LIMIT 500",
    )
    .bind(id)
    .fetch_all(db)
    .await?;
    let messages: Vec<Value> = msgs
        .into_iter()
        .map(|(mid, side, kind, user, role, body, at)| {
            json!({ "id": mid, "side": side, "kind": kind, "user": user, "lawyer": role.ends_with("lawyer"), "body": body, "at": at })
        })
        .collect();

    let wanted: &[&str] = match role {
        Role::Plaintiff => &["defendant", "plaintiff_lawyer", "jury"],
        Role::Defendant => &["defendant_lawyer", "jury"],
        _ => &["jury"],
    };
    let mut invites = serde_json::Map::new();
    if c.phase != "closed" {
        for r in wanted {
            let t: Option<(String, Option<i64>)> =
                sqlx::query_as("SELECT token, claimed_by FROM invites WHERE case_id = ? AND role = ?")
                    .bind(id)
                    .bind(*r)
                    .fetch_optional(db)
                    .await?;
            if let Some((token, claimed)) = t {
                if claimed.is_none() || *r == "jury" {
                    invites.insert((*r).to_string(), json!(token));
                }
            }
        }
    }

    let closed = c.phase == "closed";
    let sentence = c.sentence.as_ref().map(|s| {
        json!({ "text": s, "paid": c.sentence_paid == 1, "debtor": username_cache(&people, c.debtor_id, &c), "i_am_owed": c.creditor_id == Some(viewer) })
    });
    Ok(Some(json!({
        "id": c.id, "title": c.title, "charge": c.charge, "details": c.details,
        "tier": c.tier, "demand": c.demand, "phase": c.phase, "deadline_ms": c.deadline_ms,
        "now_ms": now_ms(), "outcome": c.outcome, "created_ms": c.created_ms,
        "summons_ms": c.summons_ms, "args_ms": c.args_ms, "vote_ms": c.vote_ms,
        "plaintiff": plaintiff, "defendant": defendant, "defendant_label": c.defendant_label,
        "plaintiff_lawyer": lawyer("plaintiff_lawyer"), "defendant_lawyer": lawyer("defendant_lawyer"),
        "jury": jury, "jury_voted": voted, "my_vote": my_vote,
        "tally": if closed { json!({ "guilty": c.guilty, "not_guilty": c.not_guilty }) } else { Value::Null },
        "sentence": sentence, "messages": messages, "invites": invites,
        "me": { "id": viewer, "role": role.as_str() },
    })))
}

fn username_cache(people: &[(String, String)], debtor: Option<i64>, c: &CaseRow) -> Option<String> {
    // The debtor is either the defendant or the plaintiff, both already in `people`.
    let want = if debtor == Some(c.plaintiff_id) { "plaintiff" } else { "defendant" };
    debtor?;
    people.iter().find(|(_, r)| r == want).map(|(n, _)| n.clone())
}
