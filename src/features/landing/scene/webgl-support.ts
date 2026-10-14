/**
 * True when this browser can create a WebGL2 context, which is what three.js
 * (r163 and later) requires. The probe context is released straight away so
 * it does not count against the browser's limit of live contexts.
 */
export function hasWebGL2(): boolean {
  try {
    const probe = document.createElement("canvas");
    const gl = probe.getContext("webgl2");
    if (!gl) return false;
    gl.getExtension("WEBGL_lose_context")?.loseContext();
    return true;
  } catch {
    return false;
  }
}
