"use client";

import * as React from "react";
import { useInView } from "framer-motion";
import { Lock } from "lucide-react";
import { cn } from "@/lib/utils";
import type { HomeCopy } from "@/lib/copy";
import { GOVERNANCE_COLUMNS, GOVERNANCE_ROWS } from "@/lib/home-content";
import { maskEmail, maskName, scrambleFrame } from "@/lib/scramble";
import { useReducedMotionSafe } from "@/hooks/use-reduced-motion";

/**
 * The governance section's illustration: the masked table, its "View as"
 * toggle, the one automatic flip to Analyst when it scrolls into view, and
 * the scrambling cells. The client leaf of `governance-section.tsx` (WEB-5
 * review fix R5); the headline, lead and facts render on the server there.
 */

type Role = "analyst" | "admin";
type GovernanceCopy = HomeCopy["governance"];

export function GovernanceTable({ copy }: { copy: GovernanceCopy }) {
  const tableRef = React.useRef<HTMLDivElement>(null);
  const reduced = useReducedMotionSafe();
  const [role, setRole] = React.useState<Role>("admin");
  const touched = React.useRef(false);
  const inView = useInView(tableRef, { once: true, amount: 0.7 });

  // The one automatic flip, unless the visitor has already chosen for themselves.
  React.useEffect(() => {
    if (!inView) return;
    const id = window.setTimeout(() => {
      if (!touched.current) setRole("analyst");
    }, 700);
    return () => window.clearTimeout(id);
  }, [inView]);

  const choose = (next: Role) => {
    touched.current = true;
    setRole(next);
  };

  return (
    <div
      ref={tableRef}
      className="overflow-hidden rounded-3xl border border-[#E8F4FF]/12 bg-[#020514]/55 shadow-[0_40px_120px_-50px_rgba(68,166,250,0.45)] backdrop-blur-md"
    >
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-[#E8F4FF]/10 px-4 py-3 md:px-6">
        <p className="text-[11px] tracking-[0.2em] text-[#E8F4FF]/60 uppercase">
          {copy.illustration}
        </p>
        <RoleToggle copy={copy} role={role} onChange={choose} />
      </div>
      <div className="overflow-x-auto">
        <table className="w-full min-w-[620px] text-left">
          <caption className="sr-only">{copy.tableCaption}</caption>
          <thead>
            <tr className="text-[11px] tracking-[0.16em] text-[#E8F4FF]/60 uppercase">
              {GOVERNANCE_COLUMNS.map((c) => (
                <th key={c.key} scope="col" className="px-4 py-3 font-medium md:px-6">
                  <span className="inline-flex items-center gap-1.5">
                    {copy.columns[c.key]}
                    {role === "analyst" && c.masked && (
                      <Lock aria-hidden="true" className="size-3 text-[#44A6FA]" />
                    )}
                  </span>
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {GOVERNANCE_ROWS.map((r) => (
              <tr
                key={r.email}
                className="border-t border-[#E8F4FF]/8 text-[14px] transition-colors hover:bg-[#E8F4FF]/[0.04]"
              >
                <td className="px-4 py-3.5 whitespace-nowrap md:px-6">
                  <Masked value={r.name} masked={maskName(r.name)} role={role} animate={!reduced} />
                </td>
                <td className="px-4 py-3.5 whitespace-nowrap md:px-6">
                  <Masked value={r.email} masked={maskEmail(r.email)} role={role} animate={!reduced} />
                </td>
                <td className="px-4 py-3.5 whitespace-nowrap md:px-6">{r.city}</td>
                <td className="px-4 py-3.5 whitespace-nowrap tabular-nums md:px-6">{r.amount}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function RoleToggle({ copy, role, onChange }: { copy: GovernanceCopy; role: Role; onChange: (r: Role) => void }) {
  const options: { id: Role; label: string }[] = [
    { id: "analyst", label: copy.roles.analyst },
    { id: "admin", label: copy.roles.admin },
  ];
  return (
    <div className="flex items-center gap-3">
      <span id="lk-view-as" className="text-[11px] tracking-[0.2em] text-[#E8F4FF]/60 uppercase">
        {copy.viewAs}
      </span>
      <div
        role="radiogroup"
        aria-labelledby="lk-view-as"
        className="relative flex rounded-full border border-[#E8F4FF]/15 bg-[#E8F4FF]/[0.05] p-0.5"
      >
        {options.map((o) => (
          <button
            key={o.id}
            type="button"
            role="radio"
            aria-checked={role === o.id}
            onClick={() => onChange(o.id)}
            className={cn(
              "relative rounded-full px-4 py-1.5 text-[13px] font-medium transition-colors duration-300",
              role === o.id ? "bg-[#44A6FA] text-[#020514]" : "text-[#E8F4FF]/75 hover:text-[#E8F4FF]",
            )}
          >
            {o.label}
          </button>
        ))}
      </div>
    </div>
  );
}

/**
 * One cell that cycles random glyphs for 400 ms when its role changes, then
 * rests on the masked or the full value. Screen readers get the final value
 * only; the in-between frames are `aria-hidden`.
 */
function Masked({
  value,
  masked,
  role,
  animate,
}: {
  value: string;
  masked: string;
  role: Role;
  animate: boolean;
}) {
  const target = role === "admin" ? value : masked;
  const [shown, setShown] = React.useState(target);
  const previous = React.useRef(target);

  React.useEffect(() => {
    if (!animate || previous.current === target) {
      previous.current = target;
      return;
    }
    const from = previous.current;
    previous.current = target;
    const t0 = performance.now();
    let raf = 0;
    const step = (now: number) => {
      const p = Math.min(1, (now - t0) / 400);
      setShown(scrambleFrame(from, target, p, Math.random));
      if (p < 1) raf = requestAnimationFrame(step);
    };
    raf = requestAnimationFrame(step);
    return () => cancelAnimationFrame(raf);
  }, [target, animate]);

  // Without animation the cell simply shows the target; with it, the frame.
  const text = animate ? shown : target;
  return (
    <span className={cn("text-[13px]", role === "analyst" && "text-[#E8F4FF]/90")}>
      <span aria-hidden="true">{text}</span>
      <span className="sr-only">{target}</span>
    </span>
  );
}
