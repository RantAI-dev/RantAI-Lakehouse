"use client";

import { ArrowRight, Sparkles } from "lucide-react";

import { Sheen } from "@/components/ui/sheen";
import { cn } from "@/lib/utils";

/**
 * A suggested prompt. One component for the Copilot page (pill), the
 * sidebar and the dock (card), so hover feedback is the same everywhere:
 * lift, brand-blue border and glow, a sheen sweep, the sparkle turning and
 * an arrow sliding in. Motion is dropped under `prefers-reduced-motion`.
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
        "group/sug relative flex items-center gap-2 overflow-hidden border text-left text-foreground/85 transition-all duration-200",
        "hover:-translate-y-0.5 hover:border-[color-mix(in_oklch,var(--brand-1),transparent_45%)] hover:bg-[color-mix(in_oklch,var(--brand-1),transparent_92%)] hover:text-foreground hover:shadow-[0_8px_22px_-12px_var(--brand-1)]",
        "active:translate-y-0 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50",
        "disabled:pointer-events-none disabled:opacity-50 motion-reduce:transition-none motion-reduce:hover:translate-y-0",
        variant === "pill"
          ? "rounded-full border-border bg-background px-4 py-2 text-sm font-medium"
          : "w-full rounded-xl border-border/70 bg-muted/20 px-3 py-2.5 text-xs"
      )}
    >
      <Sheen className="group-hover/sug:translate-x-[400%]" />
      <Sparkles
        aria-hidden
        className="size-3.5 shrink-0 text-[var(--brand-1)] transition-transform duration-500 group-hover/sug:rotate-[72deg] group-hover/sug:scale-125 motion-reduce:transition-none"
      />
      <span className="min-w-0 flex-1">{text}</span>
      <ArrowRight
        aria-hidden
        className="size-3.5 shrink-0 -translate-x-1.5 text-[var(--brand-1)] opacity-0 transition-all duration-200 group-hover/sug:translate-x-0 group-hover/sug:opacity-100"
      />
    </button>
  );
}
