# The workspace's own name and logo

| | |
| --- | --- |
| Module | Administration & Security |
| Backlog | `ADM-2` |
| Status | Decisions signed by the product owner, 2026-11-15, as the planner proposed them |
| Spec | [`specs/adm-2.md`](../specs/adm-2.md) |
| Plan | plan `adm-112` (slice A: the API and Settings → General, 562), plan `adm-113` (slice B: the console shows the name and logo, #570) and plan `adm-119` (the public link page and the navbar title fallback, #572). `BI-18` part D (#528), `BI-59` (#530) and `WEB-15` (#536) change the same console files and merge later, so this page's text is read again when they land |

## Problem

Every installation calls itself "Rantai Lake". A company cannot show its
own name or logo to its staff on the sign-in page, in the sidebar or in a
dashboard it embeds elsewhere.

## What the user can do when this is done

1. As an administrator, open **Settings → General**, type the workspace's
   name, upload a logo, and optionally upload a second logo for dark mode
   and a browser-tab icon. Then save.
2. Everyone sees that name and logo on the sign-in page, in the sidebar
   header, in the browser tab, on a dashboard's public link page, and as
   the footer of embedded dashboards, from their next page load.
3. Press **Reset** to go back to the product's own name, logo and icon.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | The section is **Settings → General** and needs `settings:write`, the same permission as the other administrator sections | Settings → General | Owner, 2026-11-15 |
| 2 | The images are stored in Postgres as `bytea` in a one-row table, as uploaded maps are (`0100_geo_map.sql`). They are not stored in object storage, so a backup of Postgres keeps them | Postgres `bytea` | Owner, 2026-11-15 |
| 3 | Accepted logo files are PNG, JPEG and WebP, at most 512 KB. The icon is PNG only, at most 64 KB. The server checks the first bytes of the file. SVG is refused, because a browser that opens an SVG directly runs any script inside it | 512 KB and 64 KB *(proposed)* | Owner, 2026-11-15 |
| 4 | Images are served with their stored type, `X-Content-Type-Options: nosniff` and a `Content-Security-Policy` that allows nothing to run | As stated | Owner, 2026-11-15 |
| 5 | The name is at most 60 characters, plain text, and never rendered as HTML. An empty name means the product's own name | 60 *(proposed)* | Owner, 2026-11-15 |
| 6 | The sign-in page needs the name and logo before anyone signs in. One public route returns the name and whether each image is set, and the images have their own public routes. Nothing else from the settings is public. Both routes are written in `POLICY_TABLE` as public, with the reason | Public, as stated | Owner, 2026-11-15 |
| 7 | There is **no** environment variable for the name or logo. Only the console sets them. The `ADM-7` rule that the environment wins is not needed here, because nothing in the name or logo is a secret or a deployment fact | Console only | Owner, 2026-11-15 |
| 8 | The sign-in page keeps a small "Powered by RantAI Lakehouse" line under the form. Metabase's paid tier lets a customer remove it. Keeping it is a product choice for the owner | Keep the line | Owner, 2026-11-15 |
| 9 | The one-line footer of an embedded dashboard reads the workspace's name in place of "Rantai Lake" (`features/embedding.md`, decisions 4 and 13). On a public link it stays a link to the console. `logo=none` still hides it | As stated | Owner, 2026-11-15 |
| 10 | Other API processes see a change within 30 s, through the existing settings cache | 30 s *(proposed)* | Owner, 2026-11-15 |
| 11 | Every save and reset is audited, with the fields changed and the image sizes, never the bytes | As stated | Owner, 2026-11-15 |
| 12 | The public link page (`/public/dashboard/[token]`) shows the workspace's name and logo in its header, through `useBranding()`, with the same fallbacks as slice B: the product's own name and logo when the workspace set none, the dark logo in dark mode and the light one otherwise, and a picture that fails to load falls back. Its footer reads `PRODUCT_BYLINE` ("Powered by RantAI Lakehouse"), the same small line as the sign-in page; the old "Powered by Rantai Lake — Enterprise Lakehouse Console" goes. The name renders as text, never HTML. The page is public, so it reads only the public `GET /api/branding` | As stated | Owner, 2026-11-17 |
| 13 | `pageTitleFor` returns "Rantai Lake" for a path no menu item matches; it falls back to the workspace's name instead (the product's own when none is set). It stays a pure function: its caller passes the fallback from `useBranding()`, and no hook is called inside it | As stated | Owner, 2026-11-17 |

## Limits to tell a customer

- There is one name and one logo per installation. Several organisations
  in one installation, each with its own name, is `ADM-1`, later.
- Brand colours, the interface language, and the name in emails, PDFs and
  the assistant's answers are not changed by this setting.
- Time zone and number format are set in Settings → Reporting and apply to
  dashboards only, not to the rest of the console.

## Acceptance checklist (owner)

The checklist is in the spec, `ADM-2-AC1` to `ADM-2-AC10`. It covers the
whole item, and both slices are merged, so the owner can run it now.
Every step reads "Not run" until the owner runs it.
