import { notFound } from "next/navigation";
import { hasLocale } from "next-intl";
import { setRequestLocale } from "next-intl/server";
import { SiteDocument } from "@/components/layout/site-document";
import { prefixedLocales } from "@/i18n/routing";

/**
 * Every language but English, under its own prefix: today `/id/` (WEB-5
 * D3). English is not a value here (it lives in `app/(en)`), and no other
 * value is written, so `/en/` does not exist in the export.
 */
export const dynamicParams = false;

export function generateStaticParams() {
  return prefixedLocales.map((locale) => ({ locale }));
}

export default async function LocaleLayout({
  children,
  params,
}: Readonly<{ children: React.ReactNode; params: Promise<{ locale: string }> }>) {
  const { locale } = await params;
  if (!hasLocale(prefixedLocales, locale)) notFound();
  setRequestLocale(locale);
  return <SiteDocument locale={locale}>{children}</SiteDocument>;
}
