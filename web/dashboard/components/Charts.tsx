import { useState, type PointerEvent, type ReactNode } from "react";

export interface Bar {
  value: number;
  /** Shown when the bar is hovered or tapped, e.g. "17 Oct: 11,907 runs". */
  title: string;
  /** Prints the value above the bar (use for peaks only). */
  showValue?: boolean;
}

/**
 * Vertical bar chart made of divs. Bars are blue, the last one is ink ("today" / "current hour").
 * Axis: pass `labels` (one per bar, same gap) or a free-form `axis` node (e.g. 5 evenly spaced times).
 * `ariaLabel` is the text alternative for the whole chart: summarise the range and the latest value.
 */
export function BarChart({
  bars,
  height,
  gap = 8,
  labels,
  axis,
  ariaLabel,
}: {
  bars: Bar[];
  height: number;
  gap?: number;
  labels?: string[];
  axis?: ReactNode;
  ariaLabel: string;
}) {
  const [active, setActive] = useState<number | null>(null);
  const n = bars.length;
  const max = Math.max(1, ...bars.map((b) => b.value));
  const room = height - 18; // headroom for a value label above the tallest bar
  // hover (mouse) or tap (touch) picks the bar under the pointer; touch keeps it until the next tap
  const pick = (e: PointerEvent<HTMLDivElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    setActive(Math.min(n - 1, Math.max(0, Math.floor(((e.clientX - r.left) / r.width) * n))));
  };
  return (
    <div className="flex flex-col gap-2">
      <div
        role="img"
        aria-label={ariaLabel}
        className="relative flex items-end border-b border-line-strong"
        style={{ height, gap }}
        onPointerMove={pick}
        onPointerDown={pick}
        onPointerLeave={(e) => e.pointerType === "mouse" && setActive(null)}
      >
        {bars.map((b, i) => (
          <div key={i} className="flex h-full min-w-0 flex-1 flex-col items-center justify-end gap-1">
            {b.showValue && <span className="font-mono text-[11px] font-medium text-ink-2">{b.value.toLocaleString("en-US")}</span>}
            <div
              className={`w-full rounded-t-[3px] ${i === n - 1 ? "bg-ink" : "bg-chart-1"} ${active !== null && active !== i ? "opacity-40" : ""}`}
              style={{ height: Math.max(2, Math.round((b.value / max) * room)) }}
            />
          </div>
        ))}
        {active !== null && bars[active] && (
          <div
            className={`pointer-events-none absolute top-0 z-10 whitespace-nowrap rounded-md border border-line bg-surface px-2 py-1 font-mono text-xs text-ink ${
              active < n / 4 ? "" : active >= (3 * n) / 4 ? "-translate-x-full" : "-translate-x-1/2"
            }`}
            style={{ left: `${((active + 0.5) / n) * 100}%` }}
          >
            {bars[active].title}
          </div>
        )}
      </div>
      {labels && (
        <div className="flex font-mono text-xs text-ink-3" style={{ gap }} aria-hidden="true">
          {labels.map((l, i) => (
            <span key={i} className="min-w-0 flex-1 text-center">
              {l}
            </span>
          ))}
        </div>
      )}
      {axis && (
        <div className="flex justify-between font-mono text-xs text-ink-3" aria-hidden="true">
          {axis}
        </div>
      )}
    </div>
  );
}
