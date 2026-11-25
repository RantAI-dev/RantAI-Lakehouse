# Single sign-on in Settings

| | |
| --- | --- |
| Module | Administration & Security |
| Backlog | `ADM-12` |
| Kind | Feature |
| Spec | `docs/core/specs/adm-12.md` |
| Status | In Acceptance (OIDC); SAML waits for ADR 0017 |
| Priority | P2 |
| Owner | In the base |
| Acceptor | In the base |
| Plan | `docs/superpowers/plans/2026-11-09-adm-12-sso-in-settings.md` |
| Started | 2026-11-09 |
| Shipped | 2026-11-11 (OIDC) |
| Evidence | PR #455 |

## Problem

Single sign-on can be set only in `.env` and needs a restart. Settings shows it read-only.

## What the user can do when this is done

1. A Platform Admin fills in OIDC in Settings → Sign-in: issuer, client ID, client secret, provider name, groups claim, role map (group → role picked from the role list), and first-login account creation.
2. They run a test sign-in against the draft and see the subject, email, groups and the roles they map to, before saving.
3. After saving, the next sign-in uses it, with no restart.
4. A value set in `.env` shows read-only with "Set by the server".
5. The secret is never shown again; the page says only whether it is set.
6. Someone with `settings:write` but not `*:*` sees the section read-only.

## Not included

- SAML until ADR 0017 is decided (owner wants it built).
- Several providers at once; SCIM.

## Asking the assistant

No.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| D1 | SAML is built as well as OIDC | Planned after ADR 0017 | Owner, 2026-11-09 |
| D2 | Only `*:*` may change SSO settings | As stated | Planner |
| D3 | Console-set issuer must be `https://` and not internal; an internal IdP goes in `.env` | *(proposed)* | |
| D4 | Test draft lives 10 minutes | *(proposed)* | |

## Limits to tell a customer

- One identity provider at a time.
- An identity provider on an internal address must be set in `.env`.

## Acceptance checklist

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `ADM-12-AC1` | With no `OIDC_*` in `.env`, fill in a real provider and Test sign-in | The provider's sign-in opens; back on Settings: subject, email, groups, mapped roles shown; no user created | Not run |
| `ADM-12-AC2` | Save, sign out, sign in with SSO | Works with no restart; roles follow the role map | Not run |
| `ADM-12-AC3` | Reload Settings | Secret shows "Set", never the value | Not run |
| `ADM-12-AC4` | Set `OIDC_ISSUER` in `.env`, restart | Connection fields read-only, "Set by the server" | Not run |
| `ADM-12-AC5` | As a user with `settings:write` but not `*:*` | Section read-only; `PUT /api/settings/sso` (operator, curl) answers 403 | Not run |
| `ADM-12-AC6` | Enter `http://` or an internal issuer | Refused with a plain message | Not run |
| `ADM-12-AC7` | Wrong client secret, Test sign-in | The test reports the provider's refusal in plain words, nothing saved | Not run |
| `ADM-12-AC8` | Audit log after saving | `settings.sso.update` names the changed fields, no secret | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo
- [ ] `CHANGELOG.md` entry a customer can read
