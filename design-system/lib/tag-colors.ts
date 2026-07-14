/**
 * RantAI Design System — Tag / Badge Color Presets
 *
 * Gradient presets for tags, badges, and category labels.
 * Gradient class strings are listed as literals so Tailwind JIT
 * always generates them — dynamic DB values would otherwise be purged.
 *
 * @example
 * ```tsx
 * import { gradientForColor } from "@/design-system/lib/tag-colors";
 *
 * <span className={`bg-gradient-to-r ${gradientForColor(post.tagColor)} px-3 py-1 rounded-4xl`}>
 *   {tag}
 * </span>
 * ```
 */

export const TAG_COLOR_PRESETS = [
  { key: "blue",    label: "Blue",    gradient: "from-blue-950 via-indigo-900 to-blue-900" },
  { key: "emerald", label: "Emerald", gradient: "from-emerald-950 via-teal-900 to-emerald-900" },
  { key: "violet",  label: "Violet",  gradient: "from-violet-950 via-purple-900 to-violet-900" },
  { key: "amber",   label: "Amber",   gradient: "from-amber-950 via-orange-900 to-amber-900" },
  { key: "rose",    label: "Rose",    gradient: "from-rose-950 via-pink-900 to-rose-900" },
  { key: "slate",   label: "Slate",   gradient: "from-zinc-900 via-zinc-800 to-zinc-900" },
] as const;

export type TagColorKey = (typeof TAG_COLOR_PRESETS)[number]["key"];

export const DEFAULT_TAG_GRADIENT = "from-zinc-900 via-zinc-800 to-zinc-900";

/** Returns the Tailwind gradient classes for a given color key. Falls back to slate. */
export function gradientForColor(color?: string | null): string {
  return (
    TAG_COLOR_PRESETS.find((preset) => preset.key === color)?.gradient ??
    DEFAULT_TAG_GRADIENT
  );
}

/* ─────────────────────────────────────────────
   Thumbnail / avatar background colors
   Used in admin CMS thumbnail generator
───────────────────────────────────────────── */
export const THUMBNAIL_COLOR_PRESETS = [
  "#3A5A8C", // steel blue
  "#8B5E3C", // warm brown
  "#4A6741", // forest green
  "#6B6B9A", // muted violet
  "#4A5568", // slate gray
  "#2C7A7B", // teal
  "#7B2D3A", // deep rose
  "#3D3D3D", // charcoal
] as const;

export type ThumbnailColor = (typeof THUMBNAIL_COLOR_PRESETS)[number];
