"use client";

import * as React from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import {
  ArrowRight,
  Clock,
  History,
  MessageSquare,
  Plus,
  Search,
  Sparkles,
  Trash2,
  X,
} from "lucide-react";

import { PageHeader } from "@/components/patterns/page-header";
import { Button, buttonVariants } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { formatDateTime, formatRelativeTime } from "@/lib/format";
import { cn } from "@/lib/utils";
import { useCopilot, type Mode } from "./use-copilot";

type ModeFilter = "all" | Mode;

export function CopilotHistoryPage() {
  const c = useCopilot();
  const router = useRouter();

  const [search, setSearch] = React.useState("");
  const [modeFilter, setModeFilter] = React.useState<ModeFilter>("all");
  const [deletingId, setDeletingId] = React.useState<string | null>(null);

  const filteredSessions = React.useMemo(() => {
    return c.sessions.filter((s) => {
      const matchesSearch =
        !search.trim() ||
        s.title.toLowerCase().includes(search.toLowerCase().trim()) ||
        s.mode.toLowerCase().includes(search.toLowerCase().trim());
      const matchesMode = modeFilter === "all" || s.mode === modeFilter;
      return matchesSearch && matchesMode;
    });
  }, [c.sessions, search, modeFilter]);

  const handleOpen = async (id: string) => {
    await c.loadSession(id);
    c.setExpanded(true);
    router.push("/copilot");
  };

  const handleNewChat = () => {
    c.newChat();
    c.setExpanded(true);
    router.push("/copilot");
  };

  const handleDelete = async (id: string, e: React.MouseEvent) => {
    e.stopPropagation();
    e.preventDefault();
    setDeletingId(id);
    try {
      await c.removeSession(id);
    } finally {
      setDeletingId(null);
    }
  };

  function renderHistoryList() {
    if (c.sessions.length === 0) {
      return (
        <div className="flex flex-col items-center justify-center rounded-2xl border border-dashed border-border p-12 text-center">
          <div className="grid size-12 place-items-center rounded-2xl bg-muted/60 text-muted-foreground mb-3">
            <MessageSquare className="size-6" />
          </div>
          <h3 className="text-base font-semibold text-foreground">No conversation history yet</h3>
          <p className="mt-1 max-w-sm text-sm text-muted-foreground">
            All your conversations with AI Copilot will automatically be saved here.
          </p>
          <Button onClick={handleNewChat} className="mt-4 gap-1.5" size="sm">
            <Plus className="size-3.5" />
            <span>Start First Conversation</span>
          </Button>
        </div>
      );
    }

    if (filteredSessions.length === 0) {
      return (
        <div className="flex flex-col items-center justify-center rounded-2xl border border-dashed border-border p-12 text-center">
          <div className="grid size-12 place-items-center rounded-2xl bg-muted/60 text-muted-foreground mb-3">
            <Search className="size-6" />
          </div>
          <h3 className="text-base font-semibold text-foreground">No conversations found</h3>
          <p className="mt-1 text-sm text-muted-foreground">
            No conversations match &quot;{search}&quot;.
          </p>
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              setSearch("");
              setModeFilter("all");
            }}
            className="mt-4"
          >
            Reset filters & search
          </Button>
        </div>
      );
    }

    return (
      <div className="flex flex-col gap-2">
        {filteredSessions.map((s) => {
          const isActive = s.id === c.sessionId;
          const isDeleting = deletingId === s.id;

          return (
            <div
              key={s.id}
              className={cn(
                "group relative flex items-center justify-between gap-3 rounded-lg border border-border bg-card px-3.5 py-2.5 transition-all text-left",
                "hover:border-primary/40 hover:bg-muted/30 hover:shadow-2xs",
                isActive && "border-primary/50 bg-primary/2",
                isDeleting && "opacity-50 pointer-events-none"
              )}
            >
              {/* Left: clickable main area */}
              <button
                type="button"
                onClick={() => void handleOpen(s.id)}
                className="flex min-w-0 flex-1 items-center gap-2.5 text-left focus:outline-hidden cursor-pointer"
              >
                <span
                  className={cn(
                    "grid size-7 shrink-0 place-items-center rounded-md",
                    s.mode === "build"
                      ? "bg-amber-500/10 text-amber-600 dark:text-amber-400"
                      : "bg-primary/10 text-primary"
                  )}
                >
                  {s.mode === "build" ? (
                    <Sparkles className="size-3.5" />
                  ) : (
                    <MessageSquare className="size-3.5" />
                  )}
                </span>

                <span className="truncate text-sm font-medium text-foreground group-hover:text-primary transition-colors">
                  {s.title}
                </span>

                <Badge
                  variant={s.mode === "build" ? "secondary" : "outline"}
                  className="text-[10px] uppercase font-mono tracking-wider px-1.5 py-0 h-4.5 shrink-0"
                >
                  {s.mode}
                </Badge>

                {isActive ? (
                  <Badge
                    variant="default"
                    className="text-[10px] px-1.5 py-0 h-4.5 bg-emerald-600 text-white hover:bg-emerald-600 shrink-0"
                  >
                    Active
                  </Badge>
                ) : null}
              </button>

              {/* Right: Timestamp & Actions */}
              <div className="flex items-center gap-2 sm:gap-3 shrink-0">
                <span
                  className="flex items-center gap-1.5 text-xs text-muted-foreground whitespace-nowrap"
                  title={s.updatedAt ? formatDateTime(s.updatedAt) : undefined}
                >
                  <Clock className="size-3.5 text-muted-foreground/60 shrink-0" />
                  <span>{s.updatedAt ? formatRelativeTime(s.updatedAt) : "Just now"}</span>
                </span>

                <div className="flex items-center gap-1">
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => void handleOpen(s.id)}
                    className="h-7 px-2 text-xs font-medium text-muted-foreground hover:text-primary gap-1 group/open"
                  >
                    <span className="hidden sm:inline">Open</span>
                    <ArrowRight className="size-3.5 transition-transform group-hover/open:translate-x-0.5" />
                  </Button>

                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={(e) => void handleDelete(s.id, e)}
                    className="size-7 text-muted-foreground/40 hover:text-destructive hover:bg-destructive/10 rounded transition-colors opacity-0 group-hover:opacity-100 focus:opacity-100"
                    title="Delete conversation"
                    aria-label="Delete conversation"
                  >
                    <Trash2 className="size-3.5" />
                  </Button>
                </div>
              </div>
            </div>
          );
        })}
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-6 p-4 sm:p-6 max-w-5xl mx-auto w-full">
      <PageHeader
        title={
          <div className="flex items-center gap-2.5">
            <span className="grid size-9 place-items-center rounded-xl bg-primary/10 text-primary">
              <History className="size-5" />
            </span>
            <span>History</span>
          </div>
        }
        description="Manage, search, and resume your conversations with AI Copilot."
        actions={
          <div className="flex items-center gap-2">
            <Link
              href="/copilot"
              className={cn(
                buttonVariants({ variant: "outline", size: "sm" }),
                "gap-1.5 inline-flex items-center"
              )}
            >
              <MessageSquare className="size-3.5" />
              <span>Open Copilot</span>
            </Link>
            <Button size="sm" onClick={handleNewChat} className="gap-1.5">
              <Plus className="size-3.5" />
              <span>New Chat</span>
            </Button>
          </div>
        }
      />

      {/* Toolbar: Search & Mode Filter */}
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="relative flex-1 max-w-md">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 size-4 text-muted-foreground pointer-events-none" />
          <Input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search conversations by title or topic…"
            className="pl-9 pr-8 h-9"
          />
          {search ? (
            <button
              type="button"
              onClick={() => setSearch("")}
              aria-label="Clear search"
              className="absolute right-2.5 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground p-0.5"
            >
              <X className="size-3.5" />
            </button>
          ) : null}
        </div>

        <div className="flex items-center gap-1.5 self-start sm:self-auto">
          <button
            type="button"
            onClick={() => setModeFilter("all")}
            className={cn(
              "px-3 py-1.5 rounded-lg text-xs font-medium transition-colors cursor-pointer",
              modeFilter === "all"
                ? "bg-primary text-primary-foreground shadow-xs"
                : "text-muted-foreground hover:bg-muted hover:text-foreground"
            )}
          >
            All ({c.sessions.length})
          </button>
          <button
            type="button"
            onClick={() => setModeFilter("ask")}
            className={cn(
              "px-3 py-1.5 rounded-lg text-xs font-medium transition-colors cursor-pointer",
              modeFilter === "ask"
                ? "bg-primary text-primary-foreground shadow-xs"
                : "text-muted-foreground hover:bg-muted hover:text-foreground"
            )}
          >
            Ask ({c.sessions.filter((s) => s.mode === "ask").length})
          </button>
          <button
            type="button"
            onClick={() => setModeFilter("build")}
            className={cn(
              "px-3 py-1.5 rounded-lg text-xs font-medium transition-colors cursor-pointer",
              modeFilter === "build"
                ? "bg-primary text-primary-foreground shadow-xs"
                : "text-muted-foreground hover:bg-muted hover:text-foreground"
            )}
          >
            Build ({c.sessions.filter((s) => s.mode === "build").length})
          </button>
        </div>
      </div>

      {/* History List */}
      {renderHistoryList()}
    </div>
  );
}
