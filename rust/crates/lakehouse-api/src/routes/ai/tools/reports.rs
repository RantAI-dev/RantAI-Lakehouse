//! Scheduled-report tools (`AI-10`, schedule half; `RPT-1` plan D10):
//! `create_report_subscription`, `list_report_subscriptions`,
//! `update_report_subscription` and `delete_report_subscription`.
//!
//! They call the same rules as the console's routes
//! ([`crate::routes::reports::validate_input`]): the download right, a
//! dashboard the person can see, charts that belong to it, outside addresses
//! only on a domain an administrator listed, at most 50 recipients.
//!
//! # Who needs to approve (feature decision 10; product owner, 2026-10-17)
//!
//! - A subscription that sends only to the person asking is `WriteLow`: the
//!   console's Confirm button, no approval.
//! - A call that names any other recipient is `WriteHigh` for that call
//!   ([`crate::routes::ai::gate::effective_risk`]): it waits in Approvals until
//!   a person with `agent:approve` decides. So is every delete.
//!
//! # The report belongs to the requester, not to the approver
//!
//! An approved action runs as the approver (`decide_approval` hands
//! `run_tool` the approver). A report made as the approver would carry the
//! approver's masking and row filters to the recipients. So before the
//! approval is created, [`approval_args`] checks the requester's rights and
//! stamps their id into the stored arguments as `ownerId`; when the approved
//! call runs, the tool reads `ownerId`, makes the subscription theirs and
//! validates it as them (their rights today). The approver is also checked
//! when it runs: `dashboard:read` (the registry's permission, checked by
//! `decide_approval`), the download right, and the dashboard being visible to
//! them. A model cannot name an owner: `ownerId` is stripped from every
//! call's arguments ([`crate::routes::ai::gate::strip_confirmed`]) and only
//! ever written here, server side, into an approval's stored arguments.

use lakehouse_auth::Principal;
use lakehouse_bi::store;
use lakehouse_core::ApiError;
use lakehouse_store::report_subscriptions::{self as subscriptions, Subscription};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::arg_str;
use crate::routes::reports::{self, SubscriptionInput, schedule_json, validate_input};
use crate::state::AppState;

const SIGN_IN: &str = "Sign in to manage scheduled reports.";
const NOT_YOURS: &str = "Only the person who set this report up can change it from the assistant; an administrator can use the dashboard's Subscriptions.";
const NO_SUCH: &str = "That subscription does not exist.";
const OWNER_GONE: &str = "The person who asked for this report is no longer active.";
const FAILED: &str = "The request could not be completed.";

/// The four tools.
pub(super) fn handles(name: &str) -> bool {
    matches!(
        name,
        "create_report_subscription"
            | "list_report_subscriptions"
            | "update_report_subscription"
            | "delete_report_subscription"
    )
}

/// The sentence for a refused call. Every `ApiError` `validate_input` and the
/// store produce carries one of the routes' fixed sentences; anything else
/// (a store failure under a reference) reads as [`FAILED`].
fn sentence(err: &ApiError) -> String {
    match err {
        ApiError::BadRequest(s)
        | ApiError::NotFound(s)
        | ApiError::Unavailable(s)
        | ApiError::Unprocessable(s)
        | ApiError::PermissionDenied(s) => s.clone(),
        _ => FAILED.to_owned(),
    }
}

fn refusal(err: &ApiError) -> Value {
    json!({ "error": sentence(err) })
}

