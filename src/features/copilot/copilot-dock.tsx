"use client";

import * as React from "react";
import { usePathname } from "next/navigation";
import { AnimatePresence, stagger, useAnimate, usePresence, useReducedMotion } from "motion/react";
import { Bot, Plus, ChevronDown, PanelRight } from "lucide-react";
import { SuggestionButton } from "@/features/copilot/suggestion-button";
import { useCopilot } from "./use-copilot";
import { ChatMessages } from "./chat-messages";
import { ChatComposer } from "./chat-composer";
import { CopilotHistoryButton } from "./history-menu";
import { useSidePanel } from "@/components/app-shell/side-panel";

/**
 * Pages that never show the floating dock: Home's own prompt is the entry
 * point there, and /copilot is the full conversation, so a second input
 * would be two boxes doing the same thing. `AppFrame` reads this too, to
 * drop the bottom padding it keeps free for the dock.
 */
export function dockHiddenOn(pathname: string | null): boolean {
  return pathname === "/" || Boolean(pathname?.startsWith("/copilot"));
}

/*
 * Open and close motion (owner QA 2026-11-15: "aku masih kurang suka animasi
 * transform ini tu pas dia kebuka dan ketutupnya"). Before, opening swapped
 * the pill for the composer box in one frame (the top edge jumped ~46 px),
 * tweened the dock's `width` (so every line of text re-wrapped each frame),
 * and scaled the chat card up from 0.98 with its text; closing removed all of
 * it in one frame and then squeezed the pill's width.
 *
 * Now the pill stays where it is and the composer grows out of it: the box's
 * clip-path starts as the pill's exact rectangle and opens to the box's own
 * outline, so only the outline changes shape, never the text or icons
 * inside. It then widens to 48 px outside the box, corners concentric with
 * the box's, which lets the shadow bloom out instead of popping in when the
 * clip is removed at the end. The
 * composer's controls fade in a beat later, the chat card rises 8 px after
 * that, and its suggestions last. Closing plays the same steps backwards,
 * quicker (exits are glanced at, entrances are watched).
 *
 * Opacity and clip are set on the glass surfaces themselves: either one on a
 * wrapper would make that wrapper a backdrop root and turn off the blur.
 *
 * Open:  0 ms pill fades out (120) · 0–200 ms box outline grows from the
 *        pill to the box, 200–240 ms its shadow blooms · 40 ms card rises
 *        8 px and fades in (200) · 60 ms box controls fade in (160) · 70 ms
 *        suggestions rise 4 px and fade in, 25 ms apart (150). Done by 245 ms.
 * Close: 0 ms card drops 6 px and fades (120) · 0 ms box controls fade (90) ·
 *        0–34 ms shadow folds in, 34–170 ms outline shrinks onto the pill ·
 *        60 ms pill fades in (110). Done by 170 ms.
 * Reduced motion: the two swap in one frame, nothing moves.
 */
const EASE_UI: [number, number, number, number] = [0.23, 1, 0.32, 1]; // the `ease-ui` token (globals.css)
// The `ease-drawer` token: a gentler start than `ease-ui`, so the outline is
// seen growing rather than arriving in two frames.
const EASE_DRAWER: [number, number, number, number] = [0.32, 0.72, 0, 1];
const SHADOW_ROOM = 48;

type Plan = { intent: boolean };

/** `inset()` that shows only the part of `surface` the pill covers. */
function pillClip(surface: HTMLElement, pill: HTMLElement | null): string {
  const s = surface.getBoundingClientRect();
  if (!pill) {
    // No pill on screen (should not happen while animating): centre a pill-sized box on the bottom edge.
    const w = Math.min(s.width, 460);
    const side = Math.max(0, (s.width - w) / 2);
    return `inset(${s.height - 46}px ${side}px 0px ${side}px round 23px)`;
  }
  const p = pill.getBoundingClientRect();
  return `inset(${p.top - s.top}px ${s.right - p.right}px ${s.bottom - p.bottom}px ${p.left - s.left}px round ${p.height / 2}px)`;
}

/**
 * The box's own outline (`edge`), and the open clip (`open`): past the box on
 * every side with corners concentric to its own, so nothing visible is cut,
 * the shadow included.
 */
