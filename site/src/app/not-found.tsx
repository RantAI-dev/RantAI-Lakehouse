import { getTranslations, setRequestLocale } from "next-intl/server";
import { SiteDocument } from "@/components/layout/site-document";
import { routing } from "@/i18n/routing";

/**
 * `out/404.html`. A static host serves one file for every missing path, in
 * either language, so it is the English page with a link home.
 */
export default async function NotFound() {
  setRequestLocale(routing.defaultLocale);
  const t = await getTranslations({ locale: routing.defaultLocale, namespace: "common.notFound" });
  return (
    <SiteDocument locale={routing.defaultLocale}>
      <main>
        <h1>{t("title")}</h1>
        <p>{t("body")}</p>
        {/* eslint-disable-next-line @next/next/no-html-link-for-pages -- the 404 file is served for every path; a plain link is a full load of the home page */}
        <a href="/">{t("home")}</a>
      </main>
    </SiteDocument>
  );
}
