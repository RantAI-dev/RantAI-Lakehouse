# `ADM-12` Single sign-on set in Settings by a Platform Admin

| | |
| --- | --- |
| Backlog | `ADM-12` in [BACKLOG.md](../BACKLOG.md) |
| Module | Administration & Security |
| Size | M for OIDC; SAML sized after [ADR 0017](../../adr/0017-saml-sign-in.md) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |
| Spec checked | Against the code at `dcaae8a5` on 2026-11-09. Feature page [`sso-in-settings.md`](../features/sso-in-settings.md) |

## Why

Single sign-on is set only through `.env` (`config.rs:552-617`, `OIDC_*`), read once at start-up (`state.rs:776`). Settings → Sign-in shows it read-only (`sso-page.tsx`). Changing a role map needs a terminal and a restart.

## What users get

A Platform Admin opens Settings → Sign-in, fills in the OIDC issuer, client ID and secret, provider name, groups claim, role map and first-login account creation, runs a test sign-in that shows what the provider sent and which roles it maps to, and saves. The next sign-in uses it, with no restart. A value set in `.env` wins and shows read-only. The secret is sealed and never shown again.

SAML (owner, 2026-11-09: built as well as OIDC) is planned after ADR 0017 is decided.

## Target specs

| Capability | Today | Target |
| --- | --- | --- |
| Where OIDC is set | `.env` only | Settings → Sign-in, or `.env` |
| Who may change it | n/a | Only a caller holding `*:*` (a role map can grant Platform Admin, so `settings:write` alone is not enough) |
| `.env` precedence | n/a | Issuer or client ID set in `.env` → the whole connection is server-owned and read-only, as `ADM-7` R1 |
| Client secret | `.env` | Sealed with the installation key (`secret_crypto.rs`), shown only as "set" |
| Taking effect | Restart | Next sign-in; settings cache cleared on save, 30 s TTL otherwise |
| Test before saving | None | A real sign-in round trip against the draft; shows subject, email, groups and mapped roles; creates no user and no session; draft expires after 10 minutes *(proposed)* |
| Issuer address set in the console | n/a | `https://` required; internal addresses refused (set an internal IdP in `.env`) *(proposed, as `ADM-9`)* |
| Role map editing | Text in `.env` | Rows of group → role, roles picked from the role list |
| SAML | None | After ADR 0017 |

## Benchmark

| Product | SSO set in the UI |
| --- | --- |
| Metabase | SAML, JWT, LDAP and Google set in Admin → Authentication (paid plans for SAML/JWT); group mapping in the UI |
| Grafana | OIDC/OAuth, SAML and LDAP set in the UI (SSO settings API), with `.ini` values winning |
| Superset | Config file only |

Target: match Grafana (UI with file precedence, group-to-role mapping) and add a test sign-in before saving.

## Acceptance checklist

The full list is on the feature page, [`sso-in-settings.md`](../features/sso-in-settings.md).

## Not included

- SAML until ADR 0017 is decided.
- Several identity providers at once.
- SCIM provisioning.

## Asking the assistant

No. The assistant does not handle sign-in configuration or secrets.
