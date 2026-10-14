import * as THREE from "three";
import { SHAFT_FRAG, SHAFT_VERT, SNOW_FRAG, SNOW_VERT } from "./shaders";

/**
 * What makes the water feel like water once the camera is under it: light
 * shafts falling from the surface and marine snow drifting past. Both are
 * additive and cost a handful of draw calls.
 */

export type Shafts = {
  group: THREE.Group;
  /** `intensity` 0..1; `camera` orients every shaft towards the viewer. */
  update: (time: number, intensity: number, camera: THREE.Camera) => void;
  dispose: () => void;
};

export function createShafts(count: number): Shafts {
  const group = new THREE.Group();
  const geometry = new THREE.PlaneGeometry(1, 1);
  const materials: THREE.ShaderMaterial[] = [];
  const shafts: { mesh: THREE.Mesh; tilt: number }[] = [];
  for (let i = 0; i < count; i++) {
    const u = (i + 0.5) / count;
    const material = new THREE.ShaderMaterial({
      uniforms: {
        uTime: { value: 0 },
        uIntensity: { value: 0 },
        uSeed: { value: (i * 0.618) % 1 },
        uColor: { value: new THREE.Color("#7cc4ff") },
      },
      vertexShader: SHAFT_VERT,
      fragmentShader: SHAFT_FRAG,
      transparent: true,
      depthWrite: false,
      blending: THREE.AdditiveBlending,
      side: THREE.DoubleSide,
    });
    materials.push(material);
    const mesh = new THREE.Mesh(geometry, material);
    const width = 2.6 + ((i * 7) % 5) * 0.9;
    mesh.scale.set(width, 40, 1);
    // Spread across and behind the scene; the centre stays clearer.
    mesh.position.set((u - 0.5) * 30 + Math.sin(i * 2.3) * 2, -19, -8 + ((i * 5) % 7) * 1.4);
    mesh.rotation.order = "YXZ";
    mesh.frustumCulled = false;
    mesh.renderOrder = 5;
    group.add(mesh);
    shafts.push({ mesh, tilt: 0.16 + 0.08 * Math.sin(i * 1.7) });
  }
  return {
    group,
    update(time, intensity, camera) {
      group.visible = intensity > 0.002;
      if (!group.visible) return;
      for (let i = 0; i < shafts.length; i++) {
        const { mesh, tilt } = shafts[i];
        const dx = camera.position.x - mesh.position.x;
        const dz = camera.position.z - mesh.position.z;
        mesh.rotation.y = Math.atan2(dx, dz);
        mesh.rotation.z = tilt;
        const m = materials[i];
        m.uniforms.uTime.value = time;
        m.uniforms.uIntensity.value = intensity * (0.5 + 0.5 * Math.sin(time * 0.17 + i * 2.1));
      }
    },
    dispose() {
      geometry.dispose();
      for (const m of materials) m.dispose();
    },
  };
}

export type Snow = {
  points: THREE.Points;
  update: (time: number, alpha: number, camera: THREE.Camera, pixelRatio: number) => void;
  dispose: () => void;
};

export function createSnow(count: number): Snow {
  const seeds = new Float32Array(count * 4);
  for (let i = 0; i < seeds.length; i++) seeds[i] = Math.random();
  const geometry = new THREE.BufferGeometry();
  // `position` is required by three even though the vertex shader derives the
  // real position from `aSeed`.
  geometry.setAttribute("position", new THREE.BufferAttribute(new Float32Array(count * 3), 3));
  geometry.setAttribute("aSeed", new THREE.BufferAttribute(seeds, 4));
  const material = new THREE.ShaderMaterial({
    uniforms: {
      uTime: { value: 0 },
      uCam: { value: new THREE.Vector3() },
      uPx: { value: 1 },
      uAlpha: { value: 0 },
      uColor: { value: new THREE.Color("#bfe0ff") },
    },
    vertexShader: SNOW_VERT,
    fragmentShader: SNOW_FRAG,
    transparent: true,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
  });
  const points = new THREE.Points(geometry, material);
  points.frustumCulled = false;
  points.renderOrder = 6;
  return {
    points,
    update(time, alpha, camera, pixelRatio) {
      points.visible = alpha > 0.002;
      material.uniforms.uTime.value = time;
      material.uniforms.uAlpha.value = alpha;
      material.uniforms.uPx.value = pixelRatio;
      (material.uniforms.uCam.value as THREE.Vector3).copy(camera.position);
    },
    dispose() {
      geometry.dispose();
      material.dispose();
    },
  };
}
