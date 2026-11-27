import { getTranslations } from "next-intl/server";
import { routing } from "@/i18n/routing";
import { renderOgImage } from "@/lib/og-image";

/**
 * The English contact page's preview card, written at build time as
 * `out/contact/og.png` (WEB-5 D15; see `lib/og-image.tsx` for why a route
 * handler).
 */
export const dynamic = "force-static";

export async function GET() {
  const t = await getTranslations({ locale: routing.defaultLocale, namespace: "metadata.contact" });
  return renderOgImage({ headline: t("ogHeadline") });
}
