//! `undo_last_change` (`AI-21`): take a dashboard back to how it was before the
//! assistant's last change to it.
//!
//! The walk through the history is [`lakehouse_bi::versions::undo_target`]; the
//! write is the console's own revert, [`restore_version`], as the signed-in
//! person, so the edit check and every rule a revert checks run here too. The
//! restore is labelled `Assistant undo of v<n>`, so the next undo skips it and
//! goes one assistant change further back.
//!
//! Scope is board content only (charts, layout, filters, tabs, theme): a
//! version holds nothing else. Versions written before the assistant marked its
//! changes carry no marker and are refused, not guessed at.

use lakehouse_auth::Principal;
use lakehouse_bi::access::Level;
use lakehouse_bi::store::DEFAULT_BOARD_ID;
use lakehouse_bi::versions::{self, ASSISTANT_UNDO_PREFIX, NoUndo};
use lakehouse_core::ApiError;
use serde_json::{Map, Value, json};

use super::api_result_to_value;
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::dashboard_access::{require_board_level, require_can_ever, scope_for_write};
use crate::routes::dashboard_folders::classify_ch_error;
use crate::routes::dashboard_organise::{author_of, require_board, restore_version};
use crate::state::AppState;

/// The refusal when the newest change is not the assistant's to undo (`AI-21`
/// decision 2). The sentence after it says what the undo can reach.
const SOMEONE_ELSE: &str = "Someone changed this dashboard after the assistant; use Versions in the dashboard menu. Only changes the assistant made since it could undo them are undone here: changes made before the assistant could undo are not.";

/// The refusal when no kept version is left to undo.
const NOTHING_TO_UNDO: &str = "The assistant has no change to this dashboard to undo. Changes made before the assistant could undo are not undone here.";

/// The refusal when the assistant's change is the oldest kept version.
const NOTHING_EARLIER: &str = "The assistant's change is the oldest kept version, so there is no earlier version to go back to; use Versions in the dashboard menu.";

/// The refusal with no signed-in person attached.
const NO_PERSON: &str =
    "undo_last_change changes a dashboard as the signed-in person, and there is none.";

/// `undo_last_change`: revert the board to the version before the assistant's
/// newest change that has not been undone.
pub(super) async fn undo_last_change(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let Some(principal) = principal else {
        return json!({ "error": NO_PERSON });
    };
    let board = args
        .get("board")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned();
    if board.is_empty() {
        return json!({ "error": "board is required" });
    }
    if board == DEFAULT_BOARD_ID {
        return json!({ "error": "The built-in dashboard keeps no history." });
    }
    api_result_to_value(undo(state, principal, &board).await).await
}

/// The checks, the walk and the restore, in the order the console runs them.
async fn undo(state: &AppState, principal: &Principal, board: &str) -> ApiResult<ApiJson<Value>> {
    require_can_ever(state, principal, Level::Edit).await?;
    let scope = scope_for_write(state, principal).await?;
    let ch = &state.clickhouse;
    require_board(ch, board, &scope.viewer).await?;
    require_board_level(state, &scope, board, Level::Edit).await?;
    let stamps = versions::stamps(ch, board)
        .await
        .map_err(|err| classify_ch_error(&err))?;
    let author = author_of(principal);
    let target = match versions::undo_target(&stamps, &author.id) {
        Ok(target) => target,
        Err(NoUndo::NotTheAssistants) => {
            return Err(ApiError::Unprocessable(SOMEONE_ELSE.to_owned()).into());
        }
        Err(NoUndo::Nothing) => {
            return Err(ApiError::Unprocessable(NOTHING_TO_UNDO.to_owned()).into());
        }
    };
    let Some(back) = versions::version_before(&stamps, target) else {
        return Err(ApiError::Unprocessable(NOTHING_EARLIER.to_owned()).into());
    };
    restore_version(
        state,
        principal,
        board,
        back,
        &format!("{ASSISTANT_UNDO_PREFIX}{target}"),
    )
    .await
}
