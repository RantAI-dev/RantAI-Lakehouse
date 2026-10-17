# SEC-42: the `zip` 8.6.0 advisories, ignored with a checked reason

Backlog: `SEC-42` (Task, P1). Base: `main` at the merge of #195. Branch:
`fix/sec-42-zip-advisories`. Worktree: `/home/shiro/rantai/wt/sec-42`.
Developer rules: `/home/shiro/rantai/review-logs/dev-rules.md`.

## 1. Facts (verified by the planner, 2026-10-17)

- `cargo audit` and `cargo deny check` are red on `main` and every PR:
  `zip` 8.6.0 has `RUSTSEC-2026-0367` (SUID/SGID bits kept on extract),
  `-0368` (`ZipArchive::extract` writes symlinks outside the destination),
  `-0369` (`extract` applies permissions through symlinks). All are fixed
  in `zip` 9.0.0 and all are in `extract`, which writes an archive to disk.
- `zip` arrives only through `calamine` 0.36.1 (`cargo tree -i zip@8.6.0`:
  `calamine` <- `lakehouse-api`). The newest `calamine` (0.36.1) requires
  `zip ^8.6`; no release accepts `zip` 9.
- `calamine` 0.36.1's source never calls `extract` (it reads entries with
  `by_name`); the workspace has no direct `zip` dependency and no
  `extract` call.

## 2. Decision

Ignore the three IDs, in both lists, each with the reason above and the
condition for removing it ("drop when `calamine` accepts `zip` >= 9.0.0;
then `cargo update -p zip`"). No crate changes. Do not fork or patch
`calamine`.

## 3. Tasks (one commit each)

**T1.** `rust/deny.toml` `[advisories]`: add the three IDs to `ignore`
with a comment block in the file's existing style (what, why not
reachable, `cargo tree` path, when to remove). Acceptance: `cargo deny
check advisories` passes locally (no `cargo build` needed).

**T2.** `.github/workflows/security.yml`: the `cargo audit` step adds
`--ignore` for the three IDs, with a comment pointing to `rust/deny.toml`
as the existing comment does. Acceptance: `cargo audit --ignore
RUSTSEC-2023-0071 --ignore RUSTSEC-2026-0367 --ignore RUSTSEC-2026-0368
--ignore RUSTSEC-2026-0369` exits 0 in `rust/` (install `cargo-audit` if
missing, outside the heavy lock is fine: it does not compile the
workspace).

**T3.** `CHANGELOG.md` bullet under `## [Unreleased]` in the file's style,
and re-verify the `calamine` facts in §1 yourself (quote the `grep`
output for `extract` in its source and the `cargo tree -i` output) in the
handoff.

## 4. Handoff

(developer appends: commits as 12-char hashes, commands, outputs)

## 5. Review

(planner)
