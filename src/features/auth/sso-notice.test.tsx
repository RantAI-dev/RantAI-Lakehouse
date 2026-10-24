import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, mock } from "bun:test";

/**
 * `SsoNotice` reads `useSearchParams()`, which throws outside a Next.js
 * router context. `mock.module` is hoisted by bun's test runner, so the
 * stub is in place before the import below resolves (same pattern as
 * `login-page.test.tsx`). Each test sets `currentParams` before rendering.
 */
let currentParams = new URLSearchParams();
mock.module("next/navigation", () => ({
  useSearchParams: () => currentParams,
}));

import { SSO_NOTICE_PARAM, SsoNotice } from "./sso-notice";

afterEach(() => {
  cleanup();
  currentParams = new URLSearchParams();
});

describe("SsoNotice (SEC-39)", () => {
  it("shows the fixed group-overage notice when the callback asked for it", () => {
    currentParams = new URLSearchParams({
      [SSO_NOTICE_PARAM]: "group_overage",
    });
    render(<SsoNotice />);
    expect(screen.getByRole("status").textContent).toContain(
      "Your account is in more groups than the sign-in token can list, so no group-based role was applied.",
    );
  });

  it("shows nothing on an ordinary sign-in", () => {
    render(<SsoNotice />);
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("shows nothing for a code it does not know", () => {
    currentParams = new URLSearchParams({
      [SSO_NOTICE_PARAM]: "something-else",
    });
    render(<SsoNotice />);
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("shows nothing for a code that only exists on the object prototype", () => {
    // PR review: `NOTICES["toString"]` is inherited, not a notice.
    for (const code of ["toString", "constructor", "__proto__"]) {
      currentParams = new URLSearchParams({ [SSO_NOTICE_PARAM]: code });
      render(<SsoNotice />);
      expect(screen.queryByRole("status")).toBeNull();
      cleanup();
    }
  });
});
