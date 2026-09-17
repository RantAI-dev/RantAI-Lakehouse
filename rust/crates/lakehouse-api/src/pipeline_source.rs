//! Path-safe reader for `GET /api/pipelines/{id}/source?op=` — serves op
//! source text from the `Dagster` code location tree baked into this image
//! at `/opt/pipeline-src` (`rust/Dockerfile`, WS4 item C2). `sourceRef`
//! comes from `@op` metadata (`Dagster`-side, WS4 item B2), NOT from a
//! trusted constant — a malicious or buggy code location could in
//! principle report `source_ref: "../../etc/passwd::x"`, so this module
//! trusts nothing about the string's shape and resolves everything against
//! a fixed, pre-built allowlist instead of the raw path.
//!
//! # Design
//!
//! 1. At first request (cached afterwards, in `AppState`, the same pattern
//!    `EmbedSecretResolver` uses for its own resolved secret),
//!    [`build_allowlist`] walks the base directory ONCE, canonicalizing
//!    every `.py` file it finds and keeping only the ones that
//!    canonicalize to a path still inside the canonicalized base —
//!    rejecting a symlink escape at BUILD time, not at read time.
//! 2. A request's `op=<file>::<fn>` is split; the `<file>` half is looked
//!    up in the allowlist BY EXACT KEY (a `HashMap`, not a filesystem
//!    call) — a `..`/absolute-path/unlisted string simply never matches a
//!    key and is rejected before touching disk.
//! 3. The matched file's already-canonicalized path is read; the `<fn>`
//!    half is located with a simple `^(async )?def <fn>\(` scan (Python
//!    source, not executed or parsed as an AST — a textual `def` boundary
//!    is sufficient for a read-only viewer) and the slice from that line
//!    to the next top-level `def`/`@`/EOF is returned.
//! 4. `commit` is compared against `GIT_SHA` (this image's own build arg —
//!    see [`check_commit`]) — a mismatch means the API and `Dagster`
//!    images were built from different commits, so serving "the current
//!    source" would fabricate which code actually ran; the route returns
//!    409 instead.
//!
//! `walkdir_py_files` is a small hand-rolled recursive directory walk (the
//! workspace has no `walkdir` crate dependency — `grep -n "walkdir"
//! rust/Cargo.toml rust/Cargo.lock` returns nothing; the base directory
//! here is small — five files today — and walked once per process, so a
//! hand-rolled `std::fs::read_dir` recursion is proportionate).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Errors from [`build_allowlist`]/[`read_source`]/[`check_commit`].
///
/// Every user-facing variant renders the SAME fixed string
/// (`"source not available for this build"`) — the caller-visible `Display`
/// deliberately carries no detail (not even which file/function was
/// requested), so a probing caller learns nothing about the tree's real
/// shape from the error text alone; the `String`/struct payloads exist for
/// server-side `tracing` only.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    /// The `file` half of `op=<file>::<fn>` is not an allowlist key —
    /// covers `..`, an absolute path, a symlink escape, a non-`.py`
    /// extension, and a file that is simply not part of this code
    /// location, all identically: none of them are in the map.
    #[error("source not available for this build")]
    NotAllowlisted(String),
    /// The `file` half resolved, but no `def`/`async def <fn>(` boundary
    /// was found in it.
    #[error("source not available for this build")]
    FunctionNotFound(String),
    /// The op's own `commit` metadata differs from this image's `GIT_SHA` —
    /// a real, comparable mismatch between two known commits.
    #[error("source not available for this build")]
    CommitMismatch {
        /// This image's own `GIT_SHA`.
        expected: String,
        /// The op's declared `commit` metadata.
        actual: String,
    },
    /// Either side's commit is the build-time placeholder `"unknown"`
    /// (both Dockerfiles' `ARG GIT_SHA=unknown` default, taken whenever
    /// nobody exported a real `GIT_SHA`) or an empty string — in either
    /// case there is no real commit to compare, so an "equal" comparison
    /// would be comparing two absences, not confirming provenance. Kept as
    /// a DISTINCT variant from [`Self::CommitMismatch`] so a log line (and
    /// this module's own tests) can tell "two different real commits"
    /// apart from "no real commit was ever recorded" — the second is a
    /// build/deploy misconfiguration, not a code drift.
    #[error("source provenance is unavailable for this build")]
    UnverifiableProvenance,
    /// The base directory could not be canonicalized/read, or an
    /// allowlisted (already-canonicalized) path failed to read.
    #[error("source not available for this build")]
    Io,
}

/// `"dispar_orchestrate/<relative path>.py"` → the file's already-
/// canonicalized, on-disk path. Built once by [`build_allowlist`].
pub type SourceAllowlist = HashMap<String, PathBuf>;

