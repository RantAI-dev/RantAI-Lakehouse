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
use serde_json::{Value, json};

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
#[must_use]
pub fn decide(is_build: bool, perms: Option<&PermissionSet>, spec: &ToolSpec) -> Option<Value> {
    if !is_build && spec.risk != Risk::Read {
        return Some(ask_mode_refusal(spec.name));
    }
    if !spec.permission.is_empty() {
        let granted = perms.is_some_and(|p| p.has(spec.permission));
        if !granted {
            return Some(permission_refusal(spec.name, spec.permission));
        }
    }
    None
}

/// [`decide`] by tool name, looking the [`ToolSpec`] up via
/// [`super::registry::find`] first. An unregistered name is never refused
/// here either, matching [`decide`]'s own contract for that case.
#[must_use]
pub fn decide_by_name(
    is_build: bool,
    perms: Option<&PermissionSet>,
    tool_name: &str,
) -> Option<Value> {
    let spec = super::registry::find(tool_name)?;
    decide(is_build, perms, spec)
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

    /// D3: a forged write-tool invocation (whether a hallucinated
    /// `tool_calls` entry or a `MiniMax` XML `<invoke>` the model was never
    /// offered — [`decide`] doesn't care how the call arrived, only its
    /// name and the current mode) is refused, not executed, in `ask` mode,
    /// for a principal who otherwise has every permission.
    #[test]
    fn write_tool_is_refused_in_ask_mode() {
        let perms = admin_perms();
        for name in WRITE_TOOLS {
            let refused =
                decide(false, Some(&perms), spec(name)).expect("write tool must be refused");
            assert_eq!(refused["refused"], json!(true));
            assert_eq!(refused["tool"], json!(name));
            assert!(refused.get("error").is_some());
            assert!(
                refused.get("reason").is_none(),
                "ask-mode refusal must not carry a permission `reason`"
            );
        }
    }

    /// D3: the same forged call, replayed in `mode: "build"` (the
    /// write-capable mode) with a principal that has every permission, is
    /// authorized — [`decide`] returns `None` and dispatch proceeds to
    /// `run_tool` normally.
    #[test]
    fn write_tool_is_allowed_in_build_mode_for_admin() {
        let perms = admin_perms();
        for name in WRITE_TOOLS {
            assert_eq!(
                decide(true, Some(&perms), spec(name)),
                None,
                "{name} must be allowed to dispatch in build mode for an admin"
            );
        }
    }

    /// A non-write tool is never refused for ask-mode reasons, in either
    /// mode, for a principal with every permission.
    #[test]
    fn read_only_tool_is_never_refused_for_admin() {
        let perms = admin_perms();
        assert_eq!(decide(false, Some(&perms), spec("run_sql")), None);
        assert_eq!(decide(true, Some(&perms), spec("run_sql")), None);
    }

    /// An unregistered name is never refused by the gate — it isn't a
    /// write tool, and `run_tool` (not the gate) is what reports it as
    /// unrecognised.
    #[test]
    fn unregistered_tool_is_never_refused() {
        assert_eq!(decide_by_name(false, None, "not_a_real_tool"), None);
        assert_eq!(decide_by_name(true, None, "not_a_real_tool"), None);
    }

    /// T0.2 invariant 1 (plan 3.6): an Analyst-like `PermissionSet`
    /// (`query:read, catalog:read, lineage:read` — no `dashboard:*`) is
    /// refused `create_chart` with `reason: "permission"`.
    #[test]
    fn analyst_is_refused_create_chart_for_permission() {
        let perms = analyst_perms();
        let refused = decide(true, Some(&perms), spec("create_chart")).expect("must be refused");
        assert_eq!(refused["refused"], json!(true));
        assert_eq!(refused["tool"], json!("create_chart"));
        assert_eq!(refused["reason"], json!("permission"));
        assert_eq!(refused["required"], json!("dashboard:write"));
    }

    /// T0.2 invariant 2: a Platform Admin `PermissionSet` (`*:*`) is
    /// allowed `create_chart`.
    #[test]
    fn platform_admin_is_allowed_create_chart() {
        let perms = admin_perms();
        assert_eq!(decide(true, Some(&perms), spec("create_chart")), None);
    }

    /// Ask-mode precedes the permission check: a principal WITH
    /// `dashboard:write` (so the permission check alone would allow this
    /// call) is still refused in ask mode, and the refusal is the ask-mode
    /// shape (no `reason` field), not the permission shape.
    #[test]
    fn ask_mode_refusal_precedes_permission_check() {
        let perms = PermissionSet::parse("dashboard:write");
        let refused = decide(false, Some(&perms), spec("create_chart")).expect("must be refused");
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
        let refused = decide(true, Some(&perms), read_spec).expect("must be refused");
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
        assert_eq!(decide(true, Some(&perms), empty_perm_spec), None);
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
        let refused = decide(true, Some(&perms), spec("create_chart"))
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
            let refused = decide(true, None, t);
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
        assert_eq!(decide(true, None, spec("get_quality")), None);
    }
}
