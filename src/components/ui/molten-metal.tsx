"use client"

import * as React from "react"
import { Renderer, Program, Mesh, Triangle } from "ogl"

import { cn } from "@/lib/utils"

/**
 * Animated caustic-field background (WebGL2), ported from React Bits'
 * `MoltenMetal` to TypeScript.
 *
 * Despite the upstream name, the shader is a *caustic* field — the same
 * lattice of focused light you see on the floor of a swimming pool. Fed the
 * Rantai Lake brand blues (`--brand-canvas-dark` / `--brand-canvas-light`
 * over the `--brand-2` navy) it reads as lit water rather than molten
 * metal, which is why `/login`'s hero panel uses it.
 *
 * Deliberately not SSR-safe by design — WebGL only exists in the browser,
 * so callers must load this via `next/dynamic` with `ssr: false` (see
 * `login-hero.tsx`).
 *
 * Fails soft, never loud: if WebGL2 is unavailable (older browser, blocked
 * context, driver blocklist) the constructor throw is caught and the
 * component renders an empty div, leaving whatever background the parent
 * painted behind it. `LoginHero` relies on that by keeping a solid navy +
 * CSS gradient underneath.
 */

/** `#rrggbb` → normalized `[r, g, b]`. Falls back to white on malformed input. */
const hexToRgb = (hex: string): [number, number, number] => {
  const result = /^#?([a-f\d]{2})([a-f\d]{2})([a-f\d]{2})$/i.exec(hex)
  if (!result) return [1, 1, 1]
  return [
    parseInt(result[1], 16) / 255,
    parseInt(result[2], 16) / 255,
    parseInt(result[3], 16) / 255,
  ]
}

export type MoltenMetalColorMode = "molten" | "ember" | "frost"

const colorModeToFloat = (mode: MoltenMetalColorMode): number =>
  mode === "ember" ? 1 : mode === "frost" ? 2 : 0

const vertex = `#version 300 es
in vec2 position;
void main() {
  gl_Position = vec4(position, 0.0, 1.0);
}
`

/**
 * Upstream's `uLightMode` branch (which composites against an opaque
 * background color) is dropped: the hero panel always sits on the dark navy
 * brand surface in both themes, so the transparent premultiplied output is
 * the only path ever taken. Fewer uniforms, one less branch per fragment.
 */
const fragment = `#version 300 es
precision highp float;
uniform vec2 iResolution;
uniform float iTime;
uniform float uSpeed;
uniform float uScale;
uniform float uDetail;
uniform float uGlow;
uniform float uCoreSize;
uniform float uSwirl;
uniform float uFold;
uniform float uBlackPoint;
uniform float uBrightness;
uniform float uColorMode;
uniform float uGrain;
uniform float uGrainIntensity;
uniform float uOpacity;
uniform vec2 uMouse;
uniform float uMouseStrength;
uniform bool uEnableMouse;
uniform vec3 uColor1;
uniform vec3 uColor2;
uniform vec3 uColor3;
out vec4 fragColor;

float hash(vec2 p) {
  return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453);
}

void main() {
  float time = iTime * uSpeed;
  vec2 p = uScale * ((gl_FragCoord.xy - 0.5 * iResolution.xy) / iResolution.y) - 0.5;

  vec2 drift = vec2(0.0);
  if (uEnableMouse) {
    drift = (uMouse - 0.5) * uMouseStrength * 2.0;
  }
  p += drift;

  vec2 i = p;
  float c = 0.0;
  float r = length(p + vec2(sin(time), sin(time * 0.3 + 5.0)) * 0.5);
  float d = length(p);
  float rot = d + time + p.x * uSwirl;

  float cosRot = cos(rot);
  mat2 warp = mat2(cos(rot - sin(time / 5.0)), sin(rot), -sin(cosRot - time), cosRot) * uFold;
  float glowCore = uGlow * uCoreSize;

  for (float n = 0.0; n < 8.0; n++) {
    if (n >= uDetail) break;
    p *= warp;
    float t = r - time / (n + 3.0);
    i -= p + vec2(cos(t - i.x - r) + sin(t + i.y), sin(t - i.y) + cos(t + i.x) + r);
    c += glowCore / length(vec2(sin(i.x + t), cos(i.y + t)));
  }

  c /= 6.0;

  float intensity = max(c - uBlackPoint, 0.0) * uBrightness;
  float g = clamp(intensity, 0.0, 1.0);

  float mid = 0.5;
  if (uColorMode > 1.5) {
    mid = 0.65;
  } else if (uColorMode > 0.5) {
    mid = 0.35;
  }

  vec3 col = mix(uColor1, uColor2, smoothstep(0.0, mid, g));
  col = mix(col, uColor3, smoothstep(mid, 1.0, g));

  float a = g;
  if (uGrain > 0.5) {
    float gr = hash(gl_FragCoord.xy + iTime);
    a += (gr - 0.5) * uGrainIntensity;
  }
  a = clamp(a, 0.0, 1.0) * uOpacity;

  fragColor = vec4(col * a, a);
}
`

