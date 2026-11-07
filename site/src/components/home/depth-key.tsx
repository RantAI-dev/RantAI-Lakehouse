import * as React from "react";
import { cn } from "@/lib/utils";
import type { HomeCopy } from "@/lib/copy";
import { DEPTH_LAYERS } from "@/lib/home-content";

/**
 * WEB-5 O21: the whole idea of the three depths in one glance, above the
 * chapters: "Bronze raw → Silver clean → Gold ready". The current depth is
 * lit in its own colour and the link to it fills, so the key is also where
 * the visitor is. On phones only the current step keeps its word, so the
 * three fit 360 px. Decorative: the chapter's own text says the same.
 */
export function DepthKey({
  active,
  words,
  className,
}: {
  active: 0 | 1 | 2;
  words: HomeCopy["depths"]["key"];
  className?: string;
}) {
  return (
    <div aria-hidden="true" className={cn("flex items-center gap-1.5", className)}>
      {DEPTH_LAYERS.map((l, i) => {
        const on = i === active;
        return (
          <React.Fragment key={l.id}>
            {i > 0 && (
              <span className="lk-depth-key-link relative h-px w-3 shrink-0 overflow-hidden md:w-5" data-done={i <= active || undefined}>
                <span className="block h-full w-full" style={{ background: DEPTH_LAYERS[i - 1].accent }} />
              </span>
            )}
            <span
              className="lk-depth-key-step inline-flex shrink-0 items-center gap-1.5 rounded-full px-2 py-1 text-[11px] leading-4 font-medium whitespace-nowrap md:px-2.5"
              style={
                on
                  ? { color: l.accent, background: `${l.accent}1f`, boxShadow: `inset 0 0 0 1px ${l.accent}66` }
                  : { color: i < active ? "rgba(232,244,255,0.8)" : "rgba(232,244,255,0.6)" }
              }
            >
              <span
                className="size-1.5 rounded-full"
                style={{ background: i <= active ? l.accent : "rgba(232,244,255,0.35)" }}
              />
              {l.label}
              <span className={cn("font-normal opacity-80", on ? "inline" : "hidden md:inline")}>{words[l.id]}</span>
            </span>
          </React.Fragment>
        );
      })}
    </div>
  );
}
