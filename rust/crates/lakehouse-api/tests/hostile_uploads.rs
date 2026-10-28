//! `SEC-22` T1: the hostile upload fixtures, and what today's readers do with
//! them.
//!
//! A crafted workbook can ask the API's own process for more memory than the
//! machine has. Rust aborts the process on an allocation failure, and
//! `catch_unwind` cannot catch that: the only guard the API had was a
//! `catch_unwind` around the parser ([`lakehouse_api::upload_workbook`]'s
//! `guarded`), which turns a panic into a refusal and leaves an allocation
//! failure to end the process for every tenant.
//!
//! # What this file does
//!
//! Each fixture of `ops/fixtures/uploads/hostile/` is read in a CHILD
//! process, under a memory limit (`ulimit -v`), and this process must survive
//! it. The child is this same test binary, re-run with one test selected and
//! the fixture named in the environment: a separate address space, so an
//! abort there is this process's problem to observe, not its own.
//!
//! Both tests are `#[ignore]`d: the child role is only meaningful as a child
//! of the wrapper, and the wrapper spawns processes and sets limits. Run them
//! with:
//!
//! ```text
//! cargo test -p lakehouse-api --test hostile_uploads -- --ignored --nocapture
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

/// The fixtures of `ops/fixtures/uploads/hostile/`, by name.
const FIXTURES: [(&str, Expected); 2] = [
    (
        "huge_shared_strings.xlsx",
        Expected::Abort,
    ),
    ("huge_footer.parquet", Expected::Refusal),
];

/// What a fixture does to the process that reads it, as measured on
/// 2026-10-28 (`SEC-22` T1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expected {
    /// The process dies: the shared-strings header asks for 48 GB, the
    /// allocation fails, and Rust aborts. `catch_unwind` cannot catch that,
    /// which is the whole of `SEC-22`.
    Abort,
    /// The reader refuses the file with its own sentence: the Parquet
    /// footer is checked against the file before anything is read, so the
    /// sizes it declares cost nothing.
    Refusal,
}

/// The name of the test the wrapper re-runs in the child. `--exact` needs it.
const CHILD_TEST: &str = "reads_the_hostile_fixture_named_by_the_environment";

/// The environment variable that tells the child which fixture to read.
const FIXTURE_ENV: &str = "SEC22_HOSTILE_FIXTURE";

/// The memory limit the child runs under, in kibibytes: 512 MiB of address
/// space. The workbook asks for 48 GB, so it cannot be met whatever the
/// limit is; the limit is here so that the child fails on the limit and not
/// on the machine being out of memory.
const CHILD_ADDRESS_SPACE_KIB: u32 = 512 * 1024;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../../ops/fixtures/uploads/hostile/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// The child role: read the fixture the environment names, with the reader
/// that fixture is for, and say what came of it.
///
/// Without the environment variable this test does nothing, so it is safe to
/// run in the ordinary test run (it is `#[ignore]`d anyway).
#[test]
#[ignore = "the child process of `a_hostile_fixture_ends_only_the_child_process`"]
fn reads_the_hostile_fixture_named_by_the_environment() {
    let Ok(name) = std::env::var(FIXTURE_ENV) else {
        return;
    };
    let bytes = axum::body::Bytes::from(fixture(&name));
    let outcome: Result<String, String> = if name.ends_with(".xlsx") {
        lakehouse_api::upload_workbook::list_sheets(&bytes)
            .map(|info| format!("{info:?}"))
            .map_err(|err| err.to_string())
    } else {
        lakehouse_api::upload_parquet::inspect(&bytes)
            .map(|info| format!("{info:?}"))
            .map_err(|err| err.to_string())
    };
    match outcome {
        Ok(answer) => println!("[sec22] {name}: READ: {answer}"),
        Err(err) => println!("[sec22] {name}: REFUSED: {err}"),
    }
}

/// Each fixture is read in a child process under a memory limit, and this
/// process must be alive at the end of it.
///
/// The point of `SEC-22` is not that a fixture ends a process: it is that a
/// fixture that ends a process must end the CHILD's process and never the
/// API's. So the wrapper asserts what each fixture does (an abort or a
/// refusal) and that the wrapper itself is still running.
#[test]
#[ignore = "spawns a child under `ulimit -v`; run it with `--ignored`"]
fn a_hostile_fixture_ends_only_the_child_process() {
    let exe = std::env::current_exe().unwrap();
    for (name, expected) in FIXTURES {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("ulimit -v \"$1\"; shift; exec \"$0\" \"$@\"")
            .arg(&exe)
            .arg(CHILD_ADDRESS_SPACE_KIB.to_string())
            .arg("--exact")
            .arg(CHILD_TEST)
            .arg("--ignored")
            .arg("--nocapture")
            .env(FIXTURE_ENV, name);
        let output = command.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!("[sec22] {name}: child status {:?}", output.status);
        for line in stdout.lines().chain(stderr.lines()) {
            println!("[sec22] {name}: {line}");
        }
        match expected {
            Expected::Abort => {
                let signal = std::os::unix::process::ExitStatusExt::signal(&output.status);
                assert_eq!(
                    signal,
                    Some(6),
                    "{name}: the child was expected to die of SIGABRT"
                );
            }
            Expected::Refusal => {
                assert!(
                    output.status.success(),
                    "{name}: the child was expected to refuse the file"
                );
                assert!(
                    stdout.contains("REFUSED:"),
                    "{name}: the child was expected to refuse the file: {stdout}"
                );
            }
        }
    }
    println!("[sec22] the process that ran the fixtures is still here");
}
