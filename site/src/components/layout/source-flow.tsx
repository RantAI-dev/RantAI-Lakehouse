"use client";

import * as React from "react";
import { ArrowRight, Check } from "lucide-react";
import { cn } from "@/lib/utils";

/**
 * The footer card's "Pick a source, watch it flow" (WEB-5 owner QA O14):
 * Lakehouse's own call to action, in place of rantai.dev's typewriter
 * prompt. A visitor picks a source; a light runs from it through Bronze,
 * Silver and Gold into a small dashboard that draws itself. Then "Start with
 * <source>" opens a ready-made e-mail (on the contact page it scrolls to
 * the form with the topic set to connectors).
 *
 * - The sources are a radio group: one is picked by default, the arrow keys
 *   move the choice, and the picked chip is lit and pressed.
 * - The marks arrive rendered by the server (`marks`), so their paths stay
 *   out of this chunk; the motion is CSS (".lk-sf-" in app/globals.css),
 *   restarted by keying the flow on the source; it runs only on screen (the
 *   card's `data-live`, set by an IntersectionObserver), and is still and
 *   lit under reduced motion. Without JavaScript the default source's flow
 *   is shown.
 */

export type SourceFlowText = {
  sources: string;
  raw: string;
  clean: string;
  ready: string;
  tile: string;
  start: string;
  install: string;
  subjectTopic: string;
  body: string;
  opening: string;
  fallback: string;
  contactStart: string;
};

/** The contact form listens for this to preset its topic (contact-form.tsx). */
export const CONTACT_TOPIC_EVENT = "lk:contact-topic";

const DEPTHS = [
  { name: "Bronze", colour: "#C8834A" },
  { name: "Silver", colour: "#B9C6D3" },
  { name: "Gold", colour: "#F2C14E" },
] as const;

/** A deterministic little series per source, so each one draws its own line. */
function series(seed: number): string {
  const pts = Array.from({ length: 8 }, (_, i) => {
    let h = Math.imul(seed + 1, 374761393) ^ Math.imul(i + 3, 668265263);
    h = Math.imul(h ^ (h >>> 13), 1274126177);
    const r = ((h ^ (h >>> 16)) >>> 0) / 4294967296;
    return 46 - (i * 3.2 + r * 14);
  });
  return pts.map((y, i) => `${i === 0 ? "M" : "L"}${6 + i * 15} ${y.toFixed(1)}`).join(" ");
}

