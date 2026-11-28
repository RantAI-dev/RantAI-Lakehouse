"use client";

import * as React from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { chatSessionService } from "@/services";
import { useCopilot } from "./use-copilot";
import { ChatMessages } from "./chat-messages";
import { ChatComposer } from "./chat-composer";

/**
 * `/copilot?open=latest` (where the old `/copilot/history` address lands,
 * `WEB-18`, through the redirect in `next.config.ts`) opens the person's most recent conversation, or Home when they
 * have none.
 */
export const OPEN_LATEST = "latest";

/**
 * Keeps `/copilot?id=` and the open conversation in step, both ways, so a
 * conversation has a link and Back returns to it.
 *
 * - The URL changed (a history link, Back/Forward): open that conversation.
 * - The open conversation changed (picked from the menu, New chat, or a new
 *   chat saved for the first time): rewrite the URL to match.
 *
 * The refs record what each side last looked like, so the side that just
 * moved is followed and the other never bounces it back — while a load is
 * in flight the old conversation is still open, and must not rewrite the
 * URL that asked for the new one.
 */
function SessionUrlSync() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const { sessionId: activeId, loadSession, messages, busy } = useCopilot();
  const messageCount = messages.length;
  const urlId = searchParams.get("id") ?? searchParams.get("session");
  const openLatest = !urlId && searchParams.get("open") === OPEN_LATEST;
  const lastUrl = React.useRef<string | null | undefined>(undefined);
  const lastActive = React.useRef<string | null>(activeId);

  // `open=latest`: ask for the newest conversation and open it by its address; a failure or an
  // empty list goes to Home, like any address with nothing to show.
  React.useEffect(() => {
    if (!openLatest) return;
    let live = true;
    chatSessionService
      .list({ limit: 1 })
      .then((page) => {
        const latest = page.sessions[0]?.id;
        if (live) router.replace(latest ? `/copilot?id=${encodeURIComponent(latest)}` : "/");
      })
      .catch(() => {
        if (live) router.replace("/");
      });
    return () => {
      live = false;
    };
  }, [openLatest, router]);

  React.useEffect(() => {
    if (openLatest) return;
    if (urlId !== lastUrl.current) {
      lastUrl.current = urlId;
      lastActive.current = activeId;
      if (urlId && urlId !== activeId) void loadSession(urlId);
      else if (!urlId && activeId) router.replace(`/copilot?id=${encodeURIComponent(activeId)}`);
      return;
    }
    if (activeId !== lastActive.current) {
      lastActive.current = activeId;
      if (activeId !== urlId) {
        router.replace(activeId ? `/copilot?id=${encodeURIComponent(activeId)}` : "/copilot");
      }
    }
  }, [openLatest, urlId, activeId, loadSession, router]);

  // No conversation to show (no `?id=`, nothing open, nothing being
  // asked): this page has no empty state of its own, so go to Home, where
  // conversations start. A question sent from Home already has its first
  // message and `busy` set before this page mounts (`send` sets both
  // before its first await), so it is never bounced back.
  const nothingOpen = !urlId && !openLatest && !activeId && messageCount === 0 && !busy;
  React.useEffect(() => {
    if (nothingOpen) router.replace("/");
  }, [nothingOpen, router]);

  return null;
}

/**
 * One conversation with Copilot: the messages, and a composer on the
 * bottom edge to continue it.
 *
 * It is not where a conversation starts. Home (`/`) has the composer for
 * that, and this page has no empty state: it used to carry its own
 * welcome, starters and composer, which made it a second Home (QA
 * feedback, twice). With nothing to show it sends you to Home
 * (`SessionUrlSync`).
 *
 * Plain, like Home and like chat products: the chat sits straight on the
 * page, with no card around it and no brand gradient, grid backdrop or
 * glow ring.
 *
 * A thin bar on top carries the conversation's title, and nothing else
 * (`WEB-18`, owner 2026-11-14): the other conversations, New chat and
 * Search are in the app sidebar, which lists the chats on this page
 * (`SidebarChats`), so the page's own History button, its side panel and
 * its New chat button went ("satu aksi, satu tombol"). The brain and the
 * history are shared with the global chat dock via useCopilot.
 */
export function CopilotPage() {
  const c = useCopilot();
  // AI-24 D2: the open conversation's own title, not a lookup in the newest
  // 20 the history menus list — a conversation reopened from the chat list
  // is often older than those. "New conversation" is for no conversation.
  const activeTitle = c.sessionTitle || (c.sessionId ? "Conversation" : "New conversation");
  // A conversation opened by link has no id here until its messages have
  // arrived (`loadSession` sets both together). One that is open but has
  // had every message deleted is just empty, not loading.
  const loading = c.messages.length === 0 && !c.busy && !c.error && !c.sessionId;

  // Viewport minus the navbar (`--app-navbar-h`) and `AppFrame`'s vertical padding, so the
  // message list scrolls on its own and the composer stays on the bottom edge.
  return (
    <div className="flex h-[calc(100svh-var(--app-navbar-h)-2rem)] sm:h-[calc(100svh-var(--app-navbar-h)-2.5rem)] lg:h-[calc(100svh-var(--app-navbar-h)-3rem)]">
      <React.Suspense fallback={null}>
        <SessionUrlSync />
      </React.Suspense>

      <div className="flex min-w-0 flex-1 flex-col">
        {/* Owner QA 2026-11-15: the thread's title is not drawn at the top of the chat (the sidebar
            list and the navbar already name where you are). It stays as the page's heading for
            screen readers, and the 36 px bar it sat in is gone, so the chat starts at the top. */}
        <h1 className="sr-only">{activeTitle}</h1>

        <div className="mx-auto flex min-h-0 w-full max-w-3xl flex-1 flex-col">
          <div className="min-h-0 flex-1 overflow-y-auto py-6 pr-0.5">
            {loading ? (
              <p role="status" className="py-10 text-center text-sm text-muted-foreground">
                Loading conversation…
              </p>
            ) : (
              <ChatMessages
                messages={c.messages} onEdit={c.editAndResend} onDelete={c.deleteMessage} onDismissError={c.clearError} busy={c.busy} error={c.error} onRetry={c.retry}
                onConfirmTool={c.confirmTool} onCancelTool={c.cancelTool} confirmingKey={c.confirmingKey}
                onAnswerAsk={c.answerAskOption} askNotice={c.askNotice}
              />
            )}
          </div>

          <div className="shrink-0">
            <ChatComposer onStop={c.stop} solid
              mode={c.mode} setMode={c.setMode} onSend={(t, mentions) => c.send(t, undefined, { mentions })} busy={c.busy}
              enabledCaps={c.enabledCaps} toggleCap={c.toggleCap}
              placeholder={c.mode === "build" ? "Tell AI Copilot what to build or change…" : "Ask anything about your lakehouse data…"}
            />
            <p className="mt-2 text-center text-[11px] text-muted-foreground">
              AI Copilot can be wrong. Check its answers; anything that changes data asks you first.
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}
