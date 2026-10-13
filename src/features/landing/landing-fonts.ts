/**
 * Display face for the landing page's big emotional lines.
 *
 * Instrument Serif italic is the deliberate contrast with the sans sister
 * sites (plan §2.2, §2.4). Body and UI stay in Geist, mono labels in Geist
 * Mono; both come from the root layout. `next/font` self-hosts the file at
 * build time, so the page makes no request to Google at runtime.
 */
import { Instrument_Serif } from "next/font/google";

export const displayFont = Instrument_Serif({
  subsets: ["latin"],
  weight: "400",
  style: "italic",
  variable: "--font-display",
  display: "swap",
});
