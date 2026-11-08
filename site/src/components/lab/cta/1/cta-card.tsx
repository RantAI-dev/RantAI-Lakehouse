import { CtaLink, DisplayHeading } from "@/components/shared/primitives";
import { CONTACT_EMAIL } from "@/lib/home-content";
import { CONTACT_HREF, CARD, INSTALL_HREF } from "../lab-shell";
import { CopyButton } from "../copy-button";

/**
 * Lab variant 1, "Two doors": the card splits in two equal halves for the two audiences, with
 * one hairline between them (a divider, not two boxes). Left: run it yourself, with the real
 * quickstart as a compact terminal and one Copy for all three lines. Right: talk to us, with the
 * real contact address and its own Copy. The buttons sit on one line across the halves; on a
 * phone the halves stack.
 */
const QUICKSTART = [
  "git clone https://github.com/RantAI-dev/RantAI-Lakehouse.git && cd RantAI-Lakehouse",
  "sh ops/init-env.sh",
  "scripts/compose.sh up --build",
] as const;

export function CtaCard() {
  return (
    <section aria-label="Install it or talk to us" className={CARD}>
      <div className="mx-auto grid max-w-6xl divide-y divide-[var(--lk-n-hair)] md:grid-cols-2 md:divide-x md:divide-y-0">
        <div className="flex min-w-0 flex-col p-6 sm:p-8 md:p-10">
          <DisplayHeading as="h2" size="sm" className="max-w-[16ch]">
            Run it yourself.
          </DisplayHeading>
          <p className="mt-3 max-w-[40ch] text-[15px] leading-6 text-[#E8F4FF]/70">
            Free and open source (AGPL-3.0). Runs on your servers; you need Docker.
          </p>
          <div className="mt-6 rounded-2xl bg-[var(--lk-n-bg)] p-3 pl-4">
            <div className="flex items-center justify-between gap-3">
              <span className="text-[12px] font-medium tracking-[0.08em] text-[#E8F4FF]/50 uppercase">Quickstart</span>
              <CopyButton text={QUICKSTART.join("\n")} label="Copy" ariaLabel="Copy the three quickstart commands" />
            </div>
            <pre className="mt-2 overflow-x-auto pb-1 text-[13px] leading-6 text-[#E8F4FF]/90 [mask-image:linear-gradient(90deg,#000_calc(100%-32px),transparent)] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden" tabIndex={0} aria-label="Quickstart commands">
              {QUICKSTART.map((line) => (
                <code key={line} className="block pr-8 whitespace-pre !text-[13px] !leading-6">
                  <span aria-hidden="true" className="mr-2 text-[#8fcbff]/60 select-none">$</span>
                  {line}
                </code>
              ))}
            </pre>
          </div>
          <div className="mt-auto pt-6">
            <CtaLink href={INSTALL_HREF} className="w-full sm:w-auto">
              Install it
            </CtaLink>
          </div>
        </div>

        <div className="flex min-w-0 flex-col p-6 sm:p-8 md:p-10">
          <DisplayHeading as="h2" size="sm" className="max-w-[16ch]">
            Talk to us.
          </DisplayHeading>
          <p className="mt-3 max-w-[40ch] text-[15px] leading-6 text-[#E8F4FF]/70">
            Want a hand setting it up on your servers? Write to the team.
          </p>
          <div className="mt-6 flex items-center justify-between gap-3 rounded-2xl bg-[var(--lk-n-bg)] p-3 pl-4">
            <span className="min-w-0 truncate text-[15px] text-[#E8F4FF] select-all">{CONTACT_EMAIL}</span>
            <CopyButton text={CONTACT_EMAIL} ariaLabel={`Copy ${CONTACT_EMAIL}`} />
          </div>
          <div className="mt-auto pt-6">
            <CtaLink href={CONTACT_HREF} variant="ghost" className="w-full sm:w-auto">
              Talk to us
            </CtaLink>
          </div>
        </div>
      </div>
    </section>
  );
}
