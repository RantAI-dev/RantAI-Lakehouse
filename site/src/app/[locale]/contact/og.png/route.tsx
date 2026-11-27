import { getTranslations } from "next-intl/server";
import { prefixedLocales } from "@/i18n/routing";
import { renderOgImage } from "@/lib/og-image";

/**
 * The contact page's preview card in each prefixed language, written at
 * build time as `out/<locale>/contact/og.png` (WEB-5 D15).
 */
export const dynamic = "force-static";
export const dynamicParams = false;

export function generateStaticParams() {
  return prefixedLocales.map((locale) => ({ locale }));
}

export async function GET(_request: Request, { params }: { params: Promise<{ locale: string }> }) {
  const { locale } = await params;
  const t = await getTranslations({ locale, namespace: "metadata.contact" });
  return renderOgImage({ headline: t("ogHeadline") });
}
