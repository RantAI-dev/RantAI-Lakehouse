import * as THREE from "three";
import { RoundedBoxGeometry } from "three/examples/jsm/geometries/RoundedBoxGeometry.js";
import { DEPTHS } from "../landing-copy";
import { clamp01 } from "../scroll-timeline";
import { GLOW_FRAG, GLOW_VERT, SLAB_SURFACE_FRAG, SLAB_SURFACE_VERT, STREAM_FRAG, STREAM_VERT } from "./shaders";
import { SLAB_Y } from "./scene-config";

/**
 * The medallion stack (S3): three glass slabs, Bronze above Silver above Gold.
 *
 * Each slab is a rounded glass box with a data surface on top and a soft
 * halo underneath. Between them, particles ("rows") fall from the slab above
 * into the next one: noisy and many at Bronze, aligned in rows at Silver, few
 * and bright at Gold. The slab the visitor is at lights up and the others
 * draw apart from it a little. All of it is driven by one number, the
 * continuous `focus` (0 Bronze, 1 Silver, 2 Gold) from `scroll-timeline.ts`.
 */

export const SLAB_SIZE = { w: 7, h: 0.46, d: 4.7 } as const;
const SEPARATION = 0.75;

type StreamSpec = { count: number; order: number; size: number; speed: number; cols: number; rows: number };
const STREAMS: StreamSpec[] = [
  // Into Bronze: many, loose and uneven.
  { count: 460, order: 0, size: 2.6, speed: 0.1, cols: 22, rows: 14 },
  // Into Silver: aligned into rows and columns.
  { count: 210, order: 1, size: 3.2, speed: 0.09, cols: 18, rows: 7 },
  // Into Gold: a few, bright, evenly spaced.
  { count: 40, order: 1, size: 7.5, speed: 0.07, cols: 8, rows: 5 },
];

export type Slabs = {
  group: THREE.Group;
  update: (args: { time: number; focus: number; reveal: number; pixelRatio: number }) => void;
  /** Narrow screens shrink the slabs sideways (x and z) so the stack fits the width. */
  setFootprint: (scale: number) => void;
  /** The light that makes the glass catch something; follows the focus. */
  light: THREE.PointLight;
  dispose: () => void;
};

