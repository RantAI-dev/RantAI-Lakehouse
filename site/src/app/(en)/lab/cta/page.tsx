import type { Metadata } from "next";
import { LAB_META } from "@/components/lab/cta/lab-shell";

export const metadata: Metadata = { title: "Lab: footer cards", ...LAB_META };

const VARIANTS = [
  { n: 1, name: "Two doors", line: "Run it yourself, or talk to us. Two equal halves, a real terminal and the real address." },
  { n: 2, name: "One command", line: "A short serif headline and one command line. The whale peeks over the card." },
  { n: 3, name: "The console surfaces", line: "The real console rising out of the water line, with a short headline and two buttons." },
];

export default function Page() {
  return (
    <main className="min-h-screen bg-[#04101c] px-4 py-24 text-[#E8F4FF]">
      <div className="mx-auto max-w-3xl">
        <h1 className="lk-display text-4xl">Footer card variants</h1>
        <p className="mt-3 text-[15px] text-[#E8F4FF]/65">Temporary. Each page is the last screen of sea and the real footer.</p>
        <ul className="mt-10 grid gap-3 sm:grid-cols-3">
          {VARIANTS.map((v) => (
            <li key={v.n}>
              <a
                href={`/lab/cta/${v.n}/`}
                className="flex h-full flex-col gap-2 rounded-2xl bg-[var(--lk-n-surface)] p-5 ring-1 ring-[var(--lk-n-hair)] transition-[transform,box-shadow] duration-200 ease-out outline-none hover:-translate-y-0.5 hover:ring-[#8fcbff]/40 focus-visible:ring-2 focus-visible:ring-[#44A6FA] active:translate-y-0 motion-reduce:transition-none"
              >
                <span className="text-[12px] font-medium text-[#8fcbff]">Variant {v.n}</span>
                <span className="text-[18px] font-medium">{v.name}</span>
                <span className="text-[14px] leading-6 text-[#E8F4FF]/65">{v.line}</span>
              </a>
            </li>
          ))}
        </ul>
      </div>
    </main>
  );
}
