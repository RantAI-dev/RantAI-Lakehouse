# UX-8 / UX-9 capability inventory (read from the code at `bd853184f`, 2026-11-21)

Every user-facing capability of the Catalog list, the asset detail and Query Studio before the revamp. The revamp plan requires each item to have a place in the new design (or a named, owner-approved removal).

Two findings that change the premise: (1) the catalog and Query Studio do NOT use `src/components/patterns/data-table.tsx`. That file is imported only by governance/overview/admin screens. The catalog list and the Query Studio results grid use `src/components/data-table/data-table.tsx` (TanStack, virtualized, dnd column reorder), with the toolbar in `src/components/data-table/data-table-advanced-toolbar.tsx`. (2) Some controls in this inventory are not shown on screen, see the notes at the end.

Notation: `file:line`, all under `src/`. "perm" = permission the UI gates on. Line numbers marked ~ are approximate.

=====================================================================
## 1. Catalog list
Route: `src/app/(data)/catalog/page.tsx:11-16` renders `src/features/catalog/catalog-page.tsx` (`CatalogPage`, :109). Nav entry needs `catalog:read` (`src/components/app-shell/nav-config.ts:127`).

Page header and states
1. Title "Catalog" and description text — catalog-page.tsx:321-324
2. Dropped-tables note ("Dropped tables", per row: table link to /data/assets/{assetId}, "dropped by X", row count or "rows not counted", "N days left"/"less than a day left"; error text "The list of dropped tables could not be read."; renders nothing when empty/loading) — `src/features/catalog/dropped-tables.tsx:28-78` — perm: none in component (server-scoped)
3. Loading state: DataTableSkeleton — catalog-page.tsx:349-354
4. Error state: ErrorState with Retry (refetch); 403 renders as permission notice — catalog-page.tsx:330-337; `src/components/patterns/page-states.tsx:19-54`
5. Unsupported-tenant state: EmptyState "The catalog cannot list assets for you" + server reason — catalog-page.tsx:338-348; `src/hooks/use-infinite-table-query.ts:45-50`
6. Empty state (no rows): shared table body text "No results." — `src/components/data-table/data-table.tsx:729`; no catalog-specific empty copy
7. Denied / not-found: route-level (`PermissionState`/`NotFoundState`, page-states.tsx:56-90) via server error; no catalog-specific copy
8. Width-based hidden-column note: "X is/are hidden at this window width — widen the window, or open an asset to see everything. The filters still work." — catalog-page.tsx:432-443; breakpoints for hiding Tags <1760px, Owner <1500, Size <1280, Freshness <1100, Type <950, Layer <860 — catalog-page.tsx:205-213; hidden via visibility, not removed (DATA-24)

Legacy URL handling
9. Legacy `?q=&layer=&tier=&type=` rewritten to `search` / `filters` with router.replace — catalog-page.tsx:50-78 and 117-122 (no perm)
10. `/data` redirects to `/catalog` (`src/app/(data)/data/page.tsx`, with `catalogRedirectHref`); `/data-catalog` redirects to `/catalog` (`src/app/data-catalog/page.tsx`)

Search and filter bar
11. Search box, placeholder "Search by name, column, tag, description or owner…", aria "Search", clear button with sr-only "Clear search" — catalog-page.tsx:384; `src/components/data-table/data-table-search.tsx:23-113` (writes URL `search`, debounced; resets page)
12. Matched-on reason line under each name when a search is active — `src/features/catalog/data-explorer-columns.tsx:72-76, 119-121`
13. Namespace quick-filter row: "Namespace" label, "All", one button per namespace with asset count, aria-pressed, title = id when different from name; loading skeleton ("Loading namespaces"); error text "Could not load namespaces." + Retry; hidden when zero namespaces — `src/features/catalog/catalog-namespaces.tsx:47-112`; wiring catalog-page.tsx:246-259, 387-391 (writes `filters` namespace eq)
14. Namespace summary card when exactly one namespace is picked: name, id, description, metadata "Assets / Owner / Residency / Source engine", "All namespaces" clear link — `catalog-namespaces.tsx:119-160`; catalog-page.tsx:426-431
15. Layer quick-filter row: "All", then Raw, Bronze, Silver, Gold (Semantic excluded on purpose); aria-pressed; toggles `filters` layer — catalog-page.tsx:85, 392-425
16. Toolbar "Sort" button (aria "Sort"; picker "Sort by..."; directions "Sort A → Z", "Sort Z → A", "Sort old → new", "Sort new → old", "Sort 1 → 9", "Sort 9 → 1"; disabled when no sortable columns) — `src/components/data-table/data-table-sort-chip.tsx:55-68, 160, 381`
17. Sort editor popover (sort chips): "Manage Sort" (~481), reorder handle "Reorder sort" (234), direction select, "Remove sort by X" (301), add sort (533), delete all (543) — data-table-sort-chip.tsx:426-558
18. Toolbar "Filter" button (aria "Filter", tooltip "Filter") — `src/components/data-table/data-table-property-bar-toggle.tsx:133, 154-164`; picker "Filter by..." (:167, add-filter.tsx:237); picker lists every filterable column (Name, Namespace, Owner, Tags, Type, Layer, Tier, Freshness, Size, Certification, Asset ID — data-explorer-columns.tsx meta) plus advanced filter start (property-bar-toggle.tsx:202)
19. Filter property bar (row 2) with chips "Remove X filter" (data-table-column-filter-chip.tsx:80), "Manage X" (:140)
20. Advanced filter rules editor: "Add filter rule" (add-filter.tsx:154), "Delete filter" (:167), join operator select "Select join operator" And/Or (data-table-filter-controls.tsx:197-201, labels :70-72), field search "Search fields..." (:248), operator select (:303), "Filter rule actions" menu with Delete (:333-348), value inputs with aria labels (:446-650), boolean select (:515)
21. Filter variants: text, number, range, date, dateRange, boolean, select, multiSelect — `src/config/data-table.ts:55-70`
22. Filter operators: iLike, notILike, eq, ne, inArray, notInArray, isEmpty, isNotEmpty, lt, lte, gt, gte, isBetween, isRelativeToToday — `src/config/data-table.ts:65-80`
23. Reset button (appears only when something to clear; clears search, sort, filters and remembered state incl. localStorage) — `src/components/data-table/data-table-reset-filters.tsx:13-74`
24. Table "Settings" button (aria "Table settings", tooltip "Settings") — `src/components/data-table/data-table-settings-menu.tsx:170, 180`; menu contents:
   a. "View" submenu: per-column visibility checkboxes (stays open while toggling) — settings-menu ~184-215
   b. "Reorder" submenu: drag handle "Drag X to reorder", "Move X up", "Move X down" — settings-menu:216-314
   c. "Group" submenu (only for columns with enableGrouping: Namespace, Type, Layer, Tier, Certification): one per column, "Active" marker, "Clear grouping" — settings-menu:325-356; group headers collapse/expand (`src/components/data-table/data-table.tsx:481-523`); group counts from server `groupSummaries` (catalog-page.tsx:358)
   d. "Export CSV" with row count, disabled when zero rows — settings-menu:363-373, 131-145 (exports only rows loaded so far; file name "Catalog")
   e. "Refresh data" (refetch; spinner while refreshing) — settings-menu:377-387; catalog-page.tsx:381
   f. "Reset layout" (clears saved column layout in localStorage) — settings-menu:392-396
25. Column-header menu (per sortable/hideable/pinnable column): sort ascending / descending checkboxes (column-header.tsx:77-94), "Reset" sort (:95-103), "Pin left" / "Pin right" (:106-131), "Hide" (:133-140); the header is the sort trigger (click)
26. Column drag reorder on the header (dnd-kit, 8px threshold; keyboard Enter/Space starts drag) — `src/components/data-table/data-table.tsx:206-254, 384-388`; pinned columns not draggable (:390-393)
27. Right-pinned "actions" column kept visible while scrolling — catalog-page.tsx:302

