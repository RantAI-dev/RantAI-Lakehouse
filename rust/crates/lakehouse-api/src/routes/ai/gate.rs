//! The copilot's dispatch-site gate: enforces both the Ask-mode
//! read-only restriction and the calling principal's `resource:action`
//! permissions, for EVERY tool call about to execute.
//!
//! This replaces the old `WRITE_TOOLS: [&str; 5]` array + `write_tool_refusal`
//! pair that used to live directly in `ai.rs`: the same semantics for
//! Ask-mode, plus (T0.2 of the copilot-operations-handover plan) a second
//! check so the copilot can never let a principal execute a tool whose
//! equivalent console route they could not call.
//!
//! ## Role-impact table (T0.2, current seed data — `0002_seed_identity.sql`)
//!
//! Wiring [`ToolSpec::permission`](super::registry::ToolSpec::permission)
//! into this gate is a real, visible behaviour change for six of the seven
//! seeded roles, because only Platform Admin (`*:*`) and Dashboard Viewer
//! (`dashboard:read`) hold any `dashboard:*` grant, and only Platform Admin
//! holds `dashboard:write`:
//!
//! | Role | Loses copilot access to | Keeps |
//! | --- | --- | --- |
//! | Analyst (`query:read, catalog:read, lineage:read`) | `describe_mart`, `list_charts`, `list_boards`, `suggest_dashboard` (need `dashboard:read`); `create_chart`/`update_chart`/`create_board`/`delete_chart` (need `dashboard:write`) | `run_sql`, `list_datasets`, `describe_dataset`, `get_lineage` |
//! | Data Scientist (`query:read, feature:write, notebook:run`) | same dashboard tools as Analyst | `run_sql`, `list_datasets`, `describe_dataset` |
//! | Approver (`agent:approve, policy:review`) | all dashboard tools; also `run_sql`/`list_datasets`/etc (no `query:read`/`catalog:read`) | — |
//! | Governance Admin (`policy:*, residency:*, audit:read`) | all dashboard tools; data tools too (no `query:read`/`catalog:read`/`lineage:read`) | — |
//! | Data Engineer (`pipeline:*, catalog:write, connector:manage`) | all dashboard tools | `trigger_lakehouse_build`, `get_build_status` (`pipeline:*`) |
//! | Dashboard Viewer (`dashboard:read`) | dashboard **writes** only | `describe_mart`, `list_charts`, `list_boards`, `suggest_dashboard` |
//! | Platform Admin (`*:*`) | nothing | everything |
//!
//! This is **correct** — it matches exactly what each role can already do
//! through the console (`POST /api/dashboard/specs` requires
//! `dashboard:write`, `policy.rs:226`) — but it is a visible regression in
//! what the copilot could do a moment ago, when it had no permission check
//! at all. Granting `dashboard:read` (or `dashboard:write`) more widely to
//! restore copilot dashboard access for these roles is a deliberate,
//! separate product decision — **not** made by this change. Do not add
//! role grants to work around this table.
//!
//! ## The vulnerability this closes
//!
//! Before this change, `run_tool` executed every tool with no permission
//! check at all — only the Ask/Build mode split gated writes. Since
//! `POST /api/dashboard/specs` (`create_chart`'s console equivalent)
//! requires `dashboard:write`, and only Platform Admin holds it, **an
//! Analyst who is refused a chart in the console (403) could create the
//! same chart by asking the copilot in Build mode** — the copilot was an
//! unguarded privilege-escalation path around `POLICY_TABLE`. See
//! [`tests::analyst_cannot_create_chart_the_console_would_refuse`] for the
//! regression test.

use lakehouse_auth::PermissionSet;
use lakehouse_store::PgPool;
use lakehouse_store::agents::{self as store_agents, NewApprovalRequest};
use serde_json::{Map, Value, json};

use super::registry::{Risk, ToolSpec};

