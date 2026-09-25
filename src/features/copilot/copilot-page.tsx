"use client";

import * as React from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { motion, useReducedMotion, type Variants } from "motion/react";
import { Sparkles } from "lucide-react";
import { BrandBackdrop } from "@/components/ui/brand-glow";
import { SuggestionButton } from "@/features/copilot/suggestion-button";
import { useCopilot } from "./use-copilot";
import { ChatMessages } from "./chat-messages";
import { ChatComposer } from "./chat-composer";
import { CopilotHistoryMenu } from "./history-menu";

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
 * The AI Copilot page — a RantAI-Agents-style chat view: a per-message
 * avatar, a centered welcome with suggestion pills, a soft composer.
 *
 * Styled like Home (`BrandBackdrop`, `GlowFrame`, the Copilot badge) so
 * moving from Home's prompt into the conversation feels like one surface.
 *
 * History renders above the chat (`CopilotHistoryMenu`), not in the left
 * sidebar: that list used to appear only once already on /copilot, so it
 * spent navigation space on page content, and disappeared along with the
 * sidebar when it was collapsed. The brain & history are still shared with
 * the global chat dock via useCopilot.
 */
export function CopilotPage() {
  const c = useCopilot();
  const reduce = useReducedMotion() ?? false;

  // Viewport minus the 4rem navbar and `AppFrame`'s vertical padding, so the
  // message list scrolls on its own and the composer stays on the bottom edge.
  return (
    <div className="relative isolate flex h-[calc(100svh-6rem)] flex-col overflow-hidden rounded-3xl border border-border/60 bg-card/40 sm:h-[calc(100svh-6.5rem)] lg:h-[calc(100svh-7rem)]">
      <BrandBackdrop />
      <React.Suspense fallback={null}>
        <SessionUrlSync />
      </React.Suspense>

      {/* Conversation history — shows the active conversation, last update, and a link to the history page */}
      <div className="shrink-0 border-b border-border/50 bg-background/40 px-3 py-2 backdrop-blur-md">
        <CopilotHistoryMenu
          sessions={c.sessions}
          activeId={c.sessionId}
          onSelect={(id) => void c.loadSession(id)}
          onNew={() => c.newChat()}
        />
      </div>

      <div className="mx-auto flex min-h-0 w-full max-w-3xl flex-1 flex-col px-3 sm:px-4">
        <div className="min-h-0 flex-1 overflow-y-auto py-6 pr-0.5">
          {c.messages.length === 0 ? (
            <motion.div
              key={c.mode}
              variants={reduce ? undefined : STAGGER}
              initial="hidden"
              animate="show"
              className="flex min-h-full flex-col items-center justify-center px-2 py-10 text-center"
            >
              <motion.span
                variants={reduce ? undefined : RISE}
                className="mb-3 inline-flex items-center gap-1.5 rounded-full border border-[color-mix(in_oklch,var(--brand-1),transparent_60%)] bg-[color-mix(in_oklch,var(--brand-1),transparent_88%)] px-3 py-1 text-xs font-medium text-[var(--brand-1)]"
              >
                <Sparkles className="size-3" /> Copilot · {c.mode === "build" ? "Build mode" : "Ask mode"}
              </motion.span>
              <motion.h2
                variants={reduce ? undefined : RISE}
                className="text-3xl font-semibold tracking-[-0.03em] text-balance text-foreground sm:text-4xl"
              >
                {c.pageContext.title}
              </motion.h2>
              <motion.p variants={reduce ? undefined : RISE} className="mt-2 max-w-md text-sm text-muted-foreground sm:text-base">
                {c.pageContext.hint}
              </motion.p>
              <div className="mt-7 flex flex-wrap justify-center gap-2">
                {c.pageContext.suggest[c.mode].map((s) => (
                  <motion.div key={s} variants={reduce ? undefined : RISE}>
                    <SuggestionButton text={s} variant="pill" onClick={() => c.send(s)} disabled={c.busy} />
                  </motion.div>
                ))}
              </div>
            </motion.div>
          ) : (
            <ChatMessages
              avatars
              messages={c.messages} draft={c.draft} busy={c.busy} error={c.error} progress={c.progress} onRetry={c.retry}
              onConfirmTool={c.confirmTool} onCancelTool={c.cancelTool} onCompleteTool={c.completeToolStep} confirmingKey={c.confirmingKey}
            />
          )}
        </div>

        <div className="shrink-0 pb-3 sm:pb-4">
          <ChatComposer onStop={c.stop} glow
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
