## Summary

<!-- What does this PR do, and why? Link related issues (e.g. "Closes #123"). -->

## Type of change

- [ ] `feat` — new feature
- [ ] `fix` — bug fix
- [ ] `docs` — documentation only
- [ ] `refactor` — no behavior change
- [ ] `perf` — performance improvement
- [ ] `test` — adding/fixing tests
- [ ] `chore` / `ci` / `build` — tooling, dependencies, CI
- [ ] Breaking change (describe migration/impact below)

## How was this tested?

<!-- Commands run, manual verification steps, new/updated automated tests. -->

## Verification checklist

- [ ] Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/) (see CONTRIBUTING.md)
- [ ] `bun run typecheck` / `bun run lint` pass (if frontend changed)
- [ ] `cargo fmt --check` passes (if Rust changed)
- [ ] `cargo clippy --all-targets --all-features --locked -- -D warnings` passes (if Rust changed)
- [ ] `cargo test --all-features` passes (if Rust changed)
- [ ] Relevant docs updated (`README.md`, `docs/ARCHITECTURE.md`, `CHANGELOG.md`)
- [ ] No secrets, `.env` files, or credentials included in this diff

## Additional context

<!-- Anything else reviewers should know: known limitations, follow-up work, screenshots. -->
