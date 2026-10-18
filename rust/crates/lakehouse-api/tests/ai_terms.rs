//! HTTP-level tests for `GET`/`PUT`/`DELETE /api/ai/terms`: the round trip,
//! that two users never see, overwrite or delete each other's terms, that a
//! broken rule is a 400 naming the field, and that the list is capped. The
//! `RequiresAuth` floor itself is asserted for all three methods by the
//! `POLICY_TABLE` loops in `tests/route_auth.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use common::{create_zero_permission_principal, session_cookie_for_user, spin_up};

async fn send(
    router: &axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    let request = builder
        .body(Body::from(body.map_or_else(String::new, |v| v.to_string())))
        .expect("build request");
    let response = router.clone().oneshot(request).await.expect("request");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

const TERMS: &str = "/api/ai/terms";

fn term_names(body: &Value) -> Vec<&str> {
    body["terms"]
        .as_array()
        .expect("terms is a list")
        .iter()
        .map(|t| t["term"].as_str().expect("term"))
        .collect()
}

#[tokio::test]
async fn a_new_user_has_no_terms_then_save_list_and_delete_round_trip() {
    let app = spin_up().await;
    let user = create_zero_permission_principal(&app.pool).await;
    let cookie = session_cookie_for_user(&app.pool, user).await;

    let (status, body) = send(&app.router, "GET", TERMS, Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "terms": [] }));

    let put = json!({ "term": " Net Revenue ", "meaning": "revenue after refunds", "question": "show net revenue" });
    let (status, body) = send(&app.router, "PUT", TERMS, Some(&cookie), Some(put)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["term"], "net revenue");
    assert_eq!(body["meaning"], "revenue after refunds");
    assert_eq!(body["question"], "show net revenue");
    assert!(body["updatedAt"].as_str().is_some_and(|s| s.ends_with('Z')));

    let (_, listed) = send(&app.router, "GET", TERMS, Some(&cookie), None).await;
    assert_eq!(term_names(&listed), ["net revenue"]);
    assert_eq!(listed["terms"][0], body);

    let put = json!({ "term": "net revenue", "meaning": "revenue before tax" });
    let (status, body) = send(&app.router, "PUT", TERMS, Some(&cookie), Some(put)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["meaning"], "revenue before tax");
    assert_eq!(body["question"], "", "question is optional");

    let uri = format!("{TERMS}?term=NET%20REVENUE");
    let (status, body) = send(&app.router, "DELETE", &uri, Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "deleted": true }));
    let (_, listed) = send(&app.router, "GET", TERMS, Some(&cookie), None).await;
    assert_eq!(listed, json!({ "terms": [] }));
}

#[tokio::test]
async fn deleting_a_term_that_does_not_exist_is_a_200() {
    let app = spin_up().await;
    let user = create_zero_permission_principal(&app.pool).await;
    let cookie = session_cookie_for_user(&app.pool, user).await;

    let uri = format!("{TERMS}?term=nothing");
    let (status, body) = send(&app.router, "DELETE", &uri, Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "deleted": false }));
}

#[tokio::test]
async fn one_user_never_reads_overwrites_or_deletes_another_users_term() {
    let app = spin_up().await;
    let alice = create_zero_permission_principal(&app.pool).await;
    let bob = create_zero_permission_principal(&app.pool).await;
    let alice_cookie = session_cookie_for_user(&app.pool, alice).await;
    let bob_cookie = session_cookie_for_user(&app.pool, bob).await;

    let put = json!({ "term": "revenue", "meaning": "net of refunds" });
    send(&app.router, "PUT", TERMS, Some(&alice_cookie), Some(put)).await;

    let (_, body) = send(&app.router, "GET", TERMS, Some(&bob_cookie), None).await;
    assert_eq!(
        body,
        json!({ "terms": [] }),
        "bob must not read alice's term"
    );

    let put = json!({ "term": "revenue", "meaning": "gross sales" });
    send(&app.router, "PUT", TERMS, Some(&bob_cookie), Some(put)).await;
    let (_, body) = send(&app.router, "GET", TERMS, Some(&alice_cookie), None).await;
    assert_eq!(body["terms"][0]["meaning"], "net of refunds");

    let uri = format!("{TERMS}?term=revenue");
    let (status, body) = send(&app.router, "DELETE", &uri, Some(&bob_cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "deleted": true }), "bob deletes his own row");
    let (_, body) = send(&app.router, "GET", TERMS, Some(&alice_cookie), None).await;
    assert_eq!(term_names(&body), ["revenue"]);
    assert_eq!(body["terms"][0]["meaning"], "net of refunds");

    let (_, body) = send(&app.router, "DELETE", &uri, Some(&bob_cookie), None).await;
    assert_eq!(
        body,
        json!({ "deleted": false }),
        "bob has no row left, and alice's is not his to delete"
    );
    let (_, body) = send(&app.router, "GET", TERMS, Some(&alice_cookie), None).await;
    assert_eq!(term_names(&body), ["revenue"]);
}

