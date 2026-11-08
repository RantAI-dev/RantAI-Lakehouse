"use client";

import * as React from "react";
import { cn } from "@/lib/utils";
import { FONTS, LAYOUTS, TIMES } from "./options";

type Option = { id: string; short: string; name: string };

/**
 * One segmented group: a radio group with the arrow keys moving the choice
 * (roving tabindex), a pressed state, and a sliding highlight.
 */
function Segmented({
  label,
  options,
  value,
  onChange,
}: {
  label: string;
  options: readonly Option[];
  value: string;
  onChange: (id: string) => void;
}) {
  const refs = React.useRef<(HTMLButtonElement | null)[]>([]);
  const index = Math.max(0, options.findIndex((o) => o.id === value));
  // The highlight follows the chosen button's own box (the labels differ in width).
  const [pill, setPill] = React.useState<{ left: number; width: number } | null>(null);
  React.useLayoutEffect(() => {
    const measure = () => {
      const el = refs.current[index];
      if (el) setPill({ left: el.offsetLeft, width: el.offsetWidth });
    };
    measure();
    const ro = new ResizeObserver(measure);
    const el = refs.current[index];
    if (el?.parentElement) ro.observe(el.parentElement);
    return () => ro.disconnect();
  }, [index]);
  const onKey = (e: React.KeyboardEvent) => {
    const step = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
    if (!step) return;
    e.preventDefault();
    const next = (index + step + options.length) % options.length;
    onChange(options[next].id);
    refs.current[next]?.focus();
  };
  return (
    <div className="flex items-center gap-2.5 max-md:justify-between">
      <span id={`lab-${label}`} className="w-12 shrink-0 text-[11px] font-medium tracking-[0.14em] text-[#E8F4FF]/55 uppercase">
        {label}
      </span>
      <div
        role="radiogroup"
        aria-labelledby={`lab-${label}`}
        onKeyDown={onKey}
        className="relative flex rounded-full bg-white/[0.07] p-0.5"
      >
        {/* The highlight slides to the chosen segment. */}
        <span
          aria-hidden="true"
          className="lab-pill absolute inset-y-0.5 left-0 rounded-full bg-[#1d6ccf] shadow-[0_2px_10px_-2px_rgba(29,108,207,0.8)]"
          style={pill ? { width: pill.width, transform: `translateX(${pill.left}px)` } : { opacity: 0 }}
        />
        {options.map((o, i) => (
          <button
            key={o.id}
            ref={(el) => {
              refs.current[i] = el;
            }}
            type="button"
            role="radio"
            aria-checked={o.id === value}
            aria-label={o.name}
            tabIndex={o.id === value ? 0 : -1}
            onClick={() => onChange(o.id)}
            className={cn(
              "lab-seg relative z-10 min-h-9 flex-1 rounded-full px-2 text-[13px] md:px-3 font-medium whitespace-nowrap transition-[color,transform] duration-200 ease-out outline-none focus-visible:ring-2 focus-visible:ring-[#8fcbff] active:scale-[0.96] max-md:min-h-10 motion-reduce:transition-none",
              o.id === value ? "text-white" : "text-[#E8F4FF]/70 hover:text-white",
            )}
          >
            {o.short}
          </button>
        ))}
      </div>
    </div>
  );
}

export function ControlBar({
  font,
  layout,
  time,
  onChange,
}: {
  font: string;
  layout: string;
  time: string;
  onChange: (patch: { font?: string; layout?: string; time?: string }) => void;
}) {
  return (
    <div
      role="group"
      aria-label="Hero lab controls"
      className="fixed bottom-4 left-1/2 z-[60] flex w-[calc(100%-2rem)] max-w-[22rem] -translate-x-1/2 flex-col gap-1.5 rounded-3xl border border-white/12 bg-[#0a1230]/88 p-2.5 shadow-[0_18px_50px_-12px_rgba(2,5,20,0.8)] backdrop-blur-xl md:w-auto md:max-w-none md:flex-row md:items-center md:gap-5 md:rounded-full md:px-4 md:py-2"
    >
      <Segmented label="Font" options={FONTS} value={font} onChange={(id) => onChange({ font: id })} />
      <span aria-hidden="true" className="hidden h-6 w-px bg-white/12 md:block" />
      <Segmented label="Layout" options={LAYOUTS} value={layout} onChange={(id) => onChange({ layout: id })} />
      <span aria-hidden="true" className="hidden h-6 w-px bg-white/12 md:block" />
      <Segmented label="Time" options={TIMES} value={time} onChange={(id) => onChange({ time: id })} />
    </div>
  );
}
