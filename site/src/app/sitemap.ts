import type { MetadataRoute } from "next";
import { languageAlternates, localizedPath } from "@/i18n/paths";
import { routing } from "@/i18n/routing";
import { siteUrl } from "@/lib/site-url";

// A static export writes this once, as out/sitemap.xml.
export const dynamic = "force-static";

/** The pages, by their English path. */
const PAGES = ["/"] as const;

/**
 * Each page once, at its English URL, with every language as an hreflang
 * alternate (WEB-5 D4, the RantAI site's shape).
 */
export default function sitemap(): MetadataRoute.Sitemap {
  const base = siteUrl();
  return PAGES.map((path) => ({
    url: `${base}${localizedPath(routing.defaultLocale, path)}`,
    alternates: {
      languages: Object.fromEntries(
        Object.entries(languageAlternates(path)).map(([l, p]) => [l, `${base}${p}`]),
      ),
    },
    changeFrequency: "monthly" as const,
    priority: 1,
  }));
}
