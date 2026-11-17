/**
 * Display face for the landing page's big emotional lines.
 *
 * Instrument Serif italic is the deliberate contrast with the sans sister
 * sites (plan §2.2, §2.4). Body and UI stay in Geist, mono labels in Geist
 * Mono; both come from the root layout. The file is committed under
 * `src/app/fonts/` and loaded through `next/font/local` (OPS-16, decision D1),
 * so neither the build nor the page asks a font service for it.
 */
import localFont from "next/font/local";

export const displayFont = localFont({
  src: "../../app/fonts/instrument-serif/InstrumentSerif-Italic-latin.woff2",
  weight: "400",
  style: "italic",
  variable: "--font-display",
  display: "swap",
});
