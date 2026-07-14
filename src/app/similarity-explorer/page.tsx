import { redirect } from "next/navigation"

/** Old route — combined with semantic search. */
export default function SimilarityExplorerRedirectPage() {
  redirect("/semantic-search")
}
