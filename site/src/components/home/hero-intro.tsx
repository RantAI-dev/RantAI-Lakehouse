import * as React from "react";

/**
 * WEB-5 O47, reworked in O52: a short, calm intro on the first visit, in
 * three beats, about 2.4 s:
 * 1. 0 to 0.9 s, the open sea only: the night sky fades up from black, the
 *    moon brightens and its reflection spreads down the water;
 * 2. 0.9 to 1.8 s, the whale rises from below the surface with a splash
 *    ring, settles into its rest pose and gives one small spout;
 * 3. 1.6 to 2.4 s, the text: the eyebrow, the headline unmasked from left
 *    to right, the lead, the buttons and the reassurance line; the top bar
 *    and the depth gauge come in last.
 *
 * Two paths, chosen by a tiny inline script that only puts classes on <html>.
 * WEB-5 T3: the page starts on the 3D hold, and the WebGL test runs one task
 * after the first paint (creating a context on a cold browser can take
 * seconds, and it used to sit in front of the first paint and the LCP). If
 * the answer is no, the 3D hold is dropped and the DOM beats start at once;
 * the hold and the DOM beats look the same for those first frames (black, the
 * headline masked):
 * - DOM (`lk-intro`): no fast WebGL here. The beats are CSS keyframes on
 *   hooks the hero already has (`data-intro-backdrop`, `data-hero-creature`,
 *   `data-intro-copy`, hero-motion.tsx), over the DOM night lake and the
 *   2D head.
 * - 3D (`lk-intro lk-intro-3d`): the live lake will run. The DOM lake
 *   picture and the DOM creature are never shown; the page holds on black
 *   with the text masked until the scene's first frame, and the scene
 *   (scene-mode.tsx starts it at once on this path) calls
 *   `window.__lkIntroGo`: the canvas fades in on the open sea, the 3D whale
 *   rises (lake-engine.ts, `mascot.rise`) and the text follows on the same
 *   clock as the DOM path. If the scene is not ready by 1.5 s, the DOM
 *   intro plays instead and the scene cross-fades in when it arrives.
 *
 * How it stays cheap and safe:
 * - Transforms, opacity and a mask only, so nothing moves the layout (CLS 0).
 * - The headline is never at opacity 0 (an element at opacity 0 is not a
 *   largest-paint candidate until it shows); it is masked.
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
 */

/**
 * The same test as `hasFastWebGL` (effects/scene/webgl-support.ts), inline so
 * the choice is made without waiting for hydration; the answer is kept on
 * `window.__lkFastGL` so the page does not probe twice. Under `next dev`
 * with the test-only override, the server writes `true` instead (this file
 * is a server component, so the flag's name never reaches the browser).
 */
const PROBE = `(function(){try{var g=document.createElement("canvas").getContext("webgl2",{failIfMajorPerformanceCaveat:true});if(!g)return false;var x=g.getExtension("WEBGL_debug_renderer_info"),r=x?String(g.getParameter(x.UNMASKED_RENDERER_WEBGL)):"";var l=g.getExtension("WEBGL_lose_context");if(l)l.loseContext();return!/swiftshader|llvmpipe|softpipe|software|basic render/i.test(r);}catch(e){return false;}})()`;

const BOOT = (variant: "home" | "lite", forceGL: boolean) => `(function(){try{
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
var ev=["wheel","touchstart","keydown","pointerdown"],done=false,went=false,timer=0;
function end(skip){if(done)return;done=true;clearTimeout(timer);
for(var i=0;i<ev.length;i++)removeEventListener(ev[i],skipNow,true);
if(skip)d.classList.add("lk-intro-end");
requestAnimationFrame(function(){d.classList.remove("lk-intro-lite","lk-intro","lk-intro-3d","lk-intro-go","lk-intro-end");});}
function skipNow(){end(true);}
for(var i=0;i<ev.length;i++)addEventListener(ev[i],skipNow,{capture:true,passive:true});
if(lite){d.classList.add("lk-intro-lite");timer=setTimeout(function(){end(false);},800);return;}
function go(){if(went||done)return;went=true;clearTimeout(timer);d.classList.add("lk-intro-go");timer=setTimeout(function(){end(false);},2600);}
window.__lkIntroGo=go;
d.classList.add("lk-intro","lk-intro-3d");
timer=setTimeout(function(){d.classList.remove("lk-intro-3d");go();},1500);
function probe(){if(done||went)return;var gl=window.__lkFastGL=${forceGL ? "true" : PROBE};if(!gl){d.classList.remove("lk-intro-3d");go();}}
requestAnimationFrame(function(){setTimeout(probe,0);});
}catch(e){}})();`;

