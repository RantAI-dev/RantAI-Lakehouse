"use client";

import { BarChart3, Check, ChevronDown, Plus } from "lucide-react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@rantai/design-system/ui/dropdown-menu";
import { cn } from "@/lib/utils";

export type BoardOption = { id: string; name: string };

/**
 * Picks which dashboard is open, rendered as the page title.
 *
 * This list used to live in the sidebar, but only appeared once you were
 * already on `/dashboards` — so it could never be used to *get* here. That
 * made it page content wearing navigation's clothes. A dashboard is a
 * document rather than a section of the product, so choosing one belongs
 * with the document.
 *
 * Creating a dashboard moved along with it. It was previously only
 * reachable from the sidebar, which put it out of reach for anyone
 * browsing with the sidebar collapsed to icons.
 */
export function BoardSwitcher({
  boards,
  activeId,
  activeName,
  onSelect,
  onCreate,
}: {
  boards: BoardOption[];
  activeId: string;
  activeName: string;
  onSelect: (id: string) => void;
  onCreate: () => void;
}) {
  return (
    <DropdownMenu>
      {/* `asChild`, not `render`: this menu comes from the design system,
          which wraps radix — the local `@/components/ui` primitives wrap
          base-ui and use `render` instead. */}
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label="Switch dashboard"
          className="group -ml-2 flex min-w-0 items-center gap-1.5 rounded-md px-2 py-0.5 text-left hover:bg-muted/60"
        >
          <span className="truncate text-2xl font-semibold leading-8 tracking-[-0.02em] text-foreground">
            {activeName}
          </span>
          <ChevronDown
            className="size-4 shrink-0 text-muted-foreground transition-transform group-data-[state=open]:rotate-180"
            aria-hidden
          />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-64">
        <DropdownMenuGroup>
          {boards.map((b) => (
            <DropdownMenuItem key={b.id} onClick={() => onSelect(b.id)}>
              <Check
                className={cn(
                  "size-4",
                  b.id === activeId ? "opacity-100" : "opacity-0"
                )}
                aria-hidden
              />
              <BarChart3 className="size-3.5 opacity-70" aria-hidden />
              <span className="truncate">{b.name}</span>
            </DropdownMenuItem>
          ))}
        </DropdownMenuGroup>
        <DropdownMenuSeparator />
        <DropdownMenuItem onClick={onCreate}>
          <Plus className="size-4" aria-hidden />
          New dashboard
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
