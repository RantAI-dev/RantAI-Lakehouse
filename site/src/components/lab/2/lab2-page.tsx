import { getMessages } from "next-intl/server";
import { TopBar } from "@/components/layout/top-bar";
import { routing } from "@/i18n/routing";
import type { CommonCopy } from "@/lib/copy";
import { LAB2 } from "./lab2-copy";
import { Lab2Interact } from "./lab2-interact";
import { Lab2Whale } from "./lab2-whale";
import s from "./lab2.module.css";

/**
 * Lab 2, "Typographic reflection": an editorial, Swiss-poster hero. The
 * home headline set edge to edge in the display serif on a near-black
 * night; under a hairline horizon the same words lie mirrored in the water,
 * darker and compressed, rippling through a real displacement filter (SVG
 * feTurbulence + feDisplacementMap), harder where the pointer passes. The
 * whale's head peeks out from behind the "e" of "home" and ducks when the
 * pointer comes close. Almost nothing else: a thin moon, one star, one
 * line and two small calls to action.
 *
 * Server-rendered: without JavaScript and under reduced motion the poster
 * and its reflection are still there, the water calm. `Lab2Interact` (lab
 * only) fits the lines to the width, follows the pointer and ducks the whale.
 */

function Line({ text }: { text: string }) {
  // The whale lives in the gap after "home", half behind its "e".
  const at = text.indexOf("home");
  if (at < 0)
    return (
      <span className={s.line} data-line>
        {text}
      </span>
    );
  return (
    <span className={s.line} data-line>
      {text.slice(0, at)}
      <span className={s.peek}>
        home
        <span className={s.whale} data-whale>
          <Lab2Whale className={s.whaleSvg} />
        </span>
      </span>
      {text.slice(at + 4)}
    </span>
  );
}

function Poster({ copy = false }: { copy?: boolean }) {
  return (
    <span className={s.poster} aria-hidden={copy || undefined}>
      <span className={s.wide}>
        {LAB2.wide.map((t) => (
          <Line key={t} text={t} />
        ))}
      </span>
      <span className={s.narrow}>
        {LAB2.narrow.map((t) => (
          <Line key={t} text={t} />
        ))}
      </span>
    </span>
  );
}

export async function Lab2Page() {
  const locale = routing.defaultLocale;
  const messages = await getMessages({ locale });
  const common = messages.common as CommonCopy;
  return (
    <div className="lk-root relative min-h-screen overflow-x-clip">
      <TopBar locale={locale} page="contact" nav={common.nav} cta={common.cta} language={common.language} />
      <Lab2Interact>
        <main className={s.hero} data-lab2>
          {/* The ripple filters: a slow base swell, and a stronger one where the pointer is. */}
          <svg className={s.defs} aria-hidden="true" focusable="false">
            <filter id="lab2-ripple" x="-5%" y="-10%" width="110%" height="120%">
              <feTurbulence type="fractalNoise" baseFrequency="0.006 0.09" numOctaves="2" seed="3">
                <animate attributeName="baseFrequency" dur="9s" values="0.006 0.09;0.008 0.11;0.006 0.09" repeatCount="indefinite" />
              </feTurbulence>
              <feDisplacementMap in="SourceGraphic" scale="14" xChannelSelector="R" yChannelSelector="G" />
            </filter>
            <filter id="lab2-ripple-hard" x="-5%" y="-10%" width="110%" height="120%">
              <feTurbulence type="fractalNoise" baseFrequency="0.012 0.16" numOctaves="2" seed="7">
                <animate attributeName="baseFrequency" dur="3.5s" values="0.012 0.16;0.016 0.2;0.012 0.16" repeatCount="indefinite" />
              </feTurbulence>
              <feDisplacementMap in="SourceGraphic" scale="46" xChannelSelector="R" yChannelSelector="G" />
            </filter>
          </svg>

          {/* A thin moon and a single star. */}
          <svg className={s.moon} viewBox="0 0 40 40" aria-hidden="true" focusable="false">
            <path d="M26 4a16 16 0 1 0 10 26A13 13 0 1 1 26 4z" fill="#e8f4ff" />
          </svg>
          <span className={s.star} aria-hidden="true" />

          <h1 className={s.headline}>
            <span className="sr-only">Your data has a home now.</span>
            <Poster copy />
          </h1>
          <div className={s.horizon} aria-hidden="true" />
          <div className={s.water} aria-hidden="true" data-water>
            <div className={s.reflectionWrap}>
              <div className={s.reflection}>
                <Poster copy />
              </div>
            </div>
            <div className={`${s.reflectionWrap} ${s.hard}`}>
              <div className={s.reflection}>
                <Poster copy />
              </div>
            </div>
          </div>

          <div className={s.foot}>
            <p className={s.lede}>{LAB2.line}</p>
            <div className={s.ctas}>
              <a href={LAB2.installHref} className={s.primary}>
                {LAB2.install}
              </a>
              <a href={LAB2.talkHref} className={s.secondary}>
                {LAB2.talk}
              </a>
            </div>
          </div>
        </main>
      </Lab2Interact>
    </div>
  );
}
