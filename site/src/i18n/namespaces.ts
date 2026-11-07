/**
 * The message namespaces: `src/messages/<locale>/<namespace>.json`, one file
 * each. Their own module so the parity test can read the list without
 * loading next-intl's server entry.
 */
export const NAMESPACES = ["common", "metadata", "home"] as const;

export type Namespace = (typeof NAMESPACES)[number];
