# Manage a user's roles and tenants, and switch a user off

| | |
| --- | --- |
| Module | Administration & Security |
| Backlog | `ADM-4` |
| Kind | Task in the backlog (`ADM-4`); this page is the acceptance checklist QA runs for it |
| Spec | None. The task has no target numbers beyond the decisions below |
| Status | Building |
| Priority | P1 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Who runs the acceptance checklist. Must not be the Owner. Held in the base. |
| Plan | `docs/superpowers/plans/2026-11-02-adm-4-edit-roles-and-deactivate.md` |
| Started | 2026-11-02 |
| Shipped | — |
| Evidence | The PR that delivered `ADM-4`, and this checklist's run |

## Problem

An administrator could grant roles and tenants only at invite time
(`POST /api/identity/users`). `PUT /api/identity/users/{id}/tenants/{tenant_id}`
could add a tenant afterwards, but **nothing could take a role away or take a
tenant away**.

And `app_user.status` — `'active' | 'inactive'`, in the schema since
`0001_init.sql` — was **never enforced**. An inactive user still signed in
with a password, kept working sessions, and signed in through single sign-on.
There was no way to switch somebody off when they left.

## What the user can do when this is done

1. An administrator opens a user's row, presses **Edit roles**, and ticks the
   roles the user should hold. The ticked set IS the user's whole local role
   set: a role left unticked is a role the user no longer holds.
2. A role name that does not exist is refused, and the user's roles are left
   exactly as they were.
3. An administrator presses **Remove from tenant**, and the user leaves that
   tenant. Their data, dashboards and audit history stay.
4. An administrator presses **Deactivate**, confirms, and the user is switched
   off: their password stops working, **every session they hold ends at
   once**, and single sign-on refuses them too.
5. A deactivated user is shown as **Inactive** in the users list.
6. **Reactivate** switches the same user back on, and they can sign in again.
7. A deactivated user's data is **not** deleted. Their dashboards, their
   authored work and their audit history are all still there.
8. Only an administrator who holds every permission may switch an
   unrestricted user off or on, or change the roles of one. Anyone else is
   told the user does not exist — the same answer as for a user id that does
   not exist.
9. A change that would leave the deployment with **no active administrator**
   is refused. Nobody may switch themselves off either.

## Not included

- **Deleting a user.** That would lose who authored what, so switching a
  person off is the answer, and their history stays.
- **Self-service profile editing.** A user changes their own password with
  the change-password form; everything else about an account is an
  administrator's job.
- **Syncing users from the identity provider.** Roles mapped from an
  identity provider's groups are re-resolved from the provider's claims on
  every request, and are not part of the local role set this page edits.
- **The assistant.** See below.

## Asking the assistant

No. Changing what somebody else may do is an administrative act on another
person's access; the assistant is not given a tool that grants or takes away
permissions. The assistant's own dashboard work (`AI-15`) is unrelated to the
identity directory.

## Decisions

The planner's decisions (plan D1–D7, 2026-11-02). The product owner was
informed; none is signed yet, so the defaults below are the ones in force.

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | How a role change is expressed | The request carries the user's **complete** local role set, not a delta. A role left out is a role the user no longer holds | |
| 2 | What happens to identity-provider roles | Not stored and not touched. The user keeps the union of the local roles and the roles mapped from the provider's groups, exactly as before | |
| 3 | Who may change an unrestricted user's roles, grant an unrestricted role, or switch an unrestricted user off | Only an administrator who holds every permission (`*:*`). Anyone else gets the same "not found" answer as for a user who does not exist | |
| 4 | Whether an administrator may remove their own unrestricted grant | No. A self-removal is refused | |
| 5 | What happens when a change would leave no active administrator | The change is refused with a 409, in the same transaction as the change | |
| 6 | Whether switching a user off ends their sessions | Yes, every live session of that user, at once | |
| 7 | What switching a user off does to their data | Nothing. The data, the dashboards and the audit history stay | |

## Limits to tell a customer

- A deactivated user is **not** a deleted user. Their work is still there, and
  reactivating them brings them back exactly as they were.
- Deactivating somebody ends their sessions immediately. Work in progress in
  the console is lost when their sessions end.
