# Connector types with their own marks — Implementation Plan

**Status:** asked for by the product owner on 2026-10-05, not started.
Written by the planner (Claude Opus) for a developer agent, under the role
split in `AGENTS.md` on `main`. The planner writes no product code.

**Base:** `feat/connectors`, in `/home/hv/lakehouse`.

**Goal, in the user's words:** on the New Connector page the tiles show
generic icons (a cylinder for every database). Show each product's real
icon instead, or use another icon library.

Console only. No new dependency.

---

## 1. What is there today

`src/features/connectors/connector-type-picker.tsx` picks an icon per
*adapter*, not per product: `sql` is a cylinder, so MariaDB, MySQL, Oracle,
PostgreSQL and SQL Server look the same; `cdc` is one arrows icon for
three products; MongoDB is a generic leaf; Google Sheets a generic sheet;
Kafka a radio tower. The icons are from `lucide-react`, which carries no
brand marks (it removed the few it had).

A connector type has no id: the registry row is `{ name, adapter,
supported, docsUrl }` and `name` is the label the API serves
("PostgreSQL", "MySQL CDC", "Object storage", "SAP / ERP").

## 2. Decisions

1. **Marks come from Simple Icons, copied in, not installed.** Release
   16.34.0, licence CC0-1.0. Eight single-path marks of about 0.5 to 5 KB
   each do not justify a package of 3,700 icons, and installing a package
   in this checkout is not possible while the product owner's dev server
   runs from it. The files are in the scratchpad (section 4), fetched
   from that release; the component records the release and the licence.
2. **A mark is shown only where that set publishes it today.** That is
   the rule, and it is checkable. It gives eight products: PostgreSQL,
   MySQL, MariaDB, MongoDB, Apache Kafka, Google Sheets, SAP, MQTT.
3. **Oracle and SQL Server keep a neutral icon.** Simple Icons removed
   both (and every Microsoft mark) at the owners' request; other sets
   still carry them, but a licence on a drawing is not a permission to
   use the trademark, and this repository is public. They keep the
   cylinder. Changing this is the product owner's decision and a two-line
   change; it is not made here.
4. **Things that are not a product keep a generic icon:** object storage
   (any S3-compatible bucket, so not one vendor's mark), SFTP, REST API.
5. **A CDC type shows its database's mark** ("PostgreSQL CDC" shows the
   PostgreSQL mark). The group it sits in and its one-line summary
   already say it is change data capture.
6. **Colour.** A mark is drawn in its brand colour on the tile's neutral
   chip where that colour is legible there; where it is too dark for the
   dark theme (Apache Kafka `#231F20`, MariaDB `#003545`, MQTT `#660066`)
   it is drawn in the foreground colour in that theme. One colour only,
   never a recolour to some third colour. On the selected tile and in the
   "selected type" strip the chip is the primary colour and the mark is
   the chip's foreground, as the generic icons are now.
7. **The shape is not altered:** the path as published, scaled, nothing
   added to it.
8. **Marks are decoration beside a name that is always there**, so they
   are hidden from assistive technology, as the current icons are.

## 3. Scope

The type picker's tiles and the "selected type" strip (the create page's
later steps and the edit page). Not the connectors list, the connector
page's header, lineage nodes or the pipeline builder: one component makes
those a small follow-up, and none was asked for.

## 4. Material (not in the repository)

`/tmp/claude-1000/-home-hv-lakehouse/bd29de40-a5dd-41f1-95e0-319fb8767ef5/scratchpad/brand-icons/`:
`postgresql.svg`, `mysql.svg`, `mariadb.svg`, `mongodb.svg`,
`apachekafka.svg`, `googlesheets.svg`, `sap.svg`, `mqtt.svg` (each one
`<path>` on a 24 × 24 view box), from
`https://cdn.jsdelivr.net/npm/simple-icons@16.34.0/icons/<slug>.svg`.

| Type name served | Slug | Brand colour |
| --- | --- | --- |
| PostgreSQL, PostgreSQL CDC | `postgresql` | `#4169E1` |
| MySQL, MySQL CDC | `mysql` | `#4479A1` |
| MariaDB | `mariadb` | `#003545` |
| MongoDB | `mongodb` | `#47A248` |
| Kafka | `apachekafka` | `#231F20` |
| Google Sheets | `googlesheets` | `#34A853` |
| SAP / ERP | `sap` | `#0FAAFF` |
| MQTT | `mqtt` | `#660066` |

