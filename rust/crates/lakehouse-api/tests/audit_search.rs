//! The audit trail's search, single-event and CSV export routes (`GOV-1`,
//! feature page Decisions 5 and 9).
//!
//! The tests drive the real router over a real per-test database. Rows are
//! seeded with `INSERT` (the append-only triggers of migration `0142` forbid
//! `UPDATE` and `DELETE`), with fixed past timestamps so a test never depends
//! on the clock. The expected values are literals written here, not values
//! computed the way the code computes them.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::common::{
    TestApp, audit_rows, create_principal_with_permissions, session_cookie_for_user, spin_up,
};

/// One `audit_event` row to seed.
struct Seed<'a> {
    id: &'a str,
    at: &'a str,
    principal_id: Option<&'a str>,
    principal_kind: Option<&'a str>,
    actor_label: Option<&'a str>,
    action: &'a str,
    resource_kind: Option<&'a str>,
    resource_id: Option<&'a str>,
    outcome: &'a str,
    args: Value,
    detail: Option<&'a str>,
}

impl<'a> Seed<'a> {
    fn new(id: &'a str, at: &'a str, action: &'a str) -> Self {
        Self {
            id,
            at,
            principal_id: None,
            principal_kind: None,
            actor_label: None,
            action,
            resource_kind: None,
            resource_id: None,
            outcome: "executed",
            args: json!({}),
            detail: None,
        }
    }

    async fn insert(self, pool: &PgPool) {
        sqlx::query(
            "INSERT INTO audit_event (id, at, principal_id, principal_kind, actor_label, action, \
             resource_kind, resource_id, args, outcome, detail) \
             VALUES ($1, $2::timestamptz, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(self.id)
        .bind(self.at)
        .bind(self.principal_id)
        .bind(self.principal_kind)
        .bind(self.actor_label)
        .bind(self.action)
        .bind(self.resource_kind)
        .bind(self.resource_id)
        .bind(sqlx::types::Json(self.args))
        .bind(self.outcome)
        .bind(self.detail)
        .execute(pool)
        .await
        .expect("seed an audit_event row");
    }
}

/// `GET path` with `cookie`: the status, the headers and the whole body.
async fn get(app: &axum::Router, path: &str, cookie: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(path)
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, headers, bytes.to_vec())
}

/// [`get`] with the body as text.
async fn get_text(app: &axum::Router, path: &str, cookie: &str) -> (StatusCode, String) {
    let (status, _, body) = get(app, path, cookie).await;
    (status, String::from_utf8_lossy(&body).into_owned())
}

async fn get_json(app: &axum::Router, path: &str, cookie: &str) -> (StatusCode, Value) {
    let (status, _, body) = get(app, path, cookie).await;
    let value = serde_json::from_slice(&body)
        .unwrap_or_else(|_| panic!("a JSON body: {}", String::from_utf8_lossy(&body)));
    (status, value)
}

/// A user who holds `audit:read` and nothing else, and a session for them.
async fn auditor(pool: &PgPool) -> (Uuid, String) {
    let user = create_principal_with_permissions(pool, "audit:read").await;
    let cookie = session_cookie_for_user(pool, user).await;
    (user, cookie)
}

fn ids_of(page: &Value) -> Vec<String> {
    page["events"]
        .as_array()
        .expect("an events array")
        .iter()
        .map(|e| e["id"].as_str().expect("an event id").to_owned())
        .collect()
}

/// RFC 4180 text into rows of cells: quotes, doubled quotes, commas and line
/// breaks inside quotes. The byte-order mark is dropped.
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match (quoted, ch) {
            (true, '"') if chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            (true, '"') => quoted = false,
            (false, '"') => quoted = true,
            (false, ',') => row.push(std::mem::take(&mut cell)),
            (false, '\r') => {}
            (false, '\n') => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            (_, other) => cell.push(other),
        }
    }
    assert!(
        cell.is_empty() && row.is_empty() && !quoted,
        "the file ends at a line end"
    );
    rows
}

const COLUMNS: [&str; 9] = [
    "at",
    "actor",
    "principal_kind",
    "action",
    "resource_kind",
    "resource_id",
    "outcome",
    "detail",
    "args",
];

