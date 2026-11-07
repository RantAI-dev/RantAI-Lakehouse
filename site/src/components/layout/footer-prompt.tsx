"use client";

import * as React from "react";
import { ArrowUp, ArrowUpLeft, Check, Mail } from "lucide-react";
import { useReducedMotionSafe } from "@/hooks/use-reduced-motion";
import { cn } from "@/lib/utils";

export type FooterPromptText = {
  label: string;
  send: string;
  opening: string;
  subject: string;
  examples: string[];
};

/**
 * Types each example out, holds it, deletes it, moves to the next. Only
 * while the box is on screen (an IntersectionObserver), and not at all
 * under reduced motion, where the first example simply rests.
 */
function useTypewriter(lines: readonly string[], ref: React.RefObject<HTMLElement | null>): string {
  const reduce = useReducedMotionSafe();
  const [text, setText] = React.useState(lines[0] ?? "");
  const [onScreen, setOnScreen] = React.useState(false);

  React.useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const io = new IntersectionObserver(([e]) => setOnScreen(e.isIntersecting));
    io.observe(el);
    return () => io.disconnect();
  }, [ref]);

  React.useEffect(() => {
    if (reduce || !lines.length || !onScreen) return;
    let line = 0;
    let length = lines[0].length;
    let deleting = true;
    let timer: ReturnType<typeof setTimeout>;
    const tick = () => {
      const full = lines[line];
      length += deleting ? -1 : 1;
      setText(full.slice(0, length));
      if (!deleting && length === full.length) {
        deleting = true;
        // Long enough to read it.
        timer = setTimeout(tick, 3200);
      } else if (deleting && length === 0) {
        deleting = false;
        line = (line + 1) % lines.length;
        timer = setTimeout(tick, 500);
      } else {
        timer = setTimeout(tick, deleting ? 28 : 64);
      }
    };
    timer = setTimeout(tick, 2400);
    return () => clearTimeout(timer);
  }, [lines, reduce, onScreen]);

  return text;
}

/**
 * The footer's composer (WEB-5 O7, after rantai-page's `footer-prompt.tsx`,
 * in the lake's look). A visitor describes what they want in a sentence, or
 * picks an example, and "Send" opens an e-mail to us with their words, as
 * the contact form does: nothing is sent from the page (D1). The
 * placeholder types the examples out; Enter sends, Shift+Enter breaks the
 * line. Every state is drawn: an empty send button is quiet, a filled one
 * lights up, and sending says it is opening the e-mail app.
 */
