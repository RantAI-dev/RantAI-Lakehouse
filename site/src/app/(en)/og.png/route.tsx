import { getTranslations } from "next-intl/server";
import { routing } from "@/i18n/routing";
import { renderOgImage } from "@/lib/og-image";

/**
 * The English preview card, written once at build time as `out/og.png`
 * (WEB-5 D4). Not the `opengraph-image` file convention; see
 * `lib/og-image.tsx` for why.
 */
export const dynamic = "force-static";

export async function GET() {
  const t = await getTranslations({ locale: routing.defaultLocale, namespace: "metadata.og" });
  return renderOgImage({ headline: t("headline") });
}
