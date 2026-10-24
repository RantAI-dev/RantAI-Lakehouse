//! The scheduled-report runner (`RPT-1` T3, plan decisions D3, D4, D6): a due
//! subscription is sent once and recorded, "skip when empty" records a skip,
//! a disabled or restricted owner stops the run with a reason, five failures in
//! a row pause it, the 20 MB rule leaves a file out and says so, the
//! subscription's filters reach the statements and the render token, and no
//! upstream text reaches a delivery's reason.
//!
//! Real per-test Postgres; `ClickHouse` and Gotenberg are `wiremock` servers;
//! the mail server is a fake `ReportMailer` that never opens a socket.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use common::spin_up_state;
use lakehouse_api::routes::report_runner::{
    EMAIL_NOT_SET_UP, PAUSED_AFTER_FAILURES, run_subscription, tick,
};
use lakehouse_api::state::AppState;
use lakehouse_notify::{Attachment, DeliverResult, MailFuture, ReportMailer};
use lakehouse_store::report_schedule::Schedule;
use lakehouse_store::report_subscriptions::{
    self as subs, NewSubscription, Recipient, Subscription,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

const SECRET: &str = "test-render-secret-not-real";
const CONSOLE: &str = "http://console.test:3000";
const BOARD: &str = "b_rpt";
const MB: usize = 1024 * 1024;

// ── ClickHouse ───────────────────────────────────────────────────────────

/// A recording `ClickHouse`: one board, a bar chart and a text chart, the
/// column list, the tile rows (none when `empty`) and the streamed file rows.
#[derive(Clone)]
struct FakeCh {
    log: Arc<Mutex<Vec<String>>>,
    empty: Arc<AtomicBool>,
    stream_rows: Arc<AtomicUsize>,
}

fn chart_row(id: &str, kind: &str, def: &Value) -> Value {
    let spec = json!({
        "id": id, "title": "Visits by place", "kind": kind, "mart": "mart_x",
        "sql": "SELECT stored", "x": "region", "y": "email"
    });
    let mut def = def.clone();
    def["title"] = json!("Visits by place");
    def["mart"] = json!("mart_x");
    def["kind"] = json!(kind);
    json!({
        "id": id, "board": BOARD, "created_by": "ui", "created_at": "2026-01-01 00:00:00",
        "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
    })
}

impl Respond for FakeCh {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        self.log.lock().unwrap().push(sql.clone());
        let rows = |data: Vec<Value>| {
            let n = data.len();
            ResponseTemplate::new(200).set_body_json(json!({ "meta": [], "data": data, "rows": n }))
        };
        if sql.contains("JSONCompactEachRowWithNamesAndTypes") {
            let mut body = String::from("[\"region\",\"email\"]\n[\"String\",\"String\"]\n");
            for i in 0..self.stream_rows.load(Ordering::SeqCst) {
                writeln!(body, "[\"north\",\"person-{i:06}@example.com\"]").unwrap();
            }
            return ResponseTemplate::new(200).set_body_string(body);
        }
        if sql.contains("FROM console.bi_chart") {
            return rows(vec![
                chart_row(
                    "c_bar",
                    "bar",
                    &json!({ "dimension": "region", "measures": ["email"] }),
                ),
                chart_row("c_text", "text", &json!({ "text": "hello" })),
            ]);
        }
        if sql.contains("FROM console.bi_board") {
            // A lookup of one board by id finds only the one board that exists.
            if sql.contains("AND id=") && !sql.contains(&format!("AND id='{BOARD}'")) {
                return rows(vec![]);
            }
            return rows(vec![json!({
                "id": BOARD, "name": "Sales / Q3 report", "description": "", "created_by": "t",
                "layout_json": "", "filters_json": "", "public_token": "", "embed_enabled": "0",
                "folder_id": "", "created_at": "2026-10-10 00:00:00", "owner_id": "",
                "embed_revoked_before": "0", "embed_revoked_jti_json": "[]",
                "embed_origins_json": "[]"
            })]);
        }
        if sql.contains("system.columns") {
            return rows(
                ["email", "region"]
                    .iter()
                    .map(|n| json!({"table": "mart_x", "name": n, "type": "String"}))
                    .collect(),
            );
        }
        if sql.contains("toDate(now(") {
            return rows(vec![json!({ "d": "2031-02-03" })]);
        }
        if sql.starts_with("SELECT stored")
            || sql.contains("FROM serving.mart_x")
            || sql.contains("replaceRegexpOne")
        {
            if self.empty.load(Ordering::SeqCst) {
                return rows(vec![]);
            }
            return rows(vec![json!({"email": "a@x", "region": "north"})]);
        }
        ResponseTemplate::new(200).set_body_string("Ok.\n")
    }
}

