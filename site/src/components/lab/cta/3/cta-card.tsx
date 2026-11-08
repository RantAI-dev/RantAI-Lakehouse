import { CtaLink, DisplayHeading } from "@/components/shared/primitives";
import { CONTACT_HREF, CARD, INSTALL_HREF } from "../lab-shell";
import { Shot } from "./shot";

/**
 * Lab variant 3, "The console surfaces": product first. A short headline, one line and the two
 * buttons on the left; on the right the real console (a cropped screenshot of the Dashboards page,
 * public/products/lake-dashboard.webp, illustrative data) rises from the card's bottom edge.
 */
export function CtaCard() {
  return (
    <section aria-labelledby="cta3-heading" className={`${CARD} overflow-hidden`}>
      <div className="mx-auto grid max-w-6xl md:grid-cols-[minmax(0,5fr)_minmax(0,6fr)]">
        <div className="flex flex-col justify-center p-6 sm:p-8 md:p-12 md:pr-4">
          <DisplayHeading id="cta3-heading" as="h2" size="md" className="max-w-[14ch]">
            See it run on your servers.
          </DisplayHeading>
          <p className="mt-4 max-w-[34ch] text-[15px] leading-6 text-[#E8F4FF]/70">Free, open source, and installed with Docker.</p>
          <div className="mt-8 flex flex-col gap-3 sm:flex-row md:mb-12">
            <CtaLink href={INSTALL_HREF}>Install it</CtaLink>
            <CtaLink href={CONTACT_HREF} variant="ghost">
              Talk to us
            </CtaLink>
          </div>
        </div>
        <Shot />
      </div>
    </section>
  );
}
