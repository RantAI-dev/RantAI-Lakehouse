"use client";

import * as React from "react";
import { Mail, X } from "lucide-react";
import {
  Combobox, ComboboxChip, ComboboxChips, ComboboxChipsInput, ComboboxContent, ComboboxEmpty,
  ComboboxItem, ComboboxList, useComboboxAnchor,
} from "@/components/ui/combobox";
import { reportService } from "@/services";
import type { ReportPerson } from "@/services/contracts/reports";

/**
 * One recipient of a report: a person of the console (`userId`) or an outside
 * address (`email`), as the subscription API takes them.
 */
export type ChosenRecipient = {
  /** `u:<id>` or `e:<address>`, unique in the list. */
  readonly key: string;
  readonly userId?: string;
  readonly email?: string;
  /** What the chip reads: a person's name, or the address. */
  readonly label: string;
  /** A person's address, under the name in the list. */
  readonly detail?: string | null;
};

/** The address shape the form always accepted; the server is the authority on the domain. */
const ADDRESS = /^[^\s@,;<>]+@[^\s@,;<>]+\.[^\s@,;<>]+$/;

export function addressRecipient(text: string): ChosenRecipient | null {
  const email = text.trim().toLowerCase();
  return ADDRESS.test(email) ? { key: `e:${email}`, email, label: email } : null;
}

function personRecipient(p: ReportPerson): ChosenRecipient {
  return { key: `u:${p.id}`, userId: p.id, label: p.name, detail: p.email };
}

function initials(label: string): string {
  return label.split(/\s+/).filter(Boolean).slice(0, 2).map((w) => w[0]?.toUpperCase() ?? "").join("") || "?";
}

/**
 * The recipients of a report in one field (owner QA 2026-11-16: a people
 * search and a separate "outside address" input with its own Add button were
 * two ways to do one thing). Typing searches the console's people; a full
 * email address that is not one of them is offered as an outside address.
 * Enter takes the highlighted choice, and a comma or semicolon takes a typed
 * address. Each recipient is a chip with its own remove button.
 *
 * An outside address still has to be on a domain an administrator allowed;
 * the field says so in one line and the server refuses with its own sentence.
 */
export function ReportRecipientsField({ id, labelledBy, describedBy, value, onChange }: {
  readonly id: string;
  readonly labelledBy: string;
  readonly describedBy?: string;
  readonly value: readonly ChosenRecipient[];
  readonly onChange: (next: ChosenRecipient[]) => void;
}) {
  const anchor = useComboboxAnchor();
  const [query, setQuery] = React.useState("");
  const [found, setFound] = React.useState<ReportPerson[]>([]);

  React.useEffect(() => {
    const q = query.trim();
    if (!q) { setFound([]); return; }
    const controller = new AbortController();
    const timer = setTimeout(() => {
      reportService.people(q, controller.signal).then(setFound).catch(() => setFound([]));
    }, 200);
    return () => { clearTimeout(timer); controller.abort(); };
  }, [query]);

  const chosen = new Set(value.map((r) => r.key));
  const people = found.map(personRecipient).filter((r) => !chosen.has(r.key));
  const typed = addressRecipient(query);
  // A typed address that is a person's own is that person, not an outside address.
  const offerAddress = typed && !chosen.has(typed.key) && !found.some((p) => p.email?.toLowerCase() === typed.email)
    && !value.some((r) => r.detail?.toLowerCase() === typed.email);
  const items = offerAddress && typed ? [...people, typed] : people;

  function add(next: ChosenRecipient) {
    if (!chosen.has(next.key)) onChange([...value, next]);
    setQuery("");
  }

  return (
    <Combobox
      multiple
      items={items}
      value={[...value]}
      onValueChange={(next: ChosenRecipient[]) => { onChange(next); setQuery(""); }}
      inputValue={query}
      onInputValueChange={setQuery}
      filter={null}
      isItemEqualToValue={(a: ChosenRecipient, b: ChosenRecipient) => a.key === b.key}
      itemToStringLabel={(r: ChosenRecipient) => r.label}
      autoHighlight
    >
      <ComboboxChips ref={anchor} className="w-full gap-1.5 py-1">
        {value.map((r) => (
          <ComboboxChip key={r.key} showRemove={false} className="h-6 max-w-full gap-1.5 rounded-md pr-0.5 pl-1">
            {r.userId ? (
              <span aria-hidden className="flex size-4 shrink-0 items-center justify-center rounded-full bg-primary/15 text-[9px] font-semibold text-primary">
                {initials(r.label)}
              </span>
            ) : (
              <Mail aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
            )}
            <span className="min-w-0 truncate" title={r.detail ?? r.label}>{r.label}</span>
            <button
              type="button"
              aria-label={`Remove ${r.label}`}
              onClick={() => onChange(value.filter((x) => x.key !== r.key))}
              className="flex size-5 shrink-0 items-center justify-center rounded text-muted-foreground outline-none transition-colors duration-150 ease-out hover:bg-foreground/10 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none"
            >
              <X className="size-3" aria-hidden />
            </button>
          </ComboboxChip>
        ))}
        <ComboboxChipsInput
          id={id}
          aria-labelledby={labelledBy}
          aria-describedby={describedBy}
          placeholder={value.length ? "Add another" : "Find a person or type an email address"}
          autoComplete="off"
          className="h-6 min-w-28 bg-transparent px-1 text-sm placeholder:text-muted-foreground"
          onKeyDown={(e) => {
            if ((e.key === "," || e.key === ";") && typed) { e.preventDefault(); add(typed); }
          }}
        />
      </ComboboxChips>
      <ComboboxContent anchor={anchor}>
        <ComboboxEmpty>{query.trim() ? "No one found. Type a full email address to add it." : "Type a name or an email address."}</ComboboxEmpty>
        <ComboboxList>
          {(r: ChosenRecipient) => (
            <ComboboxItem key={r.key} value={r}>
              {r.userId ? (
                <span className="flex min-w-0 flex-1 items-baseline gap-2">
                  <span className="truncate">{r.label}</span>
                  {r.detail ? <span className="truncate text-xs text-muted-foreground">{r.detail}</span> : null}
                </span>
              ) : (
                <span className="flex min-w-0 flex-1 items-center gap-2">
                  <Mail aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
                  <span className="truncate">Add {r.label}</span>
                  <span className="shrink-0 text-xs text-muted-foreground">outside address</span>
                </span>
              )}
            </ComboboxItem>
          )}
        </ComboboxList>
      </ComboboxContent>
    </Combobox>
  );
}