// ── The fake mail server ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Sent {
    bcc: Vec<String>,
    subject: String,
    html: String,
    files: Vec<(String, usize)>,
    first_bytes: Vec<Vec<u8>>,
    audit_rows: i64,
}

#[derive(Clone)]
struct FakeMailer {
    configured: bool,
    calls: Arc<Mutex<Vec<Sent>>>,
    /// Fail every mail that carries a file (a report), not a plain notice.
    fail_reports: Arc<AtomicBool>,
    pool: PgPool,
}

impl FakeMailer {
    fn sent(&self) -> Vec<Sent> {
        self.calls.lock().unwrap().clone()
    }

    fn reports(&self) -> Vec<Sent> {
        self.sent()
            .into_iter()
            .filter(|s| !s.files.is_empty())
            .collect()
    }

    fn notices(&self) -> Vec<Sent> {
        self.sent()
            .into_iter()
            .filter(|s| s.files.is_empty())
            .collect()
    }
}

impl ReportMailer for FakeMailer {
    fn is_configured(&self) -> bool {
        self.configured
    }

    fn send_with_attachments<'a>(
        &'a self,
        bcc: &'a [String],
        subject: &'a str,
        html: &'a str,
        attachments: &'a [Attachment],
    ) -> MailFuture<'a> {
        Box::pin(async move {
            // What the audit trail held at the moment the mail was handed over.
            let audit: i64 =
                sqlx::query_scalar("SELECT count(*) FROM audit_event WHERE action LIKE 'report.%'")
                    .fetch_one(&self.pool)
                    .await
                    .unwrap();
            self.calls.lock().unwrap().push(Sent {
                bcc: bcc.to_vec(),
                subject: subject.to_owned(),
                html: html.to_owned(),
                files: attachments
                    .iter()
                    .map(|a| (a.file_name.clone(), a.bytes.len()))
                    .collect(),
                first_bytes: attachments
                    .iter()
                    .map(|a| a.bytes.iter().copied().take(8).collect())
                    .collect(),
                audit_rows: audit,
            });
            if self.fail_reports.load(Ordering::SeqCst) && !attachments.is_empty() {
                return DeliverResult {
                    ok: false,
                    error: Some("the mail server could not be reached".to_owned()),
                };
            }
            DeliverResult {
                ok: true,
                error: None,
            }
        })
    }
}

// ── The fixture ──────────────────────────────────────────────────────────

struct F {
    state: AppState,
    pool: PgPool,
    mailer: FakeMailer,
    ch: FakeCh,
    gotenberg: MockServer,
    owner: Uuid,
    owner_role: String,
    colleague: Uuid,
    /// Read by the R1 test for the URL parameters of the tile queries.
    ch_server: MockServer,
}

async fn person(pool: &PgPool, name: &str, role_id: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO app_user (id, name, email, status) VALUES ($1, $2, $3, 'active')")
        .bind(id)
        .bind(name)
        .bind(format!("{}@company.example", name.to_lowercase()))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO app_user_role (user_id, role_id) VALUES ($1, $2)")
        .bind(id)
        .bind(role_id)
        .execute(pool)
        .await
        .unwrap();
    id
}

async fn fixture(pdf: ResponseTemplate, configured: bool) -> F {
    let ch = FakeCh {
        log: Arc::new(Mutex::new(Vec::new())),
        empty: Arc::new(AtomicBool::new(false)),
        stream_rows: Arc::new(AtomicUsize::new(2)),
    };
    let ch_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ch.clone())
        .mount(&ch_server)
        .await;
    let gotenberg = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/forms/chromium/convert/url"))
        .respond_with(pdf)
        .mount(&gotenberg)
        .await;
    let env: HashMap<String, String> = HashMap::from([
        ("CH_URL".to_owned(), ch_server.uri()),
        ("GOTENBERG_URL".to_owned(), gotenberg.uri()),
        ("CONSOLE_INTERNAL_URL".to_owned(), CONSOLE.to_owned()),
        ("RENDER_TOKEN_SECRET".to_owned(), SECRET.to_owned()),
        (
            "CONSOLE_PUBLIC_URL".to_owned(),
            "https://lakehouse.company.example/".to_owned(),
        ),
    ]);
    let (state, pool) = spin_up_state(&env).await;
    let role_id = Uuid::new_v4();
    let owner_role = "Report Owner".to_owned();
    sqlx::query("INSERT INTO role (id, name, permissions, description) VALUES ($1, $2, 'dashboard:read', 'test')")
        .bind(role_id)
        .bind(&owner_role)
        .execute(&pool)
        .await
        .unwrap();
    let owner = person(&pool, "Owner", role_id).await;
    let colleague = person(&pool, "Colleague", role_id).await;
    // One outside domain is allowed.
    sqlx::query(
        "INSERT INTO reporting_settings (singleton, time_zone, week_start, report_allowed_domains) \
         VALUES (TRUE, 'UTC', 'monday', ARRAY['partner.example'])",
    )
    .execute(&pool)
    .await
    .unwrap();
    let mailer = FakeMailer {
        configured,
        calls: Arc::new(Mutex::new(Vec::new())),
        fail_reports: Arc::new(AtomicBool::new(false)),
        pool: pool.clone(),
    };
    F {
        state,
        pool,
        mailer,
        ch,
        gotenberg,
        owner,
        owner_role,
        colleague,
        ch_server,
    }
}

