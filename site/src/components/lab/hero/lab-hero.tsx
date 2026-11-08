"use client";

import * as React from "react";
import { LandingModeProvider, useLandingMode } from "@/components/effects/scene-mode";
import type { LakeEngine } from "@/components/effects/scene/lake-engine";
import { Container, CtaLink, DisplayHeading, Eyebrow, LeadText } from "@/components/shared/primitives";
import { WhaleSvg } from "@/components/shared/whale-svg";
import { useReducedMotionSafe } from "@/hooks/use-reduced-motion";
import type { CommonCopy, HomeCopy } from "@/lib/copy";
import { cn } from "@/lib/utils";
import { ControlBar } from "./control-bar";
import { LabScene } from "./lab-scene";
import { choiceQuery, parseChoice, type FontId, type LayoutId } from "./options";
import "./lab.css";

/**
 * The hero lab. The real hero copy, buttons, creature and 3D scene, with the
 * headline font, the layout and night or day chosen from a control bar and
 * kept in the URL. Removing the lab is deleting `components/lab/`,
 * `src/fonts/lab/` and `app/(en)/lab/`; the scene's day look stays behind a
 * parameter the home page never passes.
 */

// ---- The choice, kept in the URL (read with a store so the server's HTML is the default) ----
const URL_EVENT = "lab-hero-url";
const subscribeUrl = (onChange: () => void) => {
  window.addEventListener("popstate", onChange);
  window.addEventListener(URL_EVENT, onChange);
  return () => {
    window.removeEventListener("popstate", onChange);
    window.removeEventListener(URL_EVENT, onChange);
  };
};

type Hero = HomeCopy["hero"];
type Parts = {
  eyebrow: React.ReactNode;
  headline: React.ReactNode;
  sub: React.ReactNode;
  actions: React.ReactNode;
  reassurance: (align: string) => React.ReactNode;
};

const SWAP_MS = 120;
/** The sun (or moon) beside the creature: `x` of the width, `above` of the height over the horizon. */
const sunFor = (layout: LayoutId): { x: number; above: number } | null => {
  if (layout === "A") return null;
  const phone = window.innerWidth < 768;
  return layout === "B" ? { x: phone ? 0.86 : 0.82, above: 0.07 } : { x: phone ? 0.84 : 0.8, above: 0.08 };
};

export function LabHero(props: {
  fontVariables: string;
  copy: Hero;
  cta: CommonCopy["cta"];
  contactHref: string;
  topBar: React.ReactNode;
}) {
  return (
    <LandingModeProvider>
      <LabHeroInner {...props} />
    </LandingModeProvider>
  );
}

