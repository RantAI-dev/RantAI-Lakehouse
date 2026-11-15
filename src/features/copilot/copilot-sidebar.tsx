"use client";

import * as React from "react";
import { usePathname } from "next/navigation";
import { Bot, Ellipsis, History, Plus, PanelBottom } from "lucide-react";
import { useSidePanel } from "@/components/app-shell/side-panel";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { SuggestionButton } from "@/features/copilot/suggestion-button";
import { useCopilot } from "./use-copilot";
import { ChatMessages } from "./chat-messages";
import { ChatComposer } from "./chat-composer";
import { CopilotHistoryButton } from "./history-menu";
import { RecentSessionsMenuContent } from "./session-parts";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { useSlidingIndicator } from "@/hooks/use-sliding-indicator";
import { cn } from "@/lib/utils";

/**
 * Resizable right sidebar for AI Copilot.
 * Docks alongside main page content so tables, charts, and metrics are not obscured.
 * Supports drag-to-resize, double-click to reset width, mode badge, and dock position switching.
 *
 * `BI-61` T6: it is also the shell's one side panel (`side-panel.tsx`). On a
 * page that registered tabs (the dashboard page: Comments and Details) the
 * header becomes a tab row, "AI Copilot" first, and the panel shows whichever
 * is chosen, so the Copilot and a page panel never stand side by side. It then
 * opens for a page tab even while the Copilot is docked at the bottom or
 * collapsed, and for someone without the assistant, who gets the page tabs
 * alone. On every other page it is the Copilot's sidebar as before.
 */
/**
 * The narrowest the panel gets: "AI Copilot", the dashboard's Comments and
 * Details tabs, the ⋯ menu and the dock button side by side at their full
 * size (owner QA 2026-11-15: a tab label must never be cut).
 */
const MIN_WIDTH = 360;

