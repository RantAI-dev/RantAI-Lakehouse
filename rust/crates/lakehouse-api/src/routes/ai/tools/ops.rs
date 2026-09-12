//! Operations tools (T2.3 of the copilot-operations-handover plan):
//! `list_workloads`, `kill_query`.
//!
//! Both call the REAL `routes::ops::{get,cancel_workload}` handler (via
//! [`super::response_to_value`]) — `kill_query` in particular goes through
//! `routes::ops::cancel_workload`, which runs a genuine `KILL QUERY`
//! against `ClickHouse`, never a second, hand-rolled `KILL QUERY` string
//! built here.

use axum::extract::{Path, State};
use serde_json::{Map, Value, json};

use super::{arg_str, response_to_value};
use crate::state::AppState;

pub(super) async fn list_workloads(state: &AppState) -> Value {
    response_to_value(
        crate::routes::ops::get(State(state.clone()), Path("workloads".to_owned())).await,
    )
    .await
}

pub(super) async fn kill_query(state: &AppState, args: &Map<String, Value>) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id wajib diisi" });
    }
    response_to_value(crate::routes::ops::cancel_workload(State(state.clone()), Path(id)).await)
        .await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use super::*;
    use crate::config::Config;

    fn state() -> AppState {
        AppState::new(Config::from_map(&HashMap::new()).unwrap())
    }

    #[tokio::test]
    async fn kill_query_requires_id() {
        let s = state();
        assert_eq!(
            kill_query(&s, &Map::new()).await,
            json!({ "error": "id wajib diisi" })
        );
    }
}
