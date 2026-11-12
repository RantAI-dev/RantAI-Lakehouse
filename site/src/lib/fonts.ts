/**
 * The page's two faces, from files committed under `src/fonts/` and loaded
 * through `next/font/local`, so neither the build nor the page asks a font
 * service for them (the console's OPS-16 rule, kept). WEB-5 D5: only the
 * landing's own faces; the console's dashboard fonts are not loaded.
 *
 * - Geist (body, UI and the uppercase labels): the console's file from
 *   `design-system/fonts/`, same variable, `swap`, preloaded.
 * - Instrument Serif italic (the big display lines): the deliberate
 *   contrast with the sans sister sites (WEB-1 plan §2.2, §2.4).
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
  src: "../fonts/instrument-serif/InstrumentSerif-Italic-latin.woff2",
  weight: "400",
  style: "italic",
  variable: "--font-display",
  display: "swap",
});

/** The variables, spread on `<html>`. */
export const fontVariables = [geist.variable, displayFont.variable].join(" ");
