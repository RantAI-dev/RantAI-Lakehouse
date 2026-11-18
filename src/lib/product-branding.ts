/**
 * The product's own branding (`ADM-2`, plan decision 1): what the console
 * shows when the workspace set no name and no picture of its own.
 *
 * The two pictures stay files under `public/`, so they need no route and no
 * session — the sign-in page can show them before anyone signs in. This
 * module is imported by client components and by the server-side metadata
 * of a later step, so it holds nothing that only one side may read.
 */

/** The console's own name, shown when the workspace set none. */
export const PRODUCT_NAME = "Rantai Lake"

/** The console's own logo, for a light background. */
export const PRODUCT_LOGO_LIGHT = "/logo-light.png"

/** The console's own logo, for a dark background. */
export const PRODUCT_LOGO_DARK = "/logo-dark.png"

/**
 * The line under the sign-in form that says who built the console
 * (feature decision 8). It spells the product's full name, as the landing
 * page spells it; `PRODUCT_NAME` is the console's own short name.
 */
export const PRODUCT_BYLINE = "Powered by RantAI Lakehouse"