function boxClips(surface: HTMLElement): { edge: string; open: string } {
  const r = Number.parseFloat(getComputedStyle(surface).borderTopLeftRadius) || 16;
  const o = -SHADOW_ROOM;
  return {
    edge: `inset(0px 0px 0px 0px round ${r}px)`,
    open: `inset(${o}px ${o}px ${o}px ${o}px round ${r + SHADOW_ROOM}px)`,
  };
}

const SURFACE = "[data-slot='chat-composer']";

/**
 * Calls `done` once the exit animations end, or after `ms` if they never
 * report it: a dropped animation must not leave an inert dock on screen.
 * (happy-dom's `Element.animate` never finishes, for one.)
 */
function whenFinished(animations: { then: (onResolve: () => void) => unknown }[], ms: number, done: () => void) {
  let called = false;
  const once = () => { if (!called) { called = true; done(); } };
  const timer = window.setTimeout(once, ms);
  void Promise.all(animations.map((a) => new Promise<void>((resolve) => { a.then(resolve); }))).then(() => {
    window.clearTimeout(timer);
    once();
  });
}

/**
 * The open dock: chat card above, full composer below. Runs its own entrance
 * and exit (it stays mounted under `AnimatePresence` until the exit ends).
 */
function DockStack({
  plan, getPill, onCollapse, children,
}: {
  plan: React.RefObject<Plan>;
  getPill: () => HTMLElement | null;
  onCollapse: () => void;
  children: React.ReactNode;
}) {
  const [scope, animate] = useAnimate<HTMLDivElement>();
  const [isPresent, safeToRemove] = usePresence();
  const reduce = useReducedMotion() ?? false;
  const wasPresent = React.useRef(true);
  const run = React.useRef(0);

  React.useLayoutEffect(() => {
    const root = scope.current;
    const surface = root?.querySelector<HTMLElement>(SURFACE);
    const card = root?.querySelector<HTMLElement>("[data-dock-part='card']");
    if (!root || !surface || !card) { if (!isPresent) safeToRemove?.(); return; }
    const id = ++run.current;
    const reentry = isPresent && !wasPresent.current;
    wasPresent.current = isPresent;
    const controls = Array.from(surface.children) as HTMLElement[];
    const chips = Array.from(root.querySelectorAll<HTMLElement>("[data-dock-part='chips'] > *"));
    const moving = plan.current.intent && !reduce;

    if (isPresent) {
      if (plan.current.intent) {
        const box = root.querySelector<HTMLTextAreaElement>("textarea");
        box?.focus({ preventScroll: true });
        box?.setSelectionRange(box.value.length, box.value.length);
      }
      if (!moving) {
        // Back before its exit ended, without the motion: settle at once.
        if (reentry) {
          surface.style.clipPath = "";
          void animate([...controls, card, ...chips], { opacity: 1, y: 0 }, { duration: 0 });
        }
        return;
      }
      if (!reentry) {
        // First frame, before paint: the box is the pill, everything else hidden.
        surface.style.clipPath = pillClip(surface, getPill());
        for (const el of controls) el.style.opacity = "0";
        card.style.opacity = "0";
        card.style.transform = "translateY(8px)";
        for (const el of chips) { el.style.opacity = "0"; el.style.transform = "translateY(4px)"; }
      }
      const from = surface.style.clipPath || pillClip(surface, getPill());
      const { edge, open } = boxClips(surface);
      void animate(
        surface,
        { clipPath: [from, edge, open] },
        { duration: 0.24, times: [0, 0.83, 1], ease: [EASE_DRAWER, "easeOut"] },
      ).then(() => {
        if (run.current === id) surface.style.clipPath = "";
      });
      void animate(controls, { opacity: 1 }, { duration: 0.16, delay: reentry ? 0 : 0.06, ease: EASE_UI });
      void animate(card, { opacity: 1, y: 0 }, { duration: 0.2, delay: reentry ? 0 : 0.04, ease: EASE_UI });
      if (chips.length) void animate(chips, { opacity: 1, y: 0 }, { duration: 0.15, delay: stagger(0.025, { startDelay: reentry ? 0 : 0.07 }), ease: EASE_UI });
      return;
    }

    // Exit.
    if (!moving) { safeToRemove?.(); return; }
    const { edge, open } = boxClips(surface);
    const from = surface.style.clipPath || open;
    whenFinished([
      animate(card, { opacity: 0, y: 6 }, { duration: 0.12, ease: EASE_UI }),
      animate(controls, { opacity: 0 }, { duration: 0.09, ease: EASE_UI }),
      animate(
        surface,
        { clipPath: [from, edge, pillClip(surface, getPill())] },
        { duration: 0.17, times: [0, 0.2, 1], ease: ["easeIn", EASE_DRAWER] },
      ),
    ], 400, () => {
      if (run.current === id) safeToRemove?.();
    });
    // `plan`, `getPill` and `animate` are read when presence changes; they are not triggers.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isPresent]);

  return (
    <div
      ref={scope}
      inert={!isPresent}
      onKeyDown={(e) => {
        // A menu or popover opened from inside (Tools, history) is portalled
        // out of this element, yet its keys still bubble here through React:
        // its Escape closes that menu only. A bare Escape folds the dock.
        if (e.key !== "Escape" || e.defaultPrevented || !e.currentTarget.contains(e.target as Node)) return;
        e.preventDefault();
        onCollapse();
      }}
      className="pointer-events-auto w-full [grid-area:1/1] self-end"
    >
      {children}
    </div>
  );
}

/** The collapsed pill. Fades out under the growing box, and back in as the box closes onto it. */
function DockPill({
  plan, focusRef, children,
}: {
  plan: React.RefObject<Plan>;
  focusRef: React.RefObject<boolean>;
  children: React.ReactNode;
}) {
  const [scope, animate] = useAnimate<HTMLDivElement>();
  const [isPresent, safeToRemove] = usePresence();
  const reduce = useReducedMotion() ?? false;
  const run = React.useRef(0);

  React.useLayoutEffect(() => {
    const surface = scope.current?.querySelector<HTMLElement>(SURFACE);
    if (!surface) { if (!isPresent) safeToRemove?.(); return; }
    const id = ++run.current;
    const moving = plan.current.intent && !reduce;
    if (isPresent) {
      if (plan.current.intent) {
        // Focus comes back to where the dock was opened from, without opening it again.
        focusRef.current = true;
        scope.current?.querySelector("input")?.focus({ preventScroll: true });
        focusRef.current = false;
      }
      if (!moving) { void animate(surface, { opacity: 1 }, { duration: 0 }); return; }
      if (surface.style.opacity === "") surface.style.opacity = "0";
      void animate(surface, { opacity: 1 }, { duration: 0.11, delay: 0.06, ease: EASE_UI });
      return;
    }
    if (!moving) { safeToRemove?.(); return; }
    whenFinished([animate(surface, { opacity: 0 }, { duration: 0.12, ease: EASE_UI })], 400, () => {
      if (run.current === id) safeToRemove?.();
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isPresent]);

  return (
    <div ref={scope} data-dock-pill inert={!isPresent} className="pointer-events-auto w-full max-w-[460px] justify-self-center [grid-area:1/1] self-end">
      {children}
    </div>
  );
}

/**
 * The GLOBAL chat dock — a bar at the BOTTOM-CENTER of every page (Google
 * Cloud Assist style). The input bar stays visible while the dock position
 * is bottom; when there's a conversation / it's focused, the chat panel
 * RISES above the bar (Ask/Build, Tools menu, tool rendering, build tree).
 * The user can switch to a resizable right sidebar via the dock toggle in
 * the header. Shares the brain & history with the /copilot page via
 * useCopilot. Hidden on /copilot (already a full page) or while the dock
 * position is right.
 */
export function CopilotDock() {
  const pathname = usePathname();
  const c = useCopilot();
  const panel = useSidePanel();
  const [draft, setDraft] = React.useState("");
  const wrapper = React.useRef<HTMLDivElement>(null);
  // Set by the dock's own open and close, read by the two halves when they
  // mount or leave: an open restored from the last visit, or changed from
  // elsewhere, shows up in place without playing the motion or moving focus.
  const plan = React.useRef<Plan>({ intent: false });
  const restoringFocus = React.useRef(false);
  const showPanel = c.expanded;

  React.useEffect(() => { plan.current.intent = false; }, [showPanel]);

  const { setExpanded } = c;
  const open = React.useCallback(() => {
    if (restoringFocus.current) return;
    plan.current.intent = true;
    setExpanded(true);
  }, [setExpanded]);
  const collapse = React.useCallback(() => {
    plan.current.intent = true;
    setExpanded(false);
  }, [setExpanded]);
  const getPill = React.useCallback(
    () => wrapper.current?.querySelector<HTMLElement>(`[data-dock-pill] ${SURFACE}`) ?? null,
    [],
  );

  // BI-61 T6: while a page tab of the shared side panel is open, the Copilot is
  // that panel's first tab, so the bottom bar steps aside rather than float as
  // a second way in over the panel.
  if (dockHiddenOn(pathname) || c.dockPosition === "right" || panel.active !== null) {
    return null;
  }

  const composer = (compact: boolean) => (
    <ChatComposer onStop={c.stop}
      glass
      compact={compact}
      mode={c.mode}
      setMode={c.setMode}
      value={draft}
      onValueChange={(t) => {
        setDraft(t);
        // Typing into the pill (focus came back to it on close) opens the dock with the text in it.
        if (compact && t) open();
      }}
      onSend={(t) => {
        if (!showPanel) plan.current.intent = true;
        c.setExpanded(true);
        c.send(t);
      }}
      busy={c.busy}
      rows={1}
      placeholder="Ask anything about your lakehouse…"
      enabledCaps={c.enabledCaps}
      toggleCap={c.toggleCap}
      onFocus={compact ? open : undefined}
    />
  );

  return (
    <>
    {/* The bar floats over the page. Owner QA 2026-11-14, composer: it is centred on the content
        area, right of the sidebar (`--content-left`, set by AppFrame), not on the window, and it
        follows the sidebar as it folds. The room under the page is AppFrame's `--content-bottom`
        (`ComposerReserve`); the 5rem spacer this rendered on top of that is gone. */}
    <div className="pointer-events-none fixed right-0 bottom-4 left-(--content-left,0px) z-50 flex justify-center px-4 transition-[left] duration-200 ease-ui print:hidden">
    {/* One cell, as wide as the open dock, that the pill and the open stack
        share, both pinned to its bottom centre: neither half moves the other,
        and the page behind never shifts. The cell itself lets clicks through;
        only the halves take them. */}
    <div ref={wrapper} className="grid w-full max-w-[560px]">
      <AnimatePresence initial={false}>
        {showPanel ? (
          <DockStack key="stack" plan={plan} getPill={getPill} onCollapse={collapse}>
            <div data-dock-part="card" className="mb-2 flex max-h-[58vh] flex-col overflow-hidden rounded-2xl border border-border/70 bg-card/90 shadow-[0_8px_40px_rgba(0,0,0,0.18)] backdrop-blur-xl supports-backdrop-filter:bg-card/80">
              <div className="flex items-center gap-2 border-b border-border px-3 py-2">
                <span className="grid size-6 place-items-center rounded-md bg-primary/10 text-primary">
                  <Bot className="size-3.5" aria-hidden />
                </span>
                <span className="text-sm font-semibold">AI Copilot</span>
                <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
                  {c.mode}
                </span>
                <div className="ml-auto flex items-center gap-0.5">
                  <CopilotHistoryButton align="end" />
                  <button
                    type="button"
                    onClick={() => {
                      c.setDockPosition("right");
                      c.setExpanded(true);
                    }}
                    aria-label="Dock to right sidebar"
                    title="Dock to right sidebar"
                    className="grid size-7 place-items-center rounded-md text-muted-foreground transition-[background-color,color,scale] duration-150 ease-ui hover:bg-muted hover:text-foreground active:scale-95"
                  >
                    <PanelRight className="size-4" />
                  </button>
                  <button
                    type="button"
                    onClick={c.newChat}
                    aria-label="New chat"
                    title="New chat"
                    className="grid size-7 place-items-center rounded-md text-muted-foreground transition-[background-color,color,scale] duration-150 ease-ui hover:bg-muted hover:text-foreground active:scale-95"
                  >
                    <Plus className="size-4" />
                  </button>
                  <button
                    type="button"
                    onClick={collapse}
                    aria-label="Collapse panel"
                    title="Collapse panel"
                    className="grid size-7 place-items-center rounded-md text-muted-foreground transition-[background-color,color,scale] duration-150 ease-ui hover:bg-muted hover:text-foreground active:scale-95"
                  >
                    <ChevronDown className="size-4" />
                  </button>
                </div>
              </div>
              <div className="flex-1 overflow-y-auto p-3">
                {c.messages.length === 0 ? (
                  <div className="space-y-3 pt-1">
                    <div>
                      <p className="text-sm font-medium text-foreground">{c.pageContext.title}</p>
                      <p className="text-xs text-muted-foreground">{c.pageContext.hint}</p>
                    </div>
                    <div data-dock-part="chips" className="grid gap-1.5 sm:grid-cols-2">
                      {c.pageContext.suggest[c.mode].map((s) => (
                        <SuggestionButton key={s} text={s} onClick={() => c.send(s)} disabled={c.busy} />
                      ))}
                    </div>
                  </div>
                ) : (
                  <ChatMessages
                    messages={c.messages} onEdit={c.editAndResend} onDelete={c.deleteMessage} onDismissError={c.clearError}
                    busy={c.busy}
                    error={c.error} onRetry={c.retry}
                    onConfirmTool={c.confirmTool}
                    onCancelTool={c.cancelTool}
                    confirmingKey={c.confirmingKey}
                    onAnswerAsk={c.answerAskOption} askNotice={c.askNotice}
                  />
                )}
              </div>
            </div>
            <ComposerReserve>{composer(false)}</ComposerReserve>
          </DockStack>
        ) : (
          <DockPill key="pill" plan={plan} focusRef={restoringFocus}>
            <div
              // A first click opens through the input's focus. Clicking the pill
              // while it already has focus (it gets it back on close) fires no
              // focus event, so the click opens it. Not on pointerdown: the
              // pill turns inert at once, and the mousedown that follows would
              // land on the box behind it and blur the textarea just focused.
              onClick={(e) => { if (!(e.target as HTMLElement).closest("button")) open(); }}
              onKeyDown={(e) => { if (e.key === "Enter" && !draft.trim()) open(); }}
            >
              <ComposerReserve>{composer(true)}</ComposerReserve>
            </div>
          </DockPill>
        )}
      </AnimatePresence>
    </div>
    </div>
    </>
  );
}

/** The bar's distance from the window's bottom edge (`bottom-4`) and the gap kept above it. */
export const DOCK_OFFSET_PX = 16;
export const DOCK_GAP_PX = 14;

/**
 * Owner QA 2026-11-14, composer reserve: the room a page keeps under its content while the floating
 * composer shows, in px: the composer's own height, its offset from the bottom, and a 14 px gap.
 */
export function dockReserve(composerHeight: number): number {
  return Math.round(composerHeight + DOCK_OFFSET_PX + DOCK_GAP_PX);
}

/** The composers on screen. While the dock opens or closes the pill and the box are both there for a moment. */
const reserving = new Set<HTMLElement>();

function updateReserve() {
  const root = document.documentElement;
  if (reserving.size === 0) {
    root.style.removeProperty("--dock-reserve");
    return;
  }
  const height = Math.max(...Array.from(reserving, (el) => el.getBoundingClientRect().height));
  root.style.setProperty("--dock-reserve", `${dockReserve(height)}px`);
}

/**
 * Owner QA 2026-11-14, composer reserve: while a composer is on screen, `--dock-reserve` on the
 * document is the room it takes (`dockReserve` of its measured height, kept current as it grows); it
 * is removed when the dock hides itself (the Copilot side panel, a page without the dock), so
 * AppFrame's `--content-bottom` falls back to the page's normal padding. The composer's height,
 * not the open chat card above it, is what is reserved. The pill and the open box each wrap their
 * composer in one of these, and both are mounted while one hands over to the other (the open and
 * close motion), so the reserve is the taller of those on screen and goes only when the last one
 * leaves: the pill leaving after the box arrived must not drop it.
 */
function ComposerReserve({ children }: { readonly children: React.ReactNode }) {
  const ref = React.useRef<HTMLDivElement>(null);
  React.useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    reserving.add(el);
    updateReserve();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(updateReserve);
    observer?.observe(el);
    return () => {
      observer?.disconnect();
      reserving.delete(el);
      updateReserve();
    };
  }, []);
  return <div ref={ref} data-slot="dock-composer">{children}</div>;
}