export function SourceFlow({
  names,
  marks,
  text,
  to,
  page,
  installHref,
}: Readonly<{
  names: readonly string[];
  marks: readonly React.ReactNode[];
  text: SourceFlowText;
  to: string;
  page: "home" | "contact";
  installHref: string;
}>) {
  const [picked, setPicked] = React.useState(0);
  const [sent, setSent] = React.useState(false);
  const rootRef = React.useRef<HTMLDivElement>(null);
  const refs = React.useRef<(HTMLButtonElement | null)[]>([]);
  const source = names[picked];

  React.useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const io = new IntersectionObserver(([e]) => el.toggleAttribute("data-live", e.isIntersecting));
    io.observe(el);
    return () => io.disconnect();
  }, []);

  const onKey = (e: React.KeyboardEvent) => {
    const step = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
    if (!step) return;
    e.preventDefault();
    const next = (picked + step + names.length) % names.length;
    setPicked(next);
    refs.current[next]?.focus();
  };

  const start = () => {
    if (page === "contact") {
      window.dispatchEvent(new CustomEvent(CONTACT_TOPIC_EVENT, { detail: { topic: "connectors", message: text.body.replace("{source}", source) } }));
      // The form is at the top of the contact page: go there, and put the caret in the message.
      const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
      window.scrollTo({ top: 0, behavior: reduce ? "auto" : "smooth" });
      document.querySelector<HTMLTextAreaElement>("#main form textarea")?.focus({ preventScroll: true });
      return;
    }
    setSent(true);
    const subject = `RantAI Lakehouse · ${text.subjectTopic} · ${source}`;
    window.location.assign(`mailto:${to}?subject=${encodeURIComponent(subject)}&body=${encodeURIComponent(text.body.replace("{source}", source))}`);
  };

  return (
    <div ref={rootRef} className="lk-sf w-full">
      <div role="radiogroup" aria-label={text.sources} onKeyDown={onKey} className="flex flex-wrap justify-center gap-2">
        {names.map((n, i) => (
          <button
            key={n}
            ref={(el) => {
              refs.current[i] = el;
            }}
            type="button"
            role="radio"
            aria-checked={picked === i}
            tabIndex={picked === i ? 0 : -1}
            onClick={() => setPicked(i)}
            className={cn(
              "inline-flex h-10 items-center gap-2 rounded-full border px-3.5 text-[13px] transition-[background-color,border-color,color,box-shadow,transform] duration-200 ease-out active:scale-[0.97] motion-reduce:transition-none",
              picked === i
                ? "border-[#8fcbff]/70 bg-[#44A6FA]/20 text-white shadow-[inset_0_1px_0_rgba(232,244,255,0.15),0_0_24px_-6px_rgba(68,166,250,0.7)]"
                : "border-[#E8F4FF]/15 bg-[#E8F4FF]/[0.04] text-[#E8F4FF]/75 hover:border-[#E8F4FF]/35 hover:text-[#E8F4FF]",
            )}
          >
            <span className={cn("[&>svg]:size-4", picked === i ? "text-white" : "text-[#E8F4FF]/70")}>{marks[i]}</span>
            {n}
          </button>
        ))}
      </div>

      {/* The flow, keyed on the source so its light and its chart start again on every pick. */}
      <div key={source} aria-hidden="true" className="lk-sf-flow mt-10 flex flex-col items-center gap-0 md:flex-row md:justify-center">
        <div className="lk-sf-node flex h-12 min-w-[9rem] items-center justify-center gap-2 rounded-2xl bg-[#020514]/70 px-4 text-[13px] text-[#E8F4FF] ring-1 ring-[#8fcbff]/40">
          <span className="[&>svg]:size-4">{marks[picked]}</span>
          {source}
        </div>
        {DEPTHS.map((d, i) => (
          <React.Fragment key={d.name}>
            <span className="lk-sf-line" style={{ animationDelay: `${i * 0.35}s` }} />
            <div className="lk-sf-node flex flex-col items-center" style={{ animationDelay: `${0.35 + i * 0.35}s` }}>
              <span className="grid size-12 place-items-center rounded-full text-[11px] font-semibold text-[#020514]" style={{ background: d.colour, boxShadow: `0 0 24px -4px ${d.colour}` }}>
                {d.name.slice(0, 1)}
              </span>
              <span className="mt-1.5 text-[11px] text-[#E8F4FF]/70">
                {d.name} · {[text.raw, text.clean, text.ready][i]}
              </span>
            </div>
          </React.Fragment>
        ))}
        <span className="lk-sf-line" style={{ animationDelay: "1.05s" }} />
        <div className="lk-sf-node lk-tile w-[11rem] rounded-2xl bg-[#E8F4FF]/[0.06] p-3 ring-1 ring-[#E8F4FF]/15" data-shown="" style={{ animationDelay: "1.4s" }}>
          <p className="text-[11px] text-[#E8F4FF]/70">{text.tile}</p>
          <svg viewBox="0 0 120 52" className="mt-1.5 h-auto w-full">
            <path className="lk-draw" style={{ animationDelay: "1.5s" }} pathLength={1} d={series(picked)} fill="none" stroke="#44A6FA" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        </div>
      </div>

      <div className="mt-10 flex flex-wrap items-center justify-center gap-3">
        <button
          type="button"
          onClick={start}
          className="group inline-flex h-12 items-center gap-2 rounded-full bg-[var(--lk-primary)] px-6 text-[15px] font-medium text-white transition-[transform,background-color,box-shadow] duration-200 ease-out hover:-translate-y-0.5 hover:bg-[var(--lk-primary-hover)] hover:shadow-[0_10px_40px_-8px_rgba(68,166,250,0.7)] active:translate-y-0 active:scale-[0.98] active:bg-[var(--lk-primary-press)] motion-reduce:transition-none motion-reduce:hover:translate-y-0"
        >
          {(page === "contact" ? text.contactStart : text.start).replace("{source}", source)}
          <ArrowRight aria-hidden="true" className="size-4 transition-transform duration-200 ease-out group-hover:translate-x-0.5 motion-reduce:transition-none" />
        </button>
        <a
          href={installHref}
          className="inline-flex h-12 items-center rounded-full border border-[#E8F4FF]/35 px-6 text-[15px] font-medium text-[#E8F4FF] transition-[transform,background-color,border-color] duration-200 ease-out hover:-translate-y-0.5 hover:border-[#E8F4FF]/70 hover:bg-[#E8F4FF]/10 active:translate-y-0 active:scale-[0.98] motion-reduce:transition-none motion-reduce:hover:translate-y-0"
        >
          {text.install}
        </a>
      </div>
      <p aria-live="polite" className="mt-4 min-h-5 text-center text-[13px] text-[#E8F4FF]/70">
        {sent && (
          <span className="lk-fade-in inline-flex items-center gap-1.5">
            <Check aria-hidden="true" className="size-3.5 text-[#7be0b0]" />
            {text.opening} {text.fallback.replace("{email}", to)}
          </span>
        )}
      </p>
    </div>
  );
}
