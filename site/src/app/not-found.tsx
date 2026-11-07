import type { Metadata } from "next";
import { getTranslations, setRequestLocale } from "next-intl/server";
import { ArrowLeft } from "lucide-react";
import { SiteDocument } from "@/components/layout/site-document";
import { MascotLogo } from "@/components/shared/mascot-svg";
import { CtaLink, DisplayHeading, Eyebrow, LeadText } from "@/components/shared/primitives";
import { localizedPath } from "@/i18n/paths";
import { routing } from "@/i18n/routing";

/**
 * `out/404.html`, served by a static host for every missing path. WEB-5
 * D18: a 404 in each language. One file answers both `/x/` and `/id/x/`
 * on every host (some hosts only ever serve the root one), so it says it
 * in English first and then in Indonesian (`lang="id"`), each with a way
 * home in its own language. The creature waits at the surface; it bobs
 * like the hero's and rests under reduced motion. Next marks the page
 * `noindex`, which is right for a missing page.
 */
export async function generateMetadata(): Promise<Metadata> {
  const [en, id] = await Promise.all(
    routing.locales.map((locale) => getTranslations({ locale, namespace: "common.notFound" })),
  );
  return { title: `${en("meta")} · ${id("meta")}` };
}

export default async function NotFound() {
  setRequestLocale(routing.defaultLocale);
  const en = await getTranslations({ locale: "en", namespace: "common.notFound" });
  const id = await getTranslations({ locale: "id", namespace: "common.notFound" });
  return (
    <SiteDocument locale={routing.defaultLocale}>
      <div className="lk-root lk-404 relative isolate flex min-h-[100svh] flex-col items-center justify-center overflow-hidden px-4 py-20 text-center">
        <div
          aria-hidden="true"
          className="absolute inset-0 -z-10 bg-[linear-gradient(180deg,#020514_0%,#050A30_42%,#0A1F55_70%,#061440_100%)]"
        />
        <main className="flex max-w-[36rem] flex-col items-center">
          <MascotLogo className="lk-bob h-24 w-auto drop-shadow-[0_0_40px_rgba(68,166,250,0.3)] md:h-28" />
          <Eyebrow className="mt-8">404</Eyebrow>
          <DisplayHeading as="h1" size="md" className="mt-4">
            {en("title")}
          </DisplayHeading>
          <LeadText className="mt-4">{en("body")}</LeadText>
          {/* A plain link (CtaLink is an <a>): the 404 file is served for every path, so going home is a full load. */}
          <CtaLink href={localizedPath("en", "/")} className="mt-8">
            <ArrowLeft className="size-4 transition-transform duration-200 ease-out group-hover:-translate-x-0.5" aria-hidden="true" />
            {en("home")}
          </CtaLink>
          <div lang="id" className="mt-12 flex flex-col items-center border-t border-[#E8F4FF]/12 pt-8">
            <p className="text-[19px] leading-7 font-medium text-[#E8F4FF]">{id("title")}</p>
            <p className="mt-2 text-[15px] leading-6 text-[#E8F4FF]/70">{id("body")}</p>
            <a
              href={localizedPath("id", "/")}
              hrefLang="id"
              className="mt-4 inline-flex min-h-10 items-center gap-1.5 rounded-full px-3 text-[15px] font-medium text-[#8fcbff] transition-colors duration-200 ease-out hover:text-[#E8F4FF]"
            >
              <ArrowLeft className="size-4" aria-hidden="true" />
              {id("home")}
            </a>
          </div>
        </main>
      </div>
    </SiteDocument>
  );
}
