import * as THREE from "three";
import { PALETTE } from "../tokens";
import { MASCOT_R, RIPPLES } from "./scene-config";
import type { SkyUniforms } from "./sky";
import { WATER_FRAG, WATER_VERT } from "./shaders";

/**
 * The lake: one large plane, one custom program (see `WATER_FRAG`). The
 * engine owns the clock and the camera; this module owns the uniforms and
 * the ripple ring buffer.
 */

export type Water = {
  mesh: THREE.Mesh;
  uniforms: Record<string, THREE.IUniform>;
  /** Start a ring at the mascot's waterline. `amp` is about 0.3 (bob) to 1 (splash). */
  spawnRipple: (time: number, amp: number) => void;
  dispose: () => void;
};

export function createWater(sky: SkyUniforms, lowQuality: boolean): Water {
  const ripples = Array.from({ length: RIPPLES }, () => new THREE.Vector4(0, 0, -100, 0));
  const uniforms: Record<string, THREE.IUniform> = {
    uTime: { value: 0 },
    uFogColor: { value: new THREE.Color("#0b2257") },
    uFogDensity: { value: 0.021 },
    uMoonDir: sky.uMoonDir,
    uMoonColor: sky.uMoonColor,
    uSkyHorizon: sky.uSkyHorizon,
    uSkyMid: sky.uSkyMid,
    uSkyZenith: sky.uSkyZenith,
    // Body colour of the lake under its reflections: near-black navy with a
    // trace of the logo's blue.
    uDeep: { value: new THREE.Color("#030a26") },
    uLake: { value: new THREE.Color(PALETTE.lake) },
    uRip: { value: ripples },
    uMascot: { value: new THREE.Vector3(0, 0, MASCOT_R * 0.99) },
    uWaveGain: { value: 1 },
  };
  const material = new THREE.ShaderMaterial({
    uniforms,
    vertexShader: WATER_VERT,
    fragmentShader: WATER_FRAG,
    side: THREE.DoubleSide,
    defines: lowQuality ? { LOW_QUALITY: "" } : {},
  });
  const geometry = new THREE.PlaneGeometry(260, 260, 1, 1);
  geometry.rotateX(-Math.PI / 2);
  const mesh = new THREE.Mesh(geometry, material);
  mesh.frustumCulled = false;
  let next = 0;
  return {
    mesh,
    uniforms,
    spawnRipple(time, amp) {
      ripples[next].set(0, 0, time, amp);
      next = (next + 1) % RIPPLES;
    },
    dispose() {
      geometry.dispose();
      material.dispose();
    },
  };
}
