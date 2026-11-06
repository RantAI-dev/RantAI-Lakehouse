//! `lakehouse-api` — the axum HTTP service for the `RantAI` Lakehouse
//! backend.
//!
//! This binary is the process entry point for `lakehouse-api`. It owns process-level
//! concerns (inspect-upload child process dispatch, `rustls` crypto provider
//! installation, runtime thread stack configuration) before handing off to
//! [`lakehouse_api::run`].
//!
//! `ADM-7 review R2`: this binary declares no module of its own since
//! `REL-20` (#397) moved the process into the library target
//! (`lakehouse_api::run`). The module tree lives only in `src/lib.rs`, so
//! `runtime_settings` is declared there and not here.

use anyhow::Context;

use lakehouse_api::{config, crypto_provider, upload_convert};

/// Builds the runtime by hand instead of `#[tokio::main]` only to set the
/// thread stack size: phase 2 catch-up with main (sqlparser 0.63), see
/// [`config::WORKER_STACK_BYTES`] for the measurement behind the value.
///
/// `SEC-22` D1: the child that reads an uploaded workbook or a Parquet file
/// is this same binary, started again with `inspect-upload`. Its branch comes
/// FIRST — before the runtime is built, before the configuration is read and
/// before the crypto provider is installed — so the child needs no
/// environment, starts no threads, opens no connection and dials nothing, and
/// so that a file which asks for more memory than the machine has ends the
/// CHILD and never the API. `PR #192` added the provider install; it stays
/// after this branch, because a child that reads a file does not dial.
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(code) = upload_convert::run_child_if_asked(&args) {
        std::process::exit(code);
    }
    // PR #192 review BLOCKER 10: install `ring` crypto provider before any task dials.
    if let Err(err) = crypto_provider::install_default_provider() {
        eprintln!("fatal: failed to install default rustls crypto provider: {err}");
        std::process::exit(1);
    }

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(config::WORKER_STACK_BYTES)
        .build()
        .context("failed to build the tokio runtime")?
        .block_on(lakehouse_api::run())
}
