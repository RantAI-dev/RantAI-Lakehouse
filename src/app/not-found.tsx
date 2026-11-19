import { NotFoundState } from "@/components/patterns/page-states"

/**
 * The page for an address no route answers (`adm-121`). Inside the console
 * the route guard shows the same state in the page area; this file covers
 * the addresses outside the shell (a public link that is not one) and gives
 * Next's own 404 the console's wording instead of its stock page.
 */
export default function NotFound() {
  return <NotFoundState className="m-4" />
}
