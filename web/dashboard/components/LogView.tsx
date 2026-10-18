import { useEffect, useRef, useState } from "react";
import { IconButton } from "./Button";
import { CopyIcon } from "./icons";

export interface LogLine {
  text: string;
  /** Shown dim before the text ("13:02:11"). */
  t?: string;
  /** Color; guessed from the text when left out. */
  kind?: "cmd" | "ok" | "warn" | "err";
}

const COLOR = { cmd: "text-term-cmd", ok: "text-term-ok", warn: "text-term-warn", err: "text-term-err" };

/** "$ cargo build" → cmd, "error…" → err, "warning…" → warn. stderr is NOT red: cargo and bun log progress there. */
function guess(text: string): LogLine["kind"] {
  if (text.startsWith("$ ")) return "cmd";
  if (/^(error|fatal|ERR!|panicked)/i.test(text.trimStart())) return "err";
  if (/^warn(ing)?\b/i.test(text.trimStart())) return "warn";
  return undefined;
}

/**
 * Terminal log (DataDisplay.dc.html "Live log"): dark panel, mono 12.5/20, Follow + Copy.
 * Feed it from useEventStream: lines={lines.map((l) => ({ text: l.text }))}.
 */
export function LogView({ title, lines, live = false, height = 360 }: { title: string; lines: LogLine[]; live?: boolean; height?: number }) {
  const [follow, setFollow] = useState(true);
  const pre = useRef<HTMLPreElement>(null);
  useEffect(() => {
    if (follow && pre.current) pre.current.scrollTop = pre.current.scrollHeight;
  }, [lines, follow]);

  return (
    <section aria-label={title} className="flex flex-col overflow-hidden rounded-xl bg-console">
      <div className="flex items-center gap-2.5 border-b border-console-raised py-2 pl-4 pr-2 text-console-text">
        {live && <span className="h-2 w-2 rounded-full bg-term-live" style={{ boxShadow: "0 0 0 4px rgba(111,163,238,0.2)" }} aria-hidden="true" />}
        <span className="flex-1 text-13 font-semibold">{title}</span>
        <label className="inline-flex h-11 items-center gap-2 px-2 text-13 text-console-dim">
          <input type="checkbox" checked={follow} onChange={(e) => setFollow(e.target.checked)} className="h-4 w-4 accent-term-live" />
          Follow
        </label>
        <IconButton label="Copy log" variant="console" onClick={() => navigator.clipboard?.writeText(lines.map((l) => l.text).join("\n"))}>
          <CopyIcon size={16} />
        </IconButton>
      </div>
      <pre
        ref={pre}
        role="log"
        aria-live={live ? "polite" : "off"}
        className="m-0 overflow-auto whitespace-pre-wrap px-4 pb-4 pt-3 font-mono text-[12.5px] leading-5 text-console-text"
        style={{ maxHeight: height }}
      >
        {lines.map((l, i) => {
          const k = l.kind ?? guess(l.text);
          return (
            <span key={i} className="block">
              {l.t && <span className="text-console-muted">{l.t}  </span>}
              <span className={k ? COLOR[k] : undefined}>{l.text}</span>
            </span>
          );
        })}
      </pre>
    </section>
  );
}
