import { afterEach, describe, expect, it, mock } from "bun:test"

// The banner reads the server's version through the service layer; the
// stub answers with whatever version the test names, and counts the calls
// so a test can wait for the answer to have arrived. `mock.module` is
// hoisted to the top of the file by bun's test runner, so the stub is in
// place before the import below pulls the banner (and its service) in.
let serverVersion = "0.1.0";
let calls = 0;
mock.module("@/services", () => ({
  versionService: {
    getServerHealth: async () => {
      calls += 1;
      return {
        status: "ok",
        database: "ok",
        version: serverVersion,
      };
    },
  },
}));

import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { VersionBanner } from "./version-banner";
import { CONSOLE_VERSION } from "./console-version";

describe("VersionBanner", () => {
  afterEach(() => cleanup());

  it("names both versions when the server runs another build", async () => {
    serverVersion = "9.9.9";
    render(<VersionBanner />);
    const banner = await screen.findByRole("status");
    expect(banner.textContent).toContain("The server runs 9.9.9");
    expect(banner.textContent).toContain(`this console is ${CONSOLE_VERSION}`);
    expect(banner.textContent).toContain("Reload, or ask your operator to update.");
  });

  it("says nothing when the server runs the same build as the console", async () => {
    serverVersion = CONSOLE_VERSION;
    calls = 0;
    const { container } = render(<VersionBanner />);
    // Wait for the answer to have arrived, so an empty tree is a decision
    // and not a request that never came back.
    await waitFor(() => expect(calls).toBeGreaterThan(0));
    expect(container.textContent).toBe("");
  });
});
