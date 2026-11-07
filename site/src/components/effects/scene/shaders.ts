import { RIPPLES } from "./scene-config";

/**
 * GLSL for the lake. Kept as strings in one module so each program's contract
 * (uniforms, varyings) sits next to its siblings. All colours arrive as
 * THREE.Color uniforms, so they are already linear; the programs end with
 * three's tone-mapping and colour-space chunks so the canvas matches the page's
 * sRGB palette.
 */

/** 2D simplex noise (Ashima / McEwan, MIT). Returns about -1..1. */
export const NOISE_GLSL = /* glsl */ `
vec3 mod289(vec3 x){ return x - floor(x * (1.0 / 289.0)) * 289.0; }
vec2 mod289(vec2 x){ return x - floor(x * (1.0 / 289.0)) * 289.0; }
vec3 permute(vec3 x){ return mod289(((x * 34.0) + 10.0) * x); }
float snoise(vec2 v){
  const vec4 C = vec4(0.211324865405187, 0.366025403784439, -0.577350269189626, 0.024390243902439);
  vec2 i = floor(v + dot(v, C.yy));
  vec2 x0 = v - i + dot(i, C.xx);
  vec2 i1 = (x0.x > x0.y) ? vec2(1.0, 0.0) : vec2(0.0, 1.0);
  vec4 x12 = x0.xyxy + C.xxzz;
  x12.xy -= i1;
  i = mod289(i);
  vec3 p = permute(permute(i.y + vec3(0.0, i1.y, 1.0)) + i.x + vec3(0.0, i1.x, 1.0));
  vec3 m = max(0.5 - vec3(dot(x0, x0), dot(x12.xy, x12.xy), dot(x12.zw, x12.zw)), 0.0);
  m = m * m; m = m * m;
  vec3 x = 2.0 * fract(p * C.www) - 1.0;
  vec3 h = abs(x) - 0.5;
  vec3 ox = floor(x + 0.5);
  vec3 a0 = x - ox;
  m *= 1.79284291400159 - 0.85373472095314 * (a0 * a0 + h * h);
  vec3 g;
  g.x = a0.x * x0.x + h.x * x0.y;
  g.yz = a0.yz * x12.xz + h.yz * x12.yw;
  return 130.0 * dot(m, g);
}
float hash11(float p){ p = fract(p * 0.1031); p *= p + 33.33; p *= p + p; return fract(p); }
`;

const OUTPUT_GLSL = /* glsl */ `
#include <tonemapping_fragment>
#include <colorspace_fragment>
`;

/** Sky gradient shared by the sky dome and the water's reflection. */
export const SKY_GRADIENT_GLSL = /* glsl */ `
uniform vec3 uSkyHorizon;
uniform vec3 uSkyMid;
uniform vec3 uSkyZenith;
vec3 skyGradient(vec3 d){
  float h = d.y;
  float t = clamp(h, 0.0, 1.0);
  vec3 col = mix(uSkyHorizon, uSkyMid, smoothstep(0.0, 0.2, t));
  col = mix(col, uSkyZenith, smoothstep(0.1, 0.8, t));
  // Only the environment map ever looks below the horizon.
  col = mix(col, uSkyHorizon * 0.3, smoothstep(-0.3, 0.0, h) * -1.0 + 1.0);
  return col;
}
`;

/**
 * WEB-5 O5: one fog for everything the camera sees, so nothing can show an
 * edge against anything else. Above the water it is the old night haze (a
 * single colour, exp2). Below it, light is absorbed per channel with
 * distance (red first, then green), and the colour the water fades to
 * depends on the direction you look: lighter towards the surface, the lake's
 * blue at eye level, darker towards the depths, so the far water is a soft
 * gradient with no horizon line. The sky dome, the water's underside and
 * every mesh material (patched in `underwater.ts`) all call `waterFog`.
 */
