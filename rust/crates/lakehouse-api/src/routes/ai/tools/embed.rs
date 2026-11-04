//! The signed-embed tools (`BI-26` slice 2 D4, with `AI-11-AC3`): make a
//! dashboard embeddable with approval, set each filter's mode, withdraw every
//! token, and describe the snippet.
//!
//! Two rules shape this module. The first is the manage guard: the console's
//! embed routes are signed-in with `manage` checked in the handler, and every
//! tool here runs the same one guard (`public_links::managed_board`, `BI-26`
//! review fix R6) as itself, so it is judged as the signed-in person and never
//! through a `dashboard:write` gate. The second is that the assistant never
//! signs a token and never shows one: nothing here reads `sign_embed`,
//! `embed_info_body` or `config.embed_secret` beyond checking that it is set
//! (`BI-26` slice 2 D7). The customer's server signs the token with the
//! secret; the console's Share > Embed has the copyable snippet built in
//! `share-embed.ts` (`BI-26` slice 2 D8).
//!
//! The writes go through the console's own handlers: `enable_embed` and
//! `withdraw_embed_tokens` call `dashboard::boards_update` (`level_needed` and
//! `apply_embed_access` then apply), and `set_embed_filter_modes` calls
//! `embed::put_settings` (whose audit record is written there).

use axum::body::Bytes;
use axum::extract::State;
use lakehouse_auth::Principal;
use lakehouse_bi::embed_access::FilterMode;
use serde_json::{Map, Value, json};

use super::{api_result_to_value, arg_str};
use crate::state::AppState;

/// The placeholder in the address template, the same words the console's Share
/// dialog uses (`embed-settings-panel.tsx`).
const TOKEN_PLACEHOLDER: &str = "<TOKEN FROM YOUR SERVER>";

/// What `get_embed_snippet` tells the model about the copyable snippet
/// (`BI-26` slice 2 D8): the full ten-line snippet stays the Share dialog's,
/// built in `share-embed.ts`, so a second builder does not have to be kept in
/// step with it (rule 4).
const SNIPPET_NOTE: &str = "Share > Embed has the copyable snippet with the resize listener. The assistant never signs or shows a token: the customer's server signs it with the embedding secret.";

/// Whether `name` is one of these tools (one guard arm in `run_tool`).
pub(super) fn handles(name: &str) -> bool {
    matches!(
        name,
        "enable_embed" | "set_embed_filter_modes" | "withdraw_embed_tokens" | "get_embed_snippet"
    )
}

pub(super) async fn run(
    state: &AppState,
    principal: Option<&Principal>,
    name: &str,
    args: &Map<String, Value>,
) -> Value {
    match name {
        "enable_embed" => enable_embed(state, principal, args).await,
        "set_embed_filter_modes" => set_embed_filter_modes(state, principal, args).await,
        "withdraw_embed_tokens" => withdraw_embed_tokens(state, principal, args).await,
        "get_embed_snippet" => get_embed_snippet(state, principal, args).await,
        other => json!({ "error": format!("unknown tool: {other}") }),
    }
}

/// `enable_embed {board, sites}`: turn embedding on and ADD the sites to the
/// ones already allowed (`BI-26` slice 2 D6). The sites are read at execution,
/// from the same board the manage guard resolved, so two approvals in a row
/// cannot lose an earlier site; the result says which sites are listed now.
async fn enable_embed(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let board = match super::managed_embed_board(state, principal, args).await {
        Ok(board) => board,
        Err(error) => return error,
    };
    if let Err(error) = super::embed_configured(state) {
        return error;
    }
    let Some(principal) = principal else {
        return json!({ "error": "enable_embed changes a dashboard as the signed-in person, and there is none." });
    };
    let sites = match super::arg_string_list(args, "sites") {
        Ok(sites) => sites,
        Err(error) => return error,
    };
    let sites = match lakehouse_bi::embed_access::validate_origins(&sites) {
        Ok(sites) => sites,
        Err(error) => return json!({ "error": error }),
    };
    // D6: the list the board holds now, and the new sites after it.
    let merged = match super::merged_origins(&board, &sites) {
        Ok(merged) => merged,
        Err(error) => return json!({ "error": error }),
    };
    let body = json!({ "id": board.id, "embed": true, "embedOrigins": merged });
    let result = api_result_to_value(
        crate::routes::dashboard::boards_update(
            State(state.clone()),
            axum::Extension(principal.clone()),
            Bytes::from(serde_json::to_vec(&body).unwrap_or_default()),
        )
        .await,
    )
    .await;
    if result.get("error").is_some() {
        return result;
    }
    json!({
        "board": board.id,
        "enabled": true,
        "sites": result.get("embedOrigins").cloned().unwrap_or(json!(merged)),
        "note": "Anyone whose server signs a token for one of these sites can show this dashboard as the fixed viewer role. Share > Embed has the copyable snippet.",
    })
}

