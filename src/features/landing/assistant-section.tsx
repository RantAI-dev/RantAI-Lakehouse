"use client";

import * as React from "react";
import { motion, useMotionValueEvent, useTransform, type MotionValue } from "motion/react";
import { ASSISTANT } from "./landing-copy";
import { Container, DisplayHeading, LeadText } from "./landing-ui";
import { SplitWords } from "./reveal";
import { useReducedMotionSafe } from "./use-reduced-motion";
import { useSectionProgress } from "./use-section-progress";

/**
 * S4 — "Ask, don't dig": the assistant, with a scripted chat that scrolling
 * plays and un-plays.
 *
 * The chat is DOM, not a screenshot, and every step is a pure function of
 * scroll progress, so scrolling back rewinds it: the typing, the thinking
 * shimmer, the answer with its bar chart, the second question, and the
 * approval card. Nothing here talks to an API. The mock is decorative
 * (`aria-hidden`); a visually hidden sentence describes it, and it is
 * labelled "Illustration" on screen. With reduced motion the finished
 * conversation is shown at once.
 */

const REGIONS = [
  { name: "North", value: 12400 },
  { name: "South", value: 9100 },
  { name: "East", value: 7800 },
  { name: "West", value: 5200 },
] as const;

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

export function AssistantSection() {
  const ref = React.useRef<HTMLElement>(null);
  const reduced = useReducedMotionSafe();
  const progress = useSectionProgress(ref, ["start start", "end end"]);

  return (
    <section ref={ref} id="assistant" className="relative h-[260vh] bg-[#050A30] motion-reduce:h-auto">
      <div className="sticky top-0 flex h-[100svh] items-center overflow-hidden motion-reduce:static motion-reduce:h-auto motion-reduce:py-28">
        <Caustics />
        <Container className="relative z-10 grid items-center gap-8 pt-16 md:grid-cols-[1fr_1.1fr] md:gap-14 md:pt-0">
          <div>
            <DisplayHeading size="lg" className="max-md:text-[clamp(2.6rem,12vw,3.4rem)]">
              <SplitWords text={ASSISTANT.headline} />
            </DisplayHeading>
            <LeadText className="mt-4 max-w-[32rem] max-md:text-[15px] max-md:leading-[1.5] md:mt-5">
              {ASSISTANT.sub}
            </LeadText>
          </div>
          <ChatWindow progress={progress} reduced={reduced} />
        </Container>
      </div>
    </section>
  );
}

/** Two slow radial lights, like the pattern the sun throws on a pool floor. Off under reduced motion. */
function Caustics() {
  return (
    <div aria-hidden="true" className="absolute inset-0 overflow-hidden">
      <div className="absolute inset-0 bg-[radial-gradient(ellipse_at_50%_0%,#0B1A4A_0%,#050A30_55%,#030824_100%)]" />
      <div className="lk-caustic-a absolute -top-1/4 -left-1/4 size-[80%] rounded-full bg-[radial-gradient(circle,rgba(68,166,250,0.20)_0%,transparent_62%)] blur-2xl" />
      <div className="lk-caustic-b absolute -right-1/4 -bottom-1/3 size-[90%] rounded-full bg-[radial-gradient(circle,rgba(68,166,250,0.14)_0%,transparent_60%)] blur-3xl" />
    </div>
  );
}