Columns (defined in `src/features/catalog/data-explorer-columns.tsx`)
28. Name: copyable (click-to-copy via Copyable, `src/components/copyable.tsx:28`), full-name tooltip, certification badge, matched-on line; sortable, filterable (placeholder "Search name…") — columns :91-131
29. Namespace: copyable mono text; filterable; groupable — :132-151
30. Owner: text or "—"; filterable; sortable; hidden below 1500px — :152-174
31. Tags: governed-tag pills ("Governed tag" tooltip) and free tags; not sortable; filterable — :175-215
32. Type: label from ASSET_TYPE_LABEL; multiselect filter; groupable — :216-233
33. Layer: label; multiselect filter; groupable — :234-251
34. Tier: tier badge; multiselect; groupable — :252-265
35. Freshness: FreshnessIndicator (lag vs target); number filter (unit s) — :266-288
36. Size: formatBytes; number filter — :289-304
37. Certification: certification badge; multiselect ("Certified", "Deprecated"); groupable; HIDDEN by default — :305-324; catalog-page.tsx:299
38. Asset ID: copyable mono; filter only; HIDDEN by default — :325-345; catalog-page.tsx:299
39. Actions column: ⋮ button (aria "Actions for {name}") — `src/features/catalog/data-explorer-actions.tsx:74-118`; column pinned right, not sortable/hideable — data-explorer-columns.tsx:346-361

Row actions (one list in `data-explorer-actions.tsx:40-65`, rendered twice)
40. Open asset → /data/assets/{id} (`catalog-page.tsx:165-167`)
41. Copy asset ID (toast "Asset ID copied") — data-explorer-actions.tsx:51-57
42. Copy namespace (toast "Namespace copied") — data-explorer-actions.tsx:58-63
43. Same three actions in the right-click context menu — catalog-page.tsx:365-377; `data-table.tsx:357-366`
44. Row click opens the asset (catalog-page.tsx:360; data-table.tsx:345). Keyboard: no Enter/Space on catalog rows in this table (data-table.tsx:329-353 sets onClick only). The ⋮ button is the keyboard path.

Pagination / loading
45. Infinite scroll: next chunk loads when near the end (last 8 rows) — `src/components/data-table/data-table.tsx:609-632`; chunk size 50 (config/data-table.ts; comment says not exposed in UI)
46. Footer: "Showing X of Y", "· End of list", "Loading more…" with spinner — `src/components/data-table/data-table-infinite-footer.tsx:21-64`
47. Row virtualization (window virtualizer) — data-table.tsx:511 (`virtualize` defaults on)
48. Cache: 30 s stale time — `src/hooks/use-infinite-table-query.ts:37`

URL and browser state
49. URL params read/written by the table: `page`, `perPage`, `sort`, `filters`, `joinOperator`, `search`, `groupBy`, `view` — `src/components/data-table/data-table-query-keys.tsx:9-17`, `src/hooks/use-table-url-state.ts:39-61`
50. `view` param (table|list) is defined (`src/hooks/use-table-url-state.ts:68`) but nothing calls `useTableView`; no list view is reachable
51. Remembered table state per pathname in localStorage key `app:table-state:<pathname>` — `src/hooks/use-table-memory.ts:12`; column layout in `app:table-layout:<pathname>` — `src/hooks/use-table-layout.ts:11, 114`; sidebar links get the remembered query — `use-table-memory.ts` (`rememberTableHref`)
52. Debounce 300 ms for search/filter writes, throttle 50 ms — `src/hooks/use-data-table.ts` (DEBOUNCE_MS/THROTTLE_MS, ~46-47)

Not present (checked): density control, export of all rows (export is loaded rows only), keyboard shortcuts on the catalog, saved views, bulk select.

=====================================================================
## 2. Asset detail
Route: `src/app/(data)/data/assets/[assetId]/page.tsx` renders `src/features/catalog/asset-detail-page.tsx` (`AssetDetailPage`, :69). Reads `assetId` from route params; `getAsset` with keepDataOnReload (:74-78).

Page states
1. Loading: LoadingSkeleton rows=8 — asset-detail-page.tsx:114
2. Error (incl. not_found "Not found", denied → PermissionState, unavailable) with Retry — asset-detail-page.tsx:115-116; page-states.tsx:19-90

Header (asset-detail-page.tsx:119-146)
3. Eyebrow link "Catalog" → /catalog — :123
4. Title = asset name — :124
5. Certification badge — :127
6. Tier badge (tooltip tierTitle) — :128; `asset-badges.ts:36`
7. Classification badge (tooltip) — :129-132
8. Health badge (tooltip) — :133
9. "Certification" button (outline, sm) opens Certification dialog — `asset-certification.tsx:219-245`; shown only with perm governance:write (asset-detail-page.tsx:134-136)
   - Dialog "Certification": radio choices "No mark" / "Certified" / "Deprecated" with notes (asset-certification.tsx:34-45); when Deprecated: reason input placeholder "Why it is deprecated", replacement input placeholder "serving.monthly_orders" with help "Leave it empty when no table replaces this one." (~180-197); Save ("Saving…") / Cancel; toast "Certification saved" (:133); error text
10. Meaning badge (AI draft / meaning state), links to ?tab=schema — `src/features/catalog/asset-meaning-view.tsx:55-80` (no perm)
11. "Request access" button, shown only when some of catalog:write / lineage:read / query:read are missing — asset-detail-page.tsx:138-142; perm check `src/lib/access-requests` (requestableAccessPermissions)
    - Dialog "Request access" (:150-213): explanatory text; Permission select (placeholder "pick a permission", required; lists missing perms only); Reason textarea optional ("Why do you need this?"); Cancel / "Request" ("Requesting…"); success text "Request sent — it is now pending in the Approvals inbox."; error text
12. Description (asset.description) under the title — :145

Deprecation notice (asset-certification.tsx:52-96)
13. "Deprecated" notice with reason; link to replacement asset /data/assets/{replacementAssetId} (title = id); hidden when the page was opened with a table grant — asset-detail-page.tsx:147

Facts strip (asset-detail-page.tsx:223-288; MetadataList)
14. Namespace (mono) — :243
15. Type — :244
16. Layer (title layerTitle) — :246-247
17. Format — :249
18. Engine category — :250
19. Rows (registry count; "—" with tooltip when the Iceberg table is gone; measured formatting) — :251-260
20. Size — :261
21. Owner — :262
22. Residency — :263
23. Freshness indicator — :264-273
24. Updated (relative) or "—" — :274-277

Tabs (asset-detail-tabs.tsx)
25. Tab strip with icons and counts: Overview (LayoutGrid), Schema (Columns3, count = columns), Sample (Rows3), Quality (BadgeCheck, count, danger/warning tone), Access (ShieldCheck, count = policies), Lineage (GitFork, count when loaded), Activity (Activity) — :38-47, counts :58-82
26. Tab state in URL `?tab=` (overview|schema|sample|quality|access|lineage|activity); unknown → overview; overview removes the param — :49-51, 108, 120-131 (history.replaceState, no reload)

### Overview tab (asset-overview.tsx:93-365)
27. Health tile (badge + reasons list or "No health signal") — :147-165
28. Freshness tile → Activity tab; hint "N snapshots" — :166-180
29. Quality tile → Quality tab; hint "No checks configured" / "Not evaluated yet" / "Last run …"; "N rules", "passed/evaluated", failed/warning badges — :181-202
30. Governance tile → Access tab; "N masked columns · classified by rule | default level"; classification badge; "N policies" — :203-225
31. About card (see 47-53)
32. Sensitive columns card: description; list of confidential/restricted or masked columns with type, classification badge, "masked" pill; empty "No sensitive columns" — :228-258
33. Storage card: Iceberg → AssetStorage; ClickHouse → ClickHouseStorage — :260-264; see 58-65
34. Open-format publish card (gold layer only) — :267; see 66-76
35. "Use this data" card: description "A starter query for Trino|ClickHouse. Masking still applies when it runs."; code block with starter SQL; Copy button ("Copied SQL") — :283-285; "Open in Query Studio" → /query-studio?sql=…&engine=… — :287-292; if no readable table: "NO_READABLE_TABLE" sentence instead — asset-overview.tsx ~296-300; starter SQL built in `src/lib/asset-query.ts:159-168` (SELECT columns FROM table LIMIT 100; `*` above SNIPPET_MAX_COLUMNS)
36. Connections card: rows Upstream, Downstream, Dependents, Related tables (only when >0); each button → Lineage tab; zero counts muted; TableGrantNote — :305-334; counts from lineage graph
37. Recent changes card: last N entries (actor, summary, relative time); "View all" → Activity when more; empty "No changes recorded" — :336-365

