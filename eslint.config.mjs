import { defineConfig, globalIgnores } from "eslint/config";
import nextVitals from "eslint-config-next/core-web-vitals";
import nextTs from "eslint-config-next/typescript";

const eslintConfig = defineConfig([
  ...nextVitals,
  ...nextTs,
  // `WEB-9` D4: the console has one dropdown. A native `<select>` opens in
  // the browser's own light palette under the dark theme and carries its own
  // class string, so a new one is refused at lint time instead of in review.
  // An exception belongs at the site, with a comment saying why
  // (`AGENTS.md`, choke points).
  // `UX-7`: the same for `<datalist>`: the browser draws it as a long unstyled
  // list (owner QA 2026-11-20, the report time zone). Use `Combobox`.
  {
    files: ["src/**/*.{ts,tsx}"],
    rules: {
      "no-restricted-syntax": [
        "error",
        {
          selector: "JSXOpeningElement[name.name='select']",
          message:
            "Use SelectField (`@/components/patterns/select-field`); a native select is unreadable in the dark theme (`WEB-9`)",
        },
        {
          selector: "JSXOpeningElement[name.name='datalist']",
          message:
            "Use Combobox (`@/components/ui/combobox`); a native datalist is drawn by the browser, not the console (`UX-7`)",
        },
      ],
    },
  },
  // `BI-29` D7: feature code reaches a client only through `@/services`
  // (contracts -> clients -> `services/index.ts` -> features). The
  // dashboard features imported `@/services/clients/*` in 21 files (types
  // from `bi-store`) and one hook fetched on its own; the types are exported
  // from `@/services` now, and this keeps it that way. `src/features/auth` is
  // the one exception: `services/clients/auth.ts` documents that login, logout,
  // `me` and change-password are not registry services, so `AuthProvider` and
  // the login pages import that client directly.
  {
    files: ["src/features/**/*.{ts,tsx}"],
    ignores: ["src/features/auth/**"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              group: ["@/services/clients/*", "**/services/clients/*"],
              message:
                "Import from `@/services`, not from a client (`BI-29`): add the method to the contract and register it in `services/index.ts`",
            },
          ],
        },
      ],
    },
  },
  // `UX-6` D3: `navigator.clipboard` exists only in a secure context (https
  // or localhost). A console opened by IP over plain http has none, so a
  // direct read throws there or fails silently behind `?.` / `() => {}` --
  // QA found it on the staging console and on the one-time password and
  // secret reveal screens, where the value is shown once. `copyText` falls
  // back to a textarea and `execCommand("copy")` and answers whether it
  // worked; the failure needs a toast, so the direct call is refused here.
  // Not in the `WEB-9` block above: a second `no-restricted-syntax` array
  // would replace that select rule (flat config replaces, it does not merge).
  {
    files: ["src/**/*.{ts,tsx}"],
    ignores: ["src/lib/copy-text.ts", "src/**/*.test.{ts,tsx}"],
    rules: {
      "no-restricted-properties": [
        "error",
        {
          object: "navigator",
          property: "clipboard",
          message:
            "Use `copyText` (`@/lib/copy-text`); `navigator.clipboard` does not exist over plain http (`UX-6`)",
        },
      ],
    },
  },
  // Override default ignores of eslint-config-next.
  globalIgnores([
    // Default ignores of eslint-config-next:
    ".next/**",
    "out/**",
    "build/**",
    "next-env.d.ts",
  ]),
]);

export default eslintConfig;
