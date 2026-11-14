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
//!
//! `BI-57` (QA `LH-BI-2-02`) is decided by the engine too: whether a column's
//! values are all whole numbers is read in the same statement that reads the
//! minimum and maximum (`min(toFloat64(c) = floor(toFloat64(c)))`), and only
//! the engine can say which bin a row falls in. So the widths and the bars of a
//! year column under "Automatic" are asserted against the answers, not against
//! the statement text.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_bi::bin::{BinMode, Range, bin_records_predicate, max_scalar_sql, whole_scalar_sql};
use lakehouse_bi::grain::TimeContext;
use lakehouse_bi::sources::SourceColumn;
use lakehouse_bi::specs::ChartSource;
use lakehouse_bi::store::{ChartInput, InlineSql, spec_from_inline_sql};
use lakehouse_clickhouse::ChClient;
use lakehouse_core::ident::Ident;
use serde_json::Value;
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::clickhouse::ClickHouse;

/// The table of the rounding rule: two `a` rows and one `b`, so an average of
/// `a` is `1.666667` to six decimals and a maximum of it is `2`.
const AVG_TABLE: &str = "default.bi55_avg";

/// The source of the rounding chart: every row of [`AVG_TABLE`].
const AVG_SOURCE: &str = "SELECT kab, v FROM default.bi55_avg";

/// `BI-57`: the eleven whole years 2014 to 2024, one row each.
const YEARS_TABLE: &str = "default.bi57_years";

/// `BI-57`: a fractional column, where the maximum keeps the closed last bin.
const STAY_TABLE: &str = "default.bi57_stay";

/// `BI-57` (D9): a whole column, a `Float64` one and a `Decimal` one, so a
/// sum of each is read from the engine.
const SUMS_TABLE: &str = "default.bi57_sums";

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
    // `BI-57` (QA `LH-BI-2-02`): a whole-number column (years) and a
    // fractional one, so the bin rule is proven for both.
    ch.exec(
        &format!("CREATE TABLE {YEARS_TABLE} (tahun UInt16) ENGINE = MergeTree ORDER BY tahun"),
        None,
    )
    .await
    .expect("create the year table");
    let years: Vec<String> = (2014..=2024).map(|y| format!("({y})")).collect();
    ch.exec(
        &format!("INSERT INTO {YEARS_TABLE} VALUES {}", years.join(", ")),
        None,
    )
    .await
    .expect("fill the year table");
    ch.exec(
        &format!("CREATE TABLE {STAY_TABLE} (stay Float64) ENGINE = MergeTree ORDER BY stay"),
        None,
    )
    .await
    .expect("create the stay table");
    ch.exec(
        &format!("INSERT INTO {STAY_TABLE} VALUES (1.5), (2.0), (2.5), (3.0), (4.5)"),
        None,
    )
    .await
    .expect("fill the stay table");
    // `BI-57` (D9): a whole column, a `Float64` one and a `Decimal` one, over
    // two `a` rows and two `b` rows, so a sum of each is proven per group.
    ch.exec(
        &format!(
            "CREATE TABLE {SUMS_TABLE} (kab String, n UInt32, f Float64, d Decimal(18, 2)) \
             ENGINE = MergeTree ORDER BY kab"
        ),
        None,
    )
    .await
    .expect("create the sums table");
    ch.exec(
        &format!(
            "INSERT INTO {SUMS_TABLE} VALUES \
             ('a', 1, 1.25, 1.25), ('a', 2, 2.5, 2.50), ('b', 7, 0.1, 0.10), ('b', 0, 0.2, 0.20)"
        ),
        None,
    )
    .await
    .expect("fill the sums table");
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

