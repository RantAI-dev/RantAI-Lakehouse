import localFont from "next/font/local"

/**
 * The fonts a dashboard may use (`BI-17` part A, decision 12). The files are
 * committed under `./fonts` and loaded through `next/font/local` (OPS-16,
 * decision D1), so these faces are never fetched at build or at runtime; that
 * build-time Google fetch is what failed CI runs 41446159405, 41449722905 and
 * 41480029356. They
 * are not preloaded: most pages never use them, and a browser fetches a file
 * only when a dashboard that names the font is shown. `system` needs no file.
 * The ids match `lakehouse_bi::formatting::SHIPPED_FONTS` and
 * `lib/dashboard-look.ts`. Sources, versions and weights are in
 * `fonts/README.md`.
 */
export const inter = localFont({ src: "./fonts/inter/Inter-latin-wght.woff2", weight: "100 900", variable: "--font-inter", display: "swap", preload: false })
export const ibmPlexSans = localFont({ src: "./fonts/ibm-plex-sans/IBMPlexSans-latin-wght.woff2", weight: "400 700", variable: "--font-ibm-plex-sans", display: "swap", preload: false })
export const sourceSerif = localFont({ src: "./fonts/source-serif-4/SourceSerif4-latin-opsz-wght.woff2", weight: "200 900", variable: "--font-source-serif", display: "swap", preload: false })
export const jetbrainsMono = localFont({ src: "./fonts/jetbrains-mono/JetBrainsMono-latin-wght.woff2", weight: "100 800", variable: "--font-jetbrains-mono", display: "swap", preload: false })

/** The class names that define the four variables; put them on `<html>`. */
export const dashboardFontVariables = [inter, ibmPlexSans, sourceSerif, jetbrainsMono].map((f) => f.variable).join(" ")
