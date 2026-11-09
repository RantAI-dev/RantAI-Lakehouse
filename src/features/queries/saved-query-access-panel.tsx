"use client"

import * as React from "react"
import { Share2Icon, UserPlusIcon, XIcon } from "lucide-react"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { levelAtLeast } from "@/lib/access-level"
import { useService, useServiceAction } from "@/hooks/use-service"
import { readableErrorMessage } from "@/lib/notify-message"
import { queryService } from "@/services"
import type {
  AccessRuleInput,
  GranteeList,
  SavedQueryAccessList,
  SavedQueryLevel,
} from "@/services/contracts/queries"
import { toServiceError } from "@/services/errors"

/** The levels a rule may carry, from the weakest to the strongest. */
const LEVELS: readonly SavedQueryLevel[] = ["view", "edit", "manage"]

/**
 * What the panel shows for a level word (an unknown word is shown as it is).
 *
 * The console's own words, copied from the dashboard's people panel: the
 * panel is a sibling of it, not a copy of it, because that one is bound to
 * `dashboardService`.
 */
function savedQueryLevelLabel(level: string): string {
  if (level === "view") return "Can view"
  if (level === "edit") return "Can edit"
  if (level === "manage") return "Can manage"
  return level
}

/**
 * One rule as the panel edits it: exactly one grantee and one level.
 *
 * The server sends a rule's own id, who granted it and when; the panel
 * edits the two things a rule is made of — who it names and what it gives
 * — and the server writes the rest again.
 */
type WorkingRule = {
  /** The person the rule names; `null` when it names a role. */
  readonly userId: string | null
  /** The role the rule names; `null` when it names a person. */
  readonly roleId: string | null
  /** The grantee's name, as the server sent it; `null` when it found none. */
  readonly granteeName: string | null
  /** The person's email; `null` for a role and when the server found none. */
  readonly granteeEmail: string | null
  /** What the rule gives. */
  readonly level: string
}

/** The grantee a rule names: the ids a write needs and the words a reader needs. */
type Grantee = Pick<
  WorkingRule,
  "userId" | "roleId" | "granteeName" | "granteeEmail"
>

/**
 * The key of a rule.
 *
 * A person and a role are two different grantees, even when they carry the
 * same id, so the key says which one it is.
 */
function ruleKey(rule: { userId: string | null; roleId: string | null }): string {
  return rule.roleId ? `role:${rule.roleId}` : `user:${rule.userId ?? ""}`
}

/** What the panel calls a grantee the server found no name for. */
function unknownName(rule: { roleId: string | null }): string {
  return rule.roleId ? "Unknown role" : "Unknown person"
}

/** The id of the grantee a rule names. */
function granteeId(rule: { userId: string | null; roleId: string | null }): string {
  return rule.roleId ?? rule.userId ?? ""
}

/**
 * What the panel calls a rule in a sentence or a label: the grantee's name,
 * with "(role)" after a role's name.
 *
 * The server sends the name; when it found none the panel says so and adds
 * the id, so two unknown grantees stay apart. It never makes a name up.
 */
function ruleLabel(rule: Grantee): string {
  const name = rule.granteeName
  if (name === null) return `${unknownName(rule)} (${granteeId(rule)})`
  return rule.roleId ? `${name} (role)` : name
}

/** A rule's grantee in a row: the name, and under it the email or the id. */
function GranteeText({ rule }: { readonly rule: Grantee }) {
  const name = rule.granteeName
  return (
    <span className="min-w-0 flex-1 truncate">
      <span>{name ?? unknownName(rule)}</span>
      {name !== null && rule.roleId ? (
        <span className="text-muted-foreground"> (role)</span>
      ) : null}
      {name === null ? (
        <span className="block text-xs text-muted-foreground">{granteeId(rule)}</span>
      ) : rule.granteeEmail ? (
        <span className="block text-xs text-muted-foreground">{rule.granteeEmail}</span>
      ) : null}
    </span>
  )
}

/** The rules of an access list, as the panel edits them. */
function workingRules(rules: SavedQueryAccessList["rules"]): WorkingRule[] {
  return rules.map((rule) => ({
    userId: rule.userId,
    roleId: rule.roleId,
    granteeName: rule.granteeName,
    granteeEmail: rule.granteeEmail,
    level: rule.level,
  }))
}

/**
 * The levels the caller may give: the ones it holds itself.
 *
 * The panel opens only where the caller holds **manage**, so a level the
 * panel cannot read yet — the list is still loading — offers every level
 * rather than none.
 */
function levelsFor(yourLevel: string): SavedQueryLevel[] {
  if (!levelAtLeast(yourLevel, "view")) return [...LEVELS]
  return LEVELS.filter((level) => levelAtLeast(yourLevel, level))
}

