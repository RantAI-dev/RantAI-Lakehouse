//! The `lakehouse-auth` integration tests, as ONE test binary (`OPS-32`).
//!
//! Every file in this directory used to be its own test crate: Cargo built and
//! linked 8 binaries, each one linking `lakehouse-auth`, `jsonwebtoken`, `sqlx`
//! and their dependencies again, before any of them ran. This file declares each
//! of those files as a module, and the package sets `autotests = false` with one
//! `[[test]] name = "it"` target, so the crate links once.
//!
//! No test changed in the move, and neither does isolation under CI: nextest
//! runs every test in a process of its own, so each still gets its own
//! `#[sqlx::test]` database, and `openfga.rs`'s `wiremock` server is still its
//! own process.
//!
//! A new `tests/<name>.rs` gets a `mod <name>;` line below, in alphabetical
//! order. Without that line the file compiles into nothing and its tests
//! disappear silently, which is why `scripts/ci/check_test_modules.sh` fails
//! the `Repo lints` job for an undeclared file. An edit to an existing file
//! needs nothing.

mod oidc;
mod openfga;
mod pages_per_role_migration;
mod password;
mod repository;
mod service_token;
mod session;
mod throttle;