export const WATER_FOG_GLSL = /* glsl */ `
uniform float uWfOn;
uniform vec3 uWfUp;
uniform vec3 uWfHorizon;
uniform vec3 uWfDown;
uniform float uWfDensity;
uniform vec3 uAirFog;
uniform float uAirDensity;
uniform float uWfTime;
vec3 waterFogColor(vec3 d){
  vec3 c = mix(uWfDown, uWfHorizon, smoothstep(-0.5, 0.06, d.y));
  return mix(c, uWfUp, smoothstep(0.0, 0.8, d.y));
}
vec3 waterFog(vec3 col, vec3 worldPos){
  vec3 toP = worldPos - cameraPosition;
  float dist = length(toP);
  vec3 d = toP / max(dist, 1e-4);
  if (uWfOn > 0.5) {
    // A little faster than linear, so the near water stays clear and the
    // far water is fully fogged well before the lake's plane ends.
    float k = pow(dist * uWfDensity, 1.35);
    vec3 T = exp(-k * vec3(1.5, 1.08, 0.85));
    return col * T + waterFogColor(d) * (1.0 - T);
  }
  float f = 1.0 - exp(-pow(dist * uAirDensity, 2.0));
  return mix(col, uAirFog, clamp(f, 0.0, 1.0));
}
`;

/**
 * WEB-5 O8 (planner's SHOULD-FIX on O5): where the creature meets the
 * surface, seen from below. The water plane is flat, so the line where it
 * cuts the creature was perfectly straight. Under water, the creature fades
 * into the surface's own mirror colour over a band just under the surface
 * whose top moves with the waves, and its reflection in the surface (in
 * WATER_FRAG) fades the same way, so the two meet in a soft, moving
 * meniscus instead of an edge. Needs NOISE_GLSL and WATER_FOG_GLSL.
 */
export const CONTACT_GLSL = /* glsl */ `
float surfaceContact(vec3 wp){
  float w = snoise(wp.xz * 1.4 + vec2(uWfTime * 0.33, -uWfTime * 0.26)) * 0.07
          + snoise(wp.xz * 3.1 - vec2(uWfTime * 0.5, uWfTime * 0.41)) * 0.025;
  return smoothstep(-0.42, -0.04, wp.y + w);
}
vec3 surfaceMirror(vec3 wp){
  vec3 d = normalize(wp - cameraPosition);
  return waterFogColor(normalize(vec3(d.x, -0.06, d.z)));
}
`;

export const SKY_VERT = /* glsl */ `
varying vec3 vDir;
void main(){
  vDir = normalize(position);
  vec4 p = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
  // Pin to the far plane so the dome is always behind everything.
  gl_Position = p.xyww;
}
`;

export const SKY_FRAG = /* glsl */ `
uniform vec3 uMoonDir;
uniform vec3 uMoonColor;
uniform float uMoonBoost;
uniform float uUnder;
varying vec3 vDir;
${SKY_GRADIENT_GLSL}
${WATER_FOG_GLSL}
void main(){
  vec3 d = normalize(vDir);
  vec3 col = skyGradient(d);
  float m = dot(d, uMoonDir);
  float disc = smoothstep(0.99978, 0.99988, m);
  float halo = pow(max(m, 0.0), 900.0) * 0.5 + pow(max(m, 0.0), 60.0) * 0.12 + pow(max(m, 0.0), 8.0) * 0.03;
  col += uMoonColor * (disc * uMoonBoost + halo);
  // Under the surface the "sky" is the water itself, far away: the same
  // direction-dependent colour every fogged object fades to (O5), so the
  // far water is a gradient with no horizon line.
  col = mix(col, waterFogColor(d), uUnder);
  gl_FragColor = vec4(col, 1.0);
  ${OUTPUT_GLSL}
}
`;

export const STARS_VERT = /* glsl */ `
attribute float aSize;
attribute float aPhase;
attribute float aBright;
uniform float uTime;
uniform float uPx;
uniform float uAlpha;
varying float vA;
void main(){
  vec4 mv = modelViewMatrix * vec4(position, 1.0);
  gl_Position = projectionMatrix * mv;
  float elev = clamp(position.y / 250.0, 0.0, 1.0);
  float tw = 0.62 + 0.38 * sin(uTime * (0.9 + aPhase * 1.6) + aPhase * 40.0);
  vA = aBright * tw * smoothstep(0.03, 0.3, elev) * uAlpha;
  gl_PointSize = aSize * uPx;
}
`;

export const STARS_FRAG = /* glsl */ `
uniform vec3 uColor;
varying float vA;
void main(){
  float d = length(gl_PointCoord - 0.5);
  float a = 1.0 - smoothstep(0.0, 0.5, d);
  a = a * a * 1.6;
  gl_FragColor = vec4(uColor * a * vA, a * vA);
  ${OUTPUT_GLSL}
}
`;