/// The Ask-mode refusal shape (unchanged from before T0.2) — the model
/// sees this exact JSON when it (or a forged/XML-extracted call) tries a
/// non-[`Read`](Risk::Read) tool in `mode: "ask"`.
fn ask_mode_refusal(tool_name: &str) -> Value {
    json!({
        "error": "ditolak: mode ask tidak boleh menjalankan tool tulis",
        "refused": true,
        "tool": tool_name,
    })
}

/// The new permission-refusal shape (T0.2): the caller is authenticated
/// (`POST /api/ai/chat` is `Policy::RequiresAuth`) but their merged
/// [`PermissionSet`] lacks the tool's required `resource:action`.
fn permission_refusal(tool_name: &str, required: &str) -> Value {
    json!({
        "error": format!(
            "ditolak: kamu tidak punya izin '{required}' untuk menjalankan tool ini"
        ),
        "refused": true,
        "tool": tool_name,
        "reason": "permission",
        "required": required,
    })
}

/// T0.4: the pending-confirmation shape for a [`Risk::WriteLow`] tool call
/// that did not carry `"confirmed": true` in its arguments. The model
/// relays this to the user; the frontend renders `summary` with a Confirm /
/// Cancel pair, and Confirm re-sends the SAME call (via
/// `POST /api/ai/tool`, [`super::tool_call`]) with `confirmed: true` merged
/// into `args`. Nothing executes when this is returned.
fn needs_confirmation(spec: &ToolSpec, args: &Map<String, Value>) -> Value {
    json!({
        "needs_confirmation": true,
        "tool": spec.name,
        "args": args,
        "summary": summary_for(spec, args),
    })
}

/// T0.5: [`decide`]'s internal signal for a [`Risk::WriteHigh`] call that
/// has passed both the ask-mode and permission checks. This is NEVER
/// returned to the model/frontend as-is — it only tells the caller (the
/// chat loop / `POST /api/ai/tool`) "create the approval now, then build
/// the real `needs_approval` response" (see [`create_write_high_approval`]
/// and [`is_write_high_pending`]). Kept as a `decide`-produced value
/// (rather than a third enum variant) so `decide` stays a single pure,
/// synchronous function — approval creation is an async database write
/// and does not belong inside it.
fn write_high_pending_marker(tool_name: &str) -> Value {
    json!({
        "__write_high_pending__": true,
        "tool": tool_name,
    })
}

/// `true` for exactly the marker [`write_high_pending_marker`] produces —
/// distinguishes "this call needs a real `WriteHigh` approval created" from
/// every other [`decide`] result (a genuine refusal, or `needs_confirmation`).
#[must_use]
pub fn is_write_high_pending(value: &Value) -> bool {
    value.get("__write_high_pending__").and_then(Value::as_bool) == Some(true)
}

/// A short, deterministic Indonesian reason sentence for a [`Risk::WriteHigh`]
/// approval request — the same "never ask the LLM" discipline as
/// [`summary_for`], with its own match arms since a `WriteHigh` tool's
/// phrasing ("menghapus", "tidak dapat dibatalkan") reads differently from
/// a `WriteLow` confirmation.
fn reason_for_write_high(spec: &ToolSpec, args: &Map<String, Value>) -> String {
    let s = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("");
    match spec.name {
        "delete_chart" => format!(
            "Menghapus chart {} dari dashboard secara permanen.",
            s("id")
        ),
        _ => format!(
            "Menjalankan tool berisiko tinggi {} yang butuh persetujuan manusia.",
            spec.name
        ),
    }
}

/// Fixed Indonesian risk statement stored on every `WriteHigh` approval —
/// there is exactly one `WriteHigh` tool today (`delete_chart`); a future
/// one gets this same generic statement unless refined.
const WRITE_HIGH_RISK: &str =
    "Tindakan berisiko tinggi (WriteHigh): tidak dapat dibatalkan setelah dijalankan.";

