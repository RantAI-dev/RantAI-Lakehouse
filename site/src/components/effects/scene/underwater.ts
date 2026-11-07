// WEB-5 D5: named imports, so the scene chunk carries only what it uses.
import {
  AdditiveBlending,
  BufferAttribute,
  BufferGeometry,
  Camera,
  Color,
  DoubleSide,
  Group,
  Material,
  Mesh,
  PlaneGeometry,
  Points,
  ShaderMaterial,
  Vector3,
  type IUniform,
} from "three";
import { CONTACT_GLSL, NOISE_GLSL, SHAFT_FRAG, SHAFT_VERT, SNOW_FRAG, SNOW_VERT, WATER_FOG_GLSL } from "./shaders";

/**
 * What makes the water feel like water once the camera is under it: the
 * water's own fog, light shafts falling from the surface and marine snow
 * drifting past. The shafts and the snow are additive and cost a handful of
 * draw calls.
 */

/** The uniforms of `WATER_FOG_GLSL`, shared by every material that uses it. */
export type WaterFogUniforms = {
  /** 1 while the under-water picture is drawn, 0 for the one above. */
  uWfOn: IUniform<number>;
  uWfUp: IUniform<Color>;
  uWfHorizon: IUniform<Color>;
  uWfDown: IUniform<Color>;
  uWfDensity: IUniform<number>;
  uAirFog: IUniform<Color>;
  uAirDensity: IUniform<number>;
  /** The clock, for the caustics on the creature (`applyWaterFog` with `caustics`). */
  uWfTime: IUniform<number>;
};

export function createWaterFog(airFog: Color): WaterFogUniforms {
  return {
    uWfOn: { value: 0 },
    uWfUp: { value: new Color("#1b5aa3") },
    uWfHorizon: { value: new Color("#0b3a74") },
    uWfDown: { value: new Color("#041640") },
    uWfDensity: { value: 0.045 },
    uAirFog: { value: airFog },
    uAirDensity: { value: 0.003 },
    uWfTime: { value: 0 },
  };
}

/**
 * Gives one of three's own materials (the creature, the slabs) the water's
 * fog in place of three's: the same function the sky and the water use, so
 * a fogged object always fades into exactly the colour behind it.
 *
 * With `caustics` (a lit material: the creature's body), under water the
 * moonlight that the waves focus plays over its upper side, strongest near
 * the surface (O5; the owner's O8 asks for the body "lit by the underwater
 * light and caustics").
 */
export function applyWaterFog(material: Material, fog: WaterFogUniforms, opts: { caustics?: boolean } = {}): void {
  const caustics = opts.caustics === true;
  material.onBeforeCompile = (shader) => {
    Object.assign(shader.uniforms, fog);
    shader.vertexShader = shader.vertexShader
      .replace("#include <fog_pars_vertex>", "varying vec3 vWfWorld;")
      .replace("#include <fog_vertex>", "vWfWorld = (modelMatrix * vec4(transformed, 1.0)).xyz;");
    let frag = shader.fragmentShader
      .replace(
        "#include <fog_pars_fragment>",
        `varying vec3 vWfWorld;\n${WATER_FOG_GLSL}\n${caustics ? NOISE_GLSL + CONTACT_GLSL : ""}`,
      )
      .replace(
        "#include <fog_fragment>",
        caustics
          ? // The creature's contact with the surface, seen from below (O8, CONTACT_GLSL).
            `if (uWfOn > 0.5) gl_FragColor.rgb = mix(gl_FragColor.rgb, surfaceMirror(vWfWorld), surfaceContact(vWfWorld));
             gl_FragColor.rgb = waterFog(gl_FragColor.rgb, vWfWorld);`
          : "gl_FragColor.rgb = waterFog(gl_FragColor.rgb, vWfWorld);",
      );
    if (caustics) {
      frag = frag.replace(
        "#include <emissivemap_fragment>",
        `#include <emissivemap_fragment>
        if (uWfOn > 0.5) {
          vec3 nW = normalize((vec4(normal, 0.0) * viewMatrix).xyz);
          vec2 cp = vWfWorld.xz * 1.1 + vec2(vWfWorld.y * 0.3, 0.0);
          float c1 = 1.0 - abs(snoise(cp + vec2(uWfTime * 0.16, uWfTime * 0.11)));
          float c2 = 1.0 - abs(snoise(cp * 1.7 - vec2(uWfTime * 0.12, -uWfTime * 0.17)));
          float lit = smoothstep(-0.2, 0.8, nW.y) * exp(min(vWfWorld.y, 0.0) / 3.5);
          totalEmissiveRadiance += vec3(0.32, 0.62, 1.0) * pow(max(c1 * c2, 0.0), 3.0) * 0.55 * lit;
        }`,
      );
    }
    shader.fragmentShader = frag;
  };
  material.customProgramCacheKey = () => (caustics ? "lk-water-fog-caustics" : "lk-water-fog");
}

