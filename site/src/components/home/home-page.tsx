import { getMessages } from "next-intl/server";
import { LandingModeProvider } from "@/components/effects/scene-mode";
import { SiteFooter } from "@/components/layout/site-footer";
import { TopBar } from "@/components/layout/top-bar";
import type { Locale } from "@/i18n/routing";
import { JsonLd } from "@/components/shared/json-ld";
import type { CommonCopy, HomeCopy } from "@/lib/copy";
import { siteUrl } from "@/lib/site-url";
import { homeStructuredData } from "@/lib/structured-data";
import { AssistantSection } from "./assistant-section";
import { DashboardsSection } from "./dashboards-section";
import { DepthBackdrop } from "./depth-backdrop";
import { DepthGauge } from "./depth-gauge";
import { DepthsSection } from "./depths-section";
import { FinalSection } from "./final-section";
import { GovernanceSection } from "./governance-section";
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

  return (
    <LandingModeProvider>
      <div className="lk-root relative min-h-screen overflow-x-clip">
        <a
          href="#main"
          className="sr-only z-[60] rounded-full bg-[#E8F4FF] px-4 py-2 text-[14px] font-medium text-[#020514] focus:not-sr-only focus:fixed focus:top-3 focus:left-3"
        >
          {common.skipToContent}
        </a>
        {/* WEB-5 D14 S2: the water the second half of the page sits in. First, so every section paints over it. */}
        <DepthBackdrop />
        {/* WEB-5 D14 S3: the depth, the whole way down and back up. */}
        <DepthGauge surfaceLabel={common.gauge.surface} />
        <TopBar locale={locale} nav={common.nav} cta={common.cta} language={common.language} />
        <main id="main" tabIndex={-1}>
          <SceneZone>
            <HeroSection copy={home.hero} cta={common.cta} />
            <SourcesSection copy={home.sources} />
            <DepthsSection copy={home.depths} />
          </SceneZone>
          <AssistantSection copy={home.assistant} />
          <DashboardsSection copy={home.dashboards} />
          <GovernanceSection copy={home.governance} />
          <OpenSection copy={home.open} cta={common.cta} />
          <FinalSection copy={home.final} cta={common.cta} />
        </main>
        <SiteFooter copy={common.footer} />
        <JsonLd data={homeStructuredData({ locale, siteUrl: siteUrl(), description: meta.home.description })} />
      </div>
    </LandingModeProvider>
  );
}