### About card (asset-about.tsx)
38. Card title "About"; "Edit" button (perm catalog:write) → Edit details dialog — :267-272
39. Read-only fact list: Domain, Owner, Steward (if set), Update frequency, Unit (if set), Publisher classification (if set), Columns, Schema (version label), Tags (governed pills + tags) — :288-320
40. Sensitivity (whole table) control row: level select (public/internal/confidential/restricted) with Set/Change/Remove; aria "Sensitivity (whole table)"; perm governance:write to change; "stricter" note when a column is more sensitive — :324-345; SensitivityControls `src/features/catalog/column-sensitivity.tsx:109`
41. Edit details dialog (asset-about.tsx:56-206): title "Edit details"; Description textarea (max 4000, placeholder from registry); Owner input (max 128); Steward input (max 128, placeholder "Who to ask about this data"); Tags input (placeholder "finance, pii, monthly-report"; validation: lowercase letters, digits, "-" and "_"; max tags; help text); governed-tag selects per defined key ("None" option) when tag keys exist; Cancel / Save ("Saving…"); toast "Asset details saved"; empty field reverts to registry value

### Schema tab (asset-schema.tsx:135-204)
42. Stand-in notice when the rows are a Silver model, not the Bronze table (role=note) — `asset-stand-in.tsx:11-23`
43. Meaning card: description; "Confirm" for the table draft (perm catalog:write, `asset-column-data.ts:89`); Edit table meaning; "AI draft" pill — `asset-meaning-view.tsx:108-190`
44. Columns card (asset-columns.tsx; `ColumnsCard` :729+):
    a. Summary text ("Showing the first N of M, in table order." / "N of M match …") — :~800-810
    b. Filter box "Filter columns…" (aria "Filter columns"), shown when table has more columns than the first page size (:81) — ~:820-830
    c. Views: "Statistics" | "Meaning" (aria "Columns view", aria-pressed) — `asset-columns.tsx:731-733, 842-845`
    d. "Drafts only" checkbox (meaning view) — ~:880-890
    e. "Show" count toggle, options 25 / 50 / 100 / All, aria "Columns to show" — `COLUMN_PAGE_SIZES` :79; CountToggle `src/features/catalog/count-toggle.tsx:14-46`
    f. Rows: family icon (aria/title = family label: Text, Number, Date or time, Boolean, Nested, Other type — :178-184), column name (mono, title), Marks (masked "Pill", classification, partition, sort key, "no longer in source" for inactiveSince) — :565-600, 638-671
    g. Statistics cells (when profile loaded): top-value bar + top value with share, Nulls %, Distinct (≈), Range — :296-390, 240-265
    h. Expand row (aria-expanded button): "Most frequent values" group (per-value shares, "Other values", "Null"), "Facts about X": Type, Can be null, Nulls, Distinct, Range, Partitioned by, Sorted by, Masked for you, Classification, Description; plus Sensitivity section when editable — :474-550, 593-685
    i. Profile notes: "Profiling columns…"; "Column statistics are read from the data, so they need the query:read permission."; "No column statistics: <reason>"; "Column statistics failed to load: …" + Retry — :210-225 (perm query:read)
    j. Meaning view rows: role (select "Role of X"), meaning text, synonyms, status pill (confirmed/draft), "Edit the meaning of X", "Confirm the draft of X" (title "Confirm this draft as it stands") — asset-meaning-view.tsx:314-391; edit dialog (title, synonyms placeholder "revenue, sales, gross", role select, Save) :486-545
    k. Empty states: "No columns registered", "No draft left to review.", "No draft matches …", "No column matches …" — :~930-940
45. Column sensitivity inside expanded column (ColumnSensitivity, column-sensitivity.tsx:58-107): "Sensitivity and masking" section; Sensitivity row: Set/Change (governance:write) with level select, Save/Cancel, Remove (confirm "Remove classification"); Masked for row: list of masks, "Remove the mask X" (aria), "Add a mask" (perm policy:write); Remove-mask confirm "Remove the mask" / "Remove mask"; AddMaskForm creates a column-mask policy with roles and name (`mask_<table>_<column>_for_<roles>`) — column-sensitivity.tsx:109-436
46. System columns card (load columns: Column, Type, Nullable) when present — asset-schema.tsx:174-199
47. Schema versions card: each version with "current" pill, change list, "since <time>" / "no data written" / "recorded <time>"; empty state "No schema history available" / "No schema version recorded yet" — asset-schema.tsx:63-125

### Sample tab (asset-sample.tsx:49-225; sample-grid.tsx; sample-inspector.tsx)
48. Card "Sample rows" with description (first N rows; "Sorted by X (ascending/descending), among the N rows loaded"; "Loading N rows… showing the first …") — asset-sample.tsx:139-147
49. Button "Open in Query Studio" (→ /query-studio with starter SQL) — :126
50. Denied state: EmptyState "Sample rows need query access" (perm query:read) — :154-157
51. Error: ErrorState with Retry; larger-sample failure note "The larger sample could not be loaded. Showing the N rows the page already has." + Retry — :159-173
52. Empty: "No sample rows available" — :165
53. Grid (role=grid, aria "Sample rows"): column header buttons sort (title "Only the rows loaded are sorted, not the table"), column header pick, cell pick; NULL shown as "NULL" (title "No value (NULL)"); arrow keys move, Enter/Space pick — sample-grid.tsx:51-62, 180-232, 301-364
54. Pager: "Rows X–Y of Z", Previous / Next — asset-sample.tsx:196-202
55. Inspector pane (aria "Inspector"): Close (aria "Close inspector"; Escape closes) — sample-inspector.tsx:107-113; asset-sample.tsx:109-111
56. Inspector cell section: row/column, whole value, "Copy value" (only when a value exists; toast "Copied value") — sample-inspector.tsx:119-132; NULL and empty-text explained (:42-51)
57. Inspector column section: column facts and statistics (aria "Column X") — sample-inspector.tsx:140+

### Quality tab (asset-quality.tsx:277-452)
58. Card "Quality checks"; "Run checks" (perm governance:write; "Running…") — :358-363
59. "Add rule" (perm governance:write) → dialog "Add quality rule" (threshold, rule name, severity select) with "Add rule"/"Adding…" — :61-177, 365-368
60. "Data quality" link → /governance/data-quality — :371
61. QualitySchedulePanel (features/governance/quality-schedule, not read in detail) — :377
62. Rules table: Name, status pill (hint title), Last run ("Never" or relative), Edit (aria "Edit rule X", rule-origin only, perm governance:write) → EditRuleDialog (features/governance/quality-rule-edit-dialog, not read), Delete (aria "Delete rule X", perm governance:write) → confirm "Delete quality rule" / "Delete rule" — :241-265, 380-420
63. Empty: "No quality checks for this asset" with description listing rule kinds — :379-381

