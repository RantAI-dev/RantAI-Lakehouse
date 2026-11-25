#!/usr/bin/env python3
"""Fail when a file Rust compiles in sits outside `rust/` and does not set `rust`.

# Why this exists

`cargo fmt`, `clippy`, `build`, `test` and MSRV run only when the change scope
says `rust` (`scripts/ci/detect_change_scope.sh`). A file that Rust compiles in
with `include_str!` or `include_bytes!` is part of that build even when it sits
outside `rust/`, so a change to it that does not set `rust` skips the only jobs
that would compile it. `OPS-24` R5 found three such paths by reading the diff;
this check finds them by reading the sources.

# What it does

1. Reads every `*.rs` file under `rust/` and extracts the argument of each
   `include_str!` / `include_bytes!`.
2. Resolves it: string literals are concatenated in order, and
   `env!("CARGO_MANIFEST_DIR")` (the only macro the workspace uses for this) is
   replaced with the directory of the nearest `Cargo.toml` above the file.
3. Keeps the paths that resolve outside `rust/`.
4. Runs the real scope script on each one, in a throwaway git repository whose
   only change is that file, and fails unless the script sets `rust=true`.

Running the script rather than re-reading its rules is deliberate: the answer
this check reports is the answer CI will act on.

# What it does not cover

- A path built from a `macro_rules!` metavariable (the `fixture!` macro in
  `rust/crates/lakehouse-api/src/upload_parse.rs` builds `ops/fixtures/uploads/`
  paths that way). Those are listed as skipped with the reason, never silently
  dropped; `ops/fixtures/**` already forces every job.
- A path built at runtime, and `include!` of another file.
- Absolute paths: they are reported as skipped rather than guessed at.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
RUST_DIR = REPO_ROOT / "rust"
SCOPE_SCRIPT = REPO_ROOT / "scripts" / "ci" / "detect_change_scope.sh"

INCLUDE_RE = re.compile(r"include_(?:str|bytes)!\s*\(")
STRING_LITERAL = '"((?:[^"\\\\]|\\\\.)*)"'
MANIFEST_DIR = 'env!("CARGO_MANIFEST_DIR")'

# Escapes a path could reasonably use. Anything else is left as it is: a wrong
# path is better reported than silently rewritten.
UNESCAPE = {"n": "\n", "t": "\t", "r": "\r", '"': '"', "\\": "\\"}


def _unescape(text: str) -> str:
    out = []
    index = 0
    while index < len(text):
        char = text[index]
        if char == "\\" and index + 1 < len(text):
            out.append(UNESCAPE.get(text[index + 1], text[index + 1]))
            index += 2
            continue
        out.append(char)
        index += 1
    return "".join(out)


def _argument(text: str, open_paren: int) -> str:
    """Return the text between the macro's parentheses."""
    depth = 0
    index = open_paren
    in_string = False
    while index < len(text):
        char = text[index]
        if in_string:
            if char == "\\":
                index += 2
                continue
            if char == '"':
                in_string = False
        elif char == '"':
            in_string = True
        elif char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
            if depth == 0:
                return text[open_paren + 1 : index]
        index += 1
    return ""


def _crate_dir(source: Path) -> Path:
    directory = source.parent
    while directory != directory.parent:
        if (directory / "Cargo.toml").is_file():
            return directory
        directory = directory.parent
    return REPO_ROOT


def _resolve(source: Path, argument: str) -> tuple[Path | None, str]:
    """Return the resolved path, or None and the reason it could not be read."""
    if "$" in argument:
        return None, "the path is built from a macro_rules! metavariable"
    parts: list[str] = []
    index = 0
    while index < len(argument):
        if argument.startswith(MANIFEST_DIR, index):
            parts.append(str(_crate_dir(source)))
            index += len(MANIFEST_DIR)
            continue
        char = argument[index]
        if char in " \t\r\n,()":
            index += 1
            continue
        if argument.startswith("concat!", index):
            index += len("concat!")
            continue
        if char == '"':
            literal = re.match(STRING_LITERAL, argument[index:])
            if literal is None:
                return None, "an unreadable string literal in the argument"
            parts.append(_unescape(literal.group(1)))
            index += literal.end()
            continue
        return None, f"the argument has a part this check does not read: {argument[index:]!r}"
    joined = "".join(parts)
    if not joined:
        return None, "no string literal in the argument"
    path = Path(joined)
    if not path.is_absolute():
        path = source.parent / path
    return Path(os.path.normpath(path)), ""


