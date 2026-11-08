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
 *
 * O35 (owner: "lebih minimalist clean dan profesional ... Background dark
 * aja gausah gradient. Gausah ada tulisan ready dan dot hijau. kurang
 * border"): one flat dark surface with a single hairline; no status, no
 * dividers, no nested cards; the parts are told apart by fill and spacing.
 * O41 (owner: "buat heightnya langsung fix aja, jangan ngikutin isi
 * content. biar kesannya jadi kek chat asli"): the panel has a fixed
 * height, like a chat window. The header stays at the top, the composer at
 * the bottom, and the messages stack from the bottom up between them; each
 * new one pushes the earlier ones up, and the oldest slide out of the top
 * under a short fade. The finished conversation shows the approval card and
 * the last reply whole; the first question may be faded at the top.
 */

type AssistantCopy = HomeCopy["assistant"];

/** The Gold table the illustrative answer reads: generic, not a client's. */
const SOURCE_TABLE = "gold.visits_by_region";

export function AssistantChat({ copy }: { copy: AssistantCopy }) {
  return (
    <div>
      <p className="sr-only">{copy.srDescription}</p>
      <AssistantPlay>
        <Panel copy={copy} />
      </AssistantPlay>
    </div>
  );
}

function Panel({ copy }: { copy: AssistantCopy }) {
  const fmt = new Intl.NumberFormat(copy.numberLocale, { notation: "compact", maximumFractionDigits: 1 });
  const full = new Intl.NumberFormat(copy.numberLocale);
  const leader = copy.regions.north;
  const max = REGIONS[0].value;
  return (
    <div aria-hidden="true" className="lk-ast lk-ast-still" data-s={ALL_STEPS}>
      {/* Header: the guide's mark (WEB-5 D14 S4) and its name. O35: no status, no divider. */}
      <div className="flex items-center gap-3 px-5 pt-4">
        {/* The mark sits in the avatar's own 24px slot, so both marks share one vertical axis. */}
        <span className="grid size-6 shrink-0 place-items-center">
          <MascotMark className="h-[11px] w-auto" />
        </span>
        <span className="text-[13px] leading-5 font-medium text-[#E8F4FF]/80">{copy.name}</span>
      </div>

      {/* O41: the conversation is its own viewport between the header and the
          composer, stacked from the bottom; older messages slide out of the
          top under a short fade. */}
      <div className="lk-ast-log flex min-h-0 flex-1 flex-col justify-end overflow-hidden px-5 pt-1 pb-4">
        <Grow from={S.ask1}>
          <UserBubble text={copy.userOne} />
        </Grow>

        <Grow from={S.think1a}>
          <div className="flex gap-3">
            {/* The mark once, beside the assistant's first reply. */}
            <GuideMark />
            <div className="min-w-0 flex-1">
              {/* The work, while it runs; then folded into one quiet line. */}
              <Grow from={S.think1a} until={S.answer}>
                <ol className="space-y-1 pb-2">
                  <WorkStep doneAt={S.think1b} label={copy.steps.read} code={SOURCE_TABLE} />
                  <Grow from={S.think1b}>
                    <WorkStep doneAt={S.answer} label={copy.steps.group} />
                  </Grow>
                </ol>
              </Grow>
              <Grow from={S.answer}>
                <p className="lk-ast-summary flex items-center gap-1.5 text-[13px] leading-5">
                  <Check className="size-3.5 shrink-0" strokeWidth={2.5} />
                  {copy.steps.done}
                  <span aria-hidden="true">·</span>
                  <code className="truncate !text-[12px] !leading-5">{SOURCE_TABLE}</code>
                </p>
                <p className="mt-2 text-[14px] leading-[22px] text-[#E8F4FF]">
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
              </Grow>
            </div>
          </div>
        </Grow>

        <Grow from={S.ask2}>
          <UserBubble text={copy.userTwo} />
        </Grow>

        <Grow from={S.think2}>
          <div className="flex gap-3">
            {/* The second reply lines up with the first, without repeating the mark. */}
            <span aria-hidden="true" className="w-6 shrink-0" />
            <div className="min-w-0 flex-1">
              <Grow from={S.think2} until={S.card}>
                <ol className="pb-2">
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

      {/* The composer: the console's placeholder and its Ask / Build modes, flat. */}
      <div className="px-3 pb-3">
        <div className="lk-ast-composer rounded-xl px-4 pt-3 pb-2.5">
          {/* One line: while a question types, the line keeps its end in view, like a real input. */}
          <p className="relative flex h-5 justify-end overflow-hidden text-[14px] leading-5 text-[#E8F4FF]">
            <span className="lk-ast-placeholder absolute inset-0 truncate text-[#E8F4FF]/45">{copy.placeholder}</span>
            <span className="mr-auto shrink-0 whitespace-pre">
              <span data-typed />
              <span className="lk-ast-caret ml-px inline-block h-[1.05em] w-px translate-y-[3px] bg-[#8FCBFF]" />
            </span>
          </p>
          <div className="mt-2.5 flex items-center justify-between">
            <span className="lk-ast-modes relative inline-flex rounded-md text-[13px] leading-5">
              <span className="lk-ast-mode-thumb absolute inset-y-0 left-0 w-1/2 rounded-md" />
              <span className="lk-ast-mode relative min-w-12 px-2.5 py-0.5 text-center" data-mode="ask">
                {copy.modes.ask}
              </span>
              <span className="lk-ast-mode relative min-w-12 px-2.5 py-0.5 text-center" data-mode="build">
                {copy.modes.build}
              </span>
            </span>
            <span className="lk-ast-send grid size-7 place-items-center rounded-full" title={copy.send}>
              <ArrowUp className="size-3.5" strokeWidth={2.5} />
            </span>
          </div>
        </div>
      </div>
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
      <p className="lk-ast-user max-w-[85%] rounded-2xl rounded-br-md px-3.5 py-2 text-[14px] leading-[22px] text-[#E8F4FF]" data-q>
        {text}
      </p>
    </div>
  );
}

/** WEB-5 D14 S4, the guide: the assistant's side carries the creature's mark the visitor met at the surface. */
function GuideMark() {
  return (
    <span className="lk-ast-avatar mt-px grid size-6 shrink-0 place-items-center rounded-full">
      <MascotMark className="h-[11px] w-auto" />
    </span>
  );
}

function WorkStep({ doneAt, label, code }: { doneAt: number; label: string; code?: string }) {
  return (
    <li className="lk-ast-step flex items-center gap-2 text-[13px] leading-5" data-done-at={doneAt}>
      <span className="lk-ast-step-icon grid size-3.5 shrink-0 place-items-center rounded-full">
        <Check className="lk-ast-step-check size-3.5" strokeWidth={2.5} />
      </span>
      <span className="lk-ast-step-label min-w-0 truncate">
        {label}
        {code && (
          <>
            {" "}
            <code className="!text-[12px] !leading-5">{code}</code>
          </>
        )}
      </span>
    </li>
  );
}

/**
 * The answer's chart: a thin baseline, a small value on each bar, the
 * leader in the solid accent and the others one flat muted tone (O35: no
 * gradients, no glow). Vertical bars where the panel is wide, horizontal
 * ones on a phone (a container query), both from one list.
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
    <div className="lk-ast-chart mt-4">
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

/** The approval card, flat like the rest: its status a small text pill, Approve the one primary button. */
function ApprovalCard({ copy }: { copy: AssistantCopy }) {
  return (
    <div className="lk-ast-card rounded-xl p-4">
      <span className="lk-ast-pill relative inline-flex h-6 items-center rounded-full px-2.5 text-[12px] leading-4 font-medium whitespace-nowrap">
        <span className="lk-ast-pill-wait inline-flex items-center gap-1.5">
          <span className="lk-ast-pill-dot size-1.5 rounded-full" />
          {copy.approvalLabel}
        </span>
        <span className="lk-ast-pill-ok inline-flex items-center justify-center gap-1">
          <Check className="size-3.5" strokeWidth={3} />
          {copy.approvedLabel}
        </span>
      </span>
      <div className="mt-3 flex min-w-0 items-center gap-3">
        <span className="lk-ast-card-icon grid size-8 shrink-0 place-items-center rounded-lg">
          <Pause className="size-3.5" strokeWidth={2.5} fill="currentColor" />
        </span>
        <div className="min-w-0">
          <p className="truncate text-[15px] leading-5 font-semibold text-[#E8F4FF]">{copy.approvalAction}</p>
          <p className="mt-0.5 flex items-center gap-1.5 text-[13px] leading-5 text-[#E8F4FF]/65">
            <GitBranch className="size-3 shrink-0" strokeWidth={2.25} />
            <code className="!text-[12px] !leading-5 text-[#E8F4FF]/85">{APPROVAL_TARGET}</code>
          </p>
        </div>
      </div>
      <p className="mt-3 flex items-center gap-1.5 text-[13px] leading-5 text-[#E8F4FF]/70">
        <Clock className="size-3.5 shrink-0" strokeWidth={2.25} />
        {copy.impact}
      </p>
      <div className="relative mt-4 h-9">
        <div className="lk-ast-decide absolute inset-0 flex items-center justify-between gap-2">
          <span className="flex min-w-0 items-center gap-2 text-[13px] leading-5 text-[#E8F4FF]/65">
            <span className="lk-ast-admin grid size-6 shrink-0 place-items-center rounded-full text-[11px] font-semibold text-[#E8F4FF]">A</span>
            {/* "Admin" is the console's role name, the same in both languages; a phone shows only it. */}
            <span className="lk-ast-reviewer truncate">{copy.reviewer}</span>
            <span className="lk-ast-reviewer-short">Admin</span>
          </span>
          <span className="flex shrink-0 gap-2">
            <span className="lk-ast-btn lk-ast-decline inline-flex h-9 items-center rounded-lg px-3.5 text-[13px] font-medium">{copy.decline}</span>
            <span className="lk-ast-btn lk-ast-approve inline-flex h-9 items-center rounded-lg px-3.5 text-[13px] font-semibold text-white">
              {copy.approve}
            </span>
          </span>
        </div>
        <p className="lk-ast-done absolute inset-0 flex items-center gap-2 text-[13px] leading-5 font-medium">
          <Check className="size-4 shrink-0" strokeWidth={2.75} />
          {copy.approvedBy}
        </p>
      </div>
    </div>
  );
}
