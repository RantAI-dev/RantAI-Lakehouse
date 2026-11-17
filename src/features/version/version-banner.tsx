"use client";

import * as React from "react";
import { TriangleAlert } from "lucide-react";
import { useService } from "@/hooks/use-service";
import { versionService } from "@/services";
import { versionMismatch } from "./console-version";

/**
 * The banner T3 Code shows when the console in the browser and the server
 * it talks to are not the same build (REL-2 S1-T5).
 *
 * It appears when the server answers `GET /health` with a `version` that
 * is not the console's own. That is the one moment a person needs to know:
 * the page they are looking at was built by one release and served by
 * another, and half of what they see may be from the other one.
 *
 * Renders NOTHING when the versions match, and nothing when the server's
 * version is unknown (the health route did not answer) — a banner that
 * says "the server runs unknown" is noise, not information.
 */
export function VersionBanner() {
  const health = useService((signal) => versionService.getServerHealth(signal), []);
  const mismatch =
    health.status === "success" ? versionMismatch(health.data.version) : null;
  if (!mismatch) return null;

  return (
    <p
      role="status"
      className="mb-4 flex items-start gap-2 rounded-lg bg-amber-500/10 px-3.5 py-2.5 text-sm text-amber-700 dark:text-amber-400"
    >
      <TriangleAlert className="mt-0.5 size-4 shrink-0" aria-hidden />
      <span>
        The server runs {mismatch.server}, this console is {mismatch.console}.
        Reload, or ask your operator to update.
      </span>
    </p>
  );
}
