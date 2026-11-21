"use client";

import * as React from "react";
import {
  Combobox, ComboboxContent, ComboboxEmpty, ComboboxInput, ComboboxItem, ComboboxList,
} from "@/components/ui/combobox";
import { listTimeZones } from "@/lib/time-zones";

/** The zone this browser is in, or `null` when the runtime cannot say. */
export function browserTimeZone(): string | null {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || null;
  } catch {
    return null;
  }
}

/** `UTC+7`, `UTC-3:30`, `UTC` for a zone now; empty when the runtime cannot say. */
function utcOffset(zone: string, now: Date): string {
  try {
    const part = new Intl.DateTimeFormat("en", { timeZone: zone, timeZoneName: "shortOffset" })
      .formatToParts(now)
      .find((p) => p.type === "timeZoneName")?.value;
    return part ? part.replace(/^GMT/, "UTC") : "";
  } catch {
    return "";
  }
}

/** "new york" finds `America/New_York`: the words of a name, not its underscores. */
function matches(zone: string, query: string): boolean {
  const q = query.trim().toLowerCase().replace(/[\s_]+/g, " ");
  return !q || zone.toLowerCase().replace(/_/g, " ").includes(q);
}

/**
 * The time zone of a subscription as a searchable list (owner QA 2026-11-16:
 * the free-text field with a `datalist` read as a typing exercise); Settings >
 * Reporting uses it too (`UX-7`), so it lives in `patterns`. The names
 * are `listTimeZones` (WEB-8 D8: the browser's IANA list, plus `UTC` and the
 * zone already held, so a saved value is never missing). The browser's own
 * zone is first, marked, so the likely answer is one press away; each name
 * shows its offset now. The server still checks the name.
 *
 * The text in the field is held here, not by the input (`PR #635 review
 * SHOULD-FIX 2`): a name typed in full and then left (blur, Enter) is chosen,
 * and text that names no zone goes back to the zone the form holds. Without
 * this the field could show Berlin while the form saved Jakarta, because the
 * value only changes when an item is picked.
 */
export function TimeZoneSelect({ id, value, onValueChange, disabled, zones: zoneSource = listTimeZones }: {
  readonly id: string;
  readonly value: string;
  readonly onValueChange: (zone: string) => void;
  /** A read-only form (a non-admin on Settings) shows the zone but cannot change it. */
  readonly disabled?: boolean;
  /** Where the names come from; tests pass a short list (`UX-7`), the app the browser's. */
  readonly zones?: (held: string) => string[];
}) {
  const own = React.useMemo(() => browserTimeZone(), []);
  const zones = React.useMemo(() => {
    const all = zoneSource(value);
    if (own && !all.includes(own)) all.push(own);
    return own ? [own, ...all.filter((z) => z !== own)] : all;
  }, [value, own, zoneSource]);
  const offsets = React.useMemo(() => {
    const now = new Date();
    return new Map(zones.map((z) => [z, utcOffset(z, now)]));
  }, [zones]);
  // The text in the field, which may not be a chosen zone yet.
  const [draft, setDraft] = React.useState(value);
  // Following `value` rather than setting the draft in each caller keeps one
  // owner for it: a chosen zone, or the parent resetting the form, lands here.
  React.useEffect(() => {
    setDraft(value);
  }, [value]);
  /**
   * On leaving the field: text that names a zone in the list is chosen; any
   * other text goes back to the held zone, so what the field shows and what
   * the form saves can never disagree (`PR #635 review SHOULD-FIX 2`).
   */
  const commitTyped = () => {
    const typed = draft.trim();
    if (typed === value) return;
    if (zones.includes(typed)) onValueChange(typed);
    else setDraft(value);
  };

  return (
    <Combobox
      items={zones}
      value={value}
      onValueChange={(next) => { if (typeof next === "string" && next) onValueChange(next); }}
      inputValue={draft}
      onInputValueChange={(next) => setDraft(typeof next === "string" ? next : "")}
      filter={(zone: string, query: string) => matches(zone, query)}
      autoHighlight
    >
      {/* Focus selects the name, so typing searches instead of adding to it. */}
      <ComboboxInput id={id} className="w-full" placeholder="Search a time zone" autoComplete="off" disabled={disabled}
        onFocus={(e) => e.currentTarget.select()}
        // Leaving the field commits what it shows (a name typed in full) or
        // puts the held zone back (`PR #635 review SHOULD-FIX 2`). A click on
        // an item also blurs first; that commit is harmless, because the item
        // then lands and `value` (which the draft follows) is what wins.
        onBlur={commitTyped}
        onKeyDown={(e) => { if (e.key === "Enter") commitTyped(); }} />
      <ComboboxContent>
        <ComboboxEmpty>No time zone matches.</ComboboxEmpty>
        <ComboboxList>
          {(zone: string) => (
            <ComboboxItem key={zone} value={zone}>
              <span className="min-w-0 flex-1 truncate">{zone}</span>
              <span className="shrink-0 text-xs text-muted-foreground tabular-nums">
                {zone === own ? "Your zone · " : ""}{offsets.get(zone)}
              </span>
            </ComboboxItem>
          )}
        </ComboboxList>
      </ComboboxContent>
    </Combobox>
  );
}
