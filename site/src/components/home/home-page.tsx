import { getMessages } from "next-intl/server";
import { LandingModeProvider } from "@/components/effects/scene-mode";
import { SiteFooter } from "@/components/layout/site-footer";
import { SkipLink } from "@/components/layout/skip-link";
import { TopBar } from "@/components/layout/top-bar";
import { localizedPath } from "@/i18n/paths";
import type { Locale } from "@/i18n/routing";
import { JsonLd } from "@/components/shared/json-ld";
import { HOME_FAQ } from "@/lib/home-content";
import type { CommonCopy, ContactCopy, HomeCopy } from "@/lib/copy";
import { siteUrl } from "@/lib/site-url";
import { homeStructuredData } from "@/lib/structured-data";
import { Faq } from "@/components/contact/faq";
import { AssistantSection } from "./assistant-section";
import { DashboardsSection } from "./dashboards-section";
import { DepthBackdrop } from "./depth-backdrop";
import { DepthGauge } from "./depth-gauge";
import { DepthsSection } from "./depths-section";
import { FinalSection } from "./final-section";
import { GetStartedSection } from "./get-started-section";
import { GovernanceSection } from "./governance-section";
import { HeroIntro } from "./hero-intro";
import { HeroSection } from "./hero-section";
import { OpenSection } from "./open-section";
import { SceneZone } from "./scene-zone";
import { SourcesSection } from "./sources-section";

/**
 * The home page (WEB-1's landing page, moved by WEB-5). It is shared by its
 * two routes, `app/(en)/page.tsx` and `app/[locale]/page.tsx`, so it lives
 * in `components/home/` rather than in a route's `_components/` (D9).
 *
 * It is static and independent of any API. The text is read here, on the
 * server, from the locale's messages and handed to each section as props,
 * so no section reads a message on the client (D5).
 */
export async function HomePage({ locale }: Readonly<{ locale: Locale }>) {
  const messages = await getMessages({ locale });
  const home = messages.home as HomeCopy;
  const common = messages.common as CommonCopy;
  const meta = messages.metadata as { home: { description: string } };
  const contactHref = localizedPath(locale, "/contact");
  const contact = messages.contact as ContactCopy;

  return (
    <LandingModeProvider>
      <div className="lk-root relative min-h-screen overflow-x-clip">
        {/* WEB-5 O47: first, so its class is on <html> before anything paints. */}
        <HeroIntro />
        <SkipLink label={common.skipToContent} />
        {/* WEB-5 D14 S2: the water the second half of the page sits in. First, so every section paints over it. */}
        <DepthBackdrop />
        {/* WEB-5 D14 S3: the depth, the whole way down and back up. */}
        <DepthGauge surfaceLabel={common.gauge.surface} />
        <TopBar locale={locale} nav={common.nav} cta={common.cta} language={common.language} />
        <main id="main" tabIndex={-1}>
          <SceneZone>
            <HeroSection copy={home.hero} cta={common.cta} contactHref={contactHref} />
            <SourcesSection copy={home.sources} />
            <DepthsSection copy={home.depths} />
          </SceneZone>
          <AssistantSection copy={home.assistant} />
          <DashboardsSection copy={home.dashboards} />
          <GovernanceSection copy={home.governance} />
          <OpenSection copy={home.open} cta={common.cta} />
          <GetStartedSection copy={home.getStarted} />
          {/* WEB-5 D21: free and open source, your servers, sources, installing; all of them on /contact/. */}
          <Faq copy={contact.faq} pick={HOME_FAQ} more={{ label: home.faq.all, href: `${contactHref}#faq` }} />
          {/* Task A: the neutral page feathers into the final call's night sea over 480 px, no hard edge. */}
          <div
            aria-hidden="true"
            className="pointer-events-none relative z-[1] -mt-[480px] h-[480px] bg-[linear-gradient(180deg,rgba(2,5,20,0)_0%,rgba(2,5,20,0.12)_30%,rgba(2,5,20,0.55)_68%,#020514_100%)]"
          />
          <FinalSection copy={home.final} />
        </main>
        <SiteFooter locale={locale} copy={common.footer} />
        <JsonLd data={homeStructuredData({ locale, siteUrl: siteUrl(), description: meta.home.description })} />
      </div>
    </LandingModeProvider>
  );
}