// ── The row shape the old list and the new routes share ───────────────────

/// The row both routes show for the same stored event. The old
/// `GET /api/governance/audit` row is read by the console, the assistant's
/// `get_audit_history` tool and `ops/g3a`, so its keys and values are pinned
/// here; the search and the single-event routes answer the same object.
#[tokio::test]
async fn the_old_list_and_the_new_routes_show_one_event_as_the_same_object() {
    let TestApp { router, pool } = spin_up().await;
    // The old list sits behind `/api/governance/{kind}` (`governance:read`) and
    // then asks for `audit:read` itself; the new routes need `audit:read` only.
    let user = create_principal_with_permissions(&pool, "governance:read,audit:read").await;
    let cookie = session_cookie_for_user(&pool, user).await;
    let mut grant = Seed::new(
        "audit-shape-grant",
        "2026-02-03T04:05:06.789Z",
        "catalog.grant",
    );
    grant.principal_id = Some("p-1");
    grant.principal_kind = Some("user");
    grant.actor_label = Some("Rina Wijaya");
    grant.resource_kind = Some("catalog");
    grant.resource_id = Some("t1");
    grant.args = json!({
        "permission": "query:read",
        "grantee": "A <a@example.test>",
        "expiresAt": "2026-12-06T08:00:00Z",
    });
    grant.insert(&pool).await;
    let mut plain = Seed::new(
        "audit-shape-plain",
        "2026-02-03T04:05:00.000Z",
        "dashboard.update",
    );
    plain.principal_id = Some("p-2");
    plain.principal_kind = Some("copilot");
    plain.resource_kind = Some("dashboard");
    plain.outcome = "needs_approval";
    plain.insert(&pool).await;

    let expected_grant = json!({
        "id": "audit-shape-grant",
        "at": "2026-02-03T04:05:06.789Z",
        "actor": "Rina Wijaya",
        "actorKind": "user",
        "tenant": "dispar-dki",
        "action": "catalog.grant",
        "resource": "catalog:t1",
        "outcome": "success",
        "policyDecision": "allow",
        "obligations": [],
        "approvalId": null,
        "source": "console",
        "detail": "Granted query:read to A <a@example.test> until 2026-12-06",
    });
    let expected_plain = json!({
        "id": "audit-shape-plain",
        "at": "2026-02-03T04:05:00.000Z",
        "actor": "p-2",
        "actorKind": "agent",
        "tenant": "dispar-dki",
        "action": "dashboard.update",
        "resource": "dashboard",
        "outcome": "pending",
        "policyDecision": "pending",
        "obligations": [],
        "approvalId": null,
        "source": "copilot",
    });

    let (status, old) = get_json(&router, "/api/governance/audit", &cookie).await;
    assert_eq!(status, StatusCode::OK, "{old}");
    let old_rows = old["audit"].as_array().expect("an audit array");
    let old_grant = old_rows.iter().find(|r| r["id"] == "audit-shape-grant");
    let old_plain = old_rows.iter().find(|r| r["id"] == "audit-shape-plain");
    assert_eq!(old_grant, Some(&expected_grant), "the old row of a grant");
    assert_eq!(
        old_plain,
        Some(&expected_plain),
        "the old row of a plain event"
    );

    let (status, page) = get_json(&router, "/api/governance/audit/events", &cookie).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["events"], json!([expected_grant, expected_plain]));

    let (status, one) = get_json(
        &router,
        "/api/governance/audit/events/audit-shape-grant",
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{one}");
    assert_eq!(one, json!({ "event": expected_grant }));
}

// ── Search ────────────────────────────────────────────────────────────────

async fn seed_four_kinds_of_event(pool: &PgPool) {
    let mut a = Seed::new("f-a", "2026-03-01T10:00:00Z", "dashboard.update");
    a.principal_id = Some("p-a");
    a.resource_kind = Some("dashboard");
    a.resource_id = Some("b-1");
    a.insert(pool).await;
    let mut b = Seed::new("f-b", "2026-03-02T00:00:00Z", "dashboard.delete");
    b.principal_id = Some("p-b");
    b.resource_kind = Some("dashboard");
    b.resource_id = Some("b-2");
    b.outcome = "refused";
    b.insert(pool).await;
    let mut c = Seed::new("f-c", "2026-03-03T10:00:00Z", "alert.create");
    c.principal_id = Some("p-a");
    c.resource_kind = Some("alert_rule");
    c.resource_id = Some("r-1");
    c.outcome = "failed";
    c.insert(pool).await;
    let mut d = Seed::new("f-d", "2026-03-04T10:00:00Z", "dashboardXupdate");
    d.principal_id = Some("p-b");
    d.insert(pool).await;
}