/**
 * The levels a rule may be given: the ones the caller holds, and the one
 * the rule already carries (a level this console cannot read stays as it
 * is, so a write never drops it).
 */
function levelChoices(yourLevel: string, current: string): string[] {
  const allowed = levelsFor(yourLevel)
  return allowed.includes(current as SavedQueryLevel)
    ? allowed
    : [...allowed, current]
}

/**
 * Who may open one saved query or one folder (`QS-3` C3).
 *
 * The panel lists the item's own rules — the people and the roles, with
 * the level each holds — and the levels it inherits from the folders
 * above it, in a group of their own: an inherited rule is not one of the
 * item's own, and it cannot be changed here.
 *
 * The picker searches the people and the roles of the caller's
 * organisation and adds one of them with a level. Every change — a rule
 * removed, a level changed, a grantee added — goes in **one** write: the
 * server replaces the whole rule list, so the panel sends the complete
 * list it wants the item to carry, and draws the answer the server sends
 * back.
 *
 * A refusal is the server's sentence, shown as it was given; the console
 * writes none of its own (C6). The one line about the shared space is
 * explanation, not a refusal: it says what the spaces mean (P1).
 */
export function SavedQueryAccessPanel({
  kind,
  id,
  title,
  onClose,
}: {
  /** What the item is: a saved query or a folder. */
  readonly kind: "query" | "folder"
  /** The item's id. */
  readonly id: string
  /** What the dialog names the item by. */
  readonly title: string
  readonly onClose: () => void
}) {
  // The panel opens only where the caller holds manage and may change the
  // rules. The folder's list is also readable at edit (so the save and move
  // dialogs can say who a folder is shared with), but editing stays here.
  const access = useService(
    (signal) =>
      kind === "query"
        ? queryService.getSavedAccess(id, signal)
        : queryService.getFolderAccess(id, signal),
    [kind, id],
  )
  const save = useServiceAction((signal, rules: AccessRuleInput[]) =>
    kind === "query"
      ? queryService.putSavedAccess(id, { rules }, signal)
      : queryService.putFolderAccess(id, { rules }, signal),
  )
  // The list the panel draws: the answer of the read, and then the answer
  // of every write (the server answers with the list as it is after it).
  const [list, setList] = React.useState<SavedQueryAccessList | null>(null)
  const [rows, setRows] = React.useState<WorkingRule[]>([])
  const [query, setQuery] = React.useState("")
  const [givenLevel, setGivenLevel] = React.useState<SavedQueryLevel>("view")
  const [matches, setMatches] = React.useState<GranteeList>({
    people: [],
    roles: [],
  })
  const [searchError, setSearchError] = React.useState<Error | null>(null)

  React.useEffect(() => {
    if (access.status !== "success") return
    setList(access.data)
    setRows(workingRules(access.data.rules))
  }, [access.status, access.data])

  // The grantee search is the server's (it knows the people and the roles
  // of the organisation); the console sends the text typed and shows the
  // answer as it comes.
  React.useEffect(() => {
    const text = query.trim()
    if (!text) {
      setMatches({ people: [], roles: [] })
      return
    }
    const controller = new AbortController()
    const timer = setTimeout(() => {
      queryService
        .findGrantees(text, controller.signal)
        .then((found) => {
          if (controller.signal.aborted) return
          setSearchError(null)
          setMatches(found)
        })
        .catch((error: unknown) => {
          if (controller.signal.aborted) return
          setMatches({ people: [], roles: [] })
          // A refusal is the server's sentence (C6): the shared narrowing
          // helper keeps it and falls back to the repo's own sentence for
          // a thrown value that is not an error (plan 47 review fix
          // (SHOULD-FIX)).
          setSearchError(toServiceError(error))
        })
    }, 250)
    return () => {
      clearTimeout(timer)
      controller.abort()
    }
  }, [query])

  const busy = save.status === "pending"
  const inherited = list?.inherited ?? []
  const yourLevel = list?.yourLevel ?? ""
  const refusal = save.error ?? access.error ?? searchError

  function add(grantee: Grantee) {
    setRows((current) =>
      current.some((rule) => ruleKey(rule) === ruleKey(grantee))
        ? current
        : [...current, { ...grantee, level: givenLevel }],
    )
    setQuery("")
    setMatches({ people: [], roles: [] })
  }

  function remove(rule: WorkingRule) {
    setRows((current) => current.filter((row) => ruleKey(row) !== ruleKey(rule)))
  }

  function changeLevel(rule: WorkingRule, level: string) {
    setRows((current) =>
      current.map((row) =>
        ruleKey(row) === ruleKey(rule) ? { ...row, level } : row,
      ),
    )
  }

  async function write() {
    const rules: AccessRuleInput[] = rows.map((row) =>
      row.roleId
        ? { roleId: row.roleId, level: row.level as SavedQueryLevel }
        : { userId: row.userId ?? "", level: row.level as SavedQueryLevel },
    )
    const written = await save.run(rules)
    if (!written) return
    // The answer is the list as it is after the write, so the panel draws
    // it rather than the list it held.
    setList(written)
    setRows(workingRules(written.rules))
  }

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Share2Icon className="size-4" aria-hidden />
            Share {kind === "query" ? "query" : "folder"}
          </DialogTitle>
          <DialogDescription>Who may open &quot;{title}&quot;.</DialogDescription>
        </DialogHeader>
        {/* What the spaces mean (P1): the shared space is open to the whole
            organisation at view, so a rule there only ever raises a level.
            This is the console's own explanation, not a refusal (C6). */}
        <p className="text-sm text-muted-foreground">
          In the shared space everyone in the organisation can already view.
        </p>
        {access.status === "loading" ? (
          <p className="text-sm text-muted-foreground">Loading…</p>
        ) : null}
        {refusal ? (
          <p role="alert" className="text-sm text-destructive">
            {readableErrorMessage(refusal.message)}
          </p>
        ) : null}
        <div className="space-y-1">
          <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
            On this item
          </p>
          <ul className="space-y-1" aria-label="People and roles with access">
            {list && rows.length === 0 ? (
              <li className="text-sm text-muted-foreground">Nobody has been added.</li>
            ) : null}
            {rows.map((row) => (
              <li key={ruleKey(row)} className="flex items-center gap-2 text-sm">
                <GranteeText rule={row} />
                <select
                  aria-label={`Level of ${ruleLabel(row)}`}
                  value={row.level}
                  disabled={busy}
                  onChange={(event) => changeLevel(row, event.target.value)}
                  className="h-7 rounded-md border border-input bg-background px-1 text-xs"
                >
                  {levelChoices(yourLevel, row.level).map((level) => (
                    <option key={level} value={level}>
                      {savedQueryLevelLabel(level)}
                    </option>
                  ))}
                </select>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label={`Remove ${ruleLabel(row)}`}
                  disabled={busy}
                  onClick={() => remove(row)}
                >
                  <XIcon className="size-3.5" />
                </Button>
              </li>
            ))}
          </ul>
        </div>
        {inherited.length > 0 ? (
          <div className="space-y-1">
            <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              From the folders above
            </p>
            <ul className="space-y-0.5" aria-label="Inherited access">
              {inherited.map((rule) => (
                <li
                  key={`${rule.folderId}:${ruleKey(rule)}`}
                  className="text-sm text-muted-foreground"
                >
                  {ruleLabel(rule)}: {savedQueryLevelLabel(rule.level).toLowerCase()},
                  from &quot;{rule.folderName}&quot;
                </li>
              ))}
            </ul>
          </div>
        ) : null}
        <div className="space-y-1">
          <div className="flex items-center gap-2">
            <UserPlusIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
            <Input
              aria-label="Add a person or role"
              placeholder="Add a person or role"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              className="h-8 text-xs"
            />
            <select
              aria-label="Level to give"
              value={givenLevel}
              disabled={busy}
              onChange={(event) => setGivenLevel(event.target.value as SavedQueryLevel)}
              className="h-7 rounded-md border border-input bg-background px-1 text-xs"
            >
              {levelsFor(yourLevel).map((level) => (
                <option key={level} value={level}>
                  {savedQueryLevelLabel(level)}
                </option>
              ))}
            </select>
          </div>
          {matches.people.length + matches.roles.length > 0 ? (
            <ul
              className="max-h-40 overflow-y-auto rounded-md border border-border bg-background text-xs"
              aria-label="Matches"
            >
              {matches.people.map((person) => (
                <li key={`u${person.id}`}>
                  <button
                    type="button"
                    className="w-full px-2 py-1 text-left hover:bg-muted"
                    onClick={() =>
                      add({
                        userId: person.id,
                        roleId: null,
                        granteeName: person.name,
                        granteeEmail: person.email,
                      })
                    }
                  >
                    {person.name}{" "}
                    <span className="text-muted-foreground">{person.email}</span>
                  </button>
                </li>
              ))}
              {matches.roles.map((role) => (
                <li key={`r${role.id}`}>
                  <button
                    type="button"
                    className="w-full px-2 py-1 text-left hover:bg-muted"
                    onClick={() =>
                      add({
                        userId: null,
                        roleId: role.id,
                        granteeName: role.name,
                        granteeEmail: null,
                      })
                    }
                  >
                    {role.name} <span className="text-muted-foreground">(role)</span>
                  </button>
                </li>
              ))}
            </ul>
          ) : null}
        </div>
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
          <Button size="sm" disabled={busy} onClick={() => void write()}>
            Save access
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
