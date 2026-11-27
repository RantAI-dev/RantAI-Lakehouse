/**
 * The first focusable element of every page (WEB-5 D10): it jumps past the
 * top bar to `<main id="main" tabIndex={-1}>`. Shared by the home page and
 * the contact page so both keep the same one.
 */
export function SkipLink({ label }: Readonly<{ label: string }>) {
  return (
    <a
      href="#main"
      className="sr-only z-[60] rounded-full bg-[#E8F4FF] px-4 py-2 text-[14px] font-medium text-[#020514] focus:not-sr-only focus:fixed focus:top-3 focus:left-3"
    >
      {label}
    </a>
  );
}
