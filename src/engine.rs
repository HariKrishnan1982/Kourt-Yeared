//! The case state machine. `decide` is pure (easy to test); `advance` applies
//! its answer to the database using compare-and-set updates, so two racing
//! callers can never close or advance the same case twice.

use crate::model::*;
use sqlx::SqlitePool;

pub struct Facts {
    pub phase: Phase,
    pub deadline_ms: i64,
    pub args_ms: i64,
    pub vote_ms: i64,
    pub defendant_joined: bool,
    pub stmts_plaintiff: i64,
    pub stmts_defendant: i64,
    pub jurors: i64,
    pub votes_guilty: i64,
    pub votes_not_guilty: i64,
}

#[derive(Debug, PartialEq)]
pub enum Transition {
    Stay,
    ToArguments { deadline_ms: i64 },
    ToVoting { deadline_ms: i64 },
    Close(Outcome),
}

pub fn decide(f: &Facts, now: i64) -> Transition {
    match f.phase {
        Phase::Summons => {
            if f.defendant_joined {
                Transition::ToArguments { deadline_ms: now + f.args_ms }
            } else if now >= f.deadline_ms {
                Transition::Close(Outcome::DefaultJudgment)
            } else {
                Transition::Stay
            }
        }
        Phase::Arguments => {
            if now < f.deadline_ms {
                return Transition::Stay;
            }
            match (f.stmts_plaintiff > 0, f.stmts_defendant > 0) {
                (true, true) => Transition::ToVoting { deadline_ms: now + f.vote_ms },
                (true, false) => Transition::Close(Outcome::DefaultJudgment),
                (false, true) => Transition::Close(Outcome::Dismissed),
                (false, false) => Transition::Close(Outcome::Abandoned),
            }
        }
        Phase::Voting => {
            let everyone_voted =
                f.jurors > 0 && f.votes_guilty + f.votes_not_guilty >= f.jurors;
            if now >= f.deadline_ms || everyone_voted {
                Transition::Close(tally(f.votes_guilty, f.votes_not_guilty))
            } else {
                Transition::Stay
            }
        }
        Phase::Closed => Transition::Stay,
    }
}

pub fn tally(guilty: i64, not_guilty: i64) -> Outcome {
    if guilty > not_guilty {
        Outcome::Guilty
    } else if not_guilty > guilty {
        Outcome::NotGuilty
    } else {
        Outcome::Mistrial
    }
}

async fn count(db: &SqlitePool, sql: &str, a: &str, b: Option<&str>) -> Result<i64, sqlx::Error> {
    let q = sqlx::query_scalar::<_, i64>(sql).bind(a);
    let q = if let Some(b) = b { q.bind(b) } else { q };
    q.fetch_one(db).await
}

async fn load_facts(db: &SqlitePool, c: &CaseRow, phase: Phase) -> Result<Facts, sqlx::Error> {
    let stmt = "SELECT COUNT(*) FROM messages WHERE case_id = ? AND kind = 'statement' AND side = ?";
    Ok(Facts {
        phase,
        deadline_ms: c.deadline_ms,
        args_ms: c.args_ms,
        vote_ms: c.vote_ms,
        defendant_joined: c.defendant_id.is_some(),
        stmts_plaintiff: count(db, stmt, &c.id, Some("plaintiff")).await?,
        stmts_defendant: count(db, stmt, &c.id, Some("defendant")).await?,
        jurors: count(db, "SELECT COUNT(*) FROM participants WHERE case_id = ? AND role = 'jury'", &c.id, None).await?,
        votes_guilty: count(db, "SELECT COUNT(*) FROM votes WHERE case_id = ? AND vote = ?", &c.id, Some("guilty")).await?,
        votes_not_guilty: count(db, "SELECT COUNT(*) FROM votes WHERE case_id = ? AND vote = ?", &c.id, Some("not_guilty")).await?,
    })
}

async fn cas_phase(db: &SqlitePool, id: &str, from: Phase, to: Phase, deadline_ms: i64) -> Result<bool, sqlx::Error> {
    let r = sqlx::query("UPDATE cases SET phase = ?, deadline_ms = ? WHERE id = ? AND phase = ?")
        .bind(to.as_str())
        .bind(deadline_ms)
        .bind(id)
        .bind(from.as_str())
        .execute(db)
        .await?;
    Ok(r.rows_affected() == 1)
}

