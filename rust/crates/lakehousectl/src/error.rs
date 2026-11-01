//! The errors `lakehousectl` reports.
//!
//! Every error an operator sees says what failed and, where the tool
//! knows it, what to do about it. Nothing else is printed: the tool
//! does not log, and it never prints a machine's paths or a secret.

use thiserror::Error;

use crate::verify::VerifyError;

/// Anything that stops the tool.
#[derive(Debug, Error)]
pub enum ToolError {
    /// A file could not be read or written.
    #[error("{context}: {source}")]
    Io {
        /// What was being read or written.
        context: String,
        /// The failure itself.
        #[source]
        source: std::io::Error,
    },
    /// The release did not verify against the release key.
    #[error("{0}")]
    Verify(#[from] VerifyError),
}
