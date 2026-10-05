"use client";

import { cn } from "@/lib/utils";

/**
 * A suggested prompt. One component for the Copilot page (pill), the
 * sidebar and the dock (card), so hover feedback is the same everywhere: a
 * light background change. It used to lift, glow in the brand blue, sweep
 * a sheen and spin a sparkle; that read as decoration (QA feedback).
 */
export function SuggestionButton({
  text,
  onClick,
  disabled,
  variant = "card",
}: {
  readonly text: string;
  readonly onClick: () => void;
  readonly disabled?: boolean;
  readonly variant?: "pill" | "card";
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={cn(
        "flex items-center gap-2 border border-border text-left text-foreground/85 transition-colors hover:bg-muted/60 hover:text-foreground",
        "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50 disabled:pointer-events-none disabled:opacity-50",
        variant === "pill" ? "rounded-full bg-background px-4 py-2 text-sm" : "w-full rounded-xl bg-muted/20 px-3 py-2.5 text-xs"
      )}
    >
      <span className="min-w-0 flex-1">{text}</span>
    </button>
  );
}