### Access tab (asset-access.tsx:400-462)
64. "Your access" card: five abilities with granted/missing and the permission token: See this asset and its schema (catalog:read); Read its rows: sample, column statistics, queries (query:read); See where it comes from and what reads it (lineage:read); Edit its description, owner and tags (catalog:write) — :36-90
65. "Policies" card: "Add policy" (perm policy:write) → dialog "Add policy" (roles checkboxes, free-text role input placeholder "Analyst, Dashboard Viewer", column checkboxes, row-filter input placeholder "region = 'ID'", "Add and enforce" or "Save draft", Cancel) — :100-312, 418-426, 422 perm check
66. Policy list rows: name linking /governance/policies?id=… (perm policy:read) — :334, 409
67. "Access rules" link → /governance/policies — :429
68. Empty: "No policy binds this asset" — :437
69. AssetGrants card "Grants" (perm access:approve; else hidden): list with Revoke (aria "Revoke X from Y") → confirm "Revoke grant"; form email input placeholder "name@company.example", permission select, submit; description "A grant gives this permission for every table, for 30 days."; ErrorState with Retry — `src/features/catalog/asset-grants.tsx:35-160`

### Lineage tab (asset-lineage.tsx, asset-lineage-map.tsx, column-lineage.tsx)
70. Gate: perm lineage:read; denied state EmptyState "Lineage needs lineage access" with "Use Request access above" — asset-lineage.tsx:42, 228-233
71. States: loading "Loading lineage" (role=status); ErrorState + Retry; "Lineage not available" + graph reason; "No lineage recorded for X" — asset-lineage.tsx:199-268
72. Lineage graph (role=list, aria "Lineage graph"): focus node, upstream/downstream nodes with captions ("Dashboard", "Charts", related), pipeline nodes, "Other charts" group; node link (non-focus nodes) — asset-lineage-map.tsx:507, 139, 237-290
73. Dashboard chart groups open/close in place (one at a time) — asset-lineage-map.tsx:237-290, 421-447
74. Pan and zoom box (drag to pan, pinch and ctrl/cmd-wheel zoom, corner buttons "+", "−", "Fit"; keys +, -, = and 0 when focused; f toggles full view; Escape closes full view; Tab handling) — asset-lineage-map.tsx:547-633, 772, 869-874
75. "Open in lineage" → /lineage?focus=<key> — asset-lineage.tsx:323-327
76. "Open in Query Studio" with starter SQL — asset-lineage.tsx:331
77. TableGrantNote (grant-admitted view) — asset-lineage.tsx:321
78. Column mappings card "Column mappings": column select (placeholder "Pick a column"; empty "Pick a column" / "Its upstream and downstream columns appear here."); loading "Loading column lineage"; ErrorState; sides "Upstream" / "Downstream" with mappings (hop number, producer label, retention line, warning pill with reason); empty texts "Nothing is recorded as feeding this column." / "Nothing is recorded as built from this column."; "Charts and dashboards" links → /dashboards?board=… ; "Charts could not be read just now." — `src/features/catalog/column-lineage.tsx:83-182`
79. Dependents card "Dependents": list linking, "No dependents recorded" — asset-lineage.tsx:359-363
80. Related tables card: "Tables the catalog ties to this one outside the recorded graph."; "No related tables" — asset-lineage.tsx:370-374

### Activity tab (asset-activity.tsx:358-612)
81. Freshness card: description ("No target is set…" or "Expected within …, by its freshness SLA | from the refresh frequency…"); "Set target" / "Change target" (perm governance:write; only for names with a dot) → FreshnessTargetDialog (remove option when SLA; not read in detail) — :258-300
82. Snapshots card: description ("One per load of an Iceberg table, newest first…", retention text, "Showing the newest N of M"); "Show" count toggle when >25 (aria "Snapshots to show"); "Drop table" (see 90); table columns: relative time, Rows after (+N or —), Snapshot id (mono), "Query this version" → /query-studio?sql=…&engine=trino (Trino only; `src/lib/asset-query.ts:196-208`) — :410-440, 160-256
83. Snapshot states: "Loading snapshots…", "No snapshots for this asset", "Only Iceberg tables keep snapshots" (ClickHouse note), ErrorState + Retry — asset-activity.tsx:166-256
84. Dropped-table state (no Iceberg table): "No Iceberg table for this asset"; "It was dropped on <date>. It can be restored until <date>."; "Restore (until <date>)" button (perm governance:write; restore confirmed → "Table restored" / "Failed to restore the table") — asset-activity.tsx:110-156
85. Change history card: description; "Show" toggle; "Showing the newest N of M"; rows with time (title full date), summary; "No changes recorded" — :454-485, 319-355
86. Usage card "Usage (Nd)": description (queries run in Query Studio or by AI Copilot; "Only your own query text is shown in full…"); states "Usage not measured" (query history could not be read) and "No Query Studio or AI Copilot query in the last N days"; stats Queries, People, Avg latency (ms); truncated note; "Frequent users" (name, query count); "Frequent queries" (SQL mono, runs, last at, "Audit" link to /audit?event=…, unparsed note); "Tables joined with it" (links to /data/assets/{id}); "Column popularity" (with note); "Not counted: reads by the N dashboards…" — asset-activity.tsx:493-612 (perm: TableGrantNote withholds usage for table-grant admits)
87. TableGrantNote: "Shown with a table grant: related tables and usage are hidden" — `src/features/catalog/table-grant-note.tsx:18-40`
88. Audit links per frequent query: /audit?event=… — asset-activity.tsx:595
89. Drop table (see also 82): button "Drop table" (outline) in Snapshots card, perm governance:write, only if server says droppable and table is an Iceberg candidate with a table name — asset-activity.tsx:376-387, 426; `src/features/catalog/asset-drop.tsx:31-142`
   - Dialog title "Drop {table}?", row count fetched (withRowCount), retention text "kept N days"/restore until date, confirm "Drop table" (pending), error text; toast "Table dropped" with "It can be restored from this page until <date>."

### Open-format publish card (gold only) — `src/features/catalog/open-format-card.tsx:197-445`
90. Title "Publish in open format (Iceberg)"; description (adds one full copy each publish; second table holds newest copy)
91. Switch (aria "Publish in open format (Iceberg)", data-testid open-format-switch); disabled without canEdit (perm gold:export — text at :276: "You do not hold the gold:export permission. Only a Platform Admin may switch publishing on or off.")
92. Publication status line (data-testid publication-status); "Last successful publish: …" / "Nothing has been published yet."; "Publishing is off; this copy is no longer updated."
93. Readers (7 days) (title reason; "Not measured" / "Outside reading is set up, but the route sent no reader list.")
94. Last published (time + snapshot) or "Could not be read"
95. Copies kept at last trim; Trim failed (reason)
96. Latest copy only: copyable table name, holds line ("The copy published … (N rows)" / "Not written yet"), latest-copy run failure sentences (per reason), "Not updated: publishing is off."
97. "Publish now" (perm gold:export; disabled while pending; shows failure text) — :148-160
98. "Last five runs" list (LastFiveRuns; "No publish runs yet.") — :38-46
99. Link "/gold-exports" — :436

### Storage card
100. Iceberg AssetStorage (asset-storage.tsx:220-340): Size, Data files, Avg file size, Snapshots, Last load, Table columns, Partitioned by, Maintenance; "Row count over snapshots" chart with bar tooltips; "Bronze lakehouse table" link button (lakehouseTableHref) — :234-237; states "Loading table metadata" (role=status), ErrorState + Retry, "No snapshot reported a row count"
101. ClickHouseStorage (asset-storage.tsx:351-395): Size on disk, Uncompressed, Parts, Last write, Engine, Table columns, Partitioned by, Sorted by, Partitions (ClickHouse view when applicable)

### Badges and helpers used on the page
102. Badge titles (tierTitle, classificationTitle, healthTitle, layerTitle) — `src/features/catalog/asset-badges.ts`

=====================================================================
## 3. Query Studio
Route: `src/app/(build)/query-studio/page.tsx` → `src/features/queries/query-studio-page.tsx:28` (`QueryStudioPage`, workspace :47). Sub-routes (each an 8-line page): `saved/page.tsx`, `history/page.tsx`, `save/page.tsx`, `save-source/page.tsx`. Nav needs `query:read` (nav-config.ts:128).

