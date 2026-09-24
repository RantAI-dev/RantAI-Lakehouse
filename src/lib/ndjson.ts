/**
 * Reads a newline-delimited JSON response body, yielding each object as its
 * line arrives — used for the Copilot chat progress stream.
 */
export async function* readNdjson(res: Response): AsyncGenerator<unknown> {
  const reader = res.body?.getReader()
  if (!reader) return
  const decoder = new TextDecoder()
  let buffer = ""
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    buffer += decoder.decode(value, { stream: true })
    let newline = buffer.indexOf("\n")
    while (newline >= 0) {
      const line = buffer.slice(0, newline).trim()
      buffer = buffer.slice(newline + 1)
      if (line) yield JSON.parse(line)
      newline = buffer.indexOf("\n")
    }
  }
  buffer += decoder.decode()
  if (buffer.trim()) yield JSON.parse(buffer)
}
