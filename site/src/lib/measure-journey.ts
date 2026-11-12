import { JOURNEY, SURFACE_AT_FINAL } from "@/lib/home-content";
import { journeyFromBoxes, type Journey, type SectionBox } from "@/lib/depth-journey";

/**
 * WEB-5 review fix R9: the one function that measures the page for the
 * depth gauge and the water. It reads each section's box in page
 * coordinates and hands them to `journeyFromBoxes`, which is pure and
 * tested (lib/depth-journey.test.ts). Browser only: call it from an effect.
 */
export function measureJourney(opts: { holdSurfaceThroughHero: boolean }): Journey {
  const boxes: Record<string, SectionBox> = {};
  // The chapters include every section the journey reads (the depths pin,
  // the dashboards where the climb starts, the final).
  for (const { id } of JOURNEY) {
    const node = document.getElementById(id);
    if (!node) continue;
    const r = node.getBoundingClientRect();
    boxes[id] = { top: r.top + window.scrollY, height: r.height };
  }
  return journeyFromBoxes(JOURNEY, boxes, window.innerHeight, { ...opts, surfaceAtFinal: SURFACE_AT_FINAL });
}
