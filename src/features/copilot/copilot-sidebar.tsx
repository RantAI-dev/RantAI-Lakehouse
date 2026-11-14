"use client";

import * as React from "react";
import { usePathname } from "next/navigation";
import { Sparkles, Plus, PanelBottom, PanelRightClose, X } from "lucide-react";
import { useSidePanel } from "@/components/app-shell/side-panel";
import { SuggestionButton } from "@/features/copilot/suggestion-button";
import { useCopilot } from "./use-copilot";
import { ChatMessages } from "./chat-messages";
import { ChatComposer } from "./chat-composer";
import { CopilotHistoryButton } from "./history-menu";
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
export function CopilotSidebar({ hasAssistant = true }: { readonly hasAssistant?: boolean }) {
  const pathname = usePathname();
  const c = useCopilot();
  const { tabs: pageTabs, active: activeTab, setHost, open: openTab, close: closePanel } = useSidePanel();
  const [isDragging, setIsDragging] = React.useState(false);
  const isDraggingRef = React.useRef(false);

  const copilotOpen = hasAssistant && !pathname?.startsWith("/copilot") && c.dockPosition === "right" && c.expanded;
  const pageTab = pageTabs.find((t) => t.id === activeTab);
  // Hidden on dedicated /copilot page or when dock position is bottom or collapsed, unless a page tab is open
  if (!copilotOpen && !pageTab) {
    return null;
  }
  const tabbed = pageTabs.length > 0;

  const startResizing = (e: React.MouseEvent) => {
    e.preventDefault();
    isDraggingRef.current = true;
    setIsDragging(true);

    const onMouseMove = (moveEvent: MouseEvent) => {
      if (!isDraggingRef.current) return;
      const rawWidth = window.innerWidth - moveEvent.clientX;
      const maxWidth = Math.max(340, Math.min(850, window.innerWidth - 320));
      const clamped = Math.max(320, Math.min(maxWidth, rawWidth));
      c.setSidebarWidth(clamped);
    };

    const onMouseUp = () => {
      isDraggingRef.current = false;
      setIsDragging(false);
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
      document.body.style.removeProperty("cursor");
      document.body.style.removeProperty("user-select");
    };

    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
  };

  const handleDoubleClick = () => {
    c.setSidebarWidth(420);
  };

  return (
    <aside
      data-slot="copilot-sidebar"
      // BI-61 T6: with page tabs the panel covers the page on a phone (a side
      // column there would leave the page no width) and takes its width from md up.
      style={tabbed
        ? ({ "--side-panel-w": `${c.sidebarWidth}px` } as React.CSSProperties)
        : { width: `${c.sidebarWidth}px`, minWidth: `${c.sidebarWidth}px`, maxWidth: "850px" }}
      className={cn(
        "sticky top-16 flex h-[calc(100vh-4rem)] shrink-0 flex-col border-l border-border bg-card/95 backdrop-blur-md",
        tabbed && "max-md:fixed max-md:inset-x-0 max-md:bottom-0 max-md:z-40 max-md:h-auto max-md:border-l-0 max-md:bg-card md:w-(--side-panel-w) md:max-w-[850px] md:min-w-(--side-panel-w)",
        isDragging && "select-none",
      )}
    >
      {/* Resizing drag handle */}
      {hasAssistant ? (
      <button
        type="button"
        aria-label="Resize AI Copilot sidebar"
        onMouseDown={startResizing}
        onDoubleClick={handleDoubleClick}
        title="Drag to resize, double click to reset"
        className={cn(
          "group absolute -left-2 top-0 bottom-0 z-30 flex w-4 cursor-col-resize select-none items-center justify-center border-0 bg-transparent p-0 transition-colors focus:outline-none",
          isDragging ? "bg-primary/10" : "hover:bg-primary/10",
        )}
      >
        <span
          className={cn(
            "h-12 w-1 rounded-full transition-all pointer-events-none",
            isDragging ? "h-16 w-1 bg-primary" : "bg-border/80 group-hover:h-14 group-hover:bg-primary/70",
          )}
        />
      </button>
      ) : null}

      {tabbed ? (
        <PanelTabs
          tabs={pageTabs}
          active={pageTab?.id ?? null}
          withCopilot={hasAssistant}
          onCopilot={() => {
            closePanel();
            if (c.dockPosition !== "right") c.setDockPosition("right");
            c.setExpanded(true);
          }}
          onTab={openTab}
          onClose={() => {
            closePanel();
            if (copilotOpen) c.setExpanded(false);
          }}
          copilotTools={pageTab ? null : (
            <>
              <CopilotHistoryButton align="end" />
              <button type="button" onClick={() => c.setDockPosition("bottom")} aria-label="Switch to bottom floating dock" title="Switch to bottom floating dock"
                className="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground">
                <PanelBottom className="size-4" />
              </button>
              <button type="button" onClick={c.newChat} aria-label="New chat" title="New chat"
                className="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground">
                <Plus className="size-4" />
              </button>
            </>
          )}
        />
      ) : null}
      {pageTab ? (
        // The page draws the tab's content here (a portal), so it keeps its own state.
        <div ref={setHost} data-slot="side-panel-body" className="flex min-h-0 flex-1 flex-col overflow-hidden" />
      ) : (
      <>
      {/* Header */}
      {!tabbed ? (
      <div className="flex h-14 shrink-0 items-center gap-2 border-b border-border px-3.5">
        <span className="grid size-6 place-items-center rounded-md bg-primary/10 text-primary">
          <Sparkles className="size-3.5" />
        </span>
        <div className="flex items-center gap-1.5 min-w-0">
          <span className="text-sm font-semibold truncate text-foreground">AI Copilot</span>
          <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
            {c.mode}
          </span>
        </div>

        <div className="ml-auto flex items-center gap-0.5">
          <CopilotHistoryButton align="end" />
          <button
            type="button"
            onClick={() => c.setDockPosition("bottom")}
            aria-label="Switch to bottom floating dock"
            title="Switch to bottom floating dock"
            className="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
          >
            <PanelBottom className="size-4" />
          </button>
          <button
            type="button"
            onClick={c.newChat}
            aria-label="New chat"
            title="New chat"
            className="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
          >
            <Plus className="size-4" />
          </button>
          <button
            type="button"
            onClick={() => c.setExpanded(false)}
            aria-label="Collapse AI Copilot"
            title="Collapse AI Copilot"
            className="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
          >
            <PanelRightClose className="size-4" />
          </button>
        </div>
      </div>
      ) : null}

      {/* Messages or suggestions body */}
      <div className="flex-1 overflow-y-auto p-3.5">
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

/**
 * The tab row of the shared panel (`BI-61` T6): "AI Copilot", then the page's
 * tabs, as one row of quiet buttons, the chosen one filled; the Copilot's own
 * tools while it is shown, and one close button for the whole panel.
 */
function PanelTabs({
  tabs, active, withCopilot, onCopilot, onTab, onClose, copilotTools,
}: {
  readonly tabs: readonly { id: string; label: string }[];
  /** The page tab shown; null when the Copilot is. */
  readonly active: string | null;
  readonly withCopilot: boolean;
  readonly onCopilot: () => void;
  readonly onTab: (id: string) => void;
  readonly onClose: () => void;
  readonly copilotTools: React.ReactNode;
}) {
  const tab = (on: boolean) => cn(
    "inline-flex h-8 shrink-0 items-center gap-1.5 rounded-lg px-2.5 text-sm font-medium outline-none transition-colors duration-200 ease-[cubic-bezier(0.2,0,0,1)] focus-visible:ring-3 focus-visible:ring-ring/50",
    on ? "bg-muted text-foreground" : "text-muted-foreground hover:bg-muted/60 hover:text-foreground",
  );
  return (
    <div className="flex h-14 shrink-0 items-center gap-1 border-b border-border px-2.5">
      <div role="tablist" aria-label="Side panel" className="flex min-w-0 items-center gap-0.5 overflow-x-auto [scrollbar-width:none]">
        {withCopilot ? (
          <button type="button" role="tab" aria-selected={active === null} onClick={onCopilot} className={tab(active === null)}>
            <Sparkles className="size-3.5 text-primary" aria-hidden /> AI Copilot
          </button>
        ) : null}
        {tabs.map((t) => (
          <button key={t.id} type="button" role="tab" aria-selected={active === t.id} onClick={() => onTab(t.id)} className={tab(active === t.id)}>
            {t.label}
          </button>
        ))}
      </div>
      <div className="ml-auto flex shrink-0 items-center gap-0.5">
        {copilotTools}
        <button type="button" onClick={onClose} aria-label="Close panel" title="Close panel"
          className="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground">
          <X className="size-4" />
        </button>
      </div>
    </div>
  );
}