/// T0.5: the real approvals-inbox flow for a [`Risk::WriteHigh`] call that
/// [`decide`] has already flagged with [`write_high_pending_marker`] (mode
/// and permission checks already passed). Creates the linked `agent_run`
/// (`waiting_approval`, attributed to `emp-copilot`) + `approval_item`
/// (`pending`) pair and returns the `needs_approval` JSON the model/frontend
/// see — **nothing executes here or until a human approves it** (see
/// `routes::agents::decide_approval`).
///
/// `redacted_args` MUST already be redacted (plan invariant 5 — see
/// `super::audit::redact`); this function stores them verbatim into
/// `approval_item.evidence` and the linked run's stored tool call.
///
/// With no Postgres pool configured, this fails closed: a `refused` result
/// is returned and NOTHING is created — there is nowhere durable to put an
/// approval a human could later see and act on, so silently allowing the
/// call through would be worse than refusing it.
pub async fn create_write_high_approval(
    pg: Option<&PgPool>,
    actor: &str,
    spec: &ToolSpec,
    raw_args: &Map<String, Value>,
    redacted_args: &Value,
    resource_id: Option<&str>,
) -> Value {
    let Some(pool) = pg else {
        return json!({
            "error": "ditolak: approval tidak tersedia (Postgres tidak dikonfigurasi)",
            "refused": true,
            "tool": spec.name,
        });
    };
    let reason = reason_for_write_high(spec, raw_args);
    let req = NewApprovalRequest {
        tool: spec.name,
        actor,
        resource: resource_id,
        reason: &reason,
        risk: WRITE_HIGH_RISK,
        redacted_args,
    };
    match store_agents::create_pending_approval(pool, req).await {
        Ok(created) => json!({
            "needs_approval": true,
            "approval_id": created.approval_id,
            "run_id": created.run_id,
            "tool": spec.name,
            "summary": reason,
        }),
        Err(err) => {
            tracing::warn!(
                %err,
                tool = spec.name,
                "copilot: failed to create WriteHigh approval; refusing the call"
            );
            json!({
                "error": "ditolak: gagal membuat approval",
                "refused": true,
                "tool": spec.name,
            })
        }
    }
}

/// A short, deterministic Indonesian sentence describing exactly what a
/// [`Risk::WriteLow`] tool call is about to do, generated from the tool
/// name and its (not yet stripped of `confirmed`) arguments — NEVER from
/// asking the LLM, so the summary can't drift from what will actually
/// execute. One arm per `WriteLow` tool; an unlisted tool (there is none
/// today, but a future `WriteLow` addition that forgets to extend this
/// falls through here rather than failing to compile) gets a generic
/// fallback.
fn summary_for(spec: &ToolSpec, args: &Map<String, Value>) -> String {
    let s = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("");
    match spec.name {
        "trigger_lakehouse_build" => {
            "Menjalankan build ulang lakehouse (Bronze → Silver → Gold).".to_owned()
        }
        "create_chart" => format!(
            "Membuat chart baru \"{}\" ({}) dari mart {}.",
            s("title"),
            s("kind"),
            s("mart")
        ),
        "update_chart" => format!(
            "Mengubah chart {} menjadi \"{}\" ({}).",
            s("id"),
            s("title"),
            s("kind")
        ),
        "create_board" => format!("Membuat board baru bernama \"{}\".", s("name")),
        _ => format!(
            "Menjalankan tool {} dengan argumen yang diberikan.",
            spec.name
        ),
    }
}

/// Strips the `confirmed` key that [`decide`] reads out of a `WriteLow`
/// call's args before those args ever reach a tool body — no tool
/// implementation should see it as a real argument (T0.4). Safe to call on
/// args for ANY tool, not only `WriteLow` ones: a no-op when the key is
/// absent.
#[must_use]
pub fn strip_confirmed(args: &Map<String, Value>) -> Map<String, Value> {
    let mut out = args.clone();
    out.remove("confirmed");
    out
}

