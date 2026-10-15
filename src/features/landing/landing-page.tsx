"use client";

import * as React from "react";
import dynamic from "next/dynamic";
import { cn } from "@/lib/utils";
import { AssistantSection } from "./assistant-section";
import { DashboardsSection } from "./dashboards-section";
import { DepthsSection } from "./depths-section";
import { FinalSection } from "./final-section";
import { GovernanceSection } from "./governance-section";
import { HeroSection } from "./hero-section";
import { LandingFooter } from "./landing-footer";
import { LandingModeProvider, useLandingMode } from "./landing-mode";
import { OpenSection } from "./open-section";
import { SourcesSection } from "./sources-section";
import { TopBar } from "./top-bar";
import "./landing.css";

/**
 * `three` loads on demand, after first paint: the visitor sees the DOM hero
 * immediately and the canvas fades in over it (plan §2.9).
 */
const LakeScene = dynamic(() => import("./scene/lake-scene"), { ssr: false });

/**
 * The `/landing` page (WEB-1). Public, static, and independent of the API:
 * it imports nothing from `@/services` (plan §5).
 */
export function LandingPage() {
  return (
    <LandingModeProvider>
      <div className="lk-root relative min-h-screen overflow-x-clip">
        <a
          href="#main"
          className="sr-only z-[60] rounded-full bg-[#E8F4FF] px-4 py-2 text-[14px] font-medium text-[#020514] focus:not-sr-only focus:fixed focus:top-3 focus:left-3"
        >
          Skip to content
        </a>
        <TopBar />
        <main id="main" tabIndex={-1}>
          <SceneZone />
          <AssistantSection />
          <DashboardsSection />
          <GovernanceSection />
          <OpenSection />
          <FinalSection />
        </main>
        <LandingFooter />
      </div>
    </LandingModeProvider>
  );
}

/**
 * Hero, sources and depths share one canvas behind them. Everything the
 * visitor reads here is real DOM above the canvas, so with no canvas the
 * same sections show over a CSS gradient instead.
 */
function SceneZone() {
  const zoneRef = React.useRef<HTMLDivElement>(null);
  const { mode, sceneReady, markReady, markFailed } = useLandingMode();
  const hasCanvas = mode === "full" || mode === "still";

  return (
    <div id="scene-zone" ref={zoneRef} className="relative">
      {/* Below the hero, with no live canvas behind it, the sections sit on a deep-water gradient. */}
      <div
        aria-hidden="true"
        className={cn(
          "absolute inset-x-0 top-[100svh] bottom-0 bg-[linear-gradient(180deg,#0a1f55_0%,#06143f_30%,#04102f_65%,#020514_100%)] transition-opacity duration-700",
          mode === "full" && sceneReady ? "opacity-0" : "opacity-100",
        )}
      />
      {hasCanvas && (
        <div
          aria-hidden="true"
          className={cn(
            "pointer-events-none z-0 transition-opacity duration-700",
            // Full: pinned behind the hero, sources and depths for as long as
            // the zone lasts (the negative margin gives its flow height back).
            // Still: one frame, in the hero only.
            mode === "full"
              ? "sticky top-0 -mb-[100lvh] h-[100lvh]"
              : "absolute inset-x-0 top-0 h-[100svh]",
            sceneReady ? "opacity-100" : "opacity-0",
          )}
        >
          <LakeScene
            mode={mode === "still" ? "still" : "full"}
            observe={zoneRef}
            onReady={markReady}
            onFail={markFailed}
          />
        </div>
      )}
      <div className="relative z-10">
        <HeroSection />
        <SourcesSection />
        <DepthsSection />
      </div>
    </div>
  );
}
