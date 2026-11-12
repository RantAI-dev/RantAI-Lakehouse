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
  // Override default ignores of eslint-config-next.
  globalIgnores([
    // Default ignores of eslint-config-next:
    ".next/**",
    "out/**",
    "build/**",
    "next-env.d.ts",
    // WEB-5 D11: the Lakehouse site is its own package with its own lint
    // run (`cd site && bun run lint`); the console's run never reads it.
    "site/**",
  ]),
]);

export default eslintConfig;