function LabHeroInner({
  fontVariables,
  copy,
  cta,
  contactHref,
  topBar,
}: {
  fontVariables: string;
  copy: Hero;
  cta: CommonCopy["cta"];
  contactHref: string;
  topBar: React.ReactNode;
}) {
  const { sceneReady } = useLandingMode();
  const reduced = useReducedMotionSafe();
  const search = React.useSyncExternalStore(subscribeUrl, () => window.location.search, () => "");
  const choice = React.useMemo(() => parseChoice(search), [search]);
  const change = (patch: { font?: string; layout?: string; time?: string }) => {
    const next = { ...choice, ...patch } as typeof choice;
    window.history.replaceState(null, "", `${window.location.pathname}${choiceQuery(next)}`);
    window.dispatchEvent(new Event(URL_EVENT));
  };

  // Font and layout change under a short fade (the words and the creature
  // leave, the new ones arrive); night and day cross-fade by CSS and by the
  // scene's own blend.
  const [shown, setShown] = React.useState<{ font: FontId; layout: LayoutId }>({ font: choice.font, layout: choice.layout });
  const swapping = shown.font !== choice.font || shown.layout !== choice.layout;
  React.useEffect(() => {
    if (!swapping) return;
    const id = window.setTimeout(() => setShown({ font: choice.font, layout: choice.layout }), reduced ? 0 : SWAP_MS);
    return () => window.clearTimeout(id);
  }, [swapping, choice.font, choice.layout, reduced]);

  const stageRef = React.useRef<HTMLDivElement>(null);
  const [engine, setEngine] = React.useState<LakeEngine | null>(null);

  // The engine learns where the creature stands whenever the layout changes.
  React.useEffect(() => {
    if (!engine) return;
    engine.setLab({ box: shown.layout !== "A", sun: sunFor(shown.layout) });
  }, [engine, shown.layout]);

  // Night to day: a 250 ms blend in the scene (the DOM picture cross-fades in CSS).
  const dayK = React.useRef(0);
  const attached = React.useRef<LakeEngine | null>(null);
  React.useEffect(() => {
    if (!engine) {
      attached.current = null;
      dayK.current = 0;
      return;
    }
    const target = choice.time === "day" ? 1 : 0;
    if (attached.current !== engine || reduced) {
      attached.current = engine;
      dayK.current = target;
      engine.setTimeOfDay(target);
      return;
    }
    const from = dayK.current;
    const t0 = performance.now();
    let raf = 0;
    const step = (now: number) => {
      const p = Math.min(1, (now - t0) / 250);
      const eased = p < 0.5 ? 2 * p * p : 1 - Math.pow(-2 * p + 2, 2) / 2;
      dayK.current = from + (target - from) * eased;
      engine.setTimeOfDay(dayK.current);
      if (p < 1) raf = requestAnimationFrame(step);
    };
    raf = requestAnimationFrame(step);
    return () => cancelAnimationFrame(raf);
  }, [engine, choice.time, reduced]);

  // The DOM picture's horizon, in the stage: where the 3D one is for the
  // centred and editorial layouts (a fixed share of the creature's height
  // over its waterline), and the home page's 61 % for the current one.
  React.useEffect(() => {
    const stage = stageRef.current;
    if (!stage) return;
    const place = () => {
      const box = stage.querySelector<HTMLElement>("[data-hero-box]");
      if (shown.layout === "A" || !box) {
        stage.style.setProperty("--lab-h", "61.2%");
        return;
      }
      const s = stage.getBoundingClientRect();
      const b = box.getBoundingClientRect();
      stage.style.setProperty("--lab-h", `${(b.bottom - s.top - 0.66 * b.height).toFixed(1)}px`);
    };
    place();
    const ro = new ResizeObserver(place);
    ro.observe(stage);
    const box = stage.querySelector("[data-hero-box]");
    if (box) ro.observe(box);
    return () => ro.disconnect();
  }, [shown.layout]);

  const parts: Parts = {
    eyebrow: <Eyebrow className="lab-eyebrow text-balance">{copy.eyebrow}</Eyebrow>,
    headline: (
      <DisplayHeading as="h1" size="xl" className="lab-h1">
        {copy.headline}
      </DisplayHeading>
    ),
    sub: <LeadText className="lab-lead">{copy.sub}</LeadText>,
    actions: (
      <div className="flex flex-wrap items-center gap-3">
        <CtaLink href="/#get-started">{cta.install}</CtaLink>
        <CtaLink href={contactHref} variant="ghost" className="lab-ghost">
          {cta.talk}
        </CtaLink>
      </div>
    ),
    reassurance: (align) => (
      <ul className={cn("lab-reassure flex flex-wrap items-center gap-y-1 text-[13px] leading-5", align)}>
        {copy.reassurance.map((part, i) => (
          <li
            key={part}
            className={cn(
              "whitespace-nowrap",
              i > 0 && "before:mx-2 before:text-[#44A6FA] before:content-['·']",
              i === 0 && "max-sm:basis-full max-sm:text-center",
              i === 1 && "max-sm:before:hidden",
            )}
          >
            {part}
          </li>
        ))}
      </ul>
    ),
  };

  return (
    <div
      className={cn("lab-hero lk-root", fontVariables)}
      data-font={shown.font}
      data-layout={shown.layout}
      data-time={choice.time}
      data-swap={swapping ? "1" : undefined}
    >
      {topBar}
      <div ref={stageRef} className="lab-stage relative isolate flex h-[100svh] flex-col overflow-hidden">
        <LabBackdrop faded={sceneReady} />
        <LabScene observe={stageRef} onEngine={setEngine} />
        <div className="lab-wash lab-wash-night pointer-events-none absolute inset-0" aria-hidden="true" />
        <div className="lab-wash lab-wash-day pointer-events-none absolute inset-0" aria-hidden="true" />
        {shown.layout === "A" && <LayoutA parts={parts} sceneReady={sceneReady} />}
        {shown.layout === "B" && <LayoutB parts={parts} sceneReady={sceneReady} />}
        {shown.layout === "C" && <LayoutC parts={parts} sceneReady={sceneReady} />}
      </div>
      <ControlBar font={choice.font} layout={choice.layout} time={choice.time} onChange={change} />
    </div>
  );
}

