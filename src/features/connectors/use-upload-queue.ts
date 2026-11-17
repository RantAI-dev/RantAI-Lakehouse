"use client"

import * as React from "react"
import { planPick } from "@/lib/uploads"
import { type QueueFile, type QueueTransport, UploadQueue } from "@/lib/upload-queue"
import { uploadService } from "@/services"

/** The queue's five calls, on the upload service (`discard` is the service's `remove`). */
const transport: QueueTransport = {
  createSession: (input, signal) => uploadService.createSession(input, signal),
  putPart: (id, number, bytes, signal) => uploadService.putPart(id, number, bytes, signal),
  listParts: (id, signal) => uploadService.listParts(id, signal),
  complete: (id, signal) => uploadService.complete(id, signal),
  get: (id, signal) => uploadService.get(id, signal),
  discard: (id, signal) => uploadService.remove(id, signal),
}

const NO_FILES: readonly QueueFile[] = []
const never = () => () => {}

/**
 * The page's upload queue (`@/lib/upload-queue`) as React state.
 *
 * Not `useServiceAction`: that hook is one action with one result, and this is
 * up to ten files, each with its own requests, retries and progress, so the
 * queue keeps that state and the page reads it as an external store. The queue
 * is made in an effect and stopped by its cleanup, so a development re-mount
 * (React strict mode runs the effect twice) gets a live queue and not the one
 * the first cleanup stopped. Leaving the page stops the queue; a file still
 * being sent is sent again from the start on the next visit (decision 7 of the
 * feature page), and the API removes its unfinished session after 24 hours.
 */
export function useUploadQueue() {
  const [queue, setQueue] = React.useState<UploadQueue | null>(null)

  React.useEffect(() => {
    const made = new UploadQueue(transport)
    setQueue(made)
    return () => made.dispose()
  }, [])

  const files = React.useSyncExternalStore(
    queue?.subscribe ?? never,
    queue?.getSnapshot ?? (() => NO_FILES),
    () => NO_FILES
  )

  /** Adds a pick of files; returns how many were left out because the list holds ten. */
  const add = React.useCallback(
    (picked: readonly File[]): number => {
      if (queue === null) return 0
      const { candidates, dropped } = planPick(queue.getSnapshot(), picked)
      queue.add(candidates)
      return dropped
    },
    [queue]
  )
  const remove = React.useCallback((key: string) => queue?.remove(key), [queue])
  const clear = React.useCallback(() => queue?.clear(), [queue])

  return { files, add, remove, clear }
}
