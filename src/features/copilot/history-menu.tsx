"use client";

import { Check, ChevronDown, MessageSquare, Plus, Trash2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@rantai/design-system/ui/dropdown-menu";
import { cn } from "@/lib/utils";
import type { SessionMeta } from "./use-copilot";

/**
 * Switches between saved conversations, above the chat.
 *
 * The history list used to sit in the sidebar, but only while you were
 * already on `/copilot` — so it was page content occupying navigation
 * space, and it disappeared entirely when the sidebar was collapsed to
 * icons. It belongs with the conversation it belongs to.
 *
 * Renders nothing until at least one conversation exists: on a first
 * visit, a "history" control listing no history is just noise on an
 * otherwise clean welcome screen.
 */
export function CopilotHistoryMenu({
  sessions,
  activeId,
  onSelect,
  onNew,
  onDelete,
}: {
  sessions: SessionMeta[];
  activeId: string | null;
  onSelect: (id: string) => void;
  onNew: () => void;
  onDelete: (id: string) => void;
}) {
  if (sessions.length === 0) return null;

  const active = sessions.find((s) => s.id === activeId);
  const label = active?.title ?? "New conversation";

  return (
    <div className="mx-auto flex w-full max-w-3xl items-center justify-between gap-2 border-b border-border pb-2">
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            aria-label="Switch conversation"
            className="group flex min-w-0 items-center gap-1.5 rounded-md px-2 py-1 text-left hover:bg-muted/60"
          >
            <MessageSquare
              className="size-3.5 shrink-0 text-muted-foreground"
              aria-hidden
            />
            <span className="truncate text-sm font-medium text-foreground">
              {label}
            </span>
            <ChevronDown
              className="size-3.5 shrink-0 text-muted-foreground transition-transform group-data-[state=open]:rotate-180"
              aria-hidden
            />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-72">
          <DropdownMenuGroup>
            {sessions.map((s) => (
              <DropdownMenuItem key={s.id} onClick={() => onSelect(s.id)}>
                <Check
                  className={cn(
                    "size-4",
                    s.id === activeId ? "opacity-100" : "opacity-0"
                  )}
                  aria-hidden
                />
                <span className="flex-1 truncate">{s.title}</span>
                <button
                  type="button"
                  aria-label={`Delete ${s.title}`}
                  className="shrink-0 text-muted-foreground hover:text-destructive"
                  onClick={(event) => {
                    // Without this the click also selects the row, so the
                    // conversation would open on its way to being deleted.
                    event.stopPropagation();
                    event.preventDefault();
                    onDelete(s.id);
                  }}
                >
                  <Trash2 className="size-3.5" />
                </button>
              </DropdownMenuItem>
            ))}
          </DropdownMenuGroup>
          <DropdownMenuSeparator />
          <DropdownMenuItem onClick={onNew}>
            <Plus className="size-4" aria-hidden />
            New chat
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <Button variant="outline" size="sm" onClick={onNew}>
        <Plus className="size-4" />
        New chat
      </Button>
    </div>
  );
}
