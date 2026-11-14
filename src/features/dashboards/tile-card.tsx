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
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";
import type { GridItem, TileLook, TileMenuItem } from "./dashboard-grid";
import { EASE } from "./dashboard-header";

/** The kinds drawn without a card: they have no data (BI-18 part C). */
export function isSectionKind(kind: string | undefined): kind is "heading" | "divider" | "section" {
  return kind === "heading" || kind === "divider" || kind === "section";
}

function MenuEntry({ m }: { m: TileMenuItem }) {
  if (m.sub?.length) {
    return (
      <DropdownMenuSub>
        <DropdownMenuSubTrigger>{m.icon}{m.label}</DropdownMenuSubTrigger>
        <DropdownMenuSubContent className="w-52">
          {m.sub.map((s) => (
            <DropdownMenuItem key={s.label} variant={s.destructive ? "destructive" : "default"} onClick={s.onSelect}>
              {s.icon}
              {s.label}
            </DropdownMenuItem>
          ))}
        </DropdownMenuSubContent>
      </DropdownMenuSub>
    );
  }
  return (
    <DropdownMenuItem variant={m.destructive ? "destructive" : "default"} onClick={m.onSelect}>
      {m.icon}
      {m.label}
    </DropdownMenuItem>
  );
}

function TileMenu({ item, triggerClassName }: { item: GridItem; triggerClassName?: string }) {
  if (!item.menu?.length) return null;
  return (
    <div className="print:hidden" onPointerDown={(e) => e.stopPropagation()}>
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              variant="ghost"
              size="icon"
              className={cn("size-7 text-muted-foreground hover:text-foreground", triggerClassName)}
              aria-label={`Actions for ${item.title}`}
            />
          }
        >
          <MoreHorizontal className="size-4" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className={item.menuNote ? "w-60" : "w-48"}>
          {item.menuNote ? (
            <>
              {/* BI-61 T4, review round 1 SHOULD-FIX 3: one line that never wraps; a long name ends in "…" and the tooltip says it all. */}
              <p className="truncate px-2 pt-1.5 pb-2 text-[12px] text-muted-foreground tabular-nums" data-slot="tile-menu-notes"
                title={item.menuNote.detail} aria-label={item.menuNote.detail}>
                {item.menuNote.label}
              </p>
              <DropdownMenuSeparator />
            </>
          ) : item.menuLabel ? (
            <DropdownMenuGroup>
              <DropdownMenuGroupLabel className="truncate font-mono text-[11px] font-normal">
                {item.menuLabel}
              </DropdownMenuGroupLabel>
            </DropdownMenuGroup>
          ) : null}
          {item.menu.map((m, i) => (
            <React.Fragment key={m.label}>
              {m.separatorBefore && i > 0 ? <DropdownMenuSeparator /> : null}
              <MenuEntry m={m} />
            </React.Fragment>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}

/**
 * A tile: header (drag handle in edit mode, title, badge, ⋯ menu) over its body.
 *
 * `soft` (`BI-61` T4, the dashboard page): a soft fill with no border and no
 * badge. Its ⋯ and the quiet "Data from" line appear when the tile is hovered
 * or the menu is focused, laid over the header's right side so they never
 * push the title into "…"; on a touch screen, which cannot hover, the ⋯ stays.
 */
export function TileCard({
  item, editable, dragging, onHeaderPointerDown, look = "card",
}: {
  item: GridItem;
  editable: boolean;
  dragging?: boolean;
  onHeaderPointerDown?: (e: React.PointerEvent) => void;
  look?: TileLook;
}) {
  const handle = editable && onHeaderPointerDown;
  if (look === "soft") {
    return (
      <div
        data-slot="tile"
        className={cn(
          "group/tile flex h-full flex-col overflow-hidden rounded-2xl bg-muted/45 dark:bg-card",
          EASE,
          handle && "ring-1 ring-primary/25 ring-inset",
          dragging && "ring-2 ring-primary/60",
        )}
      >
        <div
          className={cn("flex h-10 shrink-0 items-center gap-1.5 pl-4 pr-2 pt-1", handle && "cursor-move select-none pl-2")}
          style={handle ? { touchAction: "none" } : undefined}
          onPointerDown={handle ? onHeaderPointerDown : undefined}
        >
          {handle ? <GripVertical className="size-4 shrink-0 text-muted-foreground" aria-hidden /> : null}
          <div className="min-w-0 flex-1">
            <p className="flex items-center gap-1.5 truncate text-sm font-medium leading-tight">
              <span className="truncate">{item.title}</span>
              {item.hint}
            </p>
            {item.subtitle ? <p className="truncate text-[11px] text-muted-foreground">{item.subtitle}</p> : null}
          </div>
          <span className="relative flex shrink-0 items-center">
            {item.dataFrom ? (
              <span
                data-slot="tile-data-from"
                className={cn(
                  "pointer-events-none absolute right-8 hidden whitespace-nowrap rounded-full bg-muted/90 px-2 py-0.5 text-[11px] text-muted-foreground tabular-nums opacity-0 lg:inline dark:bg-card",
                  "group-hover/tile:opacity-100 group-focus-within/tile:opacity-100",
                  EASE,
                )}
              >
                {item.dataFrom}
              </span>
            ) : null}
            <TileMenu
              item={item}
              triggerClassName={cn(
                EASE,
                !editable && "lg:pointer-fine:opacity-0 lg:pointer-fine:group-hover/tile:opacity-100 focus-visible:opacity-100 data-[popup-open]:opacity-100",
              )}
            />
          </span>
        </div>
        <div className="min-h-0 flex-1 px-2 pb-2">{item.body}</div>
      </div>
    );
  }
  return (
    <div className={cn("flex h-full flex-col overflow-hidden rounded-xl border border-border bg-card shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]", dragging && "ring-2 ring-primary/40")}>
      {/* Header / drag handle */}
      <div className={cn("flex items-center gap-2 border-b border-border px-3 py-1.5", handle && "cursor-move select-none bg-muted/40")}
           style={handle ? { touchAction: "none" } : undefined}
           onPointerDown={editable ? onHeaderPointerDown : undefined}>
        {handle ? <GripVertical className="size-4 shrink-0 text-muted-foreground" /> : null}
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
    ? "rounded-lg bg-primary/5 px-2 ring-1 ring-primary/25 ring-inset"
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
