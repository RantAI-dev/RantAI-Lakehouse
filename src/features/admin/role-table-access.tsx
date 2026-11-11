"use client"

import * as React from "react"
import { ChevronDownIcon, ChevronRightIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useService } from "@/hooks/use-service"
import {
  assetService,
  type Asset,
  type RoleTableAccess,
  type RoleTableGrant,
} from "@/services"

/**
 * The `GOV-7` S2 section on the Edit role page: which schemas and tables the
 * role may read, saved with the role form's one Save (D8). The API that
 * stores it is `PUT /api/identity/roles/{id}/table-access`; the rule it
 * feeds is `rust/crates/lakehouse-api/src/table_access.rs`.
 *
 * The rule is a UNION over a person's roles and a role holding `*:*` reads
 * every table, so this section only decides what stays out of the role's own
 * grants. A grant may name a table the catalog does not list — a pipeline
 * creates it later — and an existing grant is always shown and kept until
 * the person removes it: the save sends the complete list, so a grant that
 * silently disappeared here would be silently revoked. A grant's table is
 * compared without regard to case, as the rule compares it (`table_access.rs`
 * `allows`), so a stored `silver.Orders` is the catalog's `orders`.
 */

/** What this section edits: the role's whole access, not a delta. */
export type TableAccessDraft = {
  scope: "all" | "listed"
  grants: RoleTableGrant[]
}

/** The catalog fields the picker reads from a list row (`GOV-7 S2 review 2 R2-S1`). */
type CatalogAsset = Pick<Asset, "id" | "tableKey">

/**
 * The schema every Bronze table lives in (`GOV-7` D9). The server addresses
 * a Bronze dataset as `bronze.<table_name>` — `ensure_asset_readable` and
 * `bronze_asset_detail_body` in `rust/crates/lakehouse-api/src/routes/
 * catalog.rs` — and the rule compares the schema part of that key. The
 * registry's `namespace` (`primer`/`sekunder`) is a catalog grouping that
 * names no schema, so it is never offered here.
 */
const BRONZE_SCHEMA = "bronze"

/**
 * One schema of the picker. `tables` are the catalog's, `unlisted` are
 * grants the catalog does not name (kept, never dropped), and
 * `unnamedAssets` counts catalog rows whose table the list does not name.
 */
export type SchemaChoice = {
  schema: string
  tables: string[]
  unlisted: string[]
  unnamedAssets: number
}

/** Whether a permission string carries the unrestricted token, as `PermissionSet::grants_unrestricted` reads it. */
export function grantsUnrestricted(permissions: string): boolean {
  return permissions.split(",").some((token) => {
    const parts = token.split(":")
    return parts.length === 2 && parts[0].trim() === "*" && parts[1].trim() === "*"
  })
}

const sameName = (a: string, b: string) => a.toLowerCase() === b.toLowerCase()

/**
 * The `<schema>.<table>` key a list row names, or `null` when it names none.
 * `GOV-7` D9: a Bronze list row carries `tableKey` (`bronze.<table_name>`);
 * a Silver or Gold list row carries none, because its `id` already is its
 * key (`silver_catalog_row`, `gold_catalog_row`). The `id` is therefore the
 * second candidate, which is also what keeps a row from an older API build —
 * one that sends no `tableKey` at all — usable.
 * `GOV-7 S2 review 2 R2-S1`.
 */
function tableKeyOf(asset: CatalogAsset): string | null {
  for (const candidate of [asset.tableKey, asset.id]) {
    const key = candidate?.trim() ?? ""
    const dot = key.indexOf(".")
    if (dot > 0 && dot < key.length - 1) return key
  }
  return null
}

const EMPTY_ASSETS: readonly CatalogAsset[] = []

const compareNames = (a: string, b: string) => a.toLowerCase().localeCompare(b.toLowerCase())

/**
 * The schemas the picker offers: the catalog's tables grouped by the schema
 * part of the `<schema>.<table>` key each row names, plus every schema and
 * table the role's grants name.
 *
 * A Silver or Gold row's `id` is its `<schema>.<table>` key, and its list
 * row carries no `tableKey` — that is why the `id` is read. A Bronze row's
 * `id` is a registry slug, and its table is named only by `tableKey` —
 * `bronze.<table_name>`, the key the server builds (`GOV-7` D9) — so such a
 * table is offered by name under `bronze`. A row whose registry records no
 * table name has no key: it is counted as unnamed under `bronze`, and no
 * grant covers it (the server hides it from a narrowed role's lists), so it
 * is never offered under an invented name.
 * `GOV-7 S2 review 2 R2-S1`.
 */