## 5. Tasks

### B1 — The marks and the lookup

- A module that holds the eight marks (path data, brand colour, whether
  the colour needs the foreground in the dark theme) with a header that
  says where they come from (set, release, licence), the rule of
  decision 2, why Oracle and SQL Server are absent, and that a mark is a
  trademark of its owner shown only to name the product a connector
  talks to. Put it where a later reader will look: a pattern under
  `src/components/patterns/` if it is generic (a `BrandMark`), with the
  mapping from a connector type's name in `src/lib/connectors/` (pure,
  tested).
- The lookup is by the type's `name`, exact for the product and with a
  trailing " CDC" removed first; anything else gives no mark. No fuzzy
  matching: a type called "PostgreSQL-compatible" must not get the mark
  by accident.
- **Accept:** tests for the lookup: each of the eight names; the CDC
  forms; Oracle, SQL Server, SQL Server CDC, Object storage, SFTP, REST
  API giving none; an unknown name and a near-miss giving none; names
  compared as served (no case folding).

### B2 — The tiles and the strip

- `connector-type-picker.tsx`: a tile and the strip show the type's mark
  when it has one, otherwise the adapter's icon as today (unsupported
  types with a mark, SAP and MQTT, show the mark, dimmed with the tile as
  now). One small component decides, so the tile and the strip cannot
  differ.
- Colour as decision 6. Check the contrast of each mark on the chip in
  both themes by its numbers before choosing which ones fall back, and
  say what you found; the three named in decision 6 are my reading, not a
  measurement.
- Size: the mark sits in the 32 px chip at the generic icons' size or a
  little larger if a mark reads too small (some are wide, some tall);
  one size for all.
- **Accept (component tests, in `connector-create-page.test.tsx` or a
  new test beside the picker):** a PostgreSQL tile renders the mark and
  not the cylinder; an Oracle tile renders the cylinder; a CDC tile
  renders its database's mark; the strip for a chosen type renders the
  same mark as its tile; a mark is `aria-hidden`; the selected tile's
  mark takes the chip's foreground. Existing picker tests still pass.

### B3 — Documents

- `CHANGELOG.md` `[Unreleased]`: one entry, its own bullet, naming the
  set, the release, the licence and the two products left out and why.
- `docs/CODE-STANDARD.md` or the place the repo lists third-party
  material, if there is one: grep for where another copied asset or
  licence is recorded and follow it; if there is none, the module's
  header is the record and the handoff says so.
- **Accept:** each sentence names only what exists.

## 6. Working on this machine

- Work in `/home/hv/lakehouse` on `feat/connectors`. It is the product
  owner's checkout: their console dev server (port 3000) reloads every
  saved file under `src/`. Never kill or restart it, never run `next
  build`, `bun install` or `bun add` here, and do not leave `src/` in a
  state that does not compile.
- **The tree already holds three uncommitted changes** waiting for the
  product owner, all under `src/features/catalog/`, `src/lib/column-*`,
  `src/lib/sample-grid*`, `src/services/`, `rust/` and documents. None
  is yours; leave every hunk as it is. Your CHANGELOG entry is a
  separate bullet.
- **Leave the work uncommitted.** No `git add`, commit, stash, reset,
  checkout, restore or branch switch. Never push.
- Untracked and not yours: `docs/plans/FEAT-CONNECTORS-REPORT.md`,
  `lark-import/`, `ops/g3/bronze_catalog.py`. Never read or print `.env`.
- No Rust, no `docker`, no network: the SVG files are in the scratchpad.
  No prettier: match the file's style by hand.
- Before the handoff, once, in the foreground: `bun run typecheck && bun
  run lint && bun run test`.

## 7. Acceptance checklist (product owner)

