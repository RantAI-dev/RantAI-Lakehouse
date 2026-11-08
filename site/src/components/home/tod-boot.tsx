import { TOD_BOOT } from "@/lib/tod";

/** Rendered first in the home page, so `data-tod` is on <html> before anything paints. */
export function TodBoot() {
  return <script dangerouslySetInnerHTML={{ __html: TOD_BOOT }} />;
}
