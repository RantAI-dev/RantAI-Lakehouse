import { ArrowRight, ChevronDown } from "lucide-react";
import type { ContactCopy } from "@/lib/copy";
import { Container, DisplayHeading, Eyebrow } from "@/components/shared/primitives";

/**
 * The product's common questions (WEB-5 D15, after the RantAI site's
 * `components/shared/faq.tsx`). A disclosure list of `<details>`, so it
 * opens and closes by keyboard and works without JavaScript; the shared
 * `name` lets a browser that supports it keep one answer open at a time.
 * Where the browser can animate a `<details>` the answer eases open and
 * shut and the chevron turns (`.lk-faq`, app/globals.css); under reduced
 * motion, or elsewhere, it opens at once.
 * Every answer is a claim, traced to the backlog in `content/claims.ts`.
 *
 * WEB-5 D21: the home page shows four of the same questions (`pick`, by
 * index, so the answers and their claims are the very same ones) before
 * the final call, with a link to all of them on the contact page (`more`).
 * The `FAQPage` JSON-LD stays on the contact page.
 */
export function Faq({
  copy,
  pick,
  more,
}: Readonly<{
  copy: ContactCopy["faq"];
  /** Indexes of `copy.items` to show; all of them when left out. */
  pick?: readonly number[];
  more?: { label: string; href: string };
}>) {
  const items = pick ? pick.map((i) => copy.items[i]) : copy.items;
  return (
    <section id="faq" aria-labelledby="faq-heading" className="relative scroll-mt-16 py-20 md:py-28">
      <Container className="grid grid-cols-[minmax(0,1fr)] gap-10 lg:grid-cols-[minmax(0,5fr)_minmax(0,7fr)] lg:gap-16">
        <div className="lg:sticky lg:top-28 lg:self-start">
          <Eyebrow>{copy.eyebrow}</Eyebrow>
          <DisplayHeading id="faq-heading" size="md" className="mt-5 whitespace-pre-line">
            {copy.title}
          </DisplayHeading>
          {more && (
            <a
              href={more.href}
              className="group mt-7 inline-flex min-h-10 items-center gap-1.5 text-[15px] font-medium text-[#8fcbff] transition-colors duration-200 ease-out hover:text-[#E8F4FF]"
            >
              {more.label}
              <ArrowRight aria-hidden="true" className="size-4 transition-transform duration-200 ease-out group-hover:translate-x-0.5 motion-reduce:transition-none" />
            </a>
          )}
        </div>
        <ul className="lk-faq border-t border-[#E8F4FF]/12">
          {items.map((item, i) => (
            <li key={item.q} className="border-b border-[#E8F4FF]/12">
              <details name="lakehouse-faq" open={i === 0} className="group">
                <summary className="flex cursor-pointer list-none items-center justify-between gap-6 rounded-lg py-5 text-left text-[17px] leading-snug font-medium text-pretty text-[#E8F4FF]/90 transition-colors duration-200 ease-out marker:hidden hover:text-[#E8F4FF] focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-[#44A6FA] group-open:text-[#E8F4FF] sm:py-6 sm:text-[19px] [&::-webkit-details-marker]:hidden">
                  {item.q}
                  <span
                    aria-hidden="true"
                    className="grid size-8 shrink-0 place-items-center rounded-full bg-[#E8F4FF]/[0.08] text-[#E8F4FF]/80 transition-[background-color,color,rotate,scale] duration-200 ease-out group-hover:bg-[#E8F4FF]/[0.16] group-active:scale-90 group-open:rotate-180 group-open:bg-[#E8F4FF] group-open:text-[#020514] motion-reduce:transition-none"
                  >
                    <ChevronDown className="size-4" />
                  </span>
                </summary>
                <p className="max-w-2xl pr-12 pb-6 text-[15px] leading-relaxed text-pretty text-[#E8F4FF]/75 sm:text-[16px]">
                  {item.a}
                </p>
              </details>
            </li>
          ))}
        </ul>
      </Container>
    </section>
  );
}
