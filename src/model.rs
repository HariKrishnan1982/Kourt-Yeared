use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use sqlx::{FromRow, SqlitePool};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MIN: i64 = 60_000;
pub const FALSE_ALARM: &str = "a chai or snack as the false-alarm fee";

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn random_token(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut b);
    URL_SAFE_NO_PAD.encode(b)
}

pub fn new_case_id() -> String {
    const A: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut b = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut b);
    b.iter().map(|x| A[*x as usize % A.len()] as char).collect()
}

pub fn default_demand(tier: &str) -> &'static str {
    match tier {
        "meal" => "a proper meal",
        "gift" => "a small gift",
        _ => "a snack of the plaintiff's choice",
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Summons,
    Arguments,
    Voting,
    Closed,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Summons => "summons",
            Phase::Arguments => "arguments",
            Phase::Voting => "voting",
            Phase::Closed => "closed",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "summons" => Phase::Summons,
            "arguments" => Phase::Arguments,
            "voting" => Phase::Voting,
            "closed" => Phase::Closed,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Plaintiff,
    Defendant,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Plaintiff => "plaintiff",
            Side::Defendant => "defendant",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    Guilty,
    NotGuilty,
    DefaultJudgment,
    Dismissed,
    Mistrial,
    Abandoned,
    Withdrawn,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Guilty => "guilty",
            Outcome::NotGuilty => "not_guilty",
            Outcome::DefaultJudgment => "default_judgment",
            Outcome::Dismissed => "dismissed",
            Outcome::Mistrial => "mistrial",
            Outcome::Abandoned => "abandoned",
            Outcome::Withdrawn => "withdrawn",
        }
    }
    /// Which side owes the sentence, if anyone does.
    pub fn debtor_side(self) -> Option<Side> {
        match self {
            Outcome::Guilty | Outcome::DefaultJudgment => Some(Side::Defendant),
            Outcome::NotGuilty | Outcome::Dismissed => Some(Side::Plaintiff),
            Outcome::Mistrial | Outcome::Abandoned | Outcome::Withdrawn => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Plaintiff,
    Defendant,
    PlaintiffLawyer,
    DefendantLawyer,
    Jury,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Plaintiff => "plaintiff",
            Role::Defendant => "defendant",
            Role::PlaintiffLawyer => "plaintiff_lawyer",
            Role::DefendantLawyer => "defendant_lawyer",
            Role::Jury => "jury",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "plaintiff" => Role::Plaintiff,
            "defendant" => Role::Defendant,
            "plaintiff_lawyer" => Role::PlaintiffLawyer,
            "defendant_lawyer" => Role::DefendantLawyer,
            "jury" => Role::Jury,
            _ => return None,
        })
    }
    pub fn side(self) -> Option<Side> {
        match self {
            Role::Plaintiff | Role::PlaintiffLawyer => Some(Side::Plaintiff),
            Role::Defendant | Role::DefendantLawyer => Some(Side::Defendant),
            Role::Jury => None,
        }
    }
}

pub struct Durations {
    pub summons_ms: i64,
    pub args_ms: i64,
    pub vote_ms: i64,
}

/// Splits a trial length into its three timed phases: a quarter to answer the
/// summons (at least 2 min), half for arguments, the rest (at least 1 min) to vote.
pub fn split_minutes(total_min: i64) -> Durations {
    let total = total_min * MIN;
    let summons_ms = (total / 4).max(2 * MIN);
    let args_ms = total / 2;
    let vote_ms = (total - summons_ms - args_ms).max(MIN);
    Durations {
        summons_ms,
        args_ms,
        vote_ms,
    }
}

pub fn minutes_for_preset(p: &str) -> Option<i64> {
    match p {
        "quick" => Some(12),
        "standard" => Some(180),
        "marathon" => Some(4320),
        _ => None,
    }
}

#[derive(Clone, Debug, FromRow)]
pub struct CaseRow {
    pub id: String,
    pub plaintiff_id: i64,
    pub defendant_id: Option<i64>,
    pub defendant_label: String,
    pub title: String,
    pub charge: String,
    pub details: String,
    pub tier: String,
    pub demand: String,
    pub phase: String,
    pub deadline_ms: i64,
    pub summons_ms: i64,
    pub args_ms: i64,
    pub vote_ms: i64,
    pub outcome: Option<String>,
    pub debtor_id: Option<i64>,
    pub creditor_id: Option<i64>,
    pub sentence: Option<String>,
    pub sentence_paid: i64,
    pub guilty: i64,
    pub not_guilty: i64,
    pub created_ms: i64,
}

pub async fn load_case(db: &SqlitePool, id: &str) -> Result<Option<CaseRow>, sqlx::Error> {
    sqlx::query_as::<_, CaseRow>("SELECT * FROM cases WHERE id = ?")
        .bind(id)
        .fetch_optional(db)
        .await
}

pub async fn role_of(db: &SqlitePool, case_id: &str, user_id: i64) -> Result<Option<Role>, sqlx::Error> {
    let r: Option<String> =
        sqlx::query_scalar("SELECT role FROM participants WHERE case_id = ? AND user_id = ?")
            .bind(case_id)
            .bind(user_id)
            .fetch_optional(db)
            .await?;
    Ok(r.and_then(|s| Role::parse(&s)))
}
