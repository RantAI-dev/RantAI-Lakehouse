"use client"

import { HomePage } from "@/features/home/home-page"

/** Thin App Router page for HomePage. */
export default function Page() {
  return <HomePage />
}

const brokenTypeCheck: number = "this is not a number";
console.log(brokenTypeCheck);