export function schemaChoices(
  assets: readonly CatalogAsset[],
  grants: readonly RoleTableGrant[]
): SchemaChoice[] {
  const byKey = new Map<string, SchemaChoice>()
  const choiceFor = (schema: string): SchemaChoice => {
    const key = schema.toLowerCase()
    const found = byKey.get(key)
    if (found) return found
    const created: SchemaChoice = { schema, tables: [], unlisted: [], unnamedAssets: 0 }
    byKey.set(key, created)
    return created
  }

  for (const asset of assets) {
    const key = tableKeyOf(asset)
    if (!key) {
      choiceFor(BRONZE_SCHEMA).unnamedAssets += 1
      continue
    }
    const dot = key.indexOf(".")
    const choice = choiceFor(key.slice(0, dot))
    const table = key.slice(dot + 1)
    if (!choice.tables.some((t) => sameName(t, table))) choice.tables.push(table)
  }

  for (const grant of grants) {
    const schema = grant.schema.trim()
    if (!schema) continue
    const choice = choiceFor(schema)
    if (grant.table === "*") continue
    if (choice.tables.some((t) => sameName(t, grant.table))) continue
    if (choice.unlisted.some((t) => sameName(t, grant.table))) continue
    choice.unlisted.push(grant.table)
  }

  for (const choice of byKey.values()) {
    choice.tables.sort(compareNames)
    choice.unlisted.sort(compareNames)
  }
  return Array.from(byKey.values()).sort((a, b) => compareNames(a.schema, b.schema))
}

