# `SRC-12` Credentials done properly

| | |
| --- | --- |
| Backlog | `SRC-12` in [BACKLOG.md](../BACKLOG.md) |
| Module | Data |
| When | Next |
| Size | L (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

Connector credentials sit in plain files at rest, no external secret manager is supported, and OAuth sources answer unsupported.

## What users get

Credentials kept in a secret manager or encrypted, and one-click sign-in for sources that support it.

## Target specs

"Today" is `main` at `f3a3196`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| External secret managers | None | HashiCorp Vault, AWS Secrets Manager, Google Secret Manager, Azure Key Vault (Airbyte's four) |
| At rest without a secret manager | Plain files | Encrypted (AES-256-GCM, proposed) with a key from the environment; compose refuses to start without it |
| OAuth sign-in | REST OAuth2 answers unsupported | OAuth 2.0 authorisation-code flow with token refresh for every connector whose source offers it |
| Rotation | Manual | Rotate a credential without recreating the connector; old value unusable at once |

## Benchmark

Airbyte Core: AWS Secrets Manager, Google Secret Manager, Azure Key Vault, HashiCorp Vault. Snowflake: secret objects and external providers. Databricks: Unity Catalog connections with OAuth.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `SRC-12-AC1` A connector reads its password from Vault; the file system holds no copy
- `SRC-12-AC2` With no secret manager, the stored file is unreadable without the key
- `SRC-12-AC3` An OAuth source connects with one sign-in and keeps working after its token expires
- `SRC-12-AC4` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