export type MoltenMetalProps = {
  /** Shadow color for the dim caustic glow. */
  color1?: string
  /** Midtone color for the flowing filaments. */
  color2?: string
  /** Highlight color for the hot filament cores. */
  color3?: string
  /** Animation speed of the liquid motion. */
  speed?: number
  /** Zoom of the caustic field (higher = more detail on screen). */
  scale?: number
  /** Number of domain-folding iterations (1-8). */
  detail?: number
  /** Gain applied to the accumulated filament glow. */
  glow?: number
  /** Thickness of the bright filament cores. */
  coreSize?: number
  /** Amount of rotational swirl across the field. */
  swirl?: number
  /** Turbulence / fold strength of the iterative warp. */
  fold?: number
  /** Raises the dark floor so shadows fade to transparent. */
  blackPoint?: number
  /** Overall brightness of the effect. */
  brightness?: number
  /** Palette mapping: `molten`, `ember`, or `frost`. */
  colorMode?: MoltenMetalColorMode
  /** Adds subtle animated film grain. */
  grain?: boolean
  /** Amplitude of the grain overlay. 0 disables it entirely. */
  grainIntensity?: number
  /** Enables gentle drift of the field toward the cursor. */
  mouseInteraction?: boolean
  /** Strength of the cursor drift. */
  mouseStrength?: number
  /** Overall opacity of the effect over the page. */
  opacity?: number
  className?: string
}

/** The mutable uniform bag `ogl` hands back — only the fields this file writes. */
type Uniforms = Record<string, { value: number | boolean | Float32Array }>

