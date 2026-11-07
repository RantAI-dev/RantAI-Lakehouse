import { setRequestLocale } from "next-intl/server";
import { LabShell, labMetadata } from "@/components/lab/lab-shell";
import { routing } from "@/i18n/routing";

export const metadata = labMetadata("Hero explorations");

const LABS = [
  { href: "/lab/1/", name: "Waterline", line: "A cinematic split: the sky above a living waterline, the product sinking through Bronze, Silver and Gold below." },
  { href: "/lab/2/", name: "Lab 2", line: "The third developer's exploration." },
  { href: "/lab/3/", name: "Console on the lake", line: "Product first: a floating assistant on a calm night lake; ask, and the chart rises out of the water." },
] as const;

/** The index of the hero explorations (temporary; see `components/lab/lab-shell.tsx`). */
export default function Page() {
  setRequestLocale(routing.defaultLocale);
  return (
    <LabShell>
      {() => (
        <section className="mx-auto max-w-5xl px-4 pt-32 pb-24 sm:px-6">
          <p className="text-[12px] font-medium tracking-[0.22em] text-[#44A6FA] uppercase">Lab · not public</p>
          <h1 className="mt-4 lk-display text-[clamp(2.5rem,5vw,4rem)] leading-[1.04] text-[#E8F4FF]">Three heroes to compare.</h1>
          <ul className="mt-10 grid gap-4 md:grid-cols-3">
            {LABS.map((l, i) => (
              <li key={l.href}>
                <a
                  href={l.href}
                  className="flex h-full flex-col rounded-3xl bg-[#070F3A] p-6 ring-1 ring-[#E8F4FF]/10 transition-[box-shadow,transform] duration-200 ease-out hover:-translate-y-0.5 hover:ring-[#8fcbff]/40 motion-reduce:transition-none"
                >
                  <span className="text-[12px] text-[#E8F4FF]/55">Lab {i + 1}</span>
                  <span className="mt-2 text-[20px] font-medium text-[#E8F4FF]">{l.name}</span>
                  <span className="mt-2 text-[14px] leading-6 text-[#E8F4FF]/70">{l.line}</span>
                </a>
              </li>
            ))}
          </ul>
        </section>
      )}
    </LabShell>
  );
}
