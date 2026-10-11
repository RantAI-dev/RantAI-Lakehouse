"use client";

import * as React from "react";
import { ASSISTANT } from "./landing-copy";
import { Container, DisplayHeading, LeadText } from "./landing-ui";

/** S4 — "Ask, don't dig": the assistant, with an illustrated chat. */
export function AssistantSection() {
  return (
    <section id="assistant" className="relative bg-[#050A30] py-28 md:py-36">
      <Container className="grid items-center gap-12 md:grid-cols-[1fr_1.1fr]">
        <div>
          <DisplayHeading>{ASSISTANT.headline}</DisplayHeading>
          <LeadText className="mt-5 max-w-[32rem]">{ASSISTANT.sub}</LeadText>
        </div>
        <div className="rounded-3xl border border-[#E8F4FF]/10 bg-[#020514]/60 p-6">
          <p className="lk-mono text-[11px] tracking-[0.2em] text-[#E8F4FF]/55 uppercase">{ASSISTANT.caption}</p>
          <p className="mt-4 rounded-2xl bg-[#44A6FA]/15 px-4 py-3 text-[15px]">{ASSISTANT.userOne}</p>
          <p className="mt-3 rounded-2xl bg-[#E8F4FF]/[0.06] px-4 py-3 text-[15px]">{ASSISTANT.answerOne}</p>
          <p className="mt-3 rounded-2xl bg-[#44A6FA]/15 px-4 py-3 text-[15px]">{ASSISTANT.userTwo}</p>
          <p className="mt-3 rounded-2xl border border-[#F2C14E]/40 px-4 py-3 text-[15px]">
            {ASSISTANT.approvalLabel} · {ASSISTANT.approvalAction}{" "}
            <code className="lk-mono text-[#F2C14E]">{ASSISTANT.approvalTarget}</code>
          </p>
        </div>
      </Container>
    </section>
  );
}