// ---- The DOM picture (the fallback when WebGL is not fast) ----------------------------------

function LabBackdrop({ faded }: { faded: boolean }) {
  return (
    <div
      aria-hidden="true"
      className={cn("absolute inset-0 -z-10 transition-opacity duration-400", faded && "opacity-0")}
    >
      <div className="lab-sky lab-sky-night absolute inset-0" />
      <div className="lab-sky lab-sky-day absolute inset-0" />
      <div className="lab-sun-glow lab-moon-glow absolute" />
      <div className="lab-sun-glow lab-sun-halo absolute" />
      <div className="lab-sun-disc absolute" />
      <div className="lab-streak lab-streak-night absolute" />
      <div className="lab-streak lab-streak-day absolute" />
      <svg
        viewBox="0 0 1200 12"
        preserveAspectRatio="none"
        className="lab-waterline absolute inset-x-0 h-3 w-full max-md:hidden"
        aria-hidden="true"
      >
        <path
          d="M0 6 Q 75 3 150 6 T 300 6 T 450 6 T 600 6 T 750 6 T 900 6 T 1050 6 T 1200 6"
          fill="none"
          stroke="currentColor"
          strokeWidth="1"
          vectorEffect="non-scaling-stroke"
        />
      </svg>
    </div>
  );
}

/** The creature as the DOM hero draws it, plus the invisible box the 3D one is fitted to. */
function Creature({ sceneReady, id }: { sceneReady: boolean; id: string }) {
  return (
    <div className="relative size-full">
      <div className={cn("size-full transition-opacity duration-400", sceneReady && "opacity-0")}>
        <WhaleSvg surface water spout={false} idPrefix={id} viewBox="104 78 392 334" className="lk-bob size-full" />
      </div>
      {/* Dome top to waterline: 240 of the drawing's 334 units. */}
      <div data-hero-box="" aria-hidden="true" className="pointer-events-none absolute inset-x-0 top-0 h-[71.9%]" />
    </div>
  );
}

/** A box the creature fits in whatever the room, centred, never wider than `max`. */
function CreatureSlot({ sceneReady, id, className }: { sceneReady: boolean; id: string; className?: string }) {
  return (
    <div className={cn("lab-fade relative min-h-0 w-full [container-type:size]", className)}>
      <div className="absolute top-1/2 left-1/2 aspect-[392/334] w-[min(100cqw,calc(100cqh*1.1737),34rem)] -translate-x-1/2 -translate-y-1/2">
        <Creature sceneReady={sceneReady} id={id} />
      </div>
    </div>
  );
}

// ---- Layout A: the home page's own hero ------------------------------------------------------

