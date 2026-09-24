"use client";

import * as React from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useInfiniteQuery, useQueryClient } from "@tanstack/react-query";
import { parseAsString, parseAsStringLiteral, useQueryStates } from "nuqs";
import { ChartColumn, Plus, Search, X } from "lucide-react";

import { PageHeader } from "@/components/patterns/page-header";
import { EmptyState, ErrorState } from "@/components/patterns/page-states";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useDebouncedCallback } from "@/hooks/use-debounced-callback";
import { groupSessions, plainPreview } from "@/lib/copilot-sessions";
import { formatDateTime, formatRelativeTime } from "@/lib/format";
import { toServiceError } from "@/services/errors";
import { apiFetch } from "@/services/http";
import { SessionActionsMenu, SessionModeIcon } from "./session-parts";
import { useCopilot, type SessionMeta } from "./use-copilot";

const MODES = ["all", "ask", "build"] as const;
type ModeFilter = (typeof MODES)[number];
const PAGE_SIZE = 30;

type SessionPage = {
  sessions: SessionMeta[];
  hasMore: boolean;
  nextOffset: number | null;
  counts: Partial<Record<"ask" | "build", number>>;
};

async function fetchSessions(q: string, mode: ModeFilter, offset: number): Promise<SessionPage> {
  const params = new URLSearchParams({ limit: String(PAGE_SIZE), offset: String(offset) });
  if (q) params.set("q", q);
  if (mode !== "all") params.set("mode", mode);
  const res = await apiFetch(`/api/ai/sessions?${params.toString()}`, { cache: "no-store" });
  const json = await res.json();
  if (!res.ok) throw new Error(json?.error ?? "Failed to load conversations");
  return json as SessionPage;
}

/**
 * `/copilot/history` — the signed-in user's Copilot conversations, grouped
 * by recency. Search (title and message text) and the mode filter run on
 * the server and live in the URL; rows load page by page as you scroll.
 */