export const WATER_VERT = /* glsl */ `
varying vec3 vWorld;
void main(){
  vec4 w = modelMatrix * vec4(position, 1.0);
  vWorld = w.xyz;
  gl_Position = projectionMatrix * viewMatrix * w;
}
`;

/**
 * The lake surface. One plane, no vertex displacement: the waves are normal
 * perturbation from three simplex layers plus radial ripples, which is
 * enough at the grazing angles the camera sees and keeps the vertex stage
 * free. Seen from above it reflects the sky and the moon with a fresnel
 * term. Seen from below (`gl_FrontFacing` false) it is the surface as a
 * diver sees it (WEB-5 O5):
 * - Snell's window: within about 49° of straight up the ray leaves the
 *   water, and you see the night sky squeezed into a bright disc, with the
 *   moon near its rim and soft caustic light;
 * - beyond it, total internal reflection: the surface is a dark, rippled
 *   mirror of the water below;
 * - the two meet in a soft rim, never a line;
 * - far away (where a pixel covers more water than the waves' scale) the
 *   ripples flatten out, so the grazing ceiling never turns into streaks or
 *   a band of shimmering noise;
 * - the above-water part of the creature shows as a soft dark shape in the
 *   window (`uMascot`), since it blocks the light from the sky.
 */
export const WATER_FRAG = /* glsl */ `
#define RIPPLES ${RIPPLES}
uniform float uTime;
uniform vec3 uFogColor;
uniform float uFogDensity;
uniform vec3 uMoonDir;
uniform vec3 uMoonColor;
uniform vec3 uDeep;
uniform vec3 uLake;
uniform vec4 uRip[RIPPLES];
uniform vec3 uMascot;
uniform float uWaveGain;
uniform vec3 uBodyC;
uniform vec3 uBodyR;
uniform vec3 uBodyTint;
varying vec3 vWorld;
${NOISE_GLSL}
${SKY_GRADIENT_GLSL}
${WATER_FOG_GLSL}
${CONTACT_GLSL}

float baseH(vec2 p){
  float t = uTime;
  float h = snoise(p * 0.24 + vec2(t * 0.04, t * 0.027)) * 0.6;
  h += snoise(p * 0.62 + vec2(-t * 0.06, t * 0.05)) * 0.24;
  #ifndef LOW_QUALITY
  h += snoise(p * 1.5 + vec2(t * 0.1, -t * 0.07)) * 0.09;
  #endif
  return h;
}

vec2 rippleGrad(vec2 p){
  vec2 g = vec2(0.0);
  for (int i = 0; i < RIPPLES; i++) {
    vec4 r = uRip[i];
    float age = uTime - r.z;
    if (age < 0.0 || age > 7.0) continue;
    vec2 d = p - r.xy;
    float dist = length(d) + 1e-4;
    float x = dist - (uMascot.z * 0.95 + age * 1.25);
    float env = exp(-x * x / 0.4) * exp(-age * 0.5) * r.w;
    float k = 7.5;
    float dh = env * (k * cos(k * x) - (2.0 * x / 0.4) * sin(k * x));
    g += dh * d / dist;
  }
  return g;
}

void main(){
  vec3 toCam = cameraPosition - vWorld;
  float dist = length(toCam);
  vec3 V = toCam / dist;
  vec2 p = vWorld.xz;

  vec2 g;
  {
    float e = 0.05;
    float h0 = baseH(p);
    g = vec2(baseH(p + vec2(e, 0.0)) - h0, baseH(p + vec2(0.0, e)) - h0) / e;
  }
  g = g * uWaveGain + rippleGrad(p);
  float lod = 1.0 / (1.0 + dist * 0.05);

  vec3 col;
  if (gl_FrontFacing) {
    vec3 n = normalize(vec3(-g.x * 0.2 * lod, 1.0, -g.y * 0.2 * lod));
    float ndv = clamp(dot(n, V), 0.0, 1.0);
    float fres = 0.03 + 0.97 * pow(1.0 - ndv, 4.5);
    vec3 R = reflect(-V, n);
    R.y = abs(R.y);
    vec3 sky = skyGradient(R);
    float m = max(dot(R, uMoonDir), 0.0);
    float spec = pow(m, 400.0) * 2.4 + pow(m, 90.0) * 0.28 + pow(m, 22.0) * 0.03;
    float glint = smoothstep(0.6, 0.98, snoise(p * 7.0 + vec2(uTime * 0.3, -uTime * 0.18))) * pow(m, 40.0) * 1.0;
    vec3 body = mix(uDeep, uLake * 0.09, pow(1.0 - ndv, 2.0));
    col = mix(body, sky, fres);
    col += uMoonColor * (spec + glint) * (0.25 + fres);

    // Contact with the mascot: darker water around it and a thin lit rim.
    float dm = length(p - uMascot.xy);
    float shade = 1.0 - smoothstep(uMascot.z, uMascot.z + 1.5, dm);
    col *= 1.0 - 0.6 * shade;
    float rimX = (dm - uMascot.z) / 0.14;
    float rim = exp(-rimX * rimX);
    col += uLake * rim * 0.4;

    float f = 1.0 - exp(-pow(dist * uFogDensity, 2.0));
    col = mix(col, uFogColor, clamp(f, 0.0, 1.0));
  } else {
    // How much water one pixel covers: where it is more than a ripple's
    // width, the ripples average out to a flat, calm mirror.
    float fw = length(fwidth(p));
    // ...and so do they where the ceiling is seen nearly edge-on.
    float detail = (1.0 - smoothstep(0.03, 0.26, fw)) * smoothstep(0.03, 0.3, -V.y);
    vec3 n = normalize(vec3(-g.x * 0.2 * lod * detail, 1.0, -g.y * 0.2 * lod * detail));
    vec3 Vd = -V;
    float dm = length(p - uMascot.xy);
    float cosI = clamp(dot(Vd, n), 0.0, 1.0);
    // The window's edge (the critical angle, cos 0.66), softened on both sides.
    float win = smoothstep(0.6, 0.76, cosI);
    vec3 air = refract(Vd, -n, 1.333);
    if (dot(air, air) < 0.5) air = Vd;
    float m = max(dot(normalize(air), uMoonDir), 0.0);
    vec3 sky = skyGradient(air) * 1.8 + uLake * 0.16;
    sky += uMoonColor * (pow(m, 260.0) * 2.2 + pow(m, 24.0) * 0.12);
    // Soft caustic light playing over the window.
    vec2 cp = p * 0.5 + g * 0.6 * detail;
    float c1 = 1.0 - abs(snoise(cp + vec2(uTime * 0.11, uTime * 0.08)));
    float c2 = 1.0 - abs(snoise(cp * 1.6 - vec2(uTime * 0.09, -uTime * 0.12)));
    sky += vec3(0.35, 0.62, 0.95) * pow(max(c1 * c2, 0.0), 4.0) * 0.35 * detail;
    // The creature above the water blocks the sky: a soft shadow in the window.
    sky *= mix(0.28, 1.0, smoothstep(uMascot.z * 0.75, uMascot.z * 1.25, dm));
    // Beyond the window the surface mirrors the water below it (always
    // below: a wave can tilt the mirror, never turn it to face the sky),
    // and, above the creature, the creature's own submerged body.
    vec3 R = reflect(Vd, -n);
    R.y = -abs(R.y);
    vec3 mirror = waterFogColor(R) * (1.0 + detail * (0.16 * (1.0 - abs(snoise(p * 0.9 + uTime * 0.05))) - 0.08));
    // The creature's submerged body in the mirror: the reflected ray against
    // the body's ellipsoid (uBodyC, uBodyR), with a soft edge, so the body
    // meets its own reflection at the surface instead of ending in a cut.
    {
      // Mostly the calm mirror, a little of the waves: the reflection
      // wavers without breaking into stripes.
      vec3 Rb = reflect(Vd, normalize(mix(vec3(0.0, -1.0, 0.0), -n, 0.3)));
      Rb.y = -abs(Rb.y);
      vec3 o = (vWorld - uBodyC) / uBodyR;
      vec3 rd = normalize(Rb / uBodyR);
      float tc = max(-dot(o, rd), 0.0);
      float miss = length(o + rd * tc);
      float hit = (1.0 - smoothstep(0.72, 1.08, miss)) * 0.92;
      vec3 hitW = uBodyC + (o + rd * tc) * uBodyR;
      vec3 body = mix(uBodyTint, surfaceMirror(hitW), surfaceContact(hitW));
      mirror = mix(mirror, waterFog(body, hitW), hit);
    }
    col = mix(mirror, sky, win);
    float rimW = (cosI - 0.68) / 0.05;
    col += uLake * 0.12 * exp(-rimW * rimW);
    col = waterFog(col, vWorld);
  }

  gl_FragColor = vec4(col, 1.0);
  ${OUTPUT_GLSL}
}
`;