/// Dispatch one of the four tools.
pub(super) async fn run(
    state: &AppState,
    principal: Option<&Principal>,
    name: &str,
    args: &Map<String, Value>,
) -> Value {
    let Some(caller) = principal else {
        return json!({ "error": SIGN_IN });
    };
    let owner = match acting_owner(state, caller, args).await {
        Ok(owner) => owner,
        Err(error) => return error,
    };
    // After an approval the approver releases the requester's report: the
    // dashboard must be one they can see themselves.
    if owner.id != caller.id
        && let Some(board) = board_of(state, name, args).await
    {
        // BI-19 B1 review R1: the approver's resolved viewer, not the baseline.
        let viewer = match super::viewer_for(state, Some(caller)).await {
            Ok(viewer) => viewer,
            Err(error) => return error,
        };
        let seen = store::get_board(&state.clickhouse, &board, &viewer).await;
        if !matches!(&seen, Ok(Some(_))) && board != store::DEFAULT_BOARD_ID {
            return json!({ "error": "You cannot see that dashboard, so you cannot approve a report of it." });
        }
    }
    match name {
        "create_report_subscription" => create(state, &owner, args).await,
        "list_report_subscriptions" => list(state, &owner, args).await,
        "update_report_subscription" => update(state, &owner, args).await,
        "delete_report_subscription" => delete(state, &owner, args).await,
        other => json!({ "error": format!("unknown tool: {other}") }),
    }
}

/// The dashboard a call is about: the `board` argument of a create, the board of
/// the subscription an update or delete names.
async fn board_of(state: &AppState, name: &str, args: &Map<String, Value>) -> Option<String> {
    if name == "create_report_subscription" {
        return Some(arg_str(args, "board")).filter(|b| !b.trim().is_empty());
    }
    let id = arg_str(args, "id").parse::<Uuid>().ok()?;
    subscriptions::get(state.pg.as_deref()?, id)
        .await
        .ok()
        .flatten()
        .map(|s| s.board)
}

/// The person the call acts for. Normally the caller. After an approval, the
/// stamped requester (`ownerId`), loaded as they are now; the approver must then
/// also be able to download.
async fn acting_owner(
    state: &AppState,
    caller: &Principal,
    args: &Map<String, Value>,
) -> Result<Principal, Value> {
    let Some(stamped) = args.get("ownerId").and_then(Value::as_str) else {
        return Ok(caller.clone());
    };
    if stamped == caller.id.uuid().to_string() {
        return Ok(caller.clone());
    }
    // Run after an approval, by someone else: the approver must be allowed to
    // download too (they are releasing data to the recipients).
    crate::routes::settings::require_download(state, caller)
        .await
        .map_err(|e| refusal(&e))?;
    match crate::routes::render::principal_of(state, stamped).await {
        Ok(Some(owner)) => Ok(owner),
        Ok(None) => Err(json!({ "error": OWNER_GONE })),
        Err(_) => Err(json!({ "error": FAILED })),
    }
}

fn pool(state: &AppState) -> Result<&lakehouse_store::PgPool, Value> {
    state
        .pg
        .as_deref()
        .ok_or_else(|| json!({ "error": "The database is unavailable." }))
}

/// A subscription as the model reads it: no addresses of people other than
/// the names and addresses the owner chose.
async fn brief(state: &AppState, sub: &Subscription) -> Value {
    let recipients = match state.pg.as_deref() {
        Some(pool) => subscriptions::recipients(pool, sub.id)
            .await
            .unwrap_or_default(),
        None => Vec::new(),
    };
    json!({
        "id": sub.id,
        "board": sub.board,
        "schedule": sub.schedule().map_or(Value::Null, schedule_json),
        "timeZone": sub.time_zone,
        "attachments": sub.attachments,
        "skipWhenEmpty": sub.skip_when_empty,
        "paused": sub.paused,
        "pausedReason": sub.paused_reason,
        "nextRunAt": sub.next_run_at.to_string(),
        "recipients": recipients.iter().map(|r| match (&r.user_id, &r.email) {
            (Some(id), _) => json!({ "userId": id, "name": r.user_name, "email": r.user_email }),
            (None, email) => json!({ "email": email }),
        }).collect::<Vec<_>>(),
    })
}

fn email_note(state: &AppState) -> Option<&'static str> {
    state
        .config
        .smtp_host
        .is_none()
        .then_some("Email is not set up on this deployment, so nothing is sent until it is.")
}

// ── Arguments to the console's body ──────────────────────────────────────

