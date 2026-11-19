/**
 * The product's own branding (`ADM-2`, plan decision 1): what the console
 * shows when the workspace set no name and no picture of its own.
 *
 * The two pictures stay files under `public/`, so they need no route and no
 * session — the sign-in page can show them before anyone signs in. This
 * module is imported by client components and by the server-side metadata
 * of a later step, so it holds nothing that only one side may read.
 */

/**
 * The console's own name, shown when the workspace set none. Written in
 * full with the official casing, as on every RantAI surface (owner decision
 * 2026-11-18; it was "Rantai Lake").
 */
export const PRODUCT_NAME = "RantAI Lakehouse"

/**
 * The console's own logo, for a light background: the official mark
 * (`WEB-15`, at its own 196 x 167 ratio; PR #536 review SHOULD-FIX 1 moved
 * the files here, where the fallback lives, instead of hard-coding them in
 * each surface).
 */
export const PRODUCT_LOGO_LIGHT = "/logo-light.svg"

/** The console's own logo, for a dark background (the official mark's dark variant). */
export const PRODUCT_LOGO_DARK = "/logo-dark.svg"

/**
 * The line under the sign-in form that says who built the console
 * (feature decision 8). It names the product, never the workspace, so it
 * stays when the workspace has a name of its own.
 */
export const PRODUCT_BYLINE = "Powered by RantAI Lakehouse"