/// Classifies a [`decide`]/[`decide_by_name`] result into the
/// `audit_event.outcome` string it corresponds to, for the "gate refused
/// or asked for confirmation, so there is no run to classify by success"
/// half of the audit story (`super::audit::record`'s callers use this for
/// `Some(refusal)`; the "actually ran" half is classified by the caller
/// from whether [`super::tools::run_tool`]'s result carries an `"error"`
/// key, matching the trace's existing `ok` computation).
#[must_use]
pub fn outcome_of(refusal: &Value) -> &'static str {
    if refusal.get("needs_confirmation").and_then(Value::as_bool) == Some(true) {
        "needs_confirmation"
    } else {
        "refused"
    }
}

/// D3: the dispatch-site enforcement of the tool registry's risk tier
/// **and** its required permission, called from inside the tool-execution
/// loop in [`super::chat`] for EVERY call about to be executed — not only
/// ones the model was offered in its `tools` schema. Advertising a
/// filtered tool list to the model (`ask` mode never lists non-`Read`
/// tools; [`super::registry::tool_schemas_for`] additionally drops tools
/// the principal lacks permission for) stops a well-behaved model from
/// choosing one, but does nothing about a `MiniMax` XML
/// `<invoke name="delete_chart">` embedded in free-form model text
/// (`parse_minimax_tool_calls` extracts calls from text unconditionally,
/// regardless of what was advertised) or a hallucinated `tool_calls`
/// entry — both reach the dispatch loop looking exactly like a legitimate
/// call. This function is the actual gate: `Some(refusal)` means "do not
/// call [`super::tools::run_tool`], return this JSON instead"; `None`
/// means the call is authorized for the current mode and principal, and
/// dispatch should proceed normally.
///
/// Checks run in order and the first that fails wins:
///
/// 1. Ask mode + non-[`Read`](Risk::Read) risk → [`ask_mode_refusal`]
///    (byte-identical to the pre-T0.2 behaviour; existing tests pin this).
/// 2. A non-empty [`ToolSpec::permission`] the principal's `perms` lacks →
///    [`permission_refusal`]. Checked for **every** risk tier, including
///    `Read` — a tool's permission requirement is not limited to writes
///    (e.g. `describe_mart` requires `dashboard:read`).
/// 3. Otherwise, `None` (allowed).
///
/// `perms: None` means "no principal was resolved for this request" —
/// see the module doc comment on the absent-principal decision. It is
/// treated as "authenticated but grants nothing": every tool with a
/// non-empty `permission` is refused, and every tool with an empty
/// `permission` (`""`, meaning "authenticated only" — see
/// [`ToolSpec::permission`]) is allowed, since `POST /api/ai/chat` itself
/// already requires authentication.
///
/// The refusal is a normal tool RESULT (`{"error": ..., "refused": true}`),
/// fed back into the conversation exactly like any other tool result — the
/// model sees it was denied and can tell the user, rather than the call
/// being dropped silently and the model assuming it happened.
///
/// An unregistered tool name is never refused here (`None`): it isn't a
/// write tool, it's not a tool at all, and `run_tool` reports it as
/// unrecognised instead.
///
/// # T0.4: risk-tier branching (checked last, only once mode+permission
/// both pass)
///
/// - [`Risk::Read`] → allowed (`None`).
/// - [`Risk::WriteLow`] → allowed if `args["confirmed"] == true`;
///   otherwise a [`needs_confirmation`] result — nothing executes, and the
///   model/frontend must resend the identical call with `confirmed: true`
///   (see `POST /api/ai/tool`, [`super::tool_call`]).
/// - [`Risk::WriteHigh`] → always [`write_high_pending_marker`] — an
///   internal signal, never returned to the model/frontend as-is, that
///   tells the caller to create a real approval via
///   [`create_write_high_approval`] (T0.5). This is deliberately NOT the
///   same as executing, and NOT the `WriteLow` confirmation path — a
///   `WriteHigh` call must never slip through by carrying `confirmed: true`.
#[must_use]
pub fn decide(
    is_build: bool,
    perms: Option<&PermissionSet>,
    spec: &ToolSpec,
    args: &Map<String, Value>,
) -> Option<Value> {
    if !is_build && spec.risk != Risk::Read {
        return Some(ask_mode_refusal(spec.name));
    }
    if !spec.permission.is_empty() {
        let granted = perms.is_some_and(|p| p.has(spec.permission));
        if !granted {
            return Some(permission_refusal(spec.name, spec.permission));
        }
    }
    match spec.risk {
        Risk::Read => None,
        Risk::WriteHigh => Some(write_high_pending_marker(spec.name)),
        Risk::WriteLow => {
            let confirmed = args.get("confirmed") == Some(&Value::Bool(true));
            if confirmed {
                None
            } else {
                Some(needs_confirmation(spec, args))
            }
        }
    }
}

