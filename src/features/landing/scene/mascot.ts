import * as THREE from "three";
import { PALETTE } from "../tokens";
import { DOME_EXPONENT, EYE_LOGO, MASCOT_H, MASCOT_R } from "./scene-config";
import { FOAM_FRAG, GLOW_FRAG, GLOW_VERT, SPLASH_FRAG, SPLASH_VERT } from "./shaders";

/**
 * The 3D mascot, built from primitives so it matches `public/logo-light.png`:
 * a navy dome (a superellipse profile, revolved) with one white eye on its
 * right. Half of it is below the waterline; the water plane hides that half
 * until the camera dives. The eye slides over the dome's surface towards where
 * the visitor is looking, inside a cone that keeps it on the face.
 */

export type Aim = { x: number; y: number };

type MascotOptions = {
  /** Called when the mascot disturbs the water; `amp` is 0.3 (bob) to 1 (pop). */
  onRipple: (amp: number) => void;
  pixelRatio: number;
};

export type Mascot = {
  /** Everything that moves with the creature. Raycast target is `dome`. */
  group: THREE.Group;
  dome: THREE.Mesh;
  /** Foam collar and splash, which live in world space at the waterline. */
  extras: THREE.Group;
  /** The foam collar alone, hidden when the camera is below the surface. */
  foam: THREE.Mesh;
  update: (dt: number, t: number, aim: Aim, look: number) => void;
  dip: (t: number) => void;
  /** 0 above the surface, 1 below: lifts the body's glow and fades in the eye seen through the water. */
  setUnderwater: (uw: number) => void;
  /** Freeze the idle motion (reduced motion). */
  still: boolean;
  dispose: () => void;
};

const TAU = Math.PI * 2;
const EXP = 2 / DOME_EXPONENT;

/** Upper dome profile at parameter t in [0, PI/2]. */
function profile(t: number): { r: number; y: number } {
  return { r: MASCOT_R * Math.pow(Math.cos(t), EXP), y: MASCOT_H * Math.pow(Math.sin(t), EXP) };
}

function buildBodyGeometry(upperOnly = false): THREE.LatheGeometry {
  const pts: THREE.Vector2[] = [];
  // Submerged half: a plain ellipse, reached only when the camera is below.
  const below = 0.9 * MASCOT_R;
  if (upperOnly) pts.push(new THREE.Vector2(0, 0), new THREE.Vector2(MASCOT_R, 0));
  else
    for (let i = 0; i <= 24; i++) {
      const s = (i / 24) * (Math.PI / 2);
      pts.push(new THREE.Vector2(MASCOT_R * Math.sin(s), -below * Math.cos(s)));
    }
  // Visible dome: dense near the base where the sides turn.
  for (let i = 1; i <= 72; i++) {
    const t = Math.pow(i / 72, 0.8) * (Math.PI / 2);
    const p = profile(t);
    pts.push(new THREE.Vector2(p.r, p.y));
  }
  return new THREE.LatheGeometry(pts, upperOnly ? 64 : 112);
}

/** Point and outward normal on the dome for azimuth `phi` and height fraction `rho` of the dome. */
function surface(phi: number, rho: number, out: { pos: THREE.Vector3; normal: THREE.Vector3 }): void {
  const sinT = Math.pow(THREE.MathUtils.clamp(rho, 0.02, 0.98), DOME_EXPONENT / 2);
  const t = Math.asin(sinT);
  const cosT = Math.cos(t);
  const r = MASCOT_R * Math.pow(cosT, EXP);
  const y = MASCOT_H * rho;
  const dr = -MASCOT_R * EXP * Math.pow(cosT, EXP - 1) * sinT;
  const dy = MASCOT_H * EXP * Math.pow(sinT, EXP - 1) * cosT;
  const len = Math.hypot(dr, dy) || 1;
  const nr = dy / len;
  const ny = -dr / len;
  out.pos.set(r * Math.sin(phi), y, r * Math.cos(phi));
  out.normal.set(nr * Math.sin(phi), ny, nr * Math.cos(phi)).normalize();
}

const SPLASH_COUNT = 56;

