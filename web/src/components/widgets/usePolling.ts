/**
 * A small polling hook shared by the info and service widgets.
 */
import { useEffect, useLayoutEffect, useRef, useState, type DependencyList } from "react";

/** What {@link usePolling} returns. */
export interface PollState<T> {
  /** Result of the last successful call for the current deps, if any. */
  data: T | undefined;
  /** Message of the last failed call for the current deps, or `null`. */
  error: string | null;
  /** `true` until the first call for the current deps has settled. */
  loading: boolean;
}

interface Settled<T> {
  key: string;
  data: T | undefined;
  error: string | null;
}

/** Turns an unknown thrown value into a short message. */
function message(e: unknown): string {
  if (e instanceof Error) return e.message || e.name;
  return String(e);
}

/**
 * Calls `fn` now and then every `intervalMs` milliseconds, and keeps its
 * latest result.
 *
 * - A new call is only scheduled after the previous one settled, so slow
 *   upstreams never pile up requests.
 * - Polling pauses while the page is hidden (`document.hidden`) and runs
 *   again right away when it becomes visible.
 * - Timers and listeners are removed on unmount, and results that arrive
 *   after unmount (or after `deps` changed) are dropped.
 * - When `deps` change, polling restarts and `loading` is `true` again until
 *   the first result for the new deps arrives.
 *
 * `fn` may change on every render; the latest one is always called. `deps`
 * must be serialisable with `JSON.stringify` (ids, numbers, strings).
 *
 * @param fn async function to poll.
 * @param intervalMs delay between the end of one call and the next.
 * @param deps values that, when changed, restart polling.
 */
export function usePolling<T>(
  fn: () => Promise<T>,
  intervalMs: number,
  deps: DependencyList,
): PollState<T> {
  const key = JSON.stringify(deps);
  const fnRef = useRef(fn);
  const [state, setState] = useState<Settled<T> | null>(null);

  useLayoutEffect(() => {
    fnRef.current = fn;
  });

  useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let running = false;

    const schedule = () => {
      clearTimeout(timer);
      if (!alive || document.hidden) return;
      timer = setTimeout(() => void tick(), intervalMs);
    };

    const tick = async () => {
      if (!alive || running) return;
      running = true;
      try {
        const data = await fnRef.current();
        if (alive) setState({ key, data, error: null });
      } catch (e) {
        if (alive)
          setState((prev) => ({
            key,
            data: prev?.key === key ? prev.data : undefined,
            error: message(e),
          }));
      } finally {
        running = false;
        schedule();
      }
    };

    const onVisibility = () => {
      if (document.hidden) {
        clearTimeout(timer);
      } else {
        void tick();
      }
    };

    void tick();
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      alive = false;
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [key, intervalMs]);

  const current = state?.key === key ? state : null;
  return {
    data: current?.data,
    error: current?.error ?? null,
    loading: current === null,
  };
}
