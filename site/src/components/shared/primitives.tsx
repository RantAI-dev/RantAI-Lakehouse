"use client";

import * as React from "react";
import { cn } from "@/lib/utils";

/**
 * Small layout and button primitives shared by the landing sections. They
 * hold the page's measurements so no section re-decides them: a 1120 px
 * column with a 24 px gutter (16 px on phones), per plan §4.
 */

export function Container({
  className,
  children,
  ...rest
}: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div className={cn("mx-auto w-full max-w-[1120px] px-4 md:px-6", className)} {...rest}>
      {children}
    </div>
  );
}

export function Eyebrow({
  className,
  children,
  ...rest
}: React.HTMLAttributes<HTMLParagraphElement>) {
  return (
    <p
      className={cn(
        "lk-mono text-[12px] leading-4 font-medium tracking-[0.22em] text-[#44A6FA] uppercase",
        className,
      )}
      {...rest}
    >
      {children}
    </p>
  );
}

type CtaProps = React.AnchorHTMLAttributes<HTMLAnchorElement> & {
  variant?: "solid" | "ghost";
  size?: "md" | "sm";
};

/** A real `<a>` (plan §2.8): works without JavaScript and with a keyboard. */
export function CtaLink({ variant = "solid", size = "md", className, children, ...rest }: CtaProps) {
  return (
    <a
      className={cn(
        "group relative inline-flex items-center justify-center gap-2 rounded-full font-medium whitespace-nowrap transition-[transform,background-color,box-shadow,border-color] duration-300 ease-out",
        size === "md" ? "h-12 px-7 text-[15px]" : "h-9 px-4 text-[13px]",
        variant === "solid"
          ? "bg-[#44A6FA] text-[#020514] shadow-[0_0_0_0_rgba(68,166,250,0)] hover:-translate-y-0.5 hover:bg-[#6bbaff] hover:shadow-[0_10px_40px_-8px_rgba(68,166,250,0.7)]"
          : "border border-[#E8F4FF]/35 text-[#E8F4FF] hover:-translate-y-0.5 hover:border-[#E8F4FF]/70 hover:bg-[#E8F4FF]/10",
        className,
      )}
      {...rest}
    >
      {children}
    </a>
  );
}

/**
 * Big serif-italic title. Size and line height travel together as one class
 * string: `tailwind-merge` drops a `leading-*` that precedes a later
 * `text-[size]`, so overriding the size alone used to reset the line height
 * to the console's 48 px h1 rule and stacked the lines on top of each other.
 */
const HEADING_SIZE = {
  xl: "text-[clamp(4.5rem,9vw,7.5rem)] leading-[1.02]",
  lg: "text-[clamp(3rem,7vw,5.75rem)] leading-[1.02]",
  md: "text-[clamp(2.5rem,5vw,4rem)] leading-[1.04]",
} as const;

export function DisplayHeading({
  as: Tag = "h2",
  size = "lg",
  className,
  children,
  ...rest
}: {
  as?: "h1" | "h2";
  size?: keyof typeof HEADING_SIZE;
  className?: string;
  children: React.ReactNode;
} & React.HTMLAttributes<HTMLHeadingElement>) {
  return (
    <Tag
      className={cn(
        "lk-display text-balance tracking-[-0.015em] text-[#E8F4FF]",
        HEADING_SIZE[size],
        className,
      )}
      {...rest}
    >
      {children}
    </Tag>
  );
}

export function LeadText({
  className,
  children,
}: {
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <p
      className={cn(
        "text-[17px] leading-[1.6] text-pretty text-[#E8F4FF]/80 md:text-[19px] md:leading-[1.6]",
        className,
      )}
    >
      {children}
    </p>
  );
}
