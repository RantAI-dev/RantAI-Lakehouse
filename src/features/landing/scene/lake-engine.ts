import * as THREE from "three";
import { sceneStateAt, smoothstep, type SceneLayout, type SceneState } from "../scroll-timeline";
import { createMascot, type Aim } from "./mascot";
import { CAMERA_FOV, MASCOT_H, MOON_DIR, STILL_TIME } from "./scene-config";
import { buildEnvironment, createSkyDome, createSkyUniforms, createStars } from "./sky";
import { createSlabs, SLAB_SIZE } from "./slabs";
import { createShafts, createSnow } from "./underwater";
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

export type LakeEngine = { dispose: () => void };

const clamp = THREE.MathUtils.clamp;
const lerp = THREE.MathUtils.lerp;

/** Fog and background by depth: lake-blue near the surface, abyss at the bottom. */
const UNDER_NEAR = new THREE.Color("#0b3a74");
const UNDER_MID = new THREE.Color("#071f52");
const UNDER_DEEP = new THREE.Color("#020b2a");

export function createLakeEngine(opts: EngineOptions): LakeEngine {
  const { canvas, mode, observe } = opts;
  const host = canvas.parentElement ?? canvas; // the layer div the page sizes
  const still = mode === "still";
  let phone = window.innerWidth < 768;

  const maxDpr = phone ? 1.25 : 1.75;
  let dpr = Math.min(window.devicePixelRatio || 1, maxDpr);

  const renderer = new THREE.WebGLRenderer({
    canvas,
    antialias: !phone,
    powerPreference: "high-performance",
  });
  renderer.setPixelRatio(dpr);
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = 1.0;

  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(CAMERA_FOV, 1, 0.05, 600);

  // ---- Sky, stars, environment ------------------------------------------
  const skyUniforms = createSkyUniforms();
  const sky = createSkyDome(skyUniforms);
  scene.add(sky);
  const stars = createStars(phone ? 300 : 600, dpr);
  scene.add(stars.points);
  const envTarget = buildEnvironment(renderer, skyUniforms);
  scene.environment = envTarget.texture;
  scene.environmentIntensity = 0.95;

  // One fog colour object shared by the background, three's fog (for the
  // mascot) and the water shader, so they can never disagree at the horizon.
  const fogColor = new THREE.Color().copy(skyUniforms.uSkyHorizon.value);
  scene.background = fogColor;
  scene.fog = new THREE.FogExp2(fogColor, 0.003);
  skyUniforms.uFogColor.value = fogColor;

  // ---- Water -------------------------------------------------------------
  const water = createWater(skyUniforms, phone);
  water.uniforms.uFogColor.value = fogColor;
  scene.add(water.mesh);

  // ---- Mascot ------------------------------------------------------------
  const mascot = createMascot({
    pixelRatio: dpr,
    onRipple: (amp) => water.spawnRipple(clock.t, amp),
  });
  mascot.still = still;
  scene.add(mascot.group);
  scene.add(mascot.extras);

  // ---- Underwater --------------------------------------------------------
  const shafts = createShafts(phone ? 5 : 9);
  scene.add(shafts.group);
  const snow = createSnow(phone ? 700 : 1500);
  scene.add(snow.points);

  // ---- Medallion slabs ----------------------------------------------------
  const slabs = createSlabs(phone);
  scene.add(slabs.group);

  // ---- Lights ------------------------------------------------------------
  // Moonlight from behind the creature rims its dome. There is deliberately no
  // front fill: a directional light in front of a clearcoated dome shows up as
  // a hot spot on its face. The face is lit by the ambient term, the water
  // bounce (a hemisphere light, diffuse only) and the eye's own glow.
  const moonLight = new THREE.DirectionalLight(0xbcd8ff, 3.2);
  moonLight.position.set(...MOON_DIR).multiplyScalar(10);
  scene.add(moonLight);
  scene.add(new THREE.AmbientLight(0x1b2f7a, 0.8));

  const clock = { t: still ? STILL_TIME : 0, last: 0 };

  // ---- Input -------------------------------------------------------------
  const pointer = { x: 0, y: 0, seen: false, lastInput: -100 };
  const tilt = { x: 0, y: 0, active: false };
  const raycaster = new THREE.Raycaster();
  const ndc = new THREE.Vector2();

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
    // Only the hero's own screen area counts as "the dome".
    if (still || window.scrollY > layout.vh * 0.5 || isInteractive(e.target)) return;
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
    layout.vh = host.clientHeight || window.innerHeight;
    layout.viewportH = window.innerHeight;
    const depths = document.getElementById("depths");
    if (depths) {
      layout.depthsTop = depths.getBoundingClientRect().top + window.scrollY;
      layout.depthsHeight = depths.offsetHeight;
    }
  };

  const anchorHero = { x: 0, y: 0 };
  const anchorDepths = { x: 0, y: 0 };
  let distScaleHero = 1;
  let distScaleDepths = 1;
  let viewKey = "";
  let width = 1;
  let height = 1;
  const resize = () => {
    width = Math.max(1, host.clientWidth);
    height = Math.max(1, host.clientHeight);
    phone = window.innerWidth < 768;
    renderer.setSize(width, height, false);
    const aspect = width / height;
    camera.aspect = aspect;
    // Desktop: the creature sits right of centre so the copy has the left.
    // Portrait: it sits in the upper part, above the centred copy.
    const wide = aspect > 1.1;
    anchorHero.x = wide ? 0.42 : 0;
    anchorHero.y = wide ? 0 : 0.42;
    anchorDepths.x = wide ? 0.36 : 0;
    anchorDepths.y = wide ? -0.1 : -0.5;
    const tanV = Math.tan(THREE.MathUtils.degToRad(CAMERA_FOV / 2));
    // Pull the camera back until the dome fits the width of a narrow screen.
    distScaleHero = Math.max(1, 2.35 / (tanV * aspect) / 7.2);
    // The slabs shrink sideways to fit a narrow screen rather than pushing the
    // camera away, which the fog would swallow.
    distScaleDepths = wide ? 1.25 : 1;
    slabs.setFootprint(Math.min(1, ((wide ? 0.84 : 0.7) * 8.06 * aspect * distScaleDepths) / SLAB_SIZE.w));
    viewKey = "";
    measure();
    camera.updateProjectionMatrix();
    renderOnceIfStill();
  };
  const ro = new ResizeObserver(resize);
  ro.observe(host);
  // The zone's own height changes when fonts load or sections reflow.
  const roZone = new ResizeObserver(measure);
  roZone.observe(observe);

  // ---- Frame -------------------------------------------------------------
  const aim: Aim = { x: 0, y: 0 };
  const mascotNdc = new THREE.Vector3();
  const look = new THREE.Vector3();
  const underColor = new THREE.Color();
  let smoothY = still ? 0 : window.scrollY;
  let lastBob = -1;
  let ema = 1 / 60;
  let slow = 0;
  let state: SceneState = sceneStateAt(0, layout);

  function applyView(wHero: number, wDepths: number): void {
    const ax = wHero * anchorHero.x + wDepths * anchorDepths.x;
    const ay = wHero * anchorHero.y + wDepths * anchorDepths.y;
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
    state = sceneStateAt(smoothY, layout);
    const uw = state.underwater;

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
    camera.position.sub(look).multiplyScalar(lerp(distScaleHero, distScaleDepths, state.depthsWeight)).add(look);
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
    mascot.update(dt, t, aim, 1 - uw * 0.5);
    mascot.foam.visible = uw < 0.95;

    // One ring per bob, from the waterline at the trough.
    const bobIndex = Math.floor((t - 3) / 4);
    if (bobIndex !== lastBob) {
      if (lastBob !== -1) water.spawnRipple(t, 0.34);
      lastBob = bobIndex;
    }

    // Fog, background and the underwater colour ramp by depth.
    const depthK = clamp((-camera.position.y - 0.5) / 26, 0, 1);
    if (depthK < 0.35) underColor.copy(UNDER_NEAR).lerp(UNDER_MID, depthK / 0.35);
    else underColor.copy(UNDER_MID).lerp(UNDER_DEEP, (depthK - 0.35) / 0.65);
    fogColor.copy(skyUniforms.uSkyHorizon.value).lerp(underColor, smoothstep(0, 1, uw));
    const density = lerp(0.021, 0.036, uw);
    (scene.fog as THREE.FogExp2).density = lerp(0.003, 0.036, uw);
    (scene.fog as THREE.FogExp2).color.copy(fogColor);
    water.uniforms.uFogDensity.value = density;
    skyUniforms.uUnder.value = smoothstep(0, 1, uw);
    mascot.setUnderwater(uw);
    stars.points.visible = uw < 0.98;
    stars.uniforms.uAlpha.value = 1 - uw;

    sky.position.copy(camera.position);
    stars.points.position.copy(camera.position);
    stars.uniforms.uTime.value = t;
    water.uniforms.uTime.value = t;

    // Light shafts fade out with depth; snow is a constant presence below.
    const below = Math.max(0, -camera.position.y);
    shafts.update(t, uw * 0.55 * Math.exp(-below / 14), camera);
    snow.update(t, uw * 0.85, camera, dpr);
    slabs.update({ time: t, focus: state.focus, reveal: state.slabsReveal, pixelRatio: dpr });

    // WEB-1 review round 1 SHOULD-FIX 2: the canvas layer is sticky only inside
    // the zone, so where S3 ends it used to be cut by a hard horizontal edge
    // across the Gold slab. Fade it to the page's abyss over the last part of
    // the pinned range so it is gone before that edge arrives. (On the canvas
    // itself, not its layer: the layer has its own opacity transition.)
    if (!still) canvas.style.opacity = String(1 - smoothstep(0.82, 1, state.depthProgress));
  }

  function renderFrame(): void {
    renderer.render(scene, camera);
  }

  function renderOnceIfStill(): void {
    if (!still) return;
    // The camera's matrices are built in `update`; run one with no elapsed time.
    update(1 / 60);
    for (let i = 0; i < 30; i++) mascot.update(1 / 60, clock.t, aim, 0);
    renderFrame();
  }

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

    // Adaptive quality: after about 1.5 s of frames slower than 25 fps, drop
    // the pixel ratio one step (never below 1).
    ema += (dt - ema) * 0.05;
    slow = ema > 0.04 ? slow + 1 : 0;
    if (slow > 90 && dpr > 1) {
      dpr = Math.max(1, dpr - 0.25);
      renderer.setPixelRatio(dpr);
      stars.uniforms.uPx.value = dpr;
      renderer.setSize(width, height, false);
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

  const io = new IntersectionObserver(
    (entries) => {
      onScreen = entries.some((e) => e.isIntersecting);
      if (onScreen) start();
    },
    { rootMargin: "0px" },
  );
  io.observe(observe);
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
  if (!still) start();

  return {
    dispose() {
      if (raf) cancelAnimationFrame(raf);
      raf = 0;
      ro.disconnect();
      roZone.disconnect();
      io.disconnect();
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
      (sky.material as THREE.Material).dispose();
      stars.points.geometry.dispose();
      (stars.points.material as THREE.Material).dispose();
      envTarget.dispose();
      renderer.dispose();
      // Release the GPU context now rather than at garbage collection, unless
      // the browser already took it (calling this on a lost context only warns).
      if (!renderer.getContext().isContextLost()) renderer.forceContextLoss();
    },
  };
}