- Single sign-on refuses a deactivated user too, and **does not create a new
  account for them** — signing in through the identity provider cannot walk
  around the deactivation.
- Only an administrator holding every permission may switch an unrestricted
  user off or on, or change the roles of one.
- The deployment can never be left without an active administrator: the
  change that would do it is refused.
- There is no "remove this user" button. Deactivation is the answer, because
  deleting loses authorship and audit history.

## Acceptance checklist

Run on a real deployment. **Before starting:** a deployment with at least one
Platform Admin, and a second browser to sign in as the user being changed.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `ADM-4-AC1` | As Platform Admin, open Administration → Users and press the **⋮** menu on a user's row | The menu offers **Edit roles**, **Remove from tenant** and **Deactivate** | Not run |
| `ADM-4-AC2` | Press **Edit roles** | A dialog lists every role the deployment has, with the user's own roles ticked. A role left unticked is a role the user no longer holds | Not run |
| `ADM-4-AC3` | Tick another role, untick one, and press **Save roles** | The row's roles are exactly the ticked ones. The role list and the row agree | Not run |
| `ADM-4-AC4` | (operator) With a session holding `identity:write`, call `PUT /api/identity/users/<id>/roles` naming a role that does not exist | `400`, and the user's roles are unchanged | Not run |
| `ADM-4-AC5` | (operator) With a session holding `identity:write` but **not** every permission, call `PUT /api/identity/users/<platform-admin-id>/roles` | `404`, the same answer as for a user id that does not exist. The Platform Admin's roles are unchanged | Not run |
| `ADM-4-AC6` | (operator) With a session holding `identity:write` but **not** every permission, call `PUT /api/identity/users/<some-id>/roles` with the `Platform Admin` role | `404`, and the target does not get the role | Not run |
| `ADM-4-AC7` | Press **Remove from tenant** on a user who is in a tenant, and press **Remove** on that tenant | The tenant is gone from the row's tenants. The user keeps their roles, their data and their history | Not run |
| `ADM-4-AC8` | Press **Deactivate** on a user and confirm | The dialog asks first. After confirming, the row shows **Inactive** | Not run |
| `ADM-4-AC9` | While the deactivated user is signed in in the second browser, try to use the session they had | The session is refused. The user is signed out | Not run |
| `ADM-4-AC10` | Try to sign in as the deactivated user with their password | The sign-in is refused with the same answer as a wrong password — the console does not say whether the account exists | Not run |
| `ADM-4-AC11` | Sign in as the deactivated user through single sign-on | The sign-in is refused, and **no new account is created** for them | Not run |
| `ADM-4-AC12` | As Platform Admin, press **Reactivate** on the same user and confirm | The row shows **Active** again, and the user can sign in with their password | Not run |
| `ADM-4-AC13` | As Platform Admin, open the Audit Log | There is one `identity.user.roles.set` row naming the administrator and the user, with the old and the new role names; one `identity.user.tenant.remove` row; and one `identity.user.deactivate` and one `identity.user.reactivate` row | Not run |
| `ADM-4-AC14` | Sign in as a user **without** `identity:write` and open Administration → Users, then the **⋮** menu on a row | The **Invite User** button is disabled, and **Edit roles**, **Remove from tenant** and **Deactivate** are shown disabled: pressing them does nothing. The server refuses all four writes for this caller in any case | Not run |
| `ADM-4-AC15` | (operator) With a session that holds `identity:write`, call `POST /api/identity/users/<own-id>/deactivate` | Refused — the caller may not switch themselves off | Not run |
| `ADM-4-AC16` | (operator) On a deployment with exactly one active administrator, call `POST /api/identity/users/<that-administrator-id>/deactivate` | `409`. The administrator is still active, and the deployment still has one | Not run |
| `ADM-4-AC17` | Look at the deactivated user's dashboards, authored work and audit history | All of it is still there. Nothing was deleted | Not run |
| `ADM-4-AC18` | (operator) Search the API's log for the deactivated user's session tokens | No token of theirs is still usable | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md`: `ADM-4` moved to Done, with the PR in the PR column
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case
      results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read
