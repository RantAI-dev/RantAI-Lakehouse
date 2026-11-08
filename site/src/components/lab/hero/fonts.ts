/**
 * The three extra faces the hero lab (`/lab/hero/`) can try on the headline.
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

export const interTight = localFont({
  src: "../../../fonts/lab/InterTight-wght-latin.woff2",
  weight: "100 900",
  style: "normal",
  variable: "--font-lab-inter-tight",
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

export const labFontVariables = [fraunces.variable, interTight.variable, bricolage.variable].join(" ");