Sources, New connector:

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | Look at Databases | MariaDB, MongoDB, MySQL and PostgreSQL show their own marks; Oracle and SQL Server a cylinder | |
| 2 | Look at Change data capture | MySQL CDC and PostgreSQL CDC show their database's mark; SQL Server CDC the generic icon | |
| 3 | Look at APIs and streams, Coming soon | Google Sheets and Kafka show their marks; REST API a globe; SAP and MQTT their marks, dimmed | |
| 4 | Pick a type, go to the next step | The strip at the top shows the same mark | |
| 5 | Switch to the dark theme | Every mark is legible; none is a dark shape on a dark chip | |
| 6 | Open an existing connector, Edit | The strip shows the type's mark | |

## 8. Handoff (developer)

Left uncommitted. Not viewed in a browser (none available to the developer).

**Files**

- `src/lib/connectors/brand-marks.ts` (new): the eight marks (`paths`, `viewBox`, hex, `needsForegroundInDark`; seven from Simple Icons, MySQL from SVG Logos, see R1 below), the licence/rule header, `brandMarkForTypeName` (strip one trailing " CDC", then exact own-property match).
- `src/lib/connectors/brand-marks.test.ts` (new): lookup tests; a path-vs-SVG byte comparison that runs only when `BRAND_ICON_DIR` points at the downloaded files (it did, once, and passed).
- `src/components/patterns/brand-mark.tsx` (new): generic `BrandMark` (`tone="brand"|"inherit"`).
- `src/features/connectors/connector-type-picker.tsx` (changed): `TypeIcon` used by `TypeCard` and `SelectedTypeSummary`.
- `src/features/connectors/connector-type-picker.test.tsx` (new): 9 tests.
- `CHANGELOG.md`: one bullet under `[Unreleased]` / Added.

**Run (foreground)**: `bun run typecheck` exit 0; `bun run lint` exit 0 (0 errors, 6 warnings, none in these files); `bun run test` exit 0, 716 pass, 1 skip, 0 fail, 80 files.

**Contrast** (WCAG, brand hex against the chip `bg-muted`; tokens in `design-system/tokens/colors.css`; dark theme is the `.dark` class on `html`):

| Mark | Light | Dark |
| --- | --- | --- |
| PostgreSQL | 4.36 | 3.05 |
| MySQL | 4.20 | 3.16 |
| MariaDB | 11.83 | 1.12 |
| MongoDB | 2.89 | 4.60 |
| Kafka | 14.65 | 1.10 |
| Google Sheets | 2.75 | 4.84 |
| SAP | 2.30 | 5.78 |
| MQTT | 10.74 | 1.24 |

Foreground on chip: 17.72 light, 14.25 dark. At the 3:1 bar for graphics, Kafka, MariaDB and MQTT fall back to the foreground in the dark theme, as guessed (MySQL joins them by eye, see R2). PostgreSQL and MySQL pass in dark only just. **Finding, not acted on:** in the light theme MongoDB (2.89), Google Sheets (2.75) and SAP (2.30) are below 3:1 on the chip; the settled decision names a dark-theme fallback only, so they are drawn in brand colour. Reviewer to judge in the light screenshots.

**R1 (review fix, MySQL)**: Simple Icons' MySQL is the wordmark and unreadable at 18 px, so MySQL uses the dolphin alone: `mysql-icon` of SVG Logos by Gil Barbara, CC0-1.0, from `@iconify-json/logos` 1.2.15, two paths kept as published on `0 0 256 252`, colour still `#4479A1` (the drawing's own `#00546b` fails on the dark chip). `BrandMark` and the data now take `paths` and `viewBox`; the other seven are unchanged. Contrast 4.20 light, 3.16 dark; R2: the dolphin is a thin line drawing and 3.16:1 was too faint on the dark screenshot, so MySQL joins the foreground-in-dark set (now Kafka, MariaDB, MQTT, MySQL); light stays `#4479A1`. The byte-comparison test, run with `BRAND_ICON_DIR`, covers all eight files.

**Departures**: none from the decisions (before R1). Mark size is 18 px (generic icons stay 16 px). The dark fallback is the class `dark:text-foreground` over a `--brand-mark` custom property (inline `color` would have beaten the class).

**Third-party record**: the repo has no third-party or licence register (grep found none); `brand-marks.ts`'s header is the record, plus the CHANGELOG bullet.

## 9. Review (planner)
