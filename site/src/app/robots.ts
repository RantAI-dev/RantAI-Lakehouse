import type { MetadataRoute } from "next";
import { siteUrl } from "@/lib/site-url";

// A static export writes this once, as out/robots.txt.
export const dynamic = "force-static";

/** Everything is public and meant to be found (WEB-5 D4): allow all, name the sitemap. */
export default function robots(): MetadataRoute.Robots {
  const base = siteUrl();
  return {
    rules: { userAgent: "*", allow: "/" },
    sitemap: `${base}/sitemap.xml`,
    host: base,
  };
}
