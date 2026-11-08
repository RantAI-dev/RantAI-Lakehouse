/**
 * The extra faces (Inter Tight is now the site's own, in `lib/fonts.ts`) the hero lab (`/lab/hero/`) can try on the headline.
 * Files are committed under `src/fonts/lab/` (SIL Open Font License 1.1, the
 * licence text beside each) and loaded through `next/font/local`, like the
 * page's own faces, so the build asks no font service for them. They are
 * imported only by the lab route, so no other page links their CSS or
 * preloads them.
 */
import localFont from "next/font/local";

export const fraunces = localFont({
  src: "../../../fonts/lab/Fraunces-wght-latin.woff2",
  weight: "100 900",
  style: "normal",
  variable: "--font-lab-fraunces",
  display: "swap",
  preload: false,
});

export const instrumentSerif = localFont({
  src: "../../../fonts/instrument-serif/InstrumentSerif-Italic-latin.woff2",
  weight: "400",
  style: "italic",
  variable: "--font-lab-serif",
  display: "swap",
  preload: false,
});

export const bricolage = localFont({
  src: "../../../fonts/lab/BricolageGrotesque-wght-latin.woff2",
  weight: "200 800",
  style: "normal",
  variable: "--font-lab-bricolage",
  display: "swap",
  preload: false,
});

export const labFontVariables = [instrumentSerif.variable, fraunces.variable, bricolage.variable].join(" ");
