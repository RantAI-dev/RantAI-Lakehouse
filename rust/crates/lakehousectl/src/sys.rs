//! The machine the tool runs on: the commands it runs and the numbers
//! it reads.
//!
//! Everything here is a thin wrapper over the operating system. The
//! decisions live in the modules that call these functions — the
//! pre-flight checks (`preflight`), the installer (`install`) — so that
//! those can be tested with facts injected instead of a machine.

use std::path::Path;
use std::process::{Command, Output};

/// How many lines of a failed command's error output the error carries.
///
/// Twenty is enough for Docker to say what went wrong, and short enough
/// for a terminal to show (REL-2 S3 review round 4 SHOULD-FIX 2).
pub const STDERR_LINES: usize = 20;

/// Run a command and return its standard output.
///
/// The command's own error output is NOT returned: the caller gets
/// the exit status instead, and the message it builds says what to do
/// about it.
///
/// # Errors
///
/// Returns the error the operating system gave when the command could
/// not be run at all, or when it exited with a failure.
pub fn output(program: &str, args: &[&str]) -> Result<String, std::io::Error> {
    let bytes = output_bytes(program, args)?;
    Ok(String::from_utf8_lossy(&bytes).trim().to_string())
}

/// Run a command and return its standard output, byte for byte.
///
/// The bytes are NOT trimmed and NOT converted into text: the release
/// manifest's signature is over its exact bytes, so a trailing newline
/// is part of the answer and may not be dropped.
///
/// # Errors
///
/// Returns the error the operating system gave when the command could
/// not be run at all, or when it exited with a failure.
pub fn output_bytes(program: &str, args: &[&str]) -> Result<Vec<u8>, std::io::Error> {
    let result = capture(program, args)?;
    if !result.status.success() {
        return Err(failed(program, &result));
    }
    Ok(result.stdout)
}

/// Run a command and return its standard output, and its error output
/// when it fails.
///
/// The error message carries the LAST [`STDERR_LINES`] lines of what
/// the command said on its error stream. `docker compose up`, `docker
/// load` and `docker pull` say WHY they failed there, and a message
/// that names only the exit status leaves the operator with nothing to
/// act on (REL-2 S3 review round 4 SHOULD-FIX 2).
///
/// # Errors
///
/// Returns the error the operating system gave when the command could
/// not be run at all, or when it exited with a failure — with the
/// command's own error output in the message.
pub fn output_or_stderr(program: &str, args: &[&str]) -> Result<String, std::io::Error> {
    let result = capture(program, args)?;
    if !result.status.success() {
        return Err(failed_with_stderr(program, &result));
    }
    Ok(String::from_utf8_lossy(&result.stdout).trim().to_string())
}

/// Run a command and keep both of its streams.
fn capture(program: &str, args: &[&str]) -> Result<Output, std::io::Error> {
    Command::new(program).args(args).output()
}

/// The error of a command that failed: its exit status, and nothing else.
fn failed(program: &str, result: &Output) -> std::io::Error {
    std::io::Error::other(format!("{program} exited with {}", result.status))
}

/// The error of a command that failed: its exit status, and the last
/// lines of what it said on its error stream.
///
/// The error output is on its OWN lines, under the status: a reader
/// sees what the command said, and the status stays a status.
fn failed_with_stderr(program: &str, result: &Output) -> std::io::Error {
    let mut message = format!("{program} exited with {}", result.status);
    let lines = last_lines(&result.stderr, STDERR_LINES);
    if !lines.is_empty() {
        message.push('\n');
        message.push_str(&lines);
    }
    std::io::Error::other(message)
}

/// The last `count` lines of a command's error output.
fn last_lines(stderr: &[u8], count: usize) -> String {
    let text = String::from_utf8_lossy(stderr);
    let lines: Vec<&str> = text.trim_end().lines().collect();
    let start = lines.len().saturating_sub(count);
    lines[start..].join("\n")
}

/// The free space on the filesystem that holds `path`, in bytes.
///
/// The space counted is the space an unprivileged process may use
/// (`blocks_available`), not the space the filesystem has: the space
/// the root may use is not the space the install may use.
///
/// # Errors
///
/// Returns the error the operating system gave when the path could not
/// be read.
pub fn free_bytes(path: &Path) -> Result<u64, std::io::Error> {
    let stats = nix::sys::statvfs::statvfs(path).map_err(|error| {
        std::io::Error::other(format!("{} could not be read: {error}", path.display()))
    })?;
    Ok(stats.blocks_available() * stats.fragment_size())
}