/** Soft additive disc used for the eye's glow. */
export const GLOW_VERT = /* glsl */ `
varying vec2 vUv;
void main(){
  vUv = uv;
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
}
`;

export const GLOW_FRAG = /* glsl */ `
uniform vec3 uColor;
uniform float uIntensity;
varying vec2 vUv;
void main(){
  float r = length(vUv - 0.5) * 2.0;
  float a = exp(-r * r * 3.2) * (1.0 - smoothstep(0.85, 1.0, r));
  gl_FragColor = vec4(uColor * a * uIntensity, a * uIntensity);
  ${OUTPUT_GLSL}
}
`;

/** The foam collar where the dome meets the water; wobbles and breathes with the bob. */
export const FOAM_FRAG = /* glsl */ `
uniform float uTime;
uniform float uAmount;
uniform vec3 uColor;
uniform float uInner;
varying vec2 vUv;
${NOISE_GLSL}
void main(){
  vec2 c = vUv - 0.5;
  float r = length(c) * 2.0;
  float ang = atan(c.y, c.x);
  r += snoise(vec2(ang * 3.0, uTime * 0.5)) * 0.012;
  float dc = (r - uInner) / 0.035;
  float ds = (r - uInner - 0.03) / 0.14;
  float core = exp(-dc * dc);
  float soft = exp(-ds * ds) * 0.3;
  float edge = 1.0 - smoothstep(0.82, 1.0, r);
  float a = (core * 0.85 + soft) * edge * uAmount;
  gl_FragColor = vec4(uColor * a, a);
  ${OUTPUT_GLSL}
}
`;