/// Every query parameter reaches the store as the filter it names, and the
/// action prefix and the two bounds mean what the page says: a value ending in
/// `.` is a prefix, `from` is inclusive, `to` is exclusive.
#[tokio::test]
async fn every_search_parameter_narrows_the_events_it_names() {
    let TestApp { router, pool } = spin_up().await;
    let (_, cookie) = auditor(&pool).await;
    seed_four_kinds_of_event(&pool).await;

    let cases: [(&str, &[&str]); 12] = [
        ("", &["f-d", "f-c", "f-b", "f-a"]),
        ("actor=p-a", &["f-c", "f-a"]),
        ("action=dashboard.update", &["f-a"]),
        ("action=dashboard.", &["f-b", "f-a"]),
        ("resourceKind=dashboard", &["f-b", "f-a"]),
        ("resourceKind=dashboard&resourceId=b-2", &["f-b"]),
        ("outcome=refused", &["f-b"]),
        ("from=2026-03-02T00:00:00Z", &["f-d", "f-c", "f-b"]),
        ("to=2026-03-02T00:00:00Z", &["f-a"]),
        ("actor=p-b&outcome=executed", &["f-d"]),
        // A NUL cannot be stored, so it matches nothing; it must not fail the query.
        ("resourceId=%00", &[]),
        (
            "from=2026-03-01T12:00:00Z&to=2026-03-04T00:00:00Z&actor=p-a",
            &["f-c"],
        ),
    ];
    for (query, expected) in cases {
        let (status, page) = get_json(
            &router,
            &format!("/api/governance/audit/events?{query}"),
            &cookie,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{query}: {page}");
        assert_eq!(ids_of(&page), expected, "the events of `{query}`");
        assert_eq!(page["nextCursor"], Value::Null, "one page: {query}");
    }
}

/// Several rows share one `at`; the pages cut through them. Every event shows
/// once, in `(at, id)` order, and the last page has no cursor.
#[tokio::test]
async fn paging_across_rows_with_the_same_time_shows_each_event_once() {
    let TestApp { router, pool } = spin_up().await;
    let (_, cookie) = auditor(&pool).await;
    Seed::new("page-newer", "2026-04-02T00:00:00Z", "a.b")
        .insert(&pool)
        .await;
    for id in [
        "page-tie-a",
        "page-tie-b",
        "page-tie-c",
        "page-tie-d",
        "page-tie-e",
        "page-tie-f",
        "page-tie-g",
    ] {
        Seed::new(id, "2026-04-01T00:00:00.123456Z", "a.b")
            .insert(&pool)
            .await;
    }
    Seed::new("page-older", "2026-03-31T00:00:00Z", "a.b")
        .insert(&pool)
        .await;

    let mut seen: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        pages += 1;
        assert!(pages <= 6, "paging does not end; seen so far {seen:?}");
        let mut path = "/api/governance/audit/events?limit=4".to_owned();
        if let Some(cursor) = &cursor {
            path = format!("{path}&cursor={cursor}");
        }
        let (status, page) = get_json(&router, &path, &cookie).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        seen.extend(ids_of(&page));
        match page["nextCursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => break,
        }
    }
    assert_eq!(pages, 3, "nine events in pages of four");
    assert_eq!(
        seen,
        [
            "page-newer",
            "page-tie-g",
            "page-tie-f",
            "page-tie-e",
            "page-tie-d",
            "page-tie-c",
            "page-tie-b",
            "page-tie-a",
            "page-older",
        ]
    );
}

