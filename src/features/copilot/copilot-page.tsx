"use client";

import * as React from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { Sparkles } from "lucide-react";
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
 * Halaman AI Copilot — tampilan chat ala RantAI-Agents: avatar per pesan,
 * welcome ketengah dengan pill saran, composer lembut.
 *
 * RIWAYAT ada di atas chat (`CopilotHistoryMenu`), bukan di sidebar kiri:
 * daftar itu dulu hanya muncul ketika sudah berada di /copilot, jadi ia
 * memakai ruang navigasi untuk isi halaman, dan ikut hilang saat sidebar
 * diciutkan. Otak & riwayat tetap dibagi dengan chat dock global lewat
 * useCopilot.
 */
export function CopilotPage() {
  const c = useCopilot();

  // Viewport minus the 4rem navbar and `AppFrame`'s vertical padding, so the
  // message list scrolls on its own and the composer stays on the bottom edge.
  return (
    <div className="flex h-[calc(100svh-6rem)] flex-col gap-3 sm:h-[calc(100svh-6.5rem)] lg:h-[calc(100svh-7rem)]">
      <React.Suspense fallback={null}>
        <SessionUrlSync />
      </React.Suspense>

      {/* Riwayat percakapan — menampilkan percakapan aktif, last update, dan link ke halaman riwayat */}
      <CopilotHistoryMenu
        sessions={c.sessions}
        activeId={c.sessionId}
        onSelect={(id) => void c.loadSession(id)}
        onNew={() => c.newChat()}
      />
      <div className="mx-auto flex min-h-0 w-full max-w-3xl flex-1 flex-col">
        <div className="min-h-0 flex-1 overflow-y-auto pr-0.5">
          {c.messages.length === 0 ? (
            <div className="flex flex-col items-center justify-center px-4 py-16 text-center">
              <div className="mb-4 grid size-12 place-items-center rounded-2xl border border-violet-500/20 bg-gradient-to-br from-violet-500/15 to-purple-600/15 text-violet-600 dark:text-violet-400">
                <Sparkles className="size-6" />
              </div>
              <h2 className="text-xl font-semibold text-foreground">Hi 👋 {c.pageContext.title}</h2>
              <p className="mt-1.5 max-w-md text-sm text-muted-foreground">{c.pageContext.hint}</p>
              <div className="mt-5 flex flex-wrap justify-center gap-2">
                {c.pageContext.suggest[c.mode].map((s) => (
                  <button
                    key={s}
                    onClick={() => c.send(s)}
                    disabled={c.busy}
                    className="inline-flex items-center gap-2 rounded-full border border-border bg-background px-4 py-2 text-sm font-medium text-foreground/80 transition-all hover:border-primary/40 hover:bg-muted/50 hover:text-foreground disabled:opacity-50"
                  >
                    {s}
                  </button>
                ))}
              </div>
            </div>
          ) : (
            <ChatMessages
              messages={c.messages} busy={c.busy} error={c.error} progress={c.progress} onRetry={c.retry}
              onConfirmTool={c.confirmTool} onCancelTool={c.cancelTool} onCompleteTool={c.completeToolStep} confirmingKey={c.confirmingKey}
            />
          )}
        </div>

        <div className="shrink-0 pt-3">
          <ChatComposer onStop={c.stop}
            mode={c.mode} setMode={c.setMode} onSend={c.send} busy={c.busy}
            enabledCaps={c.enabledCaps} toggleCap={c.toggleCap}
            placeholder="Ask anything about your lakehouse data…"
          />
        </div>
      </div>
    </div>
  );
}