/** Droplets thrown up when the mascot pops back out of the water. */
export const SPLASH_VERT = /* glsl */ `
attribute float aAlpha;
uniform float uPx;
varying float vAlpha;
void main(){
  vec4 mv = modelViewMatrix * vec4(position, 1.0);
  gl_Position = projectionMatrix * mv;
  vAlpha = aAlpha;
  gl_PointSize = max(1.0, 0.09 * uPx * 420.0 / -mv.z);
}
`;

export const SPLASH_FRAG = /* glsl */ `
uniform vec3 uColor;
varying float vAlpha;
void main(){
  float d = length(gl_PointCoord - 0.5);
  float a = (1.0 - smoothstep(0.1, 0.5, d)) * vAlpha;
  gl_FragColor = vec4(uColor * a, a);
  ${OUTPUT_GLSL}
}
`;

/** Marine snow: specks drifting down through the water around the camera. */
export const SNOW_VERT = /* glsl */ `
attribute vec4 aSeed;
uniform float uTime;
uniform vec3 uCam;
uniform float uPx;
uniform float uAlpha;
varying float vA;
void main(){
  vec3 box = vec3(36.0, 30.0, 36.0);
  vec3 p = aSeed.xyz * box;
  p.y -= uTime * (0.22 + aSeed.w * 0.32);
  p.x += sin(uTime * 0.3 + aSeed.z * 20.0) * 0.6;
  p = mod(p - uCam + box * 0.5, box) - box * 0.5;
  vec3 world = p + uCam;
  vec4 mv = viewMatrix * vec4(world, 1.0);
  gl_Position = projectionMatrix * mv;
  float d = length(mv.xyz);
  // Only below the surface, and fading with distance.
  float below = 1.0 - smoothstep(-1.2, -0.2, world.y);
  vA = uAlpha * (1.0 - smoothstep(7.0, 17.0, d)) * below * (0.35 + 0.65 * aSeed.w);
  gl_PointSize = clamp((1.4 + aSeed.w * 2.6) * uPx * (9.0 / max(d, 1.0)), 1.0, 7.0 * uPx);
}
`;

export const SNOW_FRAG = /* glsl */ `
uniform vec3 uColor;
varying float vA;
void main(){
  float d = length(gl_PointCoord - 0.5);
  float a = (1.0 - smoothstep(0.0, 0.5, d)) * vA;
  gl_FragColor = vec4(uColor * a, a);
  ${OUTPUT_GLSL}
}
`;

