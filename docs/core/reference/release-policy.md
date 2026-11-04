# Release and versioning

## Where things stand (2026-11-04)

| Fact | Source |
| --- | --- |
| Two tags exist: `v0.1.0` and `v0.2.0` | `git tag` |
| `v0.2.0` is the first release built, signed and published as a draft by the release pipeline (`REL-2`); it waits for the product owner to publish it and to make the GHCR images public | `gh release view v0.2.0`; `docs/plans/REL-2-RESULT.md` |
| The changelog has `[0.2.0] - 2026-11-02` with an "Upgrade notes" section, and an `[Unreleased]` section for what merged after the tag | `CHANGELOG.md` |
| The console package version is `0.2.0`, the same as the tag, the API crate and the changelog | `package.json`; `ops/release/check_version.py` |
| `v0.1.0` was Apache-2.0; the project is now AGPL-3.0-or-later | `README.md` |
| `SECURITY.md` still says no `v0.1.0` has been cut | Contradicts the tags; backlog `DOC-1` |
| `main` is the only supported line | `SECURITY.md` |
| `main` requires five checks and blocks force-pushes | `REL-1`, `docs/CI.md` |
| Database migrations are forward-only and never edited once applied | `AGENTS.md` |

## Policy (signed by the product owner on 2026-10-31, `REL-2`)

### Version numbers

Semantic Versioning, as the changelog already intends. While the version is
below 1.0:

| Change | Bump |
| --- | --- |
| New feature, or any change an operator must act on (new required setting, changed default, migration needing a step) | Minor: `0.1.0` → `0.2.0` |
| Fixes only, no operator action | Patch: `0.2.0` → `0.2.1` |

`1.0.0` is reserved for the first release that passes the production
readiness checklist with no blocking item.

### What makes a release

1. Every feature in it is Done ([README.md](../README.md)).
2. [PRODUCT.md](../PRODUCT.md) has been re-assessed.
3. `[Unreleased]` in the changelog is renamed to the version and date.
4. An "Upgrade notes" section lists every operator action: new settings,
   changed defaults, manual steps.
5. The version is the same in the tag, the changelog and `package.json`.
6. The acceptance gates relevant to the changes were run and their output
   kept.

### Release notes

Written for an operator and a customer, not a developer. For each release:

- What is new, in one sentence per feature.
- What changed that needs action.
- What was fixed.
- Known limitations added or removed (from [PRODUCT.md](../PRODUCT.md)).

### Applied: 0.2.0 (2026-11-04)

The product owner signed this policy on 2026-10-31 (`REL-2` decision D4: SemVer as written
here; the first release through the pipeline is 0.2.0). For 0.2.0:

- `[Unreleased]` became `[0.2.0] - 2026-11-02`, and the version is the same in the tag, the
  changelog, `package.json` and the API crate (item 5; the release job's version check).
- The upgrade notes are in the changelog's `[0.2.0]` section (item 4).
- The release job ran after the product owner approved the `release` environment on
  2026-11-04 (run 41666401368).
- Item 1 is not met: no feature in 0.2.0 has passed its acceptance checklist yet (`QA-1`).
  The release ships as built and unaccepted, as `PRODUCT.md` section 4 says.

## Upgrade

| Step | Note |
| --- | --- |
| Read the release's upgrade notes | Changed defaults are the usual trap. Example: the Gold publish feature changes the default mart list to empty. |
| Back up the application database | Procedure in `docs/OPERATIONS.md` |
| Deploy the new images | Migrations apply on start |
| Run the acceptance gates that apply | `ops/g*` |
| Check the Monitoring module | Health, Services, Alerts |

## Rollback

Migrations are forward-only, so rolling back the software does not roll back
the database.

| Situation | What to do |
| --- | --- |
| A feature misbehaves and has an off switch | Switch it off. Prefer features that can be disabled without redeploying. |
| The release is bad and no migration ran | Redeploy the previous images |
| The release is bad and a migration ran | Restore the application database from the pre-upgrade backup, then redeploy the previous images. Data written since the backup is lost. |

A tested, timed restore exists (`OPS-7`, `docs/OPERATIONS.md`), and REL-2
adds a complete local-disk backup (#295). `lakehousectl update` (REL-2
slice 4) takes that backup before every update and restores it by itself
when the new release does not become ready.

## Supported versions

To be decided with the support model ([support-model.md](support-model.md)). Today
only `main` is supported, which is not a position a customer can plan
around.

## To be decided

| Decision | Owner |
| --- | --- |
| Adopt the version policy above | Product owner |
| Cut `0.2.0` from the current `[Unreleased]` content, or wait | Product owner |
| How many past versions receive fixes | Product owner |
| Release cadence | Product owner |
