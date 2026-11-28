// WEB-5 D5: named imports, so the scene chunk carries only what it uses.
import {
  Color,
  HalfFloatType,
  Mesh,
  OrthographicCamera,
  PlaneGeometry,
  Scene,
  ShaderMaterial,
  UnsignedByteType,
  Vector2,
  WebGLRenderTarget,
  type WebGLRenderer,
} from "three";
import { LENS_FRAG, LENS_VERT } from "./shaders";

/**
 * WEB-5 O5: the camera's lens, as one full-screen pass (`LENS_FRAG`).
 *
 * The owner found the old crossing "patah": a flat bar drawn over the canvas
 * as the waterline, with a hop of the camera across the surface. Now the
 * scene is drawn off screen, once from above the water and, while the
 * camera is at the surface, once more from below, and this pass splits the
 * screen between the two along the water standing on the lens. One camera,
 * one continuous move.
 *
 * Budget (owner, O8: one extra full-screen pass at most):
 * - this is the only extra pass, and it runs every frame, so the hero, the
 *   crossing and the deep water are all graded by the same tone mapping;
 * - the scene is drawn twice only while the waterline is on screen (about
 *   one viewport of scroll); everywhere else it is drawn once;
 * - the pictures are half-float, so the dark water does not band; desktop
 *   keeps 4x MSAA on the main picture (the canvas itself no longer needs
 *   it), phones have none, as before;
 * - the second picture is created the first time the camera reaches the
 *   surface, not before.
 */

export type LensFrame = {
  /** 0 above only, 1 below only, 2 both (the waterline is on screen). */
  mode: 0 | 1 | 2;
  time: number;
  camY: number;
  camX: number;
  camZ: number;
  /** The camera's right vector on the water plane (x, z). */
  rightX: number;
  rightZ: number;
  /** 0..1: how much the under-water picture wobbles. */
  wobble: number;
};

export type Lens = {
  /** Where the above-water picture (or the only one) is drawn. */
  main: WebGLRenderTarget;
  /** Where the below-water picture is drawn while both are needed. */
  second: () => WebGLRenderTarget;
  setSize: (width: number, height: number, pixelRatio: number) => void;
  /** Puts the frame on screen from `main` (and `second`, in mode 2). */
  present: (renderer: WebGLRenderer, frame: LensFrame) => void;
  dispose: () => void;
};

export function createLens(renderer: WebGLRenderer, opts: { lensHalf: number; msaa: boolean }): Lens {
  // Half float where the GPU can draw into it (WebGL2 with
  // EXT_color_buffer_float or _half_float, which every current phone and
  // desktop browser has); 8-bit otherwise, which only risks banding.
  const ext = renderer.extensions;
  const type =
    ext.has("EXT_color_buffer_float") || ext.has("EXT_color_buffer_half_float") ? HalfFloatType : UnsignedByteType;
  const size = new Vector2(1, 1);
  const main = new WebGLRenderTarget(1, 1, { type, samples: opts.msaa ? 4 : 0, depthBuffer: true });
  let second: WebGLRenderTarget | null = null;

  const uniforms = {
    tA: { value: main.texture },
    tB: { value: main.texture },
    uMode: { value: 0 },
    uTime: { value: 0 },
    uCamY: { value: 1 },
    uLens: { value: opts.lensHalf },
    uAspect: { value: 1 },
    uCamXZ: { value: new Vector2() },
    uRight: { value: new Vector2(1, 0) },
    uRes: { value: new Vector2(1, 1) },
    uWobble: { value: 0 },
    // Linear light, before tone mapping: a faint cold glint.
    uMeniscus: { value: new Color("#9fd4ff").multiplyScalar(0.16) },
  };
  const material = new ShaderMaterial({
    uniforms,
    vertexShader: LENS_VERT,
    fragmentShader: LENS_FRAG,
    depthTest: false,
    depthWrite: false,
  });
  const geometry = new PlaneGeometry(2, 2);
  const quad = new Mesh(geometry, material);
  quad.frustumCulled = false;
  const scene = new Scene();
  scene.add(quad);
  const camera = new OrthographicCamera(-1, 1, 1, -1, 0, 1);

  return {
    main,
    second() {
      if (!second) {
        // No MSAA here: this picture is only ever seen through water.
        second = new WebGLRenderTarget(size.x, size.y, { type, samples: 0, depthBuffer: true });
      }
      return second;
    },
    setSize(width, height, pixelRatio) {
      size.set(Math.max(1, Math.round(width * pixelRatio)), Math.max(1, Math.round(height * pixelRatio)));
      main.setSize(size.x, size.y);
      second?.setSize(size.x, size.y);
      uniforms.uRes.value.copy(size);
      uniforms.uAspect.value = width / Math.max(1, height);
    },
    present(r, f) {
      uniforms.uMode.value = f.mode;
      uniforms.uTime.value = f.time;
      uniforms.uCamY.value = f.camY;
      uniforms.uCamXZ.value.set(f.camX, f.camZ);
      uniforms.uRight.value.set(f.rightX, f.rightZ);
      uniforms.uWobble.value = f.wobble;
      uniforms.tA.value = main.texture;
      uniforms.tB.value = f.mode === 1 ? main.texture : (second ?? main).texture;
      r.setRenderTarget(null);
      r.render(scene, camera);
    },
    dispose() {
      main.dispose();
      second?.dispose();
      geometry.dispose();
      material.dispose();
    },
  };
}
