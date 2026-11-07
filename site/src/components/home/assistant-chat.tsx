import * as React from "react";
import { ArrowUp, Check, Clock, GitBranch, Pause } from "lucide-react";
import { cn } from "@/lib/utils";
import type { HomeCopy } from "@/lib/copy";
import { APPROVAL_TARGET, REGIONS } from "@/lib/home-content";
import { MascotMark } from "@/components/shared/mascot-svg";
import { AssistantPlay } from "./assistant-play";
import { ALL_STEPS, S } from "./assistant-steps";

/**
 * WEB-5 O19: the assistant panel of "Ask, don't dig" (owner, 2026-11-07:
 * "ui ini juga jelek banget, coba perbaiki"). A product-grade chat that
 * tells its story by moving, once, when it comes into view, then rests:
 *
 * 1. the first question types into the composer and is sent;
 * 2. the assistant works through two steps (reading a Gold table, grouping
 *    it by region), which fold into a "2 steps" summary;
 * 3. the answer streams in word by word, and the bars grow one after
 *    another, the leader in the lake accent; a bar shows its value on hover;
 * 4. the composer switches from Ask to Build (the console's own modes) and
 *    the second request is sent;
 * 5. the approval card, the hero moment: the assistant does not pause the
 *    pipeline itself, the request waits for approval, and an admin approves
 *    it ("Approved by Admin · just now"). That is the claim: a person
 *    approves risky actions (AI-18), and not the person who asked (SEC-57).
 *
 * WEB-5 D5 and the initial-JS budget: this is a server component, and every
 * state of the conversation is CSS keyed on one attribute, `data-s`, the
 * list of steps reached (`assistant-steps.ts`). The server renders the last
 * step, so without JavaScript, under reduced motion, and when the panel is
 * already on screen at hydration the finished conversation shows, still.
 * `AssistantPlay` (a few lines of client code) rewinds it only when it will
 * be seen playing, and loads the timed script (`assistant-script.ts`) as a
 * separate chunk when the panel comes into view. Nothing follows the scroll.
 *
 * The mock is decorative (`aria-hidden`) and described by a visually hidden
 * sentence. O32 (owner, 2026-11-07): no visible "Illustration" tag, as
 * visitors can tell it is not real data.
 */

type AssistantCopy = HomeCopy["assistant"];

/** The Gold table the illustrative answer reads: generic, not a client's. */
const SOURCE_TABLE = "gold.visits_by_region";