### Section nav and page header
1. Section tabs as links (aria "Query Studio sections", aria-current): "Studio" /query-studio, "Saved Queries" /query-studio/saved, "History" /query-studio/history — `src/features/queries/query-studio-tabs.tsx:8-41`
2. Header "Query Studio" + description — query-studio-page.tsx:108-111
3. `?saved=` refused/not-found alert (role=alert): "This query is not available to you." — query-studio-page.tsx:114-121; use-query-studio.ts:97, 1437-1465
4. `?run=` refused/not-found alert (role=alert): "This run is not in your history of the last 14 days." — query-studio-page.tsx:125-132; use-query-studio.ts:104, 1505-1530

### Mode tabs
5. Tabs "Natural language" | "SQL" — query-studio-page.tsx:136-143
6. The NL/SQL choice is held in the hook (`studio.tab`, setTab) — query-studio-page.tsx:137-138

### Natural language tab (nl-panel.tsx:17-85)
7. Question textarea (label "Question", aria-required; placeholder "e.g. What was revenue by region last quarter?") — :31-38
8. "Ask (agentic)" (sparkles) → agent answer; "Agent working…" while busy; disabled when empty — :40-47; hook ask() use-query-studio.ts:1334 (queryService.askQuestion)
9. "Generate SQL only" → "Generating…"; disabled when empty — :48-55; generate() use-query-studio.ts:1094
10. Helper text "Ask runs the query and explains the answer. Generate only writes the SQL." — :57-60
11. Error cards: "The agent could not answer" (agent error text); "Could not generate SQL" (error message) — :63-72
12. Agent answer card (agent-result-card.tsx:17-61): "Agent answer", answer text, a row preview (maxRows PREVIEW_ROWS), "Agent steps (N)" disclosure with each step and detail, a "Run the SQL for all of them." note when several
13. "Explanation" card for a generated SQL answer (MiniMarkdown) — nl-panel.tsx:77-82
    (Note: how generated SQL reaches the editor was not traced; the generate result is shown as an explanation.)

### SQL tab — query tabs (query-tab-bar.tsx)
14. Group "Query tabs": tab buttons named by tab (activate, setActiveTab) — :68-107
15. Rename (aria "Rename X") → inline input; Enter commits, Escape cancels, blur commits — :37-62, 83-112 (rename via studio.renameTab)
16. Close (aria "Close X") — :118-125 (closeTab; neighbour tab becomes active)
17. "Add query tab" (aria) — :129-138; limit 20 tabs, message "At most 20 query tabs are kept. Close a tab to open another." — `src/lib/query-draft.ts:55`, tab-bar :141
18. Tab names "Query N" (next free number) — use-query-studio.ts:227-242

### SQL editor (`src/components/sql-editor.tsx`, extensions in use-query-studio.ts:773-822)
19. Label "SQL" (required) — sql-panel.tsx:131
20. Placeholder "Write SQL here, or generate it from a question" — use-query-studio.ts:67
21. CodeMirror: line numbers, fold gutter, active-line highlight, autocompletion — sql-editor.tsx:~140-170 (lineNumbers, foldGutter, highlightActiveLine, autocompletion)
22. Completion source: catalog tables (`asset.id` and `namespace.name`) and columns; Trino-only Bronze names; "proof" that a table exists — use-query-studio.ts:520-566 (tables), 582-665 (proof reads), 603-660 (columns); `src/features/queries/sql-completion-source.ts`
23. Editor keymap (lib `src/lib/editor-shortcuts.ts`, shown in dialog 24):
    - Mod-Enter run query or selection (also a window-level listener, sql-panel.tsx:112-125)
    - Mod-f find and replace; Mod-d select next occurrence; Mod-Alt-g go to line; Mod-/ toggle comment
    - Ctrl-Shift-[ / Ctrl-Shift-] fold / unfold (Mac: Cmd+Alt+[ / ]); Ctrl-Alt-[ / Ctrl-Alt-] fold all / unfold all
    - Ctrl-Space complete; Mod-z undo; Mod-y redo; Mod-a select all; Shift-Mod-\ match bracket
    - fold keys at highest precedence: sql-editor.tsx:48-~70
24. "Keyboard shortcuts" button opens dialog "Keyboard shortcuts" with a definition list of EDITOR_SHORTCUTS — `src/features/queries/keyboard-shortcuts-dialog.tsx:22-55`; button sql-panel.tsx:278
25. Hint text "⌘/Ctrl+Enter to run" — sql-panel.tsx:316-318

### Query parameters ({{name}})
26. Group "Query parameters": one field per {{name}} in the SQL; fields reset per tab — query-parameter-fields.tsx:41-90; sql-panel.tsx:146-153
27. Types (menu "Edit definition of X"): Text, Number, Date, Date range, Dropdown, Multi-select — query-parameter-menu.tsx:28-35, 99-118; contracts `src/services/contracts/queries.ts:27-35`
28. Type select (label "Type"); options editor (Label placeholder "Label", Value placeholder "Value", "Remove option N"); default value — query-parameter-menu.tsx:99-190
29. Dropdown/multi-select control: placeholder "Choose", "No value" item, searchable (placeholder "Search", aria "Search options"), "No options." — query-parameter-control.tsx:256-331
30. Date range: start and end date inputs — query-parameter-control.tsx:196-219
31. Server-named invalid parameters are marked invalid — use-query-studio.ts:461-470, 1200-1204 (fieldsNamedBy)
32. Estimate waits for a missing value: "Fill in {{x}} for a pre-run estimate." (transparency panel, see 66)

### SQL toolbar (sql-panel.tsx:154-326)
33. Engine select (ClickHouse | Trino; width w-32) — :155-163 (setEngine, use-query-studio.ts:439)
34. "Run query" (or "Run all" when several statements; "Running…" while running; disabled when running or nothing runnable) — :164-171 (runQuery :1249; runSelectionOrAll :1268 runs the selection when one exists)
35. Batch limit: more than 20 statements refused before anything runs — text "At most 20 statements are run at once. Split the buffer into smaller queries and run them one after another." — sql-panel.tsx:322-326; use-query-studio.ts:1253-1254; `src/lib/sql-text.ts:127` (MAX_STATEMENTS_PER_RUN)
36. "Stop" (shown while running or downloading) — :175-191 (stopRun :1294); tooltip by engine: Trino "Stops the page waiting; the query keeps running on Trino."; ClickHouse "Ends the query on the engine and stops the page waiting."; download "Stops the download and ends the query on the engine."
37. Elapsed timer (role=timer, aria "Elapsed time") while running — :192-200
38. Save button: label "Save" (when editing a saved query the tab came from) or "Save as new"; disabled unless exactly one statement (title "Select one statement") → /query-studio/save; writes sessionStorage draft — :201-210; query-studio-page.tsx:63-72; use-query-studio.ts:1684-1691
39. "Save as SQL source" (perm dashboard:sql; hidden otherwise) → /query-studio/save-source — :211-222; query-studio-page.tsx:53, 74-77
40. "Format" (WandSparkles) — formats with settings, disabled when nothing runnable — :223-231
41. "Format settings" popover: "Keyword case" (Upper case / Lower case / As typed; aria "Keyword case"); "Indent" (No indent / 2 spaces / 4 spaces; aria "Indent") — :232-277; `src/features/queries/use-format-settings.ts` (stored settings)
42. "Keyboard shortcuts" dialog button — :278
43. "Schema" toggle (aria-pressed) opens the schema browser beside the editor — :279-289; query-studio-page.tsx:164-170
44. "Estimate" (ghost; disabled while pending, with no single statement, or on Trino; title "No pre-run estimate for Trino queries.") — :290-305
45. "Explain" (outline; "Explaining…"; disabled with no single statement) — :306-315; explainRun :1353; card in page :185-189
46. Batch error text / statement-selection hints — see 35, and titles "Select one statement"

