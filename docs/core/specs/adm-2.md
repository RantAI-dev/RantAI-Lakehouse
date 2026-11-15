# `ADM-2` The workspace's own name and logo

| | |
| --- | --- |
| Backlog | `ADM-2` in [BACKLOG.md](../BACKLOG.md) |
| Module | Administration & Security |
| Size | S to M (planner's estimate: S = days, M = one to two weeks) |
| Blocked by | `BI-18` part D (#528) and `BI-59` (#530): both add migrations and `POLICY_TABLE` rows, and their migration numbers `0142`/`0143` already clash with `main`. `WEB-15` (#536): it redesigns the sign-in page, the sidebar header, `layout.tsx` and the embed frame, the four places this item changes |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

The console calls itself "Rantai Lake" in every installation. The name is
written into the code in `src/app/layout.tsx:31-34`, `app-sidebar.tsx:297-298`
and `login-page.tsx:206,221`. The logo and favicon are static files in
`public/`. A company that installs the product cannot show its own name
or logo to its staff without changing the code.

The backlog row first read "Workspace settings that can be changed". Most
of that is already built. The deployment settings are `ADM-6` to `ADM-12`.
Time zone, first day of the week and number format are in Settings →
Reporting (`BI-9`, migrations `0070` and `0086`). What was left is the
workspace's identity. The owner narrowed `ADM-2` to that on 2026-11-15.

## What users get

An administrator gives the workspace its own name, logo and browser-tab
icon in Settings → General. Everybody then sees them on the sign-in page,
in the sidebar and in the browser tab. The footer of an embedded dashboard
shows the name.

## Target specs

"Today" is `main` at `6da7012a`, read from the code.

| Capability | Today | Target |
| --- | --- | --- |
| Workspace name | "Rantai Lake", fixed in code | Set in Settings → General, at most 60 characters *(proposed)* |
| Logo | `public/logo-light.png` and `logo-dark.png` | Uploaded in Settings → General: one logo, plus an optional second one for dark mode |
| Browser-tab icon | `public/icon-*-32.png` | Uploaded, optional; without one, the product's icon stays |
| Logo file | n/a | PNG, JPEG or WebP, at most 512 KB *(proposed)*. The server checks the file's first bytes, not its name. SVG is refused, because an SVG can carry a script |
| Icon file | n/a | PNG, at most 64 KB *(proposed)* |
| Where the name and logo show | n/a | Sign-in page, sidebar header and browser tab title. The footer of embedded dashboards and public links shows the name, which today reads "Rantai Lake" |
| Before sign-in | n/a | The sign-in page reads the name and logo through one public route that returns only the name and whether each image is set |
| Reset | n/a | One button restores the product's own name, logo and icon |
| Who may change it | n/a | `settings:write` |
| Audit | n/a | Every save and every reset is in the audit trail, with the fields changed and the image sizes, never the image bytes |
| Change visible | n/a | On the next page load. Other API processes see it within 30 s *(proposed)*, as with `ADM-7` to `ADM-9` |

## Benchmark

The planner wrote these from knowledge of the vendors' documentation. They
were not re-checked at the source. The plan's first step checks them and
corrects this section.

- Metabase Pro and Enterprise: Admin → Appearance sets the application
  name, logo, favicon, loading message and colours. The open-source edition
  cannot do this.
- Grafana Enterprise: the title, sign-in logo, menu logo and favicon are
  set in `grafana.ini` (`[white_labeling]`), with no screen in the console.
- Superset: `APP_NAME` and `APP_ICON` in `superset_config.py`, a restart,
  no screen in the console.

The target matches Metabase's paid tier on name, logo and icon, and beats
Grafana and Superset by needing no file edit and no restart. Brand colours
are not in this item (see "Not included").

## Acceptance checklist

The product owner runs this on a running console. A step not performed is
never a pass.

- `ADM-2-AC1` In Settings → General, set the name to "Contoh Workspace" and
  save. After a reload, the sidebar header and the browser tab show it.
- `ADM-2-AC2` Upload a PNG logo. After signing out, the sign-in page shows
  that logo and the name.
- `ADM-2-AC3` Upload a dark-mode logo, then switch the console to dark. The
  dark logo shows. Remove it, and the light logo shows in dark mode too.
- `ADM-2-AC4` Upload an icon. The browser tab shows it.
- `ADM-2-AC5` Try an SVG file, a 2 MB PNG, and a text file renamed to
  `.png`. Each is refused with a message that names the rule it broke.
- `ADM-2-AC6` Open an embedded dashboard. Its footer shows the workspace's
  name, and `logo=none` still hides it.
- `ADM-2-AC7` As a user without `settings:write`, the General section is
  not offered, and a direct call to save is refused.
- `ADM-2-AC8` Press Reset. The product's name, logo and icon return.
- `ADM-2-AC9` The audit trail shows each save and the reset.

## Not included

- **Time zone and date and number format for the whole console.** They
  exist in Settings → Reporting (`BI-9`), but only dashboards use them. The
  rest of the console uses the browser's zone and an English format fixed
  in `src/lib/format.ts`. Applying them everywhere touches about 85 call
  sites and the dashboard formatting code. That is a separate item for the
  Dashboards team (owner, 2026-11-15). It is not filed yet.
- Brand colours and a custom theme.
- The console's language (translating the interface).
- The workspace name inside the assistant's prompt (`routes/ai/prompt.rs`,
  owned by the AI planner), in email templates, and in exported PDFs.
- Several organisations, each with its own name (`ADM-1`).

## Asking the assistant

Not an assistant feature. The assistant gets no tool for it.

## Before anyone builds it

This is a spec, not a plan. Its feature page is
[`features/workspace-name-and-logo.md`](../features/workspace-name-and-logo.md),
where the product owner signs the decisions and every *(proposed)* number.
The plan is written after the blockers above have merged.