fn ok_pdf() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_bytes(b"%PDF-1.7 fake".to_vec())
}

async fn subscription(f: &F, tweak: impl FnOnce(&mut NewSubscription)) -> Subscription {
    let mut new = NewSubscription {
        board: BOARD.to_owned(),
        owner_id: f.owner,
        schedule: Schedule::Daily { hour: 8, minute: 0 },
        time_zone: "Asia/Jakarta".to_owned(),
        attachments: json!({
            "pdf": { "paper": "a4", "orientation": "portrait" },
            "charts": [{ "chart": "c_bar", "format": "csv", "values": "raw" }],
        }),
        filters: None,
        params: None,
        grain: None,
        skip_when_empty: false,
        next_run_at: OffsetDateTime::now_utc() - Duration::minutes(1),
        recipients: vec![
            Recipient::User(f.colleague),
            Recipient::Email("Press@Partner.example".to_owned()),
        ],
    };
    tweak(&mut new);
    subs::create(&f.pool, &new).await.unwrap()
}

async fn deliveries(f: &F, sub: &Subscription) -> Vec<subs::Delivery> {
    subs::list_deliveries(&f.pool, sub.id, 20).await.unwrap()
}

async fn make_due(f: &F, sub: &Subscription) {
    sqlx::query(
        "UPDATE report_subscription SET next_run_at = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(sub.id)
    .execute(&f.pool)
    .await
    .unwrap();
}

// ── Tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_due_subscription_is_sent_once_with_its_files_and_recorded_as_sent() {
    let f = fixture(ok_pdf(), true).await;
    let sub = subscription(&f, |_| {}).await;

    assert_eq!(tick(&f.state, &f.mailer).await, 1);
    let reports = f.mailer.reports();
    assert_eq!(reports.len(), 1, "one email");
    let mail = &reports[0];
    assert_eq!(
        mail.bcc,
        ["colleague@company.example", "press@partner.example"],
        "the user's address and the listed outside address, lower-cased"
    );
    let date = today(&mail.subject);
    assert!(
        mail.subject.starts_with("Sales / Q3 report report, ")
            && date.len() == 10
            && date.starts_with("20"),
        "{}",
        mail.subject
    );
    let names: Vec<&str> = mail.files.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["Sales-Q3-report.pdf", "Visits-by-place.csv"]);
    assert!(mail.first_bytes[0].starts_with(b"%PDF"));
    assert!(mail.html.contains("Sales / Q3 report"));
    assert!(
        mail.html
            .contains("is allowed to see. Everyone it is sent to sees the same data."),
        "the decision-1 sentence is in the body"
    );
    assert!(
        mail.html
            .contains("href=\"https://lakehouse.company.example/dashboards/b_rpt\""),
        "{}",
        mail.html
    );
    // BI-1 D8: both files were in the audit trail before the mail was handed over.
    assert_eq!(mail.audit_rows, 2);

    let rows = deliveries(&f, &sub).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, "sent");
    assert_eq!(rows[0].origin, "schedule");
    assert_eq!(rows[0].recipient_count, 2);
    assert_eq!(rows[0].reason, None);

    // It is not taken again, and the next run is in the future.
    assert_eq!(tick(&f.state, &f.mailer).await, 0);
    assert_eq!(f.mailer.reports().len(), 1);
    let stored = subs::get(&f.pool, sub.id).await.unwrap().unwrap();
    assert!(stored.next_run_at > OffsetDateTime::now_utc());
    assert_eq!(stored.consecutive_failures, 0);
}

