# Safer embed tokens

| | |
| --- | --- |
| Module | Dashboards (embedding) |
| Backlog | `SEC-12` |
| Status | Decisions signed 2026-10-10 |
| Plan | `docs/superpowers/plans/2026-10-10-sec-12-safer-embed-tokens.md` |

## Problem

A signed embed lets a customer's site show a dashboard to people who never
sign in. The customer's server signs a token with a shared secret
(`EMBED_SECRET`) and the embed page presents it. Today:

- a token's expiry is optional, so a token copied from a page source works forever;
- nothing can withdraw a token, or all tokens of a dashboard, short of changing the secret for every dashboard at once;
- when `EMBED_SECRET` is unset the API invents a secret and stores it as plain text in `console.app_kv`;
- the embed pages can be framed by any site.

## What the user can do when this is done

1. Rely on every embed token expiring: a token must say when it was issued and when it expires, and may live at most 24 hours (an administrator can lower or raise the limit).
2. Withdraw all embed tokens of a dashboard with one action in the Share dialog; they stop working within a minute.
3. Withdraw one token by pasting it, when the token carries an id.
4. List, per dashboard, the sites allowed to show its embed; any other site that frames it gets a blank frame.
5. See plainly, in the Share dialog, when signed embedding is unavailable because no secret is configured.

## Not included

- Public links (`/public/dashboard/…`): their expiry and password are `BI-19`.
- Single-chart embeds, themes, editable filters in embeds: `BI-26`.
- Rotating `EMBED_SECRET` with an overlap period.
- An embedding SDK (decided against).

## Asking the assistant

Not an assistant feature. The assistant cannot mint, revoke or configure
embeds.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | A token must carry `exp` and `iat`; lifetime (`exp − iat`) at most the configured maximum, default 24 hours | Spec *(proposed)* | Owner, 2026-10-10 |
| 2 | Tokens without them are refused at once after the upgrade; no grace setting | — | Owner, 2026-10-10 |
| 3 | Revocation takes effect within one minute | Spec *(proposed)* | Owner, 2026-10-10 |
| 4 | `EMBED_SECRET` must be set for signed embedding to exist. Unset: signed embedding is unavailable and says so; the rest of the product runs. No secret is generated or stored any more | — | Owner, 2026-10-10 |
| 5 | A dashboard whose list of allowed sites is empty cannot be framed at all | — | Owner, 2026-10-10 |
| 6 | "Withdraw all" needs to know when a token was issued, which is why `iat` is required (decision 1). "Withdraw one" needs a token id (`jti`), which stays optional: a token without one can only be withdrawn with all the others | Planner | Owner to confirm at QA |
| 7 | A clock difference of up to 60 seconds between the customer's server and ours is tolerated | Planner | Owner to confirm at QA |

## Limits to tell a customer

- **Breaking on upgrade.** Embed tokens without `iat` and `exp`, or living longer than the limit, stop working. Sign a fresh token per page view or per session.
- **Breaking on upgrade.** An embed stops rendering until the dashboard's owner lists the embedding site in the Share dialog.
- **Breaking on upgrade.** A deployment that relied on the generated secret must set `EMBED_SECRET`; until then signed embeds are unavailable.
- Withdrawing all tokens also withdraws ones issued seconds before; tokens issued afterwards work.
- The framing rule is enforced by the viewer's browser. It stops another site from showing the embed; it is not a substitute for the token.

## Acceptance checklist

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | `SEC-12-AC1`: open an embed with a token that has no `exp`, then one with no `iat`, then one living 25 hours | Each refused with the fixed message; no data | |
| 2 | Open an embed with a valid one-hour token | Renders | |
| 3 | `SEC-12-AC2`: press "Withdraw all embed tokens" in the Share dialog, reload the embed with the same token | Refused within a minute; a token signed afterwards works | |
| 4 | Withdraw one token that carries a `jti` by pasting it | That token is refused; another token for the same dashboard still works | |
| 5 | Paste a token without a `jti` to withdraw it | Told it has no id and can only be withdrawn with all the others | |
| 6 | `SEC-12-AC3`: frame the embed page from a site not on the dashboard's list | The frame is blank (blocked by the browser) | |
| 7 | Add that site to the list and reload | Renders | |
| 8 | Unset `EMBED_SECRET` and restart (operator) | The API starts; the Share dialog says signed embedding is not configured; the embed endpoint answers with the fixed "not configured" message | |
| 9 | `SEC-12-AC4`: as a role without `dashboard:write`, try to withdraw tokens or edit the allowed sites | Refused with the usual permission message | |
| 10 | Open a public link (`/public/dashboard/…`) | Unchanged | |

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` item moved to Done; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read