export function CopilotSidebar({ hasAssistant = true }: { readonly hasAssistant?: boolean }) {
  const pathname = usePathname();
  const c = useCopilot();
  const { tabs: pageTabs, active: activeTab, setHost, open: openTab, close: closePanel, dismiss } = useSidePanel();
  const [isDragging, setIsDragging] = React.useState(false);
  const [aside, setAside] = React.useState<HTMLElement | null>(null);

  // Owner QA 2026-11-15: Escape closes the panel while the focus is in it.
  // A native listener, not `onKeyDown`: a page tab's content is a portal from
  // the page, so its keys bubble through the page's React tree, never this
  // one; through the DOM they do reach the panel. A menu opened from the
  // panel is portalled to <body> and keeps its Escape to itself.
  React.useEffect(() => {
    if (!aside) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      e.preventDefault();
      dismiss();
    };
    aside.addEventListener("keydown", onKey);
    return () => aside.removeEventListener("keydown", onKey);
  }, [aside, dismiss]);

  const copilotOpen = hasAssistant && !pathname?.startsWith("/copilot") && c.dockPosition === "right" && c.expanded;
  const pageTab = pageTabs.find((t) => t.id === activeTab);
  // Hidden on dedicated /copilot page or when dock position is bottom or collapsed, unless a page tab is open
  if (!copilotOpen && !pageTab) {
    return null;
  }
  const tabbed = pageTabs.length > 0;

  // A width saved before the minimum went up still opens at the minimum.
  const width = Math.max(MIN_WIDTH, c.sidebarWidth);
  const copilotShown = !pageTab;
  const maxWidth = typeof window === "undefined" ? 850 : Math.max(MIN_WIDTH, Math.min(850, window.innerWidth - 320));
  const resizeTo = (w: number) => c.setSidebarWidth(Math.round(Math.max(MIN_WIDTH, Math.min(maxWidth, w))));
  // Owner QA 2026-11-15: a click on the edge folds the panel away; the Copilot goes back to the bottom bar.
  const collapse = () => {
    dismiss();
    if (hasAssistant) {
      if (c.dockPosition !== "bottom") c.setDockPosition("bottom");
      c.setExpanded(false);
    }
  };
  const toBottom = () => {
    if (pageTab) {
      // From a page tab: the panel closes and the Copilot waits in the bottom bar.
      dismiss();
      if (c.dockPosition !== "bottom") c.setDockPosition("bottom");
      c.setExpanded(false);
    } else {
      // From the Copilot: the conversation moves to the floating dock, open.
      c.setDockPosition("bottom");
    }
  };

  return (
    <aside
      ref={setAside}
      data-slot="copilot-sidebar"
      // BI-61 T6: with page tabs the panel covers the page on a phone (a side
      // column there would leave the page no width) and takes its width from md up.
      style={tabbed
        ? ({ "--side-panel-w": `${width}px` } as React.CSSProperties)
        : { width: `${width}px`, minWidth: `${width}px`, maxWidth: "850px" }}
      className={cn(
        "sticky top-(--app-navbar-h) flex h-[calc(100vh-var(--app-navbar-h))] shrink-0 flex-col border-l border-border bg-card/95 backdrop-blur-md",
        // Micro-interaction audit 2026-11-14 (emil): the panel arrives from the edge it docks to (a short
        // slide and fade on the drawer curve) instead of appearing in one frame; closing stays instant, so
        // the page gets its width back at once. On a phone it rises from the bottom.
        "transition-[opacity,translate] duration-[260ms] ease-drawer starting:opacity-0 md:starting:translate-x-4 max-md:starting:translate-y-4",
        tabbed && "max-md:fixed max-md:inset-x-0 max-md:bottom-0 max-md:z-40 max-md:h-auto max-md:border-l-0 max-md:bg-card md:w-(--side-panel-w) md:max-w-[850px] md:min-w-(--side-panel-w)",
        isDragging && "select-none",
      )}
    >
      {hasAssistant ? (
        <EdgeHandle
          width={width}
          min={MIN_WIDTH}
          max={maxWidth}
          dragging={isDragging}
          onDragging={setIsDragging}
          onResize={resizeTo}
          onCollapse={collapse}
        />
      ) : null}

      {/* Owner QA 2026-11-15 ("panel copilotnya ada dua component kah?"): one
          header for the Copilot alone and for the dashboard's tab row. */}
      <PanelHeader
        tabs={pageTabs}
        active={pageTab?.id ?? null}
        withCopilot={hasAssistant}
        onCopilot={() => {
          closePanel();
          if (c.dockPosition !== "right") c.setDockPosition("right");
          c.setExpanded(true);
        }}
        onTab={openTab}
        copilotTools={hasAssistant && copilotShown}
        onDock={hasAssistant ? toBottom : null}
      />
      {pageTab ? (
        // The page draws the tab's content here (a portal), so it keeps its own state.
        <div ref={setHost} data-slot="side-panel-body" className="flex min-h-0 flex-1 flex-col overflow-hidden" />
      ) : (
      <>
      {/* Messages or suggestions body */}
      <div className="flex-1 overflow-y-auto p-4">
        {c.messages.length === 0 ? (
          <div className="space-y-4 pt-1">
            <div>
              <p className="text-sm font-semibold text-foreground">{c.pageContext.title}</p>
              <p className="mt-0.5 text-xs text-muted-foreground">{c.pageContext.hint}</p>
            </div>
            <div className="space-y-2">
              <p className="text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
                Suggested queries
              </p>
              <div className="grid gap-2">
                {c.pageContext.suggest[c.mode].map((s) => (
                  <SuggestionButton key={s} text={s} onClick={() => c.send(s)} disabled={c.busy} />
                ))}
              </div>
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

      {/* Composer footer */}
      <div className="shrink-0 border-t border-border bg-card/60 p-3">
        <ChatComposer onStop={c.stop}
          mode={c.mode}
          setMode={c.setMode}
          onSend={c.send}
          busy={c.busy}
          rows={2}
          placeholder="Ask anything about your lakehouse data…"
          enabledCaps={c.enabledCaps}
          toggleCap={c.toggleCap}
        />
      </div>
      </>
      )}
    </aside>
  );
}

const TOOL = "grid size-8 shrink-0 place-items-center rounded-md text-muted-foreground transition-[background-color,color,scale] duration-150 ease-ui hover:bg-muted hover:text-foreground active:scale-95 focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-ring/50";
/** One 32 px tool and the 2 px gap before it. */
const TOOL_SPAN = 34;

/**
 * The panel's one header (owner QA 2026-11-15), 48 px tall with 32 px
 * controls 8 px from each edge, like the app's other bars (DESIGN.md §3).
 *
 * On the left, "AI Copilot", then the page's tabs when it has any (the
 * dashboard: Comments, Details) as one row of quiet buttons, the chosen one
 * filled. Without page tabs the same label stands alone as a plain title, at
 * the same place, so the two cases look like one panel. The mode badge is
 * gone from both: the composer's Ask/Build switch already shows the mode.
 *
 * On the right, while the Copilot is shown, its history and New chat, and
 * always last the dock button, which is also how the panel closes now that
 * it has no close button. Tab labels are never cut: when the row runs out of
 * room, history and New chat fold into a ⋯ menu first, and `MIN_WIDTH` keeps
 * room for the rest.
 */
function PanelHeader({
  tabs, active, withCopilot, onCopilot, onTab, copilotTools, onDock,
}: {
  readonly tabs: readonly { id: string; label: string }[];
  /** The page tab shown; null when the Copilot is. */
  readonly active: string | null;
  readonly withCopilot: boolean;
  readonly onCopilot: () => void;
  readonly onTab: (id: string) => void;
  /** Whether the Copilot's history and New chat belong in the row. */
  readonly copilotTools: boolean;
  /** Moves the Copilot to the bottom dock, closing the panel; null without the assistant. */
  readonly onDock: (() => void) | null;
}) {
  const c = useCopilot();
  const tabbed = tabs.length > 0;
  // Micro-interaction audit 2026-11-14 (emil): the chosen tab's fill is one pill that slides to the tab
  // chosen, so the eye follows the switch; the tabs themselves only change colour and give under a press.
  const { listRef, indicatorRef } = useSlidingIndicator(`${active ?? "copilot"}|${tabs.map((t) => t.id).join(",")}`, { box: true });
  const headerRef = React.useRef<HTMLDivElement>(null);
  const [folded, setFolded] = React.useState(false);

  React.useLayoutEffect(() => {
    const header = headerRef.current;
    const left = listRef.current;
    if (!header || !left || !copilotTools) { setFolded(false); return; }
    const measure = () => {
      const style = getComputedStyle(header);
      const room = header.clientWidth - Number.parseFloat(style.paddingLeft || "0") - Number.parseFloat(style.paddingRight || "0");
      // The tabs at their full width, the gap after them, history and New chat, the dock button.
      const need = left.scrollWidth + 4 + 2 * TOOL_SPAN + (onDock ? TOOL_SPAN : 0);
      // Not laid out (a test's DOM, a hidden panel): nothing to fold for.
      setFolded(room > 0 && need > room);
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(header);
    ro.observe(left);
    return () => ro.disconnect();
  }, [copilotTools, onDock, listRef, tabs]);

  const tab = (on: boolean) => cn(
    "inline-flex h-8 shrink-0 items-center gap-1.5 whitespace-nowrap rounded-lg px-2.5 text-sm font-medium outline-none transition-[background-color,color,scale] duration-200 ease-ui focus-visible:ring-3 focus-visible:ring-ring/50 active:scale-[0.97]",
    on ? "text-foreground" : "text-muted-foreground hover:bg-muted/60 hover:text-foreground",
  );
  const mark = <Bot className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />;

  return (
    <div ref={headerRef} data-slot="side-panel-header" className="flex h-12 shrink-0 items-center gap-1 border-b border-border px-2">
      <div
        ref={listRef}
        role={tabbed ? "tablist" : undefined}
        aria-label={tabbed ? "Side panel" : undefined}
        className="relative isolate flex shrink-0 items-center gap-0.5"
      >
        {tabbed ? (
          <>
            <span ref={indicatorRef} aria-hidden data-slot="panel-tab-indicator"
              className="pointer-events-none absolute top-0 left-0 -z-10 rounded-lg bg-muted data-ready:transition-[transform,width] data-ready:duration-[240ms] data-ready:ease-move" />
            {withCopilot ? (
              <button type="button" role="tab" aria-selected={active === null} onClick={onCopilot} className={tab(active === null)}
                data-indicator-item data-indicator-active={active === null || undefined}>
                {/* BI-61 review round 1, SHOULD-FIX 2: no sparkles icon on what this branch adds (DESIGN.md, AI slop). */}
                {mark} AI Copilot
              </button>
            ) : null}
            {tabs.map((t) => (
              <button key={t.id} type="button" role="tab" aria-selected={active === t.id} onClick={() => onTab(t.id)} className={tab(active === t.id)}
                data-indicator-item data-indicator-active={active === t.id || undefined}>
                {t.label}
              </button>
            ))}
          </>
        ) : (
          // Alone, the label is a title, not a tab: same place and type, no fill or hover (DESIGN.md: what
          // cannot be clicked does not look clickable).
          <h2 className="inline-flex h-8 items-center gap-1.5 whitespace-nowrap px-2.5 text-sm font-medium text-foreground">
            {mark} AI Copilot
          </h2>
        )}
      </div>
      <div className="ml-auto flex shrink-0 items-center gap-0.5">
        {copilotTools && !folded ? (
          <>
            <CopilotHistoryButton align="end" className="size-8" />
            <button type="button" onClick={c.newChat} aria-label="New chat" title="New chat" className={TOOL}>
              <Plus className="size-4" />
            </button>
          </>
        ) : null}
        {copilotTools && folded ? (
          <DropdownMenu>
            <DropdownMenuTrigger render={<button type="button" aria-label="More AI Copilot actions" title="More" className={TOOL} />}>
              <Ellipsis className="size-4" />
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-48">
              <DropdownMenuItem onClick={c.newChat}>
                <Plus /> New chat
              </DropdownMenuItem>
              <DropdownMenuSub>
                <DropdownMenuSubTrigger>
                  <History /> Chat history
                </DropdownMenuSubTrigger>
                <DropdownMenuSubContent className="w-80">
                  <RecentSessionsMenuContent
                    sessions={c.sessions}
                    activeId={c.sessionId}
                    onSelect={(id) => { void c.loadSession(id); c.setExpanded(true); }}
                    onNew={c.newChat}
                  />
                </DropdownMenuSubContent>
              </DropdownMenuSub>
            </DropdownMenuContent>
          </DropdownMenu>
        ) : null}
        {onDock ? (
          <button type="button" onClick={onDock} aria-label="Switch to bottom floating dock" title="Switch to bottom floating dock" className={TOOL}>
            <PanelBottom className="size-4" />
          </button>
        ) : null}
      </div>
    </div>
  );
}

/** Below this many pixels of travel a press on the edge is a click, past it a drag. */
const DRAG_THRESHOLD = 4;
/** One arrow key press on the focused edge. */
const KEY_STEP = 16;

/**
 * The panel's left edge (owner QA 2026-11-15: "kalo di click doang dia bakal
 * collapse, tapi kalo di hold dan di drag dia bakal resize"). A press that
 * travels less than `DRAG_THRESHOLD` px before it is released is a click and
 * folds the panel away; once it passes the threshold it is a drag that
 * resizes, and letting go then never folds. The pointer is captured, so a
 * fast drag that leaves the edge keeps resizing, and no `click` handler is
 * used, so a drag cannot also count as one. The panel already closes in one
 * frame (its entrance is the only motion, so the page gets its width back at
 * once), which reduced motion leaves as it is.
 *
 * It is a focusable window splitter: Left and Right resize by `KEY_STEP`,
 * Enter and Space fold the panel. The old double click to reset the width is
 * gone: its first click now folds the panel.
 */
function EdgeHandle({
  width, min, max, dragging, onDragging, onResize, onCollapse,
}: {
  readonly width: number;
  readonly min: number;
  readonly max: number;
  readonly dragging: boolean;
  readonly onDragging: (on: boolean) => void;
  readonly onResize: (width: number) => void;
  readonly onCollapse: () => void;
}) {
  const press = React.useRef<{ x: number; y: number; width: number; moved: boolean } | null>(null);
  const [tip, setTip] = React.useState(false);

  const end = (el: HTMLElement, pointerId: number) => {
    press.current = null;
    try { el.releasePointerCapture(pointerId); } catch { /* not captured */ }
    onDragging(false);
    document.body.style.removeProperty("cursor");
    document.body.style.removeProperty("user-select");
  };

  return (
    <Tooltip open={tip && !dragging} onOpenChange={setTip}>
      <TooltipTrigger
        render={
          <div
            role="separator"
            aria-orientation="vertical"
            aria-label="Resize AI Copilot sidebar"
            aria-valuenow={width}
            aria-valuemin={min}
            aria-valuemax={max}
            tabIndex={0}
            onPointerDown={(e) => {
              if (e.button !== 0) return;
              e.preventDefault();
              try { e.currentTarget.setPointerCapture(e.pointerId); } catch { /* the pointer is gone already */ }
              press.current = { x: e.clientX, y: e.clientY, width, moved: false };
            }}
            onPointerMove={(e) => {
              const p = press.current;
              if (!p) return;
              if (!p.moved) {
                if (Math.hypot(e.clientX - p.x, e.clientY - p.y) < DRAG_THRESHOLD) return;
                p.moved = true;
                onDragging(true);
                document.body.style.cursor = "col-resize";
                document.body.style.userSelect = "none";
              }
              // The panel is on the right: dragging its left edge left makes it wider.
              onResize(p.width + (p.x - e.clientX));
            }}
            onPointerUp={(e) => {
              const p = press.current;
              if (!p) return;
              end(e.currentTarget, e.pointerId);
              if (!p.moved) onCollapse();
            }}
            onPointerCancel={(e) => { if (press.current) end(e.currentTarget, e.pointerId); }}
            onKeyDown={(e) => {
              if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
                e.preventDefault();
                onResize(width + (e.key === "ArrowLeft" ? KEY_STEP : -KEY_STEP));
              } else if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                onCollapse();
              }
            }}
            className={cn(
              "group absolute -left-2 top-0 bottom-0 z-30 flex w-4 cursor-col-resize touch-none select-none items-center justify-center transition-colors duration-150 ease-ui outline-none focus-visible:bg-primary/10",
              dragging ? "bg-primary/10" : "hover:bg-primary/10",
            )}
          />
        }
      >
        <span
          aria-hidden
          className={cn(
            "pointer-events-none h-12 w-1 rounded-full transition-[height,background-color] duration-200 ease-ui",
            dragging ? "h-16 bg-primary" : "bg-border/80 group-hover:h-14 group-hover:bg-primary/70 group-focus-visible:h-14 group-focus-visible:bg-primary/70",
          )}
        />
      </TooltipTrigger>
      <TooltipContent side="left">Drag to resize · Click to close</TooltipContent>
    </Tooltip>
  );
}