/// `BI-5` re-review R1: the runner calls the tiles from its own spawned loop,
/// outside any request. The tiles must run as the owner (the query id starts
/// with the owner's id, so Stop and the process list can name them) under the
/// limits an administrator saved, not the nil principal and the compiled-in
/// defaults.
#[tokio::test]
async fn a_scheduled_delivery_asks_the_engine_as_the_owner_under_the_saved_limits() {
    let f = fixture(ok_pdf(), true).await;
    lakehouse_store::query_limits::save(
        &f.pool,
        &lakehouse_store::query_limits::QueryLimits {
            timeout_seconds: 17,
            ..Default::default()
        },
        &[],
        "test",
    )
    .await
    .unwrap();
    // `skip_when_empty` reads the dashboard's tiles (`payload_for`).
    let _sub = subscription(&f, |s| s.skip_when_empty = true).await;

    assert_eq!(tick(&f.state, &f.mailer).await, 1);
    let own_prefix = format!("{}:", f.owner);
    let tile_queries: Vec<_> = f
        .ch_server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter_map(|r| {
            let param = |name: &str| {
                r.url
                    .query_pairs()
                    .find(|(k, _)| k == name)
                    .map(|(_, v)| v.into_owned())
            };
            param("query_id").map(|id| (id, param("max_execution_time")))
        })
        .collect();
    assert!(
        !tile_queries.is_empty(),
        "the delivery sent tile queries with a query id"
    );
    for (id, seconds) in &tile_queries {
        assert!(id.starts_with(&own_prefix), "{id} is not the owner's run");
        assert_eq!(seconds.as_deref(), Some("17"), "{id}");
    }
}

/// The subject carries today's date in the subscription's zone; the test reads
/// it back from the subject instead of computing a second clock.
fn today(subject: &str) -> String {
    subject.rsplit(", ").next().unwrap().to_owned()
}

#[tokio::test]
async fn skip_when_empty_sends_nothing_and_records_the_skip() {
    let f = fixture(ok_pdf(), true).await;
    f.ch.empty.store(true, Ordering::SeqCst);
    let sub = subscription(&f, |s| s.skip_when_empty = true).await;

    assert_eq!(tick(&f.state, &f.mailer).await, 1);
    assert!(f.mailer.sent().is_empty(), "no email, not even a notice");
    assert!(
        f.gotenberg.received_requests().await.unwrap().is_empty(),
        "no PDF was made for a report that is not sent"
    );
    let rows = deliveries(&f, &sub).await;
    assert_eq!(rows[0].status, "skipped");
    assert_eq!(
        rows[0].reason.as_deref(),
        Some("Every chart on the dashboard had no rows.")
    );
    assert_eq!(
        subs::get(&f.pool, sub.id)
            .await
            .unwrap()
            .unwrap()
            .consecutive_failures,
        0,
        "a skip is not a failure"
    );

    // The same subscription with rows is sent.
    f.ch.empty.store(false, Ordering::SeqCst);
    make_due(&f, &sub).await;
    assert_eq!(tick(&f.state, &f.mailer).await, 1);
    assert_eq!(
        f.mailer.reports().len(),
        1,
        "{:?}",
        deliveries(&f, &sub).await
    );
}

#[tokio::test]
async fn without_skip_when_empty_an_empty_dashboard_is_still_sent() {
    let f = fixture(ok_pdf(), true).await;
    f.ch.empty.store(true, Ordering::SeqCst);
    let _sub = subscription(&f, |_| {}).await;
    assert_eq!(tick(&f.state, &f.mailer).await, 1);
    assert_eq!(f.mailer.reports().len(), 1);
}

#[tokio::test]
async fn a_disabled_owner_stops_the_run_with_a_reason_and_nothing_is_sent() {
    let f = fixture(ok_pdf(), true).await;
    let sub = subscription(&f, |_| {}).await;
    sqlx::query("UPDATE app_user SET status = 'inactive' WHERE id = $1")
        .bind(f.owner)
        .execute(&f.pool)
        .await
        .unwrap();

    assert_eq!(tick(&f.state, &f.mailer).await, 1);
    assert!(
        f.mailer.sent().is_empty(),
        "nothing to anyone, owner included"
    );
    assert!(f.gotenberg.received_requests().await.unwrap().is_empty());
    let rows = deliveries(&f, &sub).await;
    assert_eq!(rows[0].status, "failed");
    assert_eq!(
        rows[0].reason.as_deref(),
        Some("The person who set this report up is disabled or no longer exists.")
    );
    assert_eq!(
        subs::get(&f.pool, sub.id)
            .await
            .unwrap()
            .unwrap()
            .consecutive_failures,
        1
    );
}

