//! `BI-55` (QA `LH-BI-6-04`): the numbers a chart's statement computes, on a
//! real `ClickHouse`.
//!
//! The rounding rule of `BI-55` (D4) is written once, in
//! `Aggregate::rounded`, and the statement a chart is saved with carries it.
//! Whether the engine then answers the number the screen is meant to show is
//! decided by the engine alone — `round(avg(v), 6)` of `1, 2, 2` is
//! `1.666667`, and `round(max(v), 6)` of whole numbers is the whole number —
//! so this file proves it where it is decided, not in a string assertion.
//!
//! The charts are built through the same public save path the console uses
//! (`store::spec_from_inline_sql`), so what runs here is the statement a chart
//! is stored with. The engine version is the one `docker-compose.yml` pins
//! (`clickhouse/clickhouse-server:26.8`).

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

/// The table of the rounding rule: two `a` rows and one `b`, so an average of
/// `a` is `1.666667` to six decimals and a maximum of it is `2`.
const AVG_TABLE: &str = "default.bi55_avg";

/// The source of the rounding chart: every row of [`AVG_TABLE`].
const AVG_SOURCE: &str = "SELECT kab, v FROM default.bi55_avg";

/// ONE `ClickHouse` per test binary (the arrangement of
/// `tests/breakdown_cap.rs`); the container is never stopped by the test.
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

/// The rows of [`AVG_TABLE`]: `a` averages `1.666667` and holds whole numbers
/// only, so its maximum is the whole `2`.
async fn seed(ch: &ChClient) {
    ch.exec(
        &format!("CREATE TABLE {AVG_TABLE} (kab String, v UInt32) ENGINE = MergeTree ORDER BY kab"),
        None,
    )
    .await
    .expect("create the rounding table");
    ch.exec(
        &format!("INSERT INTO {AVG_TABLE} VALUES ('a', 1), ('a', 2), ('a', 2), ('b', 10)"),
        None,
    )
    .await
    .expect("fill the rounding table");
}

/// A JSON number, or a number the engine quoted as a 64-bit integer, read as
/// `f64` so an assertion can compare it to a decimal literal.
fn number(value: &Value) -> f64 {
    match value {
        Value::Number(n) => n.as_f64().expect("a JSON number"),
        Value::String(text) => text.parse().expect("a quoted number"),
        other => panic!("not a number: {other}"),
    }
}

/// The row whose `kab` is `kab`.
fn row<'a>(
    data: &'a [serde_json::Map<String, Value>],
    kab: &str,
) -> &'a serde_json::Map<String, Value> {
    data.iter()
        .find(|row| row.get("kab") == Some(&Value::String(kab.to_owned())))
        .unwrap_or_else(|| panic!("no row for kab {kab}: {data:?}"))
}

/// An average keeps its six decimals and a maximum of whole numbers stays a
/// whole number, through the statement a chart is saved with and on the
/// engine.
#[tokio::test]
async fn an_average_keeps_its_decimals_and_a_maximum_of_whole_numbers_stays_whole_on_the_engine() {
    let (url, _container) = shared_clickhouse().await;
    let ch = ChClient::new(url.clone(), "default".to_owned(), String::new());
    let input: ChartInput = serde_json::from_value(serde_json::json!({
        "title": "BI-55 avg",
        "kind": "bar",
        "dimension": "kab",
        "measures": ["v", {"column": "v", "aggregate": "avg"}, {"column": "v", "aggregate": "max"}],
    }))
    .expect("a bar chart with three measures decodes");
    let columns = [
        SourceColumn {
            name: "kab".to_owned(),
            ty: "String".to_owned(),
        },
        SourceColumn {
            name: "v".to_owned(),
            ty: "UInt32".to_owned(),
        },
    ];
    let spec = spec_from_inline_sql(
        &input,
        InlineSql {
            sql: AVG_SOURCE,
            columns: &columns,
        },
        ChartSource::Ui,
        "ui",
        &TimeContext::default(),
    )
    .expect("the chart is saved");
    let statement = spec.spec.sql;
    assert!(
        statement.contains("round(avg(v), 6)"),
        "the saved statement keeps the average's decimals: {statement}"
    );
    assert!(
        statement.contains("round(max(v), 6)"),
        "the saved statement keeps the maximum's decimals: {statement}"
    );
    let result = ch
        .query(&statement, None)
        .await
        .expect("the saved statement runs on the engine");

    let a = row(&result.data, "a");
    assert_eq!(
        number(a.get("avg_v").expect("the average column")),
        1.666_667,
        "an average of 1, 2, 2 keeps six decimals: {statement}"
    );
    assert_eq!(
        number(a.get("max_v").expect("the maximum column")),
        2.0,
        "a maximum of whole numbers stays whole: {statement}"
    );
    assert_eq!(
        number(a.get("v").expect("the sum column")),
        5.0,
        "a sum is still a whole number: {statement}"
    );

    let b = row(&result.data, "b");
    assert_eq!(number(b.get("v").expect("the sum column")), 10.0);
    assert_eq!(number(b.get("avg_v").expect("the average column")), 10.0);
    assert_eq!(number(b.get("max_v").expect("the maximum column")), 10.0);
}
