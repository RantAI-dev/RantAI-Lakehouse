import { setRequestLocale } from "next-intl/server";
import { SiteDocument } from "@/components/layout/site-document";
import { routing } from "@/i18n/routing";

/**
 * English, at the root (WEB-5 D3). A route group rather than a `[locale]`
 * value, so the export writes `out/index.html` and never an `out/en/`.
 */
export default function EnglishLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  setRequestLocale(routing.defaultLocale);
  return <SiteDocument locale={routing.defaultLocale}>{children}</SiteDocument>;
}
