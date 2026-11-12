import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { ImageResponse } from "next/og";

/**
 * The link-preview card, 1200x630, rendered once per language at build time
 * (WEB-5 D4, after the RantAI site's `lib/og-image.tsx`): the creature, the
 * wordmark and the localized headline, in the page's night palette. The
 * headline uses the renderer's built-in sans face: the page's Instrument
 * Serif is committed as woff2 only, which the renderer cannot read.
 *
 * Why route handlers (`app/(en)/og.png`, `app/[locale]/og.png`) and not the
 * `opengraph-image` file convention (plan §6, measured in T4): under
 * `output: "export"` the convention writes the card as an extensionless
 * file (`out/id/opengraph-image`), which a static host serves as
 * `application/octet-stream` and link-preview crawlers may refuse. The fix,
 * `generateImageMetadata` with a `.png` id, does not build for a dynamic
 * segment: the `[locale]` image route then loses its `locale` param ("returned
 * incomplete params", Next 16.4.3). A force-static route handler named
 * `og.png` writes a real `.png` file per language, and `lib/seo.ts`
 * references it by path.
 */

export const ogSize = { width: 1200, height: 630 };
export const ogContentType = "image/png";

let logo: Promise<string> | null = null;
function logoDataUri(): Promise<string> {
  // The white-dome logo, made for dark backgrounds.
  logo ??= readFile(join(process.cwd(), "public/logo-dark.png")).then(
    (b) => `data:image/png;base64,${b.toString("base64")}`,
  );
  return logo;
}

export async function renderOgImage({ headline }: { headline: string }) {
  const src = await logoDataUri();
  return new ImageResponse(
    <div
      style={{
        width: "100%",
        height: "100%",
        display: "flex",
        justifyContent: "space-between",
        alignItems: "center",
        padding: "72px 88px",
        backgroundImage: "linear-gradient(180deg, #020514 0%, #050A30 55%, #0A1F55 100%)",
        color: "#E8F4FF",
      }}
    >
      <div style={{ display: "flex", flexDirection: "column", maxWidth: 680 }}>
        <span style={{ fontSize: 30, letterSpacing: "0.22em", color: "#44A6FA" }}>RANTAI LAKEHOUSE</span>
        <span style={{ marginTop: 36, fontSize: 88, lineHeight: 1.04, letterSpacing: "-0.02em" }}>{headline}</span>
      </div>
      {/* eslint-disable-next-line @next/next/no-img-element -- rendered to PNG, not the DOM */}
      <img src={src} width={330} height={330} alt="" />
    </div>,
    ogSize,
  );
}
