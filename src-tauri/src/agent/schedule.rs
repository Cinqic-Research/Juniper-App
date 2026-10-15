//! Scheduled and recurring work. Schedules store a rule, an IANA time zone,
//! and the next occurrence as a UTC instant. Occurrences run while Juniper
//! is running: the worker claims a due occurrence atomically (compare and
//! swap on `next_run_at`), records it under a unique (schedule, occurrence,
//! attempt) key, and computes the next occurrence from the current time, so
//! a backlog missed while Juniper was closed is never replayed — the
//! missed-run policy decides whether the latest one runs once or is skipped.

use super::store::{self, db_error};
use super::types::TaskStatus;
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Runtime};

/// How late an occurrence may start and still count as on time.
const GRACE_MINUTES: i64 = 10;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Rule {
    /// `at` is a local date and time, `YYYY-MM-DDTHH:MM`.
    Once {
        at: String,
    },
    Daily {
        time: String,
    },
    /// Days are 1 (Monday) to 7 (Sunday).
    Weekly {
        days: Vec<u8>,
        time: String,
    },
    /// A day of the month; months without that day use their last day.
    Monthly {
        day: u8,
        time: String,
    },
    Interval {
        minutes: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleSpec {
    pub title: String,
    /// `agent-task` runs the prompt as a task; `reminder` only notifies and
    /// never loads a model.
    pub action: String,
    pub prompt: String,
    #[serde(default)]
    pub assistant_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    pub timezone: String,
    pub rule: Rule,
    /// `run-once` (the latest missed occurrence runs at the next launch) or
    /// `skip`.
    pub missed_policy: String,
    /// Retries of a failed occurrence; only when it changed nothing.
    #[serde(default)]
    pub retry_limit: u32,
    /// The anchor for interval rules (UTC, RFC 3339).
    #[serde(default)]
    pub anchor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    pub id: String,
    pub spec: ScheduleSpec,
    pub enabled: bool,
    pub next_run_at: Option<String>,
    pub upcoming: Vec<String>,
    pub revision: i64,
    pub created_at: String,
    pub updated_at: String,
}

fn parse_time(value: &str) -> Result<NaiveTime, String> {
    NaiveTime::parse_from_str(value, "%H:%M")
        .map_err(|_| "INVALID_SCHEDULE: Use a 24-hour time like 07:30.".into())
}

/// The UTC instant for a local wall-clock time. In a DST fold the first
/// occurrence is used (it runs once); a time skipped by a DST gap runs at
/// the first valid minute after the gap.
pub fn resolve_local(zone: Tz, date: NaiveDate, time: NaiveTime) -> DateTime<Utc> {
    let mut local = NaiveDateTime::new(date, time);
    for _ in 0..180 {
        match zone.from_local_datetime(&local) {
            chrono::LocalResult::Single(instant) => return instant.with_timezone(&Utc),
            chrono::LocalResult::Ambiguous(earliest, _) => return earliest.with_timezone(&Utc),
            chrono::LocalResult::None => local += Duration::minutes(1),
        }
    }
    Utc.from_utc_datetime(&local)
}

fn last_day(year: i32, month: u32) -> u32 {
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    };
    next.and_then(|date| date.pred_opt())
        .map(|date| date.day())
        .unwrap_or(28)
}

pub fn validate(spec: &ScheduleSpec) -> Result<Tz, String> {
    let zone: Tz = spec
        .timezone
        .parse()
        .map_err(|_| "INVALID_SCHEDULE: Unknown time zone.".to_owned())?;
    if spec.title.trim().is_empty() || spec.title.chars().count() > 100 {
        return Err("INVALID_SCHEDULE: Give the schedule a short title.".into());
    }
    if !matches!(spec.action.as_str(), "agent-task" | "reminder") {
        return Err("INVALID_SCHEDULE: Unknown action.".into());
    }
    if spec.prompt.trim().is_empty() || spec.prompt.chars().count() > 8000 {
        return Err("INVALID_SCHEDULE: Describe what should happen.".into());
    }
    if !matches!(spec.missed_policy.as_str(), "run-once" | "skip") || spec.retry_limit > 3 {
        return Err("INVALID_SCHEDULE: Unknown missed-run policy or too many retries.".into());
    }
    match &spec.rule {
        Rule::Once { at } => {
            NaiveDateTime::parse_from_str(at, "%Y-%m-%dT%H:%M").map_err(|_| {
                "INVALID_SCHEDULE: Use a date and time like 2026-10-20T09:00.".to_owned()
            })?;
        }
        Rule::Daily { time } => {
            parse_time(time)?;
        }
        Rule::Weekly { days, time } => {
            parse_time(time)?;
            if days.is_empty() || days.iter().any(|day| !(1..=7).contains(day)) {
                return Err("INVALID_SCHEDULE: Choose at least one weekday.".into());
            }
        }
        Rule::Monthly { day, time } => {
            parse_time(time)?;
            if !(1..=31).contains(day) {
                return Err("INVALID_SCHEDULE: Choose a day from 1 to 31.".into());
            }
        }
        Rule::Interval { minutes } => {
            if !(15..=7 * 24 * 60).contains(minutes) {
                return Err(
                    "INVALID_SCHEDULE: Intervals run every 15 minutes to every 7 days.".into(),
                );
            }
        }
    }
    Ok(zone)
}

/// The first occurrence strictly after `after`.
pub fn next_occurrence(spec: &ScheduleSpec, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let zone: Tz = spec.timezone.parse().ok()?;
    let local_date = after.with_timezone(&zone).date_naive();
    match &spec.rule {
        Rule::Once { at } => {
            let local = NaiveDateTime::parse_from_str(at, "%Y-%m-%dT%H:%M").ok()?;
            let instant = resolve_local(zone, local.date(), local.time());
            (instant > after).then_some(instant)
        }
        Rule::Daily { time } => {
            let time = parse_time(time).ok()?;
            (0..3)
                .map(|offset| resolve_local(zone, local_date + Duration::days(offset), time))
                .find(|instant| *instant > after)
        }
        Rule::Weekly { days, time } => {
            let time = parse_time(time).ok()?;
            (0..15)
                .map(|offset| local_date + Duration::days(offset))
                .filter(|date| days.contains(&(date.weekday().number_from_monday() as u8)))
                .map(|date| resolve_local(zone, date, time))
                .find(|instant| *instant > after)
        }
        Rule::Monthly { day, time } => {
            let time = parse_time(time).ok()?;
            let (mut year, mut month) = (local_date.year(), local_date.month());
            for _ in 0..14 {
                let date = NaiveDate::from_ymd_opt(
                    year,
                    month,
                    u32::from(*day).min(last_day(year, month)),
                )?;
                let instant = resolve_local(zone, date, time);
                if instant > after {
                    return Some(instant);
                }
                (year, month) = if month == 12 {
                    (year + 1, 1)
                } else {
                    (year, month + 1)
                };
            }
            None
        }
        Rule::Interval { minutes } => {
            let anchor = spec
                .anchor
                .as_deref()
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&Utc))
                .unwrap_or(after);
            let step = Duration::minutes(i64::from(*minutes));
            if anchor > after {
                return Some(anchor);
            }
            let elapsed = (after - anchor).num_seconds() / step.num_seconds() + 1;
            Some(anchor + step * elapsed as i32)
        }
    }
}

