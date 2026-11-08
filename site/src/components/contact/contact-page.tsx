import { getMessages } from "next-intl/server";
import { ArrowUpRight, Mail } from "lucide-react";
import { HeroIntro } from "@/components/home/hero-intro";
import { SiteFooter } from "@/components/layout/site-footer";
import { SkipLink } from "@/components/layout/skip-link";
import { TopBar } from "@/components/layout/top-bar";
import { JsonLd } from "@/components/shared/json-ld";
import { Container, DisplayHeading, Eyebrow, LeadText } from "@/components/shared/primitives";
import { SocialIcon } from "@/components/shared/social-icons";
import type { Locale } from "@/i18n/routing";
import type { CommonCopy, ContactCopy } from "@/lib/copy";
import { faqJsonLd } from "@/lib/faq-json-ld";
import { CONTACT_EMAIL, LINKS, SOCIAL_LINKS, SOURCE_LABEL } from "@/lib/home-content";
import { ContactForm } from "./contact-form";
import { Faq } from "./faq";

/**
 * The contact page, `/contact/` and `/id/contact/` (WEB-5 D15), shared by its
 * two routes the way the home page is (D9). It ports the RantAI site's
 * contact page to the lake's palette: the intro, the form, the direct
 * contacts with "Follow RantAI" (D17), then the product FAQ, whose
 * `FAQPage` JSON-LD is on this page only.
 *
 * It is a page at the surface of the lake: the same top bar, footer and
 * night palette as the home page, with no depth gauge, no scene and no
 * scroll story. Everything but the form is server-rendered, so the page
 * reads in full without JavaScript, and the FAQ opens without it too.
 */
export async function ContactPage({ locale }: Readonly<{ locale: Locale }>) {
  const messages = await getMessages({ locale });
  const copy = messages.contact as ContactCopy;
  const common = messages.common as CommonCopy;

  return (
    <div className="lk-root relative min-h-screen overflow-x-clip">
      {/* WEB-5 O47: only a light reveal of the heading on this page (home/hero-intro.tsx). */}
      <HeroIntro variant="lite" />
      <SkipLink label={common.skipToContent} />
      <TopBar locale={locale} page="contact" nav={common.nav} cta={common.cta} language={common.language} />
      <main id="main" tabIndex={-1} className="relative isolate">
        <SurfaceBackdrop />
        <section aria-labelledby="contact-heading" className="relative pt-28 md:pt-36">
          {/* `minmax(0,1fr)`: an implicit `auto` column grew to the form's min-content and overflowed 360 px (measured). */}
          <Container className="grid grid-cols-[minmax(0,1fr)] gap-x-16 gap-y-10 lg:grid-cols-[minmax(0,5fr)_minmax(0,7fr)] lg:grid-rows-[auto_1fr]">
            <div>
              <Eyebrow>{copy.hero.eyebrow}</Eyebrow>
              <DisplayHeading as="h1" id="contact-heading" size="md" className="mt-5 whitespace-pre-line">
                {copy.hero.title}
              </DisplayHeading>
              <LeadText className="mt-6 max-w-[27rem]">{copy.hero.description}</LeadText>
            </div>

            <div className="lg:col-start-2 lg:row-span-2 lg:row-start-1 lg:pt-2">
              <ContactForm text={copy.form} to={CONTACT_EMAIL} />
            </div>

            <div className="lg:col-start-1 lg:row-start-2">
              <h2 className="text-[13px] font-medium text-[#E8F4FF]/70 lg:mt-4">{copy.direct.title}</h2>
              <ul className="mt-3 border-t border-[#E8F4FF]/12">
                <DirectLink href={LINKS.email} label={copy.direct.email} value={CONTACT_EMAIL} icon={<Mail className="size-[18px]" aria-hidden="true" />} />
                <DirectLink
                  href={LINKS.source}
                  external
                  label={copy.direct.source}
                  value={SOURCE_LABEL}
                  icon={<SocialIcon id="github" size={17} />}
                />
              </ul>
              <div className="mt-8 flex flex-wrap items-center gap-x-4 gap-y-2">
                <h2 className="text-[13px] font-medium text-[#E8F4FF]/70">{copy.direct.follow}</h2>
                <ul className="-ml-2 flex items-center gap-1">
                  {SOCIAL_LINKS.map((s) => (
                    <li key={s.id}>
                      <a
                        href={s.href}
                        target="_blank"
                        rel="noopener noreferrer"
                        aria-label={s.label}
                        className="grid size-11 place-items-center rounded-full text-[#E8F4FF]/70 transition-[color,background-color,transform] duration-200 ease-out hover:-translate-y-0.5 hover:bg-[#E8F4FF]/[0.08] hover:text-[#E8F4FF] active:translate-y-0 active:scale-95 motion-reduce:transition-none motion-reduce:hover:translate-y-0"
                      >
                        <SocialIcon id={s.id} />
                      </a>
                    </li>
                  ))}
                </ul>
              </div>
            </div>
          </Container>
        </section>
        <Faq copy={copy.faq} />
      </main>
      <SiteFooter locale={locale} page="contact" copy={common.footer} />
      <JsonLd data={faqJsonLd(copy.faq.items)} />
    </div>
  );
}

function DirectLink({
  href,
  label,
  value,
  icon,
  external = false,
}: Readonly<{ href: string; label: string; value: string; icon: React.ReactNode; external?: boolean }>) {
  return (
    <li className="border-b border-[#E8F4FF]/12">
      <a
        href={href}
        {...(external ? { target: "_blank", rel: "noopener noreferrer" } : {})}
        className="group flex items-center gap-4 rounded-xl py-4 transition-opacity duration-200 ease-out active:opacity-80"
      >
        <span className="grid size-10 shrink-0 place-items-center rounded-full bg-[#E8F4FF]/[0.08] text-[#E8F4FF]/85 transition-colors duration-200 ease-out group-hover:bg-[#44A6FA] group-hover:text-[#020514]">
          {icon}
        </span>
        <span className="min-w-0 flex-1">
          <span className="block text-[15px] font-medium text-[#E8F4FF]">{label}</span>
          <span className="block truncate text-[14px] text-[#E8F4FF]/70">{value}</span>
        </span>
        <ArrowUpRight
          className="size-4 shrink-0 text-[#E8F4FF]/40 transition-[color,translate] duration-200 ease-out group-hover:translate-x-0.5 group-hover:-translate-y-0.5 group-hover:text-[#E8F4FF] motion-reduce:transition-none"
          aria-hidden="true"
        />
      </a>
    </li>
  );
}

/**
 * The page's water: one solid dark colour, the abyss the FAQ and the footer
 * sit on (O39: backgrounds are solid; no gradient, no moon glow).
 * Decorative and static.
 */
function SurfaceBackdrop() {
  return (
    <div aria-hidden="true" className="pointer-events-none absolute inset-x-0 top-0 -z-10 h-[70rem] overflow-hidden">
      {/* O39: one solid dark colour, with no gradient and no moon glow. */}
      <div className="absolute inset-0 bg-[#020514]" />
    </div>
  );
}