/// `BI-57` (D9): a sum of a `Float64` or a `Decimal` column keeps its six
/// decimals, and a sum of whole numbers is the whole number it always was —
/// `round(x, 6)` of an integer is that integer in `ClickHouse`, so the builder
/// does not need to know the column's type.
#[tokio::test]
async fn a_sum_of_decimals_keeps_them_and_a_sum_of_whole_numbers_is_unchanged_on_the_engine() {
    let (url, _container) = shared_clickhouse().await;
    let ch = ChClient::new(url.clone(), "default".to_owned(), String::new());
    let input: ChartInput = serde_json::from_value(serde_json::json!({
        "title": "BI-57 sums",
        "kind": "bar",
        "dimension": "kab",
        "measures": ["n", "f", "d"],
    }))
    .expect("a bar chart with three measures decodes");
    let columns = [
        SourceColumn {
            name: "kab".to_owned(),
            ty: "String".to_owned(),
        },
        SourceColumn {
            name: "n".to_owned(),
            ty: "UInt32".to_owned(),
        },
        SourceColumn {
            name: "f".to_owned(),
            ty: "Float64".to_owned(),
        },
        SourceColumn {
            name: "d".to_owned(),
            ty: "Decimal(18, 2)".to_owned(),
        },
    ];
    let spec = spec_from_inline_sql(
        &input,
        InlineSql {
            sql: "SELECT kab, n, f, d FROM default.bi57_sums",
            columns: &columns,
        },
        ChartSource::Ui,
        "ui",
        &TimeContext::default(),
    )
    .expect("the chart is saved");
    let statement = spec.spec.sql;
    for measure in ["n", "f", "d"] {
        assert!(
            statement.contains(&format!("round(sum({measure}), 6)")),
            "the saved statement rounds the sum of {measure} to six decimals: {statement}"
        );
    }
    let result = ch
        .query(&statement, None)
        .await
        .expect("the saved statement runs on the engine");

    let a = row(&result.data, "a");
    assert_eq!(
        number(a.get("n").expect("the whole sum")),
        3.0,
        "a sum of whole numbers is unchanged: {statement}"
    );
    assert_eq!(
        number(a.get("f").expect("the float sum")),
        3.75,
        "1.25 + 2.5 keeps its decimals: {statement}"
    );
    assert_eq!(
        number(a.get("d").expect("the decimal sum")),
        3.75,
        "1.25 + 2.50 keeps its decimals: {statement}"
    );

    let b = row(&result.data, "b");
    assert_eq!(number(b.get("n").expect("the whole sum")), 7.0);
    assert_eq!(
        number(b.get("f").expect("the float sum")),
        0.3,
        "0.1 + 0.2 reads 0.3, not 0.30000000000000004: {statement}"
    );
    assert_eq!(
        number(b.get("d").expect("the decimal sum")),
        0.3,
        "0.10 + 0.20 reads 0.3: {statement}"
    );
}

/// The last years of [`YEARS_TABLE`], as the filtered base a chart reads.
fn years_from(first: u16) -> String {
    format!("(SELECT * FROM {YEARS_TABLE} WHERE tahun >= {first}) AS flt")
}

/// The statement a binned chart runs, as `Relation::Ranged` renders it: the
/// bin expression over the base with the range's scalars joined in, one row
/// per bin (`builder.rs`, `Relation::Ranged`).
fn bins_statement(column: &Ident, mode: BinMode, base: &str) -> String {
    let range = Range::new(column, mode);
    format!(
        "SELECT {} AS b, count() AS n FROM (SELECT * FROM {base} CROSS JOIN ({}) AS rng) GROUP BY b ORDER BY b",
        range.bin_expr(),
        range.scalars_sql(base)
    )
}

/// The `(bin, count)` pairs a bins statement answers.
async fn bins_of(ch: &ChClient, statement: &str) -> Vec<(f64, f64)> {
    let result = ch
        .query(statement, None)
        .await
        .expect("the bins statement runs on the engine");
    result
        .data
        .iter()
        .map(|row| {
            (
                number(row.get("b").expect("the bin column")),
                number(row.get("n").expect("the count column")),
            )
        })
        .collect()
}

/// The one row a range read answers: `(width, min, max, whole)`.
async fn range_read(
    ch: &ChClient,
    column: &Ident,
    mode: BinMode,
    base: &str,
) -> (f64, f64, f64, f64) {
    let statement = Range::new(column, mode).read_sql(base);
    let result = ch
        .query(&statement, None)
        .await
        .expect("the range read runs on the engine");
    let row = &result.data[0];
    (
        number(row.get("width").expect("the width column")),
        number(row.get("min").expect("the minimum column")),
        number(row.get("max").expect("the maximum column")),
        number(row.get("whole").expect("the whole column")),
    )
}