#[tokio::test]
async fn an_owner_who_may_no_longer_download_stops_the_run() {
    let f = fixture(ok_pdf(), true).await;
    let sub = subscription(&f, |_| {}).await;
    sqlx::query("UPDATE reporting_settings SET download_denied_roles = ARRAY[$1]")
        .bind(&f.owner_role)
        .execute(&f.pool)
        .await
        .unwrap();

    tick(&f.state, &f.mailer).await;
    assert!(f.mailer.reports().is_empty());
    let rows = deliveries(&f, &sub).await;
    assert_eq!(rows[0].status, "failed");
    assert_eq!(
        rows[0].reason.as_deref(),
        Some("The person who set this report up can no longer view or download dashboards.")
    );
    // The owner was told, by email, in a sentence.
    let notices = f.mailer.notices();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].bcc, ["owner@company.example"]);
    assert_eq!(notices[0].subject, "A scheduled report could not be sent");
    assert!(
        notices[0]
            .html
            .contains("can no longer view or download dashboards")
    );
}

#[tokio::test]
async fn five_failures_in_a_row_pause_the_subscription_and_tell_the_owner_each_time() {
    let f = fixture(ok_pdf(), true).await;
    f.mailer.fail_reports.store(true, Ordering::SeqCst);
    let sub = subscription(&f, |_| {}).await;

    for n in 1..=5 {
        assert_eq!(tick(&f.state, &f.mailer).await, 1, "run {n}");
        make_due(&f, &sub).await;
    }
    let stored = subs::get(&f.pool, sub.id).await.unwrap().unwrap();
    assert!(stored.paused);
    assert_eq!(stored.paused_reason.as_deref(), Some(PAUSED_AFTER_FAILURES));
    assert_eq!(stored.consecutive_failures, 5);
    let rows = deliveries(&f, &sub).await;
    assert_eq!(rows.len(), 5);
    for row in &rows {
        assert_eq!(row.status, "failed");
        assert_eq!(
            row.reason.as_deref(),
            Some("The email was not sent: the mail server could not be reached."),
            "the closed set of reasons, nothing else"
        );
    }
    let notices = f.mailer.notices();
    assert_eq!(notices.len(), 5, "the owner is told after each failure");
    assert!(
        !notices[3].html.contains("Paused after 5"),
        "only the fifth says it paused"
    );
    assert!(
        notices[4]
            .html
            .contains("Paused after 5 failed sends in a row")
    );
    // A paused subscription is not taken again.
    assert_eq!(tick(&f.state, &f.mailer).await, 0);
}

#[tokio::test]
async fn a_success_resets_the_failure_count() {
    let f = fixture(ok_pdf(), true).await;
    let sub = subscription(&f, |_| {}).await;
    f.mailer.fail_reports.store(true, Ordering::SeqCst);
    for _ in 0..3 {
        tick(&f.state, &f.mailer).await;
        make_due(&f, &sub).await;
    }
    assert_eq!(
        subs::get(&f.pool, sub.id)
            .await
            .unwrap()
            .unwrap()
            .consecutive_failures,
        3
    );
    f.mailer.fail_reports.store(false, Ordering::SeqCst);
    tick(&f.state, &f.mailer).await;
    let stored = subs::get(&f.pool, sub.id).await.unwrap().unwrap();
    assert_eq!(stored.consecutive_failures, 0);
    assert!(!stored.paused);
}

#[tokio::test]
async fn a_file_that_would_pass_twenty_megabytes_is_left_out_and_the_email_says_so() {
    // A PDF of 20 MB minus 100 bytes leaves room for nothing more.
    let mut big = b"%PDF-1.7 ".to_vec();
    big.resize(20 * MB - 100, b' ');
    let f = fixture(ResponseTemplate::new(200).set_body_bytes(big), true).await;
    f.ch.stream_rows.store(500, Ordering::SeqCst);
    let sub = subscription(&f, |_| {}).await;

    tick(&f.state, &f.mailer).await;
    let reports = f.mailer.reports();
    assert_eq!(reports.len(), 1, "{:?}", deliveries(&f, &sub).await);
    let names: Vec<&str> = reports[0].files.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["Sales-Q3-report.pdf"], "the chart file did not fit");
    assert!(
        reports[0].html.contains(
            "&quot;Visits by place&quot; was left out: with it the email would pass 20 MB."
        ),
        "{}",
        reports[0].html
    );
    let rows = deliveries(&f, &sub).await;
    assert_eq!(rows[0].status, "sent");
    assert!(
        rows[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("\"Visits by place\" was left out: with it the email would pass 20 MB.")
    );
    // Only the file that went out was audited.
    assert_eq!(reports[0].audit_rows, 1);
}

