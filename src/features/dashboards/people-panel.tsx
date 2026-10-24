"use client";

import * as React from "react";
import { Shield, UserPlus, Users, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { readableErrorMessage } from "@/lib/notify-message";
// Features import `@/services` only.
import {
  dashboardService, type AccessKind, type AccessLevel, type GranteeMatches, type ItemAccess,
} from "@/services";

/**
 * `BI-19` part B, feature B9: said next to the switch, because it surprises
 * people. The last sentence is BI-19 B1 owner decision Q-R14 (a restriction ends
 * the rules inherited from above it).
 */
export const RESTRICTION_NOTE =
  "Only the people and roles listed can open this. A public link or an embed on it keeps working: a manager made it on purpose. Permissions given on folders above it no longer apply to it.";

/** What the page shows for a level word (an unknown word is shown as it is). */
export function levelLabel(level: string): string {
  return level === "view" ? "Can view" : level === "edit" ? "Can edit" : level === "manage" ? "Can manage" : level;
}

type Row = { kind: "user" | "role"; id: string; name: string; email?: string; level: AccessLevel | string };

const rowKey = (r: { kind: string; id: string }) => `${r.kind}:${r.id}`;

function sentence(e: unknown): string {
  return readableErrorMessage(e instanceof Error ? e.message : "The change was not made.");
}

/**
 * Who can open one dashboard or folder (`BI-19` part B1, feature B1 to B9): add
 * a person or a role with a level, switch "only the people listed", and see
 * what reaches the item from the folders above. The rows are edited here and
 * written together with Save, because the server replaces the item's whole
 * list in one step.
 */
export function PeoplePanel({ kind, id }: { readonly kind: AccessKind; readonly id: string }) {
  const [access, setAccess] = React.useState<ItemAccess | null>(null);
  const [rows, setRows] = React.useState<Row[]>([]);
  const [restricted, setRestricted] = React.useState(false);
  const [failed, setFailed] = React.useState<string | null>(null);
  const [message, setMessage] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const [matches, setMatches] = React.useState<GranteeMatches>({ people: [], roles: [] });

  function load(next: ItemAccess) {
    setAccess({ ...next, rules: next.rules ?? [], inherited: next.inherited ?? [], restrictedAbove: next.restrictedAbove ?? [] });
    setRows((next.rules ?? []).map((r) => ({ kind: r.kind, id: r.id, name: r.name, email: r.email, level: r.level })));
    setRestricted(next.restricted);
  }

  React.useEffect(() => {
    let cancelled = false;
    setAccess(null);
    setFailed(null);
    dashboardService
      .getAccess(kind, id)
      .then((a) => { if (!cancelled) load(a); })
      .catch((e) => { if (!cancelled) setFailed(sentence(e)); });
    return () => { cancelled = true; };
  }, [kind, id]);

  React.useEffect(() => {
    const text = query.trim();
    if (text.length < 2) { setMatches({ people: [], roles: [] }); return; }
    const ctl = new AbortController();
    const timer = setTimeout(() => {
      dashboardService
        .findGrantees(text, ctl.signal)
        .then(setMatches)
        .catch(() => setMatches({ people: [], roles: [] }));
    }, 250);
    return () => { clearTimeout(timer); ctl.abort(); };
  }, [query]);

  function add(row: Row) {
    setRows((cur) => (cur.some((r) => rowKey(r) === rowKey(row)) ? cur : [...cur, row]));
    setQuery("");
    setMatches({ people: [], roles: [] });
  }

  async function save() {
    setBusy(true);
    setMessage(null);
    try {
      const saved = await dashboardService.saveAccess({
        kind,
        id,
        restricted,
        rules: rows.map((r) => (r.kind === "user" ? { userId: r.id, level: r.level as AccessLevel } : { roleId: r.id, level: r.level as AccessLevel })),
      });
      load(saved);
      setMessage("Saved.");
    } catch (e) {
      setMessage(sentence(e));
    } finally { setBusy(false); }
  }

  if (failed) {
    return (
      <div className="rounded-lg border border-border bg-muted/20 p-3">
        <p role="alert" className="text-xs text-muted-foreground">{failed}</p>
      </div>
    );
  }
  if (!access) {
    return <div className="rounded-lg border border-border bg-muted/20 p-3 text-xs text-muted-foreground">Loading who has access…</div>;
  }

  return (
    <div className="space-y-2 rounded-lg border border-border bg-muted/20 p-3">
      <div className="flex items-start gap-3">
        <Users className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium">People</p>
          <p className="text-xs text-muted-foreground">Give a person or a role access to this {kind === "board" ? "dashboard" : "folder"}.</p>
        </div>
      </div>

      <div className="flex items-start gap-3">
        <Shield className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium">Only the people listed</p>
          <p className="text-[11px] text-muted-foreground">{RESTRICTION_NOTE}</p>
        </div>
        <Switch checked={restricted} onCheckedChange={setRestricted} aria-label="Only the people listed" disabled={busy} />
      </div>

      {access.restrictedAbove.length > 0 ? (
        <p role="status" className="text-[11px] text-amber-600 dark:text-amber-400">
          Already limited to the people listed by the folder {access.restrictedAbove.map((f) => `“${f.folderName}”`).join(", ")}.
        </p>
      ) : null}

      <ul className="space-y-1" aria-label="People with access">
        {rows.length === 0 ? <li className="text-xs text-muted-foreground">Nobody has been added.</li> : null}
        {rows.map((r) => (
          <li key={rowKey(r)} className="flex items-center gap-2 text-xs">
            <span className="min-w-0 flex-1 truncate">
              {r.name || r.id}
              {r.kind === "role" ? <span className="ml-1 text-muted-foreground">(role)</span> : r.email ? <span className="ml-1 text-muted-foreground">{r.email}</span> : null}
            </span>
            <span className="text-muted-foreground">{levelLabel(r.level)}</span>
            <Button variant="ghost" size="icon-sm" aria-label={`Remove ${r.name || r.id}`} disabled={busy}
              onClick={() => setRows((cur) => cur.filter((x) => rowKey(x) !== rowKey(r)))}>
              <X className="size-3.5" />
            </Button>
          </li>
        ))}
      </ul>

      {access.inherited.length > 0 ? (
        <div className="space-y-1">
          <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">From the folders above</p>
          <ul className="space-y-0.5" aria-label="Inherited access">
            {access.inherited.map((r) => (
              <li key={`${r.folderId}:${rowKey(r)}`} className="text-xs text-muted-foreground">
                {r.name || r.id}{r.kind === "role" ? " (role)" : ""}: {levelLabel(r.level).toLowerCase()}, from “{r.folderName}”
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      <div className="space-y-1">
        <div className="flex items-center gap-2">
          <UserPlus className="size-4 shrink-0 text-muted-foreground" />
          <Input aria-label="Add a person or role" placeholder="Add a person or role" value={query}
            onChange={(e) => setQuery(e.target.value)} className="h-8 text-xs" />
        </div>
        {matches.people.length + matches.roles.length > 0 ? (
          <ul className="max-h-40 overflow-y-auto rounded-md border border-border bg-background text-xs" aria-label="Matches">
            {matches.people.map((p) => (
              <li key={`u${p.id}`}>
                <button type="button" className="w-full px-2 py-1 text-left hover:bg-muted"
                  onClick={() => add({ kind: "user", id: p.id, name: p.name, email: p.email, level: "view" })}>
                  {p.name} <span className="text-muted-foreground">{p.email}</span>
                </button>
              </li>
            ))}
            {matches.roles.map((r) => (
              <li key={`r${r.id}`}>
                <button type="button" className="w-full px-2 py-1 text-left hover:bg-muted"
                  onClick={() => add({ kind: "role", id: r.id, name: r.name, level: "view" })}>
                  {r.name} <span className="text-muted-foreground">(role)</span>
                </button>
              </li>
            ))}
          </ul>
        ) : null}
      </div>

      <div className="flex items-center gap-2">
        <Button size="sm" onClick={() => void save()} disabled={busy}>Save people</Button>
        {message ? <p role="status" className="text-xs text-muted-foreground">{message}</p> : null}
      </div>
    </div>
  );
}