pub async fn close_case(
    db: &SqlitePool,
    c: &CaseRow,
    from: Phase,
    outcome: Outcome,
    guilty: i64,
    not_guilty: i64,
    now: i64,
) -> Result<bool, sqlx::Error> {
    let (debtor, creditor, sentence) = match outcome.debtor_side() {
        Some(Side::Defendant) => (c.defendant_id, Some(c.plaintiff_id), Some(c.demand.clone())),
        Some(Side::Plaintiff) => (Some(c.plaintiff_id), c.defendant_id, Some(FALSE_ALARM.to_string())),
        None => (None, None, None),
    };
    let r = sqlx::query(
        "UPDATE cases SET phase = 'closed', outcome = ?, debtor_id = ?, creditor_id = ?, \
         sentence = ?, guilty = ?, not_guilty = ?, deadline_ms = ? WHERE id = ? AND phase = ?",
    )
    .bind(outcome.as_str())
    .bind(debtor)
    .bind(creditor)
    .bind(sentence)
    .bind(guilty)
    .bind(not_guilty)
    .bind(now)
    .bind(&c.id)
    .bind(from.as_str())
    .execute(db)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// Moves a case forward as far as the rules allow right now. Returns whether
/// anything changed (so the caller knows to tell the room).
pub async fn advance(db: &SqlitePool, case_id: &str, now: i64) -> Result<bool, sqlx::Error> {
    let mut changed = false;
    for _ in 0..4 {
        let Some(c) = load_case(db, case_id).await? else { break };
        let phase = Phase::parse(&c.phase).unwrap_or(Phase::Closed);
        if phase == Phase::Closed {
            break;
        }
        let facts = load_facts(db, &c, phase).await?;
        match decide(&facts, now) {
            Transition::Stay => break,
            Transition::ToArguments { deadline_ms } => {
                if !cas_phase(db, &c.id, phase, Phase::Arguments, deadline_ms).await? {
                    break;
                }
                changed = true;
            }
            Transition::ToVoting { deadline_ms } => {
                if !cas_phase(db, &c.id, phase, Phase::Voting, deadline_ms).await? {
                    break;
                }
                changed = true;
            }
            Transition::Close(o) => {
                if close_case(db, &c, phase, o, facts.votes_guilty, facts.votes_not_guilty, now).await? {
                    changed = true;
                }
                break;
            }
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(phase: Phase, deadline: i64) -> Facts {
        Facts {
            phase,
            deadline_ms: deadline,
            args_ms: 6 * MIN,
            vote_ms: 3 * MIN,
            defendant_joined: false,
            stmts_plaintiff: 0,
            stmts_defendant: 0,
            jurors: 0,
            votes_guilty: 0,
            votes_not_guilty: 0,
        }
    }

    #[test]
    fn summons_waits_then_defaults() {
        let f = facts(Phase::Summons, 1000);
        assert_eq!(decide(&f, 999), Transition::Stay);
        assert_eq!(decide(&f, 1000), Transition::Close(Outcome::DefaultJudgment));
    }

    #[test]
    fn defendant_joining_opens_arguments() {
        let mut f = facts(Phase::Summons, 1000);
        f.defendant_joined = true;
        assert_eq!(decide(&f, 10), Transition::ToArguments { deadline_ms: 10 + 6 * MIN });
    }

    #[test]
    fn arguments_end_by_who_spoke() {
        let mut f = facts(Phase::Arguments, 500);
        assert_eq!(decide(&f, 499), Transition::Stay);
        assert_eq!(decide(&f, 500), Transition::Close(Outcome::Abandoned));
        f.stmts_plaintiff = 1;
        assert_eq!(decide(&f, 500), Transition::Close(Outcome::DefaultJudgment));
        f.stmts_plaintiff = 0;
        f.stmts_defendant = 2;
        assert_eq!(decide(&f, 500), Transition::Close(Outcome::Dismissed));
        f.stmts_plaintiff = 1;
        assert_eq!(decide(&f, 500), Transition::ToVoting { deadline_ms: 500 + 3 * MIN });
    }

    #[test]
    fn voting_tallies_and_ends_early_when_everyone_voted() {
        let mut f = facts(Phase::Voting, 9000);
        assert_eq!(decide(&f, 100), Transition::Stay);
        f.jurors = 3;
        f.votes_guilty = 2;
        assert_eq!(decide(&f, 100), Transition::Stay);
        f.votes_not_guilty = 1;
        assert_eq!(decide(&f, 100), Transition::Close(Outcome::Guilty));
        f.votes_guilty = 1;
        f.votes_not_guilty = 1;
        assert_eq!(decide(&f, 9000), Transition::Close(Outcome::Mistrial));
        f.votes_guilty = 0;
        f.votes_not_guilty = 2;
        assert_eq!(decide(&f, 9000), Transition::Close(Outcome::NotGuilty));
    }

    #[test]
    fn empty_jury_is_a_mistrial_not_a_hang() {
        let f = facts(Phase::Voting, 100);
        assert_eq!(decide(&f, 99), Transition::Stay);
        assert_eq!(decide(&f, 100), Transition::Close(Outcome::Mistrial));
    }

    #[test]
    fn closed_never_moves() {
        let f = facts(Phase::Closed, 0);
        assert_eq!(decide(&f, i64::MAX), Transition::Stay);
    }

    #[test]
    fn durations_add_up_and_respect_minimums() {
        for m in [6, 12, 180, 4320, 20160] {
            let d = split_minutes(m);
            assert!(d.summons_ms >= 2 * MIN && d.vote_ms >= MIN && d.args_ms > 0);
            assert_eq!(d.summons_ms + d.args_ms + d.vote_ms, m * MIN);
        }
        let q = split_minutes(12);
        assert_eq!((q.summons_ms, q.args_ms, q.vote_ms), (3 * MIN, 6 * MIN, 3 * MIN));
    }

    #[test]
    fn who_owes_what() {
        assert_eq!(Outcome::Guilty.debtor_side(), Some(Side::Defendant));
        assert_eq!(Outcome::DefaultJudgment.debtor_side(), Some(Side::Defendant));
        assert_eq!(Outcome::NotGuilty.debtor_side(), Some(Side::Plaintiff));
        assert_eq!(Outcome::Dismissed.debtor_side(), Some(Side::Plaintiff));
        assert_eq!(Outcome::Mistrial.debtor_side(), None);
        assert_eq!(Outcome::Withdrawn.debtor_side(), None);
    }
}
