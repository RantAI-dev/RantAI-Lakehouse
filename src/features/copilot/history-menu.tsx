"use client";

import { History } from "lucide-react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";
import { RecentSessionsMenuContent } from "./session-parts";
import { useCopilot } from "./use-copilot";

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
