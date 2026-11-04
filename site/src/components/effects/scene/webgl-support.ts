let cached: boolean | null = null;

/**
 * Whether the 3D lake is worth running here (WEB-5 D5, a port of the RantAI
 * site's `lib/webgl-support.ts`): the browser can create a WebGL2 context,
 * which three.js r163 and later requires, and it is drawn by a real GPU.
 *
 * Without one (VMs, remote desktops, browsers with hardware acceleration
 * off) the browser emulates the GPU on the CPU, and the scene would freeze
 * the page for seconds, so `failIfMajorPerformanceCaveat` refuses such a
 * context and software renderers (SwiftShader, llvmpipe) are rejected by
 * name. The answer is `false` and the page keeps the DOM hero. The probe
 * context is released straight away so it does not count against the
 * browser's limit of live contexts.
 */
export function hasFastWebGL(): boolean {
  if (cached !== null) return cached;
  if (typeof document === "undefined") return false;
  try {
    const canvas = document.createElement("canvas");
    const gl = canvas.getContext("webgl2", { failIfMajorPerformanceCaveat: true });
    if (!gl) return (cached = false);
    const info = gl.getExtension("WEBGL_debug_renderer_info");
    const renderer = info ? String(gl.getParameter(info.UNMASKED_RENDERER_WEBGL)) : "";
    gl.getExtension("WEBGL_lose_context")?.loseContext();
    cached = !/swiftshader|llvmpipe|softpipe|software|basic render/i.test(renderer);
  } catch {
    cached = false;
  }
  return cached;
}
