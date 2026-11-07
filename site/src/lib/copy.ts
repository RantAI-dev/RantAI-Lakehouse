import type common from "@/messages/en/common.json";
import type home from "@/messages/en/home.json";

/**
 * The shapes of the message files, taken from the English ones. The parity
 * test keeps every other language to the same keys, so these types hold for
 * all of them. Sections receive their part as props from the server
 * (`components/home/home-page.tsx`), so the text is read on the server and
 * the browser gets only what each client leaf draws (WEB-5 D5).
 */
export type CommonCopy = typeof common;
export type HomeCopy = typeof home;
