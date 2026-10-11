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
uniform vec3 uFogColor;
uniform float uUnder;
varying vec3 vDir;
${SKY_GRADIENT_GLSL}
void main(){
  vec3 d = normalize(vDir);
  vec3 col = skyGradient(d);
  float m = dot(d, uMoonDir);
  float disc = smoothstep(0.99978, 0.99988, m);
  float halo = pow(max(m, 0.0), 900.0) * 0.5 + pow(max(m, 0.0), 60.0) * 0.12 + pow(max(m, 0.0), 8.0) * 0.03;
  col += uMoonColor * (disc * uMoonBoost + halo);
  // Under the surface the "sky" is just the water's fog colour. It has to be
  // drawn here, through the same tone mapping as the water, rather than as the
  // scene's clear colour, which three does not tone-map and so never matches.
  col = mix(col, uFogColor, uUnder);
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
 * term; seen from below (`gl_FrontFacing` false) it becomes a rippling
 * ceiling with a Snell's-window glow and a caustic web.
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
varying vec3 vWorld;
${NOISE_GLSL}
${SKY_GRADIENT_GLSL}

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
  vec3 n = normalize(vec3(-g.x * 0.2 * lod, 1.0, -g.y * 0.2 * lod));

  vec3 col;
  if (gl_FrontFacing) {
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
    float rim = exp(-pow((dm - uMascot.z) / 0.14, 2.0));
    col += uLake * rim * 0.4;
  } else {
    vec3 Vd = -V;
    float up = clamp(Vd.y, 0.0, 1.0);
    float win = smoothstep(0.5, 0.92, up);
    vec2 cp = p * 0.55 + g * 0.9;
    float c1 = 1.0 - abs(snoise(cp + vec2(uTime * 0.12, uTime * 0.09)));
    float c2 = 1.0 - abs(snoise(cp * 1.7 - vec2(uTime * 0.1, -uTime * 0.14)));
    float caustic = pow(c1 * c2, 3.0);
    col = mix(uDeep * 3.0, uLake * 0.55, win);
    col += vec3(0.45, 0.75, 1.0) * caustic * (0.2 + 0.9 * win);
    col += uMoonColor * pow(max(dot(Vd, uMoonDir), 0.0), 220.0) * 1.4 * win;
  }

  float f = 1.0 - exp(-pow(dist * uFogDensity, 2.0));
  col = mix(col, uFogColor, clamp(f, 0.0, 1.0));
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
  float core = exp(-pow((r - uInner) / 0.035, 2.0));
  float soft = exp(-pow((r - uInner - 0.03) / 0.14, 2.0)) * 0.3;
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
void main(){
  vUv = uv;
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
}
`;

export const SHAFT_FRAG = /* glsl */ `
uniform float uTime;
uniform float uIntensity;
uniform float uSeed;
uniform vec3 uColor;
varying vec2 vUv;
${NOISE_GLSL}
void main(){
  float across = pow(1.0 - abs(vUv.x * 2.0 - 1.0), 1.7);
  float along = pow(vUv.y, 3.0);
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
