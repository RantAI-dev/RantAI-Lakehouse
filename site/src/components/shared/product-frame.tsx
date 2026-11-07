import { MascotMark } from "./mascot-svg";

/**
 * A real screenshot of the console, framed in the lake's style (WEB-5 D19):
 * the site's glass card and hairline, a slim title strip with the mark and
 * what the window shows, and a lake-blue glow under it like the moon on the
 * water. Not a browser mock-up: no fake address bar and no traffic lights.
 *
 * The picture is real UI, so it carries no "Illustration" label; its alt
 * text says what it shows, and the caller sets it beside its caption. It is never the largest paint: it
 * sits far below the first screen, is `loading="lazy"` and
 * `decoding="async"`, and has its width and height, so it shifts nothing
 * when it arrives (CLS 0). The source is a crop at the screenshot's own
 * pixels, so there is one file and no 2x (the handoff says why).
 */
export function ProductFrame({
  src,
  width,
  height,
  alt,
  windowTitle,
  narrow,
  className,
}: Readonly<{
  src: string;
  width: number;
  height: number;
  /**
   * A narrower crop of the same screenshot for phones, where the full one
   * would show its text at half size. Its cut edge fades out, so it reads
   * as the window going on, not as a cut.
   */
  narrow?: { src: string; width: number; height: number };
  alt: string;
  /** What the window shows, in the title strip ("Home"). */
  windowTitle: string;
  className?: string;
}>) {
  return (
    <div className={className}>
      <div className="relative">
        <div
          aria-hidden="true"
          className="pointer-events-none absolute -inset-x-6 -bottom-10 h-24 rounded-[50%] bg-[radial-gradient(ellipse,rgba(68,166,250,0.28)_0%,transparent_70%)] blur-md"
        />
        <div className="relative overflow-hidden rounded-[22px] bg-[#0B1A4A]/60 shadow-[0_40px_100px_-40px_rgba(2,5,20,0.95)] ring-1 ring-[#E8F4FF]/14 backdrop-blur-md">
          <div className="flex items-center gap-2.5 border-b border-[#E8F4FF]/10 px-4 py-2.5">
            <MascotMark className="h-3.5 w-auto" />
            <span className="text-[12px] leading-4 font-medium text-[#E8F4FF]/75">{windowTitle}</span>
          </div>
          <div className="p-2 sm:p-3">
            <picture>
              {narrow && (
                <source media="(max-width: 47.99rem)" srcSet={narrow.src} width={narrow.width} height={narrow.height} />
              )}
              {/* A plain <img> in a <picture>: a static export has no image optimizer, and the files are WebP at their own size. */}
              <img
                src={src}
                width={width}
                height={height}
                alt={alt}
                loading="lazy"
                decoding="async"
                className={
                  narrow
                    ? "block h-auto w-full rounded-[14px] ring-1 ring-black/10 max-md:[mask-image:linear-gradient(90deg,#000_78%,transparent)]"
                    : "block h-auto w-full rounded-[14px] ring-1 ring-black/10"
                }
              />
            </picture>
          </div>
        </div>
      </div>
    </div>
  );
}
