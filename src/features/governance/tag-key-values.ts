/**
 * The values field of the Tag keys form is one text box: values are split on
 * commas and whitespace, empties dropped, and nothing else is changed. A
 * value the API would refuse (an upper-case letter, a repeat, more than 50)
 * is sent as typed so the refusal is the API's own sentence, not a second
 * validator here that could disagree with it.
 */
export function parseTagValues(text: string): string[] {
  return text
    .split(/[\s,]+/)
    .map((v) => v.trim())
    .filter((v) => v !== "")
}

/** The inverse, for filling the edit form: one value per entry, comma-separated. */
export function formatTagValues(values: string[]): string {
  return values.join(", ")
}