/// The subscription body the console's rules check, from the tool's flat
/// arguments. Recipients: the person themselves unless `sendToMe` is false,
/// plus the others named in `recipients`. `existing` supplies whatever an
/// update leaves out.
fn input_from_args(
    args: &Map<String, Value>,
    me: Uuid,
    existing: Option<&Subscription>,
) -> Result<SubscriptionInput, String> {
    let given = |key: &str| args.contains_key(key);
    let schedule = match (args.get("schedule"), existing) {
        (Some(s), _) => s.clone(),
        (None, Some(e)) => e
            .schedule()
            .map(schedule_json)
            .map_err(|_| FAILED.to_owned())?,
        (None, None) => return Err("schedule is required.".to_owned()),
    };
    let attachments = if existing.is_none()
        || ["pdf", "charts", "paper", "orientation"]
            .iter()
            .any(|k| given(k))
    {
        let base = existing.map_or(&Value::Null, |e| &e.attachments);
        let pdf_now = match args.get("pdf").and_then(Value::as_bool) {
            Some(on) => on,
            None => existing.is_none_or(|_| !base["pdf"].is_null()),
        };
        let pdf = pdf_now.then(|| {
            json!({
                "paper": args.get("paper").and_then(Value::as_str)
                    .or_else(|| base["pdf"]["paper"].as_str()).unwrap_or("a4"),
                "orientation": args.get("orientation").and_then(Value::as_str)
                    .or_else(|| base["pdf"]["orientation"].as_str()).unwrap_or("portrait"),
            })
        });
        let charts = args
            .get("charts")
            .cloned()
            .unwrap_or_else(|| base.get("charts").cloned().unwrap_or_else(|| json!([])));
        json!({ "pdf": pdf, "charts": charts })
    } else {
        existing.map_or(Value::Null, |e| e.attachments.clone())
    };
    let mut recipients: Vec<Value> = Vec::new();
    let recipients_given = given("recipients") || given("sendToMe");
    // An update that names no recipients keeps the stored ones; the caller
    // re-reads them (`update`).
    if recipients_given || existing.is_none() {
        if args
            .get("sendToMe")
            .and_then(Value::as_bool)
            .unwrap_or(true)
        {
            recipients.push(json!({ "userId": me.to_string() }));
        }
        if let Some(others) = args.get("recipients").and_then(Value::as_array) {
            recipients.extend(others.iter().cloned());
        }
    }
    let pick =
        |key: &str, stored: Option<Value>| -> Option<Value> { args.get(key).cloned().or(stored) };
    let body = json!({
        "schedule": schedule,
        "timeZone": pick("timeZone", existing.map(|e| json!(e.time_zone))),
        "attachments": attachments,
        "filters": pick("filters", existing.and_then(|e| e.filters.clone())),
        "params": pick("params", existing.and_then(|e| e.params.clone())),
        "grain": pick("grain", existing.and_then(|e| e.grain.clone().map(Value::String))),
        "skipWhenEmpty": args.get("skipWhenEmpty").and_then(Value::as_bool)
            .or_else(|| existing.map(|e| e.skip_when_empty)).unwrap_or(false),
        "recipients": recipients,
    });
    serde_json::from_value(body).map_err(|_| {
        "The arguments do not describe a report: check schedule, recipients and charts.".to_owned()
    })
}

// ── The tools ────────────────────────────────────────────────────────────

async fn create(state: &AppState, owner: &Principal, args: &Map<String, Value>) -> Value {
    let board = arg_str(args, "board");
    if board.trim().is_empty() {
        return json!({ "error": "board is required." });
    }
    let input = match input_from_args(args, owner.id.uuid(), None) {
        Ok(input) => input,
        Err(e) => return json!({ "error": e }),
    };
    let new = match validate_input(state, owner, board.trim(), &input).await {
        Ok(new) => new,
        Err(e) => return refusal(&e),
    };
    let pool = match pool(state) {
        Ok(p) => p,
        Err(e) => return e,
    };
    match subscriptions::create(pool, &new).await {
        Ok(made) => {
            let mut out = json!({ "created": true, "subscription": brief(state, &made).await });
            if let Some(note) = email_note(state) {
                out["warning"] = json!(note);
            }
            out
        }
        Err(_) => json!({ "error": FAILED }),
    }
}

