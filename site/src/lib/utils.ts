import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/**
 * Merges conditional class names and resolves conflicting Tailwind
 * utilities (`px-2` vs `px-4`): the console's `cn`, which the landing
 * sections were written against.
 */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