/// The old list cut at 200 rows and the last 14 days. The search has no such
/// cut: 250 events a month old are all reachable, 100 at a time by default.
#[tokio::test]
async fn events_older_than_fourteen_days_and_beyond_two_hundred_rows_are_reachable() {
    let TestApp { router, pool } = spin_up().await;
    let (_, cookie) = auditor(&pool).await;
    sqlx::query(
        "INSERT INTO audit_event (id, at, action, outcome) \
         SELECT 'old-' || lpad(g::text, 4, '0'), \
                timestamptz '2020-01-01T00:00:00Z' + g * interval '1 second', \
                'old.event', 'executed' \
         FROM generate_series(1, 250) AS g",
    )
    .execute(&pool)
    .await
    .unwrap();

    let (status, first) = get_json(&router, "/api/governance/audit/events", &cookie).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(ids_of(&first).len(), 100, "the default page is 100 events");
    assert!(first["nextCursor"].is_string());

    let (status, all) = get_json(&router, "/api/governance/audit/events?limit=500", &cookie).await;
    assert_eq!(status, StatusCode::OK, "{all}");
    let ids = ids_of(&all);
    assert_eq!(ids.len(), 250);
    assert_eq!(ids.first().map(String::as_str), Some("old-0250"));
    assert_eq!(ids.last().map(String::as_str), Some("old-0001"));
    assert_eq!(all["nextCursor"], Value::Null);
}

/// A bad query is a 400 with one fixed sentence. The sentence never repeats
/// the value the caller sent and never carries a library's wording (a key sent
/// twice is serde's "duplicate field" otherwise).
#[tokio::test]
async fn a_bad_search_query_gets_a_fixed_sentence_that_does_not_echo_the_value() {
    let TestApp { router, pool } = spin_up().await;
    let (_, cookie) = auditor(&pool).await;
    let time = "from and to must be RFC 3339 times, for example 2026-11-01T00:00:00Z.";
    let outcome = "outcome must be one of allowed, executed, refused, needs_confirmation, \
                   needs_approval, approved, rejected, failed.";
    let limit = "limit must be a whole number from 1 to 500.";
    let cursor = "The cursor is not valid.";
    let range = "from must be before to.";
    let unreadable = "The query string could not be read.";
    let cases: [(&str, &str, &str); 14] = [
        ("from=sneaky-date", time, "sneaky-date"),
        ("to=2026-13-45T00:00:00Z", time, "2026-13-45"),
        ("outcome=sneaky_outcome", outcome, "sneaky_outcome"),
        ("limit=0", limit, "limit=0"),
        ("limit=501", limit, "501"),
        ("limit=-1", limit, "-1"),
        ("limit=sneaky-limit", limit, "sneaky-limit"),
        ("cursor=sneaky!cursor", cursor, "sneaky!cursor"),
        // GOV-1 review R-1: well-formed base64 that Postgres would refuse.
        // `-300000000000000000:x` is a time before 4713 BC, and the second
        // is `1772359200000100:a<NUL>b`, an id Postgres TEXT cannot hold.
        // Without the checks these reach the database and answer 500.
        ("cursor=LTMwMDAwMDAwMDAwMDAwMDAwMDp4", cursor, "LTMwMDAw"),
        ("cursor=MTc3MjM1OTIwMDAwMDEwMDphAGI", cursor, "MTc3MjM1"),
        (
            "from=2026-02-01T00:00:00Z&to=2026-01-01T00:00:00Z",
            range,
            "2026-02-01",
        ),
        (
            "from=2026-02-01T00:00:00Z&to=2026-02-01T00:00:00Z",
            range,
            "2026-02-01",
        ),
        (
            "outcome=sneaky_one&outcome=sneaky_two",
            unreadable,
            "sneaky_one",
        ),
        ("actor=sneaky_a&actor=sneaky_b", unreadable, "sneaky_a"),
    ];
    for (query, sentence, submitted) in cases {
        let (status, body) = get_text(
            &router,
            &format!("/api/governance/audit/events?{query}"),
            &cookie,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}: {body}");
        let parsed: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["error"], sentence, "{query}");
        assert!(
            !body.contains(submitted),
            "{query} echoed the value: {body}"
        );
        assert!(!body.contains("duplicate"), "{query} leaked serde: {body}");
    }
}

