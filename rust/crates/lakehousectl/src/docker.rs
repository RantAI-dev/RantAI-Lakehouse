//! Docker and Compose, as the installer calls them.
//!
//! The tool does not talk to the Docker socket: it runs the `docker`
//! command, which is what an operator runs, and whose output the
//! operator can read in their own terminal.

use std::path::Path;

use crate::sys;

/// The Compose project the install lives in (REL-2 decision D5).
pub const PROJECT: &str = "rantai-lakehouse";

/// Pull an image by its digest reference.
///
/// # Errors
///
/// Returns the message that says what to fix when the pull fails.
pub fn pull(reference: &str) -> Result<(), String> {
    sys::output("docker", &["pull", reference])
        .map(|_| ())
        .map_err(|error| {
            format!("the image {reference} could not be pulled: {error}. Is this host online?")
        })
}

/// Load an image from a `docker save` tar.
///
/// # Errors
///
/// Returns the message that says what to fix when the load fails.
pub fn load(tar: &Path) -> Result<(), String> {
    let path = tar.to_string_lossy().to_string();
    sys::output("docker", &["load", "-i", &path])
        .map(|_| ())
        .map_err(|error| {
            format!(
                "the image file {} could not be loaded: {error}",
                tar.display()
            )
        })
}

/// The ID Docker reports for an image.
///
/// # Errors
///
/// Returns the message that says what to fix when the image is not in
/// the store.
pub fn image_id(reference: &str) -> Result<String, String> {
    sys::output(
        "docker",
        &["image", "inspect", "--format", "{{.Id}}", reference],
    )
    .map_err(|error| format!("the image {reference} is not in the local store: {error}"))
}

/// Start the stack.
///
/// The Compose file is the release's own, in the install directory, and
/// the operator's `override.yml` when there is one (REL-2 decision
/// D5).
///
/// # Errors
///
/// Returns the message that says what to fix when Compose fails.
pub fn compose_up(dir: &Path, env_file: &Path) -> Result<(), String> {
    let args = compose_up_args(dir, env_file);
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    sys::output("docker", &borrowed)
        .map(|_| ())
        .map_err(|error| format!("the stack did not start: {error}. The output above says why."))
}

/// The `docker compose` arguments that start the stack.
///
/// The list is built apart from the call, so a test can read it. The
/// `--pull never` is the difference between a stack that starts from
/// the images the bundle LOADED and a stack that silently FETCHES them
/// from a registry — on an air-gapped host the fetch is either
/// impossible or, worse, a different image than the signed manifest
/// names (REL-2 S3 D13).
fn compose_up_args(dir: &Path, env_file: &Path) -> Vec<String> {
    let compose = dir.join("docker-compose.yml");
    let override_file = dir.join("override.yml");
    let mut args: Vec<String> = vec![
        "compose".to_string(),
        "-p".to_string(),
        PROJECT.to_string(),
        "--env-file".to_string(),
        env_file.to_string_lossy().to_string(),
        "-f".to_string(),
        compose.to_string_lossy().to_string(),
    ];
    if override_file.exists() {
        args.push("-f".to_string());
        args.push(override_file.to_string_lossy().to_string());
    }
    args.push("up".to_string());
    args.push("-d".to_string());
    args.push("--pull".to_string());
    args.push("never".to_string());
    args
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn the_stack_starts_without_pulling_anything() {
        // REL-2 S3 D13: Compose never pulls. A missing image is an
        // error, never a silent fetch.
        let directory = tempfile::tempdir().expect("a temporary directory");
        let env = directory.path().join(".env");
        let args = compose_up_args(directory.path(), &env);
        assert_eq!(
            args.last().map(String::as_str),
            Some("never"),
            "the last arguments are {args:?}"
        );
        let pull = args
            .iter()
            .position(|arg| arg == "--pull")
            .expect("--pull is there");
        assert_eq!(args.get(pull + 1).map(String::as_str), Some("never"));
    }

    #[test]
    fn the_stack_starts_in_the_projects_own_directory() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let env = directory.path().join(".env");
        let args = compose_up_args(directory.path(), &env);
        assert!(
            args.contains(&PROJECT.to_string()),
            "the arguments are {args:?}"
        );
        assert!(
            args.contains(&"up".to_string()),
            "the arguments are {args:?}"
        );
        assert!(
            args.contains(&"-d".to_string()),
            "the arguments are {args:?}"
        );
    }
}