### Schema browser (schema-browser.tsx:48-308)
47. Panel aria "Schema browser", heading "Schema", close button aria "Close schema browser" — :82-95
48. Search input "Search tables" (aria, placeholder) — :100-103
49. Rows per catalog table: expand button aria "Columns of X" (loads columns); insert table button aria "Insert <ref>" (title), disabled with reason for registry slugs, Bronze-on-ClickHouse ("Bronze tables are read with Trino: switch this tab to Trino to insert it."), proof missing/failed; "Loading columns…"; "Columns could not be loaded: …" + Retry (aria "Retry columns of X" / "Retry reading the table of X") — :144-301
50. Column insert buttons aria "Insert column X" (quoted identifier) — :274-277
51. Empty/denied: "The schema browser needs catalog access"; "The catalog shows no tables." — :116, 126
52. Insert at cursor (insertAtCursor) — use-query-studio.ts:729-752

### Time travel (time-travel-controls.tsx:23-141)
53. Group "Time travel": Namespace select (placeholder "Namespace"), Table select (placeholder "Table"), Version select (placeholder "Version"; items "date · operation", title = snapshot id), button "Query this version" (pins the version into the SQL via pinSnapshot) — :65-130
54. Note text: "One version applies to every raw table in the query." (ClickHouse) / "The version applies to the table you picked." (Trino) and retention text — :131-138
55. Supported on both engines in code (time-travel-controls.tsx:35), though the page comment says Trino is disabled with a note (query-studio-page.tsx:172-177). See the notes at the end.
56. No permission gate in this component.

### Explain, Fix with AI (sql-help-card.tsx; query-studio-page.tsx:185-234)
57. Explain card "Explanation" (text) — sql-help-card.tsx:48-61; ErrorState when explain fails — page :185-186
58. "Fix with AI" button (outline; disabled on Trino with title "fixing a Trino query is not supported yet"; busy state) — sql-help-card.tsx:23-46; shown only when the run error code is invalid_request — query-studio-page.tsx:228-234
59. "Fix with AI" result card: "No fix to apply was proposed." or "Proposed fix" diff (lines marked removed/added with sr-only labels; aria "Changes to the query") with "Accept" and "Reject" — sql-help-card.tsx:63-122; fixAccept :1397 writes the editor buffer; fixReject :1420
60. ErrorState when the fix call fails — query-studio-page.tsx:190-191

### Results (query-results-section.tsx:156-322 unless stated)
61. Multi-statement result tabs: group "Statement results", tab per statement with summary title — `src/features/queries/query-result-tabs.tsx:16-47`
62. Card "Results" with actions:
    a. "Copy as TSV" (outline; disabled when no rows or downloads off; toast "Copied the rows as TSV") — :207-216, 188-191
    b. "Download" menu (disabled when no result id, downloads off, or engine not downloadable; labels "Downloading…" while busy): CSV, TSV, Excel, Parquet — `src/features/queries/query-download-menu.tsx:16-97`
    c. Reason texts: "Downloads are switched off for your role." (settings-driven) and "Downloads run on ClickHouse only." — query-download-menu.tsx:35-74; query-results-section.tsx:204-206
    d. "View in Audit" → /audit?event=… (only when the result has an audit event) — :224-232
63. Download job (use-query-download.ts): busy/done/error; toast "The file holds N rows." or "The file was cut at N rows, which is the limit of this format."; failure "The download failed."; Stop cancels the query on the engine (`queryService.cancel`) — use-query-download.ts:23-101
64. Truncation banner: "Showing the first N rows: the limit is M rows." / "...the result passed <byte cap>." / "...the result was cut." — `src/features/queries/query-result-banner.tsx:20-45`; shown when result.truncated (:236-245)
65. Column contains filters: one "contains" input per column (placeholder "contains") and "Clear filters" — `src/features/queries/query-column-filters.tsx:18-65`; filter logic `query-grid-tools.ts:49`
66. Column statistics card (aria "Column statistics"): column select (SelectField, options = result columns), "Empty cells", "Distinct values", "Minimum", "Maximum", and a histogram titled "Value range" (numeric/bins) or "Most frequent values" — `query-column-statistics.tsx:21-162`; computation `query-grid-tools.ts:95-226`
67. Grid (components/data-table/data-table.tsx, no toolbar): header sort (asc/desc/reset via DataTableColumnHeader), each cell with an "Inspect X, row N" button (aria) — query-results-section.tsx:94-134; "The query returned no rows." / "No row matches the filters." — :252-257; NULL shown as "NULL" (:52); numbers formatted; booleans as true/false
68. Cell inspector dialog ("The whole value of the cell."; "Copy value" → toast "Value copied"; object/array values pretty-printed) — `src/features/queries/query-cell-inspector.tsx:25-82`
69. Metrics list: Rows ("N shown" when truncated), Duration, Scanned, Cost, Engine, Workload, Cache (Hit / Miss / —), Pushdowns (pills or "Not measured" / "None"), Policy obligations (pills) — query-results-section.tsx:272-308
70. Execution plan (QueryPlanPanel): stages with label and "~ bytes" estimate; "Execution plan" heading — `query-transparency-panel.tsx:91-124`, used at results :309

### Run statuses per statement (use-query-studio.ts:138-172, 1112-1248; query-studio-page.tsx:203-249)
71. Done: results card (see 61-70)
72. Error: ErrorState with Retry (re-runs) — query-studio-page.tsx:213-220; error text from server
73. Refused by permission: this and every later statement marked with the same error, engine not called — use-query-studio.ts:1206-1222
74. Invalid request (400/422): names the offending parameters; shows "Fix with AI" (see 58)
75. Not run, stopped by the person: "Stopped: you stopped the run before this statement finished." — query-studio-page.tsx:237-244
76. Not run, failure earlier in the batch: "Not run: the batch stopped at the failure before this statement."
77. Running: "Running…" (role=status) for pending/running — query-studio-page.tsx:245-249

### Execution transparency panel (query-transparency-panel.tsx:126-305, card "Execution transparency")
78. Description "Pre-run estimate."
79. States: Trino — "No pre-run estimate for Trino queries." (:149-158); empty editor — "Type SQL for a pre-run estimate." (:176); waiting for a parameter — "Fill in {{x}} for a pre-run estimate." (:172); error — "Estimate unavailable." + Retry (onRetry) (:181-187); estimating — role=status aria "Estimating" (:192); no estimate — "No pre-run estimate" + Engine (:204-207); cannot plan — "This query cannot be planned" + reason (:217)
80. Ready fields: Est. scan (bytes), Est. cost (range), Workload, Engine, Cache (Eligible / Not eligible), Freshness (indicator, or "Needs catalog access" when catalogHidden), Policy obligations, Sources (table list or "No table found in the plan") — :231-295
81. Execution plan (QueryPlanPanel) — :300
82. Estimate runs on a debounce (500 ms) and on the Estimate button; "Retry" re-runs — use-query-studio.ts:70, 930-934, 966 (estimate supported only for ClickHouse: `estimateSupported = engine === "clickhouse"`)

### Rails (query-context-lists.tsx:23-174), right column of the page (query-studio-page.tsx:251-264)
83. "Saved" card: list of saved queries (title; title tooltip = SQL; click loads SQL into a tab via loadSql, including saved parameters); "Show more (N)" / "Show less"; link "Saved" → /query-studio/saved (:23-68)
84. "History" card ("Your recent runs."): rows (SQL, tooltip SQL; click loads SQL; "Refused" label for blocked; time tooltip); "Audit" link → /audit?event=… per run; "Show more"; link "History" → /query-studio/history — :81-174
85. Empty-state copy for both rails — not read in detail