export function CopilotHistoryPage() {
  const c = useCopilot();
  const router = useRouter();
  const queryClient = useQueryClient();
  const [{ q, mode }, setQuery] = useQueryStates({
    q: parseAsString.withDefault(""),
    mode: parseAsStringLiteral(MODES).withDefault("all"),
  });
  const [search, setSearch] = React.useState(q);
  const pushSearch = useDebouncedCallback((next: string) => void setQuery({ q: next || null }), 300);

  const query = useInfiniteQuery({
    queryKey: ["copilot-sessions", q, mode],
    queryFn: ({ pageParam }) => fetchSessions(q, mode, pageParam),
    initialPageParam: 0,
    getNextPageParam: (last) => last.nextOffset ?? undefined,
  });
  const sessions = React.useMemo(() => query.data?.pages.flatMap((p) => p.sessions) ?? [], [query.data]);
  const counts = query.data?.pages[0]?.counts ?? {};
  const total = (counts.ask ?? 0) + (counts.build ?? 0);
  const groups = React.useMemo(() => groupSessions(sessions), [sessions]);
  const filtered = Boolean(q) || mode !== "all";

  // Load the next page when the end of the list scrolls into view.
  const sentinel = React.useRef<HTMLDivElement>(null);
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = query;
  React.useEffect(() => {
    const el = sentinel.current;
    if (!el || !hasNextPage) return;
    const io = new IntersectionObserver((entries) => {
      if (entries[0]?.isIntersecting && !isFetchingNextPage) void fetchNextPage();
    }, { rootMargin: "200px" });
    io.observe(el);
    return () => io.disconnect();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  const refresh = () => void queryClient.invalidateQueries({ queryKey: ["copilot-sessions"] });

  function startNewChat() {
    c.newChat();
    router.push("/copilot");
  }

  function clearFilters() {
    setSearch("");
    void setQuery({ q: null, mode: null });
  }

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Copilot history"
        description="Your conversations with AI Copilot. Only you can see them."
        actions={
          <Button size="sm" onClick={startNewChat}>
            <Plus data-icon="inline-start" />
            New chat
          </Button>
        }
      />

      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="relative w-full sm:max-w-sm">
          <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            type="search"
            value={search}
            onChange={(e) => {
              setSearch(e.target.value);
              pushSearch(e.target.value.trim());
            }}
            placeholder="Search titles and messages…"
            aria-label="Search conversations"
            className="h-9 pr-8 pl-9 [&::-webkit-search-cancel-button]:appearance-none"
          />
          {search ? (
            <button
              type="button"
              onClick={() => {
                setSearch("");
                void setQuery({ q: null });
              }}
              aria-label="Clear search"
              className="absolute top-1/2 right-2.5 -translate-y-1/2 rounded p-0.5 text-muted-foreground hover:text-foreground"
            >
              <X className="size-3.5" />
            </button>
          ) : null}
        </div>

        <Tabs value={mode} onValueChange={(v) => void setQuery({ mode: v === "all" ? null : (v as ModeFilter) })}>
          <TabsList>
            <TabsTrigger value="all">All{query.data ? ` ${total}` : ""}</TabsTrigger>
            <TabsTrigger value="ask">Ask{query.data ? ` ${counts.ask ?? 0}` : ""}</TabsTrigger>
            <TabsTrigger value="build">Build{query.data ? ` ${counts.build ?? 0}` : ""}</TabsTrigger>
          </TabsList>
        </Tabs>
      </div>

      {query.isPending ? (
        <HistorySkeleton />
      ) : query.isError ? (
        <ErrorState error={toServiceError(query.error)} onRetry={() => void query.refetch()} />
      ) : sessions.length === 0 ? (
        filtered ? (
          <EmptyState
            title="No matching conversations"
            description={q ? `Nothing in your ${mode === "all" ? "" : `${mode} `}conversations mentions “${q}”.` : `You have no ${mode} conversations yet.`}
            action={<Button variant="outline" size="sm" onClick={clearFilters}>Clear filters</Button>}
          />
        ) : (
          <EmptyState
            title="No conversations yet"
            description="Conversations you have with AI Copilot are saved here automatically."
            action={<Button size="sm" onClick={startNewChat}>Start a conversation</Button>}
          />
        )
      ) : (
        <div className="flex flex-col gap-5">
          {groups.map(({ group, items }) => (
            <section key={group} aria-labelledby={`history-${group}`}>
              <h2 id={`history-${group}`} className="mb-1.5 px-2 text-xs font-medium text-muted-foreground">
                {group}
              </h2>
              <ul className="divide-y divide-border">
                {items.map((s) => (
                  <SessionRow
                    key={s.id}
                    session={s}
                    active={s.id === c.sessionId}
                    onRename={async (title) => {
                      const ok = await c.renameSession(s.id, title);
                      if (ok) refresh();
                      return ok;
                    }}
                    onDelete={async () => {
                      const ok = await c.removeSession(s.id);
                      if (ok) refresh();
                      return ok;
                    }}
                  />
                ))}
              </ul>
            </section>
          ))}
          <div ref={sentinel} className="flex h-8 items-center justify-center text-xs text-muted-foreground">
            {isFetchingNextPage ? <Spinner /> : hasNextPage ? null : `${sessions.length} conversation${sessions.length === 1 ? "" : "s"}`}
          </div>
        </div>
      )}
    </div>
  );
}

function SessionRow({
  session: s,
  active,
  onRename,
  onDelete,
}: {
  readonly session: SessionMeta;
  readonly active: boolean;
  readonly onRename: (title: string) => Promise<boolean>;
  readonly onDelete: () => Promise<boolean>;
}) {
  const preview = s.preview ? plainPreview(s.preview) : "";
  return (
    <li className="group relative flex items-center gap-3 px-2 py-2.5 transition-colors hover:bg-muted/50">
      <SessionModeIcon mode={s.mode} />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          {/* The link's box covers the row; the actions menu sits above it. */}
          <Link
            href={`/copilot?id=${encodeURIComponent(s.id)}`}
            className="truncate text-sm font-medium text-foreground after:absolute after:inset-0 focus-visible:outline-none group-hover:text-primary"
          >
            {s.title}
          </Link>
          {active ? <Badge variant="secondary" className="h-4.5 shrink-0 px-1.5 text-[10px]">Open now</Badge> : null}
        </div>
        {preview ? <p className="truncate text-xs text-muted-foreground">{preview}</p> : null}
      </div>
      <div className="hidden shrink-0 items-center gap-3 text-xs text-muted-foreground sm:flex">
        {s.chartCreated ? (
          <span className="inline-flex items-center gap-1 text-violet-600 dark:text-violet-400">
            <ChartColumn className="size-3.5" />
            Chart
          </span>
        ) : null}
        {s.updatedAt ? (
          <time dateTime={s.updatedAt} title={formatDateTime(s.updatedAt)} className="w-16 text-right">
            {formatRelativeTime(s.updatedAt)}
          </time>
        ) : null}
      </div>
      <div className="relative z-10">
        <SessionActionsMenu session={s} onRename={onRename} onDelete={onDelete} />
      </div>
    </li>
  );
}

function HistorySkeleton() {
  return (
    <div className="divide-y divide-border" aria-busy>
      {Array.from({ length: 6 }, (_, i) => (
        <div key={i} className="flex items-center gap-3 px-2 py-3">
          <Skeleton className="size-7" />
          <div className="flex-1 space-y-1.5">
            <Skeleton className="h-3.5 w-2/5" />
            <Skeleton className="h-3 w-3/5" />
          </div>
          <Skeleton className="h-3 w-12" />
        </div>
      ))}
    </div>
  );
}