/// The latest occurrence at or before `at`, constrained to the claimed
/// occurrence. The schedule row stores the next due time, so this is used to
/// collapse a missed backlog into its most recent occurrence.
fn latest_due_occurrence(
    spec: &ScheduleSpec,
    at: DateTime<Utc>,
    first_due: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let zone: Tz = spec.timezone.parse().ok()?;
    if let Rule::Interval { minutes } = &spec.rule {
        let anchor = spec
            .anchor
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc))
            .unwrap_or(first_due);
        let step = Duration::minutes(i64::from(*minutes));
        if anchor > at {
            return None;
        }
        let elapsed = (at - anchor).num_seconds() / step.num_seconds();
        let occurrence = anchor.checked_add_signed(step * elapsed as i32)?;
        return (occurrence >= first_due).then_some(occurrence);
    }
    let local_date = at.with_timezone(&zone).date_naive();
    let candidate = match &spec.rule {
        Rule::Once { at } => {
            let local = NaiveDateTime::parse_from_str(at, "%Y-%m-%dT%H:%M").ok()?;
            Some(resolve_local(zone, local.date(), local.time()))
        }
        Rule::Daily { time } => {
            let time = parse_time(time).ok()?;
            (0..=1)
                .map(|offset| resolve_local(zone, local_date - Duration::days(offset), time))
                .find(|instant| *instant <= at)
        }
        Rule::Weekly { days, time } => {
            let time = parse_time(time).ok()?;
            (0..=7)
                .map(|offset| local_date - Duration::days(offset))
                .filter(|date| days.contains(&(date.weekday().number_from_monday() as u8)))
                .map(|date| resolve_local(zone, date, time))
                .find(|instant| *instant <= at)
        }
        Rule::Monthly { day, time } => {
            let time = parse_time(time).ok()?;
            let (mut year, mut month) = (local_date.year(), local_date.month());
            (0..=1).find_map(|_| {
                let date = NaiveDate::from_ymd_opt(
                    year,
                    month,
                    u32::from(*day).min(last_day(year, month)),
                )?;
                let occurrence = resolve_local(zone, date, time);
                if occurrence <= at {
                    Some(occurrence)
                } else {
                    (year, month) = if month == 1 {
                        (year - 1, 12)
                    } else {
                        (year, month - 1)
                    };
                    None
                }
            })
        }
        Rule::Interval { .. } => unreachable!(),
    }?;
    (candidate >= first_due).then_some(candidate)
}

pub fn upcoming(spec: &ScheduleSpec, from: DateTime<Utc>, count: usize) -> Vec<DateTime<Utc>> {
    let mut found = Vec::new();
    let mut cursor = from;
    while found.len() < count {
        let Some(next) = next_occurrence(spec, cursor) else {
            break;
        };
        found.push(next);
        cursor = next;
    }
    found
}

/// A user-facing rendering of an instant in the schedule's zone.
pub fn local_label(instant: DateTime<Utc>, zone: &str) -> String {
    zone.parse::<Tz>()
        .map(|zone| {
            instant
                .with_timezone(&zone)
                .format("%a %b %-d, %Y, %-I:%M %p %Z")
                .to_string()
        })
        .unwrap_or_else(|_| instant.to_rfc3339())
}

/// id, payload, enabled, next_run_at, revision, created_at, updated_at
type ScheduleRow = (String, String, bool, Option<String>, i64, String, String);

fn schedule_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScheduleRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
    ))
}

