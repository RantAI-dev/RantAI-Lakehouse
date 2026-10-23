"use client";

import * as React from "react";
import { Check, ChevronsUpDown } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList } from "@/components/ui/command";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { cn } from "@/lib/utils";
import type { MapEntry } from "./echarts-maps";

/**
 * The map select of the chart builder, searchable (BI-16 C2a follow-up,
 * planner: picker search). With the Natural Earth maps the catalogue has
 * about 250 entries, too many for a plain select. Same Popover + Command
 * pattern as the filter bar and the data table; the Jakarta and Indonesian
 * maps, the ones the console started with, stay pinned at the top.
 */
const PINNED_IDS = ["dki-jakarta", "id-provinces", "id-regencies"];

export function MapPicker({ maps, value, onChange }: {
  maps: readonly MapEntry[]; value: string; onChange: (id: string) => void;
}) {
  const [open, setOpen] = React.useState(false);
  const pinned = PINNED_IDS.flatMap((id) => maps.find((m) => m.id === id) ?? []);
  const rest = maps.filter((m) => !PINNED_IDS.includes(m.id));
  const current = maps.find((m) => m.id === value);

  const item = (m: MapEntry) => (
    <CommandItem key={m.id} value={m.label} onSelect={() => { onChange(m.id); setOpen(false); }}>
      <span className="truncate">{m.label}</span>
      <Check className={cn("ml-auto size-4", m.id === value ? "opacity-100" : "opacity-0")} />
    </CommandItem>
  );

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button variant="outline" role="combobox" aria-expanded={open} aria-label="Map" className="w-full justify-between font-normal">
          <span className="truncate">{current?.label ?? value}</span>
          <ChevronsUpDown className="size-4 shrink-0 opacity-60" />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-[var(--radix-popover-trigger-width)] min-w-64 p-0">
        <Command>
          <CommandInput placeholder="Search maps" aria-label="Search maps" />
          <CommandList>
            <CommandEmpty>No map matches.</CommandEmpty>
            <CommandGroup heading="Indonesia">{pinned.map(item)}</CommandGroup>
            <CommandGroup heading="World and countries">{rest.map(item)}</CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}