/// [`decide`] by tool name, looking the [`ToolSpec`] up via
/// [`super::registry::find`] first. An unregistered name is never refused
/// here either, matching [`decide`]'s own contract for that case.
#[must_use]
pub fn decide_by_name(
    is_build: bool,
    perms: Option<&PermissionSet>,
    tool_name: &str,
    args: &Map<String, Value>,
) -> Option<Value> {
    let spec = super::registry::find(tool_name)?;
    decide(is_build, perms, spec, args)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::routes::ai::registry;

    /// The five former `WRITE_TOOLS` names, kept here (rather than pulled
    /// from `registry::TOOLS` by risk) so this test independently
    /// pins down the set the gate must refuse — matching the original
    /// `WRITE_TOOLS` regression test.
    const WRITE_TOOLS: [&str; 5] = [
        "trigger_lakehouse_build",
        "create_chart",
        "update_chart",
        "delete_chart",
        "create_board",
    ];

    /// A `PermissionSet` granting everything (`*:*`) — Platform Admin.
    fn admin_perms() -> PermissionSet {
        PermissionSet::parse("*:*")
    }

    /// A `PermissionSet` matching the seeded Analyst role.
    fn analyst_perms() -> PermissionSet {
        PermissionSet::parse("query:read, catalog:read, lineage:read")
    }

    fn spec(name: &str) -> &'static registry::ToolSpec {
        registry::find(name).expect("registered tool")
    }

    /// An empty args map — the common case for tests that don't care about
    /// `confirmed` (ask-mode and permission checks run before the
    /// risk-tier branch, so they're unaffected by it).
    fn no_args() -> Map<String, Value> {
        Map::new()
    }

    /// `{"confirmed": true}` — what a resent `WriteLow` call carries after
    /// the user hits Confirm (T0.4).
    fn confirmed_args() -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("confirmed".to_owned(), json!(true));
        m
    }

    /// D3: a forged write-tool invocation (whether a hallucinated
    /// `tool_calls` entry or a `MiniMax` XML `<invoke>` the model was never
    /// offered — [`decide`] doesn't care how the call arrived, only its
    /// name and the current mode) is refused, not executed, in `ask` mode,
    /// for a principal who otherwise has every permission.
    #[test]
    fn write_tool_is_refused_in_ask_mode() {
        let perms = admin_perms();
        for name in WRITE_TOOLS {
            let refused = decide(false, Some(&perms), spec(name), &no_args())
                .expect("write tool must be refused");
            assert_eq!(refused["refused"], json!(true));
            assert_eq!(refused["tool"], json!(name));
            assert!(refused.get("error").is_some());
            assert!(
                refused.get("reason").is_none(),
                "ask-mode refusal must not carry a permission `reason`"
            );
        }
    }

    /// T0.4: the same forged call, replayed in `mode: "build"` (the
    /// write-capable mode) with a principal that has every permission, is
    /// authorized for the four `WriteLow` tools ONLY once `confirmed: true`
    /// is present — [`decide`] returns `None` and dispatch proceeds to
    /// `run_tool` normally. `delete_chart` (`WriteHigh`) is covered
    /// separately: it is never allowed by this path.
    #[test]
    fn write_low_tool_confirmed_is_allowed_in_build_mode_for_admin() {
        let perms = admin_perms();
        for name in [
            "trigger_lakehouse_build",
            "create_chart",
            "update_chart",
            "create_board",
        ] {
            assert_eq!(
                decide(true, Some(&perms), spec(name), &confirmed_args()),
                None,
                "{name} must be allowed to dispatch in build mode for an admin once confirmed"
            );
        }
    }

    /// T0.4: a `WriteLow` call in build mode for an admin, WITHOUT
    /// `confirmed: true`, executes nothing — it gets the
    /// `needs_confirmation` shape instead of `None`.
    #[test]
    fn write_low_tool_without_confirmed_needs_confirmation_and_does_not_execute() {
        let perms = admin_perms();
        for name in [
            "trigger_lakehouse_build",
            "create_chart",
            "update_chart",
            "create_board",
        ] {
            let result = decide(true, Some(&perms), spec(name), &no_args())
                .expect("unconfirmed WriteLow call must not return None (i.e. must not execute)");
            assert_eq!(result["needs_confirmation"], json!(true));
            assert_eq!(result["tool"], json!(name));
            assert_eq!(result["args"], json!(no_args()));
            let summary = result["summary"]
                .as_str()
                .expect("summary must be a string");
            assert!(!summary.is_empty(), "{name} must have a non-empty summary");
        }
    }

    /// T0.4: `confirmed: false` (present but not `true`) is treated
    /// identically to absent — still needs confirmation.
    #[test]
    fn write_low_tool_confirmed_false_still_needs_confirmation() {
        let perms = admin_perms();
        let mut args = Map::new();
        args.insert("confirmed".to_owned(), json!(false));
        let result =
            decide(true, Some(&perms), spec("create_board"), &args).expect("must not execute");
        assert_eq!(result["needs_confirmation"], json!(true));
    }

    /// T0.4: the `create_chart` summary is generated deterministically from
    /// its own args (title/kind/mart), not the LLM.
    #[test]
    fn create_chart_summary_reflects_its_own_args() {
        let perms = admin_perms();
        let mut args = Map::new();
        args.insert("title".to_owned(), json!("Kunjungan Harian"));
        args.insert("kind".to_owned(), json!("bar"));
        args.insert("mart".to_owned(), json!("mart_wisman"));
        let result =
            decide(true, Some(&perms), spec("create_chart"), &args).expect("must not execute");
        let summary = result["summary"].as_str().expect("summary string");
        assert!(summary.contains("Kunjungan Harian"));
        assert!(summary.contains("bar"));
        assert!(summary.contains("mart_wisman"));
    }

    /// T0.5: `WriteHigh` (`delete_chart`) is never executed by this gate,
    /// even with `confirmed: true` — that flag only means something for
    /// `WriteLow`. It gets the internal [`write_high_pending_marker`]
    /// signal, distinct from both the ask-mode/permission refusal shape and
    /// `needs_confirmation` — the caller must turn it into a real approval
    /// via [`create_write_high_approval`], not treat it as a refusal.
    #[test]
    fn write_high_tool_is_pending_approval_even_when_confirmed() {
        let perms = admin_perms();
        let pending = decide(true, Some(&perms), spec("delete_chart"), &confirmed_args())
            .expect("WriteHigh must never return None (i.e. must never execute) from this gate");
        assert!(is_write_high_pending(&pending));
        assert!(pending.get("refused").is_none());
        assert!(pending.get("needs_confirmation").is_none());
    }

    /// A non-write tool is never refused for ask-mode reasons, in either
    /// mode, for a principal with every permission.
    #[test]
    fn read_only_tool_is_never_refused_for_admin() {
        let perms = admin_perms();
        assert_eq!(
            decide(false, Some(&perms), spec("run_sql"), &no_args()),
            None
        );
        assert_eq!(
            decide(true, Some(&perms), spec("run_sql"), &no_args()),
            None
        );
    }

    /// An unregistered name is never refused by the gate — it isn't a
    /// write tool, and `run_tool` (not the gate) is what reports it as
    /// unrecognised.
    #[test]
    fn unregistered_tool_is_never_refused() {
        assert_eq!(
            decide_by_name(false, None, "not_a_real_tool", &no_args()),
            None
        );
        assert_eq!(
            decide_by_name(true, None, "not_a_real_tool", &no_args()),
            None
        );
    }

    /// T0.2 invariant 1 (plan 3.6): an Analyst-like `PermissionSet`
    /// (`query:read, catalog:read, lineage:read` — no `dashboard:*`) is
    /// refused `create_chart` with `reason: "permission"`.
    #[test]
    fn analyst_is_refused_create_chart_for_permission() {
        let perms = analyst_perms();
        let refused = decide(true, Some(&perms), spec("create_chart"), &confirmed_args())
            .expect("must be refused");
        assert_eq!(refused["refused"], json!(true));
        assert_eq!(refused["tool"], json!("create_chart"));
        assert_eq!(refused["reason"], json!("permission"));
        assert_eq!(refused["required"], json!("dashboard:write"));
    }

    /// T0.2 invariant 2 (extended for T0.4): a Platform Admin
    /// `PermissionSet` (`*:*`) is allowed `create_chart` once confirmed.
    #[test]
    fn platform_admin_is_allowed_create_chart_when_confirmed() {
        let perms = admin_perms();
        assert_eq!(
            decide(true, Some(&perms), spec("create_chart"), &confirmed_args()),
            None
        );
    }

    /// Ask-mode precedes the permission check: a principal WITH
    /// `dashboard:write` (so the permission check alone would allow this
    /// call) is still refused in ask mode, and the refusal is the ask-mode
    /// shape (no `reason` field), not the permission shape.
    #[test]
    fn ask_mode_refusal_precedes_permission_check() {
        let perms = PermissionSet::parse("dashboard:write");
        let refused =
            decide(false, Some(&perms), spec("create_chart"), &no_args()).expect("must be refused");
        assert_eq!(
            refused,
            ask_mode_refusal("create_chart"),
            "ask-mode refusal must win even though the principal has the permission"
        );
        assert!(refused.get("reason").is_none());
    }

    /// Permission checks apply to `Read` tools too, not only writes:
    /// `describe_mart` requires `dashboard:read`, which the Analyst lacks.
    #[test]
    fn read_tool_missing_permission_is_refused() {
        let perms = analyst_perms();
        let read_spec = spec("describe_mart");
        assert_eq!(read_spec.risk, Risk::Read);
        let refused = decide(true, Some(&perms), read_spec, &no_args()).expect("must be refused");
        assert_eq!(refused["reason"], json!("permission"));
        assert_eq!(refused["required"], json!("dashboard:read"));
    }

    /// A tool with an empty `permission` (`get_quality`: no narrower route
    /// permission than `RequiresAuth`) is allowed for any authenticated
    /// principal, even one with no grants at all.
    #[test]
    fn empty_permission_tool_is_allowed_for_any_authenticated_principal() {
        let perms = PermissionSet::default();
        let empty_perm_spec = spec("get_quality");
        assert_eq!(empty_perm_spec.permission, "");
        assert_eq!(
            decide(true, Some(&perms), empty_perm_spec, &no_args()),
            None
        );
    }

    /// Named regression test for the privilege-escalation this task
    /// closes: `POST /api/dashboard/specs` (`create_chart`'s console
    /// equivalent) requires `dashboard:write` (`policy.rs:226`); of the
    /// seven seeded roles only Platform Admin holds it
    /// (`0002_seed_identity.sql`). Before T0.2, `run_tool` executed
    /// `create_chart` for ANY authenticated Build-mode principal with no
    /// permission check at all, so an Analyst who is refused a chart in
    /// the console (403) could create the same chart by asking the
    /// copilot instead — the copilot was an unguarded path around
    /// `POLICY_TABLE`. This test pins that `decide` now refuses exactly
    /// that call.
    #[test]
    fn analyst_cannot_create_chart_the_console_would_refuse() {
        let perms = analyst_perms();
        let refused = decide(true, Some(&perms), spec("create_chart"), &confirmed_args())
            .expect("an Analyst must not be able to create a chart via the copilot");
        assert_eq!(refused["reason"], json!("permission"));
        assert_eq!(refused["required"], json!("dashboard:write"));
    }

    /// Absent-principal case (fail closed): with no principal at all,
    /// every tool with a non-empty permission is refused, even in build
    /// mode.
    #[test]
    fn absent_principal_is_refused_every_permissioned_tool() {
        for t in registry::TOOLS.iter().filter(|t| !t.permission.is_empty()) {
            let refused = decide(true, None, t, &confirmed_args());
            assert!(
                refused.is_some(),
                "{} must be refused with no principal",
                t.name
            );
            assert_eq!(refused.unwrap()["reason"], json!("permission"));
        }
    }

    /// Absent-principal case: a tool with an empty `permission` is still
    /// allowed with no principal — `chat`'s route policy is
    /// `RequiresAuth`, so in practice this never happens, but the gate's
    /// own contract (documented on [`decide`]) is that an absent principal
    /// behaves like "authenticated, no grants", not like Ask mode.
    #[test]
    fn absent_principal_is_allowed_empty_permission_tool() {
        assert_eq!(decide(true, None, spec("get_quality"), &no_args()), None);
    }

    /// [`strip_confirmed`] removes only the `confirmed` key, leaving every
    /// other argument untouched, and is a no-op when the key is absent.
    #[test]
    fn strip_confirmed_removes_only_that_key() {
        let mut args = Map::new();
        args.insert("confirmed".to_owned(), json!(true));
        args.insert("title".to_owned(), json!("T"));
        let stripped = strip_confirmed(&args);
        assert!(!stripped.contains_key("confirmed"));
        assert_eq!(stripped.get("title"), Some(&json!("T")));

        let no_confirmed = strip_confirmed(&no_args());
        assert!(no_confirmed.is_empty());
    }

    /// [`outcome_of`] classifies the two refusal shapes [`decide`] can
    /// produce: `needs_confirmation` vs everything else ("refused").
    #[test]
    fn outcome_of_classifies_needs_confirmation_vs_refused() {
        let perms = admin_perms();
        let needs_confirm = decide(true, Some(&perms), spec("create_board"), &no_args())
            .expect("must need confirmation");
        assert_eq!(outcome_of(&needs_confirm), "needs_confirmation");

        let refused = decide(false, Some(&perms), spec("create_board"), &no_args())
            .expect("must be refused (ask mode)");
        assert_eq!(outcome_of(&refused), "refused");
    }

    /// The `WriteHigh` pending marker is not a [`decide`]/[`outcome_of`]
    /// refusal at all — callers must check [`is_write_high_pending`]
    /// BEFORE calling [`outcome_of`] on a `decide` result (the chat loop
    /// and `POST /api/ai/tool` dispatch do exactly that).
    #[test]
    fn write_high_pending_is_not_classified_by_outcome_of() {
        let perms = admin_perms();
        let pending = decide(true, Some(&perms), spec("delete_chart"), &confirmed_args())
            .expect("must be pending approval");
        assert!(is_write_high_pending(&pending));
    }
}
