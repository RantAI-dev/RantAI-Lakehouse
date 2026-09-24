import * as React from "react"

/**
 * Debounced mirror of `value` — updates only after `delay` ms of quiet.
 *
 * Upstream reached for lodash's `debounce`; this uses a plain `setTimeout`
 * instead rather than pulling a new dependency into the app for a
 * four-line effect. Behaviour is identical for this usage: each change
 * restarts the timer, and unmount cancels the pending update.
 */
export function useDebounce<T>(value: T, delay: number) {
  const [debouncedValue, setDebouncedValue] = React.useState(value);

  React.useEffect(() => {
    const timer = setTimeout(() => setDebouncedValue(value), delay);
    return () => clearTimeout(timer);
  }, [value, delay]);

  return debouncedValue;
}
