//! The `ClickHouse` fixture the Bronze registry tests share (`DATA-42` D7).
//!
//! One real `ClickHouse` in a `testcontainers` container, the six registry
//! tables their owner declares, and the helpers that write a row into one
//! of them.
//!
//! `DATA-40` wrote these helpers inside `routes::catalog`'s own test
//! module, because the catalog was the only reader it touched. `DATA-42`
//! moves them here so the readers it fixes — `governance`, `overview`,
//! `storage`, `lineage` and `catalog_profile` — run against the same
//! fixture: one definition of "a registry with a slug in it", not five
//! copies that drift apart.
//!
//! ONE container per TEST, not per test binary: the container is a local,
//! so it is dropped — and removed — when the test ends, on a panic too. A
//! container in a `static` is never dropped: the test harness exits the
//! process, which skips the drop of a static
//! (`tests/gold_publication.rs` leaves its container running for that
//! reason).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::time::Duration;

use lakehouse_clickhouse::ChClient;
use lakehouse_core::ident::SqlLiteral;
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::clickhouse::ClickHouse;

use crate::config::Config;
use crate::state::AppState;

/// A `ClickHouse` for one test: its address, and the container it runs in.
///
/// # Panics
///
/// Panics when the container cannot start or the server inside it never
/// answers — a fixture that cannot be built fails the test that asked for
/// it, which is the only honest outcome.
pub(crate) async fn clickhouse() -> (String, testcontainers::ContainerAsync<ClickHouse>) {
    // The compose service's env trio; all three are required, see
    // `tests/gold_publication.rs`.
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
    wait_until_ready(&ch).await;
    create_registry(&ch).await;
    (url, container)
}