#[tokio::test]
async fn the_subscriptions_filters_reach_the_statement_and_the_render_token() {
    let f = fixture(ok_pdf(), true).await;
    let filters = json!([{ "column": "region", "values": ["Bali"] }]);
    let sub = subscription(&f, |s| s.filters = Some(filters.clone())).await;

    tick(&f.state, &f.mailer).await;
    let log = f.ch.log.lock().unwrap().clone();
    let export = log
        .iter()
        .find(|s| s.contains("JSONCompactEachRowWithNamesAndTypes"))
        .expect("the chart file was read");
    assert!(
        export.contains("region") && export.contains("'Bali'"),
        "the filter is in the statement: {export}"
    );
    // And the PDF's token carries the same filters, as the owner, for this board.
    let sent = f.gotenberg.received_requests().await.unwrap();
    assert_eq!(sent.len(), 1);
    let form = String::from_utf8_lossy(&sent[0].body).into_owned();
    let url = form
        .split("name=\"url\"\r\n\r\n")
        .nth(1)
        .and_then(|rest| rest.split("\r\n").next())
        .unwrap();
    let token = url
        .strip_prefix(&format!("{CONSOLE}/render/dashboard/"))
        .unwrap();
    let claims =
        lakehouse_embed::verify_render(token, SECRET, lakehouse_embed::unix_now()).unwrap();
    assert_eq!(claims.board, BOARD);
    assert_eq!(claims.principal, f.owner.to_string());
    assert_eq!(
        serde_json::from_str::<Value>(claims.filters.as_deref().unwrap()).unwrap(),
        filters
    );
    // The email's body names the filter.
    assert!(f.mailer.reports()[0].html.contains("region: Bali"));
    drop(sub);
}

#[tokio::test]
async fn gotenbergs_own_text_never_reaches_the_delivery_reason() {
    let f = fixture(
        ResponseTemplate::new(500).set_body_string("chromium crashed at /srv/secret/internal-path"),
        true,
    )
    .await;
    let sub = subscription(&f, |_| {}).await;
    tick(&f.state, &f.mailer).await;
    let rows = deliveries(&f, &sub).await;
    assert_eq!(rows[0].status, "failed");
    assert_eq!(
        rows[0].reason.as_deref(),
        Some("The PDF could not be made.")
    );
    assert!(f.mailer.reports().is_empty());
}

#[tokio::test]
async fn without_email_set_up_the_run_says_so_and_does_nothing_else() {
    let f = fixture(ok_pdf(), false).await;
    let sub = subscription(&f, |_| {}).await;
    tick(&f.state, &f.mailer).await;
    let rows = deliveries(&f, &sub).await;
    assert_eq!(rows[0].status, "failed");
    assert_eq!(rows[0].reason.as_deref(), Some(EMAIL_NOT_SET_UP));
    assert!(f.mailer.sent().is_empty());
    assert!(
        f.gotenberg.received_requests().await.unwrap().is_empty(),
        "no PDF is made when it cannot be sent"
    );
}

