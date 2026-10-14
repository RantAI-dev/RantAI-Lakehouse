"use client";

import * as React from "react";
import { ChevronDown, GripVertical, MoreHorizontal } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuGroupLabel,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";
import type { GridItem } from "./dashboard-grid";

/** The kinds drawn without a card: they have no data (BI-18 part C). */
export function isSectionKind(kind: string | undefined): kind is "heading" | "divider" | "section" {
  return kind === "heading" || kind === "divider" || kind === "section";
}

function TileMenu({ item }: { item: GridItem }) {
  if (!item.menu?.length) return null;
  return (
    <div className="print:hidden" onPointerDown={(e) => e.stopPropagation()}>
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              variant="ghost"
              size="icon"
              className="size-7 text-muted-foreground hover:text-foreground"
              aria-label={`Actions for ${item.title}`}
            />
          }
        >
          <MoreHorizontal className="size-4" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-48">
          {item.menuLabel ? (
            <DropdownMenuGroup>
              <DropdownMenuGroupLabel className="truncate font-mono text-[11px] font-normal">
                {item.menuLabel}
              </DropdownMenuGroupLabel>
            </DropdownMenuGroup>
          ) : null}
          {item.menu.map((m, i) => (
            <React.Fragment key={m.label}>
              {m.separatorBefore && i > 0 ? <DropdownMenuSeparator /> : null}
              <DropdownMenuItem variant={m.destructive ? "destructive" : "default"} onClick={m.onSelect}>
                {m.icon}
                {m.label}
              </DropdownMenuItem>
            </React.Fragment>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}

/** A tile: header (drag handle in edit mode, title, badge, ⋯ menu) over its body. */
export function TileCard({
  item, editable, dragging, onHeaderPointerDown,
}: {
  item: GridItem;
  editable: boolean;
  dragging?: boolean;
  onHeaderPointerDown?: (e: React.PointerEvent) => void;
}) {
  return (
    <div className={cn("flex h-full flex-col overflow-hidden rounded-xl border border-border bg-card shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]", dragging && "ring-2 ring-primary/40")}>
      {/* Header / drag handle */}
      <div className={cn("flex items-center gap-2 border-b border-border px-3 py-1.5", editable && onHeaderPointerDown && "cursor-move select-none bg-muted/40")}
           style={editable && onHeaderPointerDown ? { touchAction: "none" } : undefined}
           onPointerDown={editable ? onHeaderPointerDown : undefined}>
        {editable && onHeaderPointerDown ? <GripVertical className="size-4 shrink-0 text-muted-foreground" /> : null}
        <div className="min-w-0 flex-1">
          <p className="flex items-center gap-1.5 truncate text-sm font-semibold leading-tight">
            <span className="truncate">{item.title}</span>
            {item.hint}
          </p>
          {item.subtitle ? <p className="truncate text-[11px] text-muted-foreground">{item.subtitle}</p> : null}
        </div>
        {item.badge}
        <TileMenu item={item} />
      </div>
      {/* Body */}
      <div className="min-h-0 flex-1 p-2">{item.body}</div>
    </div>
  );
}

/**
 * A heading, a rule or a section header (BI-18 part C). No card and no data.
 * A section header is a button that folds the tiles below it; the fold is the
 * viewer's own and never saved. In edit mode the tile shows a grip, an
 * outline and its menu, and does not fold.
 */
export function SectionTile({
  item, editable, folded, onToggle, onHeaderPointerDown,
}: {
  item: GridItem;
  editable: boolean;
  folded?: boolean;
  onToggle?: () => void;
  onHeaderPointerDown?: (e: React.PointerEvent) => void;
}) {
  const kind = item.kind;
  const chrome = editable
    ? "rounded-md border border-dashed border-primary/40 bg-primary/5 px-2"
    : "px-1";
  return (
    <div className={cn("flex h-full items-center gap-2", chrome)}
         onPointerDown={editable ? onHeaderPointerDown : undefined}
         style={editable && onHeaderPointerDown ? { touchAction: "none", cursor: "move" } : undefined}>
      {editable && onHeaderPointerDown ? <GripVertical className="size-4 shrink-0 text-muted-foreground" /> : null}
      {kind === "divider" ? (
        <hr className="flex-1 border-t border-border" aria-label="Divider" />
      ) : kind === "section" ? (
        <button
          type="button"
          className="flex min-w-0 flex-1 items-center gap-1.5 border-b border-border py-1 text-left text-base font-semibold"
          aria-expanded={!folded}
          onClick={editable ? undefined : onToggle}
        >
          <ChevronDown className={cn("size-4 shrink-0 text-muted-foreground transition-transform", folded && "-rotate-90", editable && "hidden")} aria-hidden />
          <span className="truncate">{item.title}</span>
        </button>
      ) : (
        <h3 className="min-w-0 flex-1 truncate text-lg font-semibold">{item.title}</h3>
      )}
      <TileMenu item={item} />
    </div>
  );
}