/** One light shaft: a tall billboard, bright at the surface, fading with depth. */
export const SHAFT_VERT = /* glsl */ `
varying vec2 vUv;
varying float vDepth;
void main(){
  vUv = uv;
  vDepth = -(modelMatrix * vec4(position, 1.0)).y;
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
}
`;

export const SHAFT_FRAG = /* glsl */ `
uniform float uTime;
uniform float uIntensity;
uniform float uSeed;
uniform vec3 uColor;
varying vec2 vUv;
varying float vDepth;
${NOISE_GLSL}
void main(){
  // WEB-5 O12: clamped. With MSAA a pixel just outside the quad is shaded
  // with vUv a little past 0..1; pow() of the negative base is NaN on
  // Apple GPUs, and NaN resolved to black dashes along every shaft's edge.
  float across = pow(max(1.0 - abs(vUv.x * 2.0 - 1.0), 0.0), 1.7);
  // WEB-5 O5: born just under the surface (soft, so the shaft never shows
  // the line where its quad meets the water) and fading as the light is
  // absorbed on the way down.
  float along = smoothstep(0.0, 1.8, vDepth) * exp(-vDepth / 11.0);
  float flick = 0.6 + 0.4 * snoise(vec2(vUv.x * 2.5 + uSeed * 9.0, uTime * 0.12 + uSeed * 5.0 + vUv.y * 1.5));
  float a = across * along * flick * uIntensity;
  gl_FragColor = vec4(uColor * a, a);
  ${OUTPUT_GLSL}
}
`;

/**
 * The data surface on top of each slab. One program, three looks, so the
 * stack tells the story without a word: Bronze is noisy and uneven, Silver is
 * ordered into rows, Gold is a few bright, evenly spaced points.
 */
export const SLAB_SURFACE_VERT = /* glsl */ `
varying vec2 vUv;
void main(){
  vUv = uv;
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
}
`;

export const SLAB_SURFACE_FRAG = /* glsl */ `
uniform float uTime;
uniform float uStyle;
uniform float uGlow;
uniform float uReveal;
uniform vec3 uColor;
varying vec2 vUv;
float hash21(vec2 p){ return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
void main(){
  vec2 uv = vUv;
  float a = 0.0;
  if (uStyle < 0.5) {
    // Bronze: uneven blobs of random size, jittered off any grid, flickering.
    vec2 g = uv * vec2(26.0, 18.0);
    vec2 id = floor(g);
    vec2 f = fract(g) - 0.5;
    vec2 j = (vec2(hash21(id), hash21(id + 7.3)) - 0.5) * 0.7;
    float r = mix(0.07, 0.3, hash21(id + 3.1));
    float d = length(f - j);
    float tw = 0.5 + 0.5 * sin(uTime * (1.0 + hash21(id) * 3.0) + hash21(id + 1.7) * 30.0);
    a = (1.0 - smoothstep(r * 0.4, r, d)) * (0.25 + 0.75 * tw) * step(0.3, hash21(id + 9.0));
  } else if (uStyle < 1.5) {
    // Silver: the same rows everywhere, dashes sweeping along them in step.
    vec2 g = uv * vec2(30.0, 12.0);
    vec2 id = floor(g);
    float row = abs(fract(g.y) - 0.5);
    float line = 1.0 - smoothstep(0.05, 0.13, row);
    float dash = smoothstep(0.35, 0.5, fract(g.x * 0.18 + id.y * 0.21 - uTime * 0.35));
    float base = 0.22 * (1.0 - smoothstep(0.04, 0.1, row));
    a = line * (base + 0.78 * dash);
  } else {
    // Gold: a sparse lattice of bright points, each with a soft halo.
    vec2 g = uv * vec2(9.0, 6.0);
    vec2 id = floor(g);
    vec2 f = fract(g) - 0.5;
    float d = length(f);
    float tw = 0.7 + 0.3 * sin(uTime * 1.2 + hash21(id) * 6.2831);
    a = ((1.0 - smoothstep(0.02, 0.1, d)) + 0.35 * exp(-d * d * 55.0)) * tw;
  }
  vec2 e = min(uv, 1.0 - uv);
  a *= smoothstep(0.0, 0.07, min(e.x, e.y));
  float k = (0.3 + 1.0 * uGlow) * uReveal;
  gl_FragColor = vec4(uColor * a * k, a * k);
  ${OUTPUT_GLSL}
}
`;

