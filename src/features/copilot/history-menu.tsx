"use client";

import * as React from "react";
import Link from "next/link";
import {
  Check,
  ChevronDown,
  Clock,
  History,
  MessageSquare,
  Plus,
  Search,
  Trash2,
} from "lucide-react";

import { Button, buttonVariants } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";
import { formatRelativeTime } from "@/lib/format";
import { useCopilot, type SessionMeta } from "./use-copilot";

type CopilotHistoryMenuProps = {
  readonly sessions: SessionMeta[];
  readonly activeId: string | null;
  readonly onSelect: (id: string) => void;
  readonly onNew: () => void;
  readonly onDelete: (id: string) => void;
};

/**
 * Switches between saved conversations, above the chat in /copilot.
 * Shows active conversation title, recent history with relative timestamps,
 * quick navigation to full history page, and new chat action.
 */
export function CopilotHistoryMenu({
  sessions,
  activeId,
  onSelect,
  onNew,
  onDelete,
}: CopilotHistoryMenuProps) {
  const active = sessions.find((s) => s.id === activeId);
  const label = active?.title ?? "New conversation";

  return (
    <div className="mx-auto flex w-full max-w-3xl items-center justify-between gap-2 border-b border-border pb-2">
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            aria-label="Switch conversation"
            className="group flex min-w-0 items-center gap-1.5 rounded-md px-2 py-1 text-left hover:bg-muted/60 transition-colors"
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
        <DropdownMenuContent align="start" className="w-80">
          {sessions.length > 0 ? (
            <DropdownMenuGroup>
              {sessions.slice(0, 8).map((s) => (
                <DropdownMenuItem
                  key={s.id}
                  onClick={() => onSelect(s.id)}
                  className="group flex items-center justify-between gap-2 py-2"
                >
                  <div className="flex items-center gap-2 min-w-0 flex-1">
                    <Check
                      className={cn(
                        "size-3.5 shrink-0",
                        s.id === activeId ? "opacity-100 text-primary" : "opacity-0"
                      )}
                      aria-hidden
                    />
                    <div className="flex flex-col min-w-0 flex-1">
                      <div className="flex items-center gap-1.5">
                        <span className="truncate text-xs font-medium text-foreground">
                          {s.title}
                        </span>
                        <span className="rounded bg-muted px-1 py-0.5 text-[9px] uppercase font-mono text-muted-foreground shrink-0">
                          {s.mode}
                        </span>
                      </div>
                      {s.updatedAt ? (
                        <span className="flex items-center gap-1 text-[11px] text-muted-foreground mt-0.5">
                          <Clock className="size-3 shrink-0" />
                          <span>Updated {formatRelativeTime(s.updatedAt)}</span>
                        </span>
                      ) : null}
                    </div>
                  </div>

                  <button
                    type="button"
                    aria-label={`Delete ${s.title}`}
                    title="Delete conversation"
                    className="shrink-0 p-1 text-muted-foreground/40 hover:text-destructive hover:bg-destructive/10 rounded transition-all opacity-0 group-hover:opacity-100 focus:opacity-100"
                    onClick={(event) => {
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
          ) : (
            <div className="px-3 py-4 text-center text-xs text-muted-foreground">
              No saved conversations yet
            </div>
          )}

          <DropdownMenuSeparator />

          <DropdownMenuItem asChild>
            <Link
              href="/copilot/history"
              className="flex items-center gap-2 text-xs font-medium text-primary cursor-pointer"
            >
              <Search className="size-3.5" />
              <span>View all history & search…</span>
            </Link>
          </DropdownMenuItem>

          <DropdownMenuItem onClick={onNew} className="flex items-center gap-2 text-xs cursor-pointer">
            <Plus className="size-3.5" aria-hidden />
            <span>New chat</span>
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <div className="flex items-center gap-1.5">
        <Link
          href="/copilot/history"
          className={cn(
            buttonVariants({ variant: "ghost", size: "sm" }),
            "h-8 gap-1.5 text-xs text-muted-foreground hover:text-foreground inline-flex items-center"
          )}
        >
          <History className="size-3.5" />
          <span>History</span>
        </Link>
        <Button variant="outline" size="sm" onClick={onNew} className="h-8 gap-1.5 text-xs">
          <Plus className="size-3.5" />
          <span>New chat</span>
        </Button>
      </div>
    </div>
  );
}

/**
 * Compact history button and dropdown for Copilot Dock & Sidebar headers.
 */
export function CopilotHistoryButton({
  align = "end",
  className,
}: {
  readonly align?: "start" | "end" | "center";
  readonly className?: string;
}) {
  const c = useCopilot();

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label="Chat history"
          title="Chat history"
          className={cn(
            "grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground",
            className
          )}
        >
          <History className="size-4" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align={align} className="w-80">
        <div className="flex items-center justify-between px-2.5 py-1.5 text-xs font-semibold text-muted-foreground border-b border-border/50">
          <span>Recent Conversations</span>
          <span className="text-[10px] font-normal">{c.sessions.length} total</span>
        </div>

        {c.sessions.length > 0 ? (
          <DropdownMenuGroup className="max-h-64 overflow-y-auto">
            {c.sessions.slice(0, 6).map((s) => (
              <DropdownMenuItem
                key={s.id}
                onClick={() => {
                  void c.loadSession(s.id);
                  c.setExpanded(true);
                }}
                className="group flex items-center justify-between gap-2 py-2 cursor-pointer"
              >
                <div className="flex items-center gap-2 min-w-0 flex-1">
                  <Check
                    className={cn(
                      "size-3.5 shrink-0",
                      s.id === c.sessionId ? "opacity-100 text-primary" : "opacity-0"
                    )}
                    aria-hidden
                  />
                  <div className="flex flex-col min-w-0 flex-1">
                    <div className="flex items-center gap-1.5">
                      <span className="truncate text-xs font-medium text-foreground">
                        {s.title}
                      </span>
                      <span className="rounded bg-muted px-1 py-0.5 text-[9px] uppercase font-mono text-muted-foreground shrink-0">
                        {s.mode}
                      </span>
                    </div>
                    {s.updatedAt ? (
                      <span className="flex items-center gap-1 text-[11px] text-muted-foreground mt-0.5">
                        <Clock className="size-3 shrink-0" />
                        <span>Updated {formatRelativeTime(s.updatedAt)}</span>
                      </span>
                    ) : null}
                  </div>
                </div>

                <button
                  type="button"
                  aria-label={`Delete ${s.title}`}
                  title="Delete conversation"
                  className="shrink-0 p-1 text-muted-foreground/40 hover:text-destructive hover:bg-destructive/10 rounded transition-all opacity-0 group-hover:opacity-100 focus:opacity-100"
                  onClick={(event) => {
                    event.stopPropagation();
                    event.preventDefault();
                    void c.removeSession(s.id);
                  }}
                >
                  <Trash2 className="size-3.5" />
                </button>
              </DropdownMenuItem>
            ))}
          </DropdownMenuGroup>
        ) : (
          <div className="px-3 py-4 text-center text-xs text-muted-foreground">
            No saved conversations yet
          </div>
        )}

        <DropdownMenuSeparator />

        <DropdownMenuItem asChild>
          <Link
            href="/copilot/history"
            className="flex items-center gap-2 text-xs font-medium text-primary cursor-pointer"
          >
            <Search className="size-3.5" />
            <span>View all history & search…</span>
          </Link>
        </DropdownMenuItem>

        <DropdownMenuItem
          onClick={c.newChat}
          className="flex items-center gap-2 text-xs cursor-pointer"
        >
          <Plus className="size-3.5" aria-hidden />
          <span>New chat</span>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
