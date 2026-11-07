"use client";

import * as React from "react";
import { useInView } from "framer-motion";
import { Lock, LockOpen, MapPin, UserRound } from "lucide-react";
import { cn } from "@/lib/utils";
import type { HomeCopy } from "@/lib/copy";
import { GOVERNANCE_COLUMNS, GOVERNANCE_ROWS } from "@/lib/home-content";

const COL_WIDTH: Record<(typeof GOVERNANCE_COLUMNS)[number]["key"], string> = {
  name: "w-[30%]",
  email: "w-[34%]",
  city: "w-[20%]",
  amount: "w-[16%]",
};
import { maskEmail, maskName, scrambleFrame } from "@/lib/scramble";
import { useReducedMotionSafe } from "@/hooks/use-reduced-motion";

/**
 * The governance section's table (WEB-5 owner QA O17, the client leaf of
 * `governance-section.tsx`): a product-grade data table of five fictional
 * customers, and the "View as" switch that shows the masking policy act.
 *
 * - The switch is a radio group with a sliding thumb (220 ms); the arrow
 *   keys move it. A policy chip beside it says what the role sees, and a
 *   live region announces it.
 * - Sensitive columns carry a "PII" tag with a tooltip on hover and focus.
 * - For Admin the cells hold the real values; switching to Admin, the
 *   masked dots decode into letters row by row, 40 ms apart
 *   (`lib/scramble`). For Analyst only the masked form exists in the DOM,
 *   as muted text that keeps the data's shape (`M••• O•••`,
 *   `m•••@•••.com`) behind a small lock with a "Masked by policy" tooltip,
 *   and the avatar is a neutral circle with a person glyph (O36: the blurred
 *   bars and blobs read as loading skeletons). Reduced motion swaps at once.
 * - O36, clean and minimal: a flat dark card with no glow, no header
 *   divider (spacing carries it), muted text tags, a hairline policy chip.
 * - The role is also written on the section (`data-role`), so the bento's
 *   access tile below opens or closes its lock in CSS, with no more script.
 * - It starts as Admin and, once, flips to Analyst when it scrolls into
 *   view, unless the visitor has already chosen.
 * - Phones: the table scrolls inside its own frame, with a fade at the edge.
 */

type Role = "analyst" | "admin";
type GovernanceCopy = HomeCopy["governance"];
const ROLES: Role[] = ["analyst", "admin"];

