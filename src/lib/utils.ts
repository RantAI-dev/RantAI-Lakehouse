import { clsx, type ClassValue } from "clsx"
import { twMerge } from "tailwind-merge"

/**
 * Merges conditional Tailwind/className inputs into a single string,
 * deduplicating conflicting Tailwind utilities (e.g. `px-2` vs `px-4`).
 *
 * Wraps `clsx` for conditional logic and `tailwind-merge` for conflict resolution.
 * Used throughout the app whenever className composition is needed.
 */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}
