/**
 * Day and night on the home page's surface scenes (the hero, the final call
 * and the sky the dive starts from). The underwater sections never change.
 *
 * - The mode lives on `<html data-tod="day|night">`, set before first paint
 *   by `TOD_BOOT` (a tiny inline script, allowed by the CSP's inline-script
 *   rule like the intro's WebGL probe, `lib/csp.ts`).
 * - Which mode wins: the visitor's stored choice (`localStorage`, in a try),
 *   then `?tod=day|night` (testing only; never stored), then the local hour
 *   (day 06:00 to 17:59). Without JavaScript the page is night.
 * - CSS keys on the attribute (app/globals.css); the 3D scene listens
 *   through `subscribeTod` (effects/scene/lake-scene.tsx).
 *
 * Every number that tunes the look is in one place: the 3D colours are the
 * `DAY` object in `effects/scene/lake-engine.ts`, the cross-fade is
 * `TOD_FADE_MS` here (the same 600 ms in CSS: `--lk-tod-ms` in globals.css).
 */
export type Tod = "day" | "night";

export const TOD_STORAGE_KEY = "lk-tod";
export const TOD_FADE_MS = 600;
export const TOD_EVENT = "lk-tod-change";

export const TOD_BOOT = `(function(){var d=document.documentElement,m;try{var s=null;try{s=localStorage.getItem("${TOD_STORAGE_KEY}")}catch(e){}var q=/[?&]tod=(day|night)(&|$)/.exec(location.search);m=q?q[1]:s==="day"||s==="night"?s:(function(h){return h>=6&&h<18?"day":"night"})(new Date().getHours())}catch(e){m="night"}d.setAttribute("data-tod",m)})();`;

export function getTod(): Tod {
  return typeof document !== "undefined" && document.documentElement.getAttribute("data-tod") === "day" ? "day" : "night";
}

export function setTod(next: Tod, remember = true): void {
  document.documentElement.setAttribute("data-tod", next);
  if (remember) {
    try {
      localStorage.setItem(TOD_STORAGE_KEY, next);
    } catch {
      /* private window or blocked storage: the choice just lasts this visit */
    }
  }
  window.dispatchEvent(new Event(TOD_EVENT));
}

export function toggleTod(): void {
  setTod(getTod() === "day" ? "night" : "day");
}

export function subscribeTod(onChange: () => void): () => void {
  window.addEventListener(TOD_EVENT, onChange);
  return () => window.removeEventListener(TOD_EVENT, onChange);
}