export function GovernanceTable({ copy }: { copy: GovernanceCopy }) {
  const tableRef = React.useRef<HTMLDivElement>(null);
  const reduced = useReducedMotionSafe();
  const [role, setRole] = React.useState<Role>("admin");
  const touched = React.useRef(false);
  const inView = useInView(tableRef, { once: true, amount: 0.6 });

  // The one automatic flip, unless the visitor has already chosen for themselves.
  React.useEffect(() => {
    if (!inView) return;
    const id = window.setTimeout(() => {
      if (!touched.current) setRole("analyst");
    }, 700);
    return () => window.clearTimeout(id);
  }, [inView]);

  // The bento's access tile follows the role (governance-section.tsx).
  React.useEffect(() => {
    tableRef.current?.closest("section")?.setAttribute("data-role", role);
  }, [role]);

  const choose = (next: Role) => {
    touched.current = true;
    setRole(next);
  };

  return (
    <div
      ref={tableRef}
      className="overflow-hidden rounded-3xl border border-[#E8F4FF]/10 bg-[#070F3A]"
    >
      {/* O32: no visible "Illustration" label (the owner's decision; the data is plainly fictional and the
          caption for screen readers stays). The policy chip leads; the switch sits at the right. */}
      <div className="flex flex-wrap items-center justify-between gap-3 px-4 pt-4 pb-2 md:px-6 md:pt-5">
        <PolicyChip copy={copy} role={role} />
        <RoleToggle copy={copy} role={role} onChange={choose} />
      </div>
      <p aria-live="polite" className="sr-only">
        {copy.status[role]}
      </p>
      {/* Phones: the table scrolls inside its frame; the right edge fades to say there is more. */}
      <div className="overflow-x-auto max-md:[mask-image:linear-gradient(90deg,#000_82%,transparent)]">
        {/* Fixed column widths: the columns stay put when the role switches and the values change length. */}
        <table className="w-full min-w-[640px] table-fixed text-left">
          <caption className="sr-only">{copy.tableCaption}</caption>
          <thead>
            <tr className="text-[13px] text-[#E8F4FF]/55">
              {GOVERNANCE_COLUMNS.map((c) => (
                <th
                  key={c.key}
                  scope="col"
                  className={cn("px-4 py-3 font-medium md:px-6", COL_WIDTH[c.key], c.key === "amount" && "text-right")}
                >
                  <span className="inline-flex items-center gap-2">
                    {copy.columns[c.key]}
                    {c.masked && <PiiTag copy={copy} />}
                  </span>
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {GOVERNANCE_ROWS.map((r, i) => (
              <tr
                key={r.email}
                className="border-t border-[#E8F4FF]/8 text-[14px] text-[#E8F4FF]/90 transition-colors duration-200 hover:bg-[#E8F4FF]/[0.04]"
              >
                <td className="px-4 py-4 whitespace-nowrap md:px-6">
                  <span className="inline-flex items-center gap-3">
                    <Avatar name={r.name} masked={role === "analyst"} />
                    <Cell value={r.name} masked={maskName(r.name)} role={role} row={i} animate={!reduced} tip={copy.maskedTip} />
                  </span>
                </td>
                <td className="px-4 py-4 whitespace-nowrap md:px-6">
                  <Cell value={r.email} masked={maskEmail(r.email)} role={role} row={i} animate={!reduced} tip={copy.maskedTip} />
                </td>
                <td className="px-4 py-4 whitespace-nowrap md:px-6">
                  <span className="inline-flex items-center gap-1.5">
                    <MapPin aria-hidden="true" className="size-3.5 text-[#44A6FA]/70" />
                    {r.city}
                  </span>
                </td>
                <td className="px-4 py-4 text-right whitespace-nowrap tabular-nums md:px-6">€{r.amount}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function PolicyChip({ copy, role }: { copy: GovernanceCopy; role: Role }) {
  const Icon = role === "analyst" ? Lock : LockOpen;
  return (
    <span
      key={role}
      className={cn(
        "lk-fade-in inline-flex h-8 items-center gap-1.5 rounded-full px-3 text-[12px] font-medium ring-1 ring-[#E8F4FF]/12",
        role === "analyst" ? "text-[#8fcbff]" : "text-[#7be0b0]",
      )}
    >
      <Icon aria-hidden="true" className="size-3.5" />
      {copy.policy[role]}
    </span>
  );
}

function PiiTag({ copy }: { copy: GovernanceCopy }) {
  const id = React.useId();
  return (
    <span className="group/pii relative inline-flex">
      <span
        tabIndex={0}
        aria-describedby={id}
        className="rounded-sm text-[10px] font-semibold tracking-[0.08em] text-[#8fcbff]/70 outline-none focus-visible:ring-2 focus-visible:ring-[#44A6FA]"
      >
        {copy.pii}
      </span>
      <span
        id={id}
        role="tooltip"
        className="pointer-events-none absolute top-full left-0 z-10 mt-2 w-max max-w-[15rem] rounded-lg bg-[#020514]/95 px-2.5 py-1.5 text-[12px] leading-4 font-normal text-[#E8F4FF] opacity-0 shadow-[inset_0_0_0_1px_rgba(232,244,255,0.2)] transition-opacity duration-200 group-hover/pii:opacity-100 group-focus-within/pii:opacity-100"
      >
        {copy.piiTip}
      </span>
    </span>
  );
}

function Avatar({ name, masked }: { name: string; masked: boolean }) {
  const initials = name
    .split(" ")
    .map((w) => w[0])
    .join("");
  return (
    <span
      aria-hidden="true"
      className={cn(
        "grid size-8 shrink-0 place-items-center rounded-full text-[11px] font-semibold transition-colors duration-300 ease-out",
        masked ? "bg-[#E8F4FF]/[0.07] text-[#E8F4FF]/45" : "bg-[#44A6FA]/20 text-[#bfe0ff]",
      )}
    >
      {masked ? <UserRound className="size-4" /> : initials}
    </span>
  );
}

/**
 * One sensitive cell. As Admin it holds the value; switching to Admin, the
 * masked dots decode into letters (420 ms, starting 40 ms after the row
 * above). As Analyst it holds only the masked form, muted, after a small
 * lock whose tooltip says why, and fades in row by row.
 */
function Cell({
  value,
  masked,
  role,
  row,
  animate,
  tip,
}: {
  value: string;
  masked: string;
  role: Role;
  row: number;
  animate: boolean;
  tip: string;
}) {
  const [shown, setShown] = React.useState(value);
  const previous = React.useRef(role);

  // Decode only on the way back to Admin; the frames are set from the
  // animation callback, never in the effect itself.
  React.useEffect(() => {
    const was = previous.current;
    previous.current = role;
    if (role !== "admin" || was === "admin" || !animate) return;
    let raf = 0;
    const t0 = performance.now() + row * 40;
    const step = (now: number) => {
      const p = Math.max(0, Math.min(1, (now - t0) / 420));
      setShown(scrambleFrame(masked, value, p, Math.random));
      if (p < 1) raf = requestAnimationFrame(step);
    };
    raf = requestAnimationFrame(step);
    return () => cancelAnimationFrame(raf);
  }, [role, value, masked, row, animate]);

  if (role === "analyst") {
    return (
      <span
        key="masked"
        className="group/mask lk-fade-in relative inline-flex items-center gap-1.5 text-[#E8F4FF]/55"
        style={{ animationDelay: `${row * 40}ms` }}
      >
        <Lock aria-hidden="true" className="size-3 shrink-0 text-[#8fcbff]/70" />
        <span className="tabular-nums">{masked}</span>
        <span className="sr-only">, {tip}</span>
        <span
          aria-hidden="true"
          className="pointer-events-none absolute bottom-full left-0 z-10 mb-1.5 w-max rounded-lg bg-[#020514]/95 px-2 py-1 text-[11px] text-[#E8F4FF] opacity-0 shadow-[inset_0_0_0_1px_rgba(232,244,255,0.2)] transition-opacity duration-200 group-hover/mask:opacity-100"
        >
          {tip}
        </span>
      </span>
    );
  }
  return (
    <span>
      <span aria-hidden="true">{animate ? shown : value}</span>
      <span className="sr-only">{value}</span>
    </span>
  );
}

function RoleToggle({ copy, role, onChange }: { copy: GovernanceCopy; role: Role; onChange: (r: Role) => void }) {
  const refs = React.useRef<(HTMLButtonElement | null)[]>([]);
  const onKey = (e: React.KeyboardEvent) => {
    if (!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(e.key)) return;
    e.preventDefault();
    const next = ROLES[(ROLES.indexOf(role) + 1) % ROLES.length];
    onChange(next);
    refs.current[ROLES.indexOf(next)]?.focus();
  };
  return (
    <div className="flex items-center gap-2.5">
      <span id="lk-view-as" className="text-[13px] text-[#E8F4FF]/55">
        {copy.viewAs}
      </span>
      <div
        role="radiogroup"
        aria-labelledby="lk-view-as"
        onKeyDown={onKey}
        className="relative grid grid-cols-2 rounded-full bg-[#E8F4FF]/[0.06] p-0.5 ring-1 ring-[#E8F4FF]/12"
      >
        {/* The thumb slides under the chosen role. */}
        <span
          aria-hidden="true"
          className="absolute inset-y-0.5 left-0.5 w-[calc(50%-2px)] rounded-full bg-[var(--lk-primary)] shadow-[0_6px_20px_-6px_rgba(68,166,250,0.7)] transition-transform duration-[220ms] ease-[cubic-bezier(0.16,1,0.3,1)] motion-reduce:transition-none"
          style={{ transform: role === "admin" ? "translateX(100%)" : "translateX(0)" }}
        />
        {ROLES.map((r, i) => (
          <button
            key={r}
            ref={(el) => {
              refs.current[i] = el;
            }}
            type="button"
            role="radio"
            aria-checked={role === r}
            tabIndex={role === r ? 0 : -1}
            onClick={() => onChange(r)}
            className={cn(
              "relative z-10 rounded-full px-4 py-1.5 text-[13px] font-medium transition-colors duration-200 ease-out active:scale-95 motion-reduce:transition-none",
              role === r ? "text-white" : "text-[#E8F4FF]/70 hover:text-[#E8F4FF]",
            )}
          >
            {copy.roles[r]}
          </button>
        ))}
      </div>
    </div>
  );
}