export function AssistantChat({ copy }: { copy: AssistantCopy }) {
  const fmt = new Intl.NumberFormat(copy.numberLocale, { notation: "compact", maximumFractionDigits: 1 });
  const full = new Intl.NumberFormat(copy.numberLocale);
  const leader = copy.regions.north;
  const max = REGIONS[0].value;

  return (
    <div>
      <p className="sr-only">{copy.srDescription}</p>
      <AssistantPlay>
        <div aria-hidden="true" className="lk-ast lk-ast-still" data-s={ALL_STEPS}>
          {/* Header: the guide's mark (WEB-5 D14 S4), its name and state. O32: no visible "Illustration" tag (owner); the sr-only description stays. */}
          <div className="flex h-12 shrink-0 items-center gap-2.5 border-b border-[#E8F4FF]/10 px-4">
            <span className="lk-ast-avatar grid size-7 place-items-center rounded-full">
              <MascotMark className="h-[14px] w-auto" />
            </span>
            <span className="text-[14px] leading-5 font-medium text-[#E8F4FF]">{copy.name}</span>
            <span className="lk-ast-state inline-flex items-center gap-1.5 text-[12px] leading-4 text-[#E8F4FF]/70">
              <span className="lk-ast-state-dot size-1.5 rounded-full" />
              <span className="lk-ast-state-ready">{copy.status.ready}</span>
              <span className="lk-ast-state-working">{copy.status.working}</span>
            </span>
          </div>

          {/* The conversation, anchored to the composer like a real chat: new messages push the older ones up. */}
          <div className="lk-ast-log relative flex min-h-0 flex-1 flex-col justify-end px-4 pt-4 pb-3">
            {/* The empty chat before the first question: the guide, waiting. */}
            <span className="lk-ast-empty pointer-events-none absolute inset-0 grid place-items-center">
              <MascotMark className="h-12 w-auto" />
            </span>
            <Grow from={S.ask1}>
              <UserBubble text={copy.userOne} />
            </Grow>

            <Grow from={S.think1a}>
              <div className="flex gap-2.5">
                <GuideMark />
                <div className="min-w-0 flex-1">
                  {/* The work, while it runs; then folded into one line. */}
                  <Grow from={S.think1a} until={S.answer}>
                    <ol className="space-y-1.5 pt-1 pb-2">
                      <WorkStep doneAt={S.think1b} label={copy.steps.read} code={SOURCE_TABLE} />
                      <Grow from={S.think1b}>
                        <WorkStep doneAt={S.answer} label={copy.steps.group} />
                      </Grow>
                    </ol>
                  </Grow>
                  <Grow from={S.answer}>
                    <span className="lk-ast-summary inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[11px] leading-4">
                      <Check className="size-3" strokeWidth={2.75} />
                      {copy.steps.done}
                      <span className="text-[#E8F4FF]/40">·</span>
                      <code className="!text-[11px] !leading-4">{SOURCE_TABLE}</code>
                    </span>
                    <div className="lk-ast-answer mt-2 rounded-2xl rounded-tl-md px-3.5 py-3">
                      <p className="text-[14px] leading-[21px] text-[#E8F4FF]">
                        {copy.answerOne.split(" ").map((w, i) => (
                          <span
                            key={i}
                            className={cn("lk-ast-word", w.replace(/[.,]/g, "") === leader && "font-semibold text-[#8FCBFF]")}
                            style={{ animationDelay: `${i * 55}ms` }}
                          >
                            {w}{" "}
                          </span>
                        ))}
                      </p>
                      <Chart copy={copy} fmt={fmt} full={full} max={max} />
                    </div>
                  </Grow>
                </div>
              </div>
            </Grow>

            <Grow from={S.ask2}>
              <UserBubble text={copy.userTwo} />
            </Grow>

            <Grow from={S.think2}>
              <div className="flex gap-2.5">
                <GuideMark />
                <div className="min-w-0 flex-1">
                  <Grow from={S.think2} until={S.card}>
                    <ol className="pt-1 pb-2">
                      <WorkStep doneAt={S.card} label={copy.steps.check} code={APPROVAL_TARGET} />
                    </ol>
                  </Grow>
                  <Grow from={S.card}>
                    <ApprovalCard copy={copy} />
                  </Grow>
                </div>
              </div>
            </Grow>
          </div>

          {/* The composer: the console's placeholder and its Ask / Build modes. */}
          <div className="shrink-0 px-3 pb-3">
            <div className="lk-ast-composer rounded-2xl px-3.5 pt-3 pb-2.5">
              {/* One line: while a question types, the line keeps its end in view, like a real input. */}
              <p className="relative flex h-5 justify-end overflow-hidden text-[14px] leading-5 text-[#E8F4FF]">
                <span className="lk-ast-placeholder absolute inset-0 truncate text-[#E8F4FF]/50">{copy.placeholder}</span>
                <span className="mr-auto shrink-0 whitespace-pre">
                  <span data-typed />
                  <span className="lk-ast-caret ml-px inline-block h-[1.05em] w-px translate-y-[3px] bg-[#8FCBFF]" />
                </span>
              </p>
              <div className="mt-1.5 flex items-center justify-between">
                <span className="lk-ast-modes relative inline-flex rounded-lg bg-[#E8F4FF]/[0.06] p-0.5 text-[12px] leading-4 font-medium">
                  <span className="lk-ast-mode-thumb absolute inset-y-0.5 left-0.5 w-[calc(50%-2px)] rounded-md" />
                  <span className="lk-ast-mode relative min-w-12 px-2.5 py-1 text-center" data-mode="ask">
                    {copy.modes.ask}
                  </span>
                  <span className="lk-ast-mode relative min-w-12 px-2.5 py-1 text-center" data-mode="build">
                    {copy.modes.build}
                  </span>
                </span>
                <span className="lk-ast-send grid size-8 place-items-center rounded-full text-white" title={copy.send}>
                  <ArrowUp className="size-4" strokeWidth={2.5} />
                </span>
              </div>
            </div>
          </div>
        </div>
      </AssistantPlay>
    </div>
  );
}

/** Opens its content's height from zero, so a new message pushes the log up instead of jumping it. */
function Grow({ from, until, children }: { from: number; until?: number; children: React.ReactNode }) {
  return (
    <div className="lk-ast-grow" data-from={from} data-until={until}>
      <div className="min-h-0">{children}</div>
    </div>
  );
}

function UserBubble({ text }: { text: string }) {
  return (
    <div className="flex justify-end">
      <p className="lk-ast-user max-w-[85%] rounded-2xl rounded-br-md px-3.5 py-2 text-[14px] leading-5 text-[#E8F4FF]" data-q>
        {text}
      </p>
    </div>
  );
}

/** WEB-5 D14 S4, the guide: the assistant's side carries the creature's mark the visitor met at the surface. */
function GuideMark() {
  return (
    <span className="lk-ast-avatar mt-0.5 grid size-7 shrink-0 place-items-center rounded-full">
      <MascotMark className="h-[14px] w-auto" />
    </span>
  );
}

function WorkStep({ doneAt, label, code }: { doneAt: number; label: string; code?: string }) {
  return (
    <li className="lk-ast-step flex items-center gap-2 text-[12px] leading-5" data-done-at={doneAt}>
      <span className="lk-ast-step-icon grid size-4 shrink-0 place-items-center rounded-full">
        <Check className="lk-ast-step-check size-2.5" strokeWidth={3} />
      </span>
      <span className="lk-ast-step-label min-w-0 truncate">
        {label}
        {code && (
          <>
            {" "}
            <code className="!text-[11.5px] !leading-5 text-[#E8F4FF]/85">{code}</code>
          </>
        )}
      </span>
    </li>
  );
}

