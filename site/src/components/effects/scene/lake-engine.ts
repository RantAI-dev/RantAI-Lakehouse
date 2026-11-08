// WEB-5 D5: named imports, so the scene chunk carries only what it uses.
import {
  ACESFilmicToneMapping,
  AmbientLight,
  Color,
  DirectionalLight,
  Material,
  MathUtils,
  Mesh,
  PerspectiveCamera,
  Raycaster,
  Scene,
  ShaderMaterial,
  Vector2,
  Vector3,
  WebGLRenderer,
} from "three";
import { LENS, lensCoverage, sceneStateAt, smoothstep, type SceneLayout, type SceneState } from "@/lib/scroll-timeline";
import { displayedHex, publishSceneColours } from "@/lib/scene-colour";
import { createLens } from "./lens";
import { createMascot, type Aim } from "./mascot";
import { CAMERA_FOV, MASCOT_H, MOON_DIR, SLAB_Y, STILL_TIME } from "./scene-config";
import { buildEnvironment, createSkyDome, createSkyUniforms, createStars } from "./sky";
import { createSlabs, SLAB_SIZE } from "./slabs";
import { applyWaterFog, createShafts, createSnow, createWaterFog } from "./underwater";
import { createWater } from "./water";

/**
 * Owns the renderer, the scene graph and the loop for the lake.
 *
 * Why plain three.js and not React Three Fiber: one canvas, one scene, and a
 * loop that must stop when it cannot be seen. The engine is created in a
 * `useEffect` by `lake-scene.tsx` and torn down by `dispose()`, which releases
 * every geometry, material, texture and the renderer itself (plan §2.9).
 *
 * The canvas is a sticky layer behind the hero, sources and depths sections;
 * each frame the engine reads `scrollY`, smooths it, and asks
 * `sceneStateAt` where the camera should be. Scroll drives everything; the
 * only clock is for ambient motion (waves, bob, twinkle).
 *
 * Budgets (plan §2.9): pixel ratio capped at 1.75, 1.25 on phones; the loop
 * runs only while the page zone is on screen and the tab is visible;
 * particle counts halve below 768 px. If frames run long for a while the
 * pixel ratio steps down by itself.
 *
 * WEB-5 O5, the crossing. Every frame goes through the lens pass
 * (`lens.ts`): the scene is drawn off screen and the lens puts it on screen.
 * While the waterline is on the lens the scene is drawn twice from the same
 * camera, once as it looks above the water and once below, and the lens
 * splits the screen along the water. The camera never hops: to keep its
 * near plane off the water, the water plane itself steps a few centimetres
 * away from the camera in each picture (`PLANE_CLEAR`), which no one can
 * see, since the split hides the plane near the lens.
 */

export type EngineMode = "full" | "still";

export type EngineOptions = {
  canvas: HTMLCanvasElement;
  mode: EngineMode;
  /** The page zone the canvas sits behind; the loop sleeps while it is off screen. */
  observe: Element;
  /** The WebGL context was lost; the page falls back to the DOM hero. */
  onContextLost: () => void;
};

export type LakeEngine = {
  dispose: () => void;
  /**
   * Day look (the lab's `/lab/hero/` only; the home page never calls it, so
   * the lake stays the night scene). 0 is night, exactly as built; 1 is a
   * calm day: a pale sky, a soft sun for the moon, brighter water. Any value
   * between blends the two, so the lab can ease from one to the other.
   */
  setTimeOfDay: (day: number) => void;
  /**
   * Lab framing (`/lab/hero/` only). `box`: put the creature where the page's
   * `[data-hero-box]` element is (its top is the dome's top, its bottom the
   * waterline), easing there when the page moves it. `sun`: put the sun (or
   * the moon) at `x` of the width and `above` of the height above the horizon;
   * null keeps the scene's own direction.
   */
  setLab: (lab: { box: boolean; sun: { x: number; above: number } | null }) => void;
  /** Measure the page again (the lab calls it after it moves the creature's box). */
  refit: () => void;
};

/** The day look, as the colours the night scene builds with and the ones it blends to (WEB-5 lab). */
const DAY = {
  horizon: new Color("#e3eef8"),
  mid: new Color("#a9cdee"),
  zenith: new Color("#6fa6e3"),
  sun: new Color("#fff3d6"),
  deep: new Color("#0f4a86"),
  light: new Color("#fff2dc"),
  ambient: new Color("#6d90c8"),
} as const;

const clamp = MathUtils.clamp;
const lerp = MathUtils.lerp;

/**
 * Fog and background by depth: lake-blue near the surface, deep blue at the
 * bottom of the slabs. WEB-5 O33: the page keeps darkening for eight more
 * viewports below this zone, down to the abyss, so the 3D water must still
 * be a deep blue at Gold (about #000d46 on screen), not black: the old
 * deep #020b2a tone-mapped to #000115, darker than the page's own end.
 */
const UNDER_NEAR = new Color("#0b3a74");
const UNDER_MID = new Color("#0a2a72");
const UNDER_DEEP = new Color("#071f52");
/** The light that comes down through the surface, scattered: the water's colour looking up. */
const UNDER_LIGHT = new Color("#2a74c4");

/** How far the water plane keeps from the camera in each picture (the near plane is 0.03). */
const PLANE_CLEAR = 0.055;

/** The moonlight under water: bent towards the vertical by the surface (Snell, n 1.333). */
const UNDER_MOON = (() => {
  const [x, y, z] = MOON_DIR;
  const sinAir = Math.sqrt(1 - y * y);
  const sinWater = sinAir / 1.333;
  const k = sinWater / sinAir;
  return [x * k, Math.sqrt(1 - sinWater * sinWater), z * k] as const;
})();

