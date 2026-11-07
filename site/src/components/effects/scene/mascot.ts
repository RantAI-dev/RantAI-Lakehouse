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
  type WebGLProgramParametersWithUniforms,
} from "three";
import { PALETTE } from "@/lib/palette";
import { BELLY_SQUASH, BLUSH, BODY_LONG, DOME_SPHERE, EYES_3D, HEAD_TURN, MASCOT_H, MASCOT_R } from "./scene-config";
import { createToyParts } from "./whale";
import { FOAM_FRAG, GLOW_FRAG, GLOW_VERT, SPLASH_FRAG, SPLASH_VERT, SPOUT_FRAG, SPOUT_VERT } from "./shaders";

/**
 * The 3D mascot, built from primitives on the official mark's shapes
 * (`components/shared/mascot-svg.tsx`, WEB-5 O2).
 *
 * WEB-5 O11 (owner, replacing O8's realistic whale, "ini lebih kek
 * dugong"): a simple, chubby toy whale.
 * - One round body: the mark's dome on top, so above the water the
 *   silhouette is exactly the logo's, rounding off below the waterline into
 *   a soft egg, longer front to back (`BODY_LONG`) and wider than tall.
 * - A white belly with soft curved grooves in lake blue, and a light coral
 *   blush under each eye, painted in the skin's own shader (`paintSkin`),
 *   so they cost no geometry. The belly is below the waterline.
 * - Two round eyes (O3: two, the mark keeps one): a bright white, a navy
 *   pupil that follows the pointer, and two small highlights. They blink.
 * - A short tail curling up to a small fluke, and two small flippers
 *   (whale.ts), seen under water only.
 * - Smooth, soft-plastic skin: satin rather than glossy, a friendlier navy.
 * - A gentle bob; under water a slow, happy swim (`swim`).
 * - Every 5 to 9 s a small, soft fountain from the blowhole.
 * The water plane hides everything below the waterline until the camera
 * dives.
 */

export type Aim = { x: number; y: number };

