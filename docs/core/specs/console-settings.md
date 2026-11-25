# `ADM-6` to `ADM-9` Deployment settings in the console

| | |
| --- | --- |
| Backlog | `ADM-6`, `ADM-7`, `ADM-8`, `ADM-9` in [BACKLOG.md](../BACKLOG.md); `REL-19` |
| Module | Administration & Security |
| Size | M (planner's estimate) |
| Blocked by | none |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

Settings an administrator changes after install live in a 187-line server
file and need a restart. Metabase, Superset (through its config UI
extensions) and Grafana put email and authentication policy in the admin
screens; Metabase lets an environment variable lock a setting, which is the
precedence chosen here.

## Target specs

"Today" is `main` at `d2b26f45`, read from the code.

| Capability | Today | Target |
| --- | --- | --- |
| Lines in `.env.example` | 187 (139 set, 49 commented) | About 25 set, the rest in commented feature blocks or only in `docs/CONFIGURATION.md` *(proposed)* |
| Fresh install has PDF export | No (`CONSOLE_INTERNAL_URL` empty) | Yes |
| See which features are off, and why | No | One page, every feature listed with its missing setting |
| Set up email | Edit `.env`, restart the API | In the console; the next send uses it, no restart |
| Test email | None | One button, to the administrator's own address |
| Change sign-in limits | Edit `.env`, restart | In the console; the next sign-in uses it |
| Give the assistant its key | Edit `.env`, restart | In the console; the next message uses it |
| Lock a value from files | n/a | A non-empty environment value wins and is shown read-only |
| Secret readable after save | n/a | Never: not in an answer, a log or the audit trail |
| Change visible to other API processes | n/a | Within 30 s *(proposed)* |

## Benchmark

- Metabase: admin Settings → Email, with "Send test email"; any setting can
  be fixed by an `MB_*` environment variable, which then shows read-only.
- Grafana: SMTP only in `grafana.ini`, no UI; auth lockout in config. We do
  better than Grafana here and match Metabase.