export function MoltenMetal({
  color1 = "#5227FF",
  color2 = "#FF9FFC",
  color3 = "#FFFFFF",
  speed = 0.35,
  scale = 4,
  detail = 3,
  glow = 1.6,
  coreSize = 0.1,
  swirl = 1,
  fold = -0.2,
  blackPoint = 0.05,
  brightness = 1.3,
  colorMode = "molten",
  grain = true,
  grainIntensity = 0.05,
  mouseInteraction = true,
  mouseStrength = 0.3,
  opacity = 1.0,
  className,
}: MoltenMetalProps) {
  const containerRef = React.useRef<HTMLDivElement>(null)
  // Held in a ref rather than upstream's module-level WeakMap: the props
  // effect below needs the same program the setup effect created, and a ref
  // scopes that to this instance without a global side table.
  const programRef = React.useRef<{ uniforms: Uniforms } | null>(null)

  React.useEffect(() => {
    const container = containerRef.current
    if (!container) return

    // Respecting the OS "reduce motion" setting is not optional for a
    // full-bleed animated background — it is a common vestibular/migraine
    // trigger. One static frame still renders (the panel keeps its
    // texture), the rAF loop just never starts. Same intent as the
    // `motion-reduce:animate-none` utilities used elsewhere in the app.
    const reduceMotion =
      window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false

    let renderer: Renderer
    try {
      renderer = new Renderer({
        webgl: 2,
        alpha: true,
        premultipliedAlpha: true,
        antialias: false,
        dpr: Math.min(window.devicePixelRatio || 1, 2),
      })
    } catch {
      // No WebGL2 (old browser, blocked context, driver blocklist). Render
      // nothing and let the parent's solid background stand in.
      return
    }

    const gl = renderer.gl
    gl.clearColor(0, 0, 0, 0)
    const canvas = gl.canvas as HTMLCanvasElement
    canvas.style.width = "100%"
    canvas.style.height = "100%"
    canvas.style.display = "block"
    container.appendChild(canvas)

    const geometry = new Triangle(gl)
    const program = new Program(gl, {
      vertex,
      fragment,
      uniforms: {
        iTime: { value: 0 },
        iResolution: { value: new Float32Array([1, 1]) },
        uSpeed: { value: speed },
        uScale: { value: scale },
        uDetail: { value: detail },
        uGlow: { value: glow },
        uCoreSize: { value: Math.max(coreSize, 0.001) },
        uSwirl: { value: swirl },
        uFold: { value: fold },
        uBlackPoint: { value: blackPoint },
        uBrightness: { value: brightness },
        uColorMode: { value: colorModeToFloat(colorMode) },
        uGrain: { value: grain ? 1 : 0 },
        uGrainIntensity: { value: grainIntensity },
        uOpacity: { value: opacity },
        uMouse: { value: new Float32Array([0.5, 0.5]) },
        uMouseStrength: { value: mouseStrength },
        uEnableMouse: { value: mouseInteraction && !reduceMotion },
        uColor1: { value: new Float32Array(hexToRgb(color1)) },
        uColor2: { value: new Float32Array(hexToRgb(color2)) },
        uColor3: { value: new Float32Array(hexToRgb(color3)) },
      },
    })

    const mesh = new Mesh(gl, { geometry, program })
    programRef.current = program as unknown as { uniforms: Uniforms }

    const setSize = () => {
      const rect = container.getBoundingClientRect()
      renderer.setSize(
        Math.max(1, Math.floor(rect.width)),
        Math.max(1, Math.floor(rect.height))
      )
      const res = program.uniforms.iResolution.value as Float32Array
      res[0] = gl.drawingBufferWidth
      res[1] = gl.drawingBufferHeight
      renderer.render({ scene: mesh })
    }

    const ro = new ResizeObserver(setSize)
    ro.observe(container)
    setSize()

    const targetMouse: [number, number] = [0.5, 0.5]
    const currentMouse: [number, number] = [0.5, 0.5]

    const handleMouseMove = (e: MouseEvent) => {
      const rect = canvas.getBoundingClientRect()
      targetMouse[0] = (e.clientX - rect.left) / rect.width
      targetMouse[1] = 1.0 - (e.clientY - rect.top) / rect.height
    }
    const handleMouseLeave = () => {
      targetMouse[0] = 0.5
      targetMouse[1] = 0.5
    }

    if (mouseInteraction && !reduceMotion) {
      canvas.addEventListener("mousemove", handleMouseMove)
      canvas.addEventListener("mouseleave", handleMouseLeave)
    }

    let raf = 0
    let isVisible = true
    let isPageVisible = !document.hidden
    const t0 = performance.now()

    const loop = (t: number) => {
      program.uniforms.iTime.value = (t - t0) * 0.001
      currentMouse[0] += 0.05 * (targetMouse[0] - currentMouse[0])
      currentMouse[1] += 0.05 * (targetMouse[1] - currentMouse[1])
      const m = program.uniforms.uMouse.value as Float32Array
      m[0] = currentMouse[0]
      m[1] = currentMouse[1]
      renderer.render({ scene: mesh })
      raf = requestAnimationFrame(loop)
    }

    // Only animate while the panel is on screen AND the tab is foregrounded
    // — a login page left open in a background tab should not keep a GPU
    // loop (and the user's battery) busy.
    const tryStart = () => {
      if (reduceMotion) return
      if (isVisible && isPageVisible && raf === 0) {
        raf = requestAnimationFrame(loop)
      }
    }
    const tryStop = () => {
      if (raf !== 0) {
        cancelAnimationFrame(raf)
        raf = 0
      }
    }

    const io = new IntersectionObserver(
      ([entry]) => {
        isVisible = entry.isIntersecting
        if (isVisible) tryStart()
        else tryStop()
      },
      { threshold: 0 }
    )
    io.observe(container)

    const onVisibility = () => {
      isPageVisible = !document.hidden
      if (isPageVisible) tryStart()
      else tryStop()
    }
    document.addEventListener("visibilitychange", onVisibility)

    tryStart()

    return () => {
      tryStop()
      io.disconnect()
      document.removeEventListener("visibilitychange", onVisibility)
      ro.disconnect()
      canvas.removeEventListener("mousemove", handleMouseMove)
      canvas.removeEventListener("mouseleave", handleMouseLeave)
      programRef.current = null
      if (canvas.parentNode === container) container.removeChild(canvas)
      // Free the GPU context eagerly: browsers cap live WebGL contexts
      // (~16 in Chrome) and React's dev-mode double-invoke means a leak
      // here would break the canvas after a handful of remounts.
      gl.getExtension("WEBGL_lose_context")?.loseContext()
    }
    // Setup intentionally runs once — live prop changes are pushed onto the
    // existing program by the effect below rather than rebuilding the
    // WebGL context. Props are read here only for the initial uniforms.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // Push prop changes onto the live program rather than tearing down and
  // rebuilding the WebGL context on every tweak. No-ops until the setup
  // effect has run (and stays a no-op forever when WebGL2 is unavailable).
  React.useEffect(() => {
    const u = programRef.current?.uniforms
    if (!u) return

    u.uSpeed.value = speed
    u.uScale.value = scale
    u.uDetail.value = detail
    u.uGlow.value = glow
    u.uCoreSize.value = Math.max(coreSize, 0.001)
    u.uSwirl.value = swirl
    u.uFold.value = fold
    u.uBlackPoint.value = blackPoint
    u.uBrightness.value = brightness
    u.uColorMode.value = colorModeToFloat(colorMode)
    u.uGrain.value = grain ? 1 : 0
    u.uGrainIntensity.value = grainIntensity
    u.uOpacity.value = opacity
    u.uMouseStrength.value = mouseStrength

    // Mutate the existing Float32Array in place — `ogl` holds the same
    // reference it uploaded, so replacing the object would not reach the GPU.
    const write = (key: string, hex: string) => {
      const target = u[key].value as Float32Array
      const [r, g, b] = hexToRgb(hex)
      target[0] = r
      target[1] = g
      target[2] = b
    }
    write("uColor1", color1)
    write("uColor2", color2)
    write("uColor3", color3)
  }, [
    color1,
    color2,
    color3,
    speed,
    scale,
    detail,
    glow,
    coreSize,
    swirl,
    fold,
    blackPoint,
    brightness,
    colorMode,
    grain,
    grainIntensity,
    mouseStrength,
    opacity,
  ])

  return (
    <div
      ref={containerRef}
      aria-hidden
      className={cn("relative size-full overflow-hidden", className)}
    />
  )
}