pub fn list(connection: &Connection) -> Result<Vec<Schedule>, String> {
    let mut statement = connection
        .prepare("SELECT id, payload, enabled, next_run_at, revision, created_at, updated_at FROM schedules ORDER BY created_at")
        .map_err(db_error)?;
    let rows = statement
        .query_map([], schedule_from_row)
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)?;
    let now = Utc::now();
    rows.into_iter()
        .map(
            |(id, payload, enabled, next_run_at, revision, created_at, updated_at)| {
                let spec: ScheduleSpec = serde_json::from_str(&payload).map_err(db_error)?;
                let upcoming = if enabled {
                    upcoming(&spec, now, 5)
                        .into_iter()
                        .map(|instant| local_label(instant, &spec.timezone))
                        .collect()
                } else {
                    Vec::new()
                };
                Ok(Schedule {
                    id,
                    spec,
                    enabled,
                    next_run_at,
                    upcoming,
                    revision,
                    created_at,
                    updated_at,
                })
            },
        )
        .collect()
}

pub fn get(connection: &Connection, id: &str) -> Result<Schedule, String> {
    list(connection)?
        .into_iter()
        .find(|schedule| schedule.id == id)
        .ok_or_else(|| "SCHEDULE_NOT_FOUND: That schedule no longer exists.".into())
}

/// Creates or updates a schedule. Editing recomputes the next occurrence
/// from now; a disabled schedule keeps none.
pub fn save(
    connection: &Connection,
    id: Option<&str>,
    mut spec: ScheduleSpec,
    enabled: bool,
    expected_revision: i64,
    now: DateTime<Utc>,
) -> Result<Schedule, String> {
    validate(&spec)?;
    if matches!(spec.rule, Rule::Interval { .. }) && spec.anchor.is_none() {
        spec.anchor = Some((now + Duration::minutes(1)).to_rfc3339());
    }
    let next = enabled
        .then(|| next_occurrence(&spec, now))
        .flatten()
        .map(|instant| instant.to_rfc3339());
    let payload = serde_json::to_string(&spec).map_err(db_error)?;
    let id = match id {
        Some(id) => {
            let changed = connection
                .execute(
                    "UPDATE schedules SET payload = ?1, title = ?2, assistant_id = ?3, project_id = ?4, enabled = ?5, next_run_at = ?6, revision = revision + 1, updated_at = ?7 WHERE id = ?8 AND revision = ?9",
                    params![payload, spec.title, spec.assistant_id, spec.project_id, enabled, next, store::now(), id, expected_revision],
                )
                .map_err(db_error)?;
            if changed != 1 {
                return Err("SCHEDULE_REVISION_CONFLICT: This schedule changed elsewhere. Reload and try again.".into());
            }
            id.to_owned()
        }
        None => {
            let id = store::new_id("schedule");
            connection
                .execute(
                    "INSERT INTO schedules(id, assistant_id, project_id, title, payload, enabled, next_run_at, revision, created_at, updated_at) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?8)",
                    params![id, spec.assistant_id, spec.project_id, spec.title, payload, enabled, next, store::now()],
                )
                .map_err(db_error)?;
            id
        }
    };
    get(connection, &id)
}

pub fn delete(connection: &Connection, id: &str) -> Result<(), String> {
    connection
        .execute("DELETE FROM schedules WHERE id = ?1", [id])
        .map_err(db_error)?;
    Ok(())
}

pub fn runs(connection: &Connection, schedule_id: &str) -> Result<Vec<Value>, String> {
    let mut statement = connection
        .prepare("SELECT id, scheduled_for, attempt, status, task_id, detail, created_at FROM schedule_runs WHERE schedule_id = ?1 ORDER BY created_at DESC LIMIT 50")
        .map_err(db_error)?;
    statement
        .query_map([schedule_id], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "scheduledFor": row.get::<_, String>(1)?,
                "attempt": row.get::<_, i64>(2)?,
                "status": row.get::<_, String>(3)?,
                "taskId": row.get::<_, Option<String>>(4)?,
                "detail": row.get::<_, Option<String>>(5)?,
                "createdAt": row.get::<_, String>(6)?,
            }))
        })
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)
}

/// What one claimed occurrence should do.
#[derive(Debug, PartialEq)]
pub enum Claim {
    Run {
        run_id: String,
        scheduled_for: DateTime<Utc>,
    },
    Skipped {
        reason: &'static str,
    },
}