async fn list(state: &AppState, owner: &Principal, args: &Map<String, Value>) -> Value {
    let pool = match pool(state) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let board = arg_str(args, "board");
    let Ok(rows) = subscriptions::list_for_owner(pool, owner.id.uuid()).await else {
        return json!({ "error": FAILED });
    };
    let mut items = Vec::new();
    for sub in rows.iter().filter(|s| board.is_empty() || s.board == board) {
        items.push(brief(state, sub).await);
    }
    json!({ "items": items })
}

/// The caller's subscription by `id` argument.
async fn own(
    state: &AppState,
    owner: &Principal,
    args: &Map<String, Value>,
) -> Result<Subscription, Value> {
    let id = arg_str(args, "id")
        .parse::<Uuid>()
        .map_err(|_| json!({ "error": NO_SUCH }))?;
    let sub = subscriptions::get(pool(state)?, id)
        .await
        .map_err(|_| json!({ "error": FAILED }))?
        .ok_or_else(|| json!({ "error": NO_SUCH }))?;
    if sub.owner_id == owner.id.uuid() || owner.has("settings:write") {
        Ok(sub)
    } else {
        Err(json!({ "error": NOT_YOURS }))
    }
}

/// Recipients the call did not touch stay as stored. Shared by the tool and the
/// check made before an approval (`RPT-1` review fix R2: that check validated an
/// update with no recipients at all).
async fn keep_stored_recipients(
    pool: &lakehouse_store::PgPool,
    sub: &Subscription,
    args: &Map<String, Value>,
    input: &mut SubscriptionInput,
) -> Result<(), Value> {
    if args.contains_key("recipients") || args.contains_key("sendToMe") {
        return Ok(());
    }
    let rows = subscriptions::recipients(pool, sub.id)
        .await
        .map_err(|_| json!({ "error": FAILED }))?;
    input.recipients = rows
        .iter()
        .filter_map(|r| match (&r.user_id, &r.email) {
            (Some(id), _) => Some(reports::RecipientInput {
                user_id: Some(id.to_string()),
                email: None,
            }),
            (None, Some(email)) => Some(reports::RecipientInput {
                user_id: None,
                email: Some(email.clone()),
            }),
            (None, None) => None,
        })
        .collect();
    Ok(())
}

async fn update(state: &AppState, owner: &Principal, args: &Map<String, Value>) -> Value {
    let sub = match own(state, owner, args).await {
        Ok(sub) => sub,
        Err(e) => return e,
    };
    if sub.owner_id != owner.id.uuid() {
        return json!({ "error": NOT_YOURS });
    }
    let pool = match pool(state) {
        Ok(p) => p,
        Err(e) => return e,
    };
    // RPT-1 review fix (SHOULD-FIX) R4: pausing or resuming alone skips the
    // re-validation, as on the console's route.
    if let Some(paused) = args.get("paused").and_then(Value::as_bool)
        && args
            .keys()
            .all(|k| matches!(k.as_str(), "id" | "paused" | "confirmed"))
    {
        return match reports::apply_pause(pool, &sub, paused).await {
            Ok(saved) => json!({ "updated": true, "subscription": brief(state, &saved).await }),
            Err(e) => refusal(&e),
        };
    }
    let mut input = match input_from_args(args, owner.id.uuid(), Some(&sub)) {
        Ok(input) => input,
        Err(e) => return json!({ "error": e }),
    };
    if let Err(e) = keep_stored_recipients(pool, &sub, args, &mut input).await {
        return e;
    }
    input.paused = args.get("paused").and_then(Value::as_bool);
    let new = match validate_input(state, owner, &sub.board, &input).await {
        Ok(new) => new,
        Err(e) => return refusal(&e),
    };
    match reports::save_update(pool, &sub, &new, input.paused).await {
        Ok(saved) => json!({ "updated": true, "subscription": brief(state, &saved).await }),
        Err(e) => refusal(&e),
    }
}

async fn delete(state: &AppState, owner: &Principal, args: &Map<String, Value>) -> Value {
    let sub = match own(state, owner, args).await {
        Ok(sub) => sub,
        Err(e) => return e,
    };
    let pool = match pool(state) {
        Ok(p) => p,
        Err(e) => return e,
    };
    match subscriptions::delete(pool, sub.id).await {
        Ok(_) => json!({ "deleted": true, "id": sub.id }),
        Err(_) => json!({ "error": FAILED }),
    }
}