const CSS = `
@media (prefers-reduced-motion: no-preference) {
  /* The hold, before the beats start (3D path only: the DOM path starts at once). */
  html.lk-intro:not(.lk-intro-go) [data-intro-backdrop],
  html.lk-intro:not(.lk-intro-go) [data-hero-room],
  html.lk-intro:not(.lk-intro-go) .lk-water,
  html.lk-intro:not(.lk-intro-go) [data-intro-copy] > :not(h1),
  html.lk-intro:not(.lk-intro-go) [data-intro-scroll-cue],
  html.lk-intro:not(.lk-intro-go) .lk-gauge { opacity: 0 !important; }
  html.lk-intro:not(.lk-intro-go) .lk-root > header { transform: translateY(-100%) !important; }
  /* 3D path: the live lake plays the sea and the whale; the DOM picture never shows. */
  html.lk-intro-3d [data-intro-backdrop], html.lk-intro-3d [data-hero-room] { opacity: 0 !important; transition: none; }

  /* Beat 1, the open sea. */
  html.lk-intro-go [data-intro-backdrop], html.lk-intro-go .lk-water { animation: lk-i-sky 0.6s ease-out backwards; }
  html.lk-intro-go [data-intro-backdrop] > :nth-child(2) { animation: lk-i-moon 0.7s 0.1s cubic-bezier(0.2, 0.7, 0.3, 1) backwards; }
  html.lk-intro-go [data-intro-backdrop] > :nth-child(3) { transform-origin: 50% 0; animation: lk-i-glitter 0.6s 0.3s cubic-bezier(0.2, 0.7, 0.3, 1) backwards; }
  /* Beat 2, the whale (DOM path; the 3D whale rises in the scene). */
  html.lk-intro-go [data-hero-creature] { animation: lk-i-rise 0.7s 0.9s cubic-bezier(0.2, 0.75, 0.3, 1) backwards; }
  html.lk-intro-go [data-hero-creature]::before {
    content: ""; position: absolute; left: 50%; top: 96%; width: 72%; aspect-ratio: 4; border-radius: 50%;
    border: 2px solid rgba(169, 212, 255, 0.55); pointer-events: none;
    animation: lk-i-ring 0.75s 1.1s ease-out both;
  }
  @media (max-width: 767.98px) { html.lk-intro-go [data-hero-creature]::before { top: 72%; } }
  html.lk-intro-go [data-hero-creature]::after {
    content: ""; position: absolute; left: 50%; top: -12%; width: 16%; aspect-ratio: 1; border-radius: 50%;
    background: radial-gradient(circle, rgba(207, 230, 255, 0.55) 0%, rgba(169, 212, 255, 0.2) 45%, transparent 70%);
    animation: lk-i-puff 0.6s 1.65s ease-out both; pointer-events: none;
  }
  @media (max-width: 767.98px) { html.lk-intro-go [data-hero-creature]::after { top: 8%; } }
  /* Beat 3, the text, then the top bar and the gauge. */
  html.lk-intro-go [data-intro-copy] > * { animation: lk-i-up 0.45s cubic-bezier(0.2, 0.7, 0.3, 1) backwards; }
  html.lk-intro-go [data-intro-copy] > :nth-child(1) { animation-delay: 1.6s; }
  html.lk-intro-go [data-intro-copy] > :nth-child(3) { animation-delay: 1.95s; }
  html.lk-intro-go [data-intro-copy] > :nth-child(4) { animation-delay: 2.05s; }
  html.lk-intro-go [data-intro-copy] > :nth-child(5) { animation-delay: 2.15s; }
  html.lk-intro [data-intro-copy] > h1,
  html.lk-intro-lite [data-intro-copy] > h1,
  html.lk-intro-lite #contact-heading {
    -webkit-mask-image: linear-gradient(90deg, #000 0%, #000 40%, rgba(0, 0, 0, 0) 60%, rgba(0, 0, 0, 0) 100%);
    mask-image: linear-gradient(90deg, #000 0%, #000 40%, rgba(0, 0, 0, 0) 60%, rgba(0, 0, 0, 0) 100%);
    -webkit-mask-size: 250% 100%; mask-size: 250% 100%;
    -webkit-mask-position: 100% 0; mask-position: 100% 0;
  }
  html.lk-intro-go [data-intro-copy] > h1 { animation: lk-i-unmask 0.65s 1.7s cubic-bezier(0.3, 0.6, 0.3, 1) both; }
  html.lk-intro-lite [data-intro-copy] > h1, html.lk-intro-lite #contact-heading {
    -webkit-mask-image: linear-gradient(90deg, #000 0%, #000 40%, rgba(0, 0, 0, 0.3) 60%, rgba(0, 0, 0, 0.3) 100%);
    mask-image: linear-gradient(90deg, #000 0%, #000 40%, rgba(0, 0, 0, 0.3) 60%, rgba(0, 0, 0, 0.3) 100%);
    animation: lk-i-unmask 0.5s cubic-bezier(0.3, 0.6, 0.3, 1) both;
  }
  html.lk-intro-go [data-intro-scroll-cue] { animation: lk-i-fade 0.35s 2.2s ease-out backwards; }
  html.lk-intro-go .lk-root > header { animation: lk-i-bar 0.4s 2.2s cubic-bezier(0.2, 0.7, 0.3, 1) backwards; }
  html.lk-intro-go .lk-gauge { animation: lk-i-fade 0.4s 2.2s ease-out backwards; }
  html.lk-intro-end *, html.lk-intro-end *::before, html.lk-intro-end *::after { animation: none !important; }
}
@keyframes lk-i-sky { from { opacity: 0; } to { opacity: 1; } }
/* Scale only: the moon and its reflection are centred with Tailwind's \`translate\` property, which a transform here adds to. */
@keyframes lk-i-moon { from { opacity: 0.25; transform: scale(0.82); } to { opacity: 1; transform: scale(1); } }
@keyframes lk-i-glitter { from { opacity: 0; transform: scaleY(0.1); } to { opacity: 1; transform: scaleY(1); } }
@keyframes lk-i-rise { from { opacity: 0; transform: translateY(28%); } to { opacity: 1; transform: translateY(0); } }
@keyframes lk-i-ring { 0% { opacity: 0; transform: translate(-50%, -50%) scale(0.3); } 25% { opacity: 1; } 100% { opacity: 0; transform: translate(-50%, -50%) scale(1.25); } }
@keyframes lk-i-puff { 0% { opacity: 0; transform: translate(-50%, 30%) scale(0.4); } 40% { opacity: 1; } 100% { opacity: 0; transform: translate(-50%, -60%) scale(1.3); } }
@keyframes lk-i-up { from { opacity: 0; transform: translateY(10px); } to { opacity: 1; transform: translateY(0); } }
@keyframes lk-i-unmask {
  from { -webkit-mask-position: 100% 0; mask-position: 100% 0; transform: translateY(0.12em); }
  to { -webkit-mask-position: 0% 0; mask-position: 0% 0; transform: translateY(0); }
}
@keyframes lk-i-fade { from { opacity: 0; } to { opacity: 1; } }
@keyframes lk-i-bar { from { transform: translateY(-100%); } to { transform: translateY(0); } }
`;

// Test-only (webgl-support.ts): `next dev` with the override; always false in the export.
const FORCE_GL = process.env.NODE_ENV === "development" && process.env.NEXT_PUBLIC_WEB5_FORCE_WEBGL === "1";

/** Rendered first in the page's root, before the top bar, so the class is set before anything paints. */
export function HeroIntro({ variant = "home" }: { variant?: "home" | "lite" }) {
  return (
    <>
      <style dangerouslySetInnerHTML={{ __html: CSS }} />
      <script dangerouslySetInnerHTML={{ __html: BOOT(variant, FORCE_GL) }} />
    </>
  );
}
