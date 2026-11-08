import { CtaLink, DisplayHeading } from "@/components/shared/primitives";
import { LINKS } from "@/lib/home-content";
import type { CommonCopy } from "@/lib/copy";
import { CopyButton } from "./copy-button";

/**
 * The footer's call to action (owner QA 2026-11-08, after comparing three variants): no card, no
 * whale, no source picker. A short serif headline, the one command that starts the install (a
 * terminal, so it keeps its solid code block, with Copy and an edge fade where it scrolls on a
 * phone), what comes next, and the two buttons, straight on the page and on the footer's own
 * left edge. On the contact page the visitor is already there, so only the install button stays.
 */
const CLONE = "git clone https://github.com/RantAI-dev/RantAI-Lakehouse.git";

export function FooterCta({
  copy,
  installHref,
  contactHref,
  page,
}: {
  copy: CommonCopy["footer"]["cta"];
  installHref: string;
  contactHref: string;
  page: "home" | "contact";
}) {
  return (
    <section aria-labelledby="footer-cta-heading" className="px-6 pt-14 sm:px-10 md:pt-24 lg:px-16">
      <DisplayHeading id="footer-cta-heading" as="h2" size="lg" className="max-w-[14ch]">
        {copy.title}
      </DisplayHeading>
      <div className="lk-term mt-8 flex max-w-3xl items-center gap-2 rounded-2xl bg-[var(--lk-n-surface)] p-2 pl-4 md:mt-10">
        <span aria-hidden="true" className="text-[#8fcbff]/60 select-none">$</span>
        <div
          className="min-w-0 flex-1 overflow-x-auto [mask-image:linear-gradient(90deg,#000_calc(100%-36px),transparent)] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
          tabIndex={0}
          aria-label={copy.commandLabel}
        >
          <code className="block py-3 pr-9 whitespace-nowrap text-[14px] leading-5 text-[#E8F4FF] select-all sm:text-[15px]">{CLONE}</code>
        </div>
        <CopyButton text={CLONE} label={copy.copy} copiedLabel={copy.copied} failedLabel={copy.copyFailed} ariaLabel={copy.copyCommand} />
      </div>
      <p className="mt-4 max-w-3xl text-[14px] leading-6 text-[#E8F4FF]/60">
        {copy.then}{" "}
        <code className="rounded bg-white/[0.07] px-1.5 py-0.5 !text-[12.5px]">sh ops/init-env.sh</code> {copy.and}{" "}
        <code className="rounded bg-white/[0.07] px-1.5 py-0.5 !text-[12.5px]">scripts/compose.sh up --build</code>
        <span aria-hidden="true"> · </span>
        <a
          href={LINKS.guide}
          target="_blank"
          rel="noopener noreferrer"
          className="rounded text-[#8fcbff] underline decoration-[#8fcbff]/30 underline-offset-4 transition-[color,text-decoration-color] duration-200 ease-out outline-none hover:text-white hover:decoration-white/60 focus-visible:ring-2 focus-visible:ring-[#44A6FA]"
        >
          {copy.guide} →
        </a>
      </p>
      <div className="mt-8 flex flex-col gap-3 sm:flex-row">
        <CtaLink href={installHref}>{copy.install}</CtaLink>
        {page === "home" && (
          <CtaLink href={contactHref} variant="ghost">
            {copy.talk}
          </CtaLink>
        )}
      </div>
    </section>
  );
}
