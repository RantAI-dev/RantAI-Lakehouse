"use client"

import * as React from "react"

/**
 * Form state that the server's answer fills until the person first edits it
 * (`adm-121`, decision 5).
 *
 * A settings form read the server's answer into its fields every time the
 * answer arrived. The answer arrives again after a save, a picture upload or
 * a reset, and each time it overwrote what the person had typed in the
 * meantime. Here the first answer fills the form, a person's edit stops
 * later answers from filling it, and `reseed` lets the next answer fill it
 * again for the one case where the server's value is the new truth (a reset
 * that cleared what the person typed).
 *
 * After the person's own save the form keeps its typed value on purpose: the
 * server now holds the same value, so keeping it is not a revert.
 *
 * @param source the server's answer, or `null` while there is none.
 * @param initial what the form holds before the first answer.
 * @param fromSource turns an answer into the form's value.
 */
export function useSeededState<T, S>(
  source: T | null,
  initial: S,
  fromSource: (source: T) => S
): readonly [S, React.Dispatch<React.SetStateAction<S>>, () => void] {
  const [value, setValue] = React.useState<S>(initial)
  const editedRef = React.useRef(false)
  const fromSourceRef = React.useRef(fromSource)
  React.useEffect(() => {
    fromSourceRef.current = fromSource
  })

  React.useEffect(() => {
    if (source !== null && !editedRef.current) setValue(fromSourceRef.current(source))
  }, [source])

  const edit = React.useCallback<React.Dispatch<React.SetStateAction<S>>>((next) => {
    editedRef.current = true
    setValue(next)
  }, [])
  const reseed = React.useCallback(() => {
    editedRef.current = false
  }, [])

  return [value, edit, reseed] as const
}
