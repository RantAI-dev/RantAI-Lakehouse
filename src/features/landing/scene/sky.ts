import * as THREE from "three";
import { PALETTE } from "../tokens";
import { MOON_DIR } from "./scene-config";
import { SKY_FRAG, SKY_VERT, STARS_FRAG, STARS_VERT } from "./shaders";

/**
 * Night sky: a gradient dome with a moon, a field of twinkling stars, and the
 * small environment map the mascot's clearcoat reflects.
 */

export type SkyUniforms = {
  uSkyHorizon: THREE.IUniform<THREE.Color>;
  uSkyMid: THREE.IUniform<THREE.Color>;
  uSkyZenith: THREE.IUniform<THREE.Color>;
  uMoonDir: THREE.IUniform<THREE.Vector3>;
  uMoonColor: THREE.IUniform<THREE.Color>;
  uMoonBoost: THREE.IUniform<number>;
  /** The engine's shared fog colour object; the dome shows it when under water. */
  uFogColor: THREE.IUniform<THREE.Color>;
  /** 0 above the surface, 1 below it. */
  uUnder: THREE.IUniform<number>;
};

export function createSkyUniforms(): SkyUniforms {
  return {
    // The horizon glow is the lake's own blue, very dark; the water fog uses
    // the same value so the far shore dissolves into the sky.
    uSkyHorizon: { value: new THREE.Color("#0b2257") },
    uSkyMid: { value: new THREE.Color(PALETTE.navy) },
    uSkyZenith: { value: new THREE.Color(PALETTE.abyss) },
    uMoonDir: { value: new THREE.Vector3(...MOON_DIR) },
    uMoonColor: { value: new THREE.Color("#cfe6ff") },
    uMoonBoost: { value: 7 },
    uFogColor: { value: new THREE.Color("#0b2257") },
    uUnder: { value: 0 },
  };
}

export function createSkyDome(uniforms: SkyUniforms): THREE.Mesh {
  const material = new THREE.ShaderMaterial({
    uniforms: uniforms as unknown as Record<string, THREE.IUniform>,
    vertexShader: SKY_VERT,
    fragmentShader: SKY_FRAG,
    side: THREE.BackSide,
    depthWrite: false,
  });
  const mesh = new THREE.Mesh(new THREE.SphereGeometry(60, 48, 24), material);
  mesh.frustumCulled = false;
  mesh.renderOrder = -10;
  return mesh;
}

/** Deterministic PRNG so the star field is the same on every load. */
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export type Stars = {
  points: THREE.Points;
  uniforms: { uTime: THREE.IUniform<number>; uPx: THREE.IUniform<number>; uAlpha: THREE.IUniform<number> };
};

/** Up to 600 points on the upper hemisphere (300 on phones, plan §4 S1). */
export function createStars(count: number, pixelRatio: number): Stars {
  const rand = mulberry32(0x1a4e);
  const pos = new Float32Array(count * 3);
  const size = new Float32Array(count);
  const phase = new Float32Array(count);
  const bright = new Float32Array(count);
  for (let i = 0; i < count; i++) {
    // Uniform on the sphere, folded to the upper half.
    const u = rand();
    const v = rand();
    const theta = 2 * Math.PI * u;
    const y = Math.sqrt(v);
    const r = Math.sqrt(1 - v);
    const R = 250;
    pos[i * 3] = Math.cos(theta) * r * R;
    pos[i * 3 + 1] = y * R;
    pos[i * 3 + 2] = Math.sin(theta) * r * R;
    const big = rand() > 0.93;
    size[i] = big ? 4 + rand() * 2.2 : 1.8 + rand() * 1.6;
    phase[i] = rand();
    bright[i] = 0.35 + rand() * 0.65;
  }
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.BufferAttribute(pos, 3));
  geometry.setAttribute("aSize", new THREE.BufferAttribute(size, 1));
  geometry.setAttribute("aPhase", new THREE.BufferAttribute(phase, 1));
  geometry.setAttribute("aBright", new THREE.BufferAttribute(bright, 1));
  const uniforms = {
    uTime: { value: 0 },
    uPx: { value: pixelRatio },
    uAlpha: { value: 1 },
  };
  const material = new THREE.ShaderMaterial({
    uniforms: { ...uniforms, uColor: { value: new THREE.Color(PALETTE.foam) } },
    vertexShader: STARS_VERT,
    fragmentShader: STARS_FRAG,
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
  });
  // Share the live uniform objects with the caller.
  material.uniforms.uTime = uniforms.uTime;
  material.uniforms.uPx = uniforms.uPx;
  material.uniforms.uAlpha = uniforms.uAlpha;
  const points = new THREE.Points(geometry, material);
  points.frustumCulled = false;
  points.renderOrder = -9;
  return { points, uniforms };
}

/**
 * Bakes the sky plus two soft light panels into a PMREM environment, once.
 * The panels are what makes the navy dome read as glossy: a dark body only
 * shows its clearcoat where there is something bright to reflect.
 */
export function buildEnvironment(
  renderer: THREE.WebGLRenderer,
  skyUniforms: SkyUniforms,
): THREE.WebGLRenderTarget {
  const envScene = new THREE.Scene();
  const dome = createSkyDome(skyUniforms);
  envScene.add(dome);
  const panel = (w: number, h: number, pos: [number, number, number], intensity: number, tint: string) => {
    const m = new THREE.Mesh(
      new THREE.PlaneGeometry(w, h),
      new THREE.MeshBasicMaterial({
        color: new THREE.Color(tint).multiplyScalar(intensity),
        side: THREE.DoubleSide,
        toneMapped: false,
      }),
    );
    m.position.set(...pos);
    m.lookAt(0, 0, 0);
    envScene.add(m);
    return m;
  };
  // A cold key from behind the creature (it draws the rim of the dome) and
  // one very large, faint panel in front, so the face has a soft gradient
  // instead of a hot spot. Small bright panels read as stickers on a dark body.
  panel(40, 14, [-18, 20, -36], 4.5, "#cfe6ff");
  panel(70, 40, [10, 8, 40], 0.22, "#44A6FA");
  const pmrem = new THREE.PMREMGenerator(renderer);
  const target = pmrem.fromScene(envScene, 0.035);
  pmrem.dispose();
  envScene.traverse((o) => {
    const m = o as THREE.Mesh;
    m.geometry?.dispose();
    const mat = m.material as THREE.Material | undefined;
    mat?.dispose();
  });
  return target;
}
