import type { Metadata } from "next"

export const metadata: Metadata = {
  title: "Intelligence Knowledge · Rantai Lake",
  description:
    "Knowledge library of summaries and source queries absorbed from user query results.",
}

export default function IntelligenceKnowledgeLayout({
  children,
}: {
  children: React.ReactNode
}) {
  return children
}
