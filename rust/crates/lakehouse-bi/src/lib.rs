//! BI / dashboarding domain: static chart specs, SQL builders, and the
//! `ClickHouse`-backed board/chart store.
//!
//! Ports `src/lib/dashboard-specs.ts` and `src/services/clients/bi-store.ts`.
//! This crate is a library only — no axum routes are wired here.

pub mod bin;
pub mod builder;
pub mod click;
pub mod embed_access;
pub mod fields;
pub mod filters;
pub mod folders;
pub mod formula;
pub mod grain;
pub mod groups;
pub mod hierarchies;
pub mod joins;
pub mod layers;
pub mod metrics;
pub mod params;
pub mod shape;
pub mod sources;
pub mod specs;
pub mod store;
pub mod tables;
