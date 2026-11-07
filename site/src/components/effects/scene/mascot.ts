// WEB-5 D5: named imports, so the scene chunk carries only what it uses.
import {
  AdditiveBlending,
  BufferAttribute,
  BufferGeometry,
  Color,
  Group,
  HemisphereLight,
  LatheGeometry,
  MathUtils,
  Matrix4,
  Mesh,
  MeshBasicMaterial,
  MeshPhysicalMaterial,
  PlaneGeometry,
  Points,
  ShaderMaterial,
  SphereGeometry,
  Vector2,
  Vector3,
} from "three";
import { PALETTE } from "@/lib/palette";
import { DOME_SPHERE, EYES_3D, HEAD_LONG, HEAD_TURN, MASCOT_H, MASCOT_R } from "./scene-config";
import { FOAM_FRAG, GLOW_FRAG, GLOW_VERT, SPLASH_FRAG, SPLASH_VERT, SPOUT_FRAG, SPOUT_VERT } from "./shaders";

/**
 * The 3D mascot, built from primitives on the official mark's shapes
 * (`components/shared/mascot-svg.tsx`, WEB-5 O2): a navy dome, the mark's
 * sphere cap revolved, with two white eyes (O3: the mark is the creature
 * seen from the side, with one eye; facing the camera it has two).
 *
 * WEB-5 O8: it is a whale, and the dome is its head breaking the surface:
 * - stretched front to back (`HEAD_LONG`), the eyes low on its sides near
 *   the waterline, small beads that stay round at an angle;
 * - the eyes follow the pointer in their sockets, and the head turns only
 *   a little, so both eyes stay on the face from the hero camera;
 * - every 5 to 9 seconds it breathes out through its blowhole: a short,
 *   soft spout of droplets and mist from the top of the head.
 * The water plane hides everything below the waterline until the camera
 * dives.
 */

export type Aim = { x: number; y: number };

type MascotOptions = {
  /** Called when the mascot disturbs the water; `amp` is 0.3 (bob) to 1 (pop). */
  onRipple: (amp: number) => void;
  pixelRatio: number;
};

export type Mascot = {
  /** Everything that moves with the creature. Raycast target is `dome`. */
  group: Group;
  dome: Mesh;
  /** The body's material: the engine gives it the water's fog and caustics. */
  skin: MeshPhysicalMaterial;
  /** Foam collar and splash, which live in world space at the waterline. */
  extras: Group;
  /** The foam collar alone, hidden when the camera is below the surface. */
  foam: Mesh;
  update: (dt: number, t: number, aim: Aim, look: number) => void;
  dip: (t: number) => void;
  /** 0 for the picture above the surface, 1 for the one below (lighting, the spout). */
  setUnderwater: (uw: number) => void;
  /** The drawing buffer's height in pixels, so the spout's droplets keep their true size. */
  setViewport: (heightPx: number, pixelRatio: number) => void;
  /** Freeze the idle motion (reduced motion). */
  still: boolean;
  dispose: () => void;
};

const TAU = Math.PI * 2;
const { r: SPHERE_R, cy: SPHERE_CY } = DOME_SPHERE;
/** The angle, seen from the sphere's centre, of the dome's flat base (the waterline): below the equator. */
const BASE_T = -Math.asin(SPHERE_CY / SPHERE_R);

/** Upper dome profile at parameter u in [0, 1]: from the waterline (r = MASCOT_R) to the top. */
function profile(u: number): { r: number; y: number } {
  const t = BASE_T + u * (Math.PI / 2 - BASE_T);
  return { r: SPHERE_R * Math.cos(t), y: SPHERE_CY + SPHERE_R * Math.sin(t) };
}

function buildBodyGeometry(upperOnly = false): LatheGeometry {
  const pts: Vector2[] = [];
  // Submerged half: a plain ellipse, reached only when the camera is below.
  const below = 0.9 * MASCOT_R;
  if (upperOnly) pts.push(new Vector2(0, 0), new Vector2(MASCOT_R, 0));
  else
    for (let i = 0; i <= 24; i++) {
      const s = (i / 24) * (Math.PI / 2);
      pts.push(new Vector2(MASCOT_R * Math.sin(s), -below * Math.cos(s)));
    }
  // Visible dome: the sphere cap, evenly spaced along its arc.
  for (let i = 1; i <= 72; i++) {
    const p = profile(i / 72);
    pts.push(new Vector2(p.r, p.y));
  }
  const geo = new LatheGeometry(pts, upperOnly ? 64 : 112);
  // O8: a whale's head is longer than it is wide; the front (+z) still
  // faces the camera, so the mark's silhouette is unchanged from there.
  geo.scale(1, 1, HEAD_LONG);
  return geo;
}

