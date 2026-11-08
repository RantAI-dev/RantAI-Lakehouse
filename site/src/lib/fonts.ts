/**
 * The page's two faces, from files committed under `src/fonts/` and loaded
 * through `next/font/local`, so neither the build nor the page asks a font
 * service for them (the console's OPS-16 rule, kept). WEB-5 D5: only the
 * landing's own faces; the console's dashboard fonts are not loaded.
 *
 * - Geist (body, UI and the uppercase labels): the console's file from
 *   `design-system/fonts/`, same variable, `swap`, preloaded.
 * - Inter Tight (the big display lines): bold, tight-tracked, upright.
 *   The owner chose it from the hero lab (`/lab/hero/`) for the whole site.
 *   Instrument Serif, the earlier display face, is now loaded by the lab only.
 *
 * No Geist Mono: WEB-5 review fix R7 (owner) keeps monospace for code only,
 * and the one code identifier uses the system's monospace stack
 * (`app/globals.css`, `code`), so no mono file is downloaded.
 */
import localFont from "next/font/local";

export const geist = localFont({
  src: "../fonts/geist/Geist-wght.woff2",
  weight: "100 900",
  style: "normal",
  variable: "--font-sans",
  display: "swap",
  preload: true,
});

export const displayFont = localFont({
  src: "../fonts/inter-tight/InterTight-wght-latin.woff2",
  weight: "100 900",
  style: "normal",
  variable: "--font-display",
  display: "swap",
});

/** The variables, spread on `<html>`. */
export const fontVariables = [geist.variable, displayFont.variable].join(" ");