/// `BI-57` (QA `LH-BI-2-02`): five years under "Automatic" are five bars, one
/// year each, where the width used to be 0.5 and the bars read
/// "2,020 – 2,020.5".
#[tokio::test]
async fn five_years_under_automatic_bins_are_five_bars_one_year_each() {
    let (url, _container) = shared_clickhouse().await;
    let ch = ChClient::new(url.clone(), "default".to_owned(), String::new());
    let tahun = Ident::new("tahun").expect("a column name");
    let base = years_from(2020);

    assert_eq!(
        range_read(&ch, &tahun, BinMode::Auto, &base).await,
        (1.0, 2020.0, 2024.0, 1.0),
        "the automatic range of a whole-number column is 1 wide and says so"
    );
    let statement = bins_statement(&tahun, BinMode::Auto, &base);
    assert_eq!(
        bins_of(&ch, &statement).await,
        vec![
            (2020.0, 1.0),
            (2021.0, 1.0),
            (2022.0, 1.0),
            (2023.0, 1.0),
            (2024.0, 1.0)
        ],
        "five years are five bars: {statement}"
    );
}

/// `BI-57` D4: six and ten years still fit one bar per year, eleven do not, so
/// their width is 2 (down from ten bars whose last held two years). Counted
/// bins take the same rule.
#[tokio::test]
async fn six_and_ten_years_are_one_bar_per_year_and_eleven_years_are_bins_of_two() {
    let (url, _container) = shared_clickhouse().await;
    let ch = ChClient::new(url.clone(), "default".to_owned(), String::new());
    let tahun = Ident::new("tahun").expect("a column name");

    for mode in [BinMode::Auto, BinMode::Count(10)] {
        for (first, expected, width) in [
            (
                2019,
                vec![
                    (2019.0, 1.0),
                    (2020.0, 1.0),
                    (2021.0, 1.0),
                    (2022.0, 1.0),
                    (2023.0, 1.0),
                    (2024.0, 1.0),
                ],
                1.0,
            ),
            (
                2015,
                (2015..=2024).map(|y| (f64::from(y), 1.0)).collect(),
                1.0,
            ),
            (
                2014,
                vec![
                    (2014.0, 2.0),
                    (2016.0, 2.0),
                    (2018.0, 2.0),
                    (2020.0, 2.0),
                    (2022.0, 3.0),
                ],
                2.0,
            ),
        ] {
            let base = years_from(first);
            let (w, _, _, _) = range_read(&ch, &tahun, mode, &base).await;
            assert_eq!(w, width, "{mode:?} from {first} is {width} wide");
            let statement = bins_statement(&tahun, mode, &base);
            assert_eq!(
                bins_of(&ch, &statement).await,
                expected,
                "{mode:?} from {first}: {statement}"
            );
        }
    }
}

/// `BI-57` D3: a fractional column keeps its width, and its maximum stays in
/// the closed last bin (4.5 in "4 – 4.5", as `BI-10` had it).
#[tokio::test]
async fn a_fractional_column_keeps_its_width_and_its_last_bin_holds_the_maximum() {
    let (url, _container) = shared_clickhouse().await;
    let ch = ChClient::new(url.clone(), "default".to_owned(), String::new());
    let stay = Ident::new("stay").expect("a column name");

    assert_eq!(
        range_read(&ch, &stay, BinMode::Auto, STAY_TABLE).await,
        (0.5, 1.5, 4.5, 0.0),
        "a fractional column is not whole and keeps its width"
    );
    let statement = bins_statement(&stay, BinMode::Auto, STAY_TABLE);
    assert_eq!(
        bins_of(&ch, &statement).await,
        vec![(1.5, 1.0), (2.0, 1.0), (2.5, 1.0), (3.0, 1.0), (4.0, 1.0)],
        "the maximum sits in the last bin: {statement}"
    );
}

/// `BI-57` D3: a width typed as 1 over whole numbers gives the maximum its own
/// bin, like an automatic one does.
#[tokio::test]
async fn a_typed_width_of_one_gives_the_last_year_its_own_bin_too() {
    let (url, _container) = shared_clickhouse().await;
    let ch = ChClient::new(url.clone(), "default".to_owned(), String::new());
    let tahun = Ident::new("tahun").expect("a column name");
    let base = years_from(2020);

    let statement = bins_statement(&tahun, BinMode::Width(1.0), &base);
    assert_eq!(
        bins_of(&ch, &statement).await,
        vec![
            (2020.0, 1.0),
            (2021.0, 1.0),
            (2022.0, 1.0),
            (2023.0, 1.0),
            (2024.0, 1.0)
        ],
        "a typed width of 1 also gives the maximum its own bin: {statement}"
    );
}

