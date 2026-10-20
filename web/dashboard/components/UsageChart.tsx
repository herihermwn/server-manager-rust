import { useState, type PointerEvent, type ReactNode } from "react";

export interface ChartCol {
  /** Main value, in the same unit as the chart's `max` (bars; used memory; inbound). */
  a: number;
  /** Second value: cache on top of `a` (stacked) or outbound below the axis (mirror). */
  b?: number;
  /** Paint `a` red (CPU above the alert threshold). */
  hot?: boolean;
  /** Hover text, e.g. "13:42 · 12%". */
  tip: string;
}

export interface LegendItem {
  color: string;
  label: string;
  value: ReactNode;
}

/**
 * Usage.dc.html chart card: divs only, one column per time slot, `null` = no sample (drawn as a
 * paper-colored gap). kind: bars (single value), stacked (a + b up from the axis), mirror (a up, b down).
 * `colorA` / `colorB` are Tailwind bg classes; keep them literal at the call site.
 */
export function UsageChart({
  title,
  legend,
  kind,
  cols,
  max,
  yLabels,
  axis,
  colorA = "bg-chart-1",
  colorB = "bg-chart-2",
  ariaLabel,
  notice,
  loading,
}: {
  title: string;
  legend: LegendItem[];
  kind: "bars" | "stacked" | "mirror";
  cols: (ChartCol | null)[];
  max: number;
  /** Top, middle and bottom axis labels. */
  yLabels: [string, string, string];
  /** Evenly spaced time labels under the plot. */
  axis: string[];
  colorA?: string;
  colorB?: string;
  ariaLabel: string;
  /** Pill inside the plot ("No data since 13:41"). */
  notice?: string;
  loading?: boolean;
}) {
  const [hover, setHover] = useState<number | null>(null);
  const n = cols.length;
  const h = (v: number) => `${Math.max(0, Math.min(100, (v / (max || 1)) * 100))}%`;
  const hovered = hover !== null ? cols[hover] : undefined;
  // hover (mouse) or tap (touch) picks the column; touch keeps it until the next tap
  const pick = (e: PointerEvent<HTMLDivElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    setHover(Math.min(n - 1, Math.max(0, Math.floor(((e.clientX - r.left) / r.width) * n))));
  };

  return (
    <section className="flex flex-col gap-3.5 rounded-xl border border-line bg-surface p-5">
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <h2 className="text-17 font-semibold text-ink">{title}</h2>
        <div className="flex flex-wrap gap-x-3.5 gap-y-1 text-xs text-ink-2">
          {legend.map((l) => (
            <span key={l.label} className="inline-flex items-center gap-1.5">
              <span className="h-2.5 w-2.5 rounded-[2px]" style={{ background: l.color }} aria-hidden="true" />
              {l.label} <span className="font-mono text-ink">{l.value}</span>
            </span>
          ))}
        </div>
      </div>
      <div className="flex gap-3.5">
        <div className="flex h-40 w-14 shrink-0 flex-col justify-between text-right font-mono text-[11px] text-ink-3" aria-hidden="true">
          {yLabels.map((l, i) => (
            <span key={i}>{l}</span>
          ))}
        </div>
        <div
          role="img"
          aria-label={ariaLabel}
          className="relative h-40 min-w-0 flex-1 border-b border-line-strong"
          onPointerMove={pick}
          onPointerDown={pick}
          onPointerLeave={(e) => e.pointerType === "mouse" && setHover(null)}
        >
          <div className="absolute inset-x-0 top-0 h-px bg-line-soft" aria-hidden="true" />
          <div className="absolute inset-x-0 top-1/2 h-px bg-line-soft" aria-hidden="true" />
          {loading ? (
            <div className="absolute inset-0 animate-pulse bg-muted" aria-hidden="true" />
          ) : (
            <div className={`absolute inset-0 flex ${n <= 90 ? "gap-0.5" : ""}`}>
              {cols.map((c, i) =>
                c === null ? (
                  <div key={i} className="h-full min-w-0 flex-1 bg-paper" />
                ) : kind === "bars" ? (
                  <div key={i} className="flex h-full min-w-0 flex-1 items-end">
                    <div className={`w-full rounded-t-[1px] ${c.hot ? "bg-danger" : colorA}`} style={{ height: h(c.a) }} />
                  </div>
                ) : kind === "stacked" ? (
                  <div key={i} className="flex h-full min-w-0 flex-1 flex-col justify-end">
                    <div className={colorB} style={{ height: h(c.b ?? 0) }} />
                    <div className={c.hot ? "bg-danger" : colorA} style={{ height: h(c.a) }} />
                  </div>
                ) : (
                  <div key={i} className="flex h-full min-w-0 flex-1 flex-col">
                    <div className="flex h-1/2 items-end">
                      <div className={`w-full rounded-t-[1px] ${colorA}`} style={{ height: `${Math.min(100, ((c.a / (max || 1)) * 100))}%` }} />
                    </div>
                    <div className="flex h-1/2 items-start">
                      <div className={`w-full rounded-b-[1px] ${colorB}`} style={{ height: `${Math.min(100, (((c.b ?? 0) / (max || 1)) * 100))}%` }} />
                    </div>
                  </div>
                ),
              )}
            </div>
          )}
          {hover !== null && !loading && (
            <>
              <div className="pointer-events-none absolute inset-y-0 w-px bg-ink/40" style={{ left: `${((hover + 0.5) / n) * 100}%` }} aria-hidden="true" />
              <div
                className={`pointer-events-none absolute top-1 whitespace-nowrap rounded-md border border-line bg-surface px-2 py-1 font-mono text-xs text-ink ${hover > n / 2 ? "right-1" : "left-1"}`}
              >
                {hovered ? hovered.tip : "No data"}
              </div>
            </>
          )}
          {notice && !loading && hover === null && (
            <span className="absolute right-1 top-2 rounded-md border border-line bg-surface px-2 py-1 text-xs font-semibold text-ink-2">{notice}</span>
          )}
        </div>
      </div>
      <div className="flex justify-between pl-[70px] font-mono text-[11px] text-ink-3" aria-hidden="true">
        {axis.map((t, i) => (
          <span key={i}>{t}</span>
        ))}
      </div>
    </section>
  );
}
