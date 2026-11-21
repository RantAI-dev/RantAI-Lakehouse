"use client";

import * as React from "react";
import { useInfiniteQuery, useQueryClient } from "@tanstack/react-query";

import { chatSessionService } from "@/services";
import type { SessionMeta } from "./use-copilot";

/** How many conversations one page of the chat list loads. */
export const CHAT_LIST_PAGE_SIZE = 30;

/** The query key every list of the signed-in user's conversations shares. */
export const CHAT_LIST_KEY = ["copilot-sessions"] as const;

type ChatPage = {
  sessions: SessionMeta[];
  nextOffset: number | null;
};

async function fetchChats(q: string, offset: number, signal?: AbortSignal): Promise<ChatPage> {
  const page = await chatSessionService.list({
    limit: CHAT_LIST_PAGE_SIZE,
    offset,
    ...(q ? { q } : {}),
  }, signal);
  return { sessions: page.sessions, nextOffset: page.hasMore ? page.nextOffset : null };
}

/**
 * The signed-in user's conversations, newest first, a page at a time
 * (`WEB-18`, the chat list in the sidebar; it took over from the History
 * page, which this logic came from). `q` searches titles and message text on
 * the server, as History did, so a match in an older page is still found.
 *
 * `enabled` is false until the list is first shown, so a page that never
 * shows it makes no request for it.
 */
export function useChatList(q: string, enabled = true) {
  const query = useInfiniteQuery({
    queryKey: [...CHAT_LIST_KEY, q],
    queryFn: ({ pageParam, signal }) => fetchChats(q, pageParam, signal),
    initialPageParam: 0,
    getNextPageParam: (last) => last.nextOffset ?? undefined,
    enabled,
  });
  const sessions = React.useMemo(() => query.data?.pages.flatMap((p) => p.sessions) ?? [], [query.data]);
  return { ...query, sessions };
}

/**
 * Reloads the chat list whenever the Copilot's own recent list changes: a
 * conversation saved for the first time, renamed, deleted, or answered (its
 * time moves it to the top). The Copilot refreshes its recent list after each
 * of those (`use-copilot.ts`), so following that one signal keeps the two in
 * step without every caller remembering to invalidate. The first list is
 * skipped: it is the page loading, not a change.
 */
export function useFollowRecentSessions(recent: SessionMeta[]) {
  const queryClient = useQueryClient();
  const seen = React.useRef<string | null>(null);
  const signature = recent.map((s) => `${s.id}:${s.title}:${s.updatedAt ?? ""}`).join("|");
  React.useEffect(() => {
    if (seen.current === null) {
      seen.current = signature;
      return;
    }
    if (seen.current === signature) return;
    seen.current = signature;
    void queryClient.invalidateQueries({ queryKey: CHAT_LIST_KEY });
  }, [signature, queryClient]);
}