/// Walk `base` once, admitting only regular `.py` files whose
/// canonicalized path stays inside `base`'s own canonicalization — a
/// symlink pointing outside `base` is silently excluded, not "allowed then
/// rejected later" (closing the class of bug where a later refactor
/// forgets the re-check).
///
/// # Errors
///
/// Returns [`SourceError::Io`] if `base` itself cannot be canonicalized.
pub fn build_allowlist(base: &Path) -> Result<SourceAllowlist, SourceError> {
    let base_real = base.canonicalize().map_err(|_| SourceError::Io)?;
    let mut map = HashMap::new();
    for entry in walkdir_py_files(&base_real) {
        let Ok(real) = entry.canonicalize() else {
            continue;
        };
        if !real.starts_with(&base_real) {
            continue; // symlink (or a raw entry escaping the base)
        }
        if real.extension().and_then(|e| e.to_str()) != Some("py") {
            continue;
        }
        let Ok(relative_path) = real.strip_prefix(&base_real) else {
            continue;
        };
        let key = format!(
            "dispar_orchestrate/{}",
            relative_path.to_string_lossy().replace('\\', "/")
        );
        map.insert(key, real);
    }
    Ok(map)
}

/// Recursively list every regular file under `dir` (following symlinks —
/// [`build_allowlist`] is what rejects a symlink escape, by canonicalizing
/// and range-checking each result, not this walk itself). A directory that
/// cannot be read is simply skipped (no entries from it), never a hard
/// failure — [`build_allowlist`]'s only hard failure is `base` itself being
/// unreadable.
fn walkdir_py_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            out.extend(walkdir_py_files(&path));
        } else {
            out.push(path);
        }
    }
    out
}

/// Resolve `source_ref` (`"dispar_orchestrate/<file>.py::<fn>"`) against
/// `allowlist` and return the function's source text.
///
/// # Errors
///
/// [`SourceError::NotAllowlisted`] if `source_ref` has no `::` separator or
/// its file half isn't an allowlist key; [`SourceError::FunctionNotFound`]
/// if the function half isn't found as a `def`/`async def` boundary in
/// that file; [`SourceError::Io`] on a read failure of an allowlisted
/// (already-canonicalized) path.
pub fn read_source(allowlist: &SourceAllowlist, source_ref: &str) -> Result<String, SourceError> {
    let (file, func) = source_ref
        .split_once("::")
        .ok_or_else(|| SourceError::NotAllowlisted(source_ref.to_owned()))?;
    let path = allowlist
        .get(file)
        .ok_or_else(|| SourceError::NotAllowlisted(file.to_owned()))?;
    let text = std::fs::read_to_string(path).map_err(|_| SourceError::Io)?;
    extract_function(&text, func).ok_or_else(|| SourceError::FunctionNotFound(func.to_owned()))
}

/// Locate a `def <func>(`/`async def <func>(` line boundary in `text` and
/// return the slice from there to the next top-level `def`/`async def`/`@`
/// (a decorator on the following definition) or EOF. Textual, not an AST
/// parse — sufficient for a read-only source viewer.
fn extract_function(text: &str, func: &str) -> Option<String> {
    let needle_def = format!("def {func}(");
    let needle_async_def = format!("async def {func}(");
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.iter().position(|l| {
        let trimmed = l.trim_start();
        trimmed.starts_with(&needle_def) || trimmed.starts_with(&needle_async_def)
    })?;
    let indent = lines[start].len() - lines[start].trim_start().len();
    let mut end = lines.len();
    for (i, line) in lines.iter().enumerate().skip(start + 1) {
        if line.trim().is_empty() {
            continue;
        }
        let this_indent = line.len() - line.trim_start().len();
        let trimmed = line.trim_start();
        if this_indent <= indent
            && (trimmed.starts_with("def ")
                || trimmed.starts_with("async def ")
                || trimmed.starts_with('@'))
        {
            end = i;
            break;
        }
    }
    Some(lines[start..end].join("\n"))
}

/// `"unknown"` (both Dockerfiles' `ARG GIT_SHA=unknown` default) or an
/// empty string — either means "no real commit was ever recorded here".
fn is_placeholder(commit: &str) -> bool {
    commit.is_empty() || commit == "unknown"
}