/// The container is running when `start` returns; the server inside it may
/// still be booting.
///
/// # Panics
///
/// Panics when the server does not answer within the wait.
pub(crate) async fn wait_until_ready(ch: &ChClient) {
    for _ in 0..60 {
        if ch.rows("SELECT 1", None).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("the ClickHouse testcontainer did not answer");
}

/// The six registry tables, with the shapes their owner declares
/// (`dagster/dispar_orchestrate/bronze_catalog.py`'s
/// `_registry_schemas`): `ReplacingMergeTree ORDER BY slug`, and
/// `ORDER BY (slug, key_asli)` for the columns.
///
/// # Panics
///
/// Panics when the engine refuses the DDL.
pub(crate) async fn create_registry(ch: &ChClient) {
    ch.exec("CREATE DATABASE IF NOT EXISTS lake", None)
        .await
        .expect("create the lake database");
    for registry in ["bronze_meta", "bronze_meta_sec"] {
        for (table, columns, order_by) in [
            (
                "dataset_catalog",
                "slug String, title String, description String, tier String, updated_at String, table_name String",
                "slug",
            ),
            (
                "dataset_sync",
                "slug String, title String, description String, table_name String, total UInt64, author String, frekuensi String, satuan String, klasifikasi String",
                "slug",
            ),
            (
                "dataset_column",
                "slug String, key_asli String, tipe String, deskripsi String",
                "(slug, key_asli)",
            ),
        ] {
            let sql = format!(
                "CREATE TABLE IF NOT EXISTS lake.`{registry}.{table}` ({columns}) \
                 ENGINE = ReplacingMergeTree ORDER BY {order_by}"
            );
            ch.exec(&sql, None)
                .await
                .expect("create the registry table");
        }
    }
}

/// One catalog row: the registry's own row for a dataset.
///
/// # Panics
///
/// Panics when the engine refuses the insert.
pub(crate) async fn seed_catalog(
    ch: &ChClient,
    registry: &str,
    slug: &str,
    title: &str,
    table_name: &str,
) {
    // The tier a real ingest writes (`bronze_catalog.py`'s
    // `register_bronze_table`).
    seed_catalog_with_tier(ch, registry, slug, title, table_name, "primer").await;
}

/// One catalog row with its own `tier`.
///
/// `WEB-3` S3a-D4: the classification list reports a registry dataset only
/// when its tier is one of the product's classification levels, and a real
/// ingest writes `primer`, so a test of that reader needs to write the other
/// kind of tier. Kept beside [`seed_catalog`] rather than copied.
///
/// # Panics
///
/// Panics when the engine refuses the insert.
pub(crate) async fn seed_catalog_with_tier(
    ch: &ChClient,
    registry: &str,
    slug: &str,
    title: &str,
    table_name: &str,
    tier: &str,
) {
    let slug = SqlLiteral::from(slug);
    let title = SqlLiteral::from(title);
    let table_name = SqlLiteral::from(table_name);
    let tier = SqlLiteral::from(tier);
    let sql = format!(
        "INSERT INTO lake.`{registry}.dataset_catalog` \
         (slug, title, description, tier, updated_at, table_name) \
         VALUES ({slug}, {title}, 'the dataset', {tier}, '2026-11-01T00:00:00Z', {table_name})"
    );
    ch.exec(&sql, None).await.expect("seed the catalog row");
}

/// One sync row: the registry's own row for a dataset's rows, published by
/// the team `team`.
///
/// # Panics
///
/// Panics when the engine refuses the insert.
pub(crate) async fn seed_sync(
    ch: &ChClient,
    registry: &str,
    slug: &str,
    title: &str,
    table_name: &str,
    total: u64,
) {
    seed_sync_with_author(ch, registry, slug, title, table_name, total, "team").await;
}

/// One sync row published by a named team.
///
/// `DATA-42` T2: the lineage reader draws one publisher edge per registry
/// row, so a test of "the primary wins" needs the two registries to name
/// DIFFERENT authors — which [`seed_sync`] cannot do.
///
/// # Panics
///
/// Panics when the engine refuses the insert.
pub(crate) async fn seed_sync_with_author(
    ch: &ChClient,
    registry: &str,
    slug: &str,
    title: &str,
    table_name: &str,
    total: u64,
    author: &str,
) {
    let slug = SqlLiteral::from(slug);
    let title = SqlLiteral::from(title);
    let table_name = SqlLiteral::from(table_name);
    let author = SqlLiteral::from(author);
    let sql = format!(
        "INSERT INTO lake.`{registry}.dataset_sync` \
         (slug, title, description, table_name, total, author, frekuensi, satuan, klasifikasi) \
         VALUES ({slug}, {title}, 'the dataset', {table_name}, {total}, {author}, 'daily', '', '')"
    );
    ch.exec(&sql, None).await.expect("seed the sync row");
}

/// One column row.
///
/// # Panics
///
/// Panics when the engine refuses the insert.
pub(crate) async fn seed_column(ch: &ChClient, registry: &str, slug: &str, key: &str) {
    seed_column_with(ch, registry, slug, key, "UInt64", "the column").await;
}

/// One column row with its own type and description.
///
/// `DATA-39` T2: a reader's answer for a column is its DESCRIPTION, so a
/// test of "the primary registry wins" or "the newer row wins" needs two
/// copies of ONE column that say different things — which [`seed_column`]
/// cannot do (it always writes `UInt64` / "the column").
///
/// # Panics
///
/// Panics when the engine refuses the insert.
pub(crate) async fn seed_column_with(
    ch: &ChClient,
    registry: &str,
    slug: &str,
    key: &str,
    ty: &str,
    description: &str,
) {
    let slug = SqlLiteral::from(slug);
    let key = SqlLiteral::from(key);
    let ty = SqlLiteral::from(ty);
    let description = SqlLiteral::from(description);
    let sql = format!(
        "INSERT INTO lake.`{registry}.dataset_column` (slug, key_asli, tipe, deskripsi) \
         VALUES ({slug}, {key}, {ty}, {description})"
    );
    ch.exec(&sql, None).await.expect("seed the column row");
}

/// The state the detail page reads with: this deployment's `ClickHouse` and
/// no store. A deployment without a store sees the page's governance and
/// annotation reads empty, which is what these tests are about.
///
/// # Panics
///
/// Panics when the environment is not a valid configuration.
pub(crate) fn state_for(ch_url: &str) -> AppState {
    let mut env = HashMap::new();
    env.insert("CH_URL".to_owned(), ch_url.to_owned());
    AppState::new(Config::from_map(&env).expect("a valid test Config"))
}

/// The same `ClickHouse`, with the setting that pins the engine's read
/// order: `max_threads=1` makes it read one part at a time, in the order
/// the parts were written. A URL setting, so the statement the module
/// sends is the statement that runs.
pub(crate) fn pinned(ch_url: &str) -> String {
    format!("{ch_url}/?max_threads=1")
}
