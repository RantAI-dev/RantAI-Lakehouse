"use client";

import * as React from "react";
import { AssistantSection } from "./assistant-section";
import { DashboardsSection } from "./dashboards-section";
import { DepthsSection } from "./depths-section";
import { FinalSection } from "./final-section";
import { GovernanceSection } from "./governance-section";
import { HeroSection } from "./hero-section";
import { LandingFooter } from "./landing-footer";
import { OpenSection } from "./open-section";
import { SourcesSection } from "./sources-section";
import { TopBar } from "./top-bar";
import "./landing.css";

/**
 * The `/landing` page (WEB-1). Public, static, and independent of the API:
 * it imports nothing from `@/services` (plan §5).
 */
export function LandingPage() {
  return (
    <div className="lk-root relative min-h-screen overflow-x-clip">
      <a
        href="#main"
        className="sr-only z-[60] rounded-full bg-[#E8F4FF] px-4 py-2 text-[14px] font-medium text-[#020514] focus:not-sr-only focus:fixed focus:top-3 focus:left-3"
      >
        Skip to content
      </a>
      <TopBar />
      <main id="main">
        <HeroSection />
        <SourcesSection />
        <DepthsSection />
        <AssistantSection />
        <DashboardsSection />
        <GovernanceSection />
        <OpenSection />
        <FinalSection />
      </main>
      <LandingFooter />
    </div>
  );
}
