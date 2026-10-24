//! The assistant's tools for the query limits (`BI-5`, plan D7):
//! `get_query_limits` and `stop_query`.
//!
//! `get_query_limits` answers the limits that apply to the person asking (the
//! same `effective` object `GET /api/settings/limits` gives) and where to
//! change them. The assistant cannot change limits: that is an
//! administrator's setting in Admin > Settings > Query limits.
//!
//! `stop_query` ends the person's own running queries on the engine, through
//! the same builder the Stop button's route uses (`ops::KillTarget`). It is
//! `Risk::Read` although it kills: it can only end the caller's own work (the
//! principal part of the pattern is the authenticated person's id, never an
//! argument), it writes nothing, and a stopped query is simply asked again.
//! With a `runId` it ends that run; without one it ends every query of the
//! person that is running now.

use lakehouse_auth::Principal;
use serde_json::{Map, Value, json};

use super::arg_str;
use crate::routes::ops::{KillTarget, kill_query_sql};
use crate::state::AppState;
use crate::upstream_error;

/// Whether `name` is one of the limits tools (one guard arm in `run_tool`).
pub(super) fn handles(name: &str) -> bool {
    matches!(name, "get_query_limits" | "stop_query")
}

pub(super) async fn run(
    state: &AppState,
    principal: Option<&Principal>,
    name: &str,
    args: &Map<String, Value>,
) -> Value {
    let Some(principal) = principal else {
        return json!({ "error": "this needs a signed-in user; this call came from a schedule" });
    };
    if name == "get_query_limits" {
        get_query_limits(state, principal).await
    } else {
        stop_query(state, principal, args).await
    }
}

async fn get_query_limits(state: &AppState, principal: &Principal) -> Value {
    match crate::query_limits::snapshot(state).await {
        Ok(snap) => json!({
            "limits": snap.effective_for(principal).to_json(),
            "note": "An administrator changes these in Admin > Settings > Query limits; \
                     the assistant cannot.",
        }),
        // The cause was logged under a reference by `snapshot`.
        Err(_) => json!({ "error": "The query limits could not be read." }),
    }
}

async fn stop_query(state: &AppState, principal: &Principal, args: &Map<String, Value>) -> Value {
    let run_id = arg_str(args, "runId");
    let target = if run_id.is_empty() {
        KillTarget::own_all(principal.id.uuid())
    } else if let Some(target) = KillTarget::own_run(principal.id.uuid(), &run_id) {
        target
    } else {
        return json!({ "error": "runId must be a UUID." });
    };
    let ch = &state.clickhouse;
    let counted = match ch.rows(&target.count_sql(), None).await {
        Ok(rows) => rows,
        Err(err) => {
            return upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json();
        }
    };
    let stopped = counted
        .first()
        .and_then(|row| row.get("n"))
        .and_then(|n| {
            n.as_u64()
                .or_else(|| n.as_str().and_then(|t| t.parse().ok()))
        })
        .unwrap_or(0);
    if stopped > 0
        && let Err(err) = ch.exec(&kill_query_sql(&target), None).await
    {
        return upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json();
    }
    json!({ "stopped": stopped })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::config::Config;

    fn person(id: uuid::Uuid) -> Principal {
        Principal {
            id: lakehouse_auth::PrincipalId::User(id),
            tenant_ids: Vec::new(),
            display_name: "Test".to_owned(),
            permissions: lakehouse_auth::PermissionSet::parse(""),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: vec!["Analyst".to_owned()],
        }
    }

    fn state_on(url: Option<&str>) -> AppState {
        let mut env = HashMap::new();
        if let Some(url) = url {
            env.insert("CH_URL".to_owned(), url.to_owned());
        }
        AppState::new(Config::from_map(&env).unwrap())
    }

    #[tokio::test]
    async fn get_query_limits_answers_the_persons_own_limits_and_says_who_changes_them() {
        let state = state_on(None);
        let got = run(
            &state,
            Some(&person(uuid::Uuid::nil())),
            "get_query_limits",
            &Map::new(),
        )
        .await;
        assert_eq!(got["limits"]["timeoutSeconds"], 60);
        assert_eq!(got["limits"]["chartRows"], 2000);
        assert_eq!(got["limits"]["source"], json!({"kind": "install"}));
        assert!(got["note"].as_str().unwrap().contains("Admin > Settings"));
    }

    #[tokio::test]
    async fn both_tools_need_a_signed_in_person() {
        let state = state_on(None);
        for name in ["get_query_limits", "stop_query"] {
            let got = run(&state, None, name, &Map::new()).await;
            assert!(
                got["error"].as_str().unwrap().contains("signed-in"),
                "{name}"
            );
        }
    }

    #[tokio::test]
    async fn stop_query_kills_only_the_callers_own_queries() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("count()"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"meta":[{"name":"n","type":"UInt64"}],"data":[{"n":"2"}],"rows":1}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("KILL QUERY"))
            .respond_with(ResponseTemplate::new(200).set_body_string("Ok."))
            .mount(&server)
            .await;
        let state = state_on(Some(&server.uri()));
        let me = uuid::Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap();

        let all = run(&state, Some(&person(me)), "stop_query", &Map::new()).await;
        assert_eq!(all, json!({ "stopped": 2 }));
        let mut args = Map::new();
        args.insert(
            "runId".to_owned(),
            json!("22222222-2222-4222-8222-222222222222"),
        );
        let one = run(&state, Some(&person(me)), "stop_query", &args).await;
        assert_eq!(one, json!({ "stopped": 2 }));

        let kills: Vec<String> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .filter(|b| b.starts_with("KILL"))
            .collect();
        assert_eq!(
            kills,
            [
                "KILL QUERY WHERE query_id LIKE '11111111-1111-4111-8111-111111111111:%' ASYNC",
                "KILL QUERY WHERE query_id LIKE '11111111-1111-4111-8111-111111111111:22222222-2222-4222-8222-222222222222:%' ASYNC",
            ]
        );
    }

    #[tokio::test]
    async fn a_run_id_that_is_not_a_uuid_is_refused_before_the_engine() {
        let state = state_on(Some("http://127.0.0.1:1"));
        let mut args = Map::new();
        args.insert("runId".to_owned(), json!("x' OR '1'='1"));
        let got = run(
            &state,
            Some(&person(uuid::Uuid::nil())),
            "stop_query",
            &args,
        )
        .await;
        assert_eq!(got, json!({ "error": "runId must be a UUID." }));
    }
}
