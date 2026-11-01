//! The files the release ships, as the install writes them (REL-2 S4-T1).
//!
//! # What a release ships
//!
//! The signed manifest names the checksum of every file the release
//! carries, under `files`. Four kinds of file are in there:
//!
//! - `docker-compose.yml` — the release's Compose file;
//! - `lakehousectl` — the installer;
//! - every RELATIVE bind-mount source of the release's Compose file.
//!   A relative mount (`./ops/backups/clickhouse/backups.xml`) is a
//!   file the deployment needs NEXT TO its Compose file: Docker creates
//!   an empty directory in its place when the file is missing, and the
//!   service that mounts it then runs against nothing;
//! - the two tools under `tools/` — `backup-local.sh` and
//!   `restore-from-backup.sh` (REL-2 S4 decision D15).
//!
//! The first two the install handles itself. This module fetches the
//! rest, checks every byte against the signed manifest, and writes each
//! file at the relative path the release ships it at — the same path
//! the Compose file mounts it at.
//!
//! # Where the bytes come from
//!
//! - OFFLINE, from the bundle: the bundle holds every shipped file at
//!   the same relative path.
//! - ONLINE, from the release's assets: one asset per file, named after
//!   the file itself. The release tooling refuses two shipped files
//!   with the same name, so an asset name names exactly one file.
//!
//! Either way the bytes are checked against the manifest's checksum
//! before they are written, and a file that does not match is a
//! REFUSAL: the install stops there, and nothing is written.

use std::collections::BTreeMap;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::ToolError;
use crate::manifest::Manifest;
use crate::release;

/// The Compose file's name in the manifest's `files`.
///
/// The install writes the Compose file itself, from the bytes the
/// release served, so it is not one of the files this module writes.
pub const COMPOSE_FILE: &str = "docker-compose.yml";

/// The installer binary's name in the manifest's `files`.
///
/// The installer is the program the operator is running; it is not
/// copied into the install's own directory.
pub const INSTALLER_FILE: &str = "lakehousectl";

/// The files the release ships, and the bytes of each.
///
/// The key is the path the file is written at, relative to the
/// install's own directory; the value is the bytes.
pub type ShippedFiles = BTreeMap<String, Vec<u8>>;

/// The files the manifest names that this module fetches and writes.
///
/// Every file but the Compose file and the installer: those two the
/// install handles itself.
#[must_use]
pub fn installable(manifest: &Manifest) -> Vec<(&str, &str)> {
    manifest
        .files
        .iter()
        .filter(|(name, _)| *name != COMPOSE_FILE && *name != INSTALLER_FILE)
        .map(|(name, sum)| (name.as_str(), sum.as_str()))
        .collect()
}

/// The bytes of every file the release ships, checked against the
/// signed manifest.
///
/// # Errors
///
/// Returns the error that stopped the install: a file the bundle does
/// not hold, an asset that could not be downloaded, or a file whose
/// bytes do not match the checksum the signed manifest names.
pub fn fetch<F>(
    manifest: &Manifest,
    bundle_dir: Option<&Path>,
    fetch_asset: &mut F,
) -> Result<ShippedFiles, ToolError>
where
    F: FnMut(&str) -> Result<Vec<u8>, ToolError>,
{
    let mut files: ShippedFiles = BTreeMap::new();
    for (name, expected) in installable(manifest) {
        let bytes = if let Some(directory) = bundle_dir {
            read(&directory.join(name))?
        } else {
            let asset = release::asset_url(&manifest.version, asset_name(name));
            fetch_asset(&asset)?
        };
        check(name, expected, &bytes)?;
        files.insert(name.to_string(), bytes);
    }
    Ok(files)
}

/// The name the release publishes a shipped file under.
///
/// The release publishes one asset per file, and the asset's name is
/// the file's own name. The tooling refuses two shipped files with the
/// same name, so the name names exactly one file.
#[must_use]
pub fn asset_name(install_path: &str) -> &str {
    install_path.rsplit('/').next().unwrap_or(install_path)
}