/** Rows falling from the slab above into this one; fewer and more ordered at each depth. */
export const STREAM_VERT = /* glsl */ `
attribute vec4 aSeed;
uniform float uTime;
uniform float uTop;
uniform float uBottom;
uniform float uOrder;
uniform float uHalfW;
uniform float uHalfD;
uniform float uCols;
uniform float uRows;
uniform float uSpeed;
uniform float uSize;
uniform float uPx;
uniform float uAlpha;
varying float vA;
void main(){
  float t = fract(aSeed.x + uTime * uSpeed * (0.7 + aSeed.w * 0.6));
  float y = mix(uTop, uBottom, t);
  float cx = (aSeed.y - 0.5) * 2.0 * uHalfW;
  float cz = (aSeed.z - 0.5) * 2.0 * uHalfD;
  float qx = ((floor(aSeed.y * uCols) + 0.5) / uCols - 0.5) * 2.0 * uHalfW;
  float qz = ((floor(aSeed.z * uRows) + 0.5) / uRows - 0.5) * 2.0 * uHalfD;
  vec3 p = vec3(mix(cx, qx, uOrder), y, mix(cz, qz, uOrder));
  p.x += sin(uTime * 1.3 + aSeed.w * 40.0) * 0.28 * (1.0 - uOrder);
  p.z += cos(uTime * 1.1 + aSeed.x * 40.0) * 0.28 * (1.0 - uOrder);
  vec4 mv = modelViewMatrix * vec4(p, 1.0);
  gl_Position = projectionMatrix * mv;
  float fadeIn = smoothstep(0.0, 0.12, t);
  float land = smoothstep(0.88, 1.0, t);
  float fadeOut = 1.0 - smoothstep(0.97, 1.0, t);
  vA = uAlpha * fadeIn * fadeOut * (0.55 + 0.9 * land);
  gl_PointSize = clamp(uSize * uPx * (10.0 / max(-mv.z, 1.0)) * (0.8 + 0.6 * land), 1.0, 14.0 * uPx);
}
`;

export const STREAM_FRAG = /* glsl */ `
uniform vec3 uColor;
varying float vA;
void main(){
  float d = length(gl_PointCoord - 0.5);
  float a = (1.0 - smoothstep(0.0, 0.5, d)) * vA;
  gl_FragColor = vec4(uColor * a, a);
  ${OUTPUT_GLSL}
}
`;

/**
 * WEB-5 O5, the lens. Every frame is drawn into an off-screen picture and
 * this pass puts it on screen, so the crossing can be a camera in the water
 * rather than an overlay on top of it:
 * - `tA` is the scene as seen above the water, `tB` as seen below it. While
 *   the camera is at the surface both are drawn and the screen splits where
 *   the water stands on the lens: the camera's height, against the lake's
 *   own waves sampled along the lens (`lensWave`). The line is wavy, moves
 *   with the water, and sweeps up the screen as the camera sinks.
 * - At the line: a thin bright meniscus with a soft shadow under it, and
 *   the picture bends and blurs a little in the water clinging to the lens.
 * - Below the line, and everywhere once under, the picture wobbles very
 *   slightly and slowly, as it does through moving water.
 * - `uMode` 0 draws only `tA`, 1 only `tB` (one scene render), 2 both.
 * The pass also does the tone mapping and the sRGB conversion, so every
 * frame is graded the same way whichever side of the surface it shows.
 */
export const LENS_VERT = /* glsl */ `
varying vec2 vUv;
void main(){
  vUv = uv;
  gl_Position = vec4(position.xy, 0.0, 1.0);
}
`;

