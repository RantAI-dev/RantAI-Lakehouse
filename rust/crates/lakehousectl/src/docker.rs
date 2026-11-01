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
    let compose = dir.join("docker-compose.yml");
    let override_file = dir.join("override.yml");
    let project = PROJECT;
    let env = env_file.to_string_lossy().to_string();
    let compose_path = compose.to_string_lossy().to_string();
    let override_path = override_file.to_string_lossy().to_string();
    let mut args: Vec<&str> = vec!["compose", "-p", project, "--env-file"];
    args.push(&env);
    args.push("-f");
    args.push(&compose_path);
    if override_file.exists() {
        args.push("-f");
        args.push(&override_path);
    }
    args.push("up");
    args.push("-d");
    sys::output("docker", &args)
        .map(|_| ())
        .map_err(|error| format!("the stack did not start: {error}. The output above says why."))
}