#[tokio::test]
async fn a_domain_removed_from_the_list_and_a_disabled_colleague_are_left_out_and_said_so() {
    let f = fixture(ok_pdf(), true).await;
    let sub = subscription(&f, |_| {}).await;
    sqlx::query("UPDATE reporting_settings SET report_allowed_domains = '{}'")
        .execute(&f.pool)
        .await
        .unwrap();

    tick(&f.state, &f.mailer).await;
    let reports = f.mailer.reports();
    assert_eq!(reports[0].bcc, ["colleague@company.example"]);
    let rows = deliveries(&f, &sub).await;
    assert_eq!(rows[0].status, "sent");
    assert_eq!(rows[0].recipient_count, 1);
    assert!(
        rows[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("1 recipient was left out")
    );

    // With the colleague disabled too, nobody is left and nothing is sent.
    sqlx::query("UPDATE app_user SET status = 'inactive' WHERE id = $1")
        .bind(f.colleague)
        .execute(&f.pool)
        .await
        .unwrap();
    make_due(&f, &sub).await;
    tick(&f.state, &f.mailer).await;
    assert_eq!(f.mailer.reports().len(), 1, "no second report");
    let rows = deliveries(&f, &sub).await;
    assert_eq!(rows[0].status, "failed");
    assert_eq!(
        rows[0].reason.as_deref(),
        Some("Nobody is left to send this report to.")
    );
}

#[tokio::test]
async fn send_now_is_recorded_as_manual_and_never_counts_toward_the_five() {
    let f = fixture(ok_pdf(), true).await;
    let sub = subscription(&f, |s| {
        s.next_run_at = OffsetDateTime::now_utc() + Duration::days(3);
    })
    .await;
    let delivery = run_subscription(&f.state, &f.mailer, &sub, true)
        .await
        .unwrap();
    assert_eq!(delivery.status, "sent");
    assert_eq!(delivery.origin, "manual");
    f.mailer.fail_reports.store(true, Ordering::SeqCst);
    for _ in 0..6 {
        let failed = run_subscription(&f.state, &f.mailer, &sub, true)
            .await
            .unwrap();
        assert_eq!(failed.status, "failed");
    }
    let stored = subs::get(&f.pool, sub.id).await.unwrap().unwrap();
    assert_eq!(stored.consecutive_failures, 0);
    assert!(!stored.paused);
    assert!(
        stored.next_run_at > OffsetDateTime::now_utc() + Duration::days(2),
        "send now does not move the schedule"
    );
}

#[tokio::test]
async fn a_board_that_is_gone_fails_with_a_plain_sentence() {
    let f = fixture(ok_pdf(), true).await;
    let sub = subscription(&f, |s| s.board = "b_deleted".to_owned()).await;
    let delivery = run_subscription(&f.state, &f.mailer, &sub, true)
        .await
        .unwrap();
    assert_eq!(delivery.status, "failed");
    assert_eq!(
        delivery.reason.as_deref(),
        Some("The dashboard no longer exists, or its owner can no longer see it.")
    );
}

/// RPT-1 review fix (SHOULD-FIX) R3: ten charts left out make a note longer than
/// the record holds; the email still goes and the record still lands, cut.
#[tokio::test]
async fn ten_left_out_charts_do_not_break_the_delivery_record() {
    let f = fixture(ok_pdf(), true).await;
    let charts: Vec<Value> = (0..10)
        .map(|i| json!({ "chart": format!("c_gone_{i}"), "format": "csv", "values": "raw" }))
        .collect();
    let sub = subscription(&f, |s| {
        s.attachments =
            json!({ "pdf": { "paper": "a4", "orientation": "portrait" }, "charts": charts });
    })
    .await;
    tick(&f.state, &f.mailer).await;
    let reports = f.mailer.reports();
    assert_eq!(reports.len(), 1, "{:?}", deliveries(&f, &sub).await);
    assert_eq!(
        reports[0].html.matches("was left out").count(),
        10,
        "the email lists every one"
    );
    let rows = deliveries(&f, &sub).await;
    assert_eq!(rows.len(), 1, "the record landed");
    assert_eq!(rows[0].status, "sent");
    let reason = rows[0].reason.as_deref().unwrap();
    assert_eq!(reason.chars().count(), 500);
    assert!(reason.ends_with('…'));
}

/// RPT-1 review fix (SHOULD-FIX) R6: each attached file's audit entry says how
/// many people it went to, which outside addresses, and whether the schedule or
/// "Send now" sent it.
#[tokio::test]
async fn the_audit_entry_of_each_file_says_who_received_it() {
    let f = fixture(ok_pdf(), true).await;
    let sub = subscription(&f, |_| {}).await;
    tick(&f.state, &f.mailer).await;
    run_subscription(&f.state, &f.mailer, &sub, true)
        .await
        .unwrap();
    let rows: Vec<(String, Value)> = sqlx::query_as(
        "SELECT action, args FROM audit_event WHERE action LIKE 'report.%' ORDER BY action",
    )
    .fetch_all(&f.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 4, "two files, sent twice");
    for (action, args) in &rows {
        assert_eq!(args["recipientCount"], json!(2), "{action}");
        assert_eq!(
            args["outsideAddresses"],
            json!(["press@partner.example"]),
            "{action}"
        );
    }
    let origins: Vec<&str> = rows
        .iter()
        .map(|(_, a)| a["origin"].as_str().unwrap())
        .collect();
    assert_eq!(origins.iter().filter(|o| **o == "schedule").count(), 2);
    assert_eq!(origins.iter().filter(|o| **o == "manual").count(), 2);
}

/// A tracing writer that keeps what is logged, to read back.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Self;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// RPT-1 review fix (SHOULD-FIX) R7: a PDF failure leaves a trace for the
/// operator (the cause under a reference), while the stored reason stays fixed.
#[tokio::test]
async fn a_pdf_failure_is_logged_for_the_operator_and_the_record_stays_fixed() {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let f = fixture(
        ResponseTemplate::new(500).set_body_string("chromium crashed at /srv/internal-path"),
        true,
    )
    .await;
    let sub = subscription(&f, |_| {}).await;
    tick(&f.state, &f.mailer).await;
    let rows = deliveries(&f, &sub).await;
    assert_eq!(
        rows[0].reason.as_deref(),
        Some("The PDF could not be made.")
    );
    let log = String::from_utf8_lossy(&captured.0.lock().unwrap()).into_owned();
    assert!(log.contains("pdf render failed"), "{log}");
    assert!(log.contains("reference"), "{log}");
}

/// RPT-1 review fix (SHOULD-FIX) R8: a console recipient who can no longer view
/// dashboards, whose roles may not download, or who shares no tenant with the
/// owner is skipped, and the record says how many.
#[tokio::test]
async fn a_recipient_without_the_right_to_see_or_download_or_a_shared_tenant_is_skipped() {
    let f = fixture(ok_pdf(), true).await;
    // Three more people: no dashboard:read, a role switched off from
    // downloading, and one in a tenant the owner does not share.
    let role = |name: &'static str, perms: &'static str| {
        let pool = f.pool.clone();
        async move {
            let id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO role (id, name, permissions, description) VALUES ($1, $2, $3, 'test')",
            )
            .bind(id)
            .bind(name)
            .bind(perms)
            .execute(&pool)
            .await
            .unwrap();
            id
        }
    };
    let blind = person(&f.pool, "Blind", role("No Dashboards", "query:read").await).await;
    let denied_role = role("No Downloads", "dashboard:read").await;
    let denied = person(&f.pool, "Denied", denied_role).await;
    sqlx::query("UPDATE reporting_settings SET download_denied_roles = ARRAY['No Downloads']")
        .execute(&f.pool)
        .await
        .unwrap();
    let sub = subscription(&f, |s| {
        s.recipients = vec![
            Recipient::User(f.colleague),
            Recipient::User(blind),
            Recipient::User(denied),
        ];
    })
    .await;
    tick(&f.state, &f.mailer).await;
    let reports = f.mailer.reports();
    assert_eq!(
        reports[0].bcc,
        ["colleague@company.example"],
        "{:?}",
        reports[0].bcc
    );
    let rows = deliveries(&f, &sub).await;
    assert_eq!(rows[0].status, "sent");
    assert_eq!(rows[0].recipient_count, 1);
    assert!(
        rows[0]
            .reason
            .as_deref()
            .unwrap()
            .starts_with("2 recipients were left out")
    );

    // Tenants: the owner belongs to one, the colleague does not.
    let tenant = Uuid::new_v4();
    sqlx::query("INSERT INTO tenant (id, name, slug, plan, residency) VALUES ($1, 'T', 't-r8', 'standard', 'id')")
        .bind(tenant)
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(f.owner)
        .bind(tenant)
        .execute(&f.pool)
        .await
        .unwrap();
    make_due(&f, &sub).await;
    tick(&f.state, &f.mailer).await;
    let rows = deliveries(&f, &sub).await;
    assert_eq!(
        rows[0].status, "failed",
        "nobody shares the owner's tenant now"
    );
    assert_eq!(
        rows[0].reason.as_deref(),
        Some("Nobody is left to send this report to.")
    );
    // The colleague joins the tenant and receives it again.
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(f.colleague)
        .bind(tenant)
        .execute(&f.pool)
        .await
        .unwrap();
    make_due(&f, &sub).await;
    tick(&f.state, &f.mailer).await;
    assert_eq!(f.mailer.reports().len(), 2);
    assert_eq!(f.mailer.reports()[1].bcc, ["colleague@company.example"]);
}

