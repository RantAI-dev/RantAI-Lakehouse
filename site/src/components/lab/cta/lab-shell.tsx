import { getMessages } from "next-intl/server";
import { LandingModeProvider } from "@/components/effects/scene-mode";
import { FinalSection } from "@/components/home/final-section";
import { SiteFooter } from "@/components/layout/site-footer";
import { localizedPath } from "@/i18n/paths";
import { routing } from "@/i18n/routing";
import type { CommonCopy, HomeCopy } from "@/lib/copy";

/**
 * Lab only: a temporary page for comparing footer call-to-action cards (owner QA 2026-11-08).
 * It shows the home page's last screen of sea (the final section, from the deep water's colour
 * into the night) and then the real footer, with a variant's card in the footer's card slot.
 * Not linked from the site, not in the sitemap, `noindex`; delete `components/lab` and
 * `app/(en)/lab` when the owner has chosen.
 */
export const LAB_META = {
  robots: { index: false, follow: false },
} as const;

export const INSTALL_HREF = "/#get-started";
export const CONTACT_HREF = localizedPath(routing.defaultLocale, "/contact");

/** The card's surface: the deep-water blue-grey, one thin hairline, nothing else. */
export const CARD = "relative isolate overflow-visible rounded-[28px] bg-[var(--lk-n-surface)] ring-1 ring-[var(--lk-n-hair)]";

export async function LabShell({ card }: { card: React.ReactNode }) {
  const locale = routing.defaultLocale;
  const messages = await getMessages({ locale });
  const home = messages.home as HomeCopy;
  const common = messages.common as CommonCopy;
  return (
    <LandingModeProvider>
      <div className="lk-root relative min-h-screen overflow-x-clip bg-[#04101c]">
        <main id="main">
          {/* The deep water the home page's FAQ ends on, then the same 480 px feather into the night sea. */}
          <div aria-hidden="true" className="h-[36svh] bg-[#04101c]" />
          <div
            aria-hidden="true"
            className="pointer-events-none relative z-[1] -mt-[480px] h-[480px] bg-[linear-gradient(180deg,rgba(2,5,20,0)_0%,rgba(2,5,20,0.12)_30%,rgba(2,5,20,0.55)_68%,#020514_100%)]"
          />
          <FinalSection copy={home.final} />
        </main>
        <SiteFooter locale={locale} copy={common.footer} cta={card} />
      </div>
    </LandingModeProvider>
  );
}