def _embedded_paths() -> tuple[list[tuple[Path, Path]], list[tuple[Path, str]]]:
    """Return (embedded path, source) pairs outside `rust/`, and the skipped ones."""
    outside: list[tuple[Path, Path]] = []
    skipped: list[tuple[Path, str]] = []
    for source in sorted(RUST_DIR.rglob("*.rs")):
        text = source.read_text(encoding="utf-8", errors="replace")
        for match in INCLUDE_RE.finditer(text):
            argument = _argument(text, match.end() - 1)
            resolved, reason = _resolve(source, argument)
            if resolved is None:
                skipped.append((source, reason))
                continue
            try:
                relative = resolved.relative_to(REPO_ROOT)
            except ValueError:
                skipped.append((source, f"the path {resolved} is outside this checkout"))
                continue
            if relative.parts and relative.parts[0] == "rust":
                continue
            if resolved in [path for path, _ in outside]:
                continue
            outside.append((resolved, source))
    return outside, skipped


def _scope_sets_rust(path: Path) -> bool:
    """Run the scope script on one changed file and return its `rust` output."""
    relative = path.relative_to(REPO_ROOT)
    environment = {
        "PATH": os.environ.get("PATH", "/usr/local/bin:/usr/bin:/bin"),
        "EVENT_NAME": "pull_request",
        "GIT_AUTHOR_NAME": "ci-lint",
        "GIT_AUTHOR_EMAIL": "ci-lint@example.com",
        "GIT_COMMITTER_NAME": "ci-lint",
        "GIT_COMMITTER_EMAIL": "ci-lint@example.com",
    }
    with tempfile.TemporaryDirectory() as temporary:
        work = Path(temporary)
        subprocess.run(["git", "init", "-q", "-b", "main"], cwd=work, check=True, env=environment)
        target = work / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text("base\n", encoding="utf-8")
        subprocess.run(["git", "add", "."], cwd=work, check=True, env=environment)
        subprocess.run(["git", "commit", "-q", "-m", "base"], cwd=work, check=True, env=environment)
        base = subprocess.run(
            ["git", "rev-parse", "HEAD"], cwd=work, check=True, env=environment,
            capture_output=True, text=True,
        ).stdout.strip()
        target.write_text("changed\n", encoding="utf-8")
        subprocess.run(["git", "add", "."], cwd=work, check=True, env=environment)
        subprocess.run(["git", "commit", "-q", "-m", "change"], cwd=work, check=True, env=environment)

        output = work / "github_output"
        output.write_text("", encoding="utf-8")
        run_env = dict(environment)
        run_env["BASE_SHA"] = base
        run_env["GITHUB_OUTPUT"] = str(output)
        completed = subprocess.run(
            ["bash", str(SCOPE_SCRIPT)], cwd=work, env=run_env,
            capture_output=True, text=True,
        )
        if completed.returncode != 0:
            sys.stderr.write(completed.stdout + completed.stderr)
            return False
        for line in output.read_text(encoding="utf-8").splitlines():
            if line == "rust=true":
                return True
        return False


def main() -> int:
    if not SCOPE_SCRIPT.is_file():
        sys.stderr.write(f"the scope script is missing: {SCOPE_SCRIPT}\n")
        return 2
    outside, skipped = _embedded_paths()
    failures = 0
    for path, source in outside:
        relative = path.relative_to(REPO_ROOT)
        from_source = source.relative_to(REPO_ROOT)
        if _scope_sets_rust(path):
            print(f"ok: {relative} (compiled in by {from_source}) sets rust")
        else:
            failures += 1
            print(f"FAIL: {relative} (compiled in by {from_source}) does not set rust in detect_change_scope.sh")
    for source, reason in skipped:
        print(f"skipped: {source.relative_to(REPO_ROOT)}: {reason}")
    print(f"{len(outside)} path(s) compiled into Rust from outside rust/, {failures} without the rust scope")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
