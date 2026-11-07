"use client";

import * as React from "react";
import { Check, Copy, TextSelect } from "lucide-react";
import { cn } from "@/lib/utils";

export type CopyText = { copy: string; copied: string; failed: string };

/**
 * Copies the text of the element `targetId` (WEB-5 D20). Every press
 * answers (DESIGN.md §3):
 * - copied: a check and "Copied" in the lake's green, for 2 s;
 * - no clipboard (a plain-HTTP address, a refused permission): the text
 *   is selected instead and the button says how to copy it, for 4 s, so it
 *   never does nothing.
 * The state is announced (`aria-live`), and the button is a real
 * `<button>`, reachable by keyboard, with the page's focus ring.
 * WEB-5 O23: the press settles back (`active:scale-95`), the icon morphs
 * from copy to check and back, the new word eases in, and a copy sends one
 * soft ring out from the button.
 */
export function CopyButton({
  targetId,
  text,
  className,
}: Readonly<{ targetId: string; text: CopyText; className?: string }>) {
  const [state, setState] = React.useState<"idle" | "copied" | "failed">("idle");

  // "Copied" for 2 s; "Selected" a little longer, as there is a key to press.
  React.useEffect(() => {
    if (state === "idle") return;
    const t = window.setTimeout(() => setState("idle"), state === "copied" ? 2000 : 4000);
    return () => window.clearTimeout(t);
  }, [state]);

  const onClick = () => {
    const el = document.getElementById(targetId);
    if (!el) return;
    const value = el.textContent ?? "";
    const fail = () => {
      const sel = window.getSelection();
      if (sel) {
        const range = document.createRange();
        range.selectNodeContents(el);
        sel.removeAllRanges();
        sel.addRange(range);
      }
      setState("failed");
    };
    if (!navigator.clipboard) return fail();
    navigator.clipboard.writeText(value).then(() => setState("copied"), fail);
  };

  return (
    <button
      type="button"
      onClick={onClick}
      aria-describedby={targetId}
      data-state={state}
      className={cn(
        "lk-copy inline-flex h-8 shrink-0 items-center gap-1.5 rounded-full px-3 text-[12px] font-medium ring-1 transition-[background-color,color,box-shadow,transform] duration-200 ease-out active:scale-95 motion-reduce:transition-none",
        state === "copied"
          ? "bg-[#7be0b0]/12 text-[#7be0b0] ring-[#7be0b0]/40"
          : state === "failed"
            ? "bg-[#44A6FA]/12 text-[#8fcbff] ring-[#44A6FA]/40"
            : "text-[#E8F4FF]/80 ring-[#E8F4FF]/15 hover:bg-[#E8F4FF]/[0.08] hover:text-[#E8F4FF] hover:ring-[#E8F4FF]/35",
        className,
      )}
    >
      {/* WEB-5 O23: the three icons are stacked and morph into each other (app/globals.css, `.lk-copy`). */}
      <span className="lk-copy-icon shrink-0" aria-hidden="true">
        <Copy className="lk-copy-idle size-3.5" />
        <Check className="lk-copy-copied size-3.5" strokeWidth={2.75} />
        <TextSelect className="lk-copy-failed size-3.5" />
      </span>
      <span aria-live="polite">
        <span key={state} className={state === "idle" ? undefined : "lk-fade-in inline-block"}>
          {state === "copied" ? text.copied : state === "failed" ? text.failed : text.copy}
        </span>
      </span>
    </button>
  );
}