export function FooterPrompt({ text, to }: Readonly<{ text: FooterPromptText; to: string }>) {
  const [value, setValue] = React.useState("");
  const [sent, setSent] = React.useState(false);
  const boxRef = React.useRef<HTMLDivElement>(null);
  const fieldRef = React.useRef<HTMLTextAreaElement>(null);
  const placeholder = useTypewriter(text.examples, boxRef);
  const id = React.useId();

  React.useEffect(() => {
    if (!sent) return;
    const t = setTimeout(() => setSent(false), 5000);
    return () => clearTimeout(t);
  }, [sent]);

  const send = (e: React.FormEvent) => {
    e.preventDefault();
    setSent(true);
    const body = value.trim() ? `&body=${encodeURIComponent(value.trim())}` : "";
    window.location.assign(`mailto:${to}?subject=${encodeURIComponent(text.subject)}${body}`);
  };

  return (
    <>
      <div
        ref={boxRef}
        className="relative w-full rounded-[20px] bg-[linear-gradient(180deg,rgba(232,244,255,0.4),rgba(232,244,255,0.08)_45%,rgba(68,166,250,0.35))] p-px shadow-[0_0_0_6px_rgba(232,244,255,0.03),0_30px_90px_-24px_rgba(68,166,250,0.35)] transition-shadow duration-300 ease-out focus-within:shadow-[0_0_0_6px_rgba(68,166,250,0.12),0_30px_90px_-24px_rgba(68,166,250,0.5)]"
      >
        <form onSubmit={send} className="relative w-full rounded-[19px] bg-[linear-gradient(180deg,#0B1A4A,#050A30)] p-3 text-left">
          <label htmlFor={`${id}-field`} className="sr-only">
            {text.label}
          </label>
          <textarea
            id={`${id}-field`}
            ref={fieldRef}
            rows={2}
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                e.currentTarget.form?.requestSubmit();
              }
            }}
            placeholder={`${placeholder}▍`}
            className="block min-h-[3.5rem] w-full resize-none bg-transparent px-2 py-1.5 text-[16px] leading-6 text-[#E8F4FF] outline-none placeholder:text-[#E8F4FF]/55 sm:text-[18px] sm:leading-7"
          />
          <div className="flex items-center justify-between gap-3 pt-2">
            <span aria-live="polite" className="flex min-w-0 items-center gap-1.5 px-2 text-[12px] text-[#E8F4FF]/55">
              {sent ? (
                <span className="lk-fade-in inline-flex items-center gap-1.5 text-[#E8F4FF]/85">
                  <Check className="size-3.5 shrink-0 text-[#7be0b0]" aria-hidden="true" />
                  {text.opening}
                </span>
              ) : (
                <span className="inline-flex items-center gap-1.5 max-sm:hidden">
                  <Mail className="size-3.5 shrink-0" aria-hidden="true" />
                  {to}
                </span>
              )}
            </span>
            <button
              type="submit"
              className={cn(
                "inline-flex h-10 shrink-0 items-center gap-2 rounded-full pr-1.5 pl-4 text-[14px] font-medium transition-[background-color,color,transform] duration-200 ease-out active:scale-[0.97] motion-reduce:transition-none",
                value.trim()
                  ? "bg-[#44A6FA] text-[#020514] hover:bg-[#6bbaff]"
                  : "bg-[#E8F4FF]/12 text-[#E8F4FF] hover:bg-[#E8F4FF]/20",
              )}
            >
              {text.send}
              <span
                className={cn(
                  "grid size-7 place-items-center rounded-full transition-colors duration-200",
                  value.trim() ? "bg-[#020514] text-[#E8F4FF]" : "bg-[#E8F4FF] text-[#020514]",
                )}
              >
                <ArrowUp className="size-3.5" aria-hidden="true" />
              </span>
            </button>
          </div>
        </form>
      </div>

      {/* Desktop: chips. Phones: quiet rows. Either fills the box. */}
      <ul className="mt-5 flex flex-wrap justify-center gap-2 max-sm:w-full max-sm:flex-col max-sm:gap-0">
        {text.examples.slice(0, 3).map((example) => (
          <li key={example}>
            <button
              type="button"
              onClick={() => {
                setValue(example);
                fieldRef.current?.focus();
              }}
              className="group rounded-full border border-[#E8F4FF]/18 bg-[#E8F4FF]/[0.04] px-3.5 py-1.5 text-[13px] text-[#E8F4FF]/85 transition-[border-color,color,background-color,transform] duration-200 ease-out hover:border-[#E8F4FF]/45 hover:bg-[#E8F4FF]/[0.08] hover:text-[#E8F4FF] active:scale-[0.98] motion-reduce:transition-none max-sm:flex max-sm:min-h-12 max-sm:w-full max-sm:items-center max-sm:justify-between max-sm:gap-4 max-sm:rounded-none max-sm:border-0 max-sm:border-b max-sm:border-[#E8F4FF]/10 max-sm:bg-transparent max-sm:px-0 max-sm:text-left max-sm:text-[15px] max-sm:text-[#E8F4FF]/70"
            >
              {example}
              <ArrowUpLeft className="size-4 shrink-0 text-[#E8F4FF]/35 sm:hidden" aria-hidden="true" />
            </button>
          </li>
        ))}
      </ul>
    </>
  );
}
