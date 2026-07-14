import { redirect } from "next/navigation"

/** Vector jobs are configured under Pipelines → Vector jobs. */
export default function EmbeddingsRedirectPage() {
  redirect("/pipelines?tab=vector")
}