### Saved Queries (/query-studio/saved) — saved-queries-page.tsx:67-383
86. Header "Saved Queries"; "Trash" button → trash dialog (see 98); "Open Studio" button (empty state and header)
87. Search input (aria and placeholder "Search saved queries"; debounced 300 ms; server-side `q`) — :213-220, 76
88. Folder tree (`saved-query-folder-tree.tsx`): nav aria "Saved query folders"; "Folders" heading; "New folder" (perm: edit/manage on the space; shared space rules :291-298); per folder actions "Actions for X": "New folder"/"New subfolder", Rename, Move, Share (manage), Delete (edit; confirm "Delete folder") — :146-189, 322-367, 220-222
89. Folder select filter (folder param in list request) — saved-queries-page.tsx:75, 79
90. Table columns: row checkbox (aria "Select all", "Select row"); Title, Owner, Tags, Updated (sort/hide/pin via DataTableColumnHeader) — `src/features/queries/saved-query-columns.tsx:74-136`
91. Row menu "Actions for X" (`saved-query-columns.tsx:156-200+`):
    - Inspect → detail drawer "Saved query" (owner, updated, tags, "Open in Studio") — saved-queries-page.tsx:50-63, 276-290
    - Open in Studio → /query-studio?saved=<id> — saved-query-columns.tsx:167-172
    - Copy SQL — :173-180
    - Copy Query ID — :181-188
    - Duplicate — :191-194 (perm: any reader; server copies)
    - History → versions dialog: "Current" marker, "Revert to version N" confirm ("The query goes back to the content of that version…"), "Restore version N" — saved-query-history-dialog.tsx:100-180
    - Rename (perm: edit) → "Rename query", aria "Query title", Cancel / Save — saved-query-columns.tsx:~204-210; saved-query-rename-dialog.tsx:46-74
    - Move (perm: manage) → "Move" dialog with Personal space / Shared space / folders; Cancel / Move — saved-query-columns.tsx ~213-219; saved-query-move-dialog.tsx:129-187
    - Delete (perm: edit, destructive) → confirm "Delete query" ("goes to the trash and leaves this list.") — saved-queries-page.tsx:313-319
    - Share (perm: manage) → access panel: title; "People and roles with access" list; Level select per row (aria "Level of X"); Remove (aria "Remove X"); "Inherited access" list; "Add a person or role" input; "Level to give" select; "Matches" checkboxes; "Nobody has been added."; Cancel / Save — saved-query-access-panel.tsx:188-457
92. Client-side column filters on Title/Owner/Tags/Updated via DataTableAdvancedToolbar (saved-queries-page.tsx:152-168, 266) — text search is server-side only
93. Empty states: "No saved queries match." (filtered) and "No saved queries" + "Open Studio" button (:251-259); error with Retry (:248)
94. Trash dialog (saved-query-trash-dialog.tsx:28-130): "The trash is empty."; Restore per entry (perm edit; restore reloads list); Close
95. Folder dialogs (saved-query-folder-dialogs.tsx:1-105): create and rename folder; Enter submits (:82-83) — details not read further

### History (/query-studio/history) — query-history-page.tsx:111-405
96. Header "Query History" — :192
97. Status filter pills (group "Filter by status"): All, Completed, Failed, Stopped, Refused — :34-37, 197-205
98. Engine filter pills (group "Filter by engine"): All, ClickHouse, Trino — :207-216
99. "Min duration (ms)" input (aria "Minimum duration in milliseconds") — :218-223
100. "Table contains…" input (aria "Table contains") — :225-230
101. "From date" and "To date" date inputs (UTC day bounds) — :236-250; `dayBound` :56-59
102. "Clear filters" button (when any filter set) — :254-262, 178-187
103. Filters are URL params: `id`, `status`, `engine`, `minDurationMs`, `table`, `from`, `to`, `page` — `src/lib/history-query-params.ts:13-30`; read on arrival (query-history-page.tsx:51-53); written with history.replaceState (:164-177)
104. Table columns: Status (StatusBadge; "blocked" labelled "Refused"), Engine, Duration, Tables (list or "This run recorded no table list."), SQL (link to /query-studio?run=<id>, title = SQL), time (title full date) — :291-352
105. Row click opens the run in Studio (router.push studioHref; ignores modifier and non-primary click) — :314-322
106. Pagination: nav "History pages", Previous, numbered pages with gaps, Next — :364-397; `pageNumbers` :56-68
107. Empty states: "No runs match these filters" (+ "Clear filters") or "No runs in the last 14 days" (+ "Open Studio") — :271-286
108. Loading and error states — :~260-270 (LoadingSkeleton, ErrorState)
109. Window: every run of the caller in the last 14 days (page comment :106-110)

### Save query (/query-studio/save) — save-query-page.tsx:52-272
110. No draft: "Open this from Query Studio" + "Back to Query Studio" — :71-76
111. Draft read from sessionStorage key `lakehouse.saveQueryDraft` — `src/features/queries/save-draft.ts:24-25, 86-102`
112. Title: label "Title" (required), placeholder "Revenue by region", prefilled from the saved query's title or the SQL summary — :92-122, 211-217
113. Tags: placeholder "finance, gold" — :221-228 area
114. Folder select (SelectField): "Personal space" (default; no folder sent), "Shared space" (sends null), and folder list grouped by space — :11-34 area, :128-143
115. FolderSharingNote (sharing rules of the chosen folder; only for folders with edit level) — folder-sharing-note.tsx; page :41-45
116. Buttons: Save (change of a saved query) or "Save as new"; "Saving…"; "Back to Query Studio"; canSubmit requires title and SQL; error text from the save — :199-206
117. Saving clears the sessionStorage draft and remembers the tab's saved id — :185-186

### Save as SQL source (/query-studio/save-source) — save-sql-source-page.tsx:32-116
118. No draft: "Open this from Query Studio" — :44-49
119. Title (label "Title", placeholder "Materials by group and type", default SQL summary); "Save source" / "Saving…"; "Back to Query Studio"
120. On success toast "SQL source “X” saved" with "Pick it under Data source → SQL sources when building a chart."; perm dashboard:sql — :29, 69-75
121. Draft key `lakehouse.saveSourceDraft` (sessionStorage) — save-draft.ts:25

### Copilot context (query-studio-page.tsx:80-104)
122. The page publishes its SQL and first rows to the assistant as page context (suggestions: "Explain this query", "What does this result show?", "Why might this query be slow?", "Turn this into a chart on a dashboard") — not a user control on this page; included for completeness.

### Drafts and persistence
123. Per-person, per-tenant tab workspace in localStorage key `rantai-lakehouse:query-studio-draft:<user>:<tenant>` (`src/lib/query-draft.ts:31, 46-48`); flushed on unmount (query-studio-page.tsx:55-61)
124. `?sql=&engine=` link opens a tab with that SQL (asset "Open in Query Studio" links) — use-query-studio.ts:846-900
125. `?saved=<id>` opens the saved query in a tab (use-query-studio.ts:1427-1465)
126. `?run=<id>` opens that run's SQL in a tab (use-query-studio.ts:1502-1545)

=====================================================================
## Duplicated across the page (same info or action in two places)

Catalog list
- Open an asset: row click, ⋮ "Open asset", right-click "Open asset" (catalog-page.tsx:360; data-explorer-actions.tsx:45-50; catalog-page.tsx:365-377)
- Copy asset ID: ⋮ and right-click, and the Asset ID column (hidden by default) (data-explorer-actions.tsx:51-57; data-explorer-columns.tsx:325-345)
- Copy namespace: ⋮ and right-click, and the Namespace cell's copy (data-explorer-actions.tsx:58-63; data-explorer-columns.tsx:139)
- Layer filter: Layer quick-filter row and the toolbar Filter menu's Layer column (catalog-page.tsx:392-425; data-explorer-columns.tsx:234-251)
- Namespace filter: namespace quick row and the toolbar Filter menu's Namespace column (catalog-namespaces.tsx:85-110; data-explorer-columns.tsx:132-151)
- Search: toolbar search box and the per-column Name, Namespace, Owner, Tags, Asset ID text filters (catalog-page.tsx:384; data-explorer-columns.tsx:91-131)
- Namespace owner: summary card "Owner" and the Owner column (catalog-namespaces.tsx:143; data-explorer-columns.tsx:152-174)
- Certification: badge in the name cell, a hidden Certification column, and its filter (data-explorer-columns.tsx:117, 305-324)
- Reset: toolbar "Reset" (clears search, sort, filters) and Settings "Reset layout" (layout only) (data-table-reset-filters.tsx:74; data-table-settings-menu.tsx:392-396)
- Counts: namespace row count, namespace summary "Assets", and the footer "Showing X of Y" (catalog-namespaces.tsx:106, 142; data-table-infinite-footer.tsx:51)
- Width note repeats the columns hidden by width, and the column filters remain (catalog-page.tsx:432-443)