// ── Who an update leaves receiving the report (RPT-1 review fix R2) ──────

/// A recipient as a comparable key: `u:<uuid>` for a console user, `e:<address>`
/// for an outside address.
fn recipient_key(user_id: Option<&str>, email: Option<&str>) -> String {
    match (user_id, email) {
        (Some(id), _) => format!(
            "u:{}",
            id.parse::<Uuid>()
                .map_or_else(|_| id.to_lowercase(), |u| u.to_string())
        ),
        (None, Some(email)) => format!(
            "e:{}",
            crate::routes::report_runner::normalise_address(email)
                .unwrap_or_else(|| email.trim().to_lowercase())
        ),
        (None, None) => "?".to_owned(),
    }
}

/// The pure half of [`report_audience`]: whether an update of a subscription
/// whose stored recipients are `stored` needs a human's approval. It does when
/// the recipients the call leaves in place (the stored ones when it names none,
/// else the person themselves unless `sendToMe` is false, plus the named ones)
/// include anyone but `me`, unless the call only narrows: it touches nothing but
/// `paused` and the recipients, and no recipient is new.
fn update_needs_approval(args: &Map<String, Value>, me: Uuid, stored: &[String]) -> bool {
    let me_key = format!("u:{me}");
    let named = args.contains_key("recipients") || args.contains_key("sendToMe");
    let resulting: Vec<String> = if named {
        let mut list = Vec::new();
        if args
            .get("sendToMe")
            .and_then(Value::as_bool)
            .unwrap_or(true)
        {
            list.push(me_key.clone());
        }
        for r in args
            .get("recipients")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            list.push(recipient_key(
                r.get("userId").and_then(Value::as_str),
                r.get("email").and_then(Value::as_str),
            ));
        }
        list
    } else {
        stored.to_vec()
    };
    if resulting.iter().all(|k| *k == me_key) {
        return false;
    }
    let only_narrowing_keys = args.keys().all(|k| {
        matches!(
            k.as_str(),
            "id" | "paused" | "recipients" | "sendToMe" | "confirmed"
        )
    });
    let nothing_new = resulting.iter().all(|k| stored.contains(k));
    !(only_narrowing_keys && nothing_new)
}

/// `RPT-1` review fix (BLOCKER) R2: the server-side fact the gate needs before it
/// decides an `update_report_subscription` call: `Some(true)` when it needs a
/// human's approval. Looked up at every dispatch site (the chat loop and
/// `POST /api/ai/tool`; headless runs refuse the tool, R1) and given to
/// `gate::decide_with`, never put in the arguments. Fails closed: a lookup that
/// cannot be made, or a subscription that is not there, reads as an approval
/// (the approval step then refuses what the requester may not do). `None` for any
/// other tool.
pub(in crate::routes) async fn report_audience(
    state: &AppState,
    requester: Option<&Principal>,
    name: &str,
    args: &Map<String, Value>,
) -> Option<bool> {
    if name != "update_report_subscription" {
        return None;
    }
    let (Some(me), Some(pool)) = (requester, state.pg.as_deref()) else {
        return Some(true);
    };
    let Ok(id) = arg_str(args, "id").parse::<Uuid>() else {
        return Some(true);
    };
    let Ok(Some(sub)) = subscriptions::get(pool, id).await else {
        return Some(true);
    };
    let Ok(rows) = subscriptions::recipients(pool, sub.id).await else {
        return Some(true);
    };
    let stored: Vec<String> = rows
        .iter()
        .map(|r| {
            recipient_key(
                r.user_id.map(|u| u.to_string()).as_deref(),
                r.email.as_deref(),
            )
        })
        .collect();
    Some(update_needs_approval(args, me.id.uuid(), &stored))
}

// ── Before an approval exists ────────────────────────────────────────────

/// Whether a call to this tool is stamped and checked before it waits for
/// approval.
pub(in crate::routes) fn needs_stamp(name: &str) -> bool {
    matches!(
        name,
        "create_report_subscription" | "update_report_subscription" | "delete_report_subscription"
    )
}

