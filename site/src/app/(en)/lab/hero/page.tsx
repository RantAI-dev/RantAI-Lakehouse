import type { Metadata } from "next";
import { getMessages, setRequestLocale } from "next-intl/server";
import { TopBar } from "@/components/layout/top-bar";
import { labFontVariables } from "@/components/lab/hero/fonts";
import { LabHero } from "@/components/lab/hero/lab-hero";
import { localizedPath } from "@/i18n/paths";
import { routing } from "@/i18n/routing";
import type { CommonCopy, HomeCopy } from "@/lib/copy";

/**
 * A playground for the home hero: the real copy, components and 3D scene,
 * with a control bar for the headline font, the layout and night or day. It
 * is linked from nowhere, kept out of the sitemap and out of search, and all
 * of its code is in `components/lab/hero/`, so removing it is deleting that
 * folder, `src/fonts/lab/` and this route.
 */
export const metadata: Metadata = {
  title: "Hero lab",
  robots: { index: false, follow: false },
};

export default async function Page() {
  setRequestLocale(routing.defaultLocale);
  const messages = await getMessages({ locale: routing.defaultLocale });
  const home = messages.home as HomeCopy;
  const common = messages.common as CommonCopy;
  return (
    <LabHero
      fontVariables={labFontVariables}
      copy={home.hero}
      cta={common.cta}
      contactHref={localizedPath(routing.defaultLocale, "/contact")}
      topBar={
        <TopBar locale={routing.defaultLocale} page="contact" nav={common.nav} cta={common.cta} language={common.language} />
      }
    />
  );
}
