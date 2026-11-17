//! `BI-46` review round 1, R1 (QA `LH-BI-6-01`): the breakdown cap keeps a
//! NULL breakdown value on a real `ClickHouse`.
//!
//! `ClickHouse` answers `NULL IN (<set>)` with false unless `transform_null_in`
//! is set, and nothing in this repository sets it, so the bare form the cap
//! shipped with
//! (`{b} IN (SELECT {b} FROM … GROUP BY {b} ORDER BY … LIMIT N)`) dropped
//! every row whose breakdown value is NULL — a nullable mart column, a
//! left-joined column (`join_use_nulls = 1`) or a bucket of a NULL date. The
//! cap now compares `(isNull(b), ifNull(toString(b), ''))` on both sides. A
//! string test (`builder::tests::a_breakdown_cap_keeps_a_null_breakdown_value_and_does_not_compare_the_bare_column`)
//! pins the clause; this test proves the statement on the engine, which is the
//! only place the semantics of `IN` over NULL is decided.
//!
//! The chart is built through the same public save path the console uses
//! (`store::spec_from_inline_sql`), so what runs here is the statement a chart
//! is stored with, not a hand-written approximation of it. The version is the
//! one `docker-compose.yml` pins (`clickhouse/clickhouse-server:26.8`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_bi::grain::TimeContext;
use lakehouse_bi::sources::SourceColumn;
use lakehouse_bi::specs::ChartSource;
use lakehouse_bi::store::{ChartInput, InlineSql, spec_from_inline_sql};
use lakehouse_clickhouse::ChClient;
use serde_json::Value;
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::clickhouse::ClickHouse;

/// The table the chart reads: a `Nullable(String)` breakdown with one NULL
/// group, an empty-string group and two named groups.
const TABLE: &str = "default.r1_cap";

/// The relation the chart is built over: every row of [`TABLE`].
const SOURCE: &str = "SELECT kab, cat, v FROM default.r1_cap";

/// ONE `ClickHouse` per test binary (the arrangement of
/// `tests/gold_publication.rs`); the container is never stopped by the test.
static CLICKHOUSE: tokio::sync::OnceCell<(String, testcontainers::ContainerAsync<ClickHouse>)> =
    tokio::sync::OnceCell::const_new();

async fn shared_clickhouse() -> &'static (String, testcontainers::ContainerAsync<ClickHouse>) {
    CLICKHOUSE
        .get_or_init(|| async {
            // The compose service's env trio, all three required — see
            // `tests/gold_publication.rs` for why.
            let container = ClickHouse::default()
                .with_env_var("CLICKHOUSE_USER", "default")
                .with_env_var("CLICKHOUSE_PASSWORD", "")
                .with_env_var("CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT", "1")
                .with_tag("26.8")
                .start()
                .await
                .expect("start the ClickHouse testcontainer");
            let host = container.get_host().await.expect("container host");
            let port = container
                .get_host_port_ipv4(8123)
                .await
                .expect("container HTTP port");
            let url = format!("http://{host}:{port}");
            let ch = ChClient::new(url.clone(), "default".to_owned(), String::new());
            seed(&ch).await;
            (url, container)
        })
        .await
}

/// The rows the cap ranks: NULL is the largest total (100), then `x` (60),
/// `y` (30) and the empty string (7). The empty string is there so a key made
/// of `ifNull(toString(b), '')` alone would merge it with NULL — the pair must
/// not.
async fn seed(ch: &ChClient) {
    ch.exec(
        &format!(
            "CREATE TABLE {TABLE} (kab String, cat Nullable(String), v UInt32) \
             ENGINE = MergeTree ORDER BY kab"
        ),
        None,
    )
    .await
    .expect("create the table");
    ch.exec(
        &format!(
            "INSERT INTO {TABLE} VALUES ('a', NULL, 100), ('a', 'x', 50), \
             ('a', 'y', 30), ('b', 'x', 10), ('b', '', 7)"
        ),
        None,
    )
    .await
    .expect("fill the table");
}

/// The statement a bar chart (dimension `kab`, breakdown `cat`, limit
/// `limit`) is saved with, through the console's own save path.
fn chart_statement(limit: u32) -> String {
    let input: ChartInput = serde_json::from_value(serde_json::json!({
        "title": "R1 cap",
        "kind": "bar",
        "dimension": "kab",
        "measures": ["v"],
        "breakdown": "cat",
        "limit": limit,
    }))
    .expect("a bar chart with a breakdown decodes");
    let columns = [
        SourceColumn {
            name: "kab".to_owned(),
            ty: "String".to_owned(),
        },
        SourceColumn {
            name: "cat".to_owned(),
            ty: "Nullable(String)".to_owned(),
        },
        SourceColumn {
            name: "v".to_owned(),
            ty: "UInt32".to_owned(),
        },
    ];
    let spec = spec_from_inline_sql(
        &input,
        InlineSql {
            sql: SOURCE,
            columns: &columns,
        },
        ChartSource::Ui,
        "ui",
        &TimeContext::default(),
    )
    .expect("the chart is saved");
    spec.spec.sql
}

/// The breakdown of every row, in the order the engine returned them, with a
/// NULL value kept as `None` so an assertion can tell it from the empty
/// string.
fn breakdowns(data: &[serde_json::Map<String, Value>]) -> Vec<Option<String>> {
    data.iter()
        .map(|row| match row.get("cat") {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => Some(text.clone()),
            Some(other) => panic!("cat is not text: {other}"),
        })
        .collect()
}

/// `limit 20` keeps every series, the NULL one included; the empty string is
/// a series of its own, not folded into NULL.
#[tokio::test]
async fn a_breakdown_cap_at_twenty_returns_the_null_series_and_the_empty_string_series() {
    let (url, _container) = shared_clickhouse().await;
    let ch = ChClient::new(url.clone(), "default".to_owned(), String::new());
    let statement = chart_statement(20);
    assert!(
        statement.contains("(isNull(cat), ifNull(toString(cat), '')) IN"),
        "the saved statement carries the NULL-safe cap: {statement}"
    );
    let result = ch
        .query(&statement, None)
        .await
        .expect("the capped statement runs on the engine");
    let mut got = breakdowns(&result.data);
    got.sort();
    assert_eq!(
        got,
        vec![
            None,
            Some(String::new()),
            Some("x".to_owned()),
            Some("x".to_owned()),
            Some("y".to_owned()),
        ],
        "five rows, the NULL series among them, in: {statement}"
    );
}

/// `limit 1` with NULL the largest total keeps the NULL series and nothing
/// else. The bare form answered no row at all here, because the value it kept
/// was the one the outer `IN` then rejected.
#[tokio::test]
async fn a_breakdown_cap_at_one_keeps_only_the_null_series_when_it_is_the_largest() {
    let (url, _container) = shared_clickhouse().await;
    let ch = ChClient::new(url.clone(), "default".to_owned(), String::new());
    let statement = chart_statement(1);
    let result = ch
        .query(&statement, None)
        .await
        .expect("the capped statement runs on the engine");
    assert_eq!(
        breakdowns(&result.data),
        vec![None],
        "only the NULL series: {statement}"
    );
    assert_eq!(
        result.data[0].get("v").and_then(Value::as_f64),
        Some(100.0),
        "{statement}"
    );
}
