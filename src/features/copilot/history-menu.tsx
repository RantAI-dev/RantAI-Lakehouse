"use client";

import Link from "next/link";
import { ChevronDown, History, MessageSquare, Plus } from "lucide-react";

import { Button, buttonVariants } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";
import { RecentSessionsMenuContent } from "./session-parts";
import { useCopilot, type SessionMeta } from "./use-copilot";

/**
 * Switches between saved conversations, above the chat in /copilot: the
 * active title opens the recent list, with History and New chat beside it.
 */
export function CopilotHistoryMenu({
  sessions,
  activeId,
  onSelect,
  onNew,
}: {
  readonly sessions: SessionMeta[];
  readonly activeId: string | null;
  readonly onSelect: (id: string) => void;
  readonly onNew: () => void;
}) {
  const active = sessions.find((s) => s.id === activeId);
  const label = active?.title ?? "New conversation";

  return (
    <div className="mx-auto flex w-full max-w-3xl items-center justify-between gap-2">
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <button
              type="button"
              aria-label="Switch conversation"
              title={label}
              className="group flex min-w-0 items-center gap-1.5 rounded-md px-2 py-1 text-left transition-colors hover:bg-muted/60"
            />
          }
        >
          <MessageSquare className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
          <span className="truncate text-sm font-medium">{label}</span>
          <ChevronDown className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-80">
          <RecentSessionsMenuContent sessions={sessions} activeId={activeId} onSelect={onSelect} onNew={onNew} />
        </DropdownMenuContent>
      </DropdownMenu>

      <div className="flex shrink-0 items-center gap-1.5">
        <Link
          href="/copilot/history"
          className={cn(buttonVariants({ variant: "ghost", size: "sm" }), "h-8 gap-1.5 text-xs text-muted-foreground")}
        >
          <History className="size-3.5" />
          History
        </Link>
        <Button variant="outline" size="sm" onClick={onNew} className="h-8 gap-1.5 text-xs">
          <Plus className="size-3.5" />
          New chat
        </Button>
      </div>
    </div>
  );
}

/** Compact history button for the Copilot dock and sidebar headers. */
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
      <DropdownMenuTrigger
        render={
          <button
            type="button"
            aria-label="Chat history"
            title="Chat history"
            className={cn(
              "grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground",
              className
            )}
          />
        }
      >
        <History className="size-4" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align={align} className="w-80">
        <RecentSessionsMenuContent
          sessions={c.sessions}
          activeId={c.sessionId}
          onSelect={(id) => {
            void c.loadSession(id);
            c.setExpanded(true);
          }}
          onNew={c.newChat}
        />
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