// ── One event ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn one_event_is_found_by_id_and_an_unknown_id_is_a_fixed_not_found() {
    let TestApp { router, pool } = spin_up().await;
    let (_, cookie) = auditor(&pool).await;
    let mut event = Seed::new("audit-one", "2026-05-01T00:00:00Z", "dashboard.update");
    event.actor_label = Some("Rina");
    event.insert(&pool).await;

    let (status, body) = get_json(&router, "/api/governance/audit/events/audit-one", &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["event"]["id"], "audit-one");
    assert_eq!(body["event"]["actor"], "Rina");

    // GOV-1 review R-2: `%00` decodes to a NUL byte, which Postgres TEXT
    // refuses; without the check in the handler that read answers 500.
    for missing in ["sneaky-missing-id", "sneaky%00id"] {
        let (status, body) = get_text(
            &router,
            &format!("/api/governance/audit/events/{missing}"),
            &cookie,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{missing}: {body}");
        let parsed: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["error"], "No audit event has that id.", "{missing}");
        assert!(
            !body.contains("sneaky"),
            "{missing}: the id is not echoed: {body}"
        );
    }
}

// ── Export ────────────────────────────────────────────────────────────────

/// The file's start and shape: a byte-order mark, the nine columns in order,
/// the headers a download carries, and one row per event, newest first.
#[tokio::test]
async fn the_export_is_a_csv_with_the_nine_columns_newest_first() {
    let TestApp { router, pool } = spin_up().await;
    let (_, cookie) = auditor(&pool).await;
    let mut old = Seed::new("x-1", "2026-06-01T08:00:00.500Z", "dashboard.update");
    old.principal_id = Some("p-1");
    old.principal_kind = Some("user");
    old.actor_label = Some("Rina Wijaya");
    old.resource_kind = Some("dashboard");
    old.resource_id = Some("b-1");
    old.detail = Some("200");
    old.args = json!({ "fields": ["name"] });
    old.insert(&pool).await;
    let mut new = Seed::new("x-2", "2026-06-02T08:00:00Z", "alert.create");
    new.principal_id = Some("p-2");
    new.principal_kind = Some("service");
    new.outcome = "failed";
    new.insert(&pool).await;

    let (status, _) = get_text(
        &router,
        "/api/governance/audit/export.csv?action=dashboard.update&action=ignored",
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a repeated key is refused");

    let (status, body) = get_text(
        &router,
        "/api/governance/audit/export.csv?resourceKind=nothing-here-matches",
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parse_csv(&body),
        vec![COLUMNS.map(str::to_owned).to_vec()],
        "no match is the header only"
    );

    let (status, headers, body) = get(
        &router,
        "/api/governance/audit/export.csv?to=2026-07-01T00:00:00Z&outcome=executed",
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "text/csv; charset=utf-8");
    assert_eq!(headers["cache-control"], "no-store");
    let disposition = headers["content-disposition"].to_str().unwrap();
    assert!(
        disposition.starts_with("attachment; filename=\"audit-events-")
            && disposition.ends_with(".csv\""),
        "{disposition}"
    );
    assert!(headers.get("x-audit-export-truncated").is_none());
    assert!(headers.get("x-audit-export-total").is_none());
    let text = String::from_utf8(body).unwrap();
    assert!(
        text.starts_with('\u{feff}'),
        "the file starts with a byte-order mark"
    );
    let rows = parse_csv(&text);
    assert_eq!(rows[0], COLUMNS);
    // The `audit.export` rows the earlier calls wrote are newer than July, so
    // the `to` bound leaves them out; `x-2` is `failed`.
    let seeded = &rows[1..];
    assert_eq!(seeded.len(), 1, "{rows:?}");
    assert_eq!(seeded[0][0], "2026-06-01T08:00:00.500Z");
    assert_eq!(seeded[0][1], "Rina Wijaya");
    assert_eq!(seeded[0][2], "user");
    assert_eq!(seeded[0][3], "dashboard.update");
    assert_eq!(seeded[0][4], "dashboard");
    assert_eq!(seeded[0][5], "b-1");
    assert_eq!(seeded[0][6], "executed");
    assert_eq!(seeded[0][7], "200");
    assert_eq!(
        serde_json::from_str::<Value>(&seeded[0][8]).unwrap(),
        json!({ "fields": ["name"] })
    );

    // Newest first, and the actor falls back to the principal id.
    let (_, _, body) = get(
        &router,
        "/api/governance/audit/export.csv?action=alert.create",
        &cookie,
    )
    .await;
    let rows = parse_csv(&String::from_utf8(body).unwrap());
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1][0], "2026-06-02T08:00:00.000Z");
    assert_eq!(rows[1][1], "p-2", "no label: the principal id");
    assert_eq!(rows[1][2], "service");

    let (_, _, body) = get(
        &router,
        "/api/governance/audit/export.csv?to=2026-07-01T00:00:00Z&from=2026-06-01T00:00:00Z",
        &cookie,
    )
    .await;
    let rows = parse_csv(&String::from_utf8(body).unwrap());
    let order: Vec<&str> = rows[1..].iter().map(|r| r[3].as_str()).collect();
    assert_eq!(order, ["alert.create", "dashboard.update"], "newest first");
}