function LayoutA({ parts, sceneReady }: { parts: Parts; sceneReady: boolean }) {
  return (
    <>
      <div
        data-hero-room=""
        aria-hidden="true"
        className={cn(
          "lab-fade relative -z-[5] flex min-h-0 flex-1 flex-col transition-opacity duration-400 md:absolute md:top-[calc(18%+min(4.45vw,54px))] md:right-[9%] md:block md:w-[min(38vw,460px)]",
          sceneReady && "opacity-0",
        )}
      >
        <div className="h-[14svh] min-h-[4.5rem] md:hidden" />
        <div
          data-hero-creature=""
          className="relative left-1/2 aspect-[196/167] h-[min(46vw,230px)] min-h-0 -translate-x-1/2 self-start md:static md:h-auto md:w-full md:translate-x-0"
        >
          <WhaleSvg surface water spout={false} idPrefix="lab-a-m" viewBox="104 78 392 334" className="lk-bob size-full md:hidden" />
          <WhaleSvg surface water spout={false} idPrefix="lab-a-d" viewBox="104 -3 392 334" className="lk-bob size-full max-md:hidden" />
        </div>
      </div>
      <Container className="relative z-10 flex flex-none flex-col items-center pt-4 pb-[11.5rem] text-center short:pb-[11.5rem] md:min-h-[100svh] md:items-start md:justify-center md:pt-24 md:pb-24 md:text-left">
        <div className="lab-fade max-w-[660px]">
          {parts.eyebrow}
          <div className="mt-5 short:mt-3">{parts.headline}</div>
          <div className="mx-auto mt-6 max-w-[34rem] short:mt-4 md:mx-0">{parts.sub}</div>
          <div className="mt-9 short:mt-6 [&>div]:justify-center md:[&>div]:justify-start">{parts.actions}</div>
          <div className="mt-6 short:mt-4">{parts.reassurance("justify-center md:justify-start")}</div>
        </div>
      </Container>
    </>
  );
}

// ---- Layout B: centred -----------------------------------------------------------------------

function LayoutB({ parts, sceneReady }: { parts: Parts; sceneReady: boolean }) {
  return (
    <div className="relative z-10 mx-auto flex min-h-0 w-full max-w-[1120px] flex-1 flex-col items-center px-4 pt-[4.5rem] pb-[11.5rem] text-center md:px-6 md:pt-24 md:pb-24">
      <div className="lab-fade lab-b-text flex max-w-[52rem] flex-col items-center gap-4 md:gap-5">
        {parts.eyebrow}
        {parts.headline}
        <div className="max-w-[34rem]">{parts.sub}</div>
      </div>
      <CreatureSlot sceneReady={sceneReady} id="lab-b" className="my-2 flex-1 basis-0 md:my-3" />
      <div className="lab-fade flex flex-col items-center gap-4 [&>div]:justify-center">
        {parts.actions}
        {parts.reassurance("justify-center")}
      </div>
    </div>
  );
}

// ---- Layout C: editorial ---------------------------------------------------------------------

function LayoutC({ parts, sceneReady }: { parts: Parts; sceneReady: boolean }) {
  return (
    <div className="relative z-10 mx-auto flex min-h-0 w-full max-w-[1440px] flex-1 flex-col px-4 pt-[4.75rem] pb-[11.5rem] md:px-8 md:pt-[4.5rem] md:pb-24">
      <CreatureSlot sceneReady={sceneReady} id="lab-c" className="flex-1 basis-0" />
      <div className="lab-fade mt-3 md:mt-5">
        {parts.eyebrow}
        <div className="mt-3 md:mt-4">{parts.headline}</div>
        <div className="mt-5 flex flex-col gap-5 md:mt-7 md:flex-row md:items-end md:justify-between md:gap-10">
          <div className="max-w-[30rem] md:max-w-[28rem]">{parts.sub}</div>
          <div className="flex flex-col gap-3">
            {parts.actions}
            {parts.reassurance("justify-start md:justify-end")}
          </div>
        </div>
      </div>
    </div>
  );
}