/// Check one file against the checksum the signed manifest names.
///
/// # Errors
///
/// Returns the error that says which file does not match, and what the
/// two checksums are.
pub fn check(name: &str, expected: &str, bytes: &[u8]) -> Result<(), ToolError> {
    let found = hex::encode(Sha256::digest(bytes));
    if found != expected {
        return Err(ToolError::Failed(format!(
            "the file {name} does not match the manifest's checksum: the manifest names \
             {expected}, this one hashes to {found}."
        )));
    }
    Ok(())
}

/// Write the shipped files into the install's own directory.
///
/// Each file is written at the relative path the release ships it at:
/// the path the Compose file mounts it at, or the path under `tools/`
/// the tools live at.
///
/// # Errors
///
/// Returns the error that stopped the writing: a directory that could
/// not be created, or a file that could not be written.
pub fn write(dir: &Path, files: &ShippedFiles) -> Result<(), ToolError> {
    for (name, bytes) in files {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ToolError::Io {
                context: format!("{} could not be created", parent.display()),
                source,
            })?;
        }
        std::fs::write(&path, bytes).map_err(|source| ToolError::Io {
            context: format!("{} could not be written", path.display()),
            source,
        })?;
        // A script the install RUNS has to be executable; a file the
        // Compose file mounts only has to be there.
        if Path::new(name)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("sh"))
        {
            set_executable(&path)?;
        }
    }
    Ok(())
}

/// Give a file the mode a script is run with.
fn set_executable(path: &Path) -> Result<(), ToolError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(|source| {
        ToolError::Io {
            context: format!("{} could not be given the mode 755", path.display()),
            source,
        }
    })
}

