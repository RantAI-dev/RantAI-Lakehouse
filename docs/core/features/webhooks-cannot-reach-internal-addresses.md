# Alert webhooks cannot reach internal addresses

| | |
| --- | --- |
| Module | Alerts |
| Backlog | `SEC-10` |
| Status | Decisions signed 2026-10-10 |
| Plan | `docs/superpowers/plans/2026-10-10-sec-10-webhook-internal-addresses.md` |

## Problem

An alert or digest sends its message to a webhook URL the user typed. The
only check is that the URL starts with `http://` or `https://`
(`lakehouse-notify`, `send_webhook`; `lakehouse-alerts`, rule
normalisation). So a user who may create alerts can make the server call
any address it can reach: a service inside the customer's network, the
cloud metadata address, the server itself. Redirects are followed, so a
public URL can also bounce the call inward. The connector probe already
refuses such targets (`connector_probe::resolve_checked`, `SEC-15`); the
webhook sender does not use it.

## What the user can do when this is done

1. Keep sending alerts and digests to public webhooks (Slack, Discord, Teams, their own endpoint) exactly as before.
2. Be told at once, with a plain message, when a webhook URL points at an internal, loopback or link-local address, when saving the rule and when testing it.
3. As an administrator, list the internal networks webhooks may reach (`WEBHOOK_ALLOWED_CIDRS`), for an on-premises chat server or an internal incident tool.
4. See a failed delivery recorded with a fixed reason, not the network library's text.

## Not included

- Email delivery (SMTP) and other outbound calls; each has its own item.
- A per-rule or per-user allowlist. The allowlist is one deployment setting.
- Retrying or queueing failed deliveries.

## Asking the assistant

The assistant's alert tools create and edit rules through the same
validation, so a webhook URL it is asked to set is refused the same way,
with the same message.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Refused by default: loopback, private, link-local, carrier-grade NAT, multicast, reserved and the wrapped forms, the same set `SEC-15` uses | Spec | 2026-10-10 |
| 2 | Redirects are not followed | Spec ("not followed, or every hop re-checked"); not following is the simpler guarantee | Planner default |
| 3 | The request goes to the address that passed the check | Spec | 2026-10-10 |
| 4 | An allowlist setting for internal webhook targets, empty by default. It is its own setting, `WEBHOOK_ALLOWED_CIDRS`, in the format of the connector allowlist but not shared with it and with no "allow everything" switch: what a connector may dial says nothing about where an alert may send data | Spec *(proposed)*; the owner approved the planner's proposals for phase 0 on 2026-10-10 | Planner default, owner to confirm at QA |
| 6 | Deliveries do not go through a system proxy (`HTTPS_PROXY`): a proxy resolves the name itself, so the address check could not be enforced | Developer's finding, accepted by the planner as the safe default | Owner to confirm at QA |
| 7 | A delivery times out after 10 seconds (5 to connect); there was no limit before | Developer's choice, accepted | Owner to confirm at QA |
| 5 | The check runs when a rule is saved or tested and again at every send | Planner default: a name can resolve differently later | 2026-10-10 |

## Limits to tell a customer

- A webhook that answers with a redirect is treated as failed; give the final URL.
- An internal chat or incident tool works only after an administrator lists its host in the allowlist setting.
- A deployment that can only reach the internet through a proxy cannot deliver webhooks to public services. This needs its own decision before such a customer relies on webhooks.
- A webhook that does not answer within 10 seconds is recorded as timed out.
- "Run now" on a rule reports a refused webhook in that rule's result, with the fixed message; only saving a rule answers with an error.
- The check is on the address the name resolves to at send time. A name that stops resolving, or starts resolving inward, fails from then on.

## Acceptance checklist

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | `SEC-10-AC1`: save an alert with webhook `http://127.0.0.1:8080/x`, then one in `10.0.0.0/8`, then `http://169.254.169.254/` | Each refused with the fixed message; nothing is sent | |
| 2 | Save an alert with a webhook whose host name resolves to a private address (operator: a test DNS name) | Refused the same way | |
| 3 | `SEC-10-AC2`: point a webhook at a public URL that answers 302 to an internal address (operator) | Delivery fails with "redirect not followed"; the internal address receives nothing | |
| 4 | Send a test to a real public webhook | Delivered, as before | |
| 5 | Add an internal host to the allowlist setting, restart, and send to it (operator) | Delivered | |
| 6 | `SEC-10-AC3`: as a role without the alerts permission, create an alert | Refused with the usual permission message | |
| 7 | Look at a failed delivery in the alert's history | A fixed reason; no library or network text | |
| 8 | Ask the assistant to set an alert's webhook to an internal address | It reports the same refusal | |

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` item moved to Done; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read