/// Claims every due occurrence at `now` exactly once, advancing each
/// schedule. Returns what was claimed, for the caller to act on.
pub fn claim_due(
    connection: &Connection,
    now: DateTime<Utc>,
) -> Result<Vec<(Schedule, Claim)>, String> {
    let due: Vec<(String, String)> = {
        let mut statement = connection
            .prepare("SELECT id, next_run_at FROM schedules WHERE enabled = 1 AND next_run_at IS NOT NULL")
            .map_err(db_error)?;
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?
    };
    let mut claimed = Vec::new();
    for (id, next_run_at) in due {
        let Ok(scheduled) =
            DateTime::parse_from_rfc3339(&next_run_at).map(|value| value.with_timezone(&Utc))
        else {
            continue;
        };
        if scheduled > now {
            continue;
        }
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        let schedule = get(&transaction, &id)?;
        let following = next_occurrence(&schedule.spec, now).map(|instant| instant.to_rfc3339());
        // Compare-and-swap: only the claimant that still sees the old value
        // advances the schedule, so an occurrence is claimed once.
        let changed = transaction
            .execute(
                "UPDATE schedules SET next_run_at = ?1, updated_at = ?2 WHERE id = ?3 AND next_run_at = ?4",
                params![following, store::now(), id, next_run_at],
            )
            .map_err(db_error)?;
        if changed != 1 {
            transaction.rollback().map_err(db_error)?;
            continue;
        }
        let missed = now - scheduled > Duration::minutes(GRACE_MINUTES);
        let occurrence = if missed && schedule.spec.missed_policy == "run-once" {
            latest_due_occurrence(&schedule.spec, now, scheduled).unwrap_or(scheduled)
        } else {
            scheduled
        };
        let overlapping: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM schedule_runs r JOIN tasks t ON t.id = r.task_id WHERE r.schedule_id = ?1 AND t.status NOT IN ('completed', 'failed', 'cancelled'))",
                [&id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        let (status, claim) = if missed && schedule.spec.missed_policy == "skip" {
            (
                "skipped-missed",
                Claim::Skipped {
                    reason: "Juniper was not running at the scheduled time.",
                },
            )
        } else if overlapping {
            (
                "skipped-overlap",
                Claim::Skipped {
                    reason: "The previous run had not finished.",
                },
            )
        } else {
            (
                "started",
                Claim::Run {
                    run_id: String::new(),
                    scheduled_for: scheduled,
                },
            )
        };
        let run_id = store::new_id("run");
        let inserted = transaction
            .execute(
                "INSERT OR IGNORE INTO schedule_runs(id, schedule_id, scheduled_for, attempt, status, created_at, updated_at, detail) VALUES(?1, ?2, ?3, 1, ?4, ?5, ?5, ?6)",
                params![run_id, id, occurrence.to_rfc3339(), status, store::now(), missed.then_some("Ran late: Juniper was not running at the scheduled time.")],
            )
            .map_err(db_error)?;
        if inserted != 1 {
            transaction.commit().map_err(db_error)?;
            continue;
        }
        transaction.commit().map_err(db_error)?;
        let claim = match claim {
            Claim::Run { scheduled_for, .. } => Claim::Run {
                run_id,
                scheduled_for: if missed { occurrence } else { scheduled_for },
            },
            skipped => skipped,
        };
        claimed.push((schedule, claim));
    }
    Ok(claimed)
}

/// Runs claimed occurrences. Called by the agent worker.
pub fn tick<R: Runtime>(app: &AppHandle<R>, connection: &Connection, now: DateTime<Utc>) {
    use tauri::Manager;
    let state = app.state::<crate::commands::AppState>();
    if state.agent.is_stopped() {
        return;
    }
    let _ = reconcile_terminal_runs(connection);
    let Ok(claimed) = claim_due(connection, now) else {
        return;
    };
    for (schedule, claim) in claimed {
        match claim {
            Claim::Skipped { reason } => {
                let _ = store::add_inbox(
                    connection,
                    None,
                    "schedule",
                    &format!("Skipped: {}", schedule.spec.title),
                    reason,
                );
            }
            Claim::Run { .. } => {}
        }
        super::notify(app, None, "schedule");
    }
    if dispatch_pending(connection).unwrap_or(0) > 0 {
        state.agent.wake.notify_one();
    }
}

/// Replays committed schedule claims that did not finish dispatching before
/// a crash. `start_run` is idempotent for both agent tasks and reminders.
fn dispatch_pending(connection: &Connection) -> Result<usize, String> {
    let pending = {
        let mut statement = connection
            .prepare("SELECT id, schedule_id, scheduled_for FROM schedule_runs WHERE status = 'started' AND task_id IS NULL ORDER BY created_at")
            .map_err(db_error)?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?
    };
    let mut dispatched = 0;
    for (run_id, schedule_id, scheduled_for) in pending {
        let schedule = get(connection, &schedule_id)?;
        let scheduled_for = DateTime::parse_from_rfc3339(
            scheduled_for
                .strip_prefix("manual:")
                .unwrap_or(&scheduled_for),
        )
        .map_err(|_| "SCHEDULE_RUN_INVALID: A pending occurrence has an invalid time.".to_owned())?
        .with_timezone(&Utc);
        start_run(connection, &schedule, &run_id, scheduled_for)?;
        dispatched += 1;
    }
    Ok(dispatched)
}