/**
 * The answer's chart: a baseline, a value on each bar and the leader in the
 * accent, the others muted. Vertical bars where the panel is wide, and
 * horizontal ones on a phone (a container query), both from one list.
 */
function Chart({
  copy,
  fmt,
  full,
  max,
}: {
  copy: AssistantCopy;
  fmt: Intl.NumberFormat;
  full: Intl.NumberFormat;
  max: number;
}) {
  return (
    <div className="lk-ast-chart mt-3">
      <ol className="lk-ast-bars">
        {REGIONS.map((r, i) => (
          <li
            key={r.key}
            className="lk-ast-bar-item"
            data-lead={i === 0 || undefined}
            style={{ "--v": r.value / max, "--i": i } as React.CSSProperties}
          >
            <span className="lk-ast-bar-label">{copy.regions[r.key]}</span>
            <span className="lk-ast-bar-track">
              <span className="lk-ast-bar" />
              <span className="lk-ast-bar-value tabular-nums">{fmt.format(r.value)}</span>
              <span className="lk-ast-tip tabular-nums">
                {copy.regions[r.key]} · {full.format(r.value)} {copy.unit}
              </span>
            </span>
          </li>
        ))}
      </ol>
    </div>
  );
}

function ApprovalCard({ copy }: { copy: AssistantCopy }) {
  return (
    // The status sits on the card's top edge, like a tag, so the title keeps the full width on a phone.
    <div className="lk-ast-card relative mt-3 rounded-2xl p-3.5 md:p-4">
      <span className="lk-ast-pill absolute -top-3 right-3 inline-flex h-6 items-center rounded-full px-2.5 text-[12px] leading-4 font-medium whitespace-nowrap">
        <span className="lk-ast-pill-wait inline-flex items-center gap-1.5">
          <span className="lk-ast-pill-dot size-1.5 rounded-full" />
          {copy.approvalLabel}
        </span>
        <span className="lk-ast-pill-ok absolute inset-0 inline-flex items-center justify-center gap-1">
          <Check className="size-3.5" strokeWidth={3} />
          {copy.approvedLabel}
        </span>
      </span>
      <div className="flex min-w-0 items-center gap-2.5">
        <span className="lk-ast-card-icon grid size-9 shrink-0 place-items-center rounded-xl">
          <Pause className="size-4" strokeWidth={2.5} fill="currentColor" />
        </span>
        <div className="min-w-0">
          <p className="truncate text-[15px] leading-5 font-semibold text-[#E8F4FF]">{copy.approvalAction}</p>
          <p className="mt-1 flex items-center gap-1.5">
            <GitBranch className="size-3 text-[#E8F4FF]/60" strokeWidth={2.25} />
            <code className="lk-ast-chip rounded-md px-1.5 !text-[12px] !leading-[18px] text-[#E8F4FF]">{APPROVAL_TARGET}</code>
          </p>
        </div>
      </div>
      <p className="mt-3 flex items-center gap-1.5 text-[13px] leading-5 text-[#E8F4FF]/80">
        <Clock className="size-3.5 shrink-0 text-[#E8F4FF]/60" strokeWidth={2.25} />
        {copy.impact}
      </p>
      <div className="relative mt-3 h-9 border-t border-[#E8F4FF]/10 pt-3 [box-sizing:content-box]">
        <div className="lk-ast-decide absolute inset-x-0 bottom-0 flex h-9 items-center justify-between gap-2">
          <span className="flex min-w-0 items-center gap-2 text-[12px] leading-4 text-[#E8F4FF]/70">
            <span className="lk-ast-admin grid size-6 shrink-0 place-items-center rounded-full text-[11px] font-semibold text-[#E8F4FF]">A</span>
            {/* "Admin" is the console's role name, the same in both languages; a phone shows only it. */}
            <span className="lk-ast-reviewer truncate">{copy.reviewer}</span>
            <span className="lk-ast-reviewer-short">Admin</span>
          </span>
          <span className="flex shrink-0 gap-2">
            <span className="lk-ast-btn lk-ast-decline inline-flex h-9 items-center rounded-full px-4 text-[13px] font-medium">{copy.decline}</span>
            <span className="lk-ast-btn lk-ast-approve inline-flex h-9 items-center rounded-full px-4 text-[13px] font-semibold text-white">
              {copy.approve}
            </span>
          </span>
        </div>
        <p className="lk-ast-done absolute inset-x-0 bottom-0 flex h-9 items-center gap-2 text-[13px] leading-5 font-medium">
          <span className="lk-ast-done-icon grid size-6 place-items-center rounded-full">
            <Check className="size-3.5" strokeWidth={3} />
          </span>
          {copy.approvedBy}
        </p>
      </div>
    </div>
  );
}
