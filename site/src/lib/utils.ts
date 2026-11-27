import { clsx, type ClassValue } from "clsx";

/**
 * Joins conditional class names.
 *
 * WEB-5 D5: the console's `cn` also ran `tailwind-merge` to resolve
 * conflicting utilities; that library was about 8.5 KB gzip of the site's
 * first JavaScript. Measured in T5, the built HTML of every page has the
 * same class attribute on every element (455 of 455) with and without it,
 * because no call here passes two utilities that conflict. A call that
 * needs an override must say so with one class, not rely on merging.
 */
export function cn(...inputs: ClassValue[]) {
  return clsx(inputs);
}
