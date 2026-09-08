//! The Ask-mode gate: dispatch-site enforcement that a `mode: "ask"` chat
//! session can never execute a non-[`Read`](super::registry::Risk::Read)
//! tool call.
//!
//! This replaces the old `WRITE_TOOLS: [&str; 5]` array + `write_tool_refusal`
//! pair that used to live directly in `ai.rs`: the same semantics, the
//! same refusal JSON shape, expressed against [`super::registry`] instead
//! of a hand-maintained name list.

use serde_json::{Value, json};

use super::registry::{self, Risk};

/// D3: the dispatch-site enforcement of the tool registry's risk tiers,
/// called from inside the tool-execution loop in [`super::chat`] for
/// EVERY call about to be executed — not only ones the model was offered
/// in its `tools` schema. Advertising a filtered tool list to the model
/// (`ask` mode never lists non-[`Read`](Risk::Read) tools) stops a
/// well-behaved model from choosing one, but does nothing about a
/// `MiniMax` XML `<invoke name="delete_chart">` embedded in free-form
/// model text (`parse_minimax_tool_calls` extracts calls from text
/// unconditionally, regardless of what was advertised) or a hallucinated
/// `tool_calls` entry — both reach the dispatch loop looking exactly like
/// a legitimate call. This function is the actual gate: `Some(refusal)`
/// means "do not call [`super::tools::run_tool`], return this JSON
/// instead"; `None` means the call is authorized for the current mode and
/// dispatch should proceed normally.
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
pub fn refusal(is_build: bool, tool_name: &str) -> Option<Value> {
    if is_build {
        return None;
    }
    let is_write = registry::find(tool_name).is_some_and(|spec| spec.risk != Risk::Read);
    if !is_write {
        return None;
    }
    Some(json!({
        "error": "ditolak: mode ask tidak boleh menjalankan tool tulis",
        "refused": true,
        "tool": tool_name,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

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

    /// D3: a forged write-tool invocation (whether a hallucinated
    /// `tool_calls` entry or a `MiniMax` XML `<invoke>` the model was never
    /// offered — [`refusal`] doesn't care how the call arrived, only its
    /// name and the current mode) is refused, not executed, in `ask` mode.
    #[test]
    fn write_tool_is_refused_in_ask_mode() {
        for name in WRITE_TOOLS {
            let refused = refusal(false, name).expect("write tool must be refused in ask mode");
            assert_eq!(refused["refused"], json!(true));
            assert_eq!(refused["tool"], json!(name));
            assert!(refused.get("error").is_some());
        }
    }

    /// D3: the same forged call, replayed in `mode: "build"` (the
    /// write-capable mode), is authorized — [`refusal`] returns `None`
    /// and dispatch proceeds to `run_tool` normally.
    #[test]
    fn write_tool_is_allowed_in_build_mode() {
        for name in WRITE_TOOLS {
            assert_eq!(
                refusal(true, name),
                None,
                "{name} must be allowed to dispatch in build mode"
            );
        }
    }

    /// A non-write tool is never refused, in either mode.
    #[test]
    fn read_only_tool_is_never_refused() {
        assert_eq!(refusal(false, "run_sql"), None);
        assert_eq!(refusal(true, "run_sql"), None);
    }

    /// An unregistered name is never refused by the gate — it isn't a
    /// write tool, and `run_tool` (not the gate) is what reports it as
    /// unrecognised.
    #[test]
    fn unregistered_tool_is_never_refused() {
        assert_eq!(refusal(false, "not_a_real_tool"), None);
        assert_eq!(refusal(true, "not_a_real_tool"), None);
    }
}
