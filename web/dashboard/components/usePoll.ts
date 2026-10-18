import { useEffect, useState } from "react";
import { api } from "../lib/api";

export interface Poll<T> {
  /** Last good response for this url (kept while a refresh fails). */
  data: T | undefined;
  /** Error of the latest request; cleared by the next success. */
  error: (Error & { status?: number }) | null;
  /** First response (or first error) for this url has arrived. */
  loaded: boolean;
  reload: () => void;
  /** When `data` arrived, in ms since epoch (0 before the first response). */
  updatedAt: number;
}

/**
 * GET `url` now and every `everyMs`, JSON in `data`. Pauses while the tab is hidden and refreshes
 * when it comes back. Changing `url` drops the old data. `url = null` does nothing.
 */
export function usePoll<T>(url: string | null, everyMs = 10_000): Poll<T> {
  const [state, setState] = useState<{ url: string | null; data?: T; error: Poll<T>["error"]; at?: number }>({ url, error: null });
  const [tick, setTick] = useState(0);

  useEffect(() => {
    if (!url) return;
    const ac = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    const run = async () => {
      clearTimeout(timer);
      if (!document.hidden) {
        try {
          const data = await api<T>(url, { signal: ac.signal });
          setState({ url, data, error: null, at: Date.now() });
        } catch (e) {
          if (ac.signal.aborted) return;
          setState((s) => ({ url, data: s.url === url ? s.data : undefined, error: e as Error, at: s.url === url ? s.at : undefined }));
        }
      }
      timer = setTimeout(run, everyMs);
    };
    const onVisible = () => !document.hidden && run();
    run();
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      ac.abort();
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [url, everyMs, tick]);

  const mine = state.url === url ? state : { data: undefined, error: null, at: undefined };
  return { data: mine.data, error: mine.error, loaded: mine.data !== undefined || mine.error !== null, reload: () => setTick((t) => t + 1), updatedAt: mine.at ?? 0 };
}

/** Re-renders every `ms` and returns the unix time in seconds ("updated 3 s ago"). */
export function useNowSecs(ms = 1000): number {
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  useEffect(() => {
    const id = setInterval(() => setNow(Math.floor(Date.now() / 1000)), ms);
    return () => clearInterval(id);
  }, [ms]);
  return now;
}
