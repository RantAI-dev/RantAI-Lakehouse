import { AlertRulePage } from "@/features/alerts/alert-rule-page"

/** `WEB-7` T7 (D2/D4/D7): a saved rule, loaded by id, as its own page. */
export default async function Page({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params
  return <AlertRulePage ruleId={decodeURIComponent(id)} />
}