#[tokio::test]
async fn a_broken_rule_is_a_400_naming_the_field_and_saves_nothing() {
    let app = spin_up().await;
    let user = create_zero_permission_principal(&app.pool).await;
    let cookie = session_cookie_for_user(&app.pool, user).await;

    let cases = [
        ("term", json!({ "term": "", "meaning": "m" })),
        ("term", json!({ "term": "   ", "meaning": "m" })),
        ("term", json!({ "term": "t".repeat(61), "meaning": "m" })),
        ("meaning", json!({ "term": "t", "meaning": "" })),
        ("meaning", json!({ "term": "t", "meaning": "  " })),
        (
            "meaning",
            json!({ "term": "t", "meaning": "m".repeat(201) }),
        ),
        (
            "question",
            json!({ "term": "t", "meaning": "m", "question": "q".repeat(501) }),
        ),
    ];
    for (field, body) in cases {
        let (status, response) = send(&app.router, "PUT", TERMS, Some(&cookie), Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{field}");
        assert!(
            response["error"]
                .as_str()
                .is_some_and(|m| m.starts_with(field)),
            "{response} should name {field}"
        );
    }

    for malformed in [
        json!({ "term": "t" }),
        json!({ "meaning": "m" }),
        json!([1]),
    ] {
        let (status, _) = send(&app.router, "PUT", TERMS, Some(&cookie), Some(malformed)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let (status, _) = send(&app.router, "PUT", TERMS, Some(&cookie), None).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an empty body is not a term"
    );

    let (status, response) = send(&app.router, "DELETE", TERMS, Some(&cookie), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        response["error"]
            .as_str()
            .is_some_and(|m| m.starts_with("term")),
        "{response} should name term"
    );

    let (_, body) = send(&app.router, "GET", TERMS, Some(&cookie), None).await;
    assert_eq!(body, json!({ "terms": [] }));
}

#[tokio::test]
async fn the_101st_term_is_a_400_and_an_existing_term_still_updates() {
    let app = spin_up().await;
    let user = create_zero_permission_principal(&app.pool).await;
    let cookie = session_cookie_for_user(&app.pool, user).await;

    for i in 0..100 {
        let put = json!({ "term": format!("term-{i}"), "meaning": "m" });
        let (status, _) = send(&app.router, "PUT", TERMS, Some(&cookie), Some(put)).await;
        assert_eq!(status, StatusCode::OK, "term {i}");
    }

    let put = json!({ "term": "one too many", "meaning": "m" });
    let (status, response) = send(&app.router, "PUT", TERMS, Some(&cookie), Some(put)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        response["error"]
            .as_str()
            .is_some_and(|m| m.contains("100")),
        "{response} should name the limit"
    );

    let put = json!({ "term": "term-3", "meaning": "changed" });
    let (status, response) = send(&app.router, "PUT", TERMS, Some(&cookie), Some(put)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["meaning"], "changed");

    let (_, body) = send(&app.router, "GET", TERMS, Some(&cookie), None).await;
    assert_eq!(body["terms"].as_array().expect("terms").len(), 100);
}

#[tokio::test]
async fn without_a_session_every_method_is_401() {
    let app = spin_up().await;
    let put = json!({ "term": "t", "meaning": "m" });
    for (method, uri) in [
        ("GET", TERMS),
        ("PUT", TERMS),
        ("DELETE", "/api/ai/terms?term=t"),
    ] {
        let (status, _) = send(&app.router, method, uri, None, Some(put.clone())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method}");
    }
}