/// Read a file.
fn read(path: &Path) -> Result<Vec<u8>, ToolError> {
    std::fs::read(path).map_err(|source| ToolError::Io {
        context: format!("{} could not be read", path.display()),
        source,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// A manifest that names three files: the Compose file, the
    /// installer and one shipped file.
    fn manifest_with_a_shipped_file() -> Manifest {
        Manifest {
            version: "0.2.0".to_string(),
            images: BTreeMap::new(),
            image_ids: BTreeMap::new(),
            files: BTreeMap::from([
                (COMPOSE_FILE.to_string(), "compose".to_string()),
                (INSTALLER_FILE.to_string(), "installer".to_string()),
                (
                    "ops/backups/clickhouse/backups.xml".to_string(),
                    "the checksum".to_string(),
                ),
            ]),
        }
    }

    #[test]
    fn the_compose_file_and_the_installer_are_not_files_the_install_writes() {
        let manifest = manifest_with_a_shipped_file();
        let names: Vec<&str> = installable(&manifest)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(names, vec!["ops/backups/clickhouse/backups.xml"]);
    }

    #[test]
    fn a_shipped_file_is_written_at_the_relative_path_the_release_ships_it_at() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let mut files: ShippedFiles = BTreeMap::new();
        files.insert(
            "ops/backups/clickhouse/backups.xml".to_string(),
            b"<clickhouse/>".to_vec(),
        );
        files.insert("tools/backup-local.sh".to_string(), b"#!/bin/sh\n".to_vec());

        write(directory.path(), &files).expect("the files are written");

        assert_eq!(
            std::fs::read(directory.path().join("ops/backups/clickhouse/backups.xml"))
                .expect("the mounted file is readable"),
            b"<clickhouse/>".to_vec(),
            "the mounted file was not written at its own relative path"
        );
        assert!(
            directory.path().join("tools/backup-local.sh").exists(),
            "the tool was not written under tools/"
        );
    }

    #[test]
    fn a_tool_is_written_executable() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let mut files: ShippedFiles = BTreeMap::new();
        files.insert("tools/backup-local.sh".to_string(), b"#!/bin/sh\n".to_vec());

        write(directory.path(), &files).expect("the files are written");

        let mode = std::fs::metadata(directory.path().join("tools/backup-local.sh"))
            .expect("the tool is there")
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "the tool is not executable: {mode:o}");
    }

    #[test]
    fn a_file_that_does_not_match_the_manifest_is_refused() {
        let error = check("tools/backup-local.sh", "the checksum", b"other bytes")
            .expect_err("a file that does not match is refused");
        let message = error.to_string();
        assert!(
            message.contains("tools/backup-local.sh"),
            "the error names the file: {message}"
        );
        assert!(
            message.contains("checksum"),
            "the error says what failed: {message}"
        );
    }

    #[test]
    fn the_bytes_of_a_shipped_file_are_read_from_the_bundle() {
        let bundle = tempfile::tempdir().expect("a temporary directory");
        let path = bundle.path().join("ops/backups/clickhouse/backups.xml");
        std::fs::create_dir_all(path.parent().expect("the parent")).expect("the directory");
        std::fs::write(&path, b"<clickhouse/>").expect("the file is written");
        let digest = hex::encode(Sha256::digest(b"<clickhouse/>"));

        let manifest = Manifest {
            version: "0.2.0".to_string(),
            images: BTreeMap::new(),
            image_ids: BTreeMap::new(),
            files: BTreeMap::from([("ops/backups/clickhouse/backups.xml".to_string(), digest)]),
        };

        let files = fetch(&manifest, Some(bundle.path()), &mut |_url| {
            panic!("a bundle install downloads nothing")
        })
        .expect("the file is read from the bundle");

        assert_eq!(
            files.get("ops/backups/clickhouse/backups.xml"),
            Some(&b"<clickhouse/>".to_vec()),
            "the file was not read from the bundle"
        );
    }

    #[test]
    fn an_online_install_downloads_every_shipped_file_by_its_own_name() {
        let digest = hex::encode(Sha256::digest(b"the bytes"));
        let manifest = Manifest {
            version: "0.2.0".to_string(),
            images: BTreeMap::new(),
            image_ids: BTreeMap::new(),
            files: BTreeMap::from([
                (
                    "ops/backups/clickhouse/backups.xml".to_string(),
                    digest.clone(),
                ),
                ("tools/backup-local.sh".to_string(), digest),
            ]),
        };
        let mut asked: Vec<String> = Vec::new();

        let files = fetch(&manifest, None, &mut |url| {
            asked.push(url.to_string());
            Ok(b"the bytes".to_vec())
        })
        .expect("the files are downloaded");

        assert_eq!(asked.len(), 2, "the files were not downloaded: {asked:?}");
        assert!(
            asked
                .iter()
                .all(|url| url.ends_with("/v0.2.0/") || url.contains("/v0.2.0/")),
            "the assets are not the release's: {asked:?}"
        );
        assert!(
            asked.iter().any(|url| url.ends_with("/backups.xml")),
            "the mounted file was not asked for: {asked:?}"
        );
        assert!(
            asked.iter().any(|url| url.ends_with("/backup-local.sh")),
            "the tool was not asked for: {asked:?}"
        );
        assert_eq!(files.len(), 2, "the files were not downloaded: {files:?}");
    }

    #[test]
    fn a_downloaded_file_that_does_not_match_the_manifest_is_refused() {
        let manifest = Manifest {
            version: "0.2.0".to_string(),
            images: BTreeMap::new(),
            image_ids: BTreeMap::new(),
            files: BTreeMap::from([(
                "tools/backup-local.sh".to_string(),
                "the checksum".to_string(),
            )]),
        };

        let error = fetch(&manifest, None, &mut |_url| Ok(b"other bytes".to_vec()))
            .expect_err("a file that does not match is refused");
        assert!(
            error.to_string().contains("tools/backup-local.sh"),
            "the error names the file: {error}"
        );
    }

    #[test]
    fn a_file_the_bundle_does_not_hold_is_refused() {
        let bundle = tempfile::tempdir().expect("a temporary directory");
        let manifest = Manifest {
            version: "0.2.0".to_string(),
            images: BTreeMap::new(),
            image_ids: BTreeMap::new(),
            files: BTreeMap::from([(
                "tools/backup-local.sh".to_string(),
                "the checksum".to_string(),
            )]),
        };

        let error = fetch(&manifest, Some(bundle.path()), &mut |_url| {
            panic!("a bundle install downloads nothing")
        })
        .expect_err("a file the bundle does not hold is refused");
        assert!(
            error.to_string().contains("could not be read"),
            "the error says what failed: {error}"
        );
    }

    #[test]
    fn the_asset_name_of_a_shipped_file_is_its_own_name() {
        assert_eq!(
            asset_name("ops/backups/clickhouse/backups.xml"),
            "backups.xml"
        );
        assert_eq!(asset_name("tools/backup-local.sh"), "backup-local.sh");
    }
}
