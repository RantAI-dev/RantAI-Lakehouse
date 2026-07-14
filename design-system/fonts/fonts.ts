/**
 * RantAI Design System — Font Setup
 *
 * Geist (sans) + Geist Mono — Google Fonts loaded via next/font.
 * Import this in your root layout.tsx and apply the variables to <html>.
 *
 * @example
 * ```tsx
 * // src/app/layout.tsx
 * import { geist, geistMono, fontVariables } from "@/design-system/fonts/fonts";
 *
 * export default function RootLayout({ children }) {
 *   return (
 *     <html className={fontVariables}>
 *       <body>{children}</body>
 *     </html>
 *   );
 * }
 * ```
 */

import { Geist, Geist_Mono } from "next/font/google";

export const geist = Geist({
  subsets: ["latin"],
  variable: "--font-sans",
  display: "swap",
});

export const geistMono = Geist_Mono({
  subsets: ["latin"],
  variable: "--font-mono",
  display: "swap",
});

/**
 * Concatenated className string to spread onto <html>.
 * Includes both CSS variable declarations + antialiased.
 */
export const fontVariables = [
  geist.variable,
  geistMono.variable,
  "font-sans",
  "antialiased",
].join(" ");
