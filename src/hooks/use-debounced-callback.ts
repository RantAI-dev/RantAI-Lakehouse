import * as React from "react";

import { useCallbackRef } from "@/hooks/use-callback-ref";

export function useDebouncedCallback<T extends (...args: never[]) => unknown>(
  callback: T,
  delay: number,
) {
  const handleCallback = useCallbackRef(callback);
  const debounceTimerRef = React.useRef(0);
  const pendingArgsRef = React.useRef<Parameters<T> | null>(null);

  React.useEffect(
    () => () => {
      window.clearTimeout(debounceTimerRef.current);
      // Flush on unmount so table memory can persist the latest value before
      // the user navigates away mid-debounce.
      if (pendingArgsRef.current !== null) {
        handleCallback(...pendingArgsRef.current);
        pendingArgsRef.current = null;
      }
    },
    [handleCallback],
  );

  const setValue = React.useCallback(
    (...args: Parameters<T>) => {
      pendingArgsRef.current = args;
      window.clearTimeout(debounceTimerRef.current);
      debounceTimerRef.current = window.setTimeout(() => {
        pendingArgsRef.current = null;
        handleCallback(...args);
      }, delay);
    },
    [handleCallback, delay],
  );

  return setValue;
}
