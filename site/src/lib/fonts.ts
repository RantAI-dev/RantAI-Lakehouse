/**
 * The page's three faces, from files committed under `src/fonts/` and loaded
 * through `next/font/local`, so neither the build nor the page asks a font
 * service for them (the console's OPS-16 rule, kept). WEB-5 D5: only the
 * landing's own faces; the console's dashboard fonts are not loaded.
 *
 * - Geist (body and UI) and Geist Mono (labels): the console's files from
 *   `design-system/fonts/`, same variables, `swap`, preloaded.
 * - Instrument Serif italic (the big display lines): the deliberate
 *   contrast with the sans sister sites (WEB-1 plan §2.2, §2.4).
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

export const geistMono = localFont({
  src: "../fonts/geist-mono/GeistMono-wght.woff2",
  weight: "100 900",
  style: "normal",
  variable: "--font-mono",
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
export const fontVariables = [geist.variable, geistMono.variable, displayFont.variable].join(" ");