/// Whether a TCP port on the host is already taken.
///
/// The check binds the port on every interface: a port Docker
/// publishes is held on `0.0.0.0`, and a port held on any address
/// keeps this one from being bound on any other.
///
/// # Errors
///
/// Returns the error the operating system gave when the port could not
/// be bound for a reason other than it being taken (no permission, for
/// instance) — that is not a "taken" answer.
pub fn port_is_busy(port: u16) -> Result<bool, std::io::Error> {
    match std::net::TcpListener::bind(("0.0.0.0", port)) {
        Ok(listener) => {
            // The bind succeeded, so the port was free; hold it for a
            // moment so nothing else takes it between the check and
            // the install.
            drop(listener);
            Ok(false)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => Ok(true),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn a_command_that_works_returns_its_output() {
        let output = output("echo", &["hello"]).expect("echo works");
        assert_eq!(output, "hello");
    }

    #[test]
    fn a_command_that_fails_is_an_error() {
        assert!(output("false", &[]).is_err());
    }

    #[test]
    fn the_error_output_of_a_failed_command_is_in_the_error() {
        // REL-2 S3 review round 4 SHOULD-FIX 2: the operator has to read
        // WHY the command failed, not only that it did.
        let error = output_or_stderr("sh", &["-c", "echo 'the reason' >&2; exit 3"])
            .expect_err("the command fails");
        let message = error.to_string();
        assert!(message.contains("the reason"), "the message is {message}");
        assert!(message.contains("exited with"), "the message is {message}");
    }

    #[test]
    fn the_error_output_of_a_failed_command_is_capped_at_twenty_lines() {
        // The cap is what keeps the message readable: a command that
        // says a hundred lines must not bury the exit status.
        let script =
            "i=1; while [ $i -le 40 ]; do echo \"line $i\" >&2; i=$((i + 1)); done; exit 1";
        let error = output_or_stderr("sh", &["-c", script]).expect_err("the command fails");
        let message = error.to_string();
        let lines: Vec<&str> = message
            .lines()
            .filter(|line| line.starts_with("line "))
            .collect();
        assert_eq!(lines.len(), STDERR_LINES, "the message is {message}");
        assert_eq!(lines[0], "line 21", "the message is {message}");
        assert_eq!(lines.last().copied(), Some("line 40"), "{message}");
    }

    #[test]
    fn a_command_that_says_nothing_on_its_error_stream_is_an_error() {
        // The message still says what happened: there is nothing to add.
        let error = output_or_stderr("false", &[]).expect_err("the command fails");
        let message = error.to_string();
        assert!(message.contains("exited with"), "the message is {message}");
        assert_eq!(message.lines().count(), 1, "the message is {message}");
    }

    #[test]
    fn a_command_that_does_not_exist_is_an_error() {
        assert!(output("lakehousectl-no-such-command", &[]).is_err());
    }

    #[test]
    fn the_bytes_of_a_command_are_returned_as_they_are() {
        // The manifest's signature is over its exact bytes: a trimmed
        // answer would not verify.
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("bytes");
        std::fs::write(&path, b"a\nb\n").expect("the file is written");
        let name = path.to_string_lossy().to_string();
        let bytes = output_bytes("cat", &[&name]).expect("cat works");
        assert_eq!(bytes, b"a\nb\n", "the bytes were changed");
    }

    #[test]
    fn the_free_space_of_a_directory_is_read() {
        let bytes = free_bytes(Path::new("/tmp")).expect("/tmp can be read");
        assert!(bytes > 0, "/tmp reported {bytes} free bytes");
    }

    #[test]
    fn a_port_nobody_holds_is_free() {
        // Port 0 asks the operating system for a free port, and the
        // answer is the port it gave: that one was free.
        let listener = std::net::TcpListener::bind(("0.0.0.0", 0)).expect("a port is free");
        let port = listener
            .local_addr()
            .expect("the listener has an address")
            .port();
        assert!(port_is_busy(port).expect("the port can be checked"));
        drop(listener);
    }
}
