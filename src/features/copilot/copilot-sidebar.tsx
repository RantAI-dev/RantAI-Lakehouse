"use client";

import * as React from "react";
import { usePathname } from "next/navigation";
import { Sparkles, Plus, PanelBottom, PanelRightClose } from "lucide-react";
import { useCopilot } from "./use-copilot";
import { ChatMessages } from "./chat-messages";
import { ChatComposer } from "./chat-composer";
import { CopilotHistoryButton } from "./history-menu";
import { cn } from "@/lib/utils";

/**
 * Resizable right sidebar for AI Copilot.
 * Docks alongside main page content so tables, charts, and metrics are not obscured.
 * Supports drag-to-resize, double-click to reset width, mode badge, and dock position switching.
 */
export function CopilotSidebar() {
  const pathname = usePathname();
  const c = useCopilot();
  const [isDragging, setIsDragging] = React.useState(false);
  const isDraggingRef = React.useRef(false);

  // Hidden on dedicated /copilot page or when dock position is bottom or collapsed
  if (pathname?.startsWith("/copilot") || c.dockPosition !== "right" || !c.expanded) {
    return null;
  }

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
      style={{ width: `${c.sidebarWidth}px`, minWidth: `${c.sidebarWidth}px`, maxWidth: "850px" }}
      className={cn(
        "sticky top-16 flex h-[calc(100vh-4rem)] shrink-0 flex-col border-l border-border bg-card/95 backdrop-blur-md",
        isDragging && "select-none",
      )}
    >
      {/* Resizing drag handle */}
      <button
        type="button"
        aria-label="Resize Copilot sidebar"
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

      {/* Header */}
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
            aria-label="Collapse Copilot"
            title="Collapse Copilot"
            className="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
          >
            <PanelRightClose className="size-4" />
          </button>
        </div>
      </div>

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
                  <button
                    key={s}
                    type="button"
                    onClick={() => c.requestSend(s)}
                    disabled={c.busy}
                    className="rounded-xl border border-border/70 bg-muted/20 p-2.5 text-left text-xs text-foreground transition-all hover:border-primary/40 hover:bg-muted/60 disabled:opacity-50"
                  >
                    {s}
                  </button>
                ))}
              </div>
            </div>
          </div>
        ) : (
          <ChatMessages
            messages={c.messages}
            busy={c.busy}
            error={c.error}
            onConfirmTool={c.confirmTool}
            onCancelTool={c.cancelTool}
            onCompleteTool={c.completeToolStep}
            confirmingKey={c.confirmingKey}
          />
        )}
      </div>

      {/* Composer footer */}
      <div className="shrink-0 border-t border-border bg-card/60 p-3">
        <ChatComposer
          mode={c.mode}
          setMode={c.setMode}
          onSend={c.requestSend}
          busy={c.busy}
          rows={2}
          placeholder="Ask anything about your lakehouse data…"
          enabledCaps={c.enabledCaps}
          toggleCap={c.toggleCap}
        />
      </div>
    </aside>
  );
}