/// `set_embed_filter_modes {board, modes}`: write each filter's embed mode
/// through `embed::put_settings`, so its column and operator checks and its
/// `dashboard.embed_settings` audit record apply (`BI-26` slice 2 D4).
async fn set_embed_filter_modes(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let board = match super::managed_embed_board(state, principal, args).await {
        Ok(board) => board,
        Err(error) => return error,
    };
    if let Err(error) = super::embed_configured(state) {
        return error;
    }
    let Some(principal) = principal else {
        return json!({ "error": "set_embed_filter_modes changes a dashboard as the signed-in person, and there is none." });
    };
    let modes = match super::arg_filter_modes(args) {
        Ok(modes) => modes,
        Err(error) => return error,
    };
    if let Err(err) = crate::routes::embed::check_filter_modes(&board, &modes) {
        return json!({ "error": err.to_string() });
    }
    let body = json!({ "board": board.id, "filterModes": modes });
    let result = api_result_to_value(
        crate::routes::embed::put_settings(
            State(state.clone()),
            axum::Extension(principal.clone()),
            Bytes::from(serde_json::to_vec(&body).unwrap_or_default()),
        )
        .await,
    )
    .await;
    if result.get("error").is_some() {
        return result;
    }
    json!({
        "board": board.id,
        "filterModes": modes,
        "note": "An editable filter can be changed by the embed's viewer; a locked one must be set by the token; a hidden one is not shown and the saved value applies.",
    })
}

/// `withdraw_embed_tokens {board}`: withdraw every token issued so far through
/// `boards_update`'s `embedRevokeAll` (`SEC-12`). The gate has already asked
/// for the chat's Confirm (`WriteLow`: it only narrows access). The answer
/// reports the time, never a token.
async fn withdraw_embed_tokens(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let board = match super::managed_embed_board(state, principal, args).await {
        Ok(board) => board,
        Err(error) => return error,
    };
    if let Err(error) = super::embed_configured(state) {
        return error;
    }
    let Some(principal) = principal else {
        return json!({ "error": "withdraw_embed_tokens changes a dashboard as the signed-in person, and there is none." });
    };
    let body = json!({ "id": board.id, "embedRevokeAll": true });
    let result = api_result_to_value(
        crate::routes::dashboard::boards_update(
            State(state.clone()),
            axum::Extension(principal.clone()),
            Bytes::from(serde_json::to_vec(&body).unwrap_or_default()),
        )
        .await,
    )
    .await;
    if result.get("error").is_some() {
        return result;
    }
    json!({
        "board": board.id,
        "withdrawn": true,
        "embedRevokedBefore": result.get("embedRevokedBefore").cloned().unwrap_or(Value::Null),
    })
}

/// `get_embed_snippet {board, chart?}`: the allowlist of facts `BI-26` slice 2
/// D7 permits. There is no `sampleToken`, no saved value of a locked filter and
/// no secret; a locked filter appears with a placeholder the customer's server
/// replaces with its own value.
async fn get_embed_snippet(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let board = match super::managed_embed_board(state, principal, args).await {
        Ok(board) => board,
        Err(error) => return error,
    };
    if let Err(error) = super::embed_configured(state) {
        return error;
    }
    let chart = arg_str(args, "chart");
    let resource = if chart.trim().is_empty() {
        json!({ "dashboard": board.id })
    } else {
        json!({ "chart": chart })
    };
    // D7: each locked column with a placeholder, never its saved value.
    let locked: Vec<Value> = board
        .filters
        .iter()
        .flatten()
        .filter(|f| !f.column.is_empty())
        .filter(|f| board.embed_access.mode_of(&f.column) == FilterMode::Locked)
        .map(|f| json!({ "column": f.column, "value": "<value your server sets>" }))
        .collect();
    json!({
        "board": board.id,
        "embedding": board.embed_enabled == Some(true),
        "sites": board.embed_access.origins,
        "resource": resource,
        "addressTemplate": format!("/embed/signed/{TOKEN_PLACEHOLDER}"),
        "lookOptions": ["theme=light|dark|dashboard", "logo=none", "background=transparent"],
        "lockedFilters": locked,
        "note": SNIPPET_NOTE,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn handles_only_the_four_embed_tools() {
        for name in [
            "enable_embed",
            "set_embed_filter_modes",
            "withdraw_embed_tokens",
            "get_embed_snippet",
        ] {
            assert!(handles(name), "{name}");
        }
        assert!(!handles("list_boards"));
    }

    /// `BI-26` slice 2 D7: the snippet names the placeholder, never a real
    /// token, and points at Share > Embed for the copyable snippet (D8).
    #[test]
    fn the_address_template_names_the_placeholder_and_the_share_dialog() {
        assert!(TOKEN_PLACEHOLDER.contains("TOKEN FROM YOUR SERVER"));
        assert!(SNIPPET_NOTE.contains("Share > Embed"));
        assert!(SNIPPET_NOTE.contains("never signs"));
    }
}
