"use client";

import * as React from "react";
import { m, useMotionValueEvent, useTransform, type MotionValue } from "framer-motion";
import type { HomeCopy } from "@/lib/copy";
import { APPROVAL_TARGET, REGIONS } from "@/lib/home-content";
import { useProgress } from "@/components/shared/progress-section";
import { MascotMark } from "@/components/shared/mascot-svg";
import { useReducedMotionSafe } from "@/hooks/use-reduced-motion";

/**
 * The assistant section's scripted chat: the client leaf of
 * `assistant-section.tsx` (WEB-5 review fix R5). The heading, the lead and
 * the caustic lights render on the server there.
 */

type AssistantCopy = HomeCopy["assistant"];

/** Beats of the script, as fractions of the pinned scroll. */
const T = {
  type1: [0.05, 0.25],
  think1: [0.25, 0.34],
  answer: [0.34, 0.42],
  bars: [0.4, 0.58],
  type2: [0.6, 0.72],
  think2: [0.72, 0.78],
  approval: [0.78, 0.9],
} as const;

const clamp01 = (x: number) => Math.min(1, Math.max(0, x));
const span = (p: number, [a, b]: readonly [number, number]) => clamp01((p - a) / (b - a));

/** The chat, driven by the enclosing section's scroll progress. */
export function AssistantChat({ copy }: { copy: AssistantCopy }) {
  const reduced = useReducedMotionSafe();
  const progress = useProgress();
  return <ChatWindow copy={copy} progress={progress} reduced={reduced} />;
}