type MascotOptions = {
  /** Phones: a lighter whale body. */
  lowQuality: boolean;
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
  /** 0..1: how much it is swimming (the camera under water) rather than resting at the surface. */
  swim: number;
  /** Paints the belly, its grooves and the blush in the skin's shader; the engine applies it with the fog. */
  paintSkin: (shader: WebGLProgramParametersWithUniforms) => void;
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

function buildBodyGeometry(): LatheGeometry {
  const pts: Vector2[] = [];
  // Below the waterline the mark's sphere carries on round to the bottom,
  // squashed towards the belly (smoothly, so the curve has no crease at
  // the waterline): the toy's chubby, slightly flat-bottomed body (O11).
  const below = 40;
  for (let i = 0; i <= below; i++) {
    const t = -Math.PI / 2 + (i / below) * (BASE_T + Math.PI / 2);
    const r = SPHERE_R * Math.cos(t);
    const y = SPHERE_CY + SPHERE_R * Math.sin(t);
    const squash = 1 - (1 - BELLY_SQUASH) * MathUtils.smoothstep(-y, 0, 0.6);
    pts.push(new Vector2(Math.max(r, i === 0 ? 0 : 1e-3), y * squash));
  }
  // Visible dome: the sphere cap, evenly spaced along its arc.
  for (let i = 1; i <= 72; i++) {
    const p = profile(i / 72);
    pts.push(new Vector2(p.r, p.y));
  }
  const geo = new LatheGeometry(pts, 112);
  // Longer front to back, like a bean; the front (+z) faces the camera,
  // so the mark's silhouette is unchanged from there.
  geo.scale(1, 1, BODY_LONG);
  return geo;
}

/** Point and outward normal on the dome for azimuth `phi` and height fraction `rho` of the dome. */
function surface(phi: number, rho: number, out: { pos: Vector3; normal: Vector3 }): void {
  // On a sphere the outward normal is the direction from its centre.
  const y = MASCOT_H * MathUtils.clamp(rho, 0.02, 0.98);
  const sinT = (y - SPHERE_CY) / SPHERE_R;
  const cosT = Math.sqrt(Math.max(0, 1 - sinT * sinT));
  const r = SPHERE_R * cosT;
  // The body is the sphere stretched along z: positions stretch with it,
  // normals by the inverse.
  out.pos.set(r * Math.sin(phi), y, r * Math.cos(phi) * BODY_LONG);
  out.normal.set(cosT * Math.sin(phi), sinT, (cosT * Math.cos(phi)) / BODY_LONG).normalize();
}

/** The blowhole, on top of the head a little behind its crown (group space). */
const BLOWHOLE = new Vector3(0, MASCOT_H - 0.03, -0.32);
const SPOUT_DROPS = 44;
const SPOUT_MIST = 22;

const SPLASH_COUNT = 56;

export function createMascot(opts: MascotOptions): Mascot {
  const group = new Group();
  const extras = new Group();
  const disposables: { dispose: () => void }[] = [];

  // ---- Body --------------------------------------------------------------
  const bodyGeo = buildBodyGeometry();
  // O11: soft plastic. Satin, a thin soft clearcoat, and a navy a step
  // towards the lake's blue, friendlier and still readable through water.
  const bodyMat = new MeshPhysicalMaterial({
    color: new Color("#2152b8"),
    roughness: 0.5,
    metalness: 0.0,
    clearcoat: 0.45,
    clearcoatRoughness: 0.38,
    // A little emission keeps the shadow side blue rather than black.
    emissive: new Color("#12307f"),
    emissiveIntensity: 0.5,
    envMapIntensity: 1.1,
  });
  const dome = new Mesh(bodyGeo, bodyMat);
  group.add(dome);
  disposables.push(bodyGeo, bodyMat);

  // O11: the tail and the flippers, in the same skin, seen only under water.
  const parts = createToyParts(bodyMat, opts.lowQuality);
  parts.group.visible = false;
  group.add(parts.group);
  disposables.push(parts);

  // Where the blush sits, in the body's own space.
  const blushAt = [-1, 1].map((side) => {
    const at = { pos: new Vector3(), normal: new Vector3() };
    surface(side * BLUSH.phi, BLUSH.rho, at);
    return at.pos;
  });
  const skinUniforms = {
    uBelly: { value: new Color(PALETTE.foam) },
    uGroove: { value: new Color(PALETTE.lake) },
    uBlush: { value: new Color("#ff8f96") },
    uBlushL: { value: blushAt[0] },
    uBlushR: { value: blushAt[1] },
    uBlushSize: { value: BLUSH.r },
    uBlushA: { value: BLUSH.alpha },
    uLongZ: { value: SPHERE_R * BODY_LONG },
  };

  // ---- Eyes --------------------------------------------------------------
  // WEB-5 O11: toy eyes, built alike and sharing their materials. Each is a
  // socket on the dome (fixed), a white that blinks, a navy pupil that
  // follows the pointer over the white, and two small highlights that ride
  // on the pupil. The lens pass tone-maps the frame once (O5), so the
  // whites are set above 1 in linear light: 2.5 maps to about 0.96 on
  // screen. Not higher: MSAA averages an edge before tone mapping, and a
  // much brighter white would saturate its edge pixels into jaggies.
  const ballGeo = new SphereGeometry(1, 32, 22);
  const whiteMat = new MeshBasicMaterial({ color: new Color(1, 1, 1).multiplyScalar(2.5) });
  const pupilMat = new MeshBasicMaterial({ color: new Color("#081038") });
  const glintMat = new MeshBasicMaterial({ color: new Color(1, 1, 1).multiplyScalar(3) });
  const glowGeo = new PlaneGeometry(1, 1);
  const glowMat = new ShaderMaterial({
    uniforms: { uColor: { value: new Color("#bfe0ff") }, uIntensity: { value: 0.3 } },
    vertexShader: GLOW_VERT,
    fragmentShader: GLOW_FRAG,
    transparent: true,
    depthWrite: false,
    blending: AdditiveBlending,
  });
  disposables.push(ballGeo, whiteMat, pupilMat, glintMat, glowGeo, glowMat);
  const R_EYE = EYES_3D.r;
  /** The white is an ellipsoid sunk into the dome: its depth and how far its centre sits inside. */
  const WHITE_DEPTH = 0.6 * R_EYE;
  const WHITE_SINK = -0.3 * R_EYE;
  const GLOW = 3.2;
  function buildEye(side: number) {
    const eye = new Group();
    const lid = new Group();
    eye.add(lid);
    const white = new Mesh(ballGeo, whiteMat);
    white.scale.set(R_EYE, R_EYE, WHITE_DEPTH);
    white.position.z = WHITE_SINK;
    lid.add(white);
    const pupil = new Mesh(ballGeo, pupilMat);
    pupil.scale.set(0.58 * R_EYE, 0.6 * R_EYE, 0.3 * R_EYE);
    lid.add(pupil);
    const glint = new Mesh(ballGeo, glintMat);
    glint.scale.setScalar(0.19 * R_EYE);
    lid.add(glint);
    const glintSmall = new Mesh(ballGeo, glintMat);
    glintSmall.scale.setScalar(0.085 * R_EYE);
    lid.add(glintSmall);
    const glow = new Mesh(glowGeo, glowMat);
    glow.scale.set(R_EYE * GLOW, R_EYE * GLOW, 1);
    glow.position.z = -0.05 * R_EYE;
    glow.renderOrder = 3;
    eye.add(glow);
    // The socket: on the dome's surface, facing out along its normal.
    const at = { pos: new Vector3(), normal: new Vector3() };
    surface(side * EYES_3D.phi, EYES_3D.rho, at);
    eye.position.copy(at.pos);
    const m = new Matrix4().lookAt(new Vector3(), at.normal.clone().negate(), new Vector3(0, 1, 0));
    eye.quaternion.setFromRotationMatrix(m);
    group.add(eye);
    return { eye, lid, pupil, glint, glintSmall, glow };
  }
  /** Left and right, by their side of the front (−phi, +phi). */
  const eyes = [buildEye(-1), buildEye(1)] as const;

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
  foam.scale.z = BODY_LONG;
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
      splashPos[i * 3 + 2] = Math.sin(a) * r * BODY_LONG;
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
    const lean = (Math.random() - 0.5) * 0.12;
    for (let i = 0; i < SPOUT_N; i++) {
      const mist = i >= SPOUT_DROPS;
      const j = i * 3;
      spoutPos[j] = holeWorld.x + (Math.random() - 0.5) * 0.08;
      spoutPos[j + 1] = holeWorld.y;
      spoutPos[j + 2] = holeWorld.z + (Math.random() - 0.5) * 0.08;
      // O11: a toy's little fountain. The breath splits into two soft
      // arcs, left and right, that rise, curl over and fall back, with a
      // puff of mist hanging between them.
      const side = i % 2 === 0 ? -1 : 1;
      spoutVel[j] = side * (mist ? 0.25 + Math.random() * 0.35 : 0.5 + Math.random() * 0.3) + lean;
      spoutVel[j + 1] = (mist ? 1.7 + Math.random() * 0.7 : 2.05 + Math.random() * 0.6) * power;
      spoutVel[j + 2] = (Math.random() - 0.5) * 0.18 - 0.06;
      // Negative age is the birth delay: the breath lasts about 0.45 s.
      spoutAge[i] = -(mist ? Math.random() * 0.45 : Math.random() * 0.3);
      spoutLife[i] = mist ? 1.6 + Math.random() * 0.9 : 0.8 + Math.random() * 0.45;
      spoutBase[i] = mist ? 0.15 + Math.random() * 0.12 : 0.028 + Math.random() * 0.022;
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
        spoutAlpha[i] = 0.24 * Math.min(1, age / 0.12) * Math.pow(1 - p, 1.6);
      } else {
        spoutVel[j + 1] -= 5.2 * dt;
        spoutSize[i] = spoutBase[i];
        spoutAlpha[i] = Math.min(1, age / 0.06) * Math.pow(1 - p, 1.4) * 0.75;
      }
      // The faintest breeze from the left.
      spoutVel[j] += 0.04 * dt;
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
  const eyeAim = { x: 0, y: 0 };
  const lookAim = { x: 0, y: 0 };
  let nextBlink = 2.2;
  let blinkStart = -10;
  let dipStart = -10;
  let poppedAt = -10;
  const DIP_DEPTH = MASCOT_H + 0.25;
  /** How deep it swims once the camera is under (O11): about the camera's own depth at −5 m. */
  const SWIM_DEPTH = 2.3;
  let swimPhase = 0;

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
    swim: 0,
    paintSkin(shader) {
      Object.assign(shader.uniforms, skinUniforms);
      shader.vertexShader = shader.vertexShader
        .replace("#include <common>", "#include <common>\nvarying vec3 vSkin;")
        .replace("#include <begin_vertex>", "#include <begin_vertex>\nvSkin = position;");
      shader.fragmentShader = shader.fragmentShader
        .replace(
          "#include <common>",
          `#include <common>
          varying vec3 vSkin;
          uniform vec3 uBelly; uniform vec3 uGroove; uniform vec3 uBlush;
          uniform vec3 uBlushL; uniform vec3 uBlushR; uniform float uBlushSize; uniform float uBlushA;
          uniform float uLongZ;`,
        )
        .replace(
          "#include <color_fragment>",
          `#include <color_fragment>
          {
            vec3 q = vSkin;
            // The belly line rises towards the front, as on a toy whale.
            float zf = clamp(q.z / uLongZ, -1.0, 1.0);
            float edge = -0.36 + 0.24 * zf;
            float belly = smoothstep(edge + 0.035, edge - 0.035, q.y);
            // Soft grooves, curving back as they run under the belly, on its front half.
            float phi = atan(q.x, q.z / ${BODY_LONG.toFixed(3)});
            float gp = phi + 0.32 * q.y;
            float cell = abs(fract(gp / 0.27 + 0.5) - 0.5) * 0.27 * 1.3;
            float groove = (1.0 - smoothstep(0.02, 0.045, cell))
              * smoothstep(edge - 0.07, edge - 0.2, q.y)
              * smoothstep(-0.78, -0.5, q.y)
              * smoothstep(-0.05, 0.45, cos(phi));
            vec3 c = mix(diffuseColor.rgb, uBelly, belly);
            c = mix(c, uGroove, groove * belly * 0.7);
            vec3 dl = q - uBlushL;
            vec3 dr = q - uBlushR;
            float s2 = uBlushSize * uBlushSize;
            float blush = max(exp(-dot(dl, dl) / s2), exp(-dot(dr, dr) / s2)) * uBlushA;
            diffuseColor.rgb = mix(c, uBlush, blush * (1.0 - belly));
            // A touch of its own light, so the blush still reads by moonlight.
            totalEmissiveRadiance += uBlush * blush * 0.1;
            // The belly keeps a little light of its own, so it reads white, not grey, in shade.
            totalEmissiveRadiance += uBelly * belly * 0.16;
          }`,
        );
    },
    setViewport(heightPx, pixelRatio) {
      spoutMat.uniforms.uHalfH.value = heightPx / 2;
      spoutMat.uniforms.uMinPx.value = 1.6 * pixelRatio;
    },
    setUnderwater(uw) {
      // The spout is above the water: only the picture above shows it.
      aboveWater = uw < 0.5;
      spout.visible = spoutAlive && aboveWater;
      parts.group.visible = !aboveWater;
      // Under water the light is scattered from every side; a little more
      // glow keeps the body readable through the water's fog without
      // flattening it (O5: the old 1.5 made it a flat disc).
      bodyMat.emissiveIntensity = MathUtils.lerp(0.5, 0.55, uw);
      // The environment is the night sky; under water there is no sky to
      // reflect, only the window overhead, so the gloss mostly goes (O8: it
      // made the throat glow white).
      bodyMat.envMapIntensity = MathUtils.lerp(1.1, 0.25, uw);
      bodyMat.clearcoat = MathUtils.lerp(0.45, 0.2, uw);
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
      // O11: it dives with the visitor. At the surface only the dome shows;
      // as the camera goes under, the whale sinks to about the camera's
      // depth, so under water the whole of it is in view, beside you.
      const swim = mascot.still ? 0 : MathUtils.clamp(mascot.swim, 0, 1);
      const sink = SWIM_DEPTH * MathUtils.smoothstep(swim, 0.15, 1);
      group.position.y = -0.05 + 0.06 * bob + dipY - sink;
      // O11: under water it swims, slowly and happily: the whole body
      // sways and drifts a little, the tail wags, the flippers paddle.
      swimPhase += dt;
      group.position.x = swim * 0.14 * Math.sin(swimPhase * 0.42);
      group.rotation.z = 0.02 * Math.sin((t * TAU) / 4 + 0.7) + swim * 0.05 * Math.sin(swimPhase * 0.8);
      // The head turns only a little; the pupils do most of the looking.
      // Swimming, it turns nearly side-on to the camera, so its tail shows.
      const turnToSwim = 0.9 * MathUtils.smoothstep(swim, 0.3, 1);
      group.rotation.y =
        lookAim.x * HEAD_TURN.yaw * look + turnToSwim + swim * 0.08 * Math.sin(swimPhase * 0.55 + 0.4);
      group.rotation.x = -lookAim.y * HEAD_TURN.pitch * look + 0.012 * Math.sin((t * TAU) / 8);
      parts.update(t, swim, mascot.still);

      // Pupils: over the white towards the pointer, on the white's curve.
      const px = MathUtils.clamp(eyeAim.x, -1, 1) * EYES_3D.pupil * R_EYE;
      const py = MathUtils.clamp(eyeAim.y, -1, 1) * EYES_3D.pupil * R_EYE * 0.85;
      const d2 = Math.min(0.95, (px * px + py * py) / (R_EYE * R_EYE));
      const pz = WHITE_SINK + WHITE_DEPTH * Math.sqrt(1 - d2) - 0.17 * R_EYE;
      for (const e of eyes) {
        e.pupil.position.set(px, py, pz);
        e.glint.position.set(px - 0.22 * R_EYE, py + 0.24 * R_EYE, pz + 0.24 * R_EYE);
        e.glintSmall.position.set(px + 0.2 * R_EYE, py - 0.15 * R_EYE, pz + 0.25 * R_EYE);
      }

      // Blink: scale-Y to 0.1 and back in 140 ms, every 4 to 7 s.
      if (!mascot.still && t >= nextBlink) {
        blinkStart = t;
        nextBlink = t + 4 + Math.random() * 3;
      }
      const bp = (t - blinkStart) / 0.14;
      const blink = bp >= 0 && bp <= 1 ? 1 - 0.9 * Math.sin(Math.PI * bp) : 1;
      for (const e of eyes) {
        e.lid.scale.y = blink;
        e.glow.scale.y = R_EYE * GLOW * (0.35 + 0.65 * blink);
      }
      glowMat.uniforms.uIntensity.value = 0.3 + 0.05 * Math.sin(t * 1.3);

      // Foam breathes with the bob and thins out while the creature is under.
      foamMat.uniforms.uTime.value = t;
      const submerged = MathUtils.clamp((-dipY + sink) / DIP_DEPTH, 0, 1);
      foamMat.uniforms.uAmount.value = (0.42 + 0.14 * bob) * (1 - 0.7 * submerged);
      foam.scale.setScalar(1 + 0.035 * bob);

      // A breath every 5 to 9 s, never while it is diving after a click.
      if (!mascot.still && t >= nextSpout) {
        if (t - dipStart > 1.8 && sink < 0.2) breathe();
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
