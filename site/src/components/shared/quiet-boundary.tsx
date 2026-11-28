"use client";

import * as React from "react";

/**
 * Catches an error under it, usually a deferred chunk that failed to load,
 * and renders nothing in its place. WEB-5 review fix R1: without it, a
 * `next/dynamic` chunk that never arrives (a flaky network, a blocked
 * script) throws a ChunkLoadError up to Next's root, which replaces the
 * whole page with its "This page couldn't load" screen, measured with the
 * chunk blocked. Every deferred piece of the page (the animation features,
 * the 3D scene, the language offer) sits under one, so the page itself
 * always stays.
 */
export class QuietBoundary extends React.Component<
  { children: React.ReactNode; onError?: () => void },
  { failed: boolean }
> {
  state = { failed: false };

  static getDerivedStateFromError(): { failed: boolean } {
    return { failed: true };
  }

  componentDidCatch(): void {
    this.props.onError?.();
  }

  render(): React.ReactNode {
    return this.state.failed ? null : this.props.children;
  }
}