// Every `m` element below whose style switches on `reduced` is keyed on it:
// before motion's features load an `m` element keeps the style it first
// rendered (WEB-5 review fix R1), and `reduced` is only known after
// hydration, so a switch of style must remount the element. Each key also
// names its element: siblings must not share a key (R1 follow-up).
function ChatWindow({ copy, progress, reduced }: { copy: AssistantCopy; progress: MotionValue<number>; reduced: boolean }) {
  // Opacity and travel of each beat, from the same progress.
  const frame = useTransform(progress, [0, 0.05], [0, 1]);
  const think1 = useTransform(progress, [T.think1[0], T.think1[0] + 0.02, T.think1[1] - 0.02, T.think1[1]], [0, 1, 1, 0]);
  const answerO = useTransform(progress, [T.answer[0], T.answer[1]], [0, 1]);
  const answerY = useTransform(progress, [T.answer[0], T.answer[1]], [18, 0]);
  const think2 = useTransform(progress, [T.think2[0], T.think2[0] + 0.02, T.think2[1] - 0.02, T.think2[1]], [0, 1, 1, 0]);
  const approvalO = useTransform(progress, [T.approval[0], T.approval[1]], [0, 1]);
  const approvalY = useTransform(progress, [T.approval[0], T.approval[1]], [34, 0]);
  const approvalS = useTransform(progress, [T.approval[0], T.approval[1]], [0.96, 1]);

  const max = REGIONS[0].value;
  return (
    <div>
      <p className="sr-only">{copy.srDescription}</p>
      {/* `data-reveal`: without JavaScript the finished conversation shows, as with reduced motion (D5). */}
      <m.div
        data-reveal
        aria-hidden="true"
        className="overflow-hidden rounded-3xl border border-[#E8F4FF]/12 bg-[#020514]/70 shadow-[0_30px_120px_-30px_rgba(68,166,250,0.35)] backdrop-blur-xl"
        key={reduced ? "frame-reduced" : "frame-motion"}
        style={reduced ? { opacity: 1 } : { opacity: frame }}
      >
        <div className="flex items-center justify-between border-b border-[#E8F4FF]/10 px-4 py-2 md:py-3">
          <div className="flex items-center gap-1.5">
            <span className="size-2.5 rounded-full bg-[#E8F4FF]/20" />
            <span className="size-2.5 rounded-full bg-[#E8F4FF]/20" />
            <span className="size-2.5 rounded-full bg-[#E8F4FF]/20" />
          </div>
          <p className="text-[11px] tracking-[0.2em] text-[#E8F4FF]/60 uppercase">{copy.caption}</p>
        </div>

        <div className="space-y-2 px-4 py-3 md:space-y-4 md:px-5 md:py-5">
          {/* Question one */}
          <TypedQuestion text={copy.userOne} beat={T.type1} progress={progress} reduced={reduced} />

          {/* Answer one: thinking, then the answer with its chart. Heights are reserved so nothing jumps. */}
          <div className="relative min-h-[120px] pl-7 md:min-h-[158px] md:pl-9">
            <GuideMark />
            <m.div
              className="absolute top-0 left-7 flex h-9 md:left-9 items-center gap-1.5 rounded-2xl rounded-bl-md bg-[#E8F4FF]/[0.06] px-4"
              key={reduced ? "think1-reduced" : "think1-motion"}
              style={reduced ? { opacity: 0 } : { opacity: think1 }}
            >
              <Dots />
            </m.div>
            <m.div
              data-reveal
              className="rounded-2xl rounded-bl-md bg-[#E8F4FF]/[0.06] px-3.5 py-3 md:max-w-[92%] md:px-4"
              key={reduced ? "answer-reduced" : "answer-motion"}
              style={reduced ? { opacity: 1, y: 0 } : { opacity: answerO, y: answerY }}
            >
              <p className="text-[13px] leading-5 text-[#E8F4FF] md:text-[15px] md:leading-6">{copy.answerOne}</p>
              <div className="mt-3 flex h-[52px] items-end gap-2.5 md:h-[68px] md:gap-3">
                {REGIONS.map((r, i) => (
                  <Bar key={r.key} progress={progress} index={i} height={r.value / max} reduced={reduced} label={copy.regions[r.key]} value={r.value} />
                ))}
              </div>
            </m.div>
          </div>

          {/* Question two */}
          <TypedQuestion text={copy.userTwo} beat={T.type2} progress={progress} reduced={reduced} />

          {/* Answer two: thinking, then the approval card. */}
          <div className="relative min-h-[92px] pl-7 md:min-h-[104px] md:pl-9">
            <GuideMark />
            <m.div
              className="absolute top-0 left-7 flex h-9 md:left-9 items-center gap-1.5 rounded-2xl rounded-bl-md bg-[#E8F4FF]/[0.06] px-4"
              key={reduced ? "think2-reduced" : "think2-motion"}
              style={reduced ? { opacity: 0 } : { opacity: think2 }}
            >
              <Dots />
            </m.div>
            <m.div
              data-reveal
              className="rounded-2xl border border-[#44A6FA]/45 bg-[#44A6FA]/[0.08] p-3.5 md:p-4"
              key={reduced ? "approval-reduced" : "approval-motion"}
              style={reduced ? { opacity: 1, y: 0, scale: 1 } : { opacity: approvalO, y: approvalY, scale: approvalS }}
            >
              <div className="flex items-center gap-2">
                <span className="lk-pulse size-2 rounded-full bg-[#44A6FA]" />
                <p className="text-[11px] tracking-[0.16em] text-[#44A6FA] uppercase">
                  {copy.approvalLabel}
                </p>
              </div>
              <p className="mt-2 text-[13px] leading-5 text-[#E8F4FF] md:text-[15px]">
                {copy.approvalAction} <code className="text-[#E8F4FF]">{APPROVAL_TARGET}</code>
              </p>
              <div className="mt-3 flex gap-2">
                <span className="rounded-full bg-[#44A6FA] px-4 py-1.5 text-[12px] font-medium text-[#020514]">
                  {copy.approve}
                </span>
                <span className="rounded-full border border-[#E8F4FF]/30 px-4 py-1.5 text-[12px] text-[#E8F4FF]">
                  {copy.decline}
                </span>
              </div>
            </m.div>
          </div>
        </div>
      </m.div>
    </div>
  );
}

