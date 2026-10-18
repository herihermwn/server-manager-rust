import { useEffect, useState } from "react";

/** One line from src/sse.rs (`stdout` / `stderr` events). */
export interface StreamLine {
  stream: "stdout" | "stderr";
  text: string;
}

/** `done` event: exit code, or null + reason (timeout | cancelled | signal | spawn). */
export interface StreamDone {
  code: number | null;
  reason?: string;
}

const MAX_LINES = 5000;

/**
 * Follow an SSE log endpoint (cookie auth works: EventSource is same-origin).
 * Pass null to stay idle. Lines are capped at the last 5,000.
 *   const { lines, done } = useEventStream(`/api/apps/${name}/deploys/${n}/log`);
 */
export function useEventStream(url: string | null) {
  const [lines, setLines] = useState<StreamLine[]>([]);
  const [done, setDone] = useState<StreamDone | null>(null);
  const [connected, setConnected] = useState(false);

  useEffect(() => {
    setLines([]);
    setDone(null);
    if (!url) return;
    const es = new EventSource(url);
    const add = (stream: StreamLine["stream"]) => (e: MessageEvent) =>
      setLines((prev) => {
        const next = prev.length >= MAX_LINES ? prev.slice(-MAX_LINES + 1) : prev.slice();
        next.push({ stream, text: e.data });
        return next;
      });
    es.onopen = () => setConnected(true);
    es.onerror = () => setConnected(false);
    es.addEventListener("stdout", add("stdout"));
    es.addEventListener("stderr", add("stderr"));
    es.addEventListener("done", (e) => {
      try {
        setDone(JSON.parse((e as MessageEvent).data));
      } catch {
        setDone({ code: null });
      }
      es.close();
      setConnected(false);
    });
    return () => es.close();
  }, [url]);

  return { lines, done, connected };
}
