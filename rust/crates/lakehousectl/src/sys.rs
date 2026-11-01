//! The machine the tool runs on: the commands it runs and the numbers
//! it reads.
//!
//! Everything here is a thin wrapper over the operating system. The
//! decisions live in the modules that call these functions — the
//! pre-flight checks (`preflight`), the installer (`install`) — so that
//! those can be tested with facts injected instead of a machine.

use std::path::Path;
use std::process::Command;

/// Run a command and return its standard output.
///
/// The command's own error output is NOT returned: it goes to the
/// operator's terminal, where it belongs, and the caller gets the
/// exit status instead.
///
/// # Errors
///
/// Returns the error the operating system gave when the command could
/// not be run at all, or when it exited with a failure.
pub fn output(program: &str, args: &[&str]) -> Result<String, std::io::Error> {
    let result = Command::new(program).args(args).output()?;
    if !result.status.success() {
        return Err(std::io::Error::other(format!(
            "{program} exited with {}",
            result.status
        )));
    }
    Ok(String::from_utf8_lossy(&result.stdout).trim().to_string())
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
    fn a_command_that_does_not_exist_is_an_error() {
        assert!(output("lakehousectl-no-such-command", &[]).is_err());
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