Asset detail
- Health and classification: header badges and the Overview Health and Governance tiles (asset-detail-page.tsx:127-133; asset-overview.tsx:147-225)
- Freshness: facts strip, Overview Freshness tile, Activity Freshness card (asset-detail-page.tsx:264-273; asset-overview.tsx:166-180; asset-activity.tsx:258-300)
- Owner: facts strip, About card, Edit details field (asset-detail-page.tsx:262; asset-about.tsx:289, 131)
- Rows and size: facts strip, Storage card (Size, Snapshots, row count chart) (asset-detail-page.tsx:251-261; asset-storage.tsx:278-281)
- Quality: Overview Quality tile, tab count, Quality tab (asset-overview.tsx:181-202; asset-detail-tabs.tsx:66-73)
- Sensitivity: Overview "Sensitive columns" card, Schema column marks, About "Sensitivity (whole table)", and per-column sensitivity (asset-overview.tsx:228-258; asset-schema/asset-columns.tsx Marks; asset-about.tsx:326-345; column-sensitivity.tsx)
- Lineage: Overview Connections counts, Lineage tab counts and lists (asset-overview.tsx:305-334; asset-detail-tabs.tsx:76-78; asset-lineage.tsx:359-374)
- Changes: Overview "Recent changes" and Activity "Change history" (asset-overview.tsx:336-365; asset-activity.tsx:454-485)
- Schema version: facts strip "Schema" in About, and the Schema tab "Schema versions" card (asset-about.tsx:297; asset-schema.tsx:63-125)
- Starter query: Overview "Use this data" (Copy and Open in Query Studio), Sample tab "Open in Query Studio", Lineage tab "Open in Query Studio" (asset-overview.tsx:283-292; asset-sample.tsx:126; asset-lineage.tsx:331)
- Request access: header button, the Access tab "Your access" list, and "Use Request access above" hints in the Lineage and Sample empty states (asset-detail-page.tsx:138-142; asset-access.tsx:36-90; asset-lineage.tsx:231; asset-sample.tsx:156)
- Certification: header "Certification" button and the Deprecated notice (asset-detail-page.tsx:134-136; asset-certification.tsx:52-96)
- Meaning: header Meaning badge (links to Schema tab), Schema tab Meaning card, and the Columns "Meaning" view (asset-meaning-view.tsx:55-80, 108-160; asset-columns.tsx:731-733)
- Snapshot version: Activity "Query this version" links and the Query Studio time-travel controls, which pin the same version (asset-activity.tsx:239; time-travel-controls.tsx:53-62)
- Usage vs history: Activity Usage "Frequent queries" and Query Studio History list the same runs (asset-activity.tsx:546-553; query-history-page.tsx)

Query Studio
- Run: "Run query" button, Mod-Enter in the editor, and window-level Mod-Enter (sql-panel.tsx:164-171, 112-125; sql-panel.tsx:316-318 hint)
- Estimate: toolbar "Estimate" button and the transparency panel's "Retry" both call reEstimate (sql-panel.tsx:290-305; query-transparency-panel.tsx:181-187; use-query-studio.ts:~1195)
- Execution plan: transparency panel and the Results metrics both render QueryPlanPanel (query-transparency-panel.tsx:300; query-results-section.tsx:309)
- Engine, Scanned, Cost, Workload, Cache: transparency panel (estimate) and Results metrics (actual run) show the same fields (query-transparency-panel.tsx:231-251; query-results-section.tsx:283-300); Engine also in the toolbar engine select (sql-panel.tsx:155)
- Rows: Results "Rows" metric, the truncation banner, and the table footer (query-results-section.tsx:277-282; query-result-banner.tsx:20-45)
- Saved: Saved rail card and the Saved Queries page (same list, Load vs Open) (query-context-lists.tsx:23-68; saved-queries-page.tsx)
- History: History rail card and the History page (same runs; rail loads SQL, page opens Studio) (query-context-lists.tsx:81-174; query-history-page.tsx)
- Section navigation appears three times on one screen: the Studio / Saved / History links, the Natural language / SQL tabs, and the query tab bar; results add a fourth (statement) tab row (query-studio-tabs.tsx; query-studio-page.tsx:136-143; query-tab-bar.tsx; query-result-tabs.tsx)
- Table pickers: Schema browser and Time travel's Namespace and Table selects both choose a catalog table (schema-browser.tsx; time-travel-controls.tsx:68-110)
- Copy: "Copy as TSV" (results), Download CSV/TSV (server file), and cell "Copy value" (inspector) all copy the same data (query-results-section.tsx:207-216; query-download-menu.tsx:86-92; query-cell-inspector.tsx:58)
- Audit: "View in Audit" in Results and an "Audit" link per history run (query-results-section.tsx:224-232; query-context-lists.tsx:149-154)
- Save: toolbar "Save"/"Save as new" and the Save page's Title and Folder form for the same write (sql-panel.tsx:201-210; save-query-page.tsx)
- Keyboard shortcuts: the dialog lists the same keys the editor binds (generated from one list), plus the hint "⌘/Ctrl+Enter to run" (keyboard-shortcuts-dialog.tsx; sql-panel.tsx:316-318)
- Errors: a run error shows ErrorState with Retry, and invalid parameters are also marked in the parameter fields (query-studio-page.tsx:213-220; use-query-studio.ts:461-470)

=====================================================================
## Notes, discrepancies and gaps

- Shared table: the catalog and Query Studio results use `src/components/data-table/data-table.tsx`; `src/components/patterns/data-table.tsx` (opt-in sort and pagination, Indonesian comments) is NOT used by either screen. Two different components are both named DataTable.
- Density: no density control exists in the shared table toolbar or settings (grep for density found none).
- Keyboard on rows: catalog rows are click-only in the shared table (data-table.tsx:329-353). Enter/Space row activation exists only in patterns/data-table.tsx:172-183, which these screens do not use.
- Export: CSV exports rows loaded so far, not the full query (settings-menu:131-145).
- `useTableView` / `view=table|list`: defined, no caller; no list view is reachable (use-table-url-state.ts:68).
- Time travel: code enables both ClickHouse and Trino (time-travel-controls.tsx:35), but the page comment says Trino is disabled with a note (query-studio-page.tsx:172-177). The code wins for behavior; the comment is stale or the code is unintended. Flag for the owner.
- Estimate: only ClickHouse (use-query-studio.ts:966); Trino shows the "No pre-run estimate for Trino queries." message.
- Fix with AI: Trino disabled (sql-help-card.tsx:34-43); shown only for invalid_request (query-studio-page.tsx:228).
- Downloads: the Download menu is disabled on non-ClickHouse engines ("Downloads run on ClickHouse only.", query-download-menu.tsx:74); role-based switch-off comes from server settings.
- Catalog width breakpoints are estimates per the code's own comments (catalog-page.tsx:186-215).
- Not read in detail (exist, listed by name): quality-schedule panel and EditRuleDialog (features/governance), FreshnessTargetDialog (asset-activity), folder dialogs (saved-query-folder-dialogs.tsx), sql-completion-source internals, query-studio rails' empty copy, use-format-settings storage key, saved-query detail drawer body. Exact line numbers for some dialog internals are approximate (~).
- Not verified at runtime: no tests were run; all statements are from reading source.
- Permissions observed: catalog:read (nav, page), catalog:write (About edit, meaning confirm/edit, saved-query?), query:read (nav, sample rows, column statistics, catalog-derived completion), lineage:read (lineage tab), policy:read / policy:write (policy links, mask controls), governance:write (certification, quality run/add/edit/delete, drop table, restore, freshness target, sensitivity set, About sensitivity), access:approve (grants), gold:export (publish), dashboard:sql (Save as SQL source). Query Studio run, history and saved list are gated by query:read at the nav only; per-action server checks exist but are not all visible in the client.