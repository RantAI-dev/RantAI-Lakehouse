/**
 * RantAI Design System — Class name utility
 *
 * Merges Tailwind classes with conflict resolution.
 * Uses clsx for conditional logic + tailwind-merge for deduplication.
 *
 * Dependencies: clsx, tailwind-merge
 *
 * @example
 * ```ts
 * cn("px-4 py-2", isActive && "bg-primary text-primary-foreground")
 * cn("rounded-lg", className)
 * ```
 */

import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
