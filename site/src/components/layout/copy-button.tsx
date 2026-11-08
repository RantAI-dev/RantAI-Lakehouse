"use client";

import * as React from "react";
import { Check, Copy } from "lucide-react";
import { cn } from "@/lib/utils";

/**
 * One button that copies a string and says so: the label turns to
 * "Copied" with a check for 1.8 s (announced politely), the icon swaps with a short soft
 * transition, and a press settles the button down. If the clipboard is blocked, the label says
 * so and the text stays selectable next to it.
 */
export function CopyButton({
  text,
  label,
  copiedLabel,
  failedLabel,
  className,
  ariaLabel,
}: {
  text: string;
  label: string;
  copiedLabel: string;
  failedLabel: string;
  className?: string;
  ariaLabel?: string;
}) {
  const [state, setState] = React.useState<"idle" | "copied" | "failed">("idle");
  const timer = React.useRef<number | undefined>(undefined);
  React.useEffect(() => () => window.clearTimeout(timer.current), []);
  const onClick = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setState("copied");
    } catch {
      setState("failed");
    }
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setState("idle"), 1800);
  };
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={ariaLabel}
      data-state={state}
      className={cn(
        "group/copy inline-flex h-10 min-w-[5.5rem] shrink-0 items-center justify-center gap-1.5 rounded-full bg-white/[0.07] px-3.5 text-[13px] font-medium text-[#E8F4FF]/85 transition-[background-color,color,transform] duration-200 ease-out outline-none hover:bg-white/[0.12] hover:text-white focus-visible:ring-2 focus-visible:ring-[#44A6FA] active:scale-[0.97] motion-reduce:transition-none data-[state=copied]:bg-[#7be0b0]/15 data-[state=copied]:text-[#7be0b0] data-[state=failed]:text-[#ff9b8e]",
        className,
      )}
    >
      <span className="relative grid size-4 place-items-center">
        <Copy aria-hidden="true" className="absolute size-4 transition-[opacity,transform] duration-200 ease-out group-data-[state=copied]/copy:scale-50 group-data-[state=copied]/copy:opacity-0" />
        <Check aria-hidden="true" strokeWidth={2.5} className="absolute size-4 scale-50 opacity-0 transition-[opacity,transform] duration-200 ease-out group-data-[state=copied]/copy:scale-100 group-data-[state=copied]/copy:opacity-100" />
      </span>
      <span aria-live="polite">{state === "copied" ? copiedLabel : state === "failed" ? failedLabel : label}</span>
    </button>
  );
}
