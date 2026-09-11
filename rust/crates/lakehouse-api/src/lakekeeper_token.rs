//! Shared helper for reading a pre-minted Lakekeeper bearer token off disk.
//!
//! ADR 0011: every Lakekeeper principal in this stack authenticates with a
//! static bearer token, minted once at `ops/oidc-mock` boot and written to
//! a shared volume every writer mounts read-only as a single-file subpath
//! (never the whole volume — see `docker-compose.yml`'s per-service mount
//! comments). `routes::gold::read_catalog_token` used to be the only
//! reader of such a file; WS2's `lakehouse-api-reader` principal (task A0)
//! needs the exact same shape for a second, unrelated token file, so this
//! module generalizes the read instead of copying it (AGENTS.md rule 4).
//!
//! # No token refresh
//!
//! Consistent with ADR 0011's "Corrected claim" section: nothing in this
//! build re-mints a token before its 30-day expiry. This helper re-reads
//! the file on every call, so a re-minted file (an operator restarting
//! `ops/oidc-mock` onto a fresh volume, or hand-rotating the file) is
//! picked up on the caller's next (re)connect — but nothing here, or
//! anywhere else in this codebase, refreshes the token proactively. A
//! token that expires mid-run surfaces as an ordinary catalog auth
//! failure at the next request, not as a distinct error from this helper.

use lakehouse_core::ApiError;
use lakehouse_core::secret::SecretValue;

/// Read and trim a pre-minted Lakekeeper bearer token from `path`.
///
/// `purpose` and `env_var` name the caller's use case (for example
/// `"gold-export"` / `"LAKEKEEPER_GOLD_EXPORT_TOKEN_FILE"`) purely for the
/// error message and the log line below — this function does not
/// interpret them.
///
/// # Errors
///
/// Returns [`ApiError::Unavailable`] if the file cannot be read (not
/// provisioned on this deployment, wrong path, permissions, ...). The io
/// error and the path are logged via `tracing::warn!` for an operator to
/// see; the response body carries neither, only fixed text naming
/// `purpose` and `env_var`, so an upstream filesystem error never reaches
/// a caller (AGENTS.md rule 4: upstream error text never reaches a
/// response).
pub async fn read_token_file(
    path: &str,
    purpose: &'static str,
    env_var: &'static str,
) -> Result<SecretValue, ApiError> {
    let raw = tokio::fs::read_to_string(path).await.map_err(|err| {
        tracing::warn!(
            %err,
            path,
            purpose,
            env_var,
            "failed to read Lakekeeper bearer token file"
        );
        ApiError::Unavailable(format!(
            "Lakekeeper {purpose} token is unavailable (check {env_var}; see ADR 0011)"
        ))
    })?;
    Ok(SecretValue::new(raw.trim().to_owned()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::time::{SystemTime, UNIX_EPOCH};

    use super::read_token_file;

    /// A unique path under the OS temp dir, never `std::env::set_var`
    /// (AGENTS.md/tests must not mutate process-global env state).
    fn temp_path(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("lakekeeper-token-test-{name}-{nanos}"))
    }

    #[tokio::test]
    async fn missing_file_returns_503_naming_only_purpose_and_env_var() {
        let path = temp_path("missing");
        let path_str = path.to_str().expect("temp path is valid UTF-8").to_owned();

        let err = read_token_file(
            &path_str,
            "gold-export",
            "LAKEKEEPER_GOLD_EXPORT_TOKEN_FILE",
        )
        .await
        .expect_err("a nonexistent file must fail");

        let body = err.to_string();
        assert!(body.contains("gold-export"));
        assert!(body.contains("LAKEKEEPER_GOLD_EXPORT_TOKEN_FILE"));
        assert!(
            !body.contains(&path_str),
            "response text leaked the token file path: {body}"
        );
        assert!(
            !body.to_lowercase().contains("no such file"),
            "response text leaked raw io error text: {body}"
        );
    }

    #[tokio::test]
    async fn present_file_is_read_and_trimmed() {
        let path = temp_path("present");
        tokio::fs::write(&path, "  a-token-value\n")
            .await
            .expect("write temp token file");

        let secret = read_token_file(
            path.to_str().expect("temp path is valid UTF-8"),
            "lakehouse-api-reader",
            "LAKEKEEPER_READ_TOKEN_FILE",
        )
        .await
        .expect("present file must be read");

        assert_eq!(secret.expose_secret(), "a-token-value");

        tokio::fs::remove_file(&path)
            .await
            .expect("cleanup temp file");
    }
}
