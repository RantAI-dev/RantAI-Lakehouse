# Release and versioning

## Where things stand

| Fact | Source |
| --- | --- |
| One tag exists: `v0.1.0` | `git tag` |
| The changelog has `[0.1.0] - 2026-08-30` and a long `[Unreleased]` section | `CHANGELOG.md` |
| The changelog follows Keep a Changelog and "intends to adhere to Semantic Versioning once a first release is tagged" | `CHANGELOG.md` |
| The console package version is `0.1.0` | `package.json` |
| `v0.1.0` was Apache-2.0; the project is now AGPL-3.0-or-later | `README.md` |
| `SECURITY.md` says no `v0.1.0` has been cut | Contradicts the tag; backlog `DOC-1` |
| `main` is the only supported line | `SECURITY.md` |
| `main` has no branch protection | Verified 2026-10-02; `docs/CI.md` has the drafted settings |
| Database migrations are forward-only and never edited once applied | `AGENTS.md` |

Everything merged since `v0.1.0` is unreleased. There is no written release
procedure.

## Proposed policy (needs product-owner sign-off, `REL-2`)

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

A tested, timed restore does not exist yet (checklist item 4.4). Until it
does, rollback after a migration is a procedure on paper.

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