/// `BI-57` D3: the rows a clicked bin lists are the rows the bar holds — the
/// last year lists itself, and a closed last bin over a fractional column is
/// unchanged.
#[tokio::test]
async fn a_clicked_year_lists_that_year_and_the_last_year_lists_itself() {
    let (url, _container) = shared_clickhouse().await;
    let ch = ChClient::new(url.clone(), "default".to_owned(), String::new());
    let tahun = Ident::new("tahun").expect("a column name");
    let base = years_from(2020);

    for (lower, expected) in [(2023.0, vec![2023.0]), (2024.0, vec![2024.0])] {
        let predicate = bin_records_predicate(
            &tahun,
            lower,
            1.0,
            Some(&max_scalar_sql(&tahun, &base)),
            Some(&whole_scalar_sql(&tahun, &base)),
        );
        let statement = format!("SELECT tahun FROM {base} WHERE {predicate} ORDER BY tahun");
        let result = ch
            .query(&statement, None)
            .await
            .expect("the records statement runs on the engine");
        let listed: Vec<f64> = result
            .data
            .iter()
            .map(|row| number(row.get("tahun").expect("the year column")))
            .collect();
        assert_eq!(
            listed, expected,
            "the clicked bin lists itself: {statement}"
        );
    }

    let stay = Ident::new("stay").expect("a column name");
    let predicate = bin_records_predicate(
        &stay,
        4.0,
        0.5,
        Some(&max_scalar_sql(&stay, STAY_TABLE)),
        None,
    );
    let statement = format!("SELECT stay FROM {STAY_TABLE} WHERE {predicate} ORDER BY stay");
    let result = ch
        .query(&statement, None)
        .await
        .expect("the records statement runs on the engine");
    let listed: Vec<f64> = result
        .data
        .iter()
        .map(|row| number(row.get("stay").expect("the stay column")))
        .collect();
    assert_eq!(
        listed,
        vec![4.5],
        "the closed last bin of a fractional column is unchanged: {statement}"
    );
}

/// `BI-57 (D3), review round 1 R1`: a typed width of 1 over a fractional column
/// whose maximum happens to be a whole number is still not a whole column
/// (1.5 and 2.5 are not), so the maximum joins the closed last bin instead of
/// getting one of its own, and a click on that bin lists it.
#[tokio::test]
async fn a_fractional_column_in_bins_of_one_keeps_today_s_bins_and_its_click_lists_the_maximum() {
    let (url, _container) = shared_clickhouse().await;
    let ch = ChClient::new(url.clone(), "default".to_owned(), String::new());
    let stay = Ident::new("stay").expect("a column name");
    let b = format!("(SELECT * FROM {STAY_TABLE} WHERE stay <= 3) AS flt");

    assert_eq!(
        range_read(&ch, &stay, BinMode::Width(1.0), &b).await,
        (1.0, 1.5, 3.0, 0.0),
        "a fractional column is not whole, even when its maximum is"
    );
    let statement = bins_statement(&stay, BinMode::Width(1.0), &b);
    assert_eq!(
        bins_of(&ch, &statement).await,
        vec![(1.0, 1.0), (2.0, 3.0)],
        "the maximum sits in the closed last bin: {statement}"
    );
    let predicate = bin_records_predicate(
        &stay,
        2.0,
        1.0,
        Some(&max_scalar_sql(&stay, &b)),
        Some(&whole_scalar_sql(&stay, &b)),
    );
    let statement = format!("SELECT stay FROM {b} WHERE {predicate} ORDER BY stay");
    let result = ch
        .query(&statement, None)
        .await
        .expect("the records statement runs on the engine");
    let listed: Vec<f64> = result
        .data
        .iter()
        .map(|row| number(row.get("stay").expect("the stay column")))
        .collect();
    assert_eq!(
        listed,
        vec![2.0, 2.5, 3.0],
        "the clicked closed last bin lists the maximum: {statement}"
    );
}
