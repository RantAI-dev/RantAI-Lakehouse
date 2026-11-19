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
 * the free-text field with a `datalist` read as a typing exercise). The names
 * are `listTimeZones` (WEB-8 D8: the browser's IANA list, plus `UTC` and the
 * zone already held, so a saved value is never missing). The browser's own
 * zone is first, marked, so the likely answer is one press away; each name
 * shows its offset now. The server still checks the name.
 */
export function TimeZoneSelect({ id, value, onValueChange }: {
  readonly id: string;
  readonly value: string;
  readonly onValueChange: (zone: string) => void;
}) {
  const own = React.useMemo(() => browserTimeZone(), []);
  const zones = React.useMemo(() => {
    const all = listTimeZones(value);
    if (own && !all.includes(own)) all.push(own);
    return own ? [own, ...all.filter((z) => z !== own)] : all;
  }, [value, own]);
  const offsets = React.useMemo(() => {
    const now = new Date();
    return new Map(zones.map((z) => [z, utcOffset(z, now)]));
  }, [zones]);

  return (
    <Combobox
      items={zones}
      value={value}
      onValueChange={(next) => { if (typeof next === "string" && next) onValueChange(next); }}
      filter={(zone: string, query: string) => matches(zone, query)}
      autoHighlight
    >
      {/* Focus selects the name, so typing searches instead of adding to it. */}
      <ComboboxInput id={id} className="w-full" placeholder="Search a time zone" autoComplete="off"
        onFocus={(e) => e.currentTarget.select()} />
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
