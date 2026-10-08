"use client";

import * as React from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { History, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useCopilot } from "./use-copilot";
import { ChatMessages } from "./chat-messages";
import { ChatComposer } from "./chat-composer";
import { CopilotHistoryMenu } from "./history-menu";
import { ConversationPanel } from "./conversation-panel";

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
  const lastUrl = React.useRef<string | null | undefined>(undefined);
  const lastActive = React.useRef<string | null>(activeId);

  React.useEffect(() => {
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
  }, [urlId, activeId, loadSession, router]);

  // No conversation to show (no `?id=`, nothing open, nothing being
  // asked): this page has no empty state of its own, so go to Home, where
  // conversations start. A question sent from Home already has its first
  // message and `busy` set before this page mounts (`send` sets both
  // before its first await), so it is never bounced back.
  const nothingOpen = !urlId && !activeId && messageCount === 0 && !busy;
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
 * (`SessionUrlSync`), and "New chat" goes there too. Old conversations are
 * found on History (`/copilot/history`), the sidebar entry.
 *
 * Plain, like Home and like chat products: the chat sits straight on the
 * page, with no card around it and no brand gradient, grid backdrop or
 * glow ring.
 *
 * A thin bar on top carries the conversation's title, New chat and
 * History. History opens `ConversationPanel` on the right, closed by
 * default, for switching without leaving the page (see that component for
 * why right and why not tabs). On narrow screens the bar is the
 * `CopilotHistoryMenu` switcher instead. The brain & history are still
 * shared with the global chat dock via useCopilot.
 */
/** Whether the history panel is open, kept per browser. */
const HISTORY_KEY = "copilot:history-open";

export function CopilotPage() {
  const c = useCopilot();
  const router = useRouter();
  // A per-viewer convenience: storage may be unavailable (private mode,
  // blocked site data), so every access is guarded and closed is the default.
  const [historyOpen, setHistoryOpen] = React.useState(false);
  React.useEffect(() => {
    try {
      setHistoryOpen(window.localStorage.getItem(HISTORY_KEY) === "true");
    } catch {
      // keep the default
    }
  }, []);
  const setHistory = (next: boolean) => {
    setHistoryOpen(next);
    try {
      window.localStorage.setItem(HISTORY_KEY, String(next));
    } catch {
      // not remembered; the toggle still works for this visit
    }
  };
  const activeTitle = c.sessions.find((s) => s.id === c.sessionId)?.title || "Conversation";
  const startNew = () => {
    c.newChat();
    router.push("/");
  };
  // A conversation opened by link has no id here until its messages have
  // arrived (`loadSession` sets both together). One that is open but has
  // had every message deleted is just empty, not loading.
  const loading = c.messages.length === 0 && !c.busy && !c.error && !c.sessionId;

  // Viewport minus the 4rem navbar and `AppFrame`'s vertical padding, so the
  // message list scrolls on its own and the composer stays on the bottom edge.
  return (
    <div className="flex h-[calc(100svh-6rem)] sm:h-[calc(100svh-6.5rem)] lg:h-[calc(100svh-7rem)]">
      <React.Suspense fallback={null}>
        <SessionUrlSync />
      </React.Suspense>

      <div className="flex min-w-0 flex-1 flex-col">
        {/* Narrow screens: the conversation switcher as a bar above the chat. */}
        <div className="shrink-0 pb-2 lg:hidden">
          <CopilotHistoryMenu
            sessions={c.sessions}
            activeId={c.sessionId}
            onSelect={(id) => void c.loadSession(id)}
            onNew={startNew}
          />
        </div>
        <div className="hidden shrink-0 items-center justify-between gap-3 pb-2 lg:flex">
          <h1 title={activeTitle} className="min-w-0 truncate text-sm font-medium text-foreground">
            {activeTitle}
          </h1>
          <div className="flex shrink-0 items-center gap-1.5">
            <Button variant="outline" size="sm" onClick={startNew} className="gap-1.5 bg-background">
              <Plus className="size-4" />
              New chat
            </Button>
            <Button
              variant={historyOpen ? "secondary" : "ghost"}
              size="sm"
              aria-pressed={historyOpen}
              onClick={() => setHistory(!historyOpen)}
              className="gap-1.5"
            >
              <History className="size-4" />
              History
            </Button>
          </div>
        </div>

        <div className="mx-auto flex min-h-0 w-full max-w-3xl flex-1 flex-col">
          <div className="min-h-0 flex-1 overflow-y-auto py-6 pr-0.5">
            {loading ? (
              <p role="status" className="py-10 text-center text-sm text-muted-foreground">
                Loading conversation…
              </p>
            ) : (
              <ChatMessages
                messages={c.messages} draft={c.draft} liveReasoning={c.liveReasoning} onEdit={c.editAndResend} onDelete={c.deleteMessage} onDismissError={c.clearError} busy={c.busy} error={c.error} progress={c.progress} onRetry={c.retry}
                onConfirmTool={c.confirmTool} onCancelTool={c.cancelTool} onCompleteTool={c.completeToolStep} confirmingKey={c.confirmingKey}
                onAnswerAsk={c.answerAskOption} askNotice={c.askNotice}
              />
            )}
          </div>

          <div className="shrink-0">
            <ChatComposer onStop={c.stop} solid
              mode={c.mode} setMode={c.setMode} onSend={c.send} busy={c.busy}
              enabledCaps={c.enabledCaps} toggleCap={c.toggleCap}
              placeholder={c.mode === "build" ? "Tell Copilot what to build or change…" : "Ask anything about your lakehouse data…"}
            />
            <p className="mt-2 text-center text-[11px] text-muted-foreground">
              Copilot can be wrong. Check its answers; anything that changes data asks you first.
            </p>
          </div>
        </div>
      </div>

      {historyOpen ? (
        <ConversationPanel
          className="ml-4 hidden lg:flex"
          sessions={c.sessions}
          activeId={c.sessionId}
          onSelect={(id) => void c.loadSession(id)}
          onClose={() => setHistory(false)}
        />
      ) : null}
    </div>
  );
}
