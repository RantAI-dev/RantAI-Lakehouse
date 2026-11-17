//! `lakehousectl`: the operator's tool for the `RantAI` Lakehouse (REL-2 S3).
//!
//! # What this tool is
//!
//! One command installs the whole stack — the API, the console, the
//! databases and the object store — on a Linux host, online or
//! air-gapped, and one command updates it (REL-2 S4). The product
//! owner's words: "like T3 Code's installer and updater" (REL-2, 2026-10-30).
//!
//! # The idea it is built on
//!
//! **Sign one file, verify everything through it** (REL-2 decision D3).
//! The release manifest `release.json` is that file: it holds every
//! image's digest, every file's checksum and the upgrade path, and it
//! is signed with the release key. This tool holds the PUBLIC key,
//! compiled in (`ops/release/release.pub`), and checks the manifest's
//! signature before it trusts a single byte of it. No `cosign`, no
//! registry and no network are needed for that check, so an air-gapped
//! host verifies exactly what an online one does.
//!
//! # What this tool never does
//!
//! - It never trusts a file it has not verified against the signed
//!   manifest.
//! - It never overwrites a secret that is already there (REL-2 decision
//!   D6): a second install changes nothing.
//! - It never accepts another public key in a release build: the key is
//!   compiled in, and the only override is a test-only build feature
//!   (see `verify`).

mod backup;
mod cli;
mod docker;
mod env;
mod error;
mod files;
mod http;
mod images;
mod install;
mod manifest;
mod pause;
mod preflight;
mod release;
mod rollback;
mod runner;
mod secrets;
mod self_update;
mod state;
mod status;
mod sys;
mod update;
mod verify;
mod version;

use clap::Parser;

use crate::cli::{Cli, Command};

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            // The message goes to stderr and nothing else does: a
            // failure an operator reads must say what went wrong and
            // nothing about the machine around it.
            eprintln!("lakehousectl: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// Run the command the operator asked for.
///
/// # Errors
///
/// Returns the error of the command, already written for an operator to
/// read: every command says what failed and what to do about it.
fn run(cli: Cli) -> Result<(), error::ToolError> {
    match cli.command {
        Command::Verify(args) => verify::command(&args),
        Command::Preflight(args) => preflight::command(&args),
        Command::Install(args) => install::command(&args),
        Command::Backup(args) => backup::command(&args),
        Command::Restore(args) => backup::command_restore(&args),
        Command::Update(args) => update::command(&args),
        Command::Rollback(args) => rollback::command(&args),
        Command::Status(args) => status::command(&args),
        Command::SelfUpdate => self_update::command(),
    }
}
