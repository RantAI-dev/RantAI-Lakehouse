# `SEC-65` Single sign-on joins an existing account only on a confirmed email

| | |
| --- | --- |
| Backlog | `SEC-65` in [BACKLOG.md](../BACKLOG.md) |
| Module | Administration & Security |
| Size | S |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |
| Spec checked | Against the code at `d1f518f2` (branch `feat/adm-12-oidc-in-settings`, which this task builds on) on 2026-11-11 |

## Why

When a person signs in through single sign-on for the first time and account creation is on, the API looks for an existing account with the same email and joins the two (`lakehouse-auth/src/oidc.rs`, `provision_user`). It does not look at whether the identity provider confirmed that email (`email_verified` in the ID token; the claim is already parsed). A provider that lets people type any email, or an account at the provider that was never confirmed, can therefore sign in as an existing Lakehouse user, a Platform Admin included. Filed from `ADM-11` (D10).

## What users get

Nobody can take over an existing account by claiming its email at the identity provider. A person whose email the provider confirmed is joined to their account as today. An administrator whose provider never sends the confirmation (some do not) can say, once, that this provider's emails are trusted.

## Target specs

| Capability | Today | Target |
| --- | --- | --- |
| Joining an existing account by email | Always, whatever the token says | Only when the token carries `email_verified: true`, or the connection is marked "trust this provider's emails" |
| Email not confirmed, an account with that email exists | Signed in as that account | Sign-in refused with a plain sentence on the sign-in page; an audit row; nothing created or changed *(proposed)* |
| Email not confirmed, no account with that email | New account with that email | New account **without** the email, so an unconfirmed address is neither stored nor mailed *(proposed)* |
| "Trust this provider's emails" | n/a | Off by default; set in Settings → Sign-in or `OIDC_TRUST_EMAIL` in `.env` (`.env` wins, as every other value) *(proposed: default off)* |
| A person already joined (their `auth_identity` row exists) | Signs in | Unchanged: they are found by the provider's subject, not by email |
| Seeing it before saving | Test sign-in shows `emailVerified` (`ADM-12`) | The card also says, when it is not `true`, that existing accounts will not be joined |

## Benchmark

OpenID Connect Core 1.0, section 5.1, defines `email_verified` as "true if the End-User's e-mail address has been verified; otherwise false", and section 5.7 tells a client not to rely on a claim as a stable identifier unless the provider's policy makes it one. How other BI and data products handle it was **not researched** for this task; the target is the standard's own rule, with an explicit switch for providers that do not send the claim.

## Acceptance checklist

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `SEC-65-AC1` | With account creation on, sign in through a provider that sends `email_verified: true` for an email an existing local account has | Signed in as that account; one account, two identities | Not run |
| `SEC-65-AC2` | The same with `email_verified: false`, or with the claim missing | Sign-in refused with the sentence; the existing account is unchanged; the Audit Log has the refusal | Not run |
| `SEC-65-AC3` | Turn on "Trust this provider's emails" and repeat `AC2` | Signed in as that account | Not run |
| `SEC-65-AC4` | Sign in with an unconfirmed email that no account has | A new account with no email | Not run |
| `SEC-65-AC5` | A person who signed in through the provider before the upgrade signs in again, the provider sending no `email_verified` | Signed in as before | Not run |

## Not included

- Removing joins that already happened. An identity joined before this change stays joined; the Audit Log and the user's identities list are where to look.
- Verifying an email ourselves (a confirmation mail).
- SAML (ADR 0017).

## Asking the assistant

No. The assistant does not handle sign-in configuration.
