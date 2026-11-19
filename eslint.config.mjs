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
