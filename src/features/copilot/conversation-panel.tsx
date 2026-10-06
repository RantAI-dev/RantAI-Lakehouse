"use client";

import * as React from "react";
import Link from "next/link";
import { History, X } from "lucide-react";

import { buttonVariants } from "@/components/ui/button";
import { groupSessions } from "@/lib/copilot-sessions";
import { formatRelativeTime } from "@/lib/format";
import { cn } from "@/lib/utils";
import { SessionModeIcon } from "./session-parts";
import type { SessionMeta } from "./use-copilot";

/**
 * The conversation history on /copilot: a panel on the right of the chat,
 * opened from the page's History button and closed by default.
 *
 * It is on the right, not the left, because the app's navigation sidebar
 * is already on the left and a second list beside it read as two stacked
 * sidebars (QA feedback). Switching between old conversations is the less
 * common thing to do here, so it is secondary content, and the right edge
 * is where a console puts that. It is a vertical list, not tabs: titles
 * are long, the list is unbounded, and it is grouped by recency.
 *
 * Rename and delete stay on the History page, as for the header menus
 * (`RecentSessionsMenuContent`).
 */
export function ConversationPanel({
  sessions,
  activeId,
  onSelect,
  onClose,
  className,
}: {
  readonly sessions: SessionMeta[];
  readonly activeId: string | null;
  readonly onSelect: (id: string) => void;
  readonly onClose: () => void;
  readonly className?: string;
}) {
  const groups = React.useMemo(() => groupSessions(sessions.slice(0, 50)), [sessions]);

  return (
    <aside aria-label="Conversation history" className={cn("flex w-72 shrink-0 min-h-0 flex-col border-l border-border pl-3", className)}>
      <div className="flex items-center justify-between gap-2 pb-2 pl-2">
        <h2 className="text-sm font-semibold">History</h2>
        <button
          type="button"
          onClick={onClose}
          aria-label="Close history"
          title="Close history"
          className="grid size-8 shrink-0 place-items-center rounded-lg text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
        >
          <X className="size-4" />
        </button>
      </div>
      <nav className="min-h-0 flex-1 overflow-y-auto">
        {groups.length === 0 ? (
          <p className="px-2 py-6 text-center text-xs text-muted-foreground">
            No conversations yet. Your chats are kept here.
          </p>
        ) : (
          groups.map((g) => (
            <div key={g.group} className="mb-3">
              <p className="px-2 pb-1 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">{g.group}</p>
              <ul className="flex flex-col gap-0.5">
                {g.items.map((s) => {
                  const active = s.id === activeId;
                  return (
                    <li key={s.id}>
                      <button
                        type="button"
                        onClick={() => onSelect(s.id)}
                        aria-current={active ? "true" : undefined}
                        title={s.title}
                        className={cn(
                          "flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50",
                          active ? "bg-muted text-foreground" : "text-muted-foreground hover:bg-muted/70 hover:text-foreground",
                        )}
                      >
                        <SessionModeIcon mode={s.mode} className="size-6" />
                        <span className="flex min-w-0 flex-1 flex-col">
                          <span className={cn("truncate text-xs", active && "font-medium")}>{s.title || "Untitled conversation"}</span>
                          {s.updatedAt ? (
                            <span className="text-[10px] text-muted-foreground tabular-nums">{formatRelativeTime(s.updatedAt)}</span>
                          ) : null}
                        </span>
                      </button>
                    </li>
                  );
                })}
              </ul>
            </div>
          ))
        )}
      </nav>
      <div className="border-t border-border pt-2">
        <Link href="/copilot/history" className={cn(buttonVariants({ variant: "ghost", size: "sm" }), "w-full justify-start gap-2 text-xs text-muted-foreground")}>
          <History className="size-3.5" />
          All history
        </Link>
      </div>
    </aside>
  );
}
