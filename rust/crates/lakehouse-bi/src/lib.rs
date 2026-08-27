//! BI / dashboarding domain: static chart specs, SQL builders, and the
//! `ClickHouse`-backed board/chart store.
//!
//! Ports `src/lib/dashboard-specs.ts` and `src/services/clients/bi-store.ts`.
//! This crate is a library only — no axum routes are wired here.

pub mod specs;