/// RPT-1 review fix (SHOULD-FIX) R9: a failed "Send now" is not emailed to the
/// owner; "Email is not set up" never counts toward the five failures.
#[tokio::test]
async fn send_now_failures_do_not_email_the_owner_and_no_email_setup_is_not_counted() {
    let f = fixture(ok_pdf(), true).await;
    f.mailer.fail_reports.store(true, Ordering::SeqCst);
    let sub = subscription(&f, |s| {
        s.next_run_at = OffsetDateTime::now_utc() + Duration::days(3);
    })
    .await;
    let failed = run_subscription(&f.state, &f.mailer, &sub, true)
        .await
        .unwrap();
    assert_eq!(failed.status, "failed");
    assert!(
        f.mailer.notices().is_empty(),
        "the dialog shows the reason; no email"
    );

    let g = fixture(ok_pdf(), false).await;
    let sub = subscription(&g, |_| {}).await;
    for _ in 0..6 {
        tick(&g.state, &g.mailer).await;
        make_due(&g, &sub).await;
    }
    let stored = subs::get(&g.pool, sub.id).await.unwrap().unwrap();
    assert_eq!(deliveries(&g, &sub).await.len(), 6, "each run is recorded");
    assert_eq!(stored.consecutive_failures, 0);
    assert!(!stored.paused);
}
