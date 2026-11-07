import * as React from "react";

/**
 * WEB-5 O47 (owner: "kita juga belum ada animasi pas pertama kali reveal
 * pas user pertama kali buka halaman kita"): a short, calm intro on the
 * first visit, about 1.5 s:
 * 1. the night sky (and the page-wide water behind it) fades up from black;
 * 2. the moon brightens and its reflection spreads down the water;
 * 3. the creature rises to the waterline and breathes out a small puff;
 * 4. the headline is unmasked from left to right as it rises a little;
 * 5. the eyebrow, the lead, the buttons and the reassurance line follow;
 * 6. the top bar slides in last, and the depth gauge fades in with it.
 *
 * How it stays cheap and safe:
 * - A tiny inline script, first in the page, decides before the first
 *   paint and only puts a class on <html>; everything else is CSS
 *   keyframes on hooks the hero already has (`data-intro-backdrop`,
 *   `data-hero-creature`, `data-intro-copy`, hero-motion.tsx). Transforms,
 *   opacity and a mask only, so nothing moves the layout (CLS 0).
 * - The headline is never at opacity 0: its mask starts with the text at
 *   30 % and sweeps to full, so it is painted in the first frame and the
 *   page's largest paint is not delayed (an element at opacity 0 is not a
 *   candidate until it shows).
 * - First visit per session only (`sessionStorage`, in a try). Not on a
 *   back or forward navigation, not under reduced motion, and without
 *   JavaScript the page is static as before.
 * - WEB-5 O49, for QA: `?intro` or `#intro` in the URL plays the full intro
 *   again on any page that has it, whatever the session says (reduced
 *   motion still wins).
 * - Any wheel, touch, key or click ends it at once (`lk-intro-end` stops
 *   every animation, so everything is in its final place).
 * - `/id/` (and the contact page, `variant="lite"`): only a light reveal
 *   of the hero text, no full intro.
 * The same sequence runs over the DOM hero whether or not WebGL follows:
 * the live lake only fades in later, once the page is usable.
 */

const BOOT = (variant: "home" | "lite") => `(function(){try{
var d=document.documentElement;
if(window.matchMedia&&matchMedia("(prefers-reduced-motion: reduce)").matches)return;
var force=/(^|[?&])intro(=|&|$)/.test(location.search.slice(1))||location.hash==="#intro";
if(!force){
var n=performance.getEntriesByType?performance.getEntriesByType("navigation")[0]:null;
if(n&&n.type==="back_forward")return;
var k="lk-intro-seen:"+location.pathname;
if(sessionStorage.getItem(k))return;
sessionStorage.setItem(k,"1");}
var lite=!force&&${variant === "lite" ? "true" : 'location.pathname.indexOf("/id")===0'};
var c=lite?"lk-intro-lite":"lk-intro";
d.classList.add(c);
var ev=["wheel","touchstart","keydown","pointerdown"],done=false;
function end(skip){if(done)return;done=true;
for(var i=0;i<ev.length;i++)removeEventListener(ev[i],skipNow,true);
if(skip)d.classList.add("lk-intro-end");
requestAnimationFrame(function(){d.classList.remove(c,"lk-intro-end");});}
function skipNow(){end(true);}
for(var i=0;i<ev.length;i++)addEventListener(ev[i],skipNow,{capture:true,passive:true});
setTimeout(function(){end(false);},lite?800:1700);
}catch(e){}})();`;