export const LENS_FRAG = /* glsl */ `
uniform sampler2D tA;
uniform sampler2D tB;
uniform float uMode;
uniform float uTime;
uniform float uCamY;
uniform float uLens;
uniform float uAspect;
uniform vec2 uCamXZ;
uniform vec2 uRight;
uniform vec2 uRes;
uniform float uWobble;
uniform vec3 uMeniscus;
varying vec2 vUv;
${NOISE_GLSL}

// Height of the water where it touches the lens, at lens position u (-1..1).
// The swell uses the lake's own wave layers (WATER_FRAG baseH, at the scale
// its normals use), the last term is the meniscus' own small ripple.
// x is the lens position in units of the screen's half-height, so a narrow
// portrait screen shows fewer ripples rather than the same ones squeezed,
// and on it the ripples are lower too.
float lensWave(float u){
  vec2 p = uCamXZ + uRight * (u * uLens * uAspect);
  float t = uTime;
  float x = u * uAspect;
  float h = snoise(p * 0.62 + vec2(-t * 0.06, t * 0.05)) * 0.024;
  h += snoise(p * 1.5 + vec2(t * 0.1, -t * 0.07)) * 0.014;
  h += snoise(vec2(x * 1.6 + t * 0.32, t * 0.19)) * 0.011;
  h += snoise(vec2(x * 4.4 - t * 0.6, 3.1 + t * 0.4)) * 0.0035;
  return h * mix(0.55, 1.0, clamp(uAspect, 0.0, 1.0));
}

vec3 tap(sampler2D t, vec2 uv, float r){
  if (r < 0.35) return texture2D(t, uv).rgb;
  vec2 px = r / uRes;
  vec3 c = texture2D(t, uv).rgb * 0.36;
  c += texture2D(t, uv + vec2(px.x, 0.0)).rgb * 0.16;
  c += texture2D(t, uv - vec2(px.x, 0.0)).rgb * 0.16;
  c += texture2D(t, uv + vec2(0.0, px.y)).rgb * 0.16;
  c += texture2D(t, uv - vec2(0.0, px.y)).rgb * 0.16;
  return c;
}

void main(){
  vec2 uv = vUv;
  vec2 wob = uWobble * vec2(
    sin(uv.y * 8.0 + uTime * 0.62) + 0.5 * sin(uv.y * 17.0 - uTime * 0.9),
    cos(uv.x * 6.0 + uTime * 0.5) + 0.5 * cos(uv.x * 13.0 + uTime * 0.77)
  ) * 0.0011;
  vec3 col;
  if (uMode < 0.5) {
    col = texture2D(tA, uv).rgb;
  } else if (uMode < 1.5) {
    col = texture2D(tB, uv + wob).rgb;
  } else {
    float v = uv.y * 2.0 - 1.0;
    float line = (lensWave(uv.x * 2.0 - 1.0) - uCamY) / uLens;
    // Pixels above (+) or below (-) the waterline.
    float s = (v - line) * 0.5 * uRes.y;
    float span = 0.035 * uRes.y;
    float near = exp(-abs(s) / span);
    // Water on the lens bends the picture towards the line and softens it.
    vec2 bend = vec2(0.0, -sign(s) * near * 0.01);
    float blur = near * 2.6 * uRes.y / 900.0;
    vec3 a = tap(tA, uv + bend, blur);
    vec3 b = tap(tB, uv + bend + wob * (1.0 - near), blur);
    float px = uRes.y / 900.0;
    col = mix(b, a, smoothstep(-1.2 * px, 1.2 * px, s));
    // The meniscus: light caught in the curved water, then its shadow.
    float mg = (s - 0.5 * px) / (1.1 * px);
    float ms = (s + 3.0 * px) / (3.0 * px);
    col += uMeniscus * exp(-mg * mg);
    col *= 1.0 - 0.22 * exp(-ms * ms);
  }
  gl_FragColor = vec4(col, 1.0);
  ${OUTPUT_GLSL}
}
`;

/**
 * WEB-5 O8, the blowhole's spout: droplets and mist as points, each sized
 * in world units and drawn at its true size on screen (with a floor, so it
 * still reads on a phone). `aSoft` 0 is a droplet (a small round bead), 1 a
 * puff of mist (a wide, very soft disc).
 */
export const SPOUT_VERT = /* glsl */ `
attribute float aSize;
attribute float aAlpha;
attribute float aSoft;
uniform float uHalfH;
uniform float uMinPx;
varying float vAlpha;
varying float vSoft;
void main(){
  vec4 mv = modelViewMatrix * vec4(position, 1.0);
  gl_Position = projectionMatrix * mv;
  vAlpha = aAlpha;
  vSoft = aSoft;
  gl_PointSize = max(uMinPx, aSize * projectionMatrix[1][1] * uHalfH / max(-mv.z, 0.1));
}
`;

export const SPOUT_FRAG = /* glsl */ `
uniform vec3 uColor;
varying float vAlpha;
varying float vSoft;
void main(){
  float d = length(gl_PointCoord - 0.5);
  float a = mix(1.0 - smoothstep(0.22, 0.5, d), exp(-d * d * 14.0), vSoft) * vAlpha;
  gl_FragColor = vec4(uColor * a, a);
  ${OUTPUT_GLSL}
}
`;