/// Compare the op's declared `commit` metadata against this image's own
/// `GIT_SHA`. `expected` is `GIT_SHA` (this image); `actual` is the op's
/// `commit` metadata value, which is `None` when `Dagster` metadata is
/// somehow absent (treated as unverifiable, never trusted silently).
///
/// `"unknown"` is the literal default BOTH `dagster/Dockerfile` and
/// `rust/Dockerfile`'s `ARG GIT_SHA=unknown` fall back to when nobody
/// exports a real `GIT_SHA` before `docker compose build` — two images
/// built that way would otherwise report `op_commit == Some("unknown")`
/// and `this_image_git_sha == "unknown"`, which a bare `==` comparison
/// treats as a match. That is backwards: it is exactly the case where
/// NEITHER image's real commit is known, so nothing about "the code that
/// ran" has been verified at all — the route must refuse, not succeed.
/// [`is_placeholder`] therefore short-circuits BEFORE the equality check,
/// on EITHER value, catching `"unknown"`/`"unknown"`, `"unknown"`/real,
/// real/`"unknown"`, and empty-string on either side.
///
/// # Errors
///
/// [`SourceError::UnverifiableProvenance`] when `op_commit` is `None` or
/// either side is a placeholder; [`SourceError::CommitMismatch`] when both
/// are real, non-placeholder values that differ.
pub fn check_commit(op_commit: Option<&str>, this_image_git_sha: &str) -> Result<(), SourceError> {
    let Some(op_commit) = op_commit else {
        return Err(SourceError::UnverifiableProvenance);
    };
    if is_placeholder(op_commit) || is_placeholder(this_image_git_sha) {
        return Err(SourceError::UnverifiableProvenance);
    }
    if op_commit == this_image_git_sha {
        Ok(())
    } else {
        Err(SourceError::CommitMismatch {
            expected: this_image_git_sha.to_owned(),
            actual: op_commit.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    fn base_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        for (name, contents) in files {
            fs::write(dir.path().join(name), contents).unwrap();
        }
        dir
    }

    #[test]
    fn resolves_a_real_allowlisted_file() {
        let dir = base_with(&[("assets.py", "def ingest_bronze_table():\n    pass\n")]);
        let allowlist = build_allowlist(dir.path()).unwrap();
        let text = read_source(
            &allowlist,
            "dispar_orchestrate/assets.py::ingest_bronze_table",
        )
        .unwrap();
        assert!(text.contains("def ingest_bronze_table"));
    }

    #[test]
    fn rejects_a_real_dot_dot_escape() {
        let dir = base_with(&[("assets.py", "x = 1\n")]);
        // A sibling file OUTSIDE the base, to prove `..` would actually
        // reach something if not rejected — not just a string that
        // contains the characters `..` with nothing behind it.
        let parent = dir.path().parent().unwrap();
        fs::write(parent.join("secret.py"), "SECRET = 1\n").unwrap();
        let allowlist = build_allowlist(dir.path()).unwrap();
        let err = read_source(&allowlist, "../secret.py::x").unwrap_err();
        assert!(matches!(err, SourceError::NotAllowlisted(_)));
    }

    #[test]
    fn rejects_a_real_absolute_path() {
        let dir = base_with(&[("assets.py", "x = 1\n")]);
        // A SECOND, independent tempdir (never a fixed shared path like
        // `/tmp/...`, which would be flaky under parallel test runs --
        // `cargo test` shares one process across threads -- and would
        // leave a file behind on the host) proves the absolute path
        // genuinely resolves to a real file outside `dir`, not merely a
        // string containing a leading `/`.
        let outside_dir = tempdir().unwrap();
        let outside_file = outside_dir.path().join("absolute_escape.py");
        fs::write(&outside_file, "SECRET = 1\n").unwrap();
        let allowlist = build_allowlist(dir.path()).unwrap();
        let op_ref = format!("{}::x", outside_file.display());
        let err = read_source(&allowlist, &op_ref).unwrap_err();
        assert!(matches!(err, SourceError::NotAllowlisted(_)));
        // `outside_dir` is dropped (and its temp directory removed) at the
        // end of this test, same as `dir` -- nothing written outside a
        // `TempDir`'s own managed lifetime.
    }

    #[test]
    #[cfg(unix)]
    fn rejects_a_real_symlink_pointing_outside_the_base() {
        let dir = base_with(&[]);
        let parent = dir.path().parent().unwrap();
        let outside = parent.join("pipeline_source_test_outside.py");
        fs::write(&outside, "SECRET = 1\n").unwrap();
        std::os::unix::fs::symlink(&outside, dir.path().join("linked.py")).unwrap();
        let allowlist = build_allowlist(dir.path()).unwrap();
        // Even if "linked.py" were (wrongly) added to the allowlist by
        // name, canonicalizing it must resolve outside `dir` and be
        // rejected -- this test asserts the allowlist builder itself
        // never admits a symlink whose target canonicalizes outside the
        // base, not just that a caller-supplied path is checked.
        assert!(!allowlist.contains_key("dispar_orchestrate/linked.py"));
        let _ = fs::remove_file(&outside);
    }

    #[test]
    fn rejects_a_non_py_extension() {
        let dir = base_with(&[("notes.txt", "not python")]);
        let allowlist = build_allowlist(dir.path()).unwrap();
        assert!(!allowlist.contains_key("dispar_orchestrate/notes.txt"));
    }

    #[test]
    fn rejects_an_unknown_function_within_an_allowlisted_file() {
        let dir = base_with(&[("assets.py", "def real_fn():\n    pass\n")]);
        let allowlist = build_allowlist(dir.path()).unwrap();
        let err =
            read_source(&allowlist, "dispar_orchestrate/assets.py::not_a_real_fn").unwrap_err();
        assert!(matches!(err, SourceError::FunctionNotFound(_)));
    }

    #[test]
    fn commit_mismatch_is_reported_distinctly() {
        assert!(matches!(
            check_commit(Some("deployed-sha"), "different-sha"),
            Err(SourceError::CommitMismatch { .. })
        ));
        assert!(check_commit(Some("real-sha-abc123"), "real-sha-abc123").is_ok());
        // An op with no commit metadata at all (should not happen after
        // WS4 item B2, but a foreign/future job might) is treated as
        // "cannot verify" -- `UnverifiableProvenance`, a distinct variant
        // from `CommitMismatch` (see `an_unknown_or_empty_git_sha_on_either_side_is_never_treated_as_a_match`
        // below) -- never silently trusted either way.
        assert!(matches!(
            check_commit(None, "real-sha-abc123"),
            Err(SourceError::UnverifiableProvenance)
        ));
    }

    /// `GIT_SHA` defaults to the literal `"unknown"` on BOTH images
    /// (`dagster/Dockerfile`'s `ARG GIT_SHA=unknown`; `rust/Dockerfile`'s
    /// identical default, added by this task) whenever the human building
    /// them forgets to export a real `GIT_SHA` first. Two images built
    /// that way still have `op_commit == Some("unknown")` and
    /// `this_image_git_sha == "unknown"` — an EQUAL comparison under a
    /// naive `Some(c) if c == this_image_git_sha => Ok(())` arm would let
    /// `/source` present unverifiable text as a confirmed-provenance
    /// match. Every one of the six cases below must be a mismatch:
    /// `"unknown"` on both sides, `"unknown"` vs. a real sha (either
    /// direction), and an EMPTY STRING on either side (a `GIT_SHA=""`
    /// export, or an `ARG GIT_SHA=unknown` line accidentally dropped
    /// entirely leaving `Dagster`'s `os.environ.get("GIT_SHA", "unknown")`
    /// fall through to its own default while this image somehow reports
    /// `""`).
    #[test]
    fn an_unknown_or_empty_git_sha_on_either_side_is_never_treated_as_a_match() {
        assert!(matches!(
            check_commit(Some("unknown"), "unknown"),
            Err(SourceError::UnverifiableProvenance)
        ));
        assert!(matches!(
            check_commit(Some("unknown"), "real-sha-abc123"),
            Err(SourceError::UnverifiableProvenance)
        ));
        assert!(matches!(
            check_commit(Some("real-sha-abc123"), "unknown"),
            Err(SourceError::UnverifiableProvenance)
        ));
        assert!(matches!(
            check_commit(Some(""), "real-sha-abc123"),
            Err(SourceError::UnverifiableProvenance)
        ));
        assert!(matches!(
            check_commit(Some("real-sha-abc123"), ""),
            Err(SourceError::UnverifiableProvenance)
        ));
        assert!(matches!(
            check_commit(Some(""), ""),
            Err(SourceError::UnverifiableProvenance)
        ));
        // A real, non-"unknown", non-empty, EQUAL sha on both sides is
        // still the only way to succeed.
        assert!(check_commit(Some("real-sha-abc123"), "real-sha-abc123").is_ok());
    }

    /// A caller-supplied `op=` with a URL-ENCODED traversal
    /// (`%2e%2e%2fsecret.py::x`) must fail the same way a literal `../`
    /// does: this module never decodes percent-escapes, so the encoded
    /// string simply never matches an allowlist key either -- closing the
    /// "encoded variants" traversal class the brief calls out explicitly.
    #[test]
    fn rejects_an_encoded_dot_dot_escape() {
        let dir = base_with(&[("assets.py", "x = 1\n")]);
        let allowlist = build_allowlist(dir.path()).unwrap();
        let err = read_source(&allowlist, "%2e%2e%2fsecret.py::x").unwrap_err();
        assert!(matches!(err, SourceError::NotAllowlisted(_)));
    }

    /// A `source_ref` with no `::` separator at all (malformed input, not
    /// necessarily hostile) is refused the same honest way, not a panic on
    /// an unwrapped `split_once`.
    #[test]
    fn rejects_a_source_ref_with_no_separator() {
        let dir = base_with(&[("assets.py", "x = 1\n")]);
        let allowlist = build_allowlist(dir.path()).unwrap();
        let err = read_source(&allowlist, "dispar_orchestrate/assets.py").unwrap_err();
        assert!(matches!(err, SourceError::NotAllowlisted(_)));
    }
}
