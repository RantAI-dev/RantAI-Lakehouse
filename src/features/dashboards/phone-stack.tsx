"use client";

import { ArrowDown, ArrowUp, Minus, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { movePhoneTile, resizePhoneTile } from "@/lib/dashboard-tabs";
import type { PhoneTile } from "@/services/clients/bi-store";
import type { GridItem } from "./dashboard-grid";
import { SectionTile, TileCard, isSectionKind } from "./tile-card";

const ROW_H = 44;
const MARGIN = 12;
const heightOf = (rows: number) => rows * ROW_H + (rows - 1) * MARGIN;

/**
 * The one-column layout under 640 px (BI-18 part C): every tile full width, in
 * `order`. With `editing` the column is shown at phone width whatever the
 * window, and each tile gets controls to move it up or down and change its
 * height; those are the only things a phone layout holds (widths are always
 * full), and they are sent up as the whole new list.
 */
export function PhoneStack({
  items, order, editing, folded, onToggleFold, onChange,
}: {
  items: GridItem[];
  /** Visible tiles in order, with their heights in rows. */
  order: PhoneTile[];
  editing: boolean;
  folded: ReadonlySet<string>;
  onToggleFold: (id: string) => void;
  /** The whole phone layout after a change (edit mode only). */
  onChange?: (next: PhoneTile[]) => void;
}) {
  const byId = new Map(items.map((i) => [i.id, i]));
  return (
    <div className="mx-auto flex w-full flex-col gap-3" style={editing ? { maxWidth: 390 } : undefined} data-testid="phone-stack">
      {order.map((tile, index) => {
        const item = byId.get(tile.id);
        if (!item) return null;
        return (
          <div key={tile.id} className="relative" style={{ height: heightOf(tile.h) }}>
            {isSectionKind(item.kind) ? (
              <SectionTile item={item} editable={editing} folded={folded.has(item.id)} onToggle={() => onToggleFold(item.id)} />
            ) : (
              <TileCard item={item} editable={false} />
            )}
            {editing && onChange ? (
              <div className="absolute right-1 top-1 z-10 flex items-center gap-0.5 rounded-md border border-border bg-card/95 p-0.5 shadow-sm">
                <Button variant="ghost" size="icon-sm" className="size-6" aria-label={`Move ${item.title} up`} disabled={index === 0}
                        onClick={() => onChange(movePhoneTile(order, tile.id, -1))}><ArrowUp className="size-3.5" /></Button>
                <Button variant="ghost" size="icon-sm" className="size-6" aria-label={`Move ${item.title} down`} disabled={index === order.length - 1}
                        onClick={() => onChange(movePhoneTile(order, tile.id, 1))}><ArrowDown className="size-3.5" /></Button>
                <Button variant="ghost" size="icon-sm" className="size-6" aria-label={`Make ${item.title} shorter`}
                        onClick={() => onChange(resizePhoneTile(order, tile.id, tile.h - 1))}><Minus className="size-3.5" /></Button>
                <span className="px-0.5 text-[10px] tabular-nums text-muted-foreground">{tile.h}</span>
                <Button variant="ghost" size="icon-sm" className="size-6" aria-label={`Make ${item.title} taller`}
                        onClick={() => onChange(resizePhoneTile(order, tile.id, tile.h + 1))}><Plus className="size-3.5" /></Button>
              </div>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
