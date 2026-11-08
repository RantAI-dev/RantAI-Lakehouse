import { WhaleSvg } from "@/components/shared/whale-svg";
import { CtaLink, DisplayHeading } from "@/components/shared/primitives";
import { LINKS } from "@/lib/home-content";
import { CONTACT_HREF, CARD, INSTALL_HREF } from "../lab-shell";
import { CopyButton } from "../copy-button";

/**
 * Lab variant 2, "One command": a short serif headline, one wide command line you can copy or
 * select, a small line for what comes next, and the two buttons. The navy whale's head
 * (shared/whale-svg.tsx, two plain white eyes, no glow, no spout) rests on the card's top edge
 * at the right and blinks now and then. Nothing else moves.
 */
const CLONE = "git clone https://github.com/RantAI-dev/RantAI-Lakehouse.git";

export function CtaCard() {
  return (
    <section aria-labelledby="cta2-heading" className={CARD}>
      <WhaleSvg
        surface
        spout={false}
        idPrefix="lab2"
        viewBox="110 74 380 246"
        className="pointer-events-none absolute right-6 bottom-full z-[1] w-[112px] translate-y-px md:right-16 md:w-[156px]"
      />
      <div className="mx-auto max-w-6xl px-6 py-12 sm:px-10 md:px-16 md:py-20">
        <DisplayHeading id="cta2-heading" as="h2" size="lg" className="max-w-[14ch]">
          Move your data in.
        </DisplayHeading>
        <div className="mt-8 flex max-w-3xl items-center gap-2 rounded-2xl bg-[var(--lk-n-bg)] p-2 pl-4 md:mt-10">
          <span aria-hidden="true" className="text-[#8fcbff]/60 select-none">$</span>
          <div className="min-w-0 flex-1 overflow-x-auto [mask-image:linear-gradient(90deg,#000_calc(100%-36px),transparent)] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden" tabIndex={0} aria-label="Clone command">
            <code className="block py-3 pr-9 whitespace-nowrap text-[14px] leading-5 text-[#E8F4FF] select-all sm:text-[15px]">{CLONE}</code>
          </div>
          <CopyButton text={CLONE} ariaLabel="Copy the clone command" />
        </div>
        <p className="mt-4 max-w-3xl text-[14px] leading-6 text-[#E8F4FF]/60">
          Then <code className="rounded bg-white/[0.07] px-1.5 py-0.5 !text-[12.5px]">sh ops/init-env.sh</code> and{" "}
          <code className="rounded bg-white/[0.07] px-1.5 py-0.5 !text-[12.5px]">scripts/compose.sh up --build</code>
          <span aria-hidden="true"> · </span>
          <a
            href={LINKS.guide}
            target="_blank"
            rel="noopener noreferrer"
            className="rounded text-[#8fcbff] underline decoration-[#8fcbff]/30 underline-offset-4 transition-[color,text-decoration-color] duration-200 ease-out outline-none hover:text-white hover:decoration-white/60 focus-visible:ring-2 focus-visible:ring-[#44A6FA]"
          >
            Full guide →
          </a>
        </p>
        <div className="mt-8 flex flex-col gap-3 sm:flex-row">
          <CtaLink href={INSTALL_HREF}>Install it</CtaLink>
          <CtaLink href={CONTACT_HREF} variant="ghost">
            Talk to us
          </CtaLink>
        </div>
      </div>
    </section>
  );
}
