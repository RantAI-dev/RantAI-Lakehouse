import { LoadingSkeleton } from "@/components/patterns/page-states"

/** Default route loading fallback. */
export default function Loading() {
  return <LoadingSkeleton rows={8} />
}