const counted = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`

/**
 * The Roles list's summary of one role (`GOV-7` D8): "All tables", or
 * "N schemas, M tables". `access` is `null` when the read failed, which is
 * said as "—" rather than dressed up as "All tables" — the one answer this
 * must never invent.
 */
export function tableAccessSummary(
  access: RoleTableAccess | null,
  permissions: string
): string {
  if (!access) return "—"
  // A `*:*` role reads every table whatever its stored scope says (D4), so
  // the list says so rather than counting grants the rule ignores.
  if (access.scope === "all" || grantsUnrestricted(permissions)) return "All tables"
  const schemas = new Set(access.grants.map((grant) => grant.schema.toLowerCase()))
  const tables = access.grants.filter((grant) => grant.table !== "*").length
  return `${counted(schemas.size, "schema")}, ${counted(tables, "table")}`
}

const slug = (name: string) => name.toLowerCase().replace(/[^a-z0-9]+/g, "-")

/**
 * The section on the Edit role page. The page owns the draft (one Save for
 * the whole page); this component only edits it, and loads its own catalog
 * because a failure there must not stop the role from being saved.
 */
export function RoleTableAccessSection({
  idPrefix,
  value,
  onChange,
  accessError,
  disabledReason,
}: {
  idPrefix: string
  /** The draft; `null` while it loads, or when it could not be read. */
  value: TableAccessDraft | null
  onChange: (next: TableAccessDraft) => void
  /** The current access could not be read, so nothing here may be saved. */
  accessError?: { message: string; retry: () => void } | null
  /** Set when the role holds `*:*`: the API refuses to narrow it (D4). */
  disabledReason?: string | null
}) {
  const assetsState = useService((signal) => assetService.listAssets({}, signal), [])
  const disabled = Boolean(disabledReason)
  const [addText, setAddText] = React.useState("")
  const [addError, setAddError] = React.useState<string | null>(null)
  // A schema this role already reads opens by default, so an existing
  // grant — including one the catalog does not list — is on screen rather
  // than behind a toggle. A person who toggles holds the set from then on.
  const [toggled, setToggled] = React.useState<ReadonlySet<string> | null>(null)

  const grants = React.useMemo(() => value?.grants ?? [], [value])
  const defaultExpanded = React.useMemo(
    () => new Set(grants.map((grant) => grant.schema.toLowerCase())),
    [grants]
  )
  const expanded = toggled ?? defaultExpanded
  const assets: readonly CatalogAsset[] =
    assetsState.status === "success" ? assetsState.data : EMPTY_ASSETS
  const choices = React.useMemo(() => schemaChoices(assets, grants), [assets, grants])

  // `GOV-7` S2 review 1 S1: the rule compares a table without regard to case
  // (`table_access.rs` `allows`), so this section does too — otherwise a
  // stored `silver.Orders` is neither listed as unlisted nor ticked against
  // the catalog's `orders`, and cannot be removed here.
  const hasGrant = (schema: string, table: string) =>
    grants.some((grant) => sameName(grant.schema, schema) && sameName(grant.table, table))

  const setGrants = (next: RoleTableGrant[]) => {
    if (value) onChange({ scope: value.scope, grants: next })
  }

  function toggleWholeSchema(schema: string, checked: boolean) {
    // The schema's single-table grants are kept, not cleared: turning the
    // whole schema off again must not silently drop tables the person had
    // picked, and the engine reads `*` as covering them anyway.
    const rest = grants.filter(
      (grant) => !(sameName(grant.schema, schema) && grant.table === "*")
    )
    setGrants(checked ? [...rest, { schema, table: "*" }] : rest)
  }

  function toggleTable(schema: string, table: string, checked: boolean) {
    const rest = grants.filter(
      (grant) => !(sameName(grant.schema, schema) && sameName(grant.table, table))
    )
    setGrants(checked ? [...rest, { schema, table }] : rest)
  }

  function toggleExpanded(schema: string) {
    setToggled((previous) => {
      const next = new Set(previous ?? defaultExpanded)
      const key = schema.toLowerCase()
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })
  }

  function addGrant() {
    const typed = addText.trim()
    const dot = typed.indexOf(".")
    if (dot <= 0 || dot === typed.length - 1) {
      setAddError("Write it as schema.table, for example finance.orders.")
      return
    }
    const schema = typed.slice(0, dot).trim()
    const table = typed.slice(dot + 1).trim()
    if (!hasGrant(schema, table)) setGrants([...grants, { schema, table }])
    if (table !== "*") {
      setToggled((previous) => new Set(previous ?? defaultExpanded).add(schema.toLowerCase()))
    }
    setAddText("")
    setAddError(null)
  }

  const retry = accessError?.retry

  return (
    <section className="space-y-3" aria-labelledby={`${idPrefix}-table-access-title`}>
      <div className="space-y-1">
        <h2 id={`${idPrefix}-table-access-title`} className="text-sm font-semibold">
          Tables this role can read
        </h2>
        <p className="text-xs text-muted-foreground">
          A person reads the union of their roles&apos; tables. A table this role may not read is
          hidden everywhere and a query naming it answers the same &quot;not found&quot; as a
          missing table.
        </p>
      </div>

      {accessError ? (
        <div className="space-y-2 rounded-md border border-destructive/40 bg-destructive/5 p-3">
          <p role="alert" className="text-sm text-destructive">
            This role&apos;s tables could not be loaded: {accessError.message}
          </p>
          <p className="text-xs text-muted-foreground">
            They are left as they are: saving the role does not change them, so a stale choice
            cannot replace them.
          </p>
          {retry ? (
            <Button type="button" variant="outline" size="sm" onClick={retry}>
              Retry
            </Button>
          ) : null}
        </div>
      ) : !value ? (
        <p className="text-xs text-muted-foreground">Loading this role&apos;s tables…</p>
      ) : (
        <>
          <div role="radiogroup" aria-label="Tables this role can read" className="flex gap-4">
            <label className="flex items-center gap-2 text-sm">
              <input
                type="radio"
                name={`${idPrefix}-table-access-scope`}
                value="all"
                checked={value.scope === "all"}
                disabled={disabled}
                onChange={() => onChange({ scope: "all", grants })}
                className="size-4 accent-primary"
              />
              All tables
            </label>
            <label className="flex items-center gap-2 text-sm">
              <input
                type="radio"
                name={`${idPrefix}-table-access-scope`}
                value="listed"
                checked={value.scope === "listed"}
                disabled={disabled}
                onChange={() => onChange({ scope: "listed", grants })}
                className="size-4 accent-primary"
              />
              Only these
            </label>
          </div>

          {disabledReason ? (
            <p className="text-xs text-muted-foreground">{disabledReason}</p>
          ) : null}

          {value.scope === "listed" ? (
            <div className="space-y-2">
              {assetsState.status === "error" ? (
                <div className="space-y-2 rounded-md border border-amber-500/40 bg-amber-500/5 p-3">
                  <p className="text-sm text-amber-700 dark:text-amber-400">
                    The catalog of tables could not be loaded, so only the tables this role already
                    has are listed here. You can still keep or remove them, and add one below.
                  </p>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={assetsState.reload}
                  >
                    Retry
                  </Button>
                </div>
              ) : null}
              {assetsState.status === "loading" ? (
                <p className="text-xs text-muted-foreground">Loading the catalog of tables…</p>
              ) : null}
              {assetsState.status === "success" && choices.length === 0 ? (
                <p className="text-xs text-muted-foreground">The catalog lists no tables.</p>
              ) : null}

              <div className="space-y-2">
                {choices.map((choice) => {
                  const key = choice.schema.toLowerCase()
                  const open = expanded.has(key)
                  const whole = hasGrant(choice.schema, "*")
                  const rows: { table: string; listed: boolean }[] = [
                    ...choice.tables.map((table) => ({ table, listed: true })),
                    ...choice.unlisted.map((table) => ({ table, listed: false })),
                  ]
                  const note =
                    rows.length > 0
                      ? counted(rows.length, "table")
                      : choice.unnamedAssets > 0
                        ? "The registry records no table name for these assets, so no grant here can cover them: they stay hidden from a role limited to these tables."
                        : "The catalog lists no table here."
                  return (
                    <div
                      key={key}
                      className="rounded-md border"
                      data-testid={`schema-group-${slug(key)}`}
                    >
                      <div className="flex items-start gap-2 p-2.5">
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon-xs"
                          aria-label={`${open ? "Collapse" : "Expand"} ${choice.schema}`}
                          aria-expanded={open}
                          disabled={rows.length === 0}
                          onClick={() => toggleExpanded(choice.schema)}
                        >
                          {open ? (
                            <ChevronDownIcon aria-hidden />
                          ) : (
                            <ChevronRightIcon aria-hidden />
                          )}
                        </Button>
                        <Checkbox
                          id={`${idPrefix}-whole-${slug(key)}`}
                          aria-label={`Whole schema ${choice.schema}`}
                          checked={whole}
                          disabled={disabled}
                          onCheckedChange={(checked) =>
                            toggleWholeSchema(choice.schema, Boolean(checked))
                          }
                          className="mt-0.5"
                        />
                        <div className="min-w-0 flex-1">
                          <Label
                            htmlFor={`${idPrefix}-whole-${slug(key)}`}
                            className="font-mono text-sm"
                          >
                            {choice.schema}
                          </Label>
                          <p className="text-[11px] leading-snug text-muted-foreground">{note}</p>
                        </div>
                      </div>
                      {open && rows.length > 0 ? (
                        <div className="space-y-1.5 border-t p-2.5 pl-10">
                          {rows.map((row) => {
                            const checked = hasGrant(choice.schema, row.table)
                            return (
                              <div key={row.table} className="flex items-start gap-2">
                                <Checkbox
                                  id={`${idPrefix}-table-${slug(key)}-${slug(row.table)}`}
                                  aria-label={`${choice.schema}.${row.table}`}
                                  checked={checked || whole}
                                  disabled={disabled || whole}
                                  onCheckedChange={(next) =>
                                    toggleTable(choice.schema, row.table, Boolean(next))
                                  }
                                  className="mt-0.5"
                                />
                                <div className="grid gap-0.5">
                                  <Label
                                    htmlFor={`${idPrefix}-table-${slug(key)}-${slug(row.table)}`}
                                    className="font-mono text-xs"
                                  >
                                    {row.table}
                                  </Label>
                                  {row.listed ? null : (
                                    <span className="text-[10px] text-muted-foreground">
                                      Not in the catalog (kept until removed)
                                    </span>
                                  )}
                                  {whole ? (
                                    <span className="text-[10px] text-muted-foreground">
                                      Covered by the whole schema
                                    </span>
                                  ) : null}
                                </div>
                              </div>
                            )
                          })}
                        </div>
                      ) : null}
                    </div>
                  )
                })}
              </div>

              <form
                className="flex items-end gap-2 border-t pt-3"
                onSubmit={(event) => {
                  event.preventDefault()
                  addGrant()
                }}
              >
                <div className="flex-1 space-y-1.5">
                  <Label htmlFor={`${idPrefix}-table-access-add`}>
                    Add a table not yet created
                  </Label>
                  <Input
                    id={`${idPrefix}-table-access-add`}
                    placeholder="schema.table"
                    value={addText}
                    disabled={disabled}
                    onChange={(event) => {
                      setAddText(event.target.value)
                      setAddError(null)
                    }}
                  />
                  <p className="text-[11px] leading-snug text-muted-foreground">
                    A grant may name a table a pipeline creates later.
                  </p>
                </div>
                <Button
                  type="submit"
                  variant="outline"
                  size="sm"
                  disabled={disabled || !addText.trim()}
                >
                  Add
                </Button>
              </form>
              {addError ? (
                <p role="alert" className="text-sm text-destructive">
                  {addError}
                </p>
              ) : null}
            </div>
          ) : null}
        </>
      )}
    </section>
  )
}