function Bar({
  progress,
  index,
  height,
  reduced,
  label,
  value,
}: {
  progress: MotionValue<number>;
  index: number;
  height: number;
  reduced: boolean;
  label: string;
  value: number;
}) {
  const a = T.bars[0] + index * 0.025;
  const scaleY = useTransform(progress, [a, a + 0.08], [0, 1]);
  return (
    <div className="flex h-full flex-1 flex-col justify-end gap-1">
      <m.div
        data-reveal
        className="w-full origin-bottom rounded-t-md bg-gradient-to-t from-[#44A6FA]/60 to-[#44A6FA]"
        key={reduced ? "reduced" : "motion"}
        style={{ height: `${Math.round(height * 100)}%`, scaleY: reduced ? 1 : scaleY }}
      />
      <span className="text-center text-[9px] leading-3 text-[#E8F4FF]/60 md:text-[10px]">
        {label} {Math.round(value / 100) / 10}k
      </span>
    </div>
  );
}

/**
 * WEB-5 D14 S4, the guide: the assistant's side of the chat carries the
 * creature's mark, the same character the visitor met at the surface and
 * will see rise again at the end. Decorative, like the whole mock, which is
 * `aria-hidden` and described in words for assistive tech.
 */
function GuideMark() {
  return (
    // Phones: a 20 px mark and a 28 px indent, with the answer at full width,
    // so the text has the width it had before the mark (the pinned stage still
    // fits 360x740).
    <span className="absolute top-1 left-0 grid size-5 place-items-center rounded-full bg-[#44A6FA]/15 ring-1 ring-[#44A6FA]/35 md:size-7">
      <MascotMark className="size-3.5 md:size-[18px]" />
    </span>
  );
}

function Dots() {
  return (
    <>
      {[0, 1, 2].map((i) => (
        <span
          key={i}
          className="lk-twinkle size-1.5 rounded-full bg-[#E8F4FF]/70"
          style={{ animationDelay: `${i * 0.25}s` }}
        />
      ))}
    </>
  );
}

/**
 * A question that types itself as the section scrolls, and un-types when it
 * scrolls back. WEB-5 D5: the typed text, the bubble's visibility and the
 * caret are written straight to the DOM through refs, and only when the
 * character count changes, so scrolling never re-renders the chat (it used
 * to be React state updated from every scroll event). The server HTML holds
 * the whole question, so it reads without JavaScript; the first effect sets
 * it to the current scroll position.
 */
function TypedQuestion({
  text,
  beat,
  progress,
  reduced,
}: {
  text: string;
  beat: readonly [number, number];
  progress: MotionValue<number>;
  reduced: boolean;
}) {
  const bubbleRef = React.useRef<HTMLParagraphElement>(null);
  const textRef = React.useRef<HTMLSpanElement>(null);
  const caretRef = React.useRef<HTMLSpanElement>(null);
  const shown = React.useRef(-1);

  const apply = React.useCallback(
    (p: number) => {
      const n = reduced ? text.length : Math.round(span(p, beat) * text.length);
      if (n === shown.current) return;
      shown.current = n;
      if (textRef.current) textRef.current.textContent = text.slice(0, n);
      bubbleRef.current?.classList.toggle("invisible", n === 0);
      if (caretRef.current) caretRef.current.hidden = !(n > 0 && n < text.length);
    },
    [text, beat, reduced],
  );
  React.useEffect(() => {
    shown.current = -1;
    apply(progress.get());
  }, [apply, progress]);
  useMotionValueEvent(progress, "change", apply);

  return (
    <div className="flex min-h-9 justify-end">
      <p
        ref={bubbleRef}
        className="max-w-[88%] rounded-2xl rounded-br-md bg-[#44A6FA]/20 px-3.5 py-2 text-[13px] leading-5 text-[#E8F4FF] md:text-[15px] md:leading-6"
      >
        <span ref={textRef}>{text}</span>
        <span
          ref={caretRef}
          hidden
          className="lk-twinkle ml-0.5 inline-block h-[1em] w-px translate-y-[2px] bg-[#E8F4FF]"
        />
      </p>
    </div>
  );
}