/// A cell a spreadsheet would run as a formula leaves the file prefixed with a
/// single quote (`csv_export::neutralise`, the one rule of every download).
#[tokio::test]
async fn a_cell_that_starts_like_a_formula_is_neutralised_in_the_export() {
    let TestApp { router, pool } = spin_up().await;
    let (_, cookie) = auditor(&pool).await;
    for (id, label, detail) in [
        ("fx-1", "=HYPERLINK(\"http://x\")", "=1+1"),
        ("fx-2", "+cmd", "+1"),
        ("fx-3", "-cmd", "-1"),
        ("fx-4", "@SUM(A1)", "@x"),
    ] {
        let mut seed = Seed::new(id, "2026-06-10T00:00:00Z", "fx.event");
        seed.actor_label = Some(label);
        seed.detail = Some(detail);
        seed.insert(&pool).await;
    }
    let (status, _, body) = get(
        &router,
        "/api/governance/audit/export.csv?action=fx.event",
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = parse_csv(&String::from_utf8(body).unwrap());
    assert_eq!(rows.len(), 5);
    let cells: Vec<(&str, &str)> = rows[1..]
        .iter()
        .map(|r| (r[1].as_str(), r[7].as_str()))
        .collect();
    assert_eq!(
        cells,
        [
            ("'@SUM(A1)", "'@x"),
            ("'-cmd", "'-1"),
            ("'+cmd", "'+1"),
            ("'=HYPERLINK(\"http://x\")", "'=1+1"),
        ]
    );
}

#[tokio::test]
async fn a_field_with_a_comma_a_quote_or_a_line_break_is_quoted() {
    let TestApp { router, pool } = spin_up().await;
    let (_, cookie) = auditor(&pool).await;
    let mut seed = Seed::new("q-1", "2026-06-11T00:00:00Z", "q.event");
    seed.detail = Some("a,b \"c\"\nsecond line");
    seed.insert(&pool).await;
    let (_, _, body) = get(
        &router,
        "/api/governance/audit/export.csv?action=q.event",
        &cookie,
    )
    .await;
    let text = String::from_utf8(body).unwrap();
    assert!(
        text.contains("\"a,b \"\"c\"\"\nsecond line\""),
        "the detail is quoted with doubled quotes: {text:?}"
    );
    let rows = parse_csv(&text);
    assert_eq!(rows[1][7], "a,b \"c\"\nsecond line");
}

/// More than 100,000 events match: the file holds the newest 100,000, and two
/// headers say it was cut and how many matched. Under the cap neither header
/// is sent.
#[tokio::test]
async fn an_export_of_more_than_one_hundred_thousand_events_keeps_the_newest_and_says_so() {
    let TestApp { router, pool } = spin_up().await;
    let (_, cookie) = auditor(&pool).await;
    sqlx::query(
        "INSERT INTO audit_event (id, at, action, outcome, detail) \
         SELECT 'big-' || lpad(g::text, 6, '0'), \
                timestamptz '2026-01-01T00:00:00Z' - g * interval '1 second', \
                'big.event', 'executed', 'row-' || lpad(g::text, 6, '0') \
         FROM generate_series(1, 100001) AS g",
    )
    .execute(&pool)
    .await
    .unwrap();

    let (status, headers, body) = get(
        &router,
        "/api/governance/audit/export.csv?action=big.event",
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["x-audit-export-truncated"], "true");
    assert_eq!(headers["x-audit-export-total"], "100001");
    let rows = parse_csv(&String::from_utf8(body).unwrap());
    assert_eq!(rows.len(), 100_001, "the header and exactly 100,000 events");
    assert_eq!(rows[1][7], "row-000001", "the newest event is kept");
    assert_eq!(rows[100_000][7], "row-100000");
    assert!(
        rows.iter().all(|r| r[7] != "row-100001"),
        "the oldest event is the one left out"
    );

    // Under the cap: no headers.
    let (status, headers, body) = get(
        &router,
        "/api/governance/audit/export.csv?action=big.event&to=2025-12-31T00:00:00Z",
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers.get("x-audit-export-truncated").is_none());
    assert!(headers.get("x-audit-export-total").is_none());
    assert_eq!(
        parse_csv(&String::from_utf8(body).unwrap()).len(),
        1 + 100_001 - 86_400
    );
}

/// An export is a way out of the trail, so it records itself: one
/// `audit.export` row per call that names the filters used, never a cursor,
/// never a NUL, never more than 200 characters of a value. A refused request
/// writes none.
#[tokio::test]
async fn each_export_writes_one_audit_row_that_names_its_filters() {
    let TestApp { router, pool } = spin_up().await;
    let (user, cookie) = auditor(&pool).await;

    let (status, _, _) = get(
        &router,
        "/api/governance/audit/export.csv?actor=p-a&outcome=executed&from=2026-01-01T00:00:00Z&cursor=abc&limit=5",
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = audit_rows(&pool, "audit.export").await;
    assert_eq!(rows.len(), 1, "{rows:#?}");
    let row = &rows[0];
    assert_eq!(row["outcome"], "executed");
    assert_eq!(row["resource_kind"], "audit");
    assert_eq!(row["principal_id"], json!(user.to_string()));
    assert_eq!(row["principal_kind"], "user");
    assert_eq!(
        row["args"],
        json!({
            "actor": "p-a",
            "outcome": "executed",
            "from": "2026-01-01T00:00:00Z",
        }),
        "the filters used, and no cursor or limit"
    );

    let long = "x".repeat(300);
    let (status, _, _) = get(
        &router,
        &format!("/api/governance/audit/export.csv?resourceId=%00{long}"),
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = audit_rows(&pool, "audit.export").await;
    assert_eq!(rows.len(), 2, "one row per call");
    let recorded = rows[1]["args"]["resourceId"].as_str().unwrap();
    assert_eq!(recorded.chars().count(), 200);
    assert!(recorded.starts_with('\u{fffd}'), "a NUL became U+FFFD");

    let (status, _, _) = get(
        &router,
        "/api/governance/audit/export.csv?outcome=nonsense",
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        audit_rows(&pool, "audit.export").await.len(),
        2,
        "a refused request exports nothing and records nothing"
    );
}

// ── Who may read ──────────────────────────────────────────────────────────

/// Each of the three routes needs `audit:read`, both ways: no session is 401,
/// every role's `governance:read` alone is 403, and `audit:read` is let in.
#[tokio::test]
async fn the_three_audit_routes_need_audit_read_both_ways() {
    let TestApp { router, pool } = spin_up().await;
    Seed::new("auth-1", "2026-06-20T00:00:00Z", "auth.event")
        .insert(&pool)
        .await;
    let analyst = create_principal_with_permissions(&pool, "query:read,governance:read").await;
    let (_, auditor_cookie) = auditor(&pool).await;
    for path in [
        "/api/governance/audit/events",
        "/api/governance/audit/events/auth-1",
        "/api/governance/audit/export.csv",
    ] {
        let (status, _, _) = get(&router, path, "").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}: no session");

        let cookie = session_cookie_for_user(&pool, analyst).await;
        let (status, _, body) = get(&router, path, &cookie).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{path}: governance:read alone"
        );
        assert!(
            String::from_utf8_lossy(&body).contains("audit:read"),
            "{path}: the refusal names the permission"
        );

        let (status, _, _) = get(&router, path, &auditor_cookie).await;
        assert_eq!(status, StatusCode::OK, "{path}: audit:read");
    }
}
