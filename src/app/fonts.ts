import { IBM_Plex_Sans, Inter, JetBrains_Mono, Source_Serif_4 } from "next/font/google"

/**
 * The fonts a dashboard may use (`BI-17` part A, decision 12). `next/font`
 * downloads and self-hosts them at BUILD time; nothing is requested from a
 * font service when a page is opened. They are not preloaded: most pages never
 * use them, and a browser fetches a file only when a dashboard that names the
 * font is shown. `system` needs no file. The ids match
 * `lakehouse_bi::formatting::SHIPPED_FONTS` and `lib/dashboard-look.ts`.
 */
export const inter = Inter({ subsets: ["latin"], variable: "--font-inter", display: "swap", preload: false })
export const ibmPlexSans = IBM_Plex_Sans({ subsets: ["latin"], weight: ["400", "500", "600", "700"], variable: "--font-ibm-plex-sans", display: "swap", preload: false })
export const sourceSerif = Source_Serif_4({ subsets: ["latin"], variable: "--font-source-serif", display: "swap", preload: false })
export const jetbrainsMono = JetBrains_Mono({ subsets: ["latin"], variable: "--font-jetbrains-mono", display: "swap", preload: false })

/** The class names that define the four variables; put them on `<html>`. */
export const dashboardFontVariables = [inter, ibmPlexSans, sourceSerif, jetbrainsMono].map((f) => f.variable).join(" ")
