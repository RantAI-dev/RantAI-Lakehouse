//! Docker and Compose, as the installer calls them.
//!
//! The tool does not talk to the Docker socket: it runs the `docker`
//! command, which is what an operator runs, and whose output the
//! operator can read in their own terminal.
//!
//! The commands here run with `sys::output_or_stderr`, so the error
//! the operator reads carries what Docker said on its error stream
//! (REL-2 S3 review round 4 SHOULD-FIX 2). That output carries NO
//! SECRET: the arguments name images, image files and the Compose
//! file, and Docker's own error output names the same things back. The
//! signing key and the secrets of the stack are never in a `docker`
//! command's arguments, and Docker never prints them.

use std::path::Path;

use crate::sys;

/// The Compose project the install lives in (REL-2 decision D5).
pub const PROJECT: &str = "rantai-lakehouse";

/// Pull an image by its digest reference.
///
/// # Errors
///
/// Returns the message that says what to fix when the pull fails, with
/// what Docker said about it.
pub fn pull(reference: &str) -> Result<(), String> {
    sys::output_or_stderr("docker", &["pull", reference])
        .map(|_| ())
        .map_err(|error| {
            format!("the image {reference} could not be pulled. Is this host online? {error}")
        })
}

/// Load an image from a `docker save` tar.
///
/// # Errors
///
/// Returns the message that says what to fix when the load fails, with
/// what Docker said about it.
pub fn load(tar: &Path) -> Result<(), String> {
    let path = tar.to_string_lossy().to_string();
    sys::output_or_stderr("docker", &["load", "-i", &path])
        .map(|_| ())
        .map_err(|error| {
            format!(
                "the image file {} could not be loaded. {error}",
                tar.display()
            )
        })
}

/// The ID Docker reports for an image.
///
/// # Errors
///
/// Returns the message that says what to fix when the image is not in
/// the store, with what Docker said about it.
pub fn image_id(reference: &str) -> Result<String, String> {
    sys::output_or_stderr(
        "docker",
        &["image", "inspect", "--format", "{{.Id}}", reference],
    )
    .map_err(|error| format!("the image {reference} is not in the local store. {error}"))
}

/// Start the stack.
///
/// The Compose file is the release's own, in the install directory, and
/// the operator's `override.yml` when there is one (REL-2 decision
/// D5).
///
/// # Errors
///
/// Returns the message that says what to fix when Compose fails, with
/// what Compose and Docker said about it: the error is the whole
/// answer, so the message does not point at an output that is not
/// printed (REL-2 S3 review round 4 SHOULD-FIX 2).
pub fn compose_up(dir: &Path, env_file: &Path) -> Result<(), String> {
    let args = compose_up_args(dir, env_file);
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    sys::output_or_stderr("docker", &borrowed)
        .map(|_| ())
        .map_err(|error| stack_error(&error))
}

/// The message of a failed stack start.
///
/// The error carries Docker's own error output, so the message is the
/// whole answer: it must not point at an output that nothing prints
/// (REL-2 S3 review round 4 SHOULD-FIX 2).
fn stack_error(error: &std::io::Error) -> String {
    format!("the stack did not start: {error}")
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
    let mut args = compose_args(dir, env_file);
    args.push("up".to_string());
    args.push("-d".to_string());
    args.push("--pull".to_string());
    args.push("never".to_string());
    args
}

/// The `docker compose` arguments that read this install.
///
/// Every `docker compose` the tool runs starts here: the project, the
/// `.env` and the Compose file of the install's own directory, and the
/// operator's `override.yml` when there is one (REL-2 decision D5).
/// One place builds them, so the update, the rollback and the status
/// cannot disagree about which deployment they act on (rule 4).
#[must_use]
pub fn compose_args(dir: &Path, env_file: &Path) -> Vec<String> {
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
    args
}

/// The services the install's Compose file names.
///
/// Every profile is asked for: the writers of the `dagster` profile are
/// stopped for an update (REL-2 S4 decision D17), and a profile that
/// is not asked for names no service at all.
///
/// # Errors
///
/// Returns the message that says what to fix when Compose fails.
pub fn compose_services(dir: &Path, env_file: &Path) -> Result<Vec<String>, String> {
    let mut args = compose_args(dir, env_file);
    args.push("--profile".to_string());
    args.push("*".to_string());
    args.push("config".to_string());
    args.push("--services".to_string());
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = sys::output_or_stderr("docker", &borrowed)
        .map_err(|error| format!("the services of the deployment could not be read: {error}"))?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

/// Stop the named services of the deployment.
///
/// An empty list stops nothing: the caller asks for the services it
/// wants stopped, and a deployment with no writers is a deployment
/// with nothing to stop.
///
/// # Errors
///
/// Returns the message that says what to fix when Compose fails.
pub fn compose_stop(dir: &Path, env_file: &Path, services: &[String]) -> Result<(), String> {
    if services.is_empty() {
        return Ok(());
    }
    let mut args = compose_args(dir, env_file);
    args.push("stop".to_string());
    for service in services {
        args.push(service.clone());
    }
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    sys::output_or_stderr("docker", &borrowed)
        .map(|_| ())
        .map_err(|error| format!("the services could not be stopped: {error}"))
}

/// Stop EVERY service of the deployment.
///
/// The rollback stops the whole stack before it restores the backup
/// (REL-2 S4 decision D18): a database that is still running while its
/// data is being written back is not a database that is restored.
///
/// # Errors
///
/// Returns the message that says what to fix when Compose fails.
pub fn compose_stop_all(dir: &Path, env_file: &Path) -> Result<(), String> {
    let mut args = compose_args(dir, env_file);
    args.push("stop".to_string());
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    sys::output_or_stderr("docker", &borrowed)
        .map(|_| ())
        .map_err(|error| format!("the stack could not be stopped: {error}"))
}

/// The state of the deployment's services.
///
/// The output is Compose's own JSON, one object per service. The
/// caller reads it (REL-2 S4 decision D19).
///
/// # Errors
///
/// Returns the message that says what to fix when Compose fails.
pub fn compose_ps(dir: &Path, env_file: &Path) -> Result<String, String> {
    let mut args = compose_args(dir, env_file);
    args.push("ps".to_string());
    args.push("--format".to_string());
    args.push("json".to_string());
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    sys::output_or_stderr("docker", &borrowed)
        .map_err(|error| format!("the state of the stack could not be read: {error}"))
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

    #[test]
    fn the_stack_error_is_the_whole_answer() {
        // REL-2 S3 review round 4 SHOULD-FIX 2: the error carries what
        // Docker said, so the message must not point at an output above
        // that nothing prints.
        let error = std::io::Error::other("docker exited with exit status: 1\nthe reason");
        let message = stack_error(&error);
        assert!(message.contains("the stack did not start:"), "{message}");
        assert!(message.contains("the reason"), "{message}");
        assert!(
            !message.contains("output above"),
            "the message points at an output that is not printed: {message}"
        );
    }
}