export function createLakeEngine(opts: EngineOptions): LakeEngine {
  const { canvas, mode, observe } = opts;
  // The layer div the page sizes. WEB-5 O53: the canvas moves between the
  // hero zone's layer and the final call's (`setView`), so this follows it.
  const heroHost = canvas.parentElement ?? canvas;
  let host: HTMLElement = heroHost;
  // WEB-5 O53: the final call (`#final`, home/final-section.tsx) shows the
  // same lake again, at the surface: one renderer, one context, one chunk.
  const finalSection = document.getElementById("final");
  const finalHost = finalSection?.querySelector<HTMLElement>("[data-final-scene]") ?? null;
  let view: "hero" | "final" = "hero";

  const still = mode === "still";
  let phone = window.innerWidth < 768;

  const maxDpr = phone ? 1.25 : 1.75;
  let dpr = Math.min(window.devicePixelRatio || 1, maxDpr);

  // The canvas needs no MSAA of its own: the scene is drawn off screen
  // (into the lens' main picture, which has it on desktop) and the canvas
  // only receives the lens pass.
  const renderer = new WebGLRenderer({
    canvas,
    antialias: false,
    powerPreference: "high-performance",
  });
  renderer.setPixelRatio(dpr);
  renderer.setClearColor(0x000000, 1);
  renderer.toneMapping = ACESFilmicToneMapping;
  renderer.toneMappingExposure = 1.0;
  const lens = createLens(renderer, { lensHalf: LENS.half, msaa: !phone });

  const scene = new Scene();
  const camera = new PerspectiveCamera(CAMERA_FOV, 1, 0.03, 600);

  // ---- Sky, stars, environment ------------------------------------------
  const skyUniforms = createSkyUniforms();
  // One air-fog colour object shared by the water's top and every mesh,
  // so they can never disagree at the horizon; the water's own fog
  // (O5) is a set of uniforms shared the same way.
  const airFog = new Color().copy(skyUniforms.uSkyHorizon.value);
  const fog = createWaterFog(airFog);
  const sky = createSkyDome(skyUniforms, fog);
  scene.add(sky);
  const stars = createStars(phone ? 300 : 600, dpr);
  scene.add(stars.points);
  const envTarget = buildEnvironment(renderer, skyUniforms, fog);
  scene.environment = envTarget.texture;
  scene.environmentIntensity = 0.95;

  // ---- Water -------------------------------------------------------------
  const water = createWater(skyUniforms, fog, phone);
  water.uniforms.uFogColor.value = airFog;
  scene.add(water.mesh);

  // ---- Mascot ------------------------------------------------------------
  const mascot = createMascot({
    lowQuality: phone,
    pixelRatio: dpr,
    onRipple: (amp) => water.spawnRipple(clock.t, amp),
  });
  mascot.still = still;
  scene.add(mascot.group);
  scene.add(mascot.extras);

  // ---- Underwater --------------------------------------------------------
  const shafts = createShafts(phone ? 5 : 9);
  scene.add(shafts.group);
  // Marine snow: sparse and small (O5), so it reads as water, not as weather.
  const snow = createSnow(phone ? 420 : 900);
  scene.add(snow.points);

  // ---- Medallion slabs ----------------------------------------------------
  const slabs = createSlabs(phone);
  scene.add(slabs.group);

  // Every one of three's own materials (the creature, the slabs) takes the
  // water's fog in place of three's (O5, `applyWaterFog`).
  scene.traverse((o) => {
    const mesh = o as Partial<Mesh>;
    const mats = mesh.material ? (Array.isArray(mesh.material) ? mesh.material : [mesh.material]) : [];
    for (const m of mats)
      if (!(m as ShaderMaterial).isShaderMaterial && (m as { fog?: boolean }).fog)
        applyWaterFog(m, fog, m === mascot.skin ? { caustics: true, extra: mascot.paintSkin } : {});
  });

  // ---- Lights ------------------------------------------------------------
  // Moonlight from behind the creature rims its dome. There is deliberately no
  // front fill: a directional light in front of a clearcoated dome shows up as
  // a hot spot on its face. The face is lit by the ambient term, the water
  // bounce (a hemisphere light, diffuse only) and the eye's own glow.
  const moonLight = new DirectionalLight(0xbcd8ff, 3.2);
  moonLight.position.set(...MOON_DIR).multiplyScalar(10);
  scene.add(moonLight);
  const ambient = new AmbientLight(0x1b2f7a, 0.8);
  scene.add(ambient);

  const clock = { t: still ? STILL_TIME : 0, last: 0 };

  // ---- Day look (lab only) ----------------------------------------------
  // The night values are read back from what the scene was built with, so
  // blending to 0 restores them exactly.
  const NIGHT = {
    horizon: skyUniforms.uSkyHorizon.value.clone(),
    mid: skyUniforms.uSkyMid.value.clone(),
    zenith: skyUniforms.uSkyZenith.value.clone(),
    moon: skyUniforms.uMoonColor.value.clone(),
    deep: (water.uniforms.uDeep.value as Color).clone(),
    light: moonLight.color.clone(),
    ambient: ambient.color.clone(),
  };
  let dayK = 0;
  const applyDay = (k: number) => {
    dayK = k;
    skyUniforms.uSkyHorizon.value.copy(NIGHT.horizon).lerp(DAY.horizon, k);
    skyUniforms.uSkyMid.value.copy(NIGHT.mid).lerp(DAY.mid, k);
    skyUniforms.uSkyZenith.value.copy(NIGHT.zenith).lerp(DAY.zenith, k);
    airFog.copy(skyUniforms.uSkyHorizon.value);
    skyUniforms.uMoonColor.value.copy(NIGHT.moon).lerp(DAY.sun, k);
    skyUniforms.uMoonBoost.value = lerp(7, 5, k);
    // The sun is a soft, wide disc with a broad glow; the moon stays small and sharp.
    skyUniforms.uMoonDisc.value.set(lerp(0.99978, 0.9991, k), lerp(0.99988, 0.9996, k));
    skyUniforms.uMoonHalo.value = lerp(1, 2.6, k);
    stars.uniforms.uAlpha.value = 1 - k;
    stars.points.visible = k < 0.999;
    (water.uniforms.uDeep.value as Color).copy(NIGHT.deep).lerp(DAY.deep, k);
    water.uniforms.uWaveGain.value = lerp(1, 0.5, k);
    water.uniforms.uFogDensity.value = lerp(0.021, 0.013, k);
    renderer.toneMappingExposure = lerp(1, 1.12, k);
    scene.environmentIntensity = lerp(0.95, 0.7, k);
    moonLight.color.copy(NIGHT.light).lerp(DAY.light, k);
    ambient.color.copy(NIGHT.ambient).lerp(DAY.ambient, k);
  };

  // ---- Input -------------------------------------------------------------
  const pointer = { x: 0, y: 0, seen: false, lastInput: -100 };
  const tilt = { x: 0, y: 0, active: false };
  const raycaster = new Raycaster();
  const ndc = new Vector2();

  const setPointer = (e: PointerEvent) => {
    pointer.x = (e.clientX / window.innerWidth) * 2 - 1;
    pointer.y = -((e.clientY / window.innerHeight) * 2 - 1);
    pointer.seen = true;
    pointer.lastInput = clock.t;
  };
  const isInteractive = (el: EventTarget | null) =>
    el instanceof Element && el.closest("a,button,input,select,textarea,summary,[role='button']") !== null;
  const onPointerDown = (e: PointerEvent) => {
    setPointer(e);
    // Only the hero's own screen area (or the final call) counts as "the dome".
    if (still || (view === "hero" && window.scrollY > layout.vh * 0.5) || isInteractive(e.target)) return;
    ndc.set(pointer.x, pointer.y);
    raycaster.setFromCamera(ndc, camera);
    if (raycaster.intersectObject(mascot.dome, false).length > 0) mascot.dip(clock.t);
  };
  const onTilt = (e: DeviceOrientationEvent) => {
    if (e.gamma == null || e.beta == null) return;
    tilt.x = clamp(e.gamma / 22, -1, 1);
    tilt.y = clamp(-(e.beta - 50) / 22, -1, 1);
    tilt.active = true;
    pointer.lastInput = clock.t;
  };
  if (!still) {
    window.addEventListener("pointermove", setPointer, { passive: true });
    window.addEventListener("pointerdown", onPointerDown, { passive: true });
    // Phones: the eye follows tilt when the browser already delivers it
    // (Android, or iOS after a permission granted earlier). Without events
    // the idle look-around takes over (plan §2.10).
    if (phone) window.addEventListener("deviceorientation", onTilt, { passive: true });
  }

  // ---- Layout ------------------------------------------------------------
  const layout: SceneLayout = {
    vh: window.innerHeight,
    viewportH: window.innerHeight,
    depthsTop: Number.MAX_SAFE_INTEGER / 4,
    depthsHeight: window.innerHeight * 3,
  };
  const measure = () => {
    if (width > 1) fitSlabsToCard();
    layout.vh = heroHost.clientHeight || window.innerHeight;
    layout.viewportH = window.innerHeight;
    const depths = document.getElementById("depths");
    if (depths) {
      layout.depthsTop = depths.getBoundingClientRect().top + window.scrollY;
      layout.depthsHeight = depths.offsetHeight;
    }
  };

  // Lab state (`/lab/hero/`): all inert unless `setLab` is called.
  const lab: { on: boolean; box: boolean; sun: { x: number; above: number } | null } = {
    on: false,
    box: false,
    sun: null,
  };
  const goal = { x: 0, y: 0, d: 1 };
  let started = false;
  const anchorHero = { x: 0, y: 0 };
  const anchorDepths = { x: 0, y: 0 };
  // WEB-5 O8/O11 framed the whale here, between the hero and the depths;
  // since O30 it stays at the surface and the sources are framed centred.
  const anchorUnder = { x: 0, y: 0 };
  let distScaleHero = 1;
  let distScaleDepths = 1;
  let viewKey = "";
  let width = 1;
  let height = 1;
  const resize = () => {
    const before = { x: anchorHero.x, y: anchorHero.y, d: distScaleHero };
    width = Math.max(1, host.clientWidth);
    height = Math.max(1, host.clientHeight);
    phone = window.innerWidth < 768;
    renderer.setSize(width, height, false);
    lens.setSize(width, height, dpr);
    mascot.setViewport(height * dpr, dpr);
    const aspect = width / height;
    camera.aspect = aspect;
    // Desktop: the creature sits right of centre so the copy has the left.
    // Portrait: it sits in the upper part, above the centred copy.
    const wide = aspect > 1.1;
    anchorHero.x = wide ? 0.42 : 0;
    anchorHero.y = wide ? 0 : 0.42;
    anchorDepths.x = depthsTarget.x = wide ? 0.42 : 0;
    anchorDepths.y = depthsTarget.y = depthsDefaultY = wide ? -0.1 : -0.5;
    const tanV = Math.tan(MathUtils.degToRad(CAMERA_FOV / 2));
    // Pull the camera back until the dome fits the width of a narrow screen.
    distScaleHero = Math.max(1, 2.35 / (tanV * aspect) / 7.2);
    // The slabs shrink sideways to fit a narrow screen rather than pushing the
    // camera away, which the fog would swallow.
    distScaleDepths = wide ? 1.25 : 1;
    // On wide screens also cap it below 1 so the stack stays clear of the copy.
    slabs.setFootprint(Math.min(wide ? 0.86 : 1, (0.7 * 8.06 * aspect * distScaleDepths) / SLAB_SIZE.w));
    fitSlabsToCard();
    // A resize places the slab at once; later refits (fonts, the card's
    // height) ease towards it in `update`.
    anchorDepths.x = depthsTarget.x;
    anchorDepths.y = depthsTarget.y;
    if (lab.box && view === "hero") fitHeroToBox();
    else if (phone && view === "hero") fitHeroToRoom();
    if (view === "final") fitFinal();
    // Lab: the creature eases to its new place instead of jumping (see `update`).
    goal.x = anchorHero.x;
    goal.y = anchorHero.y;
    goal.d = distScaleHero;
    if (lab.on && !still && started) {
      anchorHero.x = before.x;
      anchorHero.y = before.y;
      distScaleHero = before.d;
    }
    aimSun();
    viewKey = "";
    measure();
    camera.updateProjectionMatrix();
    renderOnceIfStill();
  };
  // WEB-5 review fix R12 (BLOCKER): on a short phone the creature framed
  // above (centred, as wide as the screen allows) reached down into the hero
  // copy, as the DOM creature did. The DOM hero now leaves the creature a
  // room above the copy (`data-hero-room`, whose bottom is 16 px above the
  // eyebrow) and sizes its own creature in it (`data-hero-creature`,
  // hero-section.tsx). When the 3D creature, from the waterline to the top
  // of its dome, would end below that room, the camera is pulled back and
  // the view shifted so it fills the DOM creature's box instead. Where it
  // already fits (390×844 and taller) nothing changes.
  const fitCam = new PerspectiveCamera(CAMERA_FOV, 1, 0.05, 600);
  const fitPoint = new Vector3();

  /**
   * WEB-5 O40 (owner: "tablenya di sebelah kanan aja dan di gabung dengan
   * tampilan balok ... biar lebih clean"): the focused slab is the depths
   * card's pedestal. It replaces O34's rule (keep 48 px clear of the card).
   * - The card is `[data-depths-card]` (or else the first cell of the depths
   *   grid), measured in the depths' pinned stage, where it sits while the
   *   slabs show.
   * - The slab, at its focused pose (all three are framed alike), is
   *   projected to the screen: its footprint makes it `PEDESTAL.width` of
   *   the card's width, its centre is the card's centre, and the front edge
   *   of its top face is `PEDESTAL.gap` below the card's bottom, so the card
   *   rests on it (the card's layout leaves about 28 to 40 px there).
   * - Below 1024 px, or where the card spans the screen, the slab is full
   *   width, centred, its top under the card's bottom (O34's stacked case).
   * - The framing eases to its target (`depthsTarget`), so a card that
   *   changes height from one chapter to the next never makes it jump.
   * Bronze, Silver and Gold take the pedestal in turn: the leaving slab
   * sinks and fades as the next rises into place (slabs.ts, `pedestalY`).
   */
  const PEDESTAL = { width: 1.08, gap: 32 } as const;
  const depthsTarget = { x: 0, y: 0 };
  let depthsDefaultY = 0;
  let watchedCard: Element | null = null;
  const slabCorner = new Vector3();
  const slabSpan = (footprint: number): { half: number; top: number; frontTop: number } => {
    // The focused pose for the middle slab (all three are framed alike).
    const y = SLAB_Y[1];
    const k = distScaleDepths;
    fitCam.aspect = width / height;
    fitCam.position.set(0, y - 0.3 + (5 + 0.3) * k, 9.2 * k);
    fitCam.clearViewOffset();
    fitCam.lookAt(0, y - 0.3, 0);
    fitCam.updateProjectionMatrix();
    fitCam.updateMatrixWorld();
    let half = 0;
    let top = Infinity;
    let frontTop = -Infinity;
    for (const sx of [-1, 1])
      for (const sy of [-1, 1])
        for (const sz of [-1, 1]) {
          slabCorner
            .set((sx * SLAB_SIZE.w * footprint) / 2, y + (sy * SLAB_SIZE.h) / 2, (sz * SLAB_SIZE.d * footprint) / 2)
            .project(fitCam);
          const py = ((1 - slabCorner.y) * height) / 2;
          half = Math.max(half, (Math.abs(slabCorner.x) * width) / 2);
          top = Math.min(top, py);
          // The top face's front edge: the lowest of its corners on screen.
          if (sy > 0) frontTop = Math.max(frontTop, py);
        }
    return { half, top, frontTop };
  };
  const fitSlabsToCard = () => {
    const depthsEl = document.getElementById("depths");
    const card =
      depthsEl?.querySelector("[data-depths-card]") ?? depthsEl?.querySelector(".grid")?.firstElementChild ?? null;
    if (!card) return;
    if (card !== watchedCard) {
      // The card's height changes with the chapter: refit when it does.
      if (watchedCard) roCard.unobserve(watchedCard);
      roCard.observe(card);
      watchedCard = card;
    }
    const r = card.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0) return;
    const hostLeft = host.getBoundingClientRect().left;
    const stage = card.closest(".sticky") ?? depthsEl;
    const cardBottom = r.bottom - (stage ? stage.getBoundingClientRect().top : 0);
    const cardCentre = r.left - hostLeft + r.width / 2;
    const stacked = window.innerWidth < 1024 || r.width > width * 0.6;
    if (!stacked) {
      // The projected width is not linear in the footprint (the near corners
      // grow faster), so two proportional steps settle it on the target.
      const wantHalf = (PEDESTAL.width * r.width) / 2;
      let footprint = wantHalf / slabSpan(1).half;
      for (let i = 0; i < 2; i++) footprint *= wantHalf / slabSpan(footprint).half;
      footprint = Math.min(1.4, footprint);
      slabs.setFootprint(footprint);
      const span = slabSpan(footprint);
      depthsTarget.x = ((cardCentre - width / 2) * 2) / width;
      depthsTarget.y = (2 * (span.frontTop - (cardBottom + PEDESTAL.gap))) / height;
      // Dev only (removed from the export): the fit, for review probes.
      if (process.env.NODE_ENV === "development") {
        host.dataset.lkSlabFit = JSON.stringify({
          card: [Math.round(r.left - hostLeft), Math.round(r.right - hostLeft), Math.round(cardBottom)],
          slab: [Math.round(cardCentre - span.half), Math.round(cardCentre + span.half), Math.round(cardBottom + PEDESTAL.gap)],
          footprint: Number(footprint.toFixed(3)),
        });
      }
      return;
    }
    // Stacked: full width (the footprint resize set), centred, its top
    // under the card's bottom.
    depthsTarget.x = 0;
    const top0 = slabSpan(slabs.footprint()).top;
    depthsTarget.y = Math.min(depthsDefaultY, (2 * (top0 - (cardBottom + 12))) / height);
    if (process.env.NODE_ENV === "development") {
      host.dataset.lkSlabFit = JSON.stringify({ stacked: true, cardBottom: Math.round(cardBottom), y: depthsTarget.y });
    }
  };
  const roCard = new ResizeObserver(() => fitSlabsToCard());
  const heroCreatureSpan = (dist: number, ay: number): { top: number; bottom: number } => {
    const hero = sceneStateAt(0, layout);
    fitCam.aspect = width / height;
    fitCam.position
      .set(hero.position[0] - hero.look[0], hero.position[1] - hero.look[1], hero.position[2] - hero.look[2])
      .multiplyScalar(dist)
      .add(fitPoint.set(hero.look[0], hero.look[1], hero.look[2]));
    fitCam.lookAt(fitPoint);
    fitCam.setViewOffset(width, height, 0, (ay * height) / 2, width, height);
    fitCam.updateProjectionMatrix();
    fitCam.updateMatrixWorld();
    const y = (v: number) => ((1 - fitPoint.set(0, v, 0).project(fitCam).y) / 2) * height;
    return { top: y(MASCOT_H), bottom: y(0) };
  };
  const fitHeroToRoom = () => {
    const room = document.querySelector("[data-hero-room]");
    const box = room?.querySelector("[data-hero-creature]");
    const stage = room?.parentElement;
    if (!room || !box || !stage) return;
    const top0 = stage.getBoundingClientRect().top;
    const roomBottom = room.getBoundingClientRect().bottom - top0;
    const r = box.getBoundingClientRect();
    const target = { top: r.top - top0, height: r.height };
    if (heroCreatureSpan(distScaleHero, anchorHero.y).bottom <= roomBottom || target.height <= 0) return;
    // The creature's height on screen goes as 1 / distance, so a few
    // proportional steps settle it; the view offset then moves its centre,
    // since an offset shifts every point by the same number of pixels.
    let dist = distScaleHero;
    for (let i = 0; i < 4; i++) {
      const span = heroCreatureSpan(dist, 0);
      dist *= (span.bottom - span.top) / target.height;
    }
    const span = heroCreatureSpan(dist, 0);
    distScaleHero = dist;
    anchorHero.y = (2 * ((span.top + span.bottom) / 2 - (target.top + target.height / 2))) / height;
  };

  /**
   * Lab: the creature goes where the page's `[data-hero-box]` is, whatever
   * the width: its dome top to waterline fills the box, and its centre is
   * the box's centre. The same steps as `fitHeroToRoom`, now in x as well.
   */
  const fitHeroToBox = () => {
    const box = document.querySelector("[data-hero-box]");
    if (!box) return;
    const hostR = host.getBoundingClientRect();
    const r = box.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0) return;
    let dist = distScaleHero;
    for (let i = 0; i < 4; i++) {
      const span = heroCreatureSpan(dist, 0);
      dist *= (span.bottom - span.top) / r.height;
    }
    const span = heroCreatureSpan(dist, 0);
    distScaleHero = dist;
    anchorHero.y = (2 * ((span.top + span.bottom) / 2 - (r.top - hostR.top + r.height / 2))) / height;
    anchorHero.x = (2 * (r.left - hostR.left + r.width / 2 - width / 2)) / width;
  };

  /**
   * WEB-5 O53: in the final call the whale takes the place of the 2D one,
   * framed like the hero (the same camera pose, so the sky, the moon and its
   * path on the water read the same), sized and placed from the 2D stage
   * (`data-final-stage`, shared/mascot-rising.tsx): its waterline on the
   * stage's, its centre on the stage's, its dome `FINAL_SCALE` times as tall
   * as the 2D head. The stage is below the headline, so the whale never
   * reaches the text.
   */
  const FINAL_SCALE = 1.5;
  const moonDir = skyUniforms.uMoonDir.value; // shared by the sky and the water
  const moonFinal = new Vector3(...MOON_DIR);
  const moonProbe = new Vector3();
  const anchorFinal = { x: 0, y: 0 };
  let distScaleFinal = 1;
  const fitFinal = () => {
    const stage = finalSection?.querySelector<HTMLElement>("[data-final-stage]");
    if (!stage || !finalHost) return;
    const hostR = finalHost.getBoundingClientRect();
    const r = stage.getBoundingClientRect();
    const waterline = Number(stage.dataset.waterline);
    const head = Number(stage.dataset.head) * FINAL_SCALE;
    if (!(r.height > 0) || !Number.isFinite(waterline) || !Number.isFinite(head)) return;
    const waterY = r.top - hostR.top + waterline * r.height;
    const topY = Math.max(r.top - hostR.top, waterY - head * r.height);
    const want = waterY - topY;
    let dist = distScaleHero;
    for (let i = 0; i < 4; i++) {
      const span = heroCreatureSpan(dist, 0);
      dist *= (span.bottom - span.top) / want;
    }
    const span = heroCreatureSpan(dist, 0);
    distScaleFinal = dist;
    anchorFinal.y = (2 * ((span.top + span.bottom) / 2 - (topY + waterY) / 2)) / height;
    anchorFinal.x = (2 * (r.left - hostR.left + r.width / 2 - width / 2)) / width;

    // The moon, clear of the headline: beside it where there is room (wide
    // screens, at its left), else low on the right, between the headline
    // and the horizon. Its light and its path on the water follow it.
    const heading = finalSection?.querySelector("h2, h1");
    const hr = heading?.getBoundingClientRect();
    if (!hr) return;
    const hero = sceneStateAt(0, layout);
    fitPoint.set(hero.look[0], hero.look[1], hero.look[2]);
    fitCam.aspect = width / height;
    fitCam.position
      .set(hero.position[0] - hero.look[0], hero.position[1] - hero.look[1], hero.position[2] - hero.look[2])
      .multiplyScalar(dist)
      .add(fitPoint);
    fitCam.lookAt(fitPoint);
    fitCam.setViewOffset(width, height, (-anchorFinal.x * width) / 2, (anchorFinal.y * height) / 2, width, height);
    fitCam.updateProjectionMatrix();
    fitCam.updateMatrixWorld();
    const pxY = (p: Vector3) => ((1 - p.project(fitCam).y) / 2) * height;
    const horizonY = pxY(moonProbe.set(fitCam.position.x, 0, fitCam.position.z - 400));
    const left = hr.left - hostR.left;
    const headBottom = hr.bottom - hostR.top;
    const beside = left > width * 0.22;
    const sx = beside ? left * 0.5 : width * 0.8;
    const sy = beside ? hr.bottom - hostR.top - hr.height * 0.3 : headBottom + 0.45 * (horizonY - headBottom);
    moonFinal
      .set((sx / width) * 2 - 1, 1 - (sy / height) * 2, 0.5)
      .unproject(fitCam)
      .sub(fitCam.position)
      .normalize();
    if (moonFinal.y < 0.05) moonFinal.setY(0.05).normalize();
    if (view === "final") moonDir.copy(moonFinal);
  };

  /**
   * Lab: the sun (or moon) at a place on the screen, `x` of the width and
   * `above` of the height over the horizon, seen from the hero camera as the
   * creature is framed now. The sky, the water's glitter and the light on the
   * creature all follow `moonDir`.
   */
  const aimSun = () => {
    if (!lab.on || view !== "hero") return;
    if (!lab.sun) {
      moonDir.set(...MOON_DIR);
      return;
    }
    const hero = sceneStateAt(0, layout);
    fitPoint.set(hero.look[0], hero.look[1], hero.look[2]);
    fitCam.aspect = width / height;
    fitCam.position
      .set(hero.position[0] - hero.look[0], hero.position[1] - hero.look[1], hero.position[2] - hero.look[2])
      .multiplyScalar(goal.d)
      .add(fitPoint);
    fitCam.lookAt(fitPoint);
    fitCam.setViewOffset(width, height, (-goal.x * width) / 2, (goal.y * height) / 2, width, height);
    fitCam.updateProjectionMatrix();
    fitCam.updateMatrixWorld();
    const horizonY = ((1 - moonProbe.set(fitCam.position.x, 0, fitCam.position.z - 400).project(fitCam).y) / 2) * height;
    const sy = horizonY - lab.sun.above * height;
    moonFinal
      .set(lab.sun.x * 2 - 1, 1 - (sy / height) * 2, 0.5)
      .unproject(fitCam)
      .sub(fitCam.position)
      .normalize();
    if (moonFinal.y < 0.05) moonFinal.setY(0.05).normalize();
    moonDir.copy(moonFinal);
  };

  const ro = new ResizeObserver(resize);
  ro.observe(host);
  if (finalHost) ro.observe(finalHost);
  // The room changes with the copy's height (fonts, language).
  // The elements whose size or place the creature follows. The lab swaps
  // them when it changes layout, so `watchHeroEls` is called again then.
  const watched: Record<string, Element | null> = { "[data-hero-room]": null, "[data-hero-box]": null };
  const watchHeroEls = () => {
    for (const sel of Object.keys(watched)) {
      const el = document.querySelector(sel);
      if (el === watched[sel]) continue;
      if (watched[sel]) ro.unobserve(watched[sel]);
      if (el) ro.observe(el);
      watched[sel] = el;
    }
  };
  watchHeroEls();
  // The zone's own height changes when fonts load or sections reflow.
  const roZone = new ResizeObserver(measure);
  roZone.observe(observe);

  // ---- Frame -------------------------------------------------------------
  const aim: Aim = { x: 0, y: 0 };
  const mascotNdc = new Vector3();
  const look = new Vector3();
  const underColor = new Color();
  let smoothY = still ? 0 : window.scrollY;
  let lastBob = -1;
  // Which side of the surface the camera was on, for the crossing ripple (D14 S1).
  let lastAbove: boolean | null = null;
  let ema = 1 / 60;
  let slow = 0;
  let state: SceneState = sceneStateAt(0, layout);

  function applyView(wHero: number, wDepths: number): void {
    const wUnder = Math.max(0, 1 - wHero - wDepths);
    const fin = view === "final";
    const ax = fin ? anchorFinal.x : wHero * anchorHero.x + wDepths * anchorDepths.x + wUnder * anchorUnder.x;
    const ay = fin ? anchorFinal.y : wHero * anchorHero.y + wDepths * anchorDepths.y + wUnder * anchorUnder.y;
    // Only rebuild the projection when the offset really moved.
    const key = `${Math.round(ax * 1000)}:${Math.round(ay * 1000)}`;
    if (key === viewKey) return;
    viewKey = key;
    if (ax !== 0 || ay !== 0) camera.setViewOffset(width, height, (-ax * width) / 2, (ay * height) / 2, width, height);
    else camera.clearViewOffset();
    camera.updateProjectionMatrix();
  }

  function update(dt: number): void {
    const t = clock.t;
    const scrollY = still ? 0 : window.scrollY;
    // Critically damped follow: wheel notches glide instead of stepping.
    smoothY += (scrollY - smoothY) * (1 - Math.exp(-dt * 7));
    if (Math.abs(scrollY - smoothY) < 0.05) smoothY = scrollY;
    // O53: the final call is the hero's pose, whatever the scroll.
    state = sceneStateAt(view === "final" ? 0 : smoothY, layout);
    const uw = state.underwater;
    if (lab.on && !still) {
      // Lab: ease the creature's framing to its goal (about 250 ms).
      const glide = 1 - Math.exp(-dt * 11);
      anchorHero.x += (goal.x - anchorHero.x) * glide;
      anchorHero.y += (goal.y - anchorHero.y) * glide;
      distScaleHero += (goal.d - distScaleHero) * glide;
      if (Math.abs(goal.d - distScaleHero) < 1e-4) distScaleHero = goal.d;
      aimSun();
    }

    // Camera: the scroll path, plus a small pointer parallax while above the water.
    const heroPointer = 1 - state.dive;
    const px = pointer.seen ? pointer.x : 0;
    const py = pointer.seen ? pointer.y : 0;
    look.set(state.look[0], state.look[1], state.look[2]);
    camera.position.set(
      state.position[0] + px * 0.35 * heroPointer,
      state.position[1] + py * 0.12 * heroPointer,
      state.position[2],
    );
    // On a narrow screen the camera stays pulled back so the creature, and later
    // the slabs, fit the width.
    const dist = view === "final" ? distScaleFinal : lerp(distScaleHero, distScaleDepths, state.depthsWeight);
    camera.position.sub(look).multiplyScalar(dist).add(look);
    camera.lookAt(look);
    applyView(state.heroWeight, state.depthsWeight);

    // Where the creature is on screen decides what "looking at the pointer" means.
    // The camera must be placed first: projecting through the default camera at
    // the origin divides by zero and a NaN aim would stick in the eye smoothing.
    camera.updateMatrixWorld();
    mascotNdc.set(0, MASCOT_H * 0.45, 0).project(camera);
    if (!Number.isFinite(mascotNdc.x) || !Number.isFinite(mascotNdc.y)) mascotNdc.set(0, 0, 0);
    const idle = clamp((t - pointer.lastInput - 3) / 1.5, 0, 1);
    const wanderX = Math.sin(t * 0.37) * 0.7 + Math.sin(t * 0.91) * 0.15;
    const wanderY = Math.sin(t * 0.29 + 1.3) * 0.28;
    let rawX: number;
    let rawY: number;
    if (still) {
      // Reduced motion: the eye rests, looking slightly towards the copy.
      rawX = -0.1;
      rawY = 0.04;
    } else if (tilt.active) {
      rawX = tilt.x;
      rawY = tilt.y;
    } else if (pointer.seen) {
      rawX = clamp((pointer.x - mascotNdc.x) * 1.1, -1, 1);
      rawY = clamp((pointer.y - mascotNdc.y) * 1.3, -1, 1);
    } else {
      rawX = wanderX;
      rawY = wanderY;
    }
    const w = still ? 0 : idle;
    // Under the surface the eye looks down at the visitor sinking past it.
    aim.x = lerp(rawX + (wanderX - rawX) * w, clamp(camera.position.x * 0.3, -0.5, 0.5), uw);
    aim.y = lerp(rawY + (wanderY - rawY) * w, -0.8, uw);
    mascot.swim = uw;
    mascot.update(dt, t, aim, 1 - uw * 0.5);
    (water.uniforms.uBodyC.value as Vector3).set(mascot.group.position.x, mascot.group.position.y, 0);

    // WEB-5 D14 S1: one ring spreads from the crossing as the camera goes
    // through the surface (either way), so the moment reads as a dive.
    const above = camera.position.y >= 0;
    if (lastAbove !== null && above !== lastAbove) water.spawnRipple(t, 0.5);
    lastAbove = above;

    // One ring per bob, from the waterline at the trough.
    const bobIndex = Math.floor((t - 3) / 4);
    if (bobIndex !== lastBob) {
      if (lastBob !== -1) water.spawnRipple(t, 0.34);
      lastBob = bobIndex;
    }

    // The water's colour by depth (O5): lake-blue near the surface, abyss at
    // the bottom; lighter looking up, where the light comes from, and darker
    // looking down. Used only by the picture below the water.
    const depth = Math.max(0, -camera.position.y);
    const depthK = clamp((depth - 0.5) / 26, 0, 1);
    if (depthK < 0.35) underColor.copy(UNDER_NEAR).lerp(UNDER_MID, depthK / 0.35);
    else underColor.copy(UNDER_MID).lerp(UNDER_DEEP, (depthK - 0.35) / 0.65);
    fog.uWfHorizon.value.copy(underColor);
    fog.uWfUp.value.copy(underColor).lerp(UNDER_LIGHT, 0.55 * Math.exp(-depth / 12));
    fog.uWfDown.value.copy(underColor).multiplyScalar(0.42);
    fog.uWfDensity.value = lerp(0.034, 0.042, depthK);
    water.uniforms.uFogDensity.value = lerp(0.021, 0.013, dayK);

    sky.position.copy(camera.position);
    stars.points.position.copy(camera.position);
    stars.uniforms.uTime.value = t;
    water.uniforms.uTime.value = t;
    fog.uWfTime.value = t;

    // Light shafts fade out with depth (and in their own shader with their
    // own depth); snow is a constant presence below.
    shafts.update(t, 0.75 * Math.exp(-depth / 16), camera);
    snow.update(t, 0.75, camera, dpr);
    // O40: the framing eases to the card's pedestal; the focused slab takes
    // the pedestal, which the camera's look point carries (all slab keys
    // frame it alike), clamped to the stack so the approach does not drag it.
    const ease = 1 - Math.exp(-dt * 5);
    anchorDepths.x += (depthsTarget.x - anchorDepths.x) * ease;
    anchorDepths.y += (depthsTarget.y - anchorDepths.y) * ease;
    const pedestalY = clamp(state.look[1] + 0.3, SLAB_Y[2], SLAB_Y[0]);
    slabs.update({ time: t, focus: state.focus, reveal: state.slabsReveal, pixelRatio: dpr, pedestalY });

    // WEB-1 review round 1 SHOULD-FIX 2: the canvas layer is sticky only inside
    // the zone, so where S3 ends it used to be cut by a hard horizontal edge
    // across the Gold slab. Fade it to the page's abyss over the last part of
    // the pinned range so it is gone before that edge arrives. (On the canvas
    // itself, not its layer: the layer has its own opacity transition.)
    if (!still) canvas.style.opacity = view === "final" ? "1" : String(1 - smoothstep(0.82, 1, state.depthProgress));
  }

  /** Sets the scene up as it looks above the water, or below it (O5). */
  function configure(under: boolean): void {
    const y = camera.position.y;
    water.mesh.position.y = under ? Math.max(0, y + PLANE_CLEAR) : Math.min(0, y - PLANE_CLEAR);
    fog.uWfOn.value = under ? 1 : 0;
    skyUniforms.uUnder.value = under ? 1 : 0;
    stars.points.visible = !under && dayK < 0.999;
    shafts.group.visible = under;
    snow.points.visible = under;
    slabs.group.visible = under;
    mascot.foam.visible = !under;
    mascot.setUnderwater(under ? 1 : 0);
    if (under) {
      moonLight.position.set(...UNDER_MOON).multiplyScalar(10);
      moonLight.intensity = 2.2;
      ambient.intensity = 1.1;
    } else {
      moonLight.position.copy(moonDir).multiplyScalar(10);
      moonLight.intensity = lerp(3.2, 2.7, dayK);
      ambient.intensity = lerp(0.8, 1.15, dayK);
    }
  }

  // The water's colour at the top and the bottom of the screen, as the lens
  // puts it on screen, for the page-wide water that takes over where the
  // zone ends (O5, lib/scene-colour.ts). The far water is the fog alone.
  const fogAt = (dy: number, out: Color): Color => {
    out.copy(fog.uWfDown.value).lerp(fog.uWfHorizon.value, smoothstep(-0.5, 0.06, dy));
    return out.lerp(fog.uWfUp.value, smoothstep(0, 0.8, dy));
  };
  const camUp = new Vector3();
  const camFwd = new Vector3();
  const edgeColour = new Color();
  function publishColours(): void {
    const half = MathUtils.degToRad(CAMERA_FOV / 2);
    camera.getWorldDirection(camFwd);
    camUp.setFromMatrixColumn(camera.matrixWorld, 1);
    const hex = (dy: number) => {
      fogAt(dy, edgeColour);
      return displayedHex(edgeColour.r, edgeColour.g, edgeColour.b, renderer.toneMappingExposure);
    };
    publishSceneColours({
      top: hex(camFwd.y * Math.cos(half) + camUp.y * Math.sin(half)),
      bottom: hex(camFwd.y * Math.cos(half) - camUp.y * Math.sin(half)),
    });
  }

  const right = new Vector3();
  let shown = false;
  function renderFrame(): void {
    // O53: once a frame exists, the final call shows the lake in place of its 2D scene.
    if (!shown && finalSection && finalHost) {
      shown = true;
      finalSection.dataset.scene3d = "on";
    }
    const cover = lensCoverage(camera.position.y);
    const lensMode = cover.above && cover.below ? 2 : cover.above ? 0 : 1;
    configure(lensMode === 1);
    renderer.setRenderTarget(lens.main);
    renderer.render(scene, camera);
    if (lensMode === 1) publishColours();
    if (lensMode === 2) {
      configure(true);
      renderer.setRenderTarget(lens.second());
      renderer.render(scene, camera);
    }
    right.setFromMatrixColumn(camera.matrixWorld, 0);
    lens.present(renderer, {
      mode: lensMode,
      time: clock.t,
      camY: camera.position.y,
      camX: camera.position.x,
      camZ: camera.position.z,
      rightX: right.x,
      rightZ: right.z,
      wobble: still ? 0 : 1,
    });
  }

  function renderOnceIfStill(): void {
    if (!still) return;
    // The camera's matrices are built in `update`; run one with no elapsed time.
    update(1 / 60);
    for (let i = 0; i < 30; i++) mascot.update(1 / 60, clock.t, aim, 0);
    renderFrame();
  }

  // WEB-5 O52: the first-load intro, played in this scene when the page
  // chose it for this device (hero-intro.tsx puts `lk-intro-3d` on <html>
  // before the first paint, and holds the DOM hero hidden). The whale waits
  // under the surface; on this scene's first frame the page is told to start
  // (the open sea shows as the canvas fades in, the text follows from 1.6 s);
  // 0.9 s in, the whale rises. If the intro is skipped or ends, or the page
  // fell back to the DOM intro before this scene was ready, it is put at
  // the surface at once.
  const root = document.documentElement;
  let intro3d = !still && root.classList.contains("lk-intro-3d");
  let introStart = -1;
  if (intro3d) mascot.submerge();
  const tickIntro = () => {
    if (!intro3d) return;
    if (introStart < 0) {
      introStart = clock.t;
      if (root.classList.contains("lk-intro-3d")) {
        (window as unknown as { __lkIntroGo?: () => void }).__lkIntroGo?.();
      }
    }
    if (!root.classList.contains("lk-intro-3d")) {
      mascot.surface();
      intro3d = false;
    } else if (clock.t - introStart >= 0.9) {
      mascot.rise(clock.t);
    }
  };

  let raf = 0;
  let onScreen = true;
  const running = () => !still && onScreen && !document.hidden;
  const loop = (now: number) => {
    raf = 0;
    if (!running()) return;
    raf = requestAnimationFrame(loop);
    const dt = Math.min((now - clock.last) / 1000, 0.05);
    clock.last = now;
    if (dt <= 0) return;
    clock.t += dt;
    update(dt);
    renderFrame();
    tickIntro();

    // Adaptive quality: after about 1.5 s of frames slower than 25 fps, drop
    // the pixel ratio one step (never below 1).
    ema += (dt - ema) * 0.05;
    slow = ema > 0.04 ? slow + 1 : 0;
    if (slow > 90 && dpr > 1) {
      dpr = Math.max(1, dpr - 0.25);
      renderer.setPixelRatio(dpr);
      stars.uniforms.uPx.value = dpr;
      renderer.setSize(width, height, false);
      lens.setSize(width, height, dpr);
      mascot.setViewport(height * dpr, dpr);
      slow = 0;
      ema = 1 / 60;
    }
  };
  const start = () => {
    if (raf || !running()) return;
    clock.last = performance.now();
    // After a pause the scroll may have moved far; do not glide across it.
    smoothY = window.scrollY;
    raf = requestAnimationFrame(loop);
  };

  // O53: the loop runs while the hero zone or the final call is on screen,
  // and sleeps in between (no frame is requested at all).
  let heroOn = true;
  let finalOn = false;
  const io = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (e.target === observe) heroOn = e.isIntersecting;
        else finalOn = e.isIntersecting;
      }
      onScreen = heroOn || finalOn;
      if (onScreen) start();
    },
    { rootMargin: "0px" },
  );
  io.observe(observe);
  if (finalSection && finalHost) io.observe(finalSection);

  // O53: the canvas goes to whichever of the two is near, a quarter of a
  // screen before it shows, so it is in place when it scrolls in. They are
  // many screens apart, so both are never near at once.
  const setView = (next: "hero" | "final") => {
    if (next === view) return;
    const target = next === "final" ? finalHost : heroHost;
    if (!target) return;
    view = next;
    host = target;
    target.appendChild(canvas);
    if (next === "hero") moonDir.set(...MOON_DIR);
    // Coming back from the final call, the camera does not glide down the whole dive.
    smoothY = window.scrollY;
    resize();
  };
  let nearHero = true;
  let nearFinal = false;
  const ioNear = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (e.target === observe) nearHero = e.isIntersecting;
        else nearFinal = e.isIntersecting;
      }
      if (nearFinal && !nearHero) setView("final");
      else if (nearHero) setView("hero");
    },
    { rootMargin: "25% 0px" },
  );
  if (finalSection && finalHost) {
    ioNear.observe(observe);
    ioNear.observe(finalSection);
  }
  const onVisibility = () => {
    if (!document.hidden) start();
  };
  document.addEventListener("visibilitychange", onVisibility);

  const onLost = (e: Event) => {
    e.preventDefault();
    opts.onContextLost();
  };
  canvas.addEventListener("webglcontextlost", onLost);

  // Still frame: a ring already out on the water, then a single render.
  if (still) water.spawnRipple(STILL_TIME - 2.2, 0.55);
  resize();
  started = true;
  if (!still) start();

  return {
    setTimeOfDay(day) {
      applyDay(clamp(day, 0, 1));
      renderOnceIfStill();
    },
    setLab(next) {
      watchHeroEls();
      lab.on = true;
      lab.box = next.box;
      lab.sun = next.sun;
      resize();
    },
    refit() {
      watchHeroEls();
      resize();
    },
    dispose() {
      if (raf) cancelAnimationFrame(raf);
      raf = 0;
      publishSceneColours(null);
      ro.disconnect();
      roZone.disconnect();
      roCard.disconnect();
      io.disconnect();
      ioNear.disconnect();
      if (finalSection) delete finalSection.dataset.scene3d;
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("pointermove", setPointer);
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("deviceorientation", onTilt);
      canvas.removeEventListener("webglcontextlost", onLost);
      mascot.dispose();
      water.dispose();
      shafts.dispose();
      snow.dispose();
      slabs.dispose();
      sky.geometry.dispose();
      (sky.material as Material).dispose();
      stars.points.geometry.dispose();
      (stars.points.material as Material).dispose();
      envTarget.dispose();
      lens.dispose();
      renderer.dispose();
      // Release the GPU context now rather than at garbage collection, unless
      // the browser already took it (calling this on a lost context only warns).
      if (!renderer.getContext().isContextLost()) renderer.forceContextLoss();
    },
  };
}