export type Shafts = {
  group: Group;
  /** `intensity` 0..1; `camera` orients every shaft towards the viewer. */
  update: (time: number, intensity: number, camera: Camera) => void;
  dispose: () => void;
};

export function createShafts(count: number): Shafts {
  const group = new Group();
  const geometry = new PlaneGeometry(1, 1);
  const materials: ShaderMaterial[] = [];
  const shafts: { mesh: Mesh; tilt: number }[] = [];
  for (let i = 0; i < count; i++) {
    const u = (i + 0.5) / count;
    const material = new ShaderMaterial({
      uniforms: {
        uTime: { value: 0 },
        uIntensity: { value: 0 },
        uSeed: { value: (i * 0.618) % 1 },
        uColor: { value: new Color("#7cc4ff") },
      },
      vertexShader: SHAFT_VERT,
      fragmentShader: SHAFT_FRAG,
      transparent: true,
      depthWrite: false,
      blending: AdditiveBlending,
      side: DoubleSide,
    });
    materials.push(material);
    const mesh = new Mesh(geometry, material);
    const width = 3.2 + ((i * 7) % 5) * 1.1;
    mesh.scale.set(width, 40, 1);
    // Spread across and behind the scene; the centre stays clearer.
    // Behind the creature (O8: a shaft in front of the whale read as a stripe on its body).
    mesh.position.set((u - 0.5) * 34 + Math.sin(i * 2.3) * 2, -19, -16 + ((i * 5) % 7) * 1.2);
    mesh.rotation.order = "YXZ";
    mesh.frustumCulled = false;
    mesh.renderOrder = 5;
    group.add(mesh);
    shafts.push({ mesh, tilt: 0.1 + 0.06 * Math.sin(i * 1.7) });
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
        // A slow sway, as the surface they come through moves (O5).
        mesh.rotation.z = tilt + 0.035 * Math.sin(time * 0.21 + i * 1.9) + 0.015 * Math.sin(time * 0.53 + i);
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
  points: Points;
  update: (time: number, alpha: number, camera: Camera, pixelRatio: number) => void;
  dispose: () => void;
};

export function createSnow(count: number): Snow {
  const seeds = new Float32Array(count * 4);
  for (let i = 0; i < seeds.length; i++) seeds[i] = Math.random();
  const geometry = new BufferGeometry();
  // `position` is required by three even though the vertex shader derives the
  // real position from `aSeed`.
  geometry.setAttribute("position", new BufferAttribute(new Float32Array(count * 3), 3));
  geometry.setAttribute("aSeed", new BufferAttribute(seeds, 4));
  const material = new ShaderMaterial({
    uniforms: {
      uTime: { value: 0 },
      uCam: { value: new Vector3() },
      uPx: { value: 1 },
      uAlpha: { value: 0 },
      uColor: { value: new Color("#bfe0ff") },
    },
    vertexShader: SNOW_VERT,
    fragmentShader: SNOW_FRAG,
    transparent: true,
    depthWrite: false,
    blending: AdditiveBlending,
  });
  const points = new Points(geometry, material);
  points.frustumCulled = false;
  points.renderOrder = 6;
  return {
    points,
    update(time, alpha, camera, pixelRatio) {
      points.visible = alpha > 0.002;
      material.uniforms.uTime.value = time;
      material.uniforms.uAlpha.value = alpha;
      material.uniforms.uPx.value = pixelRatio;
      (material.uniforms.uCam.value as Vector3).copy(camera.position);
    },
    dispose() {
      geometry.dispose();
      material.dispose();
    },
  };
}