export function createMascot(opts: MascotOptions): Mascot {
  const group = new THREE.Group();
  const extras = new THREE.Group();
  const disposables: { dispose: () => void }[] = [];

  // ---- Body --------------------------------------------------------------
  const bodyGeo = buildBodyGeometry();
  const bodyMat = new THREE.MeshPhysicalMaterial({
    color: new THREE.Color(PALETTE.navy),
    roughness: 0.42,
    metalness: 0.0,
    clearcoat: 1,
    clearcoatRoughness: 0.2,
    // A little emission keeps the shadow side navy rather than black.
    emissive: new THREE.Color("#0a1450"),
    emissiveIntensity: 0.55,
    envMapIntensity: 1.35,
  });
  const dome = new THREE.Mesh(bodyGeo, bodyMat);
  group.add(dome);
  disposables.push(bodyGeo, bodyMat);

  // Seen from below, the part of the creature above the water shows through
  // the surface as a faint blue shape: the dome's upper half drawn without
  // depth testing, only while the camera is under water.
  const ghostBodyGeo = buildBodyGeometry(true);
  const ghostBodyMat = new THREE.MeshBasicMaterial({
    color: new THREE.Color("#2f6fd6"),
    transparent: true,
    opacity: 0,
    depthTest: false,
    depthWrite: false,
    toneMapped: false,
  });
  const ghostBody = new THREE.Mesh(ghostBodyGeo, ghostBodyMat);
  ghostBody.renderOrder = 6;
  ghostBody.visible = false;
  group.add(ghostBody);
  disposables.push(ghostBodyGeo, ghostBodyMat);

  // ---- Eye ---------------------------------------------------------------
  const eye = new THREE.Group();
  const eyeGeo = new THREE.SphereGeometry(1, 40, 28);
  // Pure white, outside tone mapping: the logo's eye is a white cut-out, and
  // ACES would turn "white" into light grey next to the glow around it.
  const eyeMat = new THREE.MeshBasicMaterial({ color: 0xffffff, toneMapped: false });
  const eyeMesh = new THREE.Mesh(eyeGeo, eyeMat);
  eyeMesh.scale.set(EYE_LOGO.rx * MASCOT_R, EYE_LOGO.ry * MASCOT_R, 0.065 * MASCOT_R);
  eye.add(eyeMesh);
  const glowGeo = new THREE.PlaneGeometry(1, 1);
  const glowMat = new THREE.ShaderMaterial({
    uniforms: { uColor: { value: new THREE.Color("#bfe0ff") }, uIntensity: { value: 0.5 } },
    vertexShader: GLOW_VERT,
    fragmentShader: GLOW_FRAG,
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
  });
  const glow = new THREE.Mesh(glowGeo, glowMat);
  glow.scale.set(EYE_LOGO.rx * MASCOT_R * 6.5, EYE_LOGO.ry * MASCOT_R * 6.5, 1);
  glow.position.z = 0.12 * MASCOT_R;
  glow.renderOrder = 3;
  eye.add(glow);
  // Seen from below, the creature's eye still looks down at you: a faint copy
  // of the eye and its halo, drawn without depth testing so the water ceiling
  // does not hide it. Hidden above the surface.
  const ghostMat = new THREE.MeshBasicMaterial({
    color: 0xffffff,
    transparent: true,
    opacity: 0,
    depthTest: false,
    depthWrite: false,
    toneMapped: false,
  });
  const ghostEye = new THREE.Mesh(eyeGeo, ghostMat);
  ghostEye.renderOrder = 8;
  ghostEye.visible = false;
  eye.add(ghostEye);
  const ghostGlowMat = new THREE.ShaderMaterial({
    uniforms: { uColor: { value: new THREE.Color("#9fd0ff") }, uIntensity: { value: 0 } },
    vertexShader: GLOW_VERT,
    fragmentShader: GLOW_FRAG,
    transparent: true,
    depthTest: false,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
  });
  const ghostGlow = new THREE.Mesh(glowGeo, ghostGlowMat);
  ghostGlow.renderOrder = 7;
  ghostGlow.visible = false;
  ghostGlow.position.z = 0.12 * MASCOT_R;
  eye.add(ghostGlow);
  group.add(eye);
  disposables.push(eyeGeo, eyeMat, glowGeo, glowMat, ghostMat, ghostGlowMat);

  // ---- Lights that belong to the creature --------------------------------
  // Water bounce from below-front, so the lower dome is not a black cut-out.
  // A hemisphere light is diffuse only, so it tints the dome without adding a
  // second hot spot to the clearcoat the way a point light would.
  const bounce = new THREE.HemisphereLight(0x1b2f7a, new THREE.Color(PALETTE.lake), 0.9);
  extras.add(bounce);

  // ---- Foam collar -------------------------------------------------------
  const foamSize = MASCOT_R * 2 * 1.75;
  const foamGeo = new THREE.PlaneGeometry(foamSize, foamSize);
  foamGeo.rotateX(-Math.PI / 2);
  const foamMat = new THREE.ShaderMaterial({
    uniforms: {
      uTime: { value: 0 },
      uAmount: { value: 0.45 },
      uColor: { value: new THREE.Color("#7fb8f5") },
      uInner: { value: 1 / 1.75 },
    },
    vertexShader: GLOW_VERT,
    fragmentShader: FOAM_FRAG,
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
  });
  const foam = new THREE.Mesh(foamGeo, foamMat);
  foam.position.y = 0.02;
  foam.renderOrder = 1;
  extras.add(foam);
  disposables.push(foamGeo, foamMat);

  // ---- Splash ------------------------------------------------------------
  const splashPos = new Float32Array(SPLASH_COUNT * 3);
  const splashVel = new Float32Array(SPLASH_COUNT * 3);
  const splashAlpha = new Float32Array(SPLASH_COUNT);
  const splashGeo = new THREE.BufferGeometry();
  splashGeo.setAttribute("position", new THREE.BufferAttribute(splashPos, 3));
  splashGeo.setAttribute("aAlpha", new THREE.BufferAttribute(splashAlpha, 1));
  const splashMat = new THREE.ShaderMaterial({
    uniforms: { uColor: { value: new THREE.Color("#cfe6ff") }, uPx: { value: opts.pixelRatio } },
    vertexShader: SPLASH_VERT,
    fragmentShader: SPLASH_FRAG,
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
  });
  const splash = new THREE.Points(splashGeo, splashMat);
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
      splashPos[i * 3 + 2] = Math.sin(a) * r;
      const up = 2.6 + Math.random() * 3.2;
      const out = 0.5 + Math.random() * 1.1;
      splashVel[i * 3] = Math.cos(a) * out;
      splashVel[i * 3 + 1] = up;
      splashVel[i * 3 + 2] = Math.sin(a) * out;
      splashAlpha[i] = 1;
    }
  }

  // ---- Animation state ---------------------------------------------------
  const scratch = { pos: new THREE.Vector3(), normal: new THREE.Vector3() };
  const m4 = new THREE.Matrix4();
  const origin = new THREE.Vector3();
  const negNormal = new THREE.Vector3();
  const up = new THREE.Vector3(0, 1, 0);
  const eyeAim = { x: 0, y: 0 };
  const lookAim = { x: 0, y: 0 };
  let nextBlink = 2.2;
  let blinkStart = -10;
  let dipStart = -10;
  let poppedAt = -10;
  const DIP_DEPTH = MASCOT_H + 0.25;

  const phi0 = Math.asin(EYE_LOGO.x / 0.98);
  const rho0 = EYE_LOGO.y;

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
    extras,
    foam,
    still: false,
    setUnderwater(uw) {
      bodyMat.emissiveIntensity = THREE.MathUtils.lerp(0.55, 1.5, uw);
      bounce.intensity = THREE.MathUtils.lerp(0.9, 2.6, uw);
      const k = THREE.MathUtils.smoothstep(uw, 0.1, 0.9);
      ghostEye.visible = k > 0.01;
      ghostGlow.visible = ghostEye.visible;
      ghostMat.opacity = 0.85 * k;
      ghostBody.visible = ghostEye.visible;
      ghostBodyMat.opacity = 0.22 * k;
      ghostGlowMat.uniforms.uIntensity.value = 0.75 * k;
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
      group.rotation.y = lookAim.x * 0.15 * look;
      group.rotation.x = -lookAim.y * 0.06 * look + 0.012 * Math.sin((t * TAU) / 8);

      // Eye: slide over the dome inside a cone that keeps it on the face.
      const phi = THREE.MathUtils.clamp(phi0 + eyeAim.x * 0.78, -0.62, 1.02);
      const rho = THREE.MathUtils.clamp(rho0 + eyeAim.y * 0.24, 0.22, 0.62);
      surface(phi, rho, scratch);
      eye.position.copy(scratch.pos).addScaledVector(scratch.normal, 0.012 * MASCOT_R);
      negNormal.copy(scratch.normal).negate();
      m4.lookAt(origin, negNormal, up);
      eye.quaternion.setFromRotationMatrix(m4);

      // Blink: scale-Y to 0.1 and back in 140 ms, every 4 to 7 s.
      if (!mascot.still && t >= nextBlink) {
        blinkStart = t;
        nextBlink = t + 4 + Math.random() * 3;
      }
      const bp = (t - blinkStart) / 0.14;
      const blink = bp >= 0 && bp <= 1 ? 1 - 0.9 * Math.sin(Math.PI * bp) : 1;
      eyeMesh.scale.y = EYE_LOGO.ry * MASCOT_R * blink;
      ghostEye.scale.copy(eyeMesh.scale);
      ghostGlow.scale.set(EYE_LOGO.rx * MASCOT_R * 7, EYE_LOGO.ry * MASCOT_R * 7 * (0.35 + 0.65 * blink), 1);
      glow.scale.y = EYE_LOGO.ry * MASCOT_R * 6.5 * (0.35 + 0.65 * blink);
      glowMat.uniforms.uIntensity.value = 0.8 + 0.12 * Math.sin(t * 1.3);

      // Foam breathes with the bob and thins out while the creature is under.
      foamMat.uniforms.uTime.value = t;
      const submerged = THREE.MathUtils.clamp(-dipY / DIP_DEPTH, 0, 1);
      foamMat.uniforms.uAmount.value = (0.42 + 0.14 * bob) * (1 - 0.7 * submerged);
      foam.scale.setScalar(1 + 0.035 * bob);

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