/** Point and outward normal on the dome for azimuth `phi` and height fraction `rho` of the dome. */
function surface(phi: number, rho: number, out: { pos: Vector3; normal: Vector3 }): void {
  // On a sphere the outward normal is the direction from its centre.
  const y = MASCOT_H * MathUtils.clamp(rho, 0.02, 0.98);
  const sinT = (y - SPHERE_CY) / SPHERE_R;
  const cosT = Math.sqrt(Math.max(0, 1 - sinT * sinT));
  const r = SPHERE_R * cosT;
  // The head is the sphere stretched along z: positions stretch with it,
  // normals by the inverse.
  out.pos.set(r * Math.sin(phi), y, r * Math.cos(phi) * HEAD_LONG);
  out.normal.set(cosT * Math.sin(phi), sinT, (cosT * Math.cos(phi)) / HEAD_LONG).normalize();
}

/** The blowhole, on top of the head a little behind its crown (group space). */
const BLOWHOLE = new Vector3(0, MASCOT_H - 0.03, -0.32);
const SPOUT_DROPS = 46;
const SPOUT_MIST = 34;

const SPLASH_COUNT = 56;

export function createMascot(opts: MascotOptions): Mascot {
  const group = new Group();
  const extras = new Group();
  const disposables: { dispose: () => void }[] = [];

  // ---- Body --------------------------------------------------------------
  const bodyGeo = buildBodyGeometry();
  const bodyMat = new MeshPhysicalMaterial({
    color: new Color(PALETTE.navy),
    roughness: 0.42,
    metalness: 0.0,
    clearcoat: 1,
    clearcoatRoughness: 0.2,
    // A little emission keeps the shadow side navy rather than black.
    emissive: new Color("#0a1450"),
    emissiveIntensity: 0.55,
    envMapIntensity: 1.35,
  });
  const dome = new Mesh(bodyGeo, bodyMat);
  group.add(dome);
  disposables.push(bodyGeo, bodyMat);

  // ---- Eyes --------------------------------------------------------------
  // WEB-5 O3: two eyes, one each side of the front, built alike and sharing
  // their materials, so they glow, blink and fade under water as one.
  const eyeGeo = new SphereGeometry(1, 40, 28);
  // The logo's eye is a white cut-out. Since O5 the whole frame is tone
  // mapped once, in the lens pass, so "white" is set bright enough (6 in
  // linear light) that ACES maps it to about 0.99, not light grey.
  const eyeMat = new MeshBasicMaterial({ color: new Color(1, 1, 1).multiplyScalar(6) });
  const glowGeo = new PlaneGeometry(1, 1);
  const glowMat = new ShaderMaterial({
    uniforms: { uColor: { value: new Color("#bfe0ff") }, uIntensity: { value: 0.5 } },
    vertexShader: GLOW_VERT,
    fragmentShader: GLOW_FRAG,
    transparent: true,
    depthWrite: false,
    blending: AdditiveBlending,
  });
  disposables.push(eyeGeo, eyeMat, glowGeo, glowMat);
  /**
   * The halo's size, in eye radii. The single eye's was 6.5 of its width;
   * two halos that wide would merge into one blur between the eyes, so each
   * is smaller and they only just touch.
   */
  const GLOW = 4.2;
  function buildEye() {
    const eye = new Group();
    const eyeMesh = new Mesh(eyeGeo, eyeMat);
    // A bead (O8): an ellipsoid sunk into the head, so its visible part is
    // round from any side rather than a disc that turns to an oval.
    eyeMesh.scale.set(EYES_3D.r, EYES_3D.r, 0.9 * EYES_3D.r);
    eyeMesh.position.z = -0.62 * EYES_3D.r;
    eye.add(eyeMesh);
    const glow = new Mesh(glowGeo, glowMat);
    glow.scale.set(EYES_3D.r * GLOW, EYES_3D.r * GLOW, 1);
    glow.position.z = 0.12 * MASCOT_R;
    glow.renderOrder = 3;
    eye.add(glow);
    group.add(eye);
    return { eye, eyeMesh, glow };
  }
  /** Left and right, by their side of the front (−phi, +phi). */
  const eyes = [buildEye(), buildEye()] as const;
  const EYE_SIDE = [-1, 1] as const;

  // ---- Lights that belong to the creature --------------------------------
  // Water bounce from below-front, so the lower dome is not a black cut-out.
  // A hemisphere light is diffuse only, so it tints the dome without adding a
  // second hot spot to the clearcoat the way a point light would.
  const bounce = new HemisphereLight(0x1b2f7a, new Color(PALETTE.lake), 0.9);
  extras.add(bounce);
  // Under water the light comes the other way: down from the surface, with
  // the dark depths below (O5), so the submerged body is lit from above.
  const BOUNCE_ABOVE = { sky: new Color(0x1b2f7a), ground: new Color(PALETTE.lake) };
  const BOUNCE_UNDER = { sky: new Color("#3f86d6"), ground: new Color("#04102e") };

  // ---- Foam collar -------------------------------------------------------
  const foamSize = MASCOT_R * 2 * 1.75;
  const foamGeo = new PlaneGeometry(foamSize, foamSize);
  foamGeo.rotateX(-Math.PI / 2);
  const foamMat = new ShaderMaterial({
    uniforms: {
      uTime: { value: 0 },
      uAmount: { value: 0.45 },
      uColor: { value: new Color("#7fb8f5") },
      uInner: { value: 1 / 1.75 },
    },
    vertexShader: GLOW_VERT,
    fragmentShader: FOAM_FRAG,
    transparent: true,
    depthWrite: false,
    blending: AdditiveBlending,
  });
  const foam = new Mesh(foamGeo, foamMat);
  foam.position.y = 0.02;
  foam.scale.z = HEAD_LONG;
  foam.renderOrder = 1;
  extras.add(foam);
  disposables.push(foamGeo, foamMat);

  // ---- Splash ------------------------------------------------------------
  const splashPos = new Float32Array(SPLASH_COUNT * 3);
  const splashVel = new Float32Array(SPLASH_COUNT * 3);
  const splashAlpha = new Float32Array(SPLASH_COUNT);
  const splashGeo = new BufferGeometry();
  splashGeo.setAttribute("position", new BufferAttribute(splashPos, 3));
  splashGeo.setAttribute("aAlpha", new BufferAttribute(splashAlpha, 1));
  const splashMat = new ShaderMaterial({
    uniforms: { uColor: { value: new Color("#cfe6ff") }, uPx: { value: opts.pixelRatio } },
    vertexShader: SPLASH_VERT,
    fragmentShader: SPLASH_FRAG,
    transparent: true,
    depthWrite: false,
    blending: AdditiveBlending,
  });
  const splash = new Points(splashGeo, splashMat);
  splash.frustumCulled = false;
  extras.add(splash);
  disposables.push(splashGeo, splashMat);
  let splashLife = 0;

  function burst(): void {
    splashLife = 1.3;
    for (let i = 0; i < SPLASH_COUNT; i++) {
      const a = Math.random() * TAU;
      const r = MASCOT_R * (0.85 + Math.random() * 0.25);
      splashPos[i * 3] = Math.cos(a) * r;
      splashPos[i * 3 + 1] = 0.05;
      splashPos[i * 3 + 2] = Math.sin(a) * r * HEAD_LONG;
      const up = 2.6 + Math.random() * 3.2;
      const out = 0.5 + Math.random() * 1.1;
      splashVel[i * 3] = Math.cos(a) * out;
      splashVel[i * 3 + 1] = up;
      splashVel[i * 3 + 2] = Math.sin(a) * out;
      splashAlpha[i] = 1;
    }
  }

  // ---- Spout (O8) ------------------------------------------------------------
  // Droplets first, then the mist; each particle has its own birth delay,
  // so one breath is a short burst that rises, hangs and drifts, not a
  // single pop. World space, so the bob does not drag it about.
  const SPOUT_N = SPOUT_DROPS + SPOUT_MIST;
  const spoutPos = new Float32Array(SPOUT_N * 3);
  const spoutVel = new Float32Array(SPOUT_N * 3);
  const spoutAge = new Float32Array(SPOUT_N).fill(-1);
  const spoutLife = new Float32Array(SPOUT_N);
  const spoutBase = new Float32Array(SPOUT_N);
  const spoutSize = new Float32Array(SPOUT_N);
  const spoutAlpha = new Float32Array(SPOUT_N);
  const spoutSoft = new Float32Array(SPOUT_N);
  for (let i = SPOUT_DROPS; i < SPOUT_N; i++) spoutSoft[i] = 1;
  const spoutGeo = new BufferGeometry();
  spoutGeo.setAttribute("position", new BufferAttribute(spoutPos, 3));
  spoutGeo.setAttribute("aSize", new BufferAttribute(spoutSize, 1));
  spoutGeo.setAttribute("aAlpha", new BufferAttribute(spoutAlpha, 1));
  spoutGeo.setAttribute("aSoft", new BufferAttribute(spoutSoft, 1));
  const spoutMat = new ShaderMaterial({
    uniforms: {
      uColor: { value: new Color("#d8ecff") },
      uHalfH: { value: 450 },
      uMinPx: { value: 1.6 * opts.pixelRatio },
    },
    vertexShader: SPOUT_VERT,
    fragmentShader: SPOUT_FRAG,
    transparent: true,
    depthWrite: false,
    blending: AdditiveBlending,
  });
  const spout = new Points(spoutGeo, spoutMat);
  spout.frustumCulled = false;
  spout.renderOrder = 4;
  spout.visible = false;
  extras.add(spout);
  disposables.push(spoutGeo, spoutMat);
  const holeWorld = new Vector3();
  let spoutAlive = false;
  let nextSpout = 2.8;
  let aboveWater = true;

  function breathe(): void {
    group.updateMatrixWorld();
    holeWorld.copy(BLOWHOLE).applyMatrix4(group.matrixWorld);
    // A little variety per breath: a stronger or softer one, leaning a bit.
    const power = 0.85 + Math.random() * 0.3;
    const lean = (Math.random() - 0.5) * 0.3;
    for (let i = 0; i < SPOUT_N; i++) {
      const mist = i >= SPOUT_DROPS;
      const j = i * 3;
      spoutPos[j] = holeWorld.x + (Math.random() - 0.5) * 0.08;
      spoutPos[j + 1] = holeWorld.y;
      spoutPos[j + 2] = holeWorld.z + (Math.random() - 0.5) * 0.08;
      // A whale's breath opens into a bushy plume: the mist leaves fast and
      // fans out, then hangs; the droplets are fine and fall back.
      const spread = mist ? 0.9 : 0.6;
      spoutVel[j] = (Math.random() - 0.5) * spread + lean;
      spoutVel[j + 1] = (mist ? 2.5 + Math.random() * 1.3 : 2.4 + Math.random() * 1.3) * power;
      spoutVel[j + 2] = (Math.random() - 0.5) * spread * 0.6 - 0.12;
      // Negative age is the birth delay: the breath lasts about 0.45 s.
      spoutAge[i] = -(mist ? Math.random() * 0.45 : Math.random() * 0.3);
      spoutLife[i] = mist ? 1.6 + Math.random() * 0.9 : 0.8 + Math.random() * 0.45;
      spoutBase[i] = mist ? 0.18 + Math.random() * 0.16 : 0.018 + Math.random() * 0.022;
    }
    spoutAlive = true;
  }

  function updateSpout(dt: number): void {
    let alive = false;
    for (let i = 0; i < SPOUT_N; i++) {
      if (spoutAge[i] === -1 && spoutLife[i] === 0) continue;
      const age = (spoutAge[i] += dt);
      const life = spoutLife[i];
      if (age > life) {
        spoutAlpha[i] = 0;
        continue;
      }
      alive = true;
      if (age < 0) {
        spoutAlpha[i] = 0;
        continue;
      }
      const j = i * 3;
      const mist = i >= SPOUT_DROPS;
      const p = age / life;
      if (mist) {
        // Mist hangs: drag slows it, the air lifts it a little, it widens.
        const drag = Math.exp(-dt * 1.9);
        spoutVel[j] *= drag;
        spoutVel[j + 1] = spoutVel[j + 1] * drag + 0.05 * dt;
        spoutVel[j + 2] *= drag;
        spoutSize[i] = spoutBase[i] * (1 + 3.2 * Math.sqrt(p));
        spoutAlpha[i] = 0.3 * Math.min(1, age / 0.12) * Math.pow(1 - p, 1.6);
      } else {
        spoutVel[j + 1] -= 5.2 * dt;
        spoutSize[i] = spoutBase[i];
        spoutAlpha[i] = Math.min(1, age / 0.06) * Math.pow(1 - p, 1.4) * 0.75;
      }
      // A light breeze from the left.
      spoutVel[j] += 0.12 * dt;
      spoutPos[j] += spoutVel[j] * dt;
      spoutPos[j + 1] += spoutVel[j + 1] * dt;
      spoutPos[j + 2] += spoutVel[j + 2] * dt;
      // Droplets that fall back into the lake are gone.
      if (spoutPos[j + 1] < 0) spoutAlpha[i] = 0;
    }
    if (!alive) {
      spoutLife.fill(0);
      spoutAge.fill(-1);
    }
    spoutAlive = alive;
    spout.visible = alive && aboveWater;
    if (alive) {
      spoutGeo.attributes.position.needsUpdate = true;
      spoutGeo.attributes.aSize.needsUpdate = true;
      spoutGeo.attributes.aAlpha.needsUpdate = true;
    }
  }

  // ---- Animation state ---------------------------------------------------
  const scratch = { pos: new Vector3(), normal: new Vector3() };
  const m4 = new Matrix4();
  const origin = new Vector3();
  const negNormal = new Vector3();
  const up = new Vector3(0, 1, 0);
  const eyeAim = { x: 0, y: 0 };
  const lookAim = { x: 0, y: 0 };
  let nextBlink = 2.2;
  let blinkStart = -10;
  let dipStart = -10;
  let poppedAt = -10;
  const DIP_DEPTH = MASCOT_H + 0.25;

  const rho0 = EYES_3D.rho;

  function dipOffset(t: number): number {
    const age = t - dipStart;
    if (age < 0 || age > 1.6) return 0;
    if (age < 0.28) {
      const p = age / 0.28;
      return -DIP_DEPTH * (0.5 - 0.5 * Math.cos(Math.PI * p));
    }
    if (age < 0.85) return -DIP_DEPTH + Math.sin((age - 0.28) * 12) * 0.015;
    const p = Math.min(1, (age - 0.85) / 0.65);
    const c1 = 1.9;
    const c3 = c1 + 1;
    const back = 1 + c3 * Math.pow(p - 1, 3) + c1 * Math.pow(p - 1, 2);
    return -DIP_DEPTH * (1 - back);
  }

  const mascot: Mascot = {
    group,
    dome,
    skin: bodyMat,
    extras,
    foam,
    still: false,
    setViewport(heightPx, pixelRatio) {
      spoutMat.uniforms.uHalfH.value = heightPx / 2;
      spoutMat.uniforms.uMinPx.value = 1.6 * pixelRatio;
    },
    setUnderwater(uw) {
      // The spout is above the water: only the picture above shows it.
      aboveWater = uw < 0.5;
      spout.visible = spoutAlive && aboveWater;
      // Under water the light is scattered from every side; a little more
      // glow keeps the body readable through the water's fog without
      // flattening it (O5: the old 1.5 made it a flat disc).
      bodyMat.emissiveIntensity = MathUtils.lerp(0.55, 0.6, uw);
      bounce.intensity = MathUtils.lerp(0.9, 2.2, uw);
      bounce.color.copy(BOUNCE_ABOVE.sky).lerp(BOUNCE_UNDER.sky, uw);
      bounce.groundColor.copy(BOUNCE_ABOVE.ground).lerp(BOUNCE_UNDER.ground, uw);
    },
    dip(t) {
      if (t - dipStart < 1.6) return;
      dipStart = t;
      poppedAt = t + 0.85;
      opts.onRipple(0.8);
    },
    update(dt, t, aim, look) {
      const k = (rate: number) => 1 - Math.exp(-dt * rate);
      // A non-finite aim would stick in the smoothing for good, so it counts as "straight ahead".
      const ax = Number.isFinite(aim.x) ? aim.x : 0;
      const ay = Number.isFinite(aim.y) ? aim.y : 0;
      eyeAim.x += (ax - eyeAim.x) * k(9);
      eyeAim.y += (ay - eyeAim.y) * k(9);
      lookAim.x += (ax - lookAim.x) * k(2.6);
      lookAim.y += (ay - lookAim.y) * k(2.6);

      const bob = mascot.still ? 0 : Math.sin((t * TAU) / 4);
      const dipY = mascot.still ? 0 : dipOffset(t);
      group.position.y = -0.05 + 0.06 * bob + dipY;
      group.rotation.z = 0.02 * Math.sin((t * TAU) / 4 + 0.7);
      // O8: the head turns only a little; the eyes do most of the looking.
      group.rotation.y = lookAim.x * HEAD_TURN.yaw * look;
      group.rotation.x = -lookAim.y * HEAD_TURN.pitch * look + 0.012 * Math.sin((t * TAU) / 8);

      // Eyes: both move by the same amount in their sockets.
      const sock = EYES_3D.socket;
      const turn = MathUtils.clamp(eyeAim.x * sock.phi, -sock.phi, sock.phi);
      const rho = MathUtils.clamp(rho0 + eyeAim.y * sock.rho, rho0 - sock.rho, rho0 + sock.rho);
      for (let i = 0; i < eyes.length; i++) {
        surface(EYE_SIDE[i] * EYES_3D.phi + turn, rho, scratch);
        eyes[i].eye.position.copy(scratch.pos).addScaledVector(scratch.normal, 0.012 * MASCOT_R);
        negNormal.copy(scratch.normal).negate();
        m4.lookAt(origin, negNormal, up);
        eyes[i].eye.quaternion.setFromRotationMatrix(m4);
      }

      // Blink: scale-Y to 0.1 and back in 140 ms, every 4 to 7 s.
      if (!mascot.still && t >= nextBlink) {
        blinkStart = t;
        nextBlink = t + 4 + Math.random() * 3;
      }
      const bp = (t - blinkStart) / 0.14;
      const blink = bp >= 0 && bp <= 1 ? 1 - 0.9 * Math.sin(Math.PI * bp) : 1;
      for (const e of eyes) {
        e.eyeMesh.scale.y = EYES_3D.r * blink;
        e.glow.scale.y = EYES_3D.r * GLOW * (0.35 + 0.65 * blink);
      }
      glowMat.uniforms.uIntensity.value = 0.8 + 0.12 * Math.sin(t * 1.3);

      // Foam breathes with the bob and thins out while the creature is under.
      foamMat.uniforms.uTime.value = t;
      const submerged = MathUtils.clamp(-dipY / DIP_DEPTH, 0, 1);
      foamMat.uniforms.uAmount.value = (0.42 + 0.14 * bob) * (1 - 0.7 * submerged);
      foam.scale.setScalar(1 + 0.035 * bob);

      // A breath every 5 to 9 s, never while it is diving after a click.
      if (!mascot.still && t >= nextSpout) {
        if (t - dipStart > 1.8) breathe();
        nextSpout = t + 5 + Math.random() * 4;
      }
      if (spoutAlive) updateSpout(dt);

      if (!mascot.still && poppedAt > 0 && t >= poppedAt && t - dt < poppedAt) {
        opts.onRipple(1);
        burst();
      }
      if (splashLife > 0) {
        splashLife -= dt;
        for (let i = 0; i < SPLASH_COUNT; i++) {
          splashVel[i * 3 + 1] -= 9.5 * dt;
          splashPos[i * 3] += splashVel[i * 3] * dt;
          splashPos[i * 3 + 1] += splashVel[i * 3 + 1] * dt;
          splashPos[i * 3 + 2] += splashVel[i * 3 + 2] * dt;
          if (splashPos[i * 3 + 1] < 0) splashAlpha[i] = 0;
          else splashAlpha[i] = Math.min(1, splashLife * 1.6) * (splashAlpha[i] > 0 ? 1 : 0);
        }
        splashGeo.attributes.position.needsUpdate = true;
        splashGeo.attributes.aAlpha.needsUpdate = true;
        if (splashLife <= 0) splashAlpha.fill(0);
      }
    },
    dispose() {
      for (const d of disposables) d.dispose();
    },
  };
  return mascot;
}