export function createSlabs(lowQuality: boolean): Slabs {
  const group = new THREE.Group();
  const disposables: { dispose: () => void }[] = [];

  const bodyGeo = new RoundedBoxGeometry(SLAB_SIZE.w, SLAB_SIZE.h, SLAB_SIZE.d, lowQuality ? 3 : 6, 0.22);
  disposables.push(bodyGeo);
  const surfaceGeo = new THREE.PlaneGeometry(SLAB_SIZE.w - 0.8, SLAB_SIZE.d - 0.8);
  surfaceGeo.rotateX(-Math.PI / 2);
  const haloGeo = new THREE.PlaneGeometry(SLAB_SIZE.w * 1.7, SLAB_SIZE.d * 1.9);
  haloGeo.rotateX(-Math.PI / 2);
  disposables.push(surfaceGeo, haloGeo);

  type SlabParts = {
    root: THREE.Group;
    body: THREE.Mesh;
    bodyMat: THREE.MeshPhysicalMaterial;
    surfaceMat: THREE.ShaderMaterial;
    haloMat: THREE.ShaderMaterial;
    accent: THREE.Color;
  };
  const slabs: SlabParts[] = DEPTHS.layers.map((layer, i) => {
    const accent = new THREE.Color(layer.accent);
    const root = new THREE.Group();
    root.position.y = SLAB_Y[i];

    const bodyMat = new THREE.MeshPhysicalMaterial({
      // Glass tinted by its metal: the accent, pulled well towards the dark.
      color: accent.clone().multiplyScalar(0.32),
      transparent: true,
      opacity: 0.4,
      roughness: 0.1,
      metalness: 0.05,
      clearcoat: 1,
      clearcoatRoughness: 0.05,
      envMapIntensity: 1.8,
      emissive: accent,
      emissiveIntensity: 0.06,
      side: THREE.DoubleSide,
      depthWrite: false,
    });
    const body = new THREE.Mesh(bodyGeo, bodyMat);
    root.add(body);

    const surfaceMat = new THREE.ShaderMaterial({
      uniforms: {
        uTime: { value: 0 },
        uStyle: { value: i },
        uGlow: { value: 0 },
        uReveal: { value: 0 },
        uColor: { value: accent.clone().lerp(new THREE.Color("#ffffff"), 0.25) },
      },
      vertexShader: SLAB_SURFACE_VERT,
      fragmentShader: SLAB_SURFACE_FRAG,
      transparent: true,
      depthWrite: false,
      blending: THREE.AdditiveBlending,
    });
    const surface = new THREE.Mesh(surfaceGeo, surfaceMat);
    surface.position.y = SLAB_SIZE.h / 2 + 0.012;
    surface.renderOrder = 3;
    root.add(surface);

    const haloMat = new THREE.ShaderMaterial({
      uniforms: { uColor: { value: accent.clone() }, uIntensity: { value: 0 } },
      vertexShader: GLOW_VERT,
      fragmentShader: GLOW_FRAG,
      transparent: true,
      depthWrite: false,
      blending: THREE.AdditiveBlending,
    });
    const halo = new THREE.Mesh(haloGeo, haloMat);
    halo.position.y = -SLAB_SIZE.h / 2 - 0.35;
    halo.renderOrder = 2;
    root.add(halo);

    disposables.push(bodyMat, surfaceMat, haloMat);
    group.add(root);
    return { root, body, bodyMat, surfaceMat, haloMat, accent };
  });

  // ---- Streams -------------------------------------------------------------
  const streamScale = lowQuality ? 0.5 : 1;
  const streams = STREAMS.map((spec, i) => {
    const count = Math.max(8, Math.round(spec.count * streamScale));
    const seeds = new Float32Array(count * 4);
    for (let k = 0; k < seeds.length; k++) seeds[k] = Math.random();
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute("position", new THREE.BufferAttribute(new Float32Array(count * 3), 3));
    geometry.setAttribute("aSeed", new THREE.BufferAttribute(seeds, 4));
    const accent = slabs[i].accent;
    const material = new THREE.ShaderMaterial({
      uniforms: {
        uTime: { value: 0 },
        uTop: { value: 0 },
        uBottom: { value: 0 },
        uOrder: { value: spec.order },
        uHalfW: { value: SLAB_SIZE.w / 2 - 0.7 },
        uHalfD: { value: SLAB_SIZE.d / 2 - 0.7 },
        uCols: { value: spec.cols },
        uRows: { value: spec.rows },
        uSpeed: { value: spec.speed },
        uSize: { value: spec.size },
        uPx: { value: 1 },
        uAlpha: { value: 0 },
        uColor: { value: accent.clone().lerp(new THREE.Color("#ffffff"), 0.35) },
      },
      vertexShader: STREAM_VERT,
      fragmentShader: STREAM_FRAG,
      transparent: true,
      depthWrite: false,
      blending: THREE.AdditiveBlending,
    });
    const points = new THREE.Points(geometry, material);
    points.frustumCulled = false;
    points.renderOrder = 4;
    group.add(points);
    disposables.push(geometry, material);
    return { points, material };
  });

  const light = new THREE.PointLight(0x9fd0ff, 0, 40, 2);
  light.position.set(3, SLAB_Y[0] + 4, 5);
  group.add(light);

  let footprint = 1;
  return {
    group,
    light,
    setFootprint(scale) {
      footprint = scale;
      for (const s of slabs) s.root.scale.set(scale, 1, scale);
    },
    update({ time, focus, reveal, pixelRatio }) {
      group.visible = reveal > 0.003;
      if (!group.visible) return;
      const ys: number[] = [];
      for (let i = 0; i < slabs.length; i++) {
        const s = slabs[i];
        const d = i - focus;
        // 1 at the focused slab, falling to 0 a full slab away.
        const w = clamp01(1 - Math.abs(d) * 1.25);
        const lift = -Math.max(-1, Math.min(1, d)) * SEPARATION;
        s.root.position.y = SLAB_Y[i] + lift;
        ys.push(s.root.position.y);
        const grow = 1 + 0.035 * w;
        s.root.scale.set(footprint * grow, 1, footprint * grow);
        s.bodyMat.opacity = (0.3 + 0.2 * w) * reveal;
        s.bodyMat.emissiveIntensity = 0.04 + 0.4 * w;
        s.surfaceMat.uniforms.uTime.value = time;
        s.surfaceMat.uniforms.uGlow.value = w;
        s.surfaceMat.uniforms.uReveal.value = reveal;
        s.haloMat.uniforms.uIntensity.value = (0.1 + 0.55 * w) * reveal;
      }
      for (let i = 0; i < streams.length; i++) {
        const m = streams[i].material;
        // Each stream feeds the slab at the same index; the first starts from
        // well above Bronze, the others from the slab above them.
        const top = i === 0 ? ys[0] + 6.5 : ys[i - 1] - SLAB_SIZE.h / 2;
        const bottom = ys[i] + SLAB_SIZE.h / 2 + 0.05;
        m.uniforms.uTop.value = top;
        m.uniforms.uBottom.value = bottom;
        m.uniforms.uTime.value = time;
        m.uniforms.uPx.value = pixelRatio;
        m.uniforms.uHalfW.value = (SLAB_SIZE.w / 2 - 0.7) * footprint;
        m.uniforms.uHalfD.value = (SLAB_SIZE.d / 2 - 0.7) * footprint;
        const w = clamp01(1 - Math.abs(i - focus) * 1.1);
        m.uniforms.uAlpha.value = (0.22 + 0.78 * w) * reveal;
      }
      light.intensity = 26 * reveal;
      light.position.set(2.5, ys[Math.round(Math.max(0, Math.min(2, focus)))] + 3.4, 4.5);
    },
    dispose() {
      for (const d of disposables) d.dispose();
    },
  };
}
