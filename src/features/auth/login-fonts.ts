import localFont from "next/font/local";

/**
 * Inter Tight, the Lakehouse display face (owner, 2026-11-08: "untuk font
 * sekarang yang Tight aku suka"), for the sign-in page's title only (`WEB-15`,
 * mockup A). The file is the landing site's own, byte for byte, and is loaded
 * through `next/font/local` so nothing is fetched from a font service
 * (OPS-16); provenance and licence are in `src/app/fonts/README.md`.
 *
 * Preloaded: this module is imported by `/login` only, so the preload link
 * lands on that route alone, and the title is the first thing anyone reads.
 */
export const loginDisplay = localFont({
  src: "../../app/fonts/inter-tight/InterTight-wght-latin.woff2",
  weight: "100 900",
  style: "normal",
  variable: "--font-login-display",
  display: "swap",
});

/** Inter Tight, semibold (600) and tight, as on the landing site (DESIGN.md §4). */
export const LOGIN_DISPLAY =
  "font-[family-name:var(--font-login-display)] font-semibold tracking-[-0.035em]";