function ChatWindow({ progress, reduced }: { progress: MotionValue<number>; reduced: boolean }) {
  // Typed characters are React state, but only as the count changes.
  const [chars, setChars] = React.useState<[number, number]>([0, 0]);
  useMotionValueEvent(progress, "change", (p) => {
    const a = Math.round(span(p, T.type1) * ASSISTANT.userOne.length);
    const b = Math.round(span(p, T.type2) * ASSISTANT.userTwo.length);
    setChars((prev) => (prev[0] === a && prev[1] === b ? prev : [a, b]));
  });
  const [n1, n2] = reduced ? [ASSISTANT.userOne.length, ASSISTANT.userTwo.length] : chars;

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
      <p className="sr-only">{ASSISTANT.srDescription}</p>
      <motion.div
        aria-hidden="true"
        className="overflow-hidden rounded-3xl border border-[#E8F4FF]/12 bg-[#020514]/70 shadow-[0_30px_120px_-30px_rgba(68,166,250,0.35)] backdrop-blur-xl"
        style={reduced ? { opacity: 1 } : { opacity: frame }}
      >
        <div className="flex items-center justify-between border-b border-[#E8F4FF]/10 px-4 py-3">
          <div className="flex items-center gap-1.5">
            <span className="size-2.5 rounded-full bg-[#E8F4FF]/20" />
            <span className="size-2.5 rounded-full bg-[#E8F4FF]/20" />
            <span className="size-2.5 rounded-full bg-[#E8F4FF]/20" />
          </div>
          <p className="lk-mono text-[11px] tracking-[0.2em] text-[#E8F4FF]/60 uppercase">{ASSISTANT.caption}</p>
        </div>

        <div className="space-y-3 px-4 py-4 md:space-y-4 md:px-5 md:py-5">
          {/* Question one */}
          <div className="flex min-h-9 justify-end">
            <p
              className={`max-w-[88%] rounded-2xl rounded-br-md bg-[#44A6FA]/20 px-3.5 py-2 text-[13px] leading-5 text-[#E8F4FF] md:text-[15px] md:leading-6 ${n1 === 0 ? "invisible" : ""}`}
            >
              {ASSISTANT.userOne.slice(0, n1)}
              {n1 > 0 && n1 < ASSISTANT.userOne.length && <Caret />}
            </p>
          </div>

          {/* Answer one: thinking, then the answer with its chart. Heights are reserved so nothing jumps. */}
          <div className="relative min-h-[120px] md:min-h-[158px]">
            <motion.div
              className="absolute top-0 left-0 flex h-9 items-center gap-1.5 rounded-2xl rounded-bl-md bg-[#E8F4FF]/[0.06] px-4"
              style={reduced ? { opacity: 0 } : { opacity: think1 }}
            >
              <Dots />
            </motion.div>
            <motion.div
              className="max-w-[92%] rounded-2xl rounded-bl-md bg-[#E8F4FF]/[0.06] px-3.5 py-3 md:px-4"
              style={reduced ? { opacity: 1, y: 0 } : { opacity: answerO, y: answerY }}
            >
              <p className="text-[13px] leading-5 text-[#E8F4FF] md:text-[15px] md:leading-6">{ASSISTANT.answerOne}</p>
              <div className="mt-3 flex h-[52px] items-end gap-2.5 md:h-[68px] md:gap-3">
                {REGIONS.map((r, i) => (
                  <Bar key={r.name} progress={progress} index={i} height={r.value / max} reduced={reduced} label={r.name} value={r.value} />
                ))}
              </div>
            </motion.div>
          </div>

          {/* Question two */}
          <div className="flex min-h-9 justify-end">
            <p
              className={`max-w-[88%] rounded-2xl rounded-br-md bg-[#44A6FA]/20 px-3.5 py-2 text-[13px] leading-5 text-[#E8F4FF] md:text-[15px] md:leading-6 ${n2 === 0 ? "invisible" : ""}`}
            >
              {ASSISTANT.userTwo.slice(0, n2)}
              {n2 > 0 && n2 < ASSISTANT.userTwo.length && <Caret />}
            </p>
          </div>

          {/* Answer two: thinking, then the approval card. */}
          <div className="relative min-h-[92px] md:min-h-[104px]">
            <motion.div
              className="absolute top-0 left-0 flex h-9 items-center gap-1.5 rounded-2xl rounded-bl-md bg-[#E8F4FF]/[0.06] px-4"
              style={reduced ? { opacity: 0 } : { opacity: think2 }}
            >
              <Dots />
            </motion.div>
            <motion.div
              className="rounded-2xl border border-[#44A6FA]/45 bg-[#44A6FA]/[0.08] p-3.5 md:p-4"
              style={reduced ? { opacity: 1, y: 0, scale: 1 } : { opacity: approvalO, y: approvalY, scale: approvalS }}
            >
              <div className="flex items-center gap-2">
                <span className="lk-pulse size-2 rounded-full bg-[#44A6FA]" />
                <p className="lk-mono text-[11px] tracking-[0.16em] text-[#44A6FA] uppercase">
                  {ASSISTANT.approvalLabel}
                </p>
              </div>
              <p className="mt-2 text-[13px] leading-5 text-[#E8F4FF] md:text-[15px]">
                {ASSISTANT.approvalAction} <code className="lk-mono text-[#E8F4FF]">{ASSISTANT.approvalTarget}</code>
              </p>
              <div className="mt-3 flex gap-2">
                <span className="rounded-full bg-[#44A6FA] px-4 py-1.5 text-[12px] font-medium text-[#020514]">
                  {ASSISTANT.approve}
                </span>
                <span className="rounded-full border border-[#E8F4FF]/30 px-4 py-1.5 text-[12px] text-[#E8F4FF]">
                  {ASSISTANT.decline}
                </span>
              </div>
            </motion.div>
          </div>
        </div>
      </motion.div>
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
      <motion.div
        className="w-full origin-bottom rounded-t-md bg-gradient-to-t from-[#44A6FA]/60 to-[#44A6FA]"
        style={{ height: `${Math.round(height * 100)}%`, scaleY: reduced ? 1 : scaleY }}
      />
      <span className="lk-mono text-center text-[9px] leading-3 text-[#E8F4FF]/60 md:text-[10px]">
        {label} {Math.round(value / 100) / 10}k
      </span>
    </div>
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

function Caret() {
  return <span className="lk-twinkle ml-0.5 inline-block h-[1em] w-px translate-y-[2px] bg-[#E8F4FF]" />;
}
