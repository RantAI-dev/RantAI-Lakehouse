import { getTranslations } from "next-intl/server";
import { prefixedLocales } from "@/i18n/routing";
import { renderOgImage } from "@/lib/og-image";

/**
 * The preview card of each prefixed language, written once at build time as
 * `out/<locale>/og.png` (WEB-5 D4). Not the `opengraph-image` file
 * convention; see `lib/og-image.tsx` for why.
 */
export const dynamic = "force-static";
export const dynamicParams = false;

export function generateStaticParams() {
  return prefixedLocales.map((locale) => ({ locale }));
}

export async function GET(_request: Request, { params }: { params: Promise<{ locale: string }> }) {
  const { locale } = await params;
  const t = await getTranslations({ locale, namespace: "metadata.og" });
  return renderOgImage({ headline: t("headline") });
}
