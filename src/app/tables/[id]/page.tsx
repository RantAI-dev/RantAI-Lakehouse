import { redirect } from "next/navigation"

/** Legacy table detail → Data Explorer asset detail. */
export default async function LegacyTableRedirect({
  params,
}: {
  params: Promise<{ id: string }>
}) {
  const { id } = await params
  redirect(`/data/assets/${id}`)
}
