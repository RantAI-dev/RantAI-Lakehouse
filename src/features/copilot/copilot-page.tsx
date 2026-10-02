"use client";

import * as React from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { motion, useReducedMotion, type Variants } from "motion/react";
import { History, MessageSquare, Plus, Sparkles } from "lucide-react";
import { Button } from "@/components/ui/button";
import { SuggestionButton } from "@/features/copilot/suggestion-button";
import { useService } from "@/hooks/use-service";
import { cn } from "@/lib/utils";
import { pipelineService } from "@/services";
import { useCopilot, type Mode } from "./use-copilot";
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
  const { sessionId: activeId, loadSession } = useCopilot();
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

  return null;
}

/**
 * The AI Copilot page — a chat view: a centered welcome with the two
 * modes and starters, the conversation, a composer on the bottom edge.
 *
 * Plain, like Home and like chat products: the chat sits straight on the
 * page, with no card around it (a bordered frame holding the page's only
 * content read as a window inside the window) and no brand gradient, grid
 * backdrop or glow ring (QA feedback each time). It does a different job
 * from Home: Home shows what needs attention and asks one question; this
 * page is where conversations are held and resumed.
 *
 * A thin bar on top carries the open conversation's title, New chat and
 * History. History opens `ConversationPanel` on the right, closed by
 * default (see that component for why right and why not tabs). On narrow
 * screens the bar is the `CopilotHistoryMenu` switcher instead. The brain
 * & history are still shared with the global chat dock via useCopilot.
 */
/** Whether the history panel is open, kept per browser. */
const HISTORY_KEY = "copilot:history-open";

export function CopilotPage() {
  const c = useCopilot();
  const reduce = useReducedMotion() ?? false;
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
  const activeTitle = c.sessions.find((s) => s.id === c.sessionId)?.title || "New conversation";

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
            onNew={() => c.newChat()}
          />
        </div>
        <div className="hidden shrink-0 items-center justify-between gap-3 pb-2 lg:flex">
          <h1 title={activeTitle} className="min-w-0 truncate text-sm font-medium text-foreground">
            {activeTitle}
          </h1>
          <div className="flex shrink-0 items-center gap-1.5">
            <Button variant="outline" size="sm" onClick={() => c.newChat()} className="gap-1.5 bg-background">
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
            {c.messages.length === 0 ? (
              <EmptyChat
                reduce={reduce}
                mode={c.mode}
                setMode={c.setMode}
                title={c.pageContext.title}
                suggestions={c.pageContext.suggest[c.mode]}
                onPick={(q) => c.send(q)}
                busy={c.busy}
              />
            ) : (
              <ChatMessages
                messages={c.messages} draft={c.draft} liveReasoning={c.liveReasoning} onEdit={c.editAndResend} onDelete={c.deleteMessage} onDismissError={c.clearError} busy={c.busy} error={c.error} progress={c.progress} onRetry={c.retry}
                onConfirmTool={c.confirmTool} onCancelTool={c.cancelTool} onCompleteTool={c.completeToolStep} confirmingKey={c.confirmingKey}
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

const MODES: { mode: Mode; label: string; icon: typeof MessageSquare; what: string; safety: string }[] = [
  {
    mode: "ask",
    label: "Ask",
    icon: MessageSquare,
    what: "Answers questions about your data, pipelines and catalog, and explains what it finds.",
    safety: "Only reads. Nothing is changed.",
  },
  {
    mode: "build",
    label: "Build",
    icon: Sparkles,
    what: "Creates charts and dashboards, sets up alerts and sources, runs pipelines.",
    safety: "Asks you before anything is changed.",
  },
];

/**
 * The empty conversation: what the two modes do (picking one switches the
 * composer's mode), then starters for the chosen mode. A failed pipeline,
 * when there is one, leads the Ask starters: it is read from the
 * pipelines service, so it names something that exists here.
 */
function EmptyChat({
  reduce,
  mode,
  setMode,
  title,
  suggestions,
  onPick,
  busy,
}: {
  readonly reduce: boolean;
  readonly mode: Mode;
  readonly setMode: (m: Mode) => void;
  readonly title: string;
  readonly suggestions: string[];
  readonly onPick: (q: string) => void;
  readonly busy: boolean;
}) {
  const pipelines = useService((signal) => pipelineService.listPipelines(signal), []);
  const failed = (pipelines.data?.pipelines ?? []).find((p) => p.status === "failed");
  const starters = mode === "ask" && failed ? [`Why did ${failed.name} fail?`, ...suggestions.slice(0, 2)] : suggestions;

  return (
    <motion.div
      variants={reduce ? undefined : STAGGER}
      initial="hidden"
      animate="show"
      className="flex min-h-full flex-col items-center justify-center px-2 py-8 text-center"
    >
      <motion.h2
        variants={reduce ? undefined : RISE}
        className="text-2xl font-semibold tracking-tight text-balance text-foreground sm:text-3xl"
      >
        {title}
      </motion.h2>
      <motion.div variants={reduce ? undefined : RISE} className="mt-6 grid w-full max-w-xl gap-3 sm:grid-cols-2">
        {MODES.map((m) => {
          const active = m.mode === mode;
          const Icon = m.icon;
          return (
            <button
              key={m.mode}
              type="button"
              aria-pressed={active}
              onClick={() => setMode(m.mode)}
              className={cn(
                "flex flex-col gap-1.5 rounded-xl border p-4 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50",
                active ? "border-foreground/30 bg-background" : "border-border bg-background/50 hover:bg-background",
              )}
            >
              <span className="flex items-center gap-2 text-sm font-semibold text-foreground">
                <Icon className="size-4 text-muted-foreground" />
                {m.label}
                {active ? <span className="ml-auto text-[10px] font-medium text-muted-foreground">Selected</span> : null}
              </span>
              <span className="text-xs leading-relaxed text-muted-foreground">{m.what}</span>
              <span className="text-[11px] text-muted-foreground/80">{m.safety}</span>
            </button>
          );
        })}
      </motion.div>
      <motion.p variants={reduce ? undefined : RISE} className="mt-6 text-xs text-muted-foreground">
        Try one:
      </motion.p>
      <div className="mt-2 flex flex-wrap justify-center gap-2">
        {starters.map((s) => (
          <motion.div key={`${mode}-${s}`} variants={reduce ? undefined : RISE}>
            <SuggestionButton text={s} variant="pill" onClick={() => onPick(s)} disabled={busy} />
          </motion.div>
        ))}
      </div>
    </motion.div>
  );
}

const STAGGER: Variants = {
  hidden: {},
  show: { transition: { staggerChildren: 0.07, delayChildren: 0.05 } },
};

const RISE: Variants = {
  hidden: { opacity: 0, y: 12 },
  show: { opacity: 1, y: 0, transition: { type: "spring", stiffness: 170, damping: 26 } },
};
