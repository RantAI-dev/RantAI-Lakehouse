"use client";

import dynamic from "next/dynamic";

/**
 * WebGL only exists in the browser, so the caustic canvas is client-only
 * (`ssr: false`) — this also keeps `ogl` out of the server bundle. There is
 * deliberately no `loading` placeholder: the panel already paints its navy
 * base + CSS gradient, so the canvas fading in on top is the desired
 * progressive enhancement rather than a layout-shifting skeleton.
 */
const MoltenMetal = dynamic(
  () => import("@/components/ui/molten-metal").then((m) => m.MoltenMetal),
  { ssr: false }
);

/**
 * Feature list — kept in sync with the real console sections in
 * `components/app-shell/nav-config.ts`. Every line here maps to routes that
 * actually ship, so a first-time visitor is not sold something the sidebar
 * cannot deliver after they sign in.
 */
const FEATURES = [
  "Unified catalog and data explorer",
  "SQL Query Studio and pipelines",
  "Lineage, policies, and audit trail",
  "AI Copilot over your data",
] as const;

/**
 * Decorative panel beside the sign-in form, hidden from the a11y tree — it
 * carries no information the form itself does not already convey.
 *
 * `hidden lg:block` is doing real work beyond layout: below `lg` this
 * subtree never mounts, so phones never download or execute `ogl` at all.
 *
 * Layering, bottom-up: the `--brand-2` navy base (also the fallback when
 * WebGL2 is unavailable) → the caustic canvas → a bottom-weighted scrim
 * that keeps the copy legible wherever a bright filament happens to drift.
 */
export function LoginHero() {
  return (
    <div
      aria-hidden="true"
      className="relative hidden w-1/2 flex-none overflow-hidden rounded-3xl bg-brand-2 lg:my-4 lg:ml-4 lg:block"
    >
      <div className="absolute inset-0">
        <MoltenMetal
          // Brand blues reserved for canvas/WebGL surfaces
          // (`design-system/tokens/colors.css`): light refracting through
          // deep water — the "lake" in Rantai Lake.
          //
          // The ramp starts at `--brand-canvas-light` rather than the
          // darker `--brand-canvas-dark`. Because alpha tracks intensity,
          // a dark `color1` lands only where the field is already fading
          // out, so it reads as more navy instead of as blue — the whole
          // ramp has to sit above the background to be visible at all.
          // `color2` stays a touch above it so the midtones carry color
          // without washing out to near-white.
          color1="#5EB6FA"
          color2="#8FCDF7"
          color3="#FFFFFF"
          // Palette mode only shifts *where* the ramp crosses over, not the
          // hues. `molten` (0.5) splits it evenly; `frost` (0.65) would hold
          // `color1` far longer, which mattered when `color1` was the dark
          // blue but now just delays the brighter tones. Neither mode adds
          // warmth of its own — the three colors above are the whole palette.
          colorMode="molten"
          // Slow enough to read as drifting water rather than boiling metal
          // — this sits behind copy and must not pull focus.
          speed={0.18}
          scale={3.2}
          // The shader sets `alpha = intensity`, so dim regions are not just
          // dark, they are *transparent* down to the navy base. Coverage is
          // therefore governed by these four together, not by color choice:
          //
          //  · detail — folding iterations; each one adds another filament
          //    layer, so this is what turns a few lonely streaks into a
          //    woven caustic net. The shader hard-caps the loop at 8.
          //  · blackPoint — the cutoff below which the field fades out.
          //    Near zero keeps the midtones that carry the sky-blue.
          //  · glow / coreSize — multiplied together into the per-iteration
          //    gain, widening and brightening each filament.
          // Density and exposure are separate knobs, and only exposure was
          // overshooting: `detail` stays at 5 to keep the woven net, while
          // `blackPoint` rises to trim the faint outer halo back to navy and
          // the glow/core/brightness trio comes down off its peak.
          detail={5}
          blackPoint={0.035}
          glow={1.35}
          coreSize={0.11}
          brightness={1.15}
          grainIntensity={0.03}
          mouseStrength={0.18}
        />
      </div>

      {/* Scrim, weighted to the bottom where the copy sits. `/40` is the
          middle ground between the original `/60` (which washed the caustic
          flat exactly where the field is busiest) and `/25` (which left the
          panel reading too bright): enough navy to seat the effect and hold
          text contrast, not so much that it erases the filaments. */}
      <div className="absolute inset-0 bg-gradient-to-t from-brand-2 via-brand-2/40 to-transparent" />

      <div className="relative flex h-full flex-col justify-center px-14 xl:px-20">
        <p className="animate-in fade-in slide-in-from-bottom-3 fill-mode-both text-2xl leading-[1.35] font-medium tracking-[-0.02em] text-white duration-700 motion-reduce:animate-none">
          Every dataset, pipeline, and insight in one place.
        </p>
        <p className="animate-in fade-in slide-in-from-bottom-3 fill-mode-both mt-3 max-w-[400px] text-sm leading-relaxed text-white/75 delay-100 duration-700 motion-reduce:animate-none">
          Browse the catalog, run pipelines, build dashboards, and chat with an
          LLM over your data.
        </p>

        <ul className="animate-in fade-in slide-in-from-bottom-4 fill-mode-both mt-8 flex flex-col gap-3 delay-200 duration-700 motion-reduce:animate-none">
          {FEATURES.map((feature) => (
            <li
              key={feature}
              className="flex items-center gap-3 text-sm text-white/70"
            >
              <span className="size-1.5 shrink-0 rounded-full bg-white/50" />
              {feature}
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