const CSS = `
@media (prefers-reduced-motion: no-preference) {
  html.lk-intro [data-intro-backdrop], html.lk-intro .lk-water { animation: lk-i-sky 0.55s ease-out both; }
  html.lk-intro [data-intro-backdrop] > :nth-child(2) { animation: lk-i-moon 0.8s 0.15s cubic-bezier(0.2, 0.7, 0.3, 1) both; }
  html.lk-intro [data-intro-backdrop] > :nth-child(3) { transform-origin: 50% 0; animation: lk-i-glitter 0.7s 0.35s cubic-bezier(0.2, 0.7, 0.3, 1) both; }
  html.lk-intro [data-hero-creature] { animation: lk-i-rise 0.75s 0.35s cubic-bezier(0.2, 0.75, 0.3, 1) both; }
  html.lk-intro [data-hero-creature]::after {
    content: ""; position: absolute; left: 50%; top: -12%; width: 16%; aspect-ratio: 1; border-radius: 50%;
    background: radial-gradient(circle, rgba(207, 230, 255, 0.55) 0%, rgba(169, 212, 255, 0.2) 45%, transparent 70%);
    animation: lk-i-puff 0.6s 0.9s ease-out both; pointer-events: none;
  }
  html.lk-intro [data-intro-copy] > * { animation: lk-i-up 0.45s cubic-bezier(0.2, 0.7, 0.3, 1) both; }
  html.lk-intro [data-intro-copy] > :nth-child(1) { animation-delay: 0.45s; }
  html.lk-intro [data-intro-copy] > :nth-child(3) { animation-delay: 0.8s; }
  html.lk-intro [data-intro-copy] > :nth-child(4) { animation-delay: 0.9s; }
  html.lk-intro [data-intro-copy] > :nth-child(5) { animation-delay: 1s; }
  html.lk-intro [data-intro-copy] > h1,
  html.lk-intro-lite [data-intro-copy] > h1,
  html.lk-intro-lite #contact-heading {
    -webkit-mask-image: linear-gradient(90deg, #000 0%, #000 40%, rgba(0, 0, 0, 0.3) 60%, rgba(0, 0, 0, 0.3) 100%);
    mask-image: linear-gradient(90deg, #000 0%, #000 40%, rgba(0, 0, 0, 0.3) 60%, rgba(0, 0, 0, 0.3) 100%);
    -webkit-mask-size: 250% 100%; mask-size: 250% 100%;
    animation: lk-i-unmask 0.65s 0.5s cubic-bezier(0.3, 0.6, 0.3, 1) both;
  }
  html.lk-intro-lite [data-intro-copy] > h1, html.lk-intro-lite #contact-heading { animation-duration: 0.5s; animation-delay: 0s; }
  html.lk-intro [data-intro-scroll-cue] { animation: lk-i-fade 0.35s 1.15s ease-out both; }
  html.lk-intro .lk-root > header { animation: lk-i-bar 0.4s 1.1s cubic-bezier(0.2, 0.7, 0.3, 1) both; }
  html.lk-intro .lk-gauge { animation: lk-i-fade 0.4s 1.1s ease-out both; }
  html.lk-intro-end *, html.lk-intro-end *::after { animation: none !important; }
}
@keyframes lk-i-sky { from { opacity: 0; } to { opacity: 1; } }
/* Scale only: the moon and its reflection are centred with Tailwind's \`translate\` property, which a transform here adds to. */
@keyframes lk-i-moon { from { opacity: 0.25; transform: scale(0.82); } to { opacity: 1; transform: scale(1); } }
@keyframes lk-i-glitter { from { opacity: 0; transform: scaleY(0.1); } to { opacity: 1; transform: scaleY(1); } }
@keyframes lk-i-rise { from { opacity: 0; transform: translateY(28%); } to { opacity: 1; transform: translateY(0); } }
@keyframes lk-i-puff { 0% { opacity: 0; transform: translate(-50%, 30%) scale(0.4); } 40% { opacity: 1; } 100% { opacity: 0; transform: translate(-50%, -60%) scale(1.3); } }
@keyframes lk-i-up { from { opacity: 0; transform: translateY(10px); } to { opacity: 1; transform: translateY(0); } }
@keyframes lk-i-unmask {
  from { -webkit-mask-position: 100% 0; mask-position: 100% 0; transform: translateY(0.12em); }
  to { -webkit-mask-position: 0% 0; mask-position: 0% 0; transform: translateY(0); }
}
@keyframes lk-i-fade { from { opacity: 0; } to { opacity: 1; } }
@keyframes lk-i-bar { from { transform: translateY(-100%); } to { transform: translateY(0); } }
`;

/** Rendered first in the page's root, before the top bar, so the class is set before anything paints. */
export function HeroIntro({ variant = "home" }: { variant?: "home" | "lite" }) {
  return (
    <>
      <style dangerouslySetInnerHTML={{ __html: CSS }} />
      <script dangerouslySetInnerHTML={{ __html: BOOT(variant) }} />
    </>
  );
}