/// The arguments to store with an approval. For the report tools: the
/// requester's rights are checked now (so nobody is asked to approve what the
/// requester could not do), and their id is written into the arguments as
/// `ownerId`. Any other tool's arguments come back unchanged.
///
/// # Errors
///
/// The refusal to show instead of creating an approval.
pub(in crate::routes) async fn approval_args(
    state: &AppState,
    requester: Option<&Principal>,
    name: &str,
    args: &Map<String, Value>,
) -> Result<Map<String, Value>, Value> {
    if !needs_stamp(name) {
        return Ok(args.clone());
    }
    let Some(requester) = requester else {
        return Err(json!({ "error": SIGN_IN }));
    };
    let me = requester.id.uuid();
    // What the requester asks for must be something they could do: the same
    // checks the tool makes when it runs, made now, as them.
    match name {
        "create_report_subscription" => {
            let board = arg_str(args, "board");
            let input = input_from_args(args, me, None).map_err(|e| json!({ "error": e }))?;
            validate_input(state, requester, board.trim(), &input)
                .await
                .map_err(|e| refusal(&e))?;
        }
        "update_report_subscription" => {
            let sub = own(state, requester, args).await?;
            if sub.owner_id != me {
                return Err(json!({ "error": NOT_YOURS }));
            }
            let mut input =
                input_from_args(args, me, Some(&sub)).map_err(|e| json!({ "error": e }))?;
            keep_stored_recipients(pool(state)?, &sub, args, &mut input).await?;
            validate_input(state, requester, &sub.board, &input)
                .await
                .map_err(|e| refusal(&e))?;
        }
        _ => {
            own(state, requester, args).await?;
        }
    }
    let mut stamped = args.clone();
    stamped.insert("ownerId".to_owned(), json!(me.to_string()));
    Ok(stamped)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[allow(
        clippy::needless_pass_by_value,
        reason = "every caller passes a fresh json! literal"
    )]
    fn args(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    const ME: Uuid = Uuid::from_u128(1);
    const COLLEAGUE: Uuid = Uuid::from_u128(2);

    fn stored_with_outsiders() -> Vec<String> {
        vec![
            format!("u:{ME}"),
            format!("u:{COLLEAGUE}"),
            "e:press@partner.example".to_owned(),
        ]
    }

    /// RPT-1 review fix (BLOCKER) R2.
    #[test]
    fn a_change_to_a_report_with_other_recipients_needs_approval_unless_it_only_narrows() {
        let stored = stored_with_outsiders();
        // A new chart sent to the stored list: approval.
        assert!(update_needs_approval(
            &args(json!({ "id": "x", "charts": [] })),
            ME,
            &stored
        ));
        // So is a new time, a filter, a recipient that is new.
        assert!(update_needs_approval(
            &args(json!({ "id": "x", "schedule": {} })),
            ME,
            &stored
        ));
        assert!(update_needs_approval(
            &args(json!({ "id": "x", "recipients": [{ "email": "new@partner.example" }] })),
            ME,
            &stored
        ));
        // Pausing, resuming, removing a recipient: a Confirm.
        assert!(!update_needs_approval(
            &args(json!({ "id": "x", "paused": true })),
            ME,
            &stored
        ));
        assert!(!update_needs_approval(
            &args(json!({ "id": "x", "recipients": [{ "email": "Press@Partner.example" }] })),
            ME,
            &stored
        ));
        assert!(!update_needs_approval(
            &args(
                json!({ "id": "x", "sendToMe": false, "recipients": [{ "userId": COLLEAGUE.to_string() }], "confirmed": true })
            ),
            ME,
            &stored
        ));
        // Narrowing to the person alone while changing a chart: nobody else receives it.
        assert!(!update_needs_approval(
            &args(json!({ "id": "x", "recipients": [], "charts": [] })),
            ME,
            &stored
        ));
        // A report that only ever went to the person stays a Confirm for anything.
        let alone = vec![format!("u:{ME}")];
        assert!(!update_needs_approval(
            &args(json!({ "id": "x", "charts": [] })),
            ME,
            &alone
        ));
    }
}
