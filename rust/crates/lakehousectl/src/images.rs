//! The images of a release, and how they reach the host (REL-2 S3-T3).
//!
//! # Two paths, one check
//!
//! - ONLINE, the images are PULLED by their digest — the reference the
//!   signed manifest names;
//! - OFFLINE, they are LOADED from the bundle's `docker save` archives.
//!   The file is hashed BEFORE it is loaded: a copy that was damaged
//!   on its way to the host fails there, with a message that says so,
//!   and not as a `docker load` that fails halfway (REL-2 S3 review
//!   round 1 SHOULD-FIX 2).
//!
//! Either way the LOADED image is checked against the signed manifest:
//! the manifest names every image's ID, and an image that is not the
//! one the release published stops the install (REL-2 S3 T0).
//!
//! The install and the update both fetch the images this way, so the
//! rule is in ONE place (rule 4).

use std::path::Path;

use sha2::{Digest, Sha256};

use crate::docker;
use crate::error::ToolError;
use crate::manifest::{Manifest, image_name};

/// How much of a file is hashed at a time.
const HASH_BUFFER_BYTES: usize = 64 * 1024;

/// Fetch every image the release runs.
///
/// # Errors
///
/// Returns the error that says which image could not be fetched, and
/// what to do about it.
pub fn fetch(manifest: &Manifest, bundle_dir: Option<&Path>) -> Result<(), ToolError> {
    let Some(directory) = bundle_dir else {
        for reference in manifest.references() {
            println!("lakehousectl: pulling {reference}");
            docker::pull(&reference).map_err(ToolError::Failed)?;
        }
        return Ok(());
    };

    let checksums = read_to_string(&directory.join("SHA256SUMS"))?;
    for reference in manifest.references() {
        let tar = directory
            .join("images")
            .join(format!("{}.tar", image_name(&reference)));
        verify_checksum(&checksums, &tar, directory)?;
        println!("lakehousectl: loading {}", tar.display());
        docker::load(&tar).map_err(ToolError::Failed)?;

        let Some(expected) = manifest.image_ids.get(&reference) else {
            return Err(ToolError::Failed(format!(
                "the manifest names no id for {reference}, so the loaded image cannot be checked \
                 against it."
            )));
        };
        let found = docker::image_id(&reference).map_err(ToolError::Failed)?;
        // Docker's two image stores identify an image differently: the
        // classic store reports the CONFIG digest, the containerd store
        // the image INDEX digest. Both are named in the SIGNED
        // manifest — the config digest in `imageIds`, the registry
        // digest in `images` — so the loaded image is accepted when its
        // ID is one of them, and refused when it is neither.
        let registry_digest = Manifest::digest_of(&reference).unwrap_or_default();
        if found != *expected && found != registry_digest {
            return Err(ToolError::Failed(format!(
                "the image {reference} has the id {found}, and the manifest names {expected} and \
                 {registry_digest}. The image is not the one the release published, so the \
                 install stops here."
            )));
        }
    }
    Ok(())
}

/// Check one file of the bundle against the bundle's checksums.
///
/// The bundle's `SHA256SUMS` is NOT signed: it catches a file that was
/// damaged or truncated on its way to the host, and nothing else. The
/// TRUST check is the image's ID, compared with the SIGNED manifest's
/// `imageIds` after the load (REL-2 S3 review round 1 SHOULD-FIX 2).
///
/// # Errors
///
/// Returns the error that says which file does not match.
pub fn verify_checksum(checksums: &str, path: &Path, root: &Path) -> Result<(), ToolError> {
    let Some(expected) = checksum_of(checksums, path, root) else {
        return Err(ToolError::Failed(format!(
            "{} is not in the bundle's SHA256SUMS, so nothing says it is the file the release \
             published.",
            path.display()
        )));
    };
    let found = sha256_of(path)?;
    if found != expected {
        return Err(ToolError::Failed(format!(
            "{} does not match the bundle's SHA256SUMS: the list names {expected}, this one hashes \
             to {found}. The file was damaged on its way here.",
            path.display()
        )));
    }
    Ok(())
}

/// The checksum the bundle's list names for a file.
///
/// The list holds one line per file: the checksum, then the name, with
/// the path relative to the bundle's root.
fn checksum_of(checksums: &str, path: &Path, root: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let relative = relative.to_string_lossy();
    checksums.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let digest = fields.next()?;
        let name = fields.next()?;
        (name.trim_start_matches('*') == relative).then(|| digest.to_string())
    })
}

/// The SHA-256 of a file, as hexadecimal.
///
/// The file is read in pieces: an image tar is hundreds of megabytes,
/// and the installer has no reason to hold one in memory.
///
/// # Errors
///
/// Returns the error that says what could not be read.
pub fn sha256_of(path: &Path) -> Result<String, ToolError> {
    let file = std::fs::File::open(path).map_err(|source| ToolError::Io {
        context: format!("{} could not be read", path.display()),
        source,
    })?;
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; HASH_BUFFER_BYTES];
    loop {
        let read =
            std::io::Read::read(&mut reader, &mut buffer).map_err(|source| ToolError::Io {
                context: format!("{} could not be read", path.display()),
                source,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Read a file as text.
fn read_to_string(path: &Path) -> Result<String, ToolError> {
    std::fs::read_to_string(path).map_err(|source| ToolError::Io {
        context: format!("{} could not be read", path.display()),
        source,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::path::PathBuf;

    use super::*;

    /// A bundle with one image tar and the list of checksums.
    fn bundle_with_one_image() -> (tempfile::TempDir, PathBuf, String) {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let images = directory.path().join("images");
        std::fs::create_dir_all(&images).expect("the images directory is created");
        let tar = images.join("alpine.tar");
        std::fs::write(&tar, b"the image").expect("the tar is written");
        let digest = hex::encode(Sha256::digest(b"the image"));
        let checksums = format!("{digest}  images/alpine.tar\n");
        (directory, tar, checksums)
    }

    #[test]
    fn a_bundle_file_that_matches_its_checksum_is_accepted() {
        let (directory, tar, checksums) = bundle_with_one_image();
        verify_checksum(&checksums, &tar, directory.path()).expect("the file matches its checksum");
    }

    #[test]
    fn a_damaged_bundle_file_is_refused() {
        // The copy was damaged on its way to the host: the checksum
        // says so before `docker load` reads a broken tar.
        let (directory, tar, checksums) = bundle_with_one_image();
        std::fs::write(&tar, b"the imag3").expect("the tar is written again");

        let error = verify_checksum(&checksums, &tar, directory.path())
            .expect_err("a damaged file is refused");
        let message = error.to_string();
        assert!(
            message.contains("SHA256SUMS"),
            "the error says what failed: {message}"
        );
        assert!(
            message.contains("images/alpine.tar"),
            "the error names the file: {message}"
        );
    }

    #[test]
    fn a_bundle_file_the_list_does_not_name_is_refused() {
        let (directory, _tar, _checksums) = bundle_with_one_image();
        let other = directory.path().join("images").join("other.tar");
        std::fs::write(&other, b"another image").expect("the tar is written");

        let error = verify_checksum("", &other, directory.path())
            .expect_err("a file the list does not name is refused");
        assert!(
            error.to_string().contains("SHA256SUMS"),
            "the error says what failed: {error}"
        );
    }
}
