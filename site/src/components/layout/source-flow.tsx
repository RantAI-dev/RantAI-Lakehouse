"use client";

import * as React from "react";
import { ArrowRight, Check, ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";

/**
 * The footer card's "Pick a source, watch it flow" (WEB-5 owner QA O14,
 * made compact in O46: two columns, the words and the actions on the left,
 * the picker and the flow on the right; stacked on phones):
 * Lakehouse's own call to action, in place of rantai.dev's typewriter
 * prompt. A visitor picks a source; a light runs from it through Bronze,
 * Silver and Gold into a small dashboard that draws itself. Then "Start with
 * <source>" opens a ready-made e-mail (on the contact page it scrolls to
 * the form with the topic set to connectors).
 *
 * - The first sources are one row of chips, a radio group: one is picked
 *   by default, the arrow keys move the choice. The rest sit behind a
 *   "+N" chip, a native select, so the row never wraps.
 * - The flow is one thin line from the source through three small dots
 *   (Bronze, Silver, Gold) into a small dashboard chip; a light travels it.
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
  more: string;
};

/** The contact form listens for this to preset its topic (contact-form.tsx). */
export const CONTACT_TOPIC_EVENT = "lk:contact-topic";

/** Muted medal colours (O46): small dots, no glow. */
const DEPTHS = [
  { name: "Bronze", colour: "#b98157" },
  { name: "Silver", colour: "#aab6c2" },
  { name: "Gold", colour: "#d9b45a" },
] as const;

const CHIP =
  "inline-flex h-8 shrink-0 items-center gap-1.5 rounded-full border px-2.5 text-[12px] transition-[background-color,border-color,color] duration-200 ease-out motion-reduce:transition-none";
/** Audit #12: the site's one selected style (also the contact topics and the governance role toggle). */
const CHIP_ON = "border-[#8fcbff]/60 bg-[#44A6FA]/15 text-white";
const CHIP_OFF = "border-[#E8F4FF]/12 text-[#E8F4FF]/70 hover:border-[#E8F4FF]/30 hover:text-[#E8F4FF]";

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
  intro,
  shown = 5,
}: Readonly<{
  names: readonly string[];
  marks: readonly React.ReactNode[];
  text: SourceFlowText;
  to: string;
  page: "home" | "contact";
  installHref: string;
  /** The card's words (eyebrow, headline, line), rendered by the server. */
  intro: React.ReactNode;
  /** How many sources get a chip; the rest are in the "+N" select. */
  shown?: number;
}>) {
  const [picked, setPicked] = React.useState(0);
  const [sent, setSent] = React.useState(false);
  const rootRef = React.useRef<HTMLDivElement>(null);
  const refs = React.useRef<(HTMLButtonElement | null)[]>([]);
  const source = names[picked];
  const extra = picked >= shown;

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
    const from = extra ? (step > 0 ? -1 : 0) : picked;
    const next = (from + step + shown) % shown;
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
    <div
      ref={rootRef}
      className="lk-sf grid items-center gap-8 px-6 py-9 sm:px-8 lg:grid-cols-[minmax(0,4.5fr)_minmax(0,7.5fr)] lg:gap-10 lg:px-10 lg:py-10"
    >
      <div>
        {intro}
        <div className="mt-6 flex flex-wrap items-center gap-3">
          <button
            type="button"
            onClick={start}
            className="group inline-flex h-11 items-center gap-2 rounded-full bg-[var(--lk-primary)] px-5 text-[14px] font-medium text-white transition-[background-color,transform] duration-200 ease-out hover:bg-[var(--lk-primary-hover)] active:scale-[0.98] active:bg-[var(--lk-primary-press)] motion-reduce:transition-none"
          >
            {(page === "contact" ? text.contactStart : text.start).replace("{source}", source)}
            <ArrowRight aria-hidden="true" className="size-4 transition-transform duration-200 ease-out group-hover:translate-x-0.5 motion-reduce:transition-none" />
          </button>
          <a
            href={installHref}
            className="inline-flex h-11 items-center rounded-full border border-[#E8F4FF]/25 px-5 text-[14px] font-medium text-[#E8F4FF] transition-[background-color,border-color,transform] duration-200 ease-out hover:border-[#E8F4FF]/50 hover:bg-[#E8F4FF]/[0.06] active:scale-[0.98] motion-reduce:transition-none"
          >
            {text.install}
          </a>
        </div>
        <p aria-live="polite" className="mt-3 min-h-5 text-[13px] text-[#E8F4FF]/70">
          {sent && (
            <span className="lk-fade-in inline-flex items-center gap-1.5">
              <Check aria-hidden="true" className="size-3.5 text-[#7be0b0]" />
              {text.opening} {text.fallback.replace("{email}", to)}
            </span>
          )}
        </p>
      </div>

      <div className="min-w-0 rounded-2xl bg-[#020514]/50 p-4 ring-1 ring-[#E8F4FF]/[0.07] sm:p-5">
        {/* One row, never wrapped: it scrolls sideways on a narrow screen instead. */}
        <div className="-mx-1 flex items-center gap-1.5 overflow-x-auto px-1 py-1 [scrollbar-width:none] max-sm:[mask-image:linear-gradient(90deg,#000_80%,transparent)]">
          <div role="radiogroup" aria-label={text.sources} onKeyDown={onKey} className="flex gap-1.5">
            {names.slice(0, shown).map((n, i) => (
              <button
                key={n}
                ref={(el) => {
                  refs.current[i] = el;
                }}
                type="button"
                role="radio"
                aria-checked={picked === i}
                tabIndex={picked === i || (extra && i === 0) ? 0 : -1}
                onClick={() => setPicked(i)}
                className={cn(CHIP, picked === i ? CHIP_ON : CHIP_OFF)}
              >
                <span className="[&>svg]:size-3.5">{marks[i]}</span>
                {n}
              </button>
            ))}
          </div>
          {names.length > shown && (
            <label className={cn(CHIP, extra ? CHIP_ON : CHIP_OFF, "relative cursor-pointer has-[select:focus-visible]:outline-2 has-[select:focus-visible]:outline-offset-2 has-[select:focus-visible]:outline-[#8fcbff]")}>
              <span aria-hidden="true">+{names.length - shown}</span>
              <ChevronDown aria-hidden="true" className="size-3" />
              <select
                aria-label={text.more}
                value={extra ? String(picked) : ""}
                onChange={(e) => e.target.value && setPicked(Number(e.target.value))}
                className="absolute inset-0 cursor-pointer opacity-0 [color-scheme:dark]"
              >
                <option value="" disabled>
                  {text.more}
                </option>
                {names.slice(shown).map((n, j) => (
                  <option key={n} value={shown + j}>
                    {n}
                  </option>
                ))}
              </select>
            </label>
          )}
        </div>

        {/* The flow, keyed on the source so its light and its chart start again on every pick. */}
        <div key={source} aria-hidden="true" className="lk-sf-flow mt-6 flex items-center max-md:flex-col max-md:items-start md:pb-7">
          <div className="lk-sf-node inline-flex h-8 shrink-0 items-center gap-1.5 rounded-full bg-[#020514]/70 px-3 text-[12px] text-[#E8F4FF] ring-1 ring-[#E8F4FF]/15">
            <span className="[&>svg]:size-3.5">{marks[picked]}</span>
            {source}
          </div>
          {DEPTHS.map((d, i) => (
            <React.Fragment key={d.name}>
              <span className="lk-sf-line" style={{ "--lk-sf-d": `${i * 0.45}s` } as React.CSSProperties} />
              <span className="lk-sf-node relative size-2.5 shrink-0 rounded-full max-md:ml-[13.5px]" style={{ background: d.colour, animationDelay: `${0.1 + i * 0.1}s` }}>
                <span className="absolute text-[11px] whitespace-nowrap text-[#E8F4FF]/65 max-md:top-1/2 max-md:left-full max-md:ml-3 max-md:-translate-y-1/2 md:top-full md:left-1/2 md:mt-2.5 md:-translate-x-1/2">
                  {d.name} · {[text.raw, text.clean, text.ready][i]}
                </span>
              </span>
            </React.Fragment>
          ))}
          <span className="lk-sf-line" style={{ "--lk-sf-d": "1.35s" } as React.CSSProperties} />
          <div className="lk-sf-node lk-tile shrink-0 rounded-xl bg-[#E8F4FF]/[0.05] px-2.5 py-1.5 ring-1 ring-[#E8F4FF]/12" data-shown="" style={{ animationDelay: "0.4s" }}>
            <p className="text-[10px] leading-3 whitespace-nowrap text-[#E8F4FF]/65">{text.tile}</p>
            <svg viewBox="0 0 120 52" className="mt-1 h-5 w-[4.5rem]">
              <path className="lk-draw" style={{ animationDelay: "0.5s" }} pathLength={1} d={series(picked)} fill="none" stroke="#44A6FA" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          </div>
        </div>
      </div>
    </div>
  );
}