/// Starts one claimed run: a deterministic reminder goes straight to the
/// inbox; an agent task is created under the agent's current permissions.
pub fn start_run(
    connection: &Connection,
    schedule: &Schedule,
    run_id: &str,
    scheduled_for: DateTime<Utc>,
) -> Result<(), String> {
    if schedule.spec.action == "reminder" {
        let transaction = connection.unchecked_transaction().map_err(db_error)?;
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM schedule_runs WHERE id = ?1 AND status = 'completed')",
                [run_id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if exists {
            return Ok(());
        }
        store::add_inbox(
            &transaction,
            None,
            "reminder",
            &schedule.spec.title,
            &schedule.spec.prompt,
        )?;
        let changed = transaction
            .execute(
                "UPDATE schedule_runs SET status = 'completed', updated_at = ?1 WHERE id = ?2 AND status = 'started' AND task_id IS NULL",
                params![store::now(), run_id],
            )
            .map_err(db_error)?;
        if changed != 1 {
            return Err(
                "SCHEDULE_RUN_CHANGED: This schedule occurrence is no longer pending.".into(),
            );
        }
        transaction.commit().map_err(db_error)?;
        return Ok(());
    }
    if let Some(task_id) = connection
        .query_row(
            "SELECT id FROM tasks WHERE schedule_run_id = ?1",
            [run_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(db_error)?
    {
        connection
            .execute(
                "UPDATE schedule_runs SET task_id = ?1, status = 'running', updated_at = ?2 WHERE id = ?3 AND task_id IS NULL",
                params![task_id, store::now(), run_id],
            )
            .map_err(db_error)?;
        return Ok(());
    }
    let request = super::commands::StartTask {
        assistant_id: schedule.spec.assistant_id.clone(),
        goal: format!(
            "{}\n\n(Scheduled run of “{}” for {}.)",
            schedule.spec.prompt,
            schedule.spec.title,
            local_label(scheduled_for, &schedule.spec.timezone)
        ),
        project_id: schedule.spec.project_id.clone(),
        title: Some(format!("Scheduled: {}", schedule.spec.title)),
    };
    match super::commands::create_task(connection, &request, Some(run_id.to_owned())) {
        Ok(task) => {
            connection
                .execute("UPDATE schedule_runs SET task_id = ?1, status = 'running', updated_at = ?2 WHERE id = ?3", params![task.id, store::now(), run_id])
                .map_err(db_error)?;
            Ok(())
        }
        Err(error) => {
            connection
                .execute("UPDATE schedule_runs SET status = 'failed', detail = ?1, updated_at = ?2 WHERE id = ?3", params![error, store::now(), run_id])
                .map_err(db_error)?;
            store::add_inbox(
                connection,
                None,
                "failure",
                &format!("Could not start: {}", schedule.spec.title),
                &error,
            )?;
            Err(error)
        }
    }
}

/// Settles a terminal task and records its retry claim in one transaction.
/// `finalized_at` makes startup reconciliation idempotent, including recovery
/// of legacy v5 rows interrupted between status update and retry insertion.
fn finalize_terminal_run(
    connection: &Connection,
    run_id: &str,
    status: TaskStatus,
) -> Result<(), String> {
    if !status.is_terminal() {
        return Ok(());
    }
    let transaction = connection.unchecked_transaction().map_err(db_error)?;
    let run: Option<(String, String, i64, Option<String>, Option<String>)> = transaction
        .query_row(
            "SELECT schedule_id, scheduled_for, attempt, task_id, finalized_at FROM schedule_runs WHERE id = ?1",
            [run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .optional()
        .map_err(db_error)?;
    let Some((schedule_id, scheduled_for, attempt, task_id, finalized_at)) = run else {
        transaction.commit().map_err(db_error)?;
        return Ok(());
    };
    if finalized_at.is_some() {
        transaction.commit().map_err(db_error)?;
        return Ok(());
    }

    let now = store::now();
    transaction
        .execute(
            "UPDATE schedule_runs SET status = ?1, finalized_at = ?2, updated_at = ?2 WHERE id = ?3 AND finalized_at IS NULL",
            params![status.as_str(), now, run_id],
        )
        .map_err(db_error)?;

    if status == TaskStatus::Failed
        && let Some(task_id) = task_id.as_deref()
    {
        let schedule = get(&transaction, &schedule_id)?;
        if schedule.enabled
            && attempt > 0
            && (attempt as u64) <= u64::from(schedule.spec.retry_limit)
        {
            let root_id: Option<String> = transaction
                .query_row(
                    "SELECT root_id FROM tasks WHERE id = ?1",
                    [task_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(db_error)?;
            let changed_something = if let Some(root_id) = root_id {
                transaction
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM task_actions a JOIN tasks t ON t.id = a.task_id WHERE t.root_id = ?1 AND a.status IN ('succeeded', 'uncertain', 'started') AND (a.tool IN ('workspace.write', 'memory.save', 'code.python', 'code.run', 'schedule.create') OR a.tool LIKE 'mcp.%'))",
                        [root_id],
                        |row| row.get::<_, bool>(0),
                    )
                    .map_err(db_error)?
            } else {
                true
            };
            if !changed_something && let Some(next_attempt) = attempt.checked_add(1) {
                let retry_id = store::new_id("run");
                transaction
                    .execute(
                        "INSERT OR IGNORE INTO schedule_runs(id, schedule_id, scheduled_for, attempt, status, created_at, updated_at, detail) VALUES(?1, ?2, ?3, ?4, 'started', ?5, ?5, 'Retry of a failed occurrence that changed nothing')",
                        params![retry_id, schedule_id, scheduled_for, next_attempt, now],
                    )
                    .map_err(db_error)?;
            }
        }
    }
    transaction.commit().map_err(db_error)
}

/// Finishes a task-backed run. A retry claim remains pending for `tick` to
/// dispatch, so a crash cannot lose the task between claim and creation.
pub fn finish_run(connection: &Connection, run_id: &str, status: TaskStatus) -> Result<(), String> {
    finalize_terminal_run(connection, run_id, status)
}

/// Reconciles terminal tasks whose schedule bookkeeping was interrupted by a
/// crash. The finalization marker and retry row are committed together.
pub fn reconcile_terminal_runs(connection: &Connection) -> Result<usize, String> {
    let pending = {
        let mut statement = connection
            .prepare(
                "SELECT r.id, t.status FROM schedule_runs r JOIN tasks t ON t.id = r.task_id WHERE r.finalized_at IS NULL AND t.status IN ('completed', 'failed', 'cancelled') ORDER BY r.updated_at, r.id",
            )
            .map_err(db_error)?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?
    };
    let mut reconciled = 0;
    for (run_id, task_status) in pending {
        let status = TaskStatus::parse(&task_status)
            .ok_or_else(|| format!("SCHEDULE_RUN_INVALID: Unknown task status {task_status:?}."))?;
        finalize_terminal_run(connection, &run_id, status)?;
        reconciled += 1;
    }
    Ok(reconciled)
}

/// Starts an occurrence now, outside the rule ("Run now").
pub fn run_now(connection: &Connection, id: &str, now: DateTime<Utc>) -> Result<String, String> {
    let schedule = get(connection, id)?;
    let run_id = store::new_id("run");
    connection
        .execute(
            "INSERT INTO schedule_runs(id, schedule_id, scheduled_for, attempt, status, created_at, updated_at, detail) VALUES(?1, ?2, ?3, 1, 'started', ?4, ?4, 'Run now')",
            params![run_id, id, format!("manual:{}", now.to_rfc3339()), store::now()],
        )
        .map_err(db_error)?;
    start_run(connection, &schedule, &run_id, now)?;
    Ok(run_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(rule: Rule, zone: &str) -> ScheduleSpec {
        ScheduleSpec {
            title: "T".into(),
            action: "reminder".into(),
            prompt: "Drink water".into(),
            assistant_id: "a".into(),
            project_id: None,
            timezone: zone.into(),
            rule,
            missed_policy: "run-once".into(),
            retry_limit: 0,
            anchor: None,
        }
    }

    fn utc(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn daily_occurrences_follow_new_york_across_both_dst_changes() {
        let daily = spec(
            Rule::Daily {
                time: "09:00".into(),
            },
            "America/New_York",
        );
        // 9:00 EDT is 13:00 UTC; after November 1, 2026, 9:00 EST is 14:00 UTC.
        let next = upcoming(&daily, utc("2026-10-30T12:00:00Z"), 4);
        assert_eq!(next[0], utc("2026-10-30T13:00:00Z"));
        assert_eq!(next[2], utc("2026-11-01T14:00:00Z"));
        let spring = spec(
            Rule::Daily {
                time: "09:00".into(),
            },
            "America/New_York",
        );
        let march = upcoming(&spring, utc("2027-03-13T00:00:00Z"), 2);
        assert_eq!(march[0], utc("2027-03-13T14:00:00Z"));
        assert_eq!(march[1], utc("2027-03-14T13:00:00Z"));
    }

    #[test]
    fn a_time_in_the_spring_gap_runs_after_it_and_a_folded_time_runs_once() {
        let gap = spec(
            Rule::Daily {
                time: "02:30".into(),
            },
            "America/New_York",
        );
        // 2:30 does not exist on 2027-03-14; it runs at 3:00 EDT (07:00 UTC).
        let next = next_occurrence(&gap, utc("2027-03-14T05:00:00Z")).unwrap();
        assert_eq!(next, utc("2027-03-14T07:00:00Z"));
        let fold = spec(
            Rule::Daily {
                time: "01:30".into(),
            },
            "America/New_York",
        );
        // 1:30 happens twice on 2026-11-01; only the first (EDT, 05:30 UTC) counts.
        let first = next_occurrence(&fold, utc("2026-11-01T00:00:00Z")).unwrap();
        assert_eq!(first, utc("2026-11-01T05:30:00Z"));
        let after = next_occurrence(&fold, first).unwrap();
        assert_eq!(after, utc("2026-11-02T06:30:00Z"));
    }

    #[test]
    fn weekly_monthly_once_and_interval_rules() {
        let weekly = spec(
            Rule::Weekly {
                days: vec![1, 5],
                time: "08:00".into(),
            },
            "UTC",
        );
        let next = upcoming(&weekly, utc("2026-10-15T00:00:00Z"), 3);
        assert_eq!(next[0], utc("2026-10-16T08:00:00Z"));
        assert_eq!(next[1], utc("2026-10-19T08:00:00Z"));
        let monthly = spec(
            Rule::Monthly {
                day: 31,
                time: "10:00".into(),
            },
            "UTC",
        );
        assert_eq!(
            next_occurrence(&monthly, utc("2026-11-01T00:00:00Z")).unwrap(),
            utc("2026-11-30T10:00:00Z")
        );
        let once = spec(
            Rule::Once {
                at: "2026-10-20T09:00".into(),
            },
            "America/New_York",
        );
        assert_eq!(
            next_occurrence(&once, utc("2026-10-15T00:00:00Z")).unwrap(),
            utc("2026-10-20T13:00:00Z")
        );
        assert!(next_occurrence(&once, utc("2026-10-21T00:00:00Z")).is_none());
        let mut interval = spec(Rule::Interval { minutes: 60 }, "UTC");
        interval.anchor = Some("2026-10-15T00:00:00Z".into());
        assert_eq!(
            next_occurrence(&interval, utc("2026-10-15T05:30:00Z")).unwrap(),
            utc("2026-10-15T06:00:00Z")
        );
    }

    #[test]
    fn invalid_specs_are_refused() {
        assert!(
            validate(&spec(
                Rule::Daily {
                    time: "25:00".into()
                },
                "UTC"
            ))
            .is_err()
        );
        assert!(
            validate(&spec(
                Rule::Daily {
                    time: "09:00".into()
                },
                "Mars/Olympus"
            ))
            .is_err()
        );
        assert!(validate(&spec(Rule::Interval { minutes: 1 }, "UTC")).is_err());
        assert!(
            validate(&spec(
                Rule::Weekly {
                    days: vec![8],
                    time: "09:00".into()
                },
                "UTC"
            ))
            .is_err()
        );
    }

    fn database() -> (std::path::PathBuf, Connection) {
        let path =
            std::env::temp_dir().join(format!("juniper-schedule-{}.db", uuid::Uuid::new_v4()));
        crate::storage::initialize(&path).unwrap();
        let connection = store::open(&path).unwrap();
        (path, connection)
    }

    fn terminal_scheduled_task(
        connection: &Connection,
        retry_limit: u32,
        scheduled_for: &str,
        uncertain_side_effect: bool,
    ) -> (Schedule, String, String) {
        let mut agent_spec = spec(
            Rule::Daily {
                time: "09:00".into(),
            },
            "UTC",
        );
        agent_spec.action = "agent-task".into();
        agent_spec.retry_limit = retry_limit;
        let schedule = save(
            connection,
            None,
            agent_spec,
            true,
            0,
            utc("2026-10-15T00:00:00Z"),
        )
        .unwrap();
        let run_id = store::new_id("run");
        let task_id = store::new_id("task");
        connection
            .execute(
                "INSERT INTO schedule_runs(id, schedule_id, scheduled_for, attempt, status, task_id, created_at, updated_at) VALUES(?1, ?2, ?3, 1, 'running', ?4, ?5, ?5)",
                params![run_id, schedule.id, scheduled_for, task_id, store::now()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO tasks(id, root_id, assistant_id, schedule_run_id, title, goal, status, budget, usage, provenance, created_at, updated_at) VALUES(?1, ?1, 'a', ?2, 'Scheduled task', 'Goal', 'failed', '{}', '{}', '{}', ?3, ?3)",
                params![task_id, run_id, store::now()],
            )
            .unwrap();
        if uncertain_side_effect {
            connection
                .execute(
                    "INSERT INTO task_actions(id, task_id, step, call_id, tool, arguments, binding, status, created_at, updated_at) VALUES(?1, ?2, 1, 'call-1', 'mcp.send', '{}', '', 'uncertain', ?3, ?3)",
                    params![store::new_id("action"), task_id, store::now()],
                )
                .unwrap();
        }
        (schedule, run_id, task_id)
    }

    #[test]
    fn terminal_run_and_retry_claim_reconcile_atomically_and_idempotently() {
        let (path, connection) = database();
        let (_, run_id, _) =
            terminal_scheduled_task(&connection, 1, "2026-10-15T09:00:00+00:00", false);
        connection
            .execute_batch(
                "CREATE TRIGGER reject_retry BEFORE INSERT ON schedule_runs WHEN NEW.attempt = 2 BEGIN SELECT RAISE(ABORT, 'simulated retry insert failure'); END;",
            )
            .unwrap();
        assert!(finish_run(&connection, &run_id, TaskStatus::Failed).is_err());
        let (status, finalized_at): (String, Option<String>) = connection
            .query_row(
                "SELECT status, finalized_at FROM schedule_runs WHERE id = ?1",
                [&run_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "running", "the failed transaction must roll back");
        assert!(finalized_at.is_none());

        connection
            .execute_batch("DROP TRIGGER reject_retry;")
            .unwrap();
        // Startup recovery handles both the terminal task and the old crash
        // window, then a second pass leaves the retry claim unchanged.
        assert_eq!(store::recover_after_restart(&connection).unwrap(), 0);
        assert_eq!(store::recover_after_restart(&connection).unwrap(), 0);
        let retry_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM schedule_runs WHERE attempt = 2",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retry_count, 1);
        let (status, finalized_at): (String, Option<String>) = connection
            .query_row(
                "SELECT status, finalized_at FROM schedule_runs WHERE id = ?1",
                [&run_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "failed");
        assert!(finalized_at.is_some());
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn terminal_reconciliation_never_retries_an_uncertain_side_effect() {
        let (path, connection) = database();
        let (_, run_id, _) =
            terminal_scheduled_task(&connection, 1, "2026-10-16T09:00:00+00:00", true);
        finish_run(&connection, &run_id, TaskStatus::Failed).unwrap();
        let retries: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM schedule_runs WHERE attempt > 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retries, 0);
        let status: String = connection
            .query_row(
                "SELECT status FROM schedule_runs WHERE id = ?1",
                [&run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "failed");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn an_occurrence_is_claimed_once_and_a_backlog_is_not_replayed() {
        let (path, connection) = database();
        let created = utc("2026-10-15T00:00:00Z");
        let schedule = save(
            &connection,
            None,
            spec(
                Rule::Daily {
                    time: "09:00".into(),
                },
                "UTC",
            ),
            true,
            0,
            created,
        )
        .unwrap();
        assert_eq!(
            schedule.next_run_at.as_deref(),
            Some("2026-10-15T09:00:00+00:00")
        );
        // Two claimants at the same moment: one wins.
        let at = utc("2026-10-15T09:01:00Z");
        let first = claim_due(&connection, at).unwrap();
        let second = claim_due(&connection, at).unwrap();
        assert_eq!(first.len(), 1);
        assert!(second.is_empty());
        // Closed for three days: at the next launch only the latest missed
        // occurrence runs (run-once), and the next one is in the future.
        let reopened = utc("2026-10-18T12:00:00Z");
        let claimed = claim_due(&connection, reopened).unwrap();
        assert_eq!(claimed.len(), 1);
        assert!(matches!(
            claimed[0].1,
            Claim::Run { scheduled_for, .. } if scheduled_for == utc("2026-10-18T09:00:00Z")
        ));
        let after = get(&connection, &schedule.id).unwrap();
        assert_eq!(
            after.next_run_at.as_deref(),
            Some("2026-10-19T09:00:00+00:00")
        );
        assert!(claim_due(&connection, reopened).unwrap().is_empty());
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn failed_occurrence_insert_does_not_advance_the_schedule() {
        let (path, connection) = database();
        let schedule = save(
            &connection,
            None,
            spec(
                Rule::Daily {
                    time: "09:00".into(),
                },
                "UTC",
            ),
            true,
            0,
            utc("2026-10-15T00:00:00Z"),
        )
        .unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_schedule_run BEFORE INSERT ON schedule_runs BEGIN SELECT RAISE(ABORT, 'simulated run-row failure'); END;",
            )
            .unwrap();
        assert!(claim_due(&connection, utc("2026-10-15T09:01:00Z")).is_err());
        assert_eq!(
            get(&connection, &schedule.id)
                .unwrap()
                .next_run_at
                .as_deref(),
            Some("2026-10-15T09:00:00+00:00")
        );
        connection
            .execute_batch("DROP TRIGGER fail_schedule_run;")
            .unwrap();
        assert_eq!(
            claim_due(&connection, utc("2026-10-15T09:01:00Z"))
                .unwrap()
                .len(),
            1
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_skip_policy_skips_missed_runs_and_reminders_need_no_model() {
        let (path, connection) = database();
        let mut skipping = spec(
            Rule::Daily {
                time: "09:00".into(),
            },
            "UTC",
        );
        skipping.missed_policy = "skip".into();
        let schedule = save(
            &connection,
            None,
            skipping,
            true,
            0,
            utc("2026-10-15T00:00:00Z"),
        )
        .unwrap();
        let claimed = claim_due(&connection, utc("2026-10-15T11:00:00Z")).unwrap();
        assert!(matches!(claimed[0].1, Claim::Skipped { .. }));
        // A reminder completes without creating a task.
        let run_id = run_now(&connection, &schedule.id, utc("2026-10-15T11:05:00Z")).unwrap();
        let status: String = connection
            .query_row(
                "SELECT status FROM schedule_runs WHERE id = ?1",
                [&run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "completed");
        let tasks: i64 = connection
            .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(tasks, 0);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_committed_reminder_claim_is_replayed_once_after_dispatch_interruption() {
        let (path, connection) = database();
        let schedule = save(
            &connection,
            None,
            spec(
                Rule::Daily {
                    time: "09:00".into(),
                },
                "UTC",
            ),
            true,
            0,
            utc("2026-10-15T00:00:00Z"),
        )
        .unwrap();
        let run_id = store::new_id("run");
        connection
            .execute(
                "INSERT INTO schedule_runs(id, schedule_id, scheduled_for, attempt, status, created_at, updated_at) VALUES(?1, ?2, ?3, 1, 'started', ?4, ?4)",
                params![run_id, schedule.id, "2026-10-15T09:00:00+00:00", store::now()],
            )
            .unwrap();

        assert_eq!(dispatch_pending(&connection).unwrap(), 1);
        assert_eq!(dispatch_pending(&connection).unwrap(), 0);
        let reminders: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM inbox WHERE kind = 'reminder' AND title = ?1",
                [&schedule.spec.title],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(reminders, 1);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_committed_agent_claim_recovers_after_task_creation_before_run_link() {
        let (path, connection) = database();
        let mut agent_spec = spec(
            Rule::Daily {
                time: "09:00".into(),
            },
            "UTC",
        );
        agent_spec.action = "agent-task".into();
        agent_spec.assistant_id = "assistant-1".into();
        let schedule = save(
            &connection,
            None,
            agent_spec,
            true,
            0,
            utc("2026-10-15T00:00:00Z"),
        )
        .unwrap();
        connection
            .execute(
                "INSERT INTO app_state(id, payload, updated_at) VALUES('singleton', ?1, ?2)",
                params![
                    json!({ "assistants": [{ "id": "assistant-1" }] }).to_string(),
                    store::now()
                ],
            )
            .unwrap();
        let run_id = store::new_id("run");
        connection
            .execute(
                "INSERT INTO schedule_runs(id, schedule_id, scheduled_for, attempt, status, created_at, updated_at) VALUES(?1, ?2, ?3, 1, 'started', ?4, ?4)",
                params![run_id, schedule.id, "2026-10-15T09:00:00+00:00", store::now()],
            )
            .unwrap();

        assert_eq!(dispatch_pending(&connection).unwrap(), 1);
        let task_id: String = connection
            .query_row(
                "SELECT task_id FROM schedule_runs WHERE id = ?1",
                [&run_id],
                |row| row.get(0),
            )
            .unwrap();
        connection
            .execute(
                "UPDATE schedule_runs SET task_id = NULL, status = 'started' WHERE id = ?1",
                [&run_id],
            )
            .unwrap();

        assert_eq!(dispatch_pending(&connection).unwrap(), 1);
        let linked_task: String = connection
            .query_row(
                "SELECT task_id FROM schedule_runs WHERE id = ?1",
                [&run_id],
                |row| row.get(0),
            )
            .unwrap();
        let task_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM tasks WHERE schedule_run_id = ?1",
                [&run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(linked_task, task_id);
        assert_eq!(task_count, 1);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn disabled_and_edited_schedules_recompute_their_next_run() {
        let (path, connection) = database();
        let now = utc("2026-10-15T00:00:00Z");
        let schedule = save(
            &connection,
            None,
            spec(
                Rule::Daily {
                    time: "09:00".into(),
                },
                "UTC",
            ),
            false,
            0,
            now,
        )
        .unwrap();
        assert!(schedule.next_run_at.is_none());
        assert!(
            claim_due(&connection, utc("2026-10-16T00:00:00Z"))
                .unwrap()
                .is_empty()
        );
        let edited = save(
            &connection,
            Some(&schedule.id),
            spec(
                Rule::Daily {
                    time: "18:00".into(),
                },
                "America/New_York",
            ),
            true,
            schedule.revision,
            now,
        )
        .unwrap();
        assert_eq!(
            edited.next_run_at.as_deref(),
            Some("2026-10-15T22:00:00+00:00")
        );
        assert!(
            save(
                &connection,
                Some(&schedule.id),
                spec(
                    Rule::Daily {
                        time: "18:00".into()
                    },
                    "UTC"
                ),
                true,
                schedule.revision,
                now
            )
            .is_err(),
            "a stale revision is refused"
        );
        std::fs::remove_file(path).ok();
    }
}
