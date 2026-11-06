# Deployment settings in the console

| | |
| --- | --- |
| Module | Administration & Security |
| Backlog | `ADM-6` (status), `ADM-7` (email), `ADM-8` (sign-in security), `ADM-9` (assistant key and model); `ADM-10` (backups) is later. With `REL-19` (a shorter `.env.example`) and `REL-18` |
| Status | Planner defaults, built at the owner's request (2026-11-06); the owner confirms or changes them at QA |
| Spec | [`specs/console-settings.md`](../specs/console-settings.md) |
| Plans | `docs/superpowers/plans/2026-11-06-rel-19-env-cleanup.md`, `docs/superpowers/plans/2026-11-06-adm-6-9-console-settings.md` |

## Problem

An install has 187 lines in `.env.example`. Turning on email for scheduled
reports, or giving the assistant its key, means editing a server file and
restarting the API. An administrator cannot see from the console which
features are off or why.

## What the user can do when this is done

1. As an administrator, open **Settings → Deployment** and see, for PDF
   export, email, the console address, single sign-on and the assistant,
   whether each is on, and if not, which setting is missing.
2. Open **Settings → Email**, enter the mail server, port, security, user,
   password, sender and the console's public address, save, and send a test
   email to their own address. Scheduled reports, alerts and comment
   mentions use it from the next send, with no restart.
3. Open **Settings → Sign-in** and change the failed-login limit, the
   window, the lockout, how long sign-in records are kept and the longest
   life of an embed link. The next sign-in uses the new numbers.
4. Open **Settings → Assistant**, enter the provider address, model and key,
   check it, and save. The next assistant message uses it.
5. See any value set in the server's `.env` shown as **Set by the server**,
   read-only: a team that manages its config as files keeps control.
6. Install with a `.env.example` of about 25 visible lines plus commented
   blocks for optional features; every setting is still documented in
   `docs/CONFIGURATION.md`.

## Decisions (planner defaults)

- **The server file wins.** A setting given a non-empty value in the
  environment is used as it is and shown read-only. The console's value is
  used only when the environment leaves it unset or empty.
- **Secrets are sealed and never sent back.** The email password and the
  assistant key are sealed with the installation key that already seals
  connector credentials (`CONNECTOR_SECRETS_KEY`). The console only learns
  whether one is set. Sending nothing keeps the stored one; an explicit
  "clear" removes it.
- **Only an administrator.** Every section here needs `settings:write`, to
  read as well as to change: a mail server's name and user are not for every
  signed-in person.
- **A test email goes only to the person asking**, so the button cannot be
  used to send mail to anyone else.
- **Every save is audited**, naming the fields changed, never a secret.
- **The assistant's address must be `https://`** and must not point inside
  the network (the same check connector dials use), so an administrator
  account cannot be used to make the API call an internal service.
- **Not in the console, on purpose:** start-up secrets and encryption keys,
  internal addresses and ports, memory and upload limits (they are tied to
  each container's memory cap), network allowlists (`*_ALLOWED_CIDRS`) and
  single sign-on settings (a wrong value locks everyone out). These stay in
  `.env`.
- **PDF export is on by default** (`REL-18`): `CONSOLE_INTERNAL_URL`
  defaults to the console's name on the compose network.

## Limits to tell a customer

- With more than one API process, a saved sign-in or assistant setting
  reaches the other processes within 30 seconds, not at once.
- Backups are still set in `.env` (`ADM-10`, later): the backup job runs in
  its own container, which the API does not configure.
- Single sign-on stays in `.env`; the Sign-in page shows only whether it is
  set up.

## Acceptance checklist (owner)

- [ ] A fresh install from the new `.env.example` starts, signs in, and a
      dashboard downloads as PDF without editing `.env`.
- [ ] Settings → Deployment names what is missing for each feature that is
      off.
- [ ] Email saved in the console: the test email arrives; a scheduled report
      sent now arrives; the password is not visible anywhere afterwards.
- [ ] With `SMTP_HOST` set in `.env`, the Email page shows it read-only.
- [ ] Sign-in: set the failed-login limit to 3; the fourth wrong password
      is refused as locked.
- [ ] Assistant: key entered in the console; the assistant answers.
- [ ] The audit log shows each save, with no secret in it.
